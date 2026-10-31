//! [`Session`]: one program running in a PTY, with the emulator its output feeds and the threads
//! that move bytes between them.
//!
//! [`Session::spawn`] opens the PTY (portable-pty; ConPTY on Windows), starts the program through
//! [`crate::shell::command`] and starts three threads:
//!
//! - `pty-reader` reads output, runs it through [`OscSniffer`] (notifications, OSC 7) and then
//!   the emulator, and raises [`SessionEvent::Wakeup`].
//! - `pty-waiter` waits on the child and raises [`SessionEvent::Exited`].
//! - `pty-sync` flushes a synchronized update (mode 2026) the program never closed.
//!
//! The app (`crates/app/src/session.rs` and `update/terminals.rs`) forwards the events into its
//! own message loop and calls [`Session::snapshot`] to draw. The emulator lives behind a
//! [`FairMutex`] that is only held to feed, query or copy, never while rendering.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Instant;

use alacritty_terminal::sync::FairMutex;
use portable_pty::{ChildKiller, MasterPty, PtySize, native_pty_system};

use crate::emulator::{AlacrittyEmulator, Emulator, EmulatorEvent};
use crate::grid::{
    CellSide, CursorShape, DefaultColors, GridPoint, GridSize, Modes, Scroll, SearchDirection,
    SelectionKind, Snapshot,
};
use crate::osc::{Notification, OscEvent, OscSniffer};

/// What to run in a new [`Session`].
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Program to run; `None` runs the platform default shell.
    pub program: Option<String>,
    /// Arguments for `program`. Ignored when `program` is `None`.
    pub args: Vec<String>,
    /// Starting directory; `None` starts in the home directory.
    pub cwd: Option<PathBuf>,
    /// Initial size of the PTY and the grid.
    pub size: GridSize,
    /// Lines of scrollback history to keep.
    pub scrollback: usize,
    /// Extra environment variables for the program, applied after the `TERM` family so they
    /// can override it.
    pub env: Vec<(String, String)>,
    /// What the program is told when it asks for a color it hasn't set (OSC 4, 10, 11, 12);
    /// `None` leaves those queries unanswered. [`Session::set_default_colors`] changes them.
    pub colors: Option<DefaultColors>,
    /// The cursor's shape until the program asks for another; [`Session::set_default_cursor`]
    /// changes it.
    pub cursor: CursorShape,
}

/// Bytes the emulator wants sent back to the program, waiting for the emulator lock to be
/// released (see [`send_replies`]).
type Replies = Arc<Mutex<Vec<u8>>>;

/// Something that happened in a session. Raised on the session's own threads, through the
/// `notify` callback given to [`Session::spawn`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// New output was parsed. At most one is pending until the next [`Session::snapshot`].
    Wakeup,
    /// The program set the title (OSC 0/2).
    Title(String),
    /// The program reset the title to the default.
    ResetTitle,
    /// A BEL character.
    Bell,
    /// The program asked to copy text to the clipboard (OSC 52).
    ClipboardStore(String),
    /// A program asked for a desktop notification (OSC 9, 777 or 99).
    Notify(Notification),
    /// The shell reported its working directory (OSC 7).
    Cwd(String),
    /// The child exited, with its exit code when it could be read. Output may still arrive after
    /// this (see the `pty-waiter` thread in [`Session::spawn`]).
    Exited(Option<u32>),
}

/// One shell: a PTY, the emulator it feeds, and the threads that move bytes between them.
///
/// All methods take `&self` and are safe to call from the UI thread; they lock the emulator only
/// for the duration of the call. Dropping a session kills the child process.
pub struct Session {
    /// The emulator, shared with the reader and sync threads. A [`FairMutex`] so a reader thread
    /// that relocks constantly under heavy output can't starve [`Session::snapshot`].
    emulator: Arc<FairMutex<AlacrittyEmulator>>,
    /// The PTY's input, shared with the reader and sync threads, which send the emulator's
    /// replies to terminal queries.
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// The PTY master, kept to resize it. Dropping it closes the pseudoconsole, which is what
    /// finally ends the reader thread on Windows.
    master: Box<dyn MasterPty + Send>,
    /// Kills the child on drop; the child itself was moved into the waiter thread.
    killer: Box<dyn ChildKiller + Send + Sync>,
    /// Set when output has been parsed since the last snapshot; merges wakeups.
    dirty: Arc<AtomicBool>,
}

impl Session {
    /// Opens a PTY of `config.size`, spawns the program in it and starts the `pty-reader`,
    /// `pty-waiter` and `pty-sync` threads.
    ///
    /// `notify` is called from those threads (never the caller's), so it should only forward
    /// the event, for example into a channel. The child gets `TERM=xterm-256color`,
    /// `COLORTERM=truecolor` and `TERM_PROGRAM=macaroni` plus `config.env`.
    ///
    /// Fails when the PTY can't be opened, the program can't be started, or a thread can't be
    /// spawned.
    pub fn spawn(
        config: SessionConfig,
        notify: impl Fn(SessionEvent) + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let notify: Arc<dyn Fn(SessionEvent) + Send + Sync> = Arc::new(notify);

        let pair = native_pty_system()
            .openpty(pty_size(config.size))
            .map_err(io::Error::other)?;

        let mut command = crate::shell::command(config.program.as_deref(), &config.args);
        if let Some(cwd) = &config.cwd {
            command.cwd(cwd);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "macaroni");
        for (key, value) in &config.env {
            command.env(key, value);
        }

        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(io::Error::other)?;
        // Only the child should hold the slave side; our copy would keep the PTY open after the
        // child exits.
        drop(pair.slave);
        let killer = child.clone_killer();

        let mut reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
        let writer = Arc::new(Mutex::new(
            pair.master.take_writer().map_err(io::Error::other)?,
        ));

        // Replies to terminal queries, collected while `feed` runs and written by whoever fed
        // once the emulator lock is released (`send_replies`). Writing from inside `feed` would
        // hold the lock while blocked on a program that isn't reading its input, freezing
        // `snapshot` on the UI thread with it.
        let replies: Replies = Arc::new(Mutex::new(Vec::new()));
        let emulator = {
            let replies = replies.clone();
            let notify = notify.clone();
            let mut emulator =
                AlacrittyEmulator::new(config.size, config.scrollback, move |event| {
                    let event = match event {
                        EmulatorEvent::PtyWrite(text) => {
                            if let Ok(mut replies) = replies.lock() {
                                replies.extend_from_slice(text.as_bytes());
                            }
                            return;
                        }
                        EmulatorEvent::Title(title) => SessionEvent::Title(title),
                        EmulatorEvent::ResetTitle => SessionEvent::ResetTitle,
                        EmulatorEvent::Bell => SessionEvent::Bell,
                        EmulatorEvent::ClipboardStore(text) => SessionEvent::ClipboardStore(text),
                    };
                    notify(event);
                });
            if let Some(colors) = config.colors {
                emulator.set_default_colors(colors);
            }
            if config.cursor != CursorShape::Block {
                emulator.set_default_cursor(config.cursor);
            }
            emulator
        };
        let emulator = Arc::new(FairMutex::new(emulator));
        let dirty = Arc::new(AtomicBool::new(false));

        // A program that opens a synchronized update and never closes it must not freeze the
        // screen: this thread applies the update at its deadline if it's still open.
        let (sync_tx, sync_rx) = mpsc::channel::<Instant>();
        {
            let emulator = emulator.clone();
            let dirty = dirty.clone();
            let notify = notify.clone();
            let replies = replies.clone();
            let writer = writer.clone();
            thread::Builder::new()
                .name("pty-sync".into())
                .spawn(move || {
                    // Sleeps until a deadline arrives, then until it passes. A newer deadline
                    // received meanwhile (the program kept sending updates) replaces it. The
                    // loop ends when the reader thread exits and drops `sync_tx`.
                    while let Ok(mut deadline) = sync_rx.recv() {
                        while let Ok(later) =
                            sync_rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                        {
                            deadline = later;
                        }
                        // Re-check under the lock: the update may have ended (or a new one
                        // started with a later deadline) while this thread slept.
                        let mut emulator = emulator.lock();
                        if emulator
                            .sync_deadline()
                            .is_some_and(|d| d <= Instant::now())
                        {
                            emulator.flush_sync();
                            drop(emulator);
                            send_replies(&replies, &writer);
                            if !dirty.swap(true, Ordering::AcqRel) {
                                notify(SessionEvent::Wakeup);
                            }
                        }
                    }
                })?;
        }

        {
            let emulator = emulator.clone();
            let dirty = dirty.clone();
            let notify = notify.clone();
            let writer = writer.clone();
            thread::Builder::new()
                .name("pty-reader".into())
                .spawn(move || {
                    let mut buf = vec![0u8; 64 * 1024];
                    let mut sniffer = OscSniffer::default();
                    loop {
                        match reader.read(&mut buf) {
                            // EOF ends the thread but is not how exit is detected (ConPTY keeps
                            // the pipe open; see `pty-waiter` below).
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                // The sniffer runs first and outside the lock: it has its own
                                // parser and doesn't need the grid.
                                for event in sniffer.feed(&buf[..n]) {
                                    notify(match event {
                                        OscEvent::Notify(n) => SessionEvent::Notify(n),
                                        OscEvent::Cwd(path) => SessionEvent::Cwd(path),
                                    });
                                }
                                let deadline = {
                                    let mut emulator = emulator.lock();
                                    emulator.feed(&buf[..n]);
                                    emulator.sync_deadline()
                                };
                                // Before the next read, and so before any further output: this
                                // is load-bearing on Windows. ConPTY sends `ESC[6n` at startup
                                // (the pseudoconsole is created with
                                // PSEUDOCONSOLE_INHERIT_CURSOR) and the inbox version prints
                                // nothing until the cursor position comes back.
                                send_replies(&replies, &writer);
                                if let Some(deadline) = deadline {
                                    let _ = sync_tx.send(deadline);
                                }
                                // Wakeup merging: only the chunk that flips `dirty` from false
                                // to true notifies. Later chunks just update the grid, and the
                                // UI sees them all in its next snapshot, which clears the flag.
                                if !dirty.swap(true, Ordering::AcqRel) {
                                    notify(SessionEvent::Wakeup);
                                }
                            }
                        }
                    }
                })?;
        }

        // ConPTY keeps the output pipe open after the child exits, so the reader never sees EOF
        // until the pseudoconsole is closed. Exit is detected by waiting on the child instead.
        // The reader runs independently, so the last output can be parsed after `Exited`.
        thread::Builder::new()
            .name("pty-waiter".into())
            .spawn(move || {
                let code = child.wait().ok().map(|status| status.exit_code());
                notify(SessionEvent::Exited(code));
            })?;

        Ok(Self {
            emulator,
            writer,
            master: pair.master,
            killer,
            dirty,
        })
    }

    /// Changes the colors the program is told when it asks for one it hasn't set, for a new
    /// theme.
    pub fn set_default_colors(&self, colors: DefaultColors) {
        self.emulator.lock().set_default_colors(colors);
    }

    /// Changes the cursor's shape for while the program hasn't asked for one.
    pub fn set_default_cursor(&self, shape: CursorShape) {
        self.emulator.lock().set_default_cursor(shape);
    }

    /// Shows `bytes` as if the program had printed them, for the app's own messages in a
    /// terminal whose program has exited. Takes effect in the next snapshot.
    pub fn print(&self, bytes: &[u8]) {
        self.emulator.lock().feed(bytes);
    }

    /// Sends input (encoded keys, pastes, mouse reports) to the program. Errors are ignored:
    /// they only happen once the child is gone, which `Exited` reports.
    pub fn write(&self, bytes: &[u8]) {
        write_all(&self.writer, bytes);
    }

    /// Resizes the PTY (so the program gets `SIGWINCH` or the ConPTY equivalent) and the grid.
    pub fn resize(&self, size: GridSize) {
        let _ = self.master.resize(pty_size(size));
        self.emulator.lock().resize(size);
    }

    /// Moves the view through the scrollback. Takes effect in the next snapshot.
    pub fn scroll(&self, scroll: Scroll) {
        self.emulator.lock().scroll(scroll);
    }

    /// Copies the visible screen and re-arms the [`SessionEvent::Wakeup`] notification.
    pub fn snapshot(&self) -> Snapshot {
        // Clear before copying, not after: output parsed while the copy is being taken sets the
        // flag again and raises a new wakeup, so no update is lost.
        self.dirty.store(false, Ordering::Release);
        self.emulator.lock().snapshot()
    }

    /// The program's current terminal modes, without copying the screen.
    pub fn modes(&self) -> Modes {
        self.emulator.lock().modes()
    }

    /// See [`Emulator::start_selection`].
    pub fn start_selection(&self, kind: SelectionKind, at: GridPoint, side: CellSide) {
        self.emulator.lock().start_selection(kind, at, side);
    }

    /// See [`Emulator::update_selection`].
    pub fn update_selection(&self, at: GridPoint, side: CellSide) {
        self.emulator.lock().update_selection(at, side);
    }

    /// See [`Emulator::clear_selection`].
    pub fn clear_selection(&self) {
        self.emulator.lock().clear_selection();
    }

    /// See [`Emulator::selection_text`].
    pub fn selection_text(&self) -> Option<String> {
        self.emulator.lock().selection_text()
    }

    /// See [`Emulator::hyperlink_at`].
    pub fn hyperlink_at(&self, at: GridPoint) -> Option<String> {
        self.emulator.lock().hyperlink_at(at)
    }

    /// See [`Emulator::search`].
    pub fn search(&self, query: &str, direction: SearchDirection) -> bool {
        self.emulator.lock().search(query, direction)
    }

    /// See [`Emulator::reset_search`].
    pub fn reset_search(&self) {
        self.emulator.lock().reset_search();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Runs before the fields drop. Killing the child ends the waiter thread; dropping
        // `master` afterwards closes the pseudoconsole, which ends the reader thread, which in
        // turn ends the sync thread. Nothing is joined.
        let _ = self.killer.kill();
    }
}

/// Writes the replies queued by the emulator to the program, if there are any. Called without
/// the emulator lock held, so a program that isn't reading its input only blocks this thread.
fn send_replies(replies: &Replies, writer: &Mutex<Box<dyn Write + Send>>) {
    let bytes = match replies.lock() {
        Ok(mut replies) if !replies.is_empty() => std::mem::take(&mut *replies),
        _ => return,
    };
    write_all(writer, &bytes);
}

/// Writes all of `bytes` to the PTY and flushes. Errors (and a poisoned lock) are ignored, since
/// they only mean the program is gone.
fn write_all(writer: &Mutex<Box<dyn Write + Send>>, bytes: &[u8]) {
    let Ok(mut writer) = writer.lock() else {
        return;
    };
    let _ = writer.write_all(bytes).and_then(|()| writer.flush());
}

/// The PTY size for a grid size. Rows and columns are at least 1, matching the emulator's clamp.
fn pty_size(size: GridSize) -> PtySize {
    PtySize {
        rows: size.rows.max(1),
        cols: size.cols.max(1),
        pixel_width: size.cols.saturating_mul(size.cell_width),
        pixel_height: size.rows.saturating_mul(size.cell_height),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;

    /// Spawns a real shell through the platform PTY (ConPTY on Windows) and waits for output.
    #[test]
    fn real_shell_round_trip() {
        let (program, args, expected) = if cfg!(windows) {
            (
                "cmd.exe",
                vec!["/c", "echo macaroni-%OS%"],
                "macaroni-Windows_NT",
            )
        } else {
            (
                "/bin/sh",
                vec!["-c", "echo macaroni-$((40+2))"],
                "macaroni-42",
            )
        };
        let (tx, rx) = mpsc::channel();
        let session = Session::spawn(
            SessionConfig {
                program: Some(program.into()),
                args: args.into_iter().map(String::from).collect(),
                cwd: None,
                size: GridSize {
                    cols: 80,
                    rows: 24,
                    cell_width: 8,
                    cell_height: 16,
                },
                scrollback: 100,
                env: Vec::new(),
                colors: None,
                cursor: CursorShape::Block,
            },
            move |event| {
                let _ = tx.send(event);
            },
        )
        .expect("spawn");

        // ConPTY can deliver the output after the child has already exited.
        let mut deadline = Instant::now() + Duration::from_secs(15);
        let mut screen = String::new();
        while Instant::now() < deadline {
            screen = session
                .snapshot()
                .rows
                .iter()
                .flat_map(|row| row.iter().map(|c| c.ch).chain(['\n']))
                .collect();
            if screen.contains(expected) {
                return;
            }
            if let Ok(SessionEvent::Exited(_)) = rx.recv_timeout(Duration::from_millis(100)) {
                deadline = deadline.min(Instant::now() + Duration::from_secs(3));
            }
        }
        panic!("{expected:?} never appeared:\n{screen}");
    }
}

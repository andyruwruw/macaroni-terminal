//! [`Session`]: one program, local in a PTY or remote over SSH, with the emulator its output
//! feeds and the threads that move bytes between them.
//!
//! [`Session::spawn`] opens its [`Source`] (a local PTY through portable-pty, ConPTY on Windows,
//! with the program started through [`crate::shell::command`]; or an SSH channel through
//! [`crate::ssh`]) and starts three threads:
//!
//! - `pty-reader` reads output, runs it through [`OscSniffer`] (notifications, OSC 7, prompt
//!   marks, inline images), through
//!   [`Lines`] and the triggers when there are any, and then the emulator, and raises
//!   [`SessionEvent::Wakeup`].
//! - `pty-waiter` waits for the program to end and raises [`SessionEvent::Exited`].
//! - `pty-sync` flushes a synchronized update (mode 2026) the program never closed.
//!
//! The app (`crates/app/src/session.rs` and `update/terminals.rs`) forwards the events into its
//! own message loop and calls [`Session::snapshot`] to draw. The emulator lives behind a
//! [`FairMutex`] that is only held to feed, query or copy, never while rendering.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::thread;
use std::time::Instant;

use alacritty_terminal::sync::FairMutex;
use regex::RegexSet;

use crate::emulator::{AlacrittyEmulator, Emulator, EmulatorEvent};
use crate::grid::{
    CellSide, CursorShape, DefaultColors, GridPoint, GridSize, Modes, Scroll, SearchDirection,
    SelectionKind, Snapshot,
};
use crate::lines::Lines;
use crate::osc::{Notification, OscEvent, OscSniffer};
use crate::source::{Control, LocalCommand, Opened, Source, open_local};
use crate::tmux::{Gate, TmuxClient, TmuxEvent};

/// What to run in a new [`Session`].
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// What runs: a local program (the default), or a shell over SSH, which ignores
    /// `program`, `args` and `cwd`.
    pub source: Source,
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
    /// Patterns matched against each line of output, raising [`SessionEvent::Triggered`];
    /// [`Session::set_triggers`] changes them.
    pub triggers: Option<RegexSet>,
}

/// The patterns the reader thread matches lines against, shared so they can change while it
/// runs.
type Triggers = Arc<RwLock<Option<RegexSet>>>;

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
    /// A line of output matched a trigger. Raised at most once per trigger per read, for the
    /// first line it matched, so a flood of matching lines doesn't flood the app.
    Triggered {
        /// The pattern's index in the [`RegexSet`].
        trigger: usize,
        /// The line, in plain text.
        line: String,
    },
    /// The shell started running a command the user typed (OSC 133 `C`): the command, as
    /// shown after the prompt.
    CommandStarted(String),
    /// A command the shell marked (OSC 133 `C` to `D`) finished: its exit code when the shell
    /// gave one, and how long it ran.
    CommandFinished {
        /// The exit code.
        exit: Option<i32>,
        /// From the output mark to the finish mark.
        elapsed: std::time::Duration,
    },
    /// The child exited, with its exit code when it could be read. Output may still arrive after
    /// this (see the `pty-waiter` thread in [`Session::spawn`]).
    Exited(Option<u32>),
    /// tmux control mode in this session started, changed or ended.
    Tmux(TmuxEvent),
}

/// The reader thread's OSC 133 state: turns each mark into an OSC 8 link for the emulator
/// ([`crate::emulator::MARK_PREFIX`]) and times commands.
///
/// alacritty ignores OSC 133, and a row number recorded for a mark would go stale as soon as
/// scrollback is trimmed. A hyperlink lives in the cells themselves, so it moves with the text:
/// the prompt's cells carry `macaroni-mark:prompt[:<exit code of the command before>]`, the
/// output's `macaroni-mark:output`, each with an id of its own so two marks of a kind in a row
/// stay apart. Links are only looked up on demand and never drawn, so they don't show.
#[derive(Default)]
struct Marks {
    /// Makes each mark's link id unique within the session.
    seq: u64,
    /// When the running command's output started (`C`), until its `D`.
    started: Option<Instant>,
    /// The exit code of the last command that ran, for the next prompt's link.
    exit: Option<i32>,
}

impl Marks {
    /// The bytes to feed the emulator right after `mark`, and the event to send when it ends a
    /// command. A `D` without a `C` before it (a prompt redrawn with no command run) reports
    /// nothing.
    fn next(&mut self, mark: crate::osc::Mark) -> (Vec<u8>, Option<SessionEvent>) {
        use crate::osc::Mark;
        self.seq += 1;
        let open = |uri: String| format!("\x1b]8;id=mcr{};{uri}\x07", self.seq).into_bytes();
        let close = b"\x1b]8;;\x07".to_vec();
        match mark {
            Mark::Prompt => {
                let uri = match self.exit.take() {
                    Some(code) => format!("{}prompt:{code}", crate::emulator::MARK_PREFIX),
                    None => format!("{}prompt", crate::emulator::MARK_PREFIX),
                };
                (open(uri), None)
            }
            Mark::Command => (close, None),
            Mark::Output => {
                self.started = Some(Instant::now());
                (
                    open(format!("{}output", crate::emulator::MARK_PREFIX)),
                    None,
                )
            }
            Mark::Finished(exit) => {
                let event = self.started.take().map(|started| {
                    self.exit = exit;
                    SessionEvent::CommandFinished {
                        exit,
                        elapsed: started.elapsed(),
                    }
                });
                (close, event)
            }
        }
    }
}

/// `bytes` with each insert's bytes placed at its offset (offsets in order); `bytes` itself,
/// unallocated, when there are none.
fn with_inserts<'a, B: AsRef<[u8]>>(
    bytes: &'a [u8],
    inserts: &[(usize, B)],
) -> std::borrow::Cow<'a, [u8]> {
    if inserts.is_empty() {
        return std::borrow::Cow::Borrowed(bytes);
    }
    let mut out = Vec::with_capacity(bytes.len() + inserts.len() * 32);
    let mut from = 0;
    for (at, insert) in inserts {
        out.extend_from_slice(&bytes[from..*at]);
        out.extend_from_slice(insert.as_ref());
        from = *at;
    }
    out.extend_from_slice(&bytes[from..]);
    std::borrow::Cow::Owned(out)
}

/// One shell: a PTY, the emulator it feeds, and the threads that move bytes between them.
///
/// All methods take `&self` and are safe to call from the UI thread; they lock the emulator only
/// for the duration of the call. Dropping a session kills the child process.
pub struct Session {
    /// The emulator, shared with the reader and sync threads. A [`FairMutex`] so a reader thread
    /// that relocks constantly under heavy output can't starve [`Session::snapshot`].
    emulator: Arc<FairMutex<AlacrittyEmulator>>,
    /// The program's input, shared with the reader and sync threads, which send the emulator's
    /// replies to terminal queries.
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// Resizes the program's terminal; dropping it (with the session) ends the program, which
    /// ends the waiter thread, and closing its output ends the reader and sync threads. Nothing
    /// is joined.
    control: Box<dyn Control>,
    /// Set when output has been parsed since the last snapshot; merges wakeups.
    dirty: Arc<AtomicBool>,
    /// The triggers, shared with the reader thread.
    triggers: Triggers,
    /// The tmux control client while this session carries one (`tmux -CC` runs in it).
    tmux: Arc<Mutex<Option<Arc<TmuxClient>>>>,
    /// Set by the reader thread from the shell's marks: on at the prompt's end (OSC 133 `B`),
    /// off at anything else, so it's on while the shell's line editor has the input.
    at_prompt: Arc<AtomicBool>,
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

        let opened = match &config.source {
            Source::Local => open_local(
                &LocalCommand {
                    program: config.program.as_deref(),
                    args: &config.args,
                    cwd: config.cwd.as_ref(),
                    env: &config.env,
                },
                config.size,
            )?,
            Source::Ssh(target) => crate::ssh::open(target.clone(), config.size, &config.env),
            Source::Tmux(pane) => crate::tmux::open(pane),
        };
        // A tmux pane can't start control mode itself; any other source can carry it.
        let watch_tmux = !matches!(config.source, Source::Tmux(_));
        let Opened {
            mut reader,
            writer,
            control,
            waiter,
        } = opened;
        let writer = Arc::new(Mutex::new(writer));
        let tmux: Arc<Mutex<Option<Arc<TmuxClient>>>> = Arc::new(Mutex::new(None));

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
        let triggers: Triggers = Arc::new(RwLock::new(config.triggers));

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

        let at_prompt = Arc::new(AtomicBool::new(false));
        {
            let emulator = emulator.clone();
            let dirty = dirty.clone();
            let notify = notify.clone();
            let writer = writer.clone();
            let triggers = triggers.clone();
            let tmux_slot = tmux.clone();
            let at_prompt = at_prompt.clone();
            thread::Builder::new()
                .name("pty-reader".into())
                .spawn(move || {
                    let mut buf = vec![0u8; 64 * 1024];
                    let mut sniffer = OscSniffer::default();
                    let mut marks = Marks::default();
                    let mut lines = Lines::default();
                    let mut gate =
                        watch_tmux.then(|| Gate::new(writer.clone(), notify.clone(), tmux_slot));
                    loop {
                        match reader.read(&mut buf) {
                            // EOF ends the thread but is not how exit is detected (ConPTY keeps
                            // the pipe open; see `pty-waiter` below).
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                // In tmux control mode the protocol goes to the client, and the
                                // terminal only shows what's around it.
                                let data: std::borrow::Cow<'_, [u8]> = match &mut gate {
                                    Some(gate) => gate.filter(&buf[..n]).into(),
                                    None => (&buf[..n]).into(),
                                };
                                if data.is_empty() {
                                    continue;
                                }
                                let n = data.len();
                                // The sniffer runs first and outside the lock: it has its own
                                // parser and doesn't need the grid.
                                let mut inserts = Vec::new();
                                // Where the prompt and typed command start (`true`) and end, for
                                // `Lines` to leave them out.
                                let mut quiet = Vec::new();
                                let mut images = Vec::new();
                                // Where commands' output starts, to read each command there.
                                let mut starts = Vec::new();
                                for (at, event) in sniffer.feed_at(&data[..n]) {
                                    match event {
                                        OscEvent::Notify(n) => notify(SessionEvent::Notify(n)),
                                        OscEvent::Cwd(path) => notify(SessionEvent::Cwd(path)),
                                        OscEvent::Image(request) => images.push((at, request)),
                                        OscEvent::Mark(mark) => {
                                            use crate::osc::Mark;
                                            quiet.push((
                                                at,
                                                matches!(mark, Mark::Prompt | Mark::Command),
                                            ));
                                            if mark == Mark::Output {
                                                starts.push(at);
                                            }
                                            at_prompt
                                                .store(mark == Mark::Command, Ordering::Relaxed);
                                            let (link, finished) = marks.next(mark);
                                            inserts.push((at, link));
                                            if let Some(event) = finished {
                                                notify(event);
                                            }
                                        }
                                    }
                                }
                                if let Ok(triggers) = triggers.read()
                                    && let Some(set) = triggers.as_ref()
                                {
                                    let mut fired = vec![false; set.len()];
                                    let mut matched = |line: &str| {
                                        for trigger in set.matches(line).iter() {
                                            if !std::mem::replace(&mut fired[trigger], true) {
                                                notify(SessionEvent::Triggered {
                                                    trigger,
                                                    line: line.to_owned(),
                                                });
                                            }
                                        }
                                    };
                                    let mut from = 0;
                                    for &(at, on) in &quiet {
                                        lines.feed(&data[from..at], &mut matched);
                                        lines.set_quiet(on);
                                        from = at;
                                    }
                                    lines.feed(&data[from..n], &mut matched);
                                } else if let Some(&(_, on)) = quiet.last() {
                                    lines.set_quiet(on);
                                }
                                let mut commands = Vec::new();
                                let deadline = {
                                    let mut emulator = emulator.lock();
                                    // Placed under the lock: how many cells an image covers
                                    // depends on the grid's size, which a resize could change.
                                    for (at, request) in images {
                                        if let Some(image) = request.place(emulator.size()) {
                                            inserts.push((at, image.anchors()));
                                            emulator.add_image(image);
                                        }
                                    }
                                    // Stable, so a mark and an image ending together stay in
                                    // the order they came.
                                    inserts.sort_by_key(|(at, _)| *at);
                                    // Fed in parts cut where each command's output starts: the
                                    // command is read off the grid there, before output moves
                                    // the cursor away from it.
                                    let mut from = 0;
                                    let mut next = 0;
                                    for (i, &cut) in starts.iter().chain([&n]).enumerate() {
                                        let end = inserts[next..]
                                            .iter()
                                            .position(|(at, _)| *at > cut)
                                            .map_or(inserts.len(), |p| next + p);
                                        let part: Vec<(usize, &[u8])> = inserts[next..end]
                                            .iter()
                                            .map(|(at, bytes)| (at - from, bytes.as_slice()))
                                            .collect();
                                        emulator.feed(&with_inserts(&data[from..cut], &part));
                                        if i < starts.len()
                                            && let Some(command) = emulator.typed_command()
                                        {
                                            commands.push(command);
                                        }
                                        from = cut;
                                        next = end;
                                    }
                                    emulator.sync_deadline()
                                };
                                for command in commands {
                                    notify(SessionEvent::CommandStarted(command));
                                }
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

        // The reader runs independently, so the last output can be parsed after `Exited`.
        thread::Builder::new()
            .name("pty-waiter".into())
            .spawn(move || notify(SessionEvent::Exited(waiter())))?;

        Ok(Self {
            emulator,
            writer,
            control,
            dirty,
            triggers,
            tmux,
            at_prompt,
        })
    }

    /// The tmux client this session carries while `tmux -CC` runs in it.
    pub fn tmux(&self) -> Option<Arc<TmuxClient>> {
        self.tmux.lock().ok()?.clone()
    }

    /// Replaces the patterns output lines are matched against; `None` stops matching.
    pub fn set_triggers(&self, triggers: Option<RegexSet>) {
        if let Ok(mut current) = self.triggers.write() {
            *current = triggers;
        }
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
        self.control.resize(size);
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

    /// See [`Emulator::jump_to_prompt`].
    pub fn jump_to_prompt(&self, up: bool) -> bool {
        self.emulator.lock().jump_to_prompt(up)
    }

    /// See [`Emulator::all_text`].
    pub fn all_text(&self) -> String {
        self.emulator.lock().all_text()
    }

    /// See [`Emulator::last_output`].
    pub fn last_output(&self) -> Option<String> {
        self.emulator.lock().last_output()
    }

    /// The arrow keys that move the cursor to the screen cell `at` (left-clicked) in the
    /// command being typed at the shell's prompt, as the program expects them; `None` unless
    /// the shell is waiting at a marked prompt and `at` is in that command
    /// ([`Emulator::cursor_moves`]) somewhere other than the cursor.
    pub fn click_to_cursor(&self, at: GridPoint) -> Option<Vec<u8>> {
        if !self.at_prompt.load(Ordering::Relaxed) {
            return None;
        }
        let emulator = self.emulator.lock();
        let moves = emulator.cursor_moves(at).filter(|&n| n != 0)?;
        let arrow: &[u8] = match (moves > 0, emulator.modes().app_cursor) {
            (true, false) => b"\x1b[C",
            (false, false) => b"\x1b[D",
            (true, true) => b"\x1bOC",
            (false, true) => b"\x1bOD",
        };
        Some(arrow.repeat(moves.unsigned_abs() as usize))
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

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    #[test]
    fn marks_become_links_and_time_commands() {
        use crate::osc::Mark;
        let mut marks = super::Marks::default();
        // A prompt redrawn with no command (D without C) reports nothing.
        assert!(marks.next(Mark::Finished(Some(0))).1.is_none());
        let (link, _) = marks.next(Mark::Prompt);
        assert!(
            String::from_utf8(link)
                .unwrap()
                .ends_with(";macaroni-mark:prompt\x07")
        );
        marks.next(Mark::Output);
        let (close, event) = marks.next(Mark::Finished(Some(3)));
        assert_eq!(close, b"\x1b]8;;\x07");
        assert!(matches!(
            event,
            Some(super::SessionEvent::CommandFinished { exit: Some(3), .. })
        ));
        let (link, _) = marks.next(Mark::Prompt);
        assert!(
            String::from_utf8(link)
                .unwrap()
                .ends_with(";macaroni-mark:prompt:3\x07")
        );
    }

    #[test]
    fn inserts_land_at_their_offsets() {
        let inserts = [(1, b"<".to_vec()), (3, b">".to_vec())];
        assert_eq!(&*super::with_inserts(b"abcd", &inserts), b"a<bc>d");
        assert!(matches!(
            super::with_inserts::<Vec<u8>>(b"abcd", &[]),
            std::borrow::Cow::Borrowed(_)
        ));
    }
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
                source: crate::Source::Local,
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
                triggers: Some(RegexSet::new([r"macaroni-\w+"]).unwrap()),
            },
            move |event| {
                let _ = tx.send(event);
            },
        )
        .expect("spawn");

        // ConPTY can deliver the output after the child has already exited.
        let mut deadline = Instant::now() + Duration::from_secs(15);
        let mut screen = String::new();
        let mut triggered = false;
        while Instant::now() < deadline {
            screen = session
                .snapshot()
                .rows
                .iter()
                .flat_map(|row| row.iter().map(|c| c.ch).chain(['\n']))
                .collect();
            if screen.contains(expected) && triggered {
                return;
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(SessionEvent::Exited(_)) => {
                    deadline = deadline.min(Instant::now() + Duration::from_secs(3));
                }
                Ok(SessionEvent::Triggered { trigger: 0, line }) => {
                    triggered |= line.contains(expected);
                }
                _ => {}
            }
        }
        panic!("{expected:?} never appeared (triggered: {triggered}):\n{screen}");
    }

    /// A program that prints a marked prompt, a command and its output start, as a shell with
    /// the integration does, reports the command when its output starts.
    #[test]
    fn a_marked_command_is_reported_when_it_starts() {
        // Pauses between the parts, as typing makes, so ConPTY has drawn the command before
        // the output mark passes through.
        let (program, args) = if cfg!(windows) {
            (
                "powershell.exe",
                vec![
                    "-NoProfile",
                    "-Command",
                    "$e=[char]27; $b=[char]7; \
                     [Console]::Write(\"$e]133;A$b`$ $e]133;B$b\"); Start-Sleep -m 300; \
                     [Console]::Write('git  status'); Start-Sleep -m 300; \
                     [Console]::Write(\"`r`n$e]133;C$b\"); Start-Sleep -m 300; \
                     [Console]::Write(\"clean`r`n$e]133;D;0$b\"); Start-Sleep -m 300",
                ],
            )
        } else {
            (
                "/bin/sh",
                vec![
                    "-c",
                    "printf '\\033]133;A\\007$ \\033]133;B\\007git  status'; \
                     printf '\\r\\n\\033]133;C\\007clean\\r\\n\\033]133;D;0\\007'",
                ],
            )
        };
        let (tx, rx) = mpsc::channel();
        let _session = Session::spawn(
            SessionConfig {
                source: crate::Source::Local,
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
                triggers: None,
            },
            move |event| {
                let _ = tx.send(event);
            },
        )
        .expect("spawn");
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(SessionEvent::CommandStarted(command)) => {
                    assert_eq!(command, "git  status");
                    return;
                }
                Ok(SessionEvent::CommandFinished { .. }) => panic!("finished, never started"),
                _ => {}
            }
        }
        panic!("the command was never reported");
    }

    /// The real shells' integrations mark where the command line starts, so a typed command
    /// is reported: bash (Git Bash on Windows) and, on Windows, PowerShell.
    #[test]
    fn shells_report_typed_commands() {
        let mut shells: Vec<(String, Vec<String>)> = Vec::new();
        let bash = if cfg!(windows) {
            crate::shell::git_bash()
        } else {
            Some(std::path::PathBuf::from("/bin/bash")).filter(|p| p.exists())
        };
        if let Some(bash) = bash {
            shells.push((
                bash.to_string_lossy().into_owned(),
                vec!["--norc".into(), "--noprofile".into(), "-i".into()],
            ));
        }
        if cfg!(windows) {
            shells.push(("powershell.exe".into(), Vec::new()));
        }
        for (program, args) in shells {
            let (tx, rx) = mpsc::channel();
            let session = Session::spawn(
                SessionConfig {
                    source: crate::Source::Local,
                    program: Some(program.clone()),
                    args,
                    cwd: None,
                    size: GridSize {
                        cols: 100,
                        rows: 24,
                        cell_width: 8,
                        cell_height: 16,
                    },
                    scrollback: 100,
                    env: Vec::new(),
                    colors: None,
                    cursor: CursorShape::Block,
                    triggers: None,
                },
                move |event| {
                    let _ = tx.send(event);
                },
            )
            .expect("spawn");
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut typed = false;
            let mut reported = None;
            while Instant::now() < deadline && reported.is_none() {
                // Typed once the first prompt is up, as a person would.
                if !typed && !session.all_text().trim().is_empty() {
                    std::thread::sleep(Duration::from_millis(1000));
                    session.write(b"echo  macaroni-history\r");
                    typed = true;
                }
                if let Ok(SessionEvent::CommandStarted(command)) =
                    rx.recv_timeout(Duration::from_millis(100))
                {
                    reported = Some(command);
                }
            }
            session.write(b"exit\r");
            assert_eq!(
                reported.as_deref(),
                Some("echo  macaroni-history"),
                "{program} (typed: {typed}):\n{}",
                session.all_text()
            );
        }
    }
}

//! Where a [`crate::Session`]'s bytes come from: a program in a local PTY, a shell on another
//! machine over SSH ([`crate::ssh`]), or a tmux pane in control mode ([`crate::tmux`]).
//!
//! Every source opens to the same four parts ([`Opened`]), so the session's threads don't know
//! which they talk to: a blocking reader of the program's output, a writer for its input, a
//! [`Control`] that resizes it and ends it on drop, and a waiter that blocks until it exits.
//! A source that isn't a local process (SSH) runs on tokio and bridges into these through
//! [`crate::pipe`].

use std::io::{self, Read, Write};
use std::path::PathBuf;

use portable_pty::{ChildKiller, MasterPty, PtySize, native_pty_system};

use crate::grid::GridSize;
use crate::ssh::SshTarget;
use crate::tmux::TmuxPane;

/// What a session runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Source {
    /// A local program in a PTY: [`crate::SessionConfig::program`] (else the default shell),
    /// with its arguments, directory and environment.
    #[default]
    Local,
    /// A login shell on another machine, over macaroni's own SSH client.
    Ssh(SshTarget),
    /// A pane of a tmux session in control mode, carried by another session ([`crate::tmux`]).
    Tmux(TmuxPane),
}

/// An open source, split into what the session's threads use.
pub(crate) struct Opened {
    /// The program's output; reads block until there is some, and return 0 or an error once
    /// it's gone.
    pub reader: Box<dyn Read + Send>,
    /// The program's input.
    pub writer: Box<dyn Write + Send>,
    /// Resizes the program's terminal; dropping it ends the program.
    pub control: Box<dyn Control>,
    /// Blocks until the program exits, with its exit code when it has one. Run on its own
    /// thread.
    pub waiter: Box<dyn FnOnce() -> Option<u32> + Send>,
}

/// What a session keeps of its source to steer it. Dropping it ends the program.
pub(crate) trait Control: Send {
    /// Tells the program its terminal is now `size`.
    fn resize(&self, size: GridSize);
}

/// What a local program needs beyond its size, from [`crate::SessionConfig`].
pub(crate) struct LocalCommand<'a> {
    /// The program; `None` for the default shell.
    pub program: Option<&'a str>,
    /// Its arguments.
    pub args: &'a [String],
    /// Its starting directory.
    pub cwd: Option<&'a PathBuf>,
    /// Environment variables, after the `TERM` family.
    pub env: &'a [(String, String)],
}

/// Opens a PTY of `size` and starts the program in it, with `TERM=xterm-256color`,
/// `COLORTERM=truecolor` and `TERM_PROGRAM=macaroni` before `command.env`.
pub(crate) fn open_local(command: &LocalCommand<'_>, size: GridSize) -> io::Result<Opened> {
    let pair = native_pty_system()
        .openpty(pty_size(size))
        .map_err(io::Error::other)?;
    let mut builder = crate::shell::command(command.program, command.args);
    if let Some(cwd) = command.cwd {
        builder.cwd(cwd);
    }
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    builder.env("TERM_PROGRAM", "macaroni");
    for (key, value) in command.env {
        builder.env(key, value);
    }
    let mut child = pair
        .slave
        .spawn_command(builder)
        .map_err(io::Error::other)?;
    // Only the child should hold the slave side; our copy would keep the PTY open after the
    // child exits.
    drop(pair.slave);
    let killer = child.clone_killer();
    let reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
    let writer = pair.master.take_writer().map_err(io::Error::other)?;
    Ok(Opened {
        reader,
        writer,
        control: Box::new(LocalControl {
            master: pair.master,
            killer: std::sync::Mutex::new(killer),
        }),
        // ConPTY keeps the output pipe open after the child exits, so the reader never sees EOF
        // until the pseudoconsole is closed. Exit is detected by waiting on the child instead.
        waiter: Box::new(move || child.wait().ok().map(|status| status.exit_code())),
    })
}

/// A local program's PTY and a way to kill it.
struct LocalControl {
    /// The PTY master, kept to resize it. Dropping it closes the pseudoconsole, which is what
    /// finally ends the reader thread on Windows.
    master: Box<dyn MasterPty + Send>,
    /// Kills the child; the child itself is in the waiter.
    killer: std::sync::Mutex<Box<dyn ChildKiller + Send + Sync>>,
}

impl Control for LocalControl {
    fn resize(&self, size: GridSize) {
        let _ = self.master.resize(pty_size(size));
    }
}

impl Drop for LocalControl {
    fn drop(&mut self) {
        // Runs before the fields drop. Killing the child ends the waiter thread; dropping
        // `master` afterwards closes the pseudoconsole, which ends the reader thread, which in
        // turn ends the sync thread. Nothing is joined.
        if let Ok(killer) = self.killer.get_mut() {
            let _ = killer.kill();
        }
    }
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

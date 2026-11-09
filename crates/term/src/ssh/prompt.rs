//! The tab's terminal while an SSH connection is being made: progress, errors and questions
//! (host keys, passphrases, passwords, keyboard-interactive prompts) are printed into it, and
//! answers are read from what's typed there, the way OpenSSH asks in its own terminal.
//!
//! Typing that comes before the shell is up and isn't an answer is kept and sent to the shell
//! once it starts, as typeahead.

use std::sync::mpsc;

use tokio::sync::{mpsc::UnboundedReceiver, oneshot};

use crate::pipe::Input;

/// A session's two ends, as the connection task sees them.
pub(crate) struct Term {
    /// Into the emulator.
    out: mpsc::Sender<Vec<u8>>,
    /// From the keyboard (and the emulator's replies), plus resizes.
    input: UnboundedReceiver<Input>,
    /// Fires when the session is dropped.
    kill: oneshot::Receiver<()>,
    /// The session was dropped: every question gets no answer.
    killed: bool,
    /// The terminal's size, in columns and rows.
    pub size: (u16, u16),
    /// Typed bytes not taken as an answer, for the shell.
    pub typeahead: Vec<u8>,
}

/// What [`Term::next`] got.
pub(crate) enum Next {
    /// Typed bytes.
    Data(Vec<u8>),
    /// The terminal changed size (already in [`Term::size`]).
    Resized,
    /// The session is gone.
    Closed,
}

impl Term {
    /// The ends of a session of `size`.
    pub(crate) fn new(
        out: mpsc::Sender<Vec<u8>>,
        input: UnboundedReceiver<Input>,
        kill: oneshot::Receiver<()>,
        size: (u16, u16),
    ) -> Self {
        Self {
            out,
            input,
            kill,
            killed: false,
            size,
            typeahead: Vec::new(),
        }
    }

    /// Sends bytes to the emulator as the program's output.
    pub(crate) fn output(&self, bytes: &[u8]) {
        let _ = self.out.send(bytes.to_vec());
    }

    /// Prints `text`, with each newline a CRLF, since there's no remote terminal to translate.
    pub(crate) fn print(&self, text: &str) {
        self.output(text.replace("\r\n", "\n").replace('\n', "\r\n").as_bytes());
    }

    /// Prints `text` dimmed, for macaroni's own progress lines.
    pub(crate) fn note(&self, text: &str) {
        self.print(&format!("\x1b[2m{text}\x1b[0m\n"));
    }

    /// Waits for typing, a resize or the end of the session.
    pub(crate) async fn next(&mut self) -> Next {
        if self.killed {
            return Next::Closed;
        }
        tokio::select! {
            input = self.input.recv() => match input {
                Some(Input::Data(bytes)) => Next::Data(bytes),
                Some(Input::Resize(cols, rows)) => {
                    self.size = (cols, rows);
                    Next::Resized
                }
                None => {
                    self.killed = true;
                    Next::Closed
                }
            },
            _ = &mut self.kill => {
                self.killed = true;
                Next::Closed
            }
        }
    }

    /// Prints `prompt` and reads a line: shown as typed when `echo`, hidden otherwise (a
    /// password). Backspace edits; Ctrl+C or Ctrl+D, or the session closing, gives `None`.
    /// Escape sequences (arrow keys, the emulator's replies) are skipped.
    pub(crate) async fn ask(&mut self, prompt: &str, echo: bool) -> Option<String> {
        self.print(prompt);
        let mut line: Vec<u8> = Vec::new();
        let mut escape = false;
        // Typing that came before the question answers it first.
        let mut pending = std::mem::take(&mut self.typeahead);
        loop {
            if pending.is_empty() {
                match self.next().await {
                    Next::Data(bytes) => pending = bytes,
                    Next::Resized => continue,
                    Next::Closed => return None,
                }
            }
            let bytes = std::mem::take(&mut pending);
            for (i, &byte) in bytes.iter().enumerate() {
                if escape {
                    // CSI and SS3 sequences end on a letter (or `~`); a lone ESC ends there.
                    if byte.is_ascii_alphabetic() || byte == b'~' {
                        escape = false;
                    }
                    continue;
                }
                match byte {
                    0x1b => escape = true,
                    b'\r' | b'\n' => {
                        self.print("\n");
                        self.typeahead = bytes[i + 1..].to_vec();
                        return Some(String::from_utf8_lossy(&line).into_owned());
                    }
                    0x03 | 0x04 => {
                        self.print("\n");
                        return None;
                    }
                    0x7f | 0x08 => {
                        // Back to the start of the last character, UTF-8 continuation bytes and all.
                        while let Some(last) = line.pop() {
                            if last & 0xC0 != 0x80 {
                                break;
                            }
                        }
                        if echo {
                            self.output(b"\x08 \x08");
                        }
                    }
                    byte if byte < 0x20 => {}
                    byte => {
                        line.push(byte);
                        if echo {
                            self.output(&[byte]);
                        }
                    }
                }
            }
        }
    }

    /// Asks a yes or no question until it's answered; `false` for no or no answer.
    pub(crate) async fn confirm(&mut self, prompt: &str) -> bool {
        loop {
            match self.ask(prompt, true).await {
                None => return false,
                Some(answer) => match answer.trim().to_ascii_lowercase().as_str() {
                    "yes" => return true,
                    "no" => return false,
                    _ => self.print("Please type 'yes' or 'no'.\n"),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A term fed `typed`, its output collected on the returned receiver.
    fn term(typed: &[&[u8]]) -> (Term, mpsc::Receiver<Vec<u8>>) {
        let (out_tx, out_rx) = mpsc::channel();
        let (in_tx, in_rx) = tokio::sync::mpsc::unbounded_channel();
        for chunk in typed {
            in_tx.send(Input::Data(chunk.to_vec())).unwrap();
        }
        // Kept open, so the term doesn't see the session as closed.
        std::mem::forget(in_tx);
        let (kill_tx, kill_rx) = oneshot::channel();
        std::mem::forget(kill_tx);
        (Term::new(out_tx, in_rx, kill_rx, (80, 24)), out_rx)
    }

    #[test]
    fn reads_hidden_answers_with_backspace_and_keeps_typeahead() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (mut t, out) = term(&[b"sex\x7fcr", b"\x1b[Aet\rls\r"]);
        let answer = runtime.block_on(t.ask("Password: ", false));
        assert_eq!(answer.as_deref(), Some("secret"));
        assert_eq!(t.typeahead, b"ls\r");
        let shown: Vec<u8> = out.try_iter().flatten().collect();
        assert_eq!(String::from_utf8_lossy(&shown), "Password: \r\n");

        let (mut t, _) = term(&[b"maybe\r", b"yes\r"]);
        assert!(runtime.block_on(t.confirm("Continue? ")));
        let (mut t, _) = term(&[b"\x03"]);
        assert_eq!(runtime.block_on(t.ask("Password: ", false)), None);
    }
}

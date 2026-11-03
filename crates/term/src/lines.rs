//! [`Lines`]: turns a program's output back into lines of plain text, for triggers to match.
//!
//! It reads the byte stream, not the grid: a line is whatever came before a newline, with
//! escape sequences taken out. Two things keep that close to what's on screen. A carriage return
//! not followed by a newline starts the line over, as a progress bar redrawing itself does.
//! ConPTY (Windows) writes runs of spaces as cursor-forward (`CSI n C`) and moves to a new line
//! with cursor positioning (`CSI H`) as often as with a newline, so the first becomes spaces and
//! the second ends the line.
//!
//! With shell integration (OSC 133), the session marks the prompt and the command typed at it
//! with [`Lines::set_quiet`], and those lines aren't handed over: a trigger is about what
//! commands print, not about the command that was typed to print it.

/// Longest line kept, in bytes; anything past it is dropped until the line ends.
const MAX_LINE: usize = 4096;

/// Where the splitter is in an escape sequence.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Plain text.
    #[default]
    Ground,
    /// After `ESC`.
    Escape,
    /// In a CSI sequence, collecting its parameter bytes.
    Csi,
    /// In a string sequence (OSC, DCS, APC, PM, SOS), until BEL or `ESC \`.
    String,
    /// After `ESC` inside a string sequence.
    StringEscape,
}

/// Splits output into lines of plain text. Feed it every chunk read, in order.
#[derive(Debug, Default)]
pub struct Lines {
    /// The line so far, as bytes (decoded when it ends, so a character split across reads
    /// survives).
    line: Vec<u8>,
    /// The escape sequence state.
    state: State,
    /// A CSI sequence's parameter bytes.
    params: Vec<u8>,
    /// A carriage return came last: the next byte decides whether it ends the line (`\n`) or
    /// starts it over.
    cr: bool,
    /// Lines that end now are the prompt or the command typed at it, and are dropped.
    quiet: bool,
}

impl Lines {
    /// Drops the lines that end from now on (`true`), or hands them over again.
    pub fn set_quiet(&mut self, quiet: bool) {
        self.quiet = quiet;
    }

    /// Reads `bytes` and calls `line` with each line they finish, without its line ending.
    /// Blank lines are skipped.
    pub fn feed(&mut self, bytes: &[u8], mut line: impl FnMut(&str)) {
        for &b in bytes {
            match self.state {
                State::Ground => self.ground(b, &mut line),
                State::Escape => {
                    self.state = match b {
                        b'[' => {
                            self.params.clear();
                            State::Csi
                        }
                        b']' | b'P' | b'_' | b'^' | b'X' => State::String,
                        _ => State::Ground,
                    };
                }
                State::Csi => {
                    if (0x40..=0x7e).contains(&b) {
                        self.state = State::Ground;
                        self.csi(b, &mut line);
                    } else {
                        self.params.push(b);
                    }
                }
                State::String => match b {
                    0x07 => self.state = State::Ground,
                    0x1b => self.state = State::StringEscape,
                    _ => {}
                },
                State::StringEscape => {
                    self.state = if b == b'\\' {
                        State::Ground
                    } else {
                        State::String
                    };
                }
            }
        }
    }

    /// One byte of plain text.
    fn ground(&mut self, b: u8, line: &mut impl FnMut(&str)) {
        if std::mem::take(&mut self.cr) && b != b'\n' {
            self.line.clear();
        }
        match b {
            b'\n' => self.end(line),
            b'\r' => self.cr = true,
            0x1b => self.state = State::Escape,
            0x08 => {
                self.line.pop();
            }
            b'\t' => self.push(b' '),
            0..0x20 | 0x7f => {}
            _ => self.push(b),
        }
    }

    /// The end of a CSI sequence, with `last` its final byte.
    fn csi(&mut self, last: u8, line: &mut impl FnMut(&str)) {
        match last {
            b'C' => {
                let n = std::str::from_utf8(&self.params)
                    .ok()
                    .and_then(|p| p.parse::<usize>().ok())
                    .unwrap_or(1)
                    .min(MAX_LINE);
                for _ in 0..n {
                    self.push(b' ');
                }
            }
            b'H' | b'f' => self.end(line),
            _ => {}
        }
    }

    /// Adds a byte to the line, unless it's already at [`MAX_LINE`].
    fn push(&mut self, b: u8) {
        if self.line.len() < MAX_LINE {
            self.line.push(b);
        }
    }

    /// Ends the line: hands it over unless blank, and starts the next.
    fn end(&mut self, line: &mut impl FnMut(&str)) {
        let text = String::from_utf8_lossy(&self.line);
        let text = text.trim_end();
        if !self.quiet && !text.trim_start().is_empty() {
            line(text);
        }
        self.line.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every line `chunks` finish, fed in order.
    fn lines(chunks: &[&[u8]]) -> Vec<String> {
        let mut splitter = Lines::default();
        let mut out = Vec::new();
        for chunk in chunks {
            splitter.feed(chunk, |l| out.push(l.to_owned()));
        }
        out
    }

    #[test]
    fn splits_lines_without_escapes() {
        assert_eq!(
            lines(&[b"\x1b[31mred\x1b[0m text\r\nnext\n\npartial"]),
            ["red text", "next"]
        );
    }

    #[test]
    fn strings_and_titles_are_dropped() {
        assert_eq!(
            lines(&[b"\x1b]0;title\x07a\x1b]8;;http://x\x1b\\b\x1b]8;;\x1b\\c\n"]),
            ["abc"]
        );
    }

    #[test]
    fn carriage_return_starts_the_line_over() {
        assert_eq!(lines(&[b"10%\r50%\r100%\r\n"]), ["100%"]);
    }

    #[test]
    fn conpty_spacing_and_positioning() {
        assert_eq!(
            lines(&[b"a\x1b[3Cb\x1b[5;1Hc\x1b[Cd\r\n"]),
            ["a   b", "c d"]
        );
    }

    #[test]
    fn quiet_lines_are_dropped() {
        let mut splitter = Lines::default();
        let mut out = Vec::new();
        splitter.set_quiet(true);
        splitter.feed(b"$ echo hi\r\n", |l| out.push(l.to_owned()));
        splitter.set_quiet(false);
        splitter.feed(b"hi\r\n", |l| out.push(l.to_owned()));
        assert_eq!(out, ["hi"]);
    }

    #[test]
    fn split_reads_and_characters_join() {
        let text = "caf\u{e9}\n".as_bytes();
        assert_eq!(
            lines(&[&text[..4], &text[4..], b"\x1b[3", b"1mx\n"]),
            ["caf\u{e9}", "x"]
        );
    }
}

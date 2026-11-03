//! Finds the escape sequences alacritty doesn't handle in a session's output: notifications,
//! the working directory, prompt marks and inline images.
//!
//! `alacritty_terminal` ignores these, so the reader thread runs every chunk through this small
//! parser before feeding the emulator. The parser keeps its state between chunks, so a sequence
//! split across two reads is still found.
//!
//! - OSC 9 (iTerm2): `ESC ] 9 ; <message> BEL`. ConEmu reuses OSC 9 with a numeric first field
//!   (`9;4;...` is progress), so those are skipped.
//! - OSC 777 (rxvt-unicode, Ghostty): `ESC ] 777 ; notify ; <title> ; <body> BEL`.
//! - OSC 99 (kitty, basic): `ESC ] 99 ; <metadata> ; <payload> ST`, with `p=title|body` and
//!   `d=0` chunks accumulated until the final one. Other kitty options are ignored.
//! - OSC 7: `ESC ] 7 ; file://<host>/<path> BEL`, the shell's working directory.
//! - OSC 133 (FinalTerm prompt marks): `A` a prompt starts, `B` the command line starts, `C` the
//!   command's output starts, `D[;<exit code>]` the command finished. [`OscSniffer::feed_at`]
//!   says where in the chunk each one ended, so the reader can mark that spot in the grid.
//! - OSC 1337 `File=...:<base64>` (iTerm2): an inline image ([`crate::images`]), placed where
//!   it ended in the chunk the same way.
//!
//! The state machine is vte's own [`Parser`]; [`Collector`] only implements `osc_dispatch`, so
//! everything else (text, CSI, DCS) passes through it and is ignored. The one piece of state of
//! ours is a pending kitty notification ([`KittyPending`]) waiting for its final chunk. The
//! reader thread in `session.rs` turns each [`OscEvent`] into a
//! [`SessionEvent`](crate::SessionEvent); BEL is not handled here but by the emulator
//! ([`EmulatorEvent::Bell`](crate::EmulatorEvent::Bell)).

use alacritty_terminal::vte::{Parser, Perform};

/// Longest title or body kept, in bytes. Output is untrusted, so a runaway sequence can't make a
/// huge toast or sidebar line.
const MAX_LEN: usize = 1024;

/// A request for the user's attention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// The heading, when the sequence carried one separately from the body.
    pub title: Option<String>,
    /// The message. Never empty: a title with no body is moved here.
    pub body: String,
}

/// Something found in the output.
#[derive(Debug, Clone, PartialEq)]
pub enum OscEvent {
    /// A notification (OSC 9, 777 or 99).
    Notify(Notification),
    /// The shell reported its working directory (OSC 7), as a local path.
    Cwd(String),
    /// A shell integration mark (OSC 133).
    Mark(Mark),
    /// An image to show inline (OSC 1337 `File=`).
    Image(crate::images::ImageRequest),
}

/// An OSC 133 prompt mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// `A`: the prompt starts here.
    Prompt,
    /// `B`: the prompt ends and the command line starts.
    Command,
    /// `C`: the command runs; its output starts here.
    Output,
    /// `D`: the command finished, with its exit code when the shell gave one.
    Finished(Option<i32>),
}

/// Watches a byte stream for notification and directory sequences. One per session; feed it
/// every chunk, in order, before the emulator sees it.
#[derive(Default)]
pub struct OscSniffer {
    /// vte's escape sequence state machine. It survives between [`OscSniffer::feed`] calls,
    /// which is what makes split sequences work.
    parser: Parser,
    /// Receives the parsed OSC sequences.
    collector: Collector,
}

/// The [`Perform`] side of the sniffer: turns dispatched OSC sequences into [`OscEvent`]s.
#[derive(Default)]
struct Collector {
    /// Events found during the current [`OscSniffer::feed`], drained at its end.
    found: Vec<OscEvent>,
    /// A kitty notification sent in chunks (`d=0`) whose last chunk hasn't arrived yet.
    kitty: Option<KittyPending>,
}

/// The parts of a chunked kitty notification received so far.
#[derive(Default)]
struct KittyPending {
    /// The notification's `i=` id; a chunk with a different id starts over.
    id: String,
    /// Title text accumulated from `p=title` chunks (the default payload).
    title: String,
    /// Body text accumulated from `p=body` chunks.
    body: String,
}

impl OscSniffer {
    /// Parses `bytes` and returns the events completed in them. A sequence that starts here and
    /// ends in a later chunk is returned by that later call.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<OscEvent> {
        self.feed_at(bytes).into_iter().map(|(_, e)| e).collect()
    }

    /// [`OscSniffer::feed`], with each event's position: the offset in `bytes` just past the
    /// sequence's terminator.
    ///
    /// OSC sequences end in BEL or ST (`ESC \`), so the bytes are parsed in runs ending at each
    /// BEL or backslash, and whatever completed in a run ended at its last byte. That's exact,
    /// and costs one more `advance` per BEL or backslash in the output.
    pub fn feed_at(&mut self, bytes: &[u8]) -> Vec<(usize, OscEvent)> {
        let mut events = Vec::new();
        let mut start = 0;
        while start < bytes.len() {
            let end = bytes[start..]
                .iter()
                .position(|&b| b == 0x07 || b == b'\\')
                .map_or(bytes.len(), |i| start + i + 1);
            self.parser.advance(&mut self.collector, &bytes[start..end]);
            events.extend(self.collector.found.drain(..).map(|e| (end, e)));
            start = end;
        }
        events
    }
}

/// Joins OSC parameters back into one string and cleans it for display.
///
/// vte splits the payload on every `;`, so a message containing `;` arrives as several parts;
/// rejoining restores it. Invalid UTF-8 is replaced, control characters are dropped, and the
/// result is cut to [`MAX_LEN`] bytes at a character boundary.
fn text(parts: &[&[u8]]) -> String {
    let joined = parts.join(&b';');
    let mut s: String = String::from_utf8_lossy(&joined)
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    if s.len() > MAX_LEN {
        let mut end = MAX_LEN;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

impl Collector {
    /// Records a notification after trimming both parts. A title with no body becomes the body
    /// (so the body is never empty), and one with neither is dropped.
    fn push(&mut self, title: String, body: String) {
        let (title, body) = (title.trim().to_owned(), body.trim().to_owned());
        let (title, body) = match (title.is_empty(), body.is_empty()) {
            (_, false) => ((!title.is_empty()).then_some(title), body),
            (false, true) => (None, title),
            (true, true) => return,
        };
        self.found
            .push(OscEvent::Notify(Notification { title, body }));
    }

    /// Handles one OSC 99 chunk: `metadata` is the `key=value:key=value` field, `payload` the
    /// rest (rejoined on `;`).
    ///
    /// Chunks with the same `i=` id accumulate into [`Collector::kitty`] while `d=0`; the first
    /// chunk without `d=0` completes the notification. A chunk for a payload type other than
    /// `title` or `body` (icons, buttons and so on) is ignored and leaves any pending
    /// notification alone.
    fn kitty(&mut self, metadata: &[u8], payload: &[&[u8]]) {
        let metadata = String::from_utf8_lossy(metadata);
        let mut id = String::new();
        let mut done = true;
        let mut is_body = false;
        for pair in metadata.split(':') {
            match pair.split_once('=') {
                Some(("i", v)) => id = v.to_owned(),
                Some(("d", "0")) => done = false,
                Some(("p", "body")) => is_body = true,
                Some(("p", "title")) => is_body = false,
                Some(("p", _)) => return,
                _ => {}
            }
        }
        // Only one notification is tracked at a time: a chunk with another id discards the
        // unfinished one rather than mixing their text.
        let pending = match self.kitty.take() {
            Some(p) if p.id == id => p,
            _ => KittyPending {
                id,
                ..KittyPending::default()
            },
        };
        let mut pending = pending;
        let chunk = text(payload);
        if is_body {
            pending.body.push_str(&chunk);
        } else {
            pending.title.push_str(&chunk);
        }
        if done {
            self.push(pending.title, pending.body);
        } else {
            self.kitty = Some(pending);
        }
    }
}

impl Perform for Collector {
    // Called by vte once per complete OSC sequence (BEL or ST terminated), with the payload
    // already split on `;`. params[0] is the OSC number.
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        match params {
            // A numeric first field is ConEmu's OSC 9 extension (progress, cwd, ...), not a
            // message, and would otherwise raise a notification on every progress update.
            [b"9", rest @ ..] if !rest.is_empty() => {
                let first = String::from_utf8_lossy(rest[0]);
                if first.trim().parse::<u32>().is_ok() {
                    return;
                }
                self.push(String::new(), text(rest));
            }
            [b"777", b"notify", title, body @ ..] => self.push(text(&[title]), text(body)),
            [b"99", metadata, payload @ ..] => self.kitty(metadata, payload),
            [b"133", kind, rest @ ..] => {
                let mark = match *kind {
                    b"A" => Mark::Prompt,
                    b"B" => Mark::Command,
                    b"C" => Mark::Output,
                    b"D" => Mark::Finished(
                        rest.first()
                            .and_then(|code| std::str::from_utf8(code).ok()?.trim().parse().ok()),
                    ),
                    _ => return,
                };
                self.found.push(OscEvent::Mark(mark));
            }
            [b"1337", rest @ ..] if !rest.is_empty() => {
                if let Some(request) = crate::images::ImageRequest::parse(&rest.join(&b';')) {
                    self.found.push(OscEvent::Image(request));
                }
            }
            [b"7", rest @ ..] if !rest.is_empty() => {
                if let Some(path) = file_url_path(&String::from_utf8_lossy(&rest.join(&b';'))) {
                    self.found.push(OscEvent::Cwd(path));
                }
            }
            _ => {}
        }
    }
}

/// The local path in a `file://host/path` URL, percent-decoded. Windows drive paths come out as
/// `C:/...`.
///
/// The host is ignored. Returns `None` for anything that isn't a `file://` URL with a path, a
/// path that doesn't decode to UTF-8, or one containing control characters.
fn file_url_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    // Malformed escapes (`%zz`, a trailing `%`) are kept literally rather than rejecting the
    // whole path.
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = path.get(i + 1..i + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            decoded.push(byte);
            i += 3;
            continue;
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    let path = String::from_utf8(decoded).ok()?;
    // `file:///C:/x` yields `/C:/x`; strip the slash so Windows sees a drive path.
    let windows_drive = path.len() > 2
        && path.as_bytes()[0] == b'/'
        && path.as_bytes()[1].is_ascii_alphabetic()
        && path.as_bytes()[2] == b':';
    let path = if windows_drive { &path[1..] } else { &path[..] };
    (!path.is_empty() && !path.chars().any(char::is_control)).then(|| path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(title: Option<&str>, body: &str) -> OscEvent {
        OscEvent::Notify(Notification {
            title: title.map(String::from),
            body: body.to_owned(),
        })
    }

    #[test]
    fn osc_7_working_directory() {
        let mut s = OscSniffer::default();
        assert_eq!(
            s.feed(b"\x1b]7;file://box/home/me/my%20project\x07"),
            vec![OscEvent::Cwd("/home/me/my project".into())]
        );
        assert_eq!(
            s.feed(b"\x1b]7;file://box/C:/Users/me\x1b\\"),
            vec![OscEvent::Cwd("C:/Users/me".into())]
        );
        assert!(s.feed(b"\x1b]7;https://example.com/x\x07").is_empty());
    }

    #[test]
    fn osc_133_marks_and_where_they_end() {
        let mut s = OscSniffer::default();
        let out = b"\x1b]133;A\x07$ ls\x1b]133;C\x1b\\file\n\x1b]133;D;2\x07";
        let found = s.feed_at(out);
        assert_eq!(
            found,
            vec![
                (8, OscEvent::Mark(Mark::Prompt)),
                (21, OscEvent::Mark(Mark::Output)),
                (36, OscEvent::Mark(Mark::Finished(Some(2)))),
            ]
        );
        assert_eq!(&out[found[1].0..found[1].0 + 4], b"file");
        assert_eq!(
            s.feed(b"\x1b]133;D\x07\x1b]133;B\x07"),
            vec![
                OscEvent::Mark(Mark::Finished(None)),
                OscEvent::Mark(Mark::Command)
            ]
        );
    }

    #[test]
    fn osc_9() {
        let mut s = OscSniffer::default();
        assert_eq!(
            s.feed(b"hi\x1b]9;Build finished; 3 warnings\x07there"),
            vec![note(None, "Build finished; 3 warnings")]
        );
    }

    #[test]
    fn conemu_progress_is_not_a_notification() {
        let mut s = OscSniffer::default();
        assert!(s.feed(b"\x1b]9;4;1;50\x07\x1b]9;9;C:\\\\x\x07").is_empty());
    }

    #[test]
    fn osc_777() {
        let mut s = OscSniffer::default();
        assert_eq!(
            s.feed(b"\x1b]777;notify;Claude Code;Waiting for input\x1b\\"),
            vec![note(Some("Claude Code"), "Waiting for input")]
        );
        assert_eq!(
            s.feed(b"\x1b]777;notify;Only a title;\x07"),
            vec![note(None, "Only a title")]
        );
    }

    #[test]
    fn sequences_split_across_reads() {
        let mut s = OscSniffer::default();
        assert!(s.feed(b"\x1b]777;noti").is_empty());
        assert!(s.feed(b"fy;Tests;all pa").is_empty());
        assert_eq!(s.feed(b"ssed\x07"), vec![note(Some("Tests"), "all passed")]);
    }

    #[test]
    fn kitty_title_and_chunked_body() {
        let mut s = OscSniffer::default();
        assert_eq!(s.feed(b"\x1b]99;;Hello\x1b\\"), vec![note(None, "Hello")]);
        assert!(s.feed(b"\x1b]99;i=1:d=0;Deploy\x1b\\").is_empty());
        assert!(s.feed(b"\x1b]99;i=1:d=0:p=body;done in \x1b\\").is_empty());
        assert_eq!(
            s.feed(b"\x1b]99;i=1:p=body;4s\x1b\\"),
            vec![note(Some("Deploy"), "done in 4s")]
        );
    }

    #[test]
    fn control_characters_and_empty_messages_are_dropped() {
        let mut s = OscSniffer::default();
        assert!(s.feed(b"\x1b]9;   \x07").is_empty());
        assert_eq!(s.feed(b"\x1b]9;a\x08b\x07"), vec![note(None, "ab")]);
    }

    #[test]
    fn other_osc_sequences_are_ignored() {
        let mut s = OscSniffer::default();
        assert!(
            s.feed(b"\x1b]0;title\x07\x1b]8;;http://x\x07link\x1b]8;;\x07")
                .is_empty()
        );
    }
}

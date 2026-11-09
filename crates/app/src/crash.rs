//! A record of the last crash: a GUI app has no console, so a panic's message would otherwise
//! go nowhere. [`log_panics`] writes each panic to `crash.log` in the local data folder with
//! the trail of what the app did just before ([`note`], [`note_message`]): the last
//! [`TRAIL_LEN`] messages it handled, with typed text, paths and keystrokes blanked out.
//!
//! The first panic of a run replaces the file, so it always holds the most recent crash; later
//! panics in the same run (another thread's, then the main thread's) are added after it.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use macaroni_term::SessionEvent;

use crate::message::Message;
use crate::terminal::ViewEvent;

/// How many recent events the trail keeps.
const TRAIL_LEN: usize = 300;

/// The longest a trail line gets, in characters; the rest is cut.
const MAX_LINE: usize = 240;

/// The recent events, oldest first, each with the seconds since [`log_panics`] ran.
static TRAIL: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// When the run started, for the trail's times.
static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Set once this run has written `crash.log`, so later panics add to it instead of replacing it.
static WROTE: AtomicBool = AtomicBool::new(false);

/// Starts the trail with the version and system, and makes every panic (its thread, message,
/// place and a backtrace, then the trail) replace `local_data("crash.log")`. The default hook
/// still runs after, printing it as before.
pub fn log_panics() {
    STARTED.get_or_init(Instant::now);
    note(format!(
        "macaroni {} started on {} {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    ));
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write_crash(&info.to_string());
        default(info);
    }));
}

/// Adds `line` to the trail, dropping the oldest past [`TRAIL_LEN`].
pub fn note(line: impl Into<String>) {
    let seconds = STARTED.get().map_or(0.0, |s| s.elapsed().as_secs_f64());
    let mut line: String = line.into();
    if let Some((cut, _)) = line.char_indices().nth(MAX_LINE) {
        line.truncate(cut);
        line.push('…');
    }
    let mut trail = TRAIL.lock().unwrap_or_else(|e| e.into_inner());
    if trail.len() == TRAIL_LEN {
        trail.pop_front();
    }
    trail.push_back(format!("[{seconds:9.3}s] {line}"));
}

/// Adds `message` to the trail, unless it's one that comes many times a second (frames, ticks,
/// hovering, output wakeups, keystrokes). Its text and bytes are blanked by [`redact`].
pub fn note_message(message: &Message) {
    let noisy = matches!(
        message,
        Message::Frame
            | Message::DragMove(..)
            | Message::DragTick
            | Message::ItemDragMove(..)
            | Message::TabHover(..)
            | Message::SidebarHover(..)
            | Message::MenuHover(..)
            | Message::ReaderTick
            | Message::ReaderBounds(..)
            | Message::SaveTick
            | Message::GitTick
            | Message::AgentTick
            | Message::FileTick
            | Message::PluginTick
            | Message::TmuxBounds(..)
            | Message::BrowserBounds(..)
            | Message::WindowMoved(..)
            | Message::SidebarZone(..)
            | Message::TabZone(..)
            | Message::TabViewport(..)
            | Message::Key(..)
            | Message::Session(_, SessionEvent::Wakeup)
            | Message::View(_, ViewEvent::Write(_) | ViewEvent::Scroll(_))
    );
    if !noisy {
        note(redact(&format!("{message:?}")));
    }
}

/// `debug` with every quoted string and character shown as `"…"` and every list of numbers as
/// `[…]`: those hold what was typed or pasted, titles, paths and output, which a crash report
/// shouldn't carry and doesn't need.
fn redact(debug: &str) -> String {
    let mut out = String::with_capacity(debug.len().min(MAX_LINE * 2));
    let mut chars = debug.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' | '\'' => {
                // Up to the matching unescaped quote.
                while let Some(d) = chars.next() {
                    match d {
                        '\\' => {
                            chars.next();
                        }
                        d if d == c => break,
                        _ => {}
                    }
                }
                out.push_str("\"…\"");
            }
            '[' if chars
                .peek()
                .is_some_and(|d| d.is_ascii_digit() || *d == '-') =>
            {
                for d in chars.by_ref() {
                    if d == ']' {
                        break;
                    }
                }
                out.push_str("[…]");
            }
            c => out.push(c),
        }
        if out.len() > MAX_LINE * 4 {
            break;
        }
    }
    out
}

/// Writes a crash with panic text `panic`: the backtrace, then the trail. Best effort: it
/// mustn't panic itself, so failures are ignored, and the trail is skipped if its lock is held
/// (the panic happened while noting).
fn write_crash(panic: &str) {
    let path = crate::browser::local_data("crash.log");
    let thread = std::thread::current();
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut entry = format!(
        "--- panic at unix time {seconds}, macaroni {}, {} {}, thread '{}'\n{panic}\n\n{}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        thread.name().unwrap_or("unnamed"),
        std::backtrace::Backtrace::force_capture(),
    );
    entry.push_str("Recent events, oldest first:\n");
    match TRAIL.try_lock() {
        Ok(trail) => {
            for line in trail.iter() {
                entry.push_str(line);
                entry.push('\n');
            }
        }
        Err(_) => entry.push_str("(unavailable: the panic happened while recording one)\n"),
    }
    entry.push('\n');
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let first = !WROTE.swap(true, Ordering::SeqCst);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(!first)
        .truncate(first)
        .open(&path);
    if let Ok(mut file) = file {
        let _ = file.write_all(entry.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_bytes_are_blanked() {
        assert_eq!(
            redact(r#"Paste(SessionId(3), Some("secret \"x\" here"))"#),
            r#"Paste(SessionId(3), Some("…"))"#
        );
        assert_eq!(
            redact("View(SessionId(1), Write([27, 91, 65]))"),
            "View(SessionId(1), Write([…]))"
        );
        assert_eq!(redact("Char('a')"), "Char(\"…\")");
        assert_eq!(
            redact("Commands([CloseTab(TabId(2))])"),
            "Commands([CloseTab(TabId(2))])"
        );
    }

    #[test]
    fn the_trail_keeps_the_latest() {
        for i in 0..TRAIL_LEN + 5 {
            note(format!("event {i}"));
        }
        let trail = TRAIL.lock().unwrap();
        assert_eq!(trail.len(), TRAIL_LEN);
        assert!(
            trail
                .back()
                .unwrap()
                .ends_with(&format!("event {}", TRAIL_LEN + 4))
        );
        let long = "x".repeat(MAX_LINE * 2);
        drop(trail);
        note(long);
        assert!(TRAIL.lock().unwrap().back().unwrap().ends_with('…'));
    }
}

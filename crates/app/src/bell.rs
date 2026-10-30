//! The terminal bell's sound (`[notifications] bell_sound`): nothing, the system's alert, or a
//! sound file.
//!
//! [`ring`] is called for every BEL a terminal prints, which a program can do in a tight loop,
//! so rings closer together than [`MIN_GAP`] are dropped. Playing never blocks the UI thread:
//! Windows plays asynchronously, and macOS runs `afplay` or `osascript` as a child process that
//! nothing waits for.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The shortest time between two rings; the rest are dropped.
const MIN_GAP: Duration = Duration::from_millis(150);

/// When the bell last rang.
static LAST: Mutex<Option<Instant>> = Mutex::new(None);

/// Plays `sound` (a `bell_sound` value): `"off"` (or empty) plays nothing, `"system"` the
/// system's alert sound, anything else is a sound file's path. A file that can't be played is
/// silently skipped, as a bell should never get in the way.
pub fn ring(sound: &str) {
    let sound = sound.trim();
    if sound.is_empty() || sound == "off" {
        return;
    }
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if last.is_some_and(|at| now - at < MIN_GAP) {
            return;
        }
        *last = Some(now);
    }
    play(sound);
}

/// Plays the system alert, or the file at `sound`, without waiting for it.
#[cfg(windows)]
#[allow(unsafe_code)]
fn play(sound: &str) {
    use windows_sys::Win32::Media::Audio::{
        PlaySoundW, SND_ALIAS, SND_ASYNC, SND_FILENAME, SND_NODEFAULT,
    };

    // ".Default" is the alias of the sound Windows calls Default Beep.
    let (name, kind) = if sound == "system" {
        (".Default", SND_ALIAS)
    } else {
        (sound, SND_FILENAME)
    };
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: `name` is a NUL-terminated UTF-16 string that outlives the call; with SND_ASYNC
    // PlaySoundW copies what it needs before returning. No module handle is needed for a file
    // or an alias.
    unsafe {
        PlaySoundW(
            name.as_ptr(),
            std::ptr::null_mut(),
            SND_ASYNC | SND_NODEFAULT | kind,
        )
    };
}

/// Plays the system alert, or the file at `sound`, without waiting for it.
#[cfg(not(windows))]
fn play(sound: &str) {
    let mut command = if sound == "system" {
        let mut c = std::process::Command::new("osascript");
        c.args(["-e", "beep"]);
        c
    } else {
        let mut c = std::process::Command::new("afplay");
        c.arg(sound);
        c
    };
    // Reaped on a thread so a finished player doesn't linger as a zombie.
    if let Ok(mut child) = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

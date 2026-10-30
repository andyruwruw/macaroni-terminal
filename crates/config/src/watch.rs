//! Live reload: [`watch()`] reports edits to the config directory (`config.toml` and
//! `themes/*.toml`), merging bursts into one callback.
//!
//! The app runs it inside an iced subscription and turns each callback into a reload; a bad file
//! keeps the last good config there. Writes from [`crate::write_setting`] come back through here
//! like any other edit.

use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::{EventKind, RecursiveMode, Watcher as _};

/// Editors often save in several steps (truncate, write, rename), so bursts of events are merged
/// into one callback after this quiet period.
const DEBOUNCE: Duration = Duration::from_millis(150);

/// Keeps watching while alive.
pub struct Watcher {
    /// The OS watcher; dropping it stops events, which ends the debounce thread too (its channel
    /// disconnects).
    _inner: notify::RecommendedWatcher,
}

/// Calls `on_change` after the config file or anything in the themes folder changes.
///
/// Creates `<dir>/themes` if it's missing, so new theme files are seen, and watches `dir`
/// recursively. Spawns a `config-watch` thread that runs `on_change` once per burst of events,
/// after 150 ms of quiet (`DEBOUNCE`). Fails if the directory can't be created or watched, or
/// the thread can't start.
pub fn watch(dir: &Path, on_change: impl Fn() + Send + 'static) -> notify::Result<Watcher> {
    std::fs::create_dir_all(dir.join("themes"))?;

    let (tx, rx) = mpsc::channel::<()>();
    let mut inner = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else {
            return;
        };
        // Access events (and others) are ignored, and so is anything that isn't TOML, such as
        // `session.json`, which the app writes next to `config.toml` about once a second.
        let relevant = matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        ) && event.paths.iter().any(|p| {
            p.file_name()
                .is_some_and(|n| n == crate::settings::FILE_NAME)
                || p.extension().is_some_and(|e| e == "toml")
        });
        if relevant {
            let _ = tx.send(());
        }
    })?;
    inner.watch(dir, RecursiveMode::Recursive)?;

    thread::Builder::new()
        .name("config-watch".into())
        .spawn(move || {
            // Wait for the first event of a burst, then swallow the rest until it goes quiet.
            while rx.recv().is_ok() {
                while rx.recv_timeout(DEBOUNCE).is_ok() {}
                on_change();
            }
        })?;

    Ok(Watcher { _inner: inner })
}

//! OS notifications for agent attention. The app decides when to notify (`notifications.os` on,
//! the tab not muted, and the window unfocused or the tab hidden); this delivers them, and
//! reports a click on one as [`Message::NotificationClicked`] so the app can show its tab.
//!
//! A coding agent's permission request gets Allow and Deny buttons ([`show`]'s `ask`); pressing
//! one is [`Message::PermissionAnswer`], which answers the prompt in the agent's terminal.
//!
//! - Windows: a toast under macaroni's own identity ([`APP_ID`]). [`register`] writes it to
//!   `HKCU\Software\Classes\AppUserModelId\<id>` (a display name and icon), which is all an
//!   unpackaged app needs for its toasts to carry its name; without it they'd have to borrow
//!   PowerShell's. A click while the toast is up reaches [`CLICKS`].
//! - macOS: `mac-notification-sys`, waiting on its own thread for the click. Only the bundled
//!   `.app` gets notifications at all.
//! - Elsewhere: `notify-rust`, without clicks.

use std::sync::Mutex;

use iced::Subscription;
use iced::futures::channel::mpsc::{UnboundedSender, unbounded};
use iced::futures::{SinkExt, Stream, StreamExt};
use macaroni_core::TabId;

use crate::message::Message;

/// macaroni's notification identity: the Windows AppUserModelID, and the macOS bundle id (which
/// `cargo xtask bundle` writes into `Info.plist`).
pub const APP_ID: &str = "dev.macaroni.terminal";

/// What happened to a notification: clicked (show the tab), or one of its permission buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clicked {
    /// The notification itself.
    Show,
    /// "Allow" (`true`) or "Deny".
    Answer(bool),
}

/// Where a clicked notification's tab goes: the app's end of [`clicks`], once it's running.
static CLICKS: Mutex<Option<UnboundedSender<(TabId, Clicked)>>> = Mutex::new(None);

/// The toast argument and button label for Allow and Deny.
const ALLOW: &str = "Allow";
/// See [`ALLOW`].
const DENY: &str = "Deny";

/// Shows an OS notification for `tab` on a background thread; delivery can block, and on macOS
/// the thread waits for a click. `ask` adds Allow and Deny buttons (a permission request).
/// `sound` lets the system play its notification sound; otherwise the notification is silent
/// (the app plays its own, see [`crate::bell::play`]).
pub fn show(title: String, body: String, tab: TabId, ask: bool, sound: bool) {
    std::thread::spawn(move || deliver(&title, &body, tab, ask, sound));
}

/// Hands `tab` and what was clicked to the app.
fn clicked(tab: TabId, what: Clicked) {
    if let Some(sender) = CLICKS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = sender.unbounded_send((tab, what));
    }
}

/// A button's activation argument as what was clicked; anything else is the toast itself.
fn answer(argument: Option<&str>) -> Clicked {
    match argument {
        Some(ALLOW) => Clicked::Answer(true),
        Some(DENY) => Clicked::Answer(false),
        _ => Clicked::Show,
    }
}

/// A Windows toast; a click while it's showing calls [`clicked`].
#[cfg(windows)]
fn deliver(title: &str, body: &str, tab: TabId, ask: bool, sound: bool) {
    let mut toast = tauri_winrt_notification::Toast::new(APP_ID)
        .title(title)
        .text1(body)
        .sound(sound.then_some(tauri_winrt_notification::Sound::Default));
    if ask {
        toast = toast.add_button(ALLOW, ALLOW).add_button(DENY, DENY);
    }
    let _ = toast
        .on_activated(move |argument| {
            clicked(tab, answer(argument.as_deref()));
            Ok(())
        })
        .show();
}

/// A macOS notification, waited on so a click (or, for a permission request, its Allow or Deny
/// action) can be reported.
#[cfg(target_os = "macos")]
fn deliver(title: &str, body: &str, tab: TabId, ask: bool, sound: bool) {
    use mac_notification_sys::{MainButton, NotificationResponse};
    let _ = mac_notification_sys::set_application(APP_ID);
    let mut notification = mac_notification_sys::Notification::new();
    notification.title(title).message(body).wait_for_click(true);
    if sound {
        notification.default_sound();
    }
    if ask {
        notification.main_button(MainButton::DropdownActions("Answer", &[ALLOW, DENY]));
    }
    match notification.send() {
        Ok(NotificationResponse::Click) => clicked(tab, Clicked::Show),
        Ok(NotificationResponse::ActionButton(name)) => clicked(tab, answer(Some(&name))),
        _ => {}
    }
}

/// A notification through `notify-rust`; clicks aren't reported.
#[cfg(not(any(windows, target_os = "macos")))]
fn deliver(title: &str, body: &str, _tab: TabId, _ask: bool, _sound: bool) {
    let _ = notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .appname("macaroni")
        .show();
}

/// Registers [`APP_ID`] for toasts: its display name, and the app icon written to
/// `%LOCALAPPDATA%\macaroni`, under `HKCU\Software\Classes\AppUserModelId`. Cheap and
/// idempotent: run at every start. The registration is per user, so the icon goes to the real
/// local data folder even for a test instance with its own `MACARONI_CONFIG_DIR`, whose folder
/// may not outlive it. Does nothing off Windows.
pub fn register() {
    #[cfg(windows)]
    {
        let Some(local) = std::env::var_os("LOCALAPPDATA") else {
            return;
        };
        let icon = std::path::Path::new(&local)
            .join("macaroni")
            .join("app-icon.png");
        let png: &[u8] = include_bytes!("../assets/app-icon.png");
        if std::fs::read(&icon).ok().as_deref() != Some(png) {
            let _ = icon.parent().map(std::fs::create_dir_all);
            let _ = std::fs::write(&icon, png);
        }
        registry::set_app_id(APP_ID, "macaroni", &icon);
    }
}

/// Clicked notifications, as [`Message::NotificationClicked`]. Installs its sender in
/// [`CLICKS`] when it starts.
pub fn clicks() -> Subscription<Message> {
    Subscription::run(click_stream)
}

/// The stream behind [`clicks`].
fn click_stream() -> impl Stream<Item = Message> {
    iced::stream::channel(8, async |mut output| {
        let (sender, mut receiver) = unbounded();
        *CLICKS.lock().unwrap_or_else(|e| e.into_inner()) = Some(sender);
        while let Some((tab, what)) = receiver.next().await {
            let message = match what {
                Clicked::Show => Message::NotificationClicked(tab),
                Clicked::Answer(allow) => Message::PermissionAnswer(tab, allow),
            };
            let _ = output.send(message).await;
        }
    })
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod registry {
    //! Writing the toast identity's registry values.

    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};

    /// `text` as NUL-terminated UTF-16.
    fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
        text.encode_wide().chain(std::iter::once(0)).collect()
    }

    /// Sets `DisplayName` and `IconUri` under `HKCU\Software\Classes\AppUserModelId\<id>`,
    /// creating the key. Failures are ignored: toasts then show the id instead of the name.
    pub fn set_app_id(id: &str, name: &str, icon: &Path) {
        let key = wide(format!(r"Software\Classes\AppUserModelId\{id}").as_ref());
        for (value, data) in [
            ("DisplayName", wide(name.as_ref())),
            ("IconUri", wide(icon.as_os_str())),
        ] {
            let value = wide(value.as_ref());
            // SAFETY: every string is NUL-terminated and outlives the call; the byte count
            // includes the terminator, as REG_SZ requires.
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    value.as_ptr(),
                    REG_SZ,
                    data.as_ptr().cast(),
                    (data.len() * 2) as u32,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_answer_and_the_body_shows_the_tab() {
        assert_eq!(answer(Some(ALLOW)), Clicked::Answer(true));
        assert_eq!(answer(Some(DENY)), Clicked::Answer(false));
        assert_eq!(answer(None), Clicked::Show);
        assert_eq!(answer(Some("something else")), Clicked::Show);
    }
}

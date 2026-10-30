//! OS notifications for agent attention, through `notify-rust`. The app decides when to notify
//! (`notifications.os` on, the tab not muted, and the window unfocused or the tab hidden); this
//! only delivers.

/// Shows an OS notification on a background thread; delivery can block on some platforms.
///
/// On Windows the toast uses PowerShell's app identity: a custom one needs an AppUserModelID
/// registered through a Start-menu shortcut, which comes with packaging.
pub fn show(title: String, body: String) {
    std::thread::spawn(move || {
        let mut notification = notify_rust::Notification::new();
        notification.summary(&title).body(&body).appname("macaroni");
        let _ = notification.show();
    });
}

//! The CEF implementation (Windows).
//!
//! Lifecycle: [`helper_process`] runs first in `main` and exits for Chromium's helper processes.
//! The app starts [`Runtime`] lazily, the first time a browser tab is shown, then calls
//! [`Browser::open`] per tab. Creation is asynchronous: CEF hands the new browser to
//! `on_after_created` on its own UI thread, and [`Shared`] carries state (bounds, the drop flag)
//! across that gap. Dropping a [`Browser`] asks CEF to close it and its popups; `on_before_close`
//! counts it down in [`LIVE`], which [`wait_closed`] waits on before the app's window goes away.
//!
//! Threading: CEF runs its own UI thread (`multi_threaded_message_loop`), so every handler in
//! this file runs there, while [`Browser`]'s methods run on iced's thread. They share only
//! [`Shared`] behind a mutex, and window placement is asynchronous (`SWP_ASYNCWINDOWPOS`), so
//! neither thread waits on the other.
//!
//! Each page is a native child window of the app's window, laid over a pane by [`Browser::place`].
//! That only works because the app renders with GL (`browser::prefer_gl` in the app crate): DWM
//! composites a DX12 or Vulkan flip-model swap chain above the window's children, so the page
//! would be there but never seen.
//!
//! The handlers are `wrap_*!` structs from the `cef` crate: [`PanelClient`] and its handlers for
//! a tab's page, [`PopupClient`] for real popup windows (`window.open` with features).

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cef::*;

use crate::{Bounds, Event, KeyPress};

/// The app's `on_event` callback, shared by a panel's handlers.
type Sink = Arc<dyn Fn(Event) + Send + Sync>;
/// Returns true when the app handled the key as a shortcut, so the page doesn't get it.
type Shortcuts = Arc<dyn Fn(KeyPress) -> bool + Send + Sync>;

/// Browsers (panels and popups) that have been created, or are being created, and haven't
/// finished closing; shutdown waits for them. Incremented before CEF is asked to create one and
/// decremented in `on_before_close`.
static LIVE: AtomicUsize = AtomicUsize::new(0);

/// If this process is a CEF helper, runs it to completion and returns its exit code.
///
/// Chromium starts its renderer, GPU and utility processes as this same executable with a
/// `--type=...` argument. Returns `None` in the main process (no `--type`), which carries on.
pub fn helper_process() -> Option<i32> {
    // CEF requires the API version to be fixed before any other call; the hash itself is unused.
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = args::Args::new();
    // -1 means "not a helper, this is the browser process".
    let code = execute_process(Some(args.as_main_args()), None, std::ptr::null_mut());
    (code >= 0).then_some(code)
}

/// CEF, initialized. Dropping it waits (up to 3 seconds) for browsers to finish closing and
/// shuts CEF down; do that on the thread that started it, after the app's event loop has ended.
pub struct Runtime(());

impl Runtime {
    /// Starts CEF with its profile (cookies, cache) in `data_dir`. Call on the main thread.
    ///
    /// Fails, with a message for the tab, when Chromium won't initialize. The usual cause is
    /// another process already using the same profile: Chromium allows one per profile.
    pub fn start(data_dir: &Path) -> Result<Self, String> {
        let data = data_dir.to_string_lossy();
        // No sandbox: it needs a separate sandbox-enabled bootstrap executable. The multi-threaded
        // message loop gives CEF its own UI thread (Windows only), so iced's loop stays untouched
        // and nothing has to pump CEF messages.
        let settings = Settings {
            no_sandbox: 1,
            multi_threaded_message_loop: 1,
            root_cache_path: CefString::from(data.as_ref()),
            cache_path: CefString::from(data.as_ref()),
            persist_session_cookies: 1,
            log_severity: LogSeverity::WARNING,
            ..Default::default()
        };
        let args = args::Args::new();
        if initialize(
            Some(args.as_main_args()),
            Some(&settings),
            None,
            std::ptr::null_mut(),
        ) != 1
        {
            return Err(format!(
                "Chromium failed to start (is another macaroni using {}?)",
                data_dir.display()
            ));
        }
        Ok(Self(()))
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        wait_closed(Duration::from_secs(3));
        shutdown();
    }
}

/// Blocks until every dropped [`Browser`] has finished closing, or `timeout` passes. Close the
/// app's window only after this: a browser whose parent window is destroyed under it leaves
/// Chromium with dangling state at shutdown.
///
/// Polls `LIVE` every 20 ms. Call it off the thread whose window hosts the pages if that
/// thread must keep pumping messages meanwhile.
pub fn wait_closed(timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while LIVE.load(Ordering::Acquire) > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// State a [`Browser`] (iced's thread) shares with its CEF handlers (CEF's UI thread).
#[derive(Default)]
struct Shared {
    /// The CEF browser, once `on_after_created` delivered it; cleared in `on_before_close`.
    browser: Option<cef::Browser>,
    /// Where the window should be; `None` hides it. Applied once the browser exists.
    bounds: Option<Bounds>,
    /// The bounds last sent to the window (`Some(None)` = hidden), or `None` before the first
    /// placement. Lets [`Shared::apply_bounds`] skip redundant `SetWindowPos` calls, since the
    /// app re-places every visible browser on each update.
    placed: Option<Option<Bounds>>,
    /// The panel was dropped, possibly before the browser existed.
    dropped: bool,
    /// The app's window, which owns the page's popup windows.
    parent: isize,
    /// Popup windows the page (or its popups) opened; closed with the panel.
    popups: Vec<cef::Browser>,
}

impl Shared {
    /// Moves the window to [`Shared::bounds`] if it isn't there already. Does nothing until the
    /// browser exists; `on_after_created` calls it again then.
    fn apply_bounds(&mut self) {
        if self.placed == Some(self.bounds) {
            return;
        }
        let Some(host) = self.browser.as_ref().and_then(|b| b.host()) else {
            return;
        };
        self.placed = Some(self.bounds);
        platform::place(host.window_handle(), self.bounds);
    }
}

/// One browser panel. Dropping it closes the browser.
///
/// Until CEF has created the page, the navigation and focus methods do nothing.
pub struct Browser {
    /// State shared with this panel's handlers and its popups' handlers.
    shared: Arc<Mutex<Shared>>,
}

impl Browser {
    /// Opens `url` in a new child window of `parent` (an `HWND` on Windows).
    ///
    /// Returns at once; the page appears once CEF has created it, at `bounds` (or hidden). Every
    /// [`Event`] goes to `on_event`, and each keydown is offered to `shortcut` first, both on CEF's
    /// UI thread. The runtime must already be started ([`Runtime::start`]).
    pub fn open(
        parent: isize,
        url: &str,
        bounds: Option<Bounds>,
        on_event: impl Fn(Event) + Send + Sync + 'static,
        shortcut: impl Fn(KeyPress) -> bool + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            bounds,
            parent,
            ..Shared::default()
        }));
        let sink: Sink = Arc::new(on_event);
        let shortcuts: Shortcuts = Arc::new(shortcut);
        let mut client = PanelClient::new(shared.clone(), sink, shortcuts);
        // Alloy style is the bare embeddable page, with none of Chrome's own window UI. The
        // child starts with an empty rect; `apply_bounds` sizes it once it exists.
        let window_info = WindowInfo {
            runtime_style: RuntimeStyle::ALLOY,
            ..WindowInfo::default()
        }
        .set_as_child(platform::handle(parent), &Rect::default());
        // Counted before creation so a shutdown that starts in between still waits for it.
        LIVE.fetch_add(1, Ordering::AcqRel);
        browser_host_create_browser(
            Some(&window_info),
            Some(&mut client),
            Some(&CefString::from(url)),
            Some(&BrowserSettings::default()),
            None,
            None,
        );
        Self { shared }
    }

    /// Moves the window over `bounds`, or hides it. Cheap to call repeatedly with the same
    /// value. Before the page exists, the bounds are remembered and applied on creation.
    pub fn place(&self, bounds: Option<Bounds>) {
        let mut shared = lock(&self.shared);
        shared.bounds = bounds;
        shared.apply_bounds();
    }

    /// Loads `url` in the main frame.
    pub fn navigate(&self, url: &str) {
        if let Some(frame) = self.with(|b| b.main_frame()).flatten() {
            frame.load_url(Some(&CefString::from(url)));
        }
    }

    /// Goes back in history.
    pub fn back(&self) {
        self.with(|b| b.go_back());
    }

    /// Goes forward in history.
    pub fn forward(&self) {
        self.with(|b| b.go_forward());
    }

    /// Reloads the page.
    pub fn reload(&self) {
        self.with(|b| b.reload());
    }

    /// Stops loading.
    pub fn stop(&self) {
        self.with(|b| b.stop_load());
    }

    /// Gives the page keyboard focus.
    pub fn focus(&self) {
        if let Some(host) = self.with(|b| b.host()).flatten() {
            host.set_focus(1);
        }
    }

    /// The page's visible text, delivered to `done` on CEF's UI thread. `done` gets an empty
    /// string at once (on the caller's thread) when the page doesn't exist yet.
    pub fn read_text(&self, done: impl FnOnce(String) + Send + 'static) {
        let Some(frame) = self.with(|b| b.main_frame()).flatten() else {
            done(String::new());
            return;
        };
        let done: TextDone = Arc::new(Mutex::new(Some(Box::new(done))));
        let mut visitor = TextVisitor::new(done);
        frame.text(Some(&mut visitor));
    }

    /// Opens DevTools for this page in its own top-level window.
    pub fn devtools(&self) {
        let Some(host) = self.with(|b| b.host()).flatten() else {
            return;
        };
        let info = platform::devtools_window();
        host.show_dev_tools(Some(&info), None, None, None);
    }

    /// Runs `f` on the CEF browser, or returns `None` if it doesn't exist (yet, or anymore).
    /// Holds the shared lock while `f` runs.
    fn with<T>(&self, f: impl FnOnce(&cef::Browser) -> T) -> Option<T> {
        lock(&self.shared).browser.as_ref().map(f)
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let mut shared = lock(&self.shared);
        // If the page isn't created yet, `adopt` sees this flag and closes it on arrival.
        shared.dropped = true;
        // Force close (1): skip `beforeunload` prompts. The rest happens asynchronously and ends
        // in `on_before_close`.
        for browser in shared.browser.iter().chain(&shared.popups) {
            if let Some(host) = browser.host() {
                host.close_browser(1);
            }
        }
    }
}

/// Gives keyboard focus back to the app's own window (an `HWND` on Windows), away from a page.
/// Call it on the thread that owns `window`.
pub fn focus_window(window: isize) {
    platform::focus(window);
}

/// The [`Browser::read_text`] callback. The `Option` lets the visitor call it at most once,
/// whatever CEF does.
type TextDone = Arc<Mutex<Option<Box<dyn FnOnce(String) + Send>>>>;

// Receives the frame's text for [`Browser::read_text`].
wrap_string_visitor! {
    struct TextVisitor {
        // The callback, taken on the first visit.
        done: TextDone,
    }

    impl CefStringVisitor {
        fn visit(&self, text: Option<&CefString>) {
            let done = self.done.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(done) = done {
                done(string(text));
            }
        }
    }
}

/// Locks [`Shared`], ignoring poisoning: a handler that panicked mid-update leaves nothing a
/// later caller can't cope with.
fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

/// A CEF string as a Rust one; empty when CEF passed none.
fn string(value: Option<&CefString>) -> String {
    value.map(ToString::to_string).unwrap_or_default()
}

// The client of a tab's page: hands CEF the handlers that report [`Event`]s and offer keys
// to the app.
wrap_client! {
    struct PanelClient {
        // The panel's shared state.
        shared: Arc<Mutex<Shared>>,
        // Where events go.
        sink: Sink,
        // The app's shortcut check.
        shortcuts: Shortcuts,
    }

    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(PanelLifeSpan::new(self.shared.clone(), self.sink.clone()))
        }

        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(PanelDisplay::new(self.sink.clone()))
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(PanelLoad::new(self.sink.clone()))
        }

        fn focus_handler(&self) -> Option<FocusHandler> {
            Some(PanelFocus::new(self.sink.clone()))
        }

        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(PanelKeys::new(self.shortcuts.clone()))
        }
    }
}

// Creation, popups and closing for a tab's page.
wrap_life_span_handler! {
    struct PanelLifeSpan {
        // The panel's shared state.
        shared: Arc<Mutex<Shared>>,
        // Where events go.
        sink: Sink,
    }

    impl LifeSpanHandler {
        // The page now exists: keep it and apply the bounds the app set while waiting.
        fn on_after_created(&self, browser: Option<&mut cef::Browser>) {
            adopt(&self.shared, browser, |shared, browser| {
                shared.browser = Some(browser.clone());
                shared.apply_bounds();
            });
        }

        fn on_before_popup(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            _user_gesture: ::std::os::raw::c_int,
            popup_features: Option<&PopupFeatures>,
            window_info: Option<&mut WindowInfo>,
            client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            before_popup(
                &self.shared,
                Some(&self.sink),
                target_url,
                target_disposition,
                popup_features,
                window_info,
                client,
            )
        }

        // CEF calls this only for a popup `before_popup` allowed whose creation then failed, so
        // `PopupLifeSpan::on_before_close` will never run for it.
        fn on_before_popup_aborted(
            &self,
            _browser: Option<&mut cef::Browser>,
            _popup_id: ::std::os::raw::c_int,
        ) {
            popup_aborted();
        }

        // The last callback for this browser. Dropping our reference here lets CEF free it.
        fn on_before_close(&self, _browser: Option<&mut cef::Browser>) {
            lock(&self.shared).browser = None;
            LIVE.fetch_sub(1, Ordering::AcqRel);
            (self.sink)(Event::Closed);
        }
    }
}

/// Hands a just-created browser to `keep`, or closes it if the panel was dropped meanwhile.
fn adopt(
    shared: &Mutex<Shared>,
    browser: Option<&mut cef::Browser>,
    keep: impl FnOnce(&mut Shared, &cef::Browser),
) {
    let Some(browser) = browser else {
        return;
    };
    let mut shared = lock(shared);
    if shared.dropped {
        if let Some(host) = browser.host() {
            host.close_browser(1);
        }
        return;
    }
    keep(&mut shared, browser);
}

/// Routes a page's new window; `sink` is `None` when the opener is itself a popup.
///
/// Returns CEF's verdict for `on_before_popup`: 0 lets CEF create the popup window that
/// `window_info` and `client` now describe, 1 cancels it (after asking the app to open the URL as
/// a tab, when it came from a panel).
fn before_popup(
    shared: &Arc<Mutex<Shared>>,
    sink: Option<&Sink>,
    target_url: Option<&CefString>,
    disposition: WindowOpenDisposition,
    features: Option<&PopupFeatures>,
    window_info: Option<&mut WindowInfo>,
    client: Option<&mut Option<Client>>,
) -> ::std::os::raw::c_int {
    // A popup has no tab strip, so whatever it opens is a window too. From the panel,
    // `window.open` with window features (sign-in dialogs and the like) gets a real window, so it
    // keeps `window.opener`; other new windows become tabs.
    let wants_window = sink.is_none() || disposition == WindowOpenDisposition::NEW_POPUP;
    if wants_window && open_popup(shared, features, window_info, client) {
        return 0;
    }
    let url = string(target_url);
    if let Some(sink) = sink
        && !url.is_empty()
    {
        sink(Event::NewWindow(url));
    }
    1
}

/// Lets CEF open a popup as its own window, owned by the app's, with a [`PopupClient`]. False if
/// CEF didn't pass the out-parameters to fill.
fn open_popup(
    shared: &Arc<Mutex<Shared>>,
    features: Option<&PopupFeatures>,
    window_info: Option<&mut WindowInfo>,
    client: Option<&mut Option<Client>>,
) -> bool {
    let (Some(info), Some(client)) = (window_info, client) else {
        return false;
    };
    let parent = lock(shared).parent;
    *info = platform::popup_window(parent, features);
    *client = Some(PopupClient::new(shared.clone()));
    // Matched by `PopupLifeSpan::on_before_close`, or `popup_aborted` if creation fails.
    LIVE.fetch_add(1, Ordering::AcqRel);
    true
}

/// Undoes `open_popup`'s count of a popup that was never created.
fn popup_aborted() {
    LIVE.fetch_sub(1, Ordering::AcqRel);
}

// The client of a popup window. No keyboard handler, so app shortcuts don't apply inside
// popups, and no events reach the app; only its title is mirrored onto the window.
wrap_client! {
    struct PopupClient {
        // The state of the panel the popup chain started from.
        shared: Arc<Mutex<Shared>>,
    }

    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(PopupLifeSpan::new(self.shared.clone()))
        }

        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(PopupDisplay::new())
        }
    }
}

// Tracks a popup in [`Shared::popups`] so closing the tab closes it too.
wrap_life_span_handler! {
    struct PopupLifeSpan {
        // The state of the panel the popup chain started from.
        shared: Arc<Mutex<Shared>>,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut cef::Browser>) {
            adopt(&self.shared, browser, |shared, browser| {
                shared.popups.push(browser.clone());
            });
        }

        fn on_before_popup(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            _user_gesture: ::std::os::raw::c_int,
            popup_features: Option<&PopupFeatures>,
            window_info: Option<&mut WindowInfo>,
            client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            before_popup(
                &self.shared,
                None,
                target_url,
                target_disposition,
                popup_features,
                window_info,
                client,
            )
        }

        // CEF calls this only for a popup `before_popup` allowed whose creation then failed, so
        // `PopupLifeSpan::on_before_close` will never run for it.
        fn on_before_popup_aborted(
            &self,
            _browser: Option<&mut cef::Browser>,
            _popup_id: ::std::os::raw::c_int,
        ) {
            popup_aborted();
        }

        fn on_before_close(&self, browser: Option<&mut cef::Browser>) {
            if let Some(browser) = browser {
                let id = browser.identifier();
                lock(&self.shared).popups.retain(|b| b.identifier() != id);
            }
            LIVE.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

// Copies a popup's page title onto its window, since nothing else shows it.
wrap_display_handler! {
    struct PopupDisplay {}

    impl DisplayHandler {
        fn on_title_change(&self, browser: Option<&mut cef::Browser>, title: Option<&CefString>) {
            if let Some(host) = browser.and_then(|b| b.host()) {
                platform::set_title(host.window_handle(), &string(title));
            }
        }
    }
}

// Reports the address and title of a tab's page.
wrap_display_handler! {
    struct PanelDisplay {
        // Where events go.
        sink: Sink,
    }

    impl DisplayHandler {
        fn on_address_change(
            &self,
            _browser: Option<&mut cef::Browser>,
            frame: Option<&mut Frame>,
            url: Option<&CefString>,
        ) {
            // Subframes (iframes) navigate too; only the main frame is the tab's address.
            if frame.is_some_and(|f| f.is_main() == 1) {
                (self.sink)(Event::Address(string(url)));
            }
        }

        fn on_title_change(&self, _browser: Option<&mut cef::Browser>, title: Option<&CefString>) {
            (self.sink)(Event::Title(string(title)));
        }
    }
}

// Reports loading state and history availability for the address bar buttons.
wrap_load_handler! {
    struct PanelLoad {
        // Where events go.
        sink: Sink,
    }

    impl LoadHandler {
        fn on_loading_state_change(
            &self,
            _browser: Option<&mut cef::Browser>,
            is_loading: ::std::os::raw::c_int,
            can_go_back: ::std::os::raw::c_int,
            can_go_forward: ::std::os::raw::c_int,
        ) {
            (self.sink)(Event::Loading {
                loading: is_loading == 1,
                back: can_go_back == 1,
                forward: can_go_forward == 1,
            });
        }
    }
}

// Reports when the page takes keyboard focus, so the app can select its tab.
wrap_focus_handler! {
    struct PanelFocus {
        // Where events go.
        sink: Sink,
    }

    impl FocusHandler {
        fn on_got_focus(&self, _browser: Option<&mut cef::Browser>) {
            (self.sink)(Event::Focused);
        }
    }
}

// Offers keys to the app before the page sees them. While a page has focus the app's own
// window gets no key events at all, so this is the only way its shortcuts can work there.
wrap_keyboard_handler! {
    struct PanelKeys {
        // The app's shortcut check.
        shortcuts: Shortcuts,
    }

    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            _browser: Option<&mut cef::Browser>,
            event: Option<&KeyEvent>,
            _os_event: Option<&mut sys::MSG>,
            _is_keyboard_shortcut: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let Some(event) = event else {
                return 0;
            };
            // Only the raw keydown: it carries the virtual-key code, and checking it alone means
            // each press is offered once (not again as CHAR or KEYUP).
            if event.type_ != KeyEventType::RAWKEYDOWN {
                return 0;
            }
            let flag = |f: sys::cef_event_flags_t| event.modifiers & f.0 as u32 != 0;
            let press = KeyPress {
                code: event.windows_key_code as u32,
                ctrl: flag(sys::cef_event_flags_t::EVENTFLAG_CONTROL_DOWN),
                shift: flag(sys::cef_event_flags_t::EVENTFLAG_SHIFT_DOWN),
                alt: flag(sys::cef_event_flags_t::EVENTFLAG_ALT_DOWN),
                meta: flag(sys::cef_event_flags_t::EVENTFLAG_COMMAND_DOWN),
            };
            // 1 swallows the event; 0 lets the page have it.
            (self.shortcuts)(press).into()
        }
    }
}

// Placing the browser window takes Win32 calls on its handle.
#[allow(unsafe_code)]
mod platform {
    //! The Win32 side: window handles, placement, focus and titles, and the [`WindowInfo`]s for
    //! popups and DevTools. The only unsafe code in the crate.

    use cef::{CefString, PopupFeatures, RuntimeStyle, WindowInfo, sys::cef_window_handle_t};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SWP_ASYNCWINDOWPOS, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
        SWP_SHOWWINDOW, SetWindowPos, SetWindowTextW,
    };

    use super::Bounds;

    /// An app-side window handle (`HWND` as `isize`) as CEF's handle type.
    pub fn handle(window: isize) -> cef_window_handle_t {
        cef::sys::HWND(window as *mut _)
    }

    /// Moves and shows the window at `bounds`, or hides it.
    ///
    /// Async so the caller never waits on CEF's UI thread, which owns the window.
    pub fn place(window: cef_window_handle_t, bounds: Option<Bounds>) {
        let flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS;
        // Hidden in place: resizing a hidden page would make it lay out again for nothing.
        let (b, flags) = match bounds {
            Some(b) => (b, flags | SWP_SHOWWINDOW),
            None => (
                Bounds {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                },
                flags | SWP_HIDEWINDOW | SWP_NOMOVE | SWP_NOSIZE,
            ),
        };
        // SAFETY: SetWindowPos tolerates any handle value, including a destroyed window.
        unsafe {
            SetWindowPos(
                window.0.cast(),
                std::ptr::null_mut(),
                b.x,
                b.y,
                b.width,
                b.height,
                flags,
            );
        }
    }

    /// Gives `window` the keyboard focus.
    pub fn focus(window: isize) {
        // SAFETY: SetFocus tolerates any handle value; the window belongs to this thread.
        unsafe {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(window as _);
        }
    }

    /// A page's popup: a top-level window owned by the app's, where the page asked for it.
    pub fn popup_window(owner: isize, features: Option<&PopupFeatures>) -> WindowInfo {
        let mut info = WindowInfo {
            runtime_style: RuntimeStyle::ALLOY,
            ..WindowInfo::default()
        }
        .set_as_popup(handle(owner), "macaroni");
        if let Some(f) = features {
            let pick = |set: i32, value: i32, default: i32| if set == 1 { value } else { default };
            let b = &mut info.bounds;
            b.x = pick(f.x_set, f.x, b.x);
            b.y = pick(f.y_set, f.y, b.y);
            b.width = pick(f.width_set, f.width, b.width);
            b.height = pick(f.height_set, f.height, b.height);
        }
        // Unset features keep `set_as_popup`'s defaults (CW_USEDEFAULT), so Windows picks.
        info
    }

    /// Sets a top-level window's title bar text.
    pub fn set_title(window: cef_window_handle_t, title: &str) {
        let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call; any handle value is tolerated.
        unsafe {
            SetWindowTextW(window.0.cast(), wide.as_ptr());
        }
    }

    /// A top-level, unowned window for DevTools.
    pub fn devtools_window() -> WindowInfo {
        WindowInfo {
            window_name: CefString::from("DevTools - macaroni"),
            ..WindowInfo::default()
        }
        .set_as_popup(handle(0), "DevTools - macaroni")
    }
}

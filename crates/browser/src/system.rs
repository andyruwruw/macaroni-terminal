//! The system engine: the OS's own web view through `wry`, WebView2 (Edge) on Windows and
//! WKWebView (Safari's WebKit) on macOS.
//!
//! Everything runs on the app's main thread, which owns the parent window: `wry` creates, moves
//! and calls into the web view there, and its handlers run there too. So unlike Chromium there's
//! no shared state behind a lock and nothing to wait for on close; dropping the [`Browser`]
//! drops the web view.
//!
//! What `wry` doesn't report comes from [`SCRIPT`], injected into every page's main frame: it
//! posts in-page navigation (`history.pushState`, hash changes) and `window.close()` over the IPC
//! channel, and on macOS focus and the keys pressed with a modifier. On Windows those two come
//! from WebView2's own events instead (`native`): the page's `focus` event also fires when
//! WebView2 shuffles focus internally after an app shortcut, which would pull the tab back, and
//! the accelerator key event can keep a shortcut from the page.

use std::cell::{Cell, OnceCell};
use std::path::Path;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};

use wry::dpi::{PhysicalPosition, PhysicalSize};
use wry::raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
use wry::{NewWindowResponse, PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder};

use crate::{Bounds, Event, KeyPress};

/// The app's `on_event` callback, shared by a page's handlers.
type Sink = Rc<dyn Fn(Event)>;
/// Returns true when the app handled the key as a shortcut.
type Shortcuts = Arc<dyn Fn(KeyPress) -> bool + Send + Sync>;
/// The web view, for handlers created before it exists: set once it's built, and weak so the
/// handlers it owns don't keep it alive.
type Slot = Rc<OnceCell<Weak<WebView>>>;

/// Posts what `wry` doesn't report as `kind` or `kind payload` messages: `nav <url>` for in-page
/// navigation and `close` for `window.close()`; on macOS also `focus` when the page takes the
/// keyboard and `key <code> <ctrl> <shift> <alt> <meta>` for keys pressed with a modifier.
const SCRIPT: &str = r#"(() => {
  const post = (m) => window.ipc.postMessage(m);
  const nav = () => post('nav ' + location.href);
  for (const name of ['pushState', 'replaceState']) {
    const original = history[name];
    history[name] = function (...args) {
      const result = original.apply(this, args);
      nav();
      return result;
    };
  }
  addEventListener('popstate', nav);
  addEventListener('hashchange', nav);
  window.close = () => post('close');
  if (MACOS) {
    addEventListener('focus', () => post('focus'));
    addEventListener('keydown', (e) => {
      if (e.ctrlKey || e.altKey || e.metaKey) {
        const b = (v) => (v ? 1 : 0);
        post(`key ${e.keyCode} ${b(e.ctrlKey)} ${b(e.shiftKey)} ${b(e.altKey)} ${b(e.metaKey)}`);
      }
    }, true);
  }
})();"#;

/// The page text [`Browser::read_text`] asks for.
const TEXT: &str = "document.body ? document.body.innerText : ''";

/// One page. Dropping it closes the web view.
pub struct Browser {
    /// The web view; the handlers hold it weakly through their [`Slot`].
    view: Rc<WebView>,
    /// The bounds last applied (`Some(None)` = hidden), or `None` before the first placement.
    /// Lets [`Browser::place`] skip redundant calls, since the app re-places every visible
    /// browser on each update.
    placed: Cell<Option<Option<Bounds>>>,
}

impl Browser {
    /// Opens `url` in a new web view inside `parent`; see [`crate::Browser::open`]. WebView2
    /// keeps cookies and cache under `<data_dir>/system`; WebKit uses the app's default store.
    pub fn open(
        data_dir: &Path,
        parent: isize,
        url: &str,
        bounds: Option<Bounds>,
        on_event: impl Fn(Event) + Send + Sync + 'static,
        shortcut: impl Fn(KeyPress) -> bool + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let sink: Sink = Rc::new(on_event);
        let shortcuts: Shortcuts = Arc::new(shortcut);
        let slot: Slot = Rc::default();
        let mut context = WebContext::new(Some(data_dir.join("system")));
        let macos = if cfg!(target_os = "macos") {
            "true"
        } else {
            "false"
        };
        let script = SCRIPT.replace("MACOS", macos);

        let builder = WebViewBuilder::new_with_web_context(&mut context)
            .with_url(url)
            .with_bounds(rect(bounds.unwrap_or(Bounds {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            })))
            .with_visible(bounds.is_some())
            .with_focused(false)
            .with_devtools(true)
            .with_initialization_script_for_main_only(script, true)
            .with_ipc_handler({
                let (sink, slot, shortcuts) = (sink.clone(), slot.clone(), shortcuts.clone());
                move |request| message(request.body(), &sink, &slot, &shortcuts)
            })
            .with_document_title_changed_handler({
                let sink = sink.clone();
                move |title| sink(Event::Title(title))
            })
            .with_on_page_load_handler({
                let (sink, slot) = (sink.clone(), slot.clone());
                move |event, url| {
                    let loading = matches!(event, PageLoadEvent::Started);
                    if loading {
                        sink(Event::Address(url));
                    }
                    loading_state(&sink, &slot, loading);
                }
            })
            // Every new window becomes a tab beside this one; a popup loses `window.opener`.
            .with_new_window_req_handler({
                let sink = sink.clone();
                move |url, _features| {
                    if !url.is_empty() {
                        sink(Event::NewWindow(url));
                    }
                    NewWindowResponse::Deny
                }
            });
        let view = builder
            .build_as_child(&Parent(parent))
            .map_err(|e| format!("the system web view didn't start: {e}"))?;
        #[cfg(windows)]
        native::hook(&view, sink.clone(), shortcuts);
        let view = Rc::new(view);
        let _ = slot.set(Rc::downgrade(&view));
        Ok(Self {
            view,
            placed: Cell::new(Some(bounds)),
        })
    }

    /// Moves the web view over `bounds`, or hides it in place (keeping its size, so a hidden
    /// page doesn't lay out again for nothing).
    pub fn place(&self, bounds: Option<Bounds>) {
        if self.placed.get() == Some(bounds) {
            return;
        }
        self.placed.set(Some(bounds));
        match bounds {
            Some(b) => {
                let _ = self.view.set_bounds(rect(b));
                let _ = self.view.set_visible(true);
            }
            None => {
                let _ = self.view.set_visible(false);
            }
        }
    }

    /// Loads `url`.
    pub fn navigate(&self, url: &str) {
        let _ = self.view.load_url(url);
    }

    /// Goes back in history.
    pub fn back(&self) {
        let _ = self.view.go_back();
    }

    /// Goes forward in history.
    pub fn forward(&self) {
        let _ = self.view.go_forward();
    }

    /// Reloads the page.
    pub fn reload(&self) {
        let _ = self.view.reload();
    }

    /// Stops loading; `wry` has no call for it, the page's own `window.stop()` does the same.
    pub fn stop(&self) {
        let _ = self.view.evaluate_script("window.stop()");
    }

    /// Gives the page keyboard focus.
    pub fn focus(&self) {
        let _ = self.view.focus();
    }

    /// Gives keyboard focus back to the app's window.
    pub fn blur(&self) {
        let _ = self.view.focus_parent();
    }

    /// Opens the web inspector in its own window.
    pub fn devtools(&self) {
        self.view.open_devtools();
    }

    /// The page's visible text, delivered to `done` once the page answers.
    pub fn read_text(&self, done: impl FnOnce(String) + Send + 'static) {
        // `wry` wants a callback it may call more than once; `done` runs at most once.
        let done = Mutex::new(Some(done));
        let callback = move |json: String| {
            let done = done.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(done) = done {
                done(serde_json::from_str(&json).unwrap_or_default());
            }
        };
        // On failure the callback is dropped unrun, and `done` with it.
        let _ = self.view.evaluate_script_with_callback(TEXT, callback);
    }
}

/// Handles a message [`SCRIPT`] posted.
fn message(body: &str, sink: &Sink, slot: &Slot, shortcuts: &Shortcuts) {
    let (kind, rest) = body.split_once(' ').unwrap_or((body, ""));
    match kind {
        "nav" => {
            sink(Event::Address(rest.to_owned()));
            loading_state(sink, slot, false);
        }
        "focus" => sink(Event::Focused),
        "close" => sink(Event::Closed),
        "key" => {
            let mut fields = rest.split(' ').map(|f| f.parse::<u32>().unwrap_or(0));
            let mut next = || fields.next().unwrap_or(0);
            let press = KeyPress {
                code: next(),
                ctrl: next() == 1,
                shift: next() == 1,
                alt: next() == 1,
                meta: next() == 1,
            };
            // The page has already seen the key: a script can't wait for the app's answer.
            shortcuts(press);
        }
        _ => {}
    }
}

/// Reports whether the page is loading, with the history buttons' state.
fn loading_state(sink: &Sink, slot: &Slot, loading: bool) {
    let view = slot.get().and_then(Weak::upgrade);
    let can = |f: fn(&WebView) -> wry::Result<bool>| {
        view.as_deref().is_some_and(|v| f(v).unwrap_or(false))
    };
    sink(Event::Loading {
        loading,
        back: can(WebView::can_go_back),
        forward: can(WebView::can_go_forward),
    });
}

/// `bounds` (physical pixels) as `wry`'s rectangle.
fn rect(bounds: Bounds) -> Rect {
    Rect {
        position: PhysicalPosition::new(bounds.x, bounds.y).into(),
        size: PhysicalSize::new(bounds.width.max(0) as u32, bounds.height.max(0) as u32).into(),
    }
}

/// The app's window, as the native handle `wry` builds into.
struct Parent(isize);

impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        #[cfg(windows)]
        let raw = RawWindowHandle::Win32(wry::raw_window_handle::Win32WindowHandle::new(
            std::num::NonZeroIsize::new(self.0).ok_or(HandleError::Unavailable)?,
        ));
        #[cfg(target_os = "macos")]
        let raw = RawWindowHandle::AppKit(wry::raw_window_handle::AppKitWindowHandle::new(
            std::ptr::NonNull::new(self.0 as *mut std::ffi::c_void)
                .ok_or(HandleError::Unavailable)?,
        ));
        // SAFETY: the app passes its live main window's handle, which outlives every page (the
        // pages are dropped before the window closes), and `wry` only uses it while building.
        #[allow(unsafe_code)]
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod native {
    //! Windows: WebView2's own focus and key events. The accelerator key event fires before the
    //! page sees the key, for every key pressed with Ctrl or Alt and every key that doesn't type
    //! a character; handling it keeps the key from the page.

    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
        COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN,
    };
    use webview2_com::{AcceleratorKeyPressedEventHandler, FocusChangedEventHandler};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    use wry::{WebView, WebViewExtWindows};

    use super::{Shortcuts, Sink};
    use crate::{Event, KeyPress};

    /// Registers the handlers on `view`'s controller for as long as the view lives: focus goes
    /// to `sink`, keys to `shortcuts`.
    pub fn hook(view: &WebView, sink: Sink, shortcuts: Shortcuts) {
        let controller = view.controller();
        let focused = FocusChangedEventHandler::create(Box::new(move |_, _| {
            sink(Event::Focused);
            Ok(())
        }));
        let handler = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else {
                return Ok(());
            };
            let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
            let mut code = 0;
            // SAFETY: out-parameter getters on the event args WebView2 passed this handler.
            unsafe {
                args.KeyEventKind(&mut kind)?;
                args.VirtualKey(&mut code)?;
            }
            // System key-downs are the ones pressed with Alt.
            if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
            {
                return Ok(());
            }
            // SAFETY: GetKeyState only reads this thread's keyboard state; the high bit is set
            // while the key is down.
            let held = |vk: u16| unsafe { GetKeyState(i32::from(vk)) } < 0;
            let press = KeyPress {
                code,
                ctrl: held(VK_CONTROL),
                shift: held(VK_SHIFT),
                alt: held(VK_MENU),
                meta: held(VK_LWIN) || held(VK_RWIN),
            };
            if shortcuts(press) {
                // SAFETY: as above.
                unsafe { args.SetHandled(true)? };
            }
            Ok(())
        }));
        let mut token = 0;
        // SAFETY: registers handlers COM keeps a reference to; the controller lives as long as
        // the view.
        unsafe {
            let _ = controller.add_GotFocus(&focused, &mut token);
            let _ = controller.add_AcceleratorKeyPressed(&handler, &mut token);
        }
    }
}

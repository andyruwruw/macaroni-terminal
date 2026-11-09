//! The platform side of screen reader support: one AccessKit adapter per OS window, which
//! answers the screen reader from the tree `accessibility::tree` builds.
//!
//! Adapters aren't `Send` and must be made and fed on the thread that owns the window, the
//! event loop's, so every call here runs inside `window::run` and adapters live in a
//! thread-local map. On Windows the window is subclassed (`SetWindowSubclass`, beside
//! `move_loop`'s) to answer `WM_GETOBJECT` and follow its focus; on macOS AccessKit subclasses
//! the view itself. Nothing is built until a screen reader asks: the activation handler sends
//! [`Event::Activated`] and the app pushes a whole tree. Actions the screen reader asks for
//! (click, focus, open a menu) come back as [`Event::Action`], possibly from another thread.
//! Both go through [`set_events`]' channel to `Message::Access`.

use std::sync::Mutex;

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, TreeUpdate};
use iced::futures::channel::mpsc::UnboundedSender;
use iced::window;

/// What an adapter tells the app.
#[derive(Debug, Clone)]
pub enum Event {
    /// A screen reader started reading the window: it needs a whole tree now, and updates
    /// from now on.
    Activated(window::Id),
    /// A screen reader asked for an action on one of the window's nodes.
    Action(window::Id, ActionRequest),
}

/// Where adapters send [`Event`]s; set by the app's subscription before any window opens.
static EVENTS: Mutex<Option<UnboundedSender<Event>>> = Mutex::new(None);

/// Sets where adapters send their events.
pub fn set_events(sender: UnboundedSender<Event>) {
    *EVENTS.lock().unwrap_or_else(|e| e.into_inner()) = Some(sender);
}

/// Sends `event` to the app, if it's listening.
fn send(event: Event) {
    if let Some(sender) = EVENTS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = sender.unbounded_send(event);
    }
}

/// Answers a screen reader's first look at a window: the tree comes later, from the app.
struct Activation(window::Id);

impl ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        send(Event::Activated(self.0));
        None
    }
}

/// Passes a screen reader's action requests to the app.
struct Actions(window::Id);

impl ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        send(Event::Action(self.0, request));
    }
}

/// Gives window `id` (native handle `handle`: an `HWND`, or an `NSView` on macOS) an adapter.
/// Runs on the event loop's thread.
pub fn attach(id: window::Id, handle: isize) {
    imp::attach(id, handle);
}

/// Applies `update` to window `id`'s tree, if a screen reader is reading it. Runs on the event
/// loop's thread.
pub fn update(id: window::Id, update: TreeUpdate) {
    imp::update(id, update);
}

/// Tells window `id`'s adapter whether the window has the keyboard. Windows follows this on
/// its own, through the window's messages. Runs on the event loop's thread.
pub fn focus(id: window::Id, focused: bool) {
    imp::focus(id, focused);
}

/// Drops window `id`'s adapter, before the window closes. Runs on the event loop's thread.
pub fn detach(id: window::Id) {
    imp::detach(id);
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    //! A window subclass that hands `WM_GETOBJECT` to AccessKit's adapter.

    use std::cell::RefCell;
    use std::collections::HashMap;

    use accesskit_windows::WPARAM as KitWparam;
    use accesskit_windows::{Adapter, HWND as KitHwnd, LPARAM as KitLparam, LRESULT as KitLresult};
    use iced::window;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, WM_GETOBJECT, WM_KILLFOCUS, WM_NCDESTROY, WM_SETFOCUS,
    };

    use super::{Actions, Activation, TreeUpdate};

    /// The subclass's id; `move_loop` uses 1.
    const SUBCLASS: usize = 2;

    /// A window's adapter and the handler it asks for its first tree.
    struct State {
        /// The window's app id.
        id: window::Id,
        /// AccessKit's adapter.
        adapter: Adapter,
        /// Asked for the first tree.
        activation: Activation,
    }

    thread_local! {
        /// Each subclassed window's state, by `HWND`.
        static STATES: RefCell<HashMap<isize, State>> = RefCell::new(HashMap::new());
    }

    /// See [`super::attach`].
    pub fn attach(id: window::Id, handle: isize) {
        let hwnd = handle as HWND;
        // SAFETY: `GetForegroundWindow` only reads.
        let focused = unsafe { GetForegroundWindow() } == hwnd;
        let adapter = Adapter::new(KitHwnd(hwnd.cast()), focused, Actions(id));
        STATES.with(|s| {
            s.borrow_mut().insert(
                handle,
                State {
                    id,
                    adapter,
                    activation: Activation(id),
                },
            )
        });
        // SAFETY: `hwnd` is the live window and this runs on its thread, as subclassing needs;
        // the subclass is removed on `WM_NCDESTROY`, before the window goes.
        unsafe { SetWindowSubclass(hwnd, Some(proc), SUBCLASS, 0) };
    }

    /// See [`super::update`].
    pub fn update(id: window::Id, update: TreeUpdate) {
        // Raised after the borrow ends: raising can call back into the window procedure.
        let events = STATES.with(|s| {
            let mut states = s.borrow_mut();
            let state = states.values_mut().find(|st| st.id == id)?;
            state.adapter.update_if_active(|| update)
        });
        if let Some(events) = events {
            events.raise();
        }
    }

    /// See [`super::focus`]: the subclass follows `WM_SETFOCUS` and `WM_KILLFOCUS` instead.
    pub fn focus(_: window::Id, _: bool) {}

    /// See [`super::detach`]: `WM_NCDESTROY` takes the state away when the window goes.
    pub fn detach(_: window::Id) {}

    /// The subclass procedure: `WM_GETOBJECT` goes to the adapter, focus changes update it, and
    /// `WM_NCDESTROY` drops it. Everything goes on to the next procedure (winit's) unless the
    /// adapter answered.
    unsafe extern "system" fn proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _: usize,
        _: usize,
    ) -> LRESULT {
        let key = hwnd as isize;
        match msg {
            WM_GETOBJECT => {
                // A screen reader asking while an update is being applied (the map is
                // borrowed) gets the default answer this time and asks again.
                let answer = STATES.with(|s| {
                    let mut states = s.try_borrow_mut().ok()?;
                    let state = states.get_mut(&key)?;
                    let State {
                        adapter,
                        activation,
                        ..
                    } = state;
                    let result: KitLresult = adapter
                        .handle_wm_getobject(KitWparam(wparam), KitLparam(lparam), activation)?
                        .into();
                    Some(result.0)
                });
                if let Some(answer) = answer {
                    return answer;
                }
            }
            WM_SETFOCUS | WM_KILLFOCUS => {
                let events = STATES.with(|s| {
                    let mut states = s.try_borrow_mut().ok()?;
                    states
                        .get_mut(&key)?
                        .adapter
                        .update_window_focus_state(msg == WM_SETFOCUS)
                });
                if let Some(events) = events {
                    events.raise();
                }
            }
            WM_NCDESTROY => {
                STATES.with(|s| s.borrow_mut().remove(&key));
                // SAFETY: removing this procedure from the window it was set on, on its thread.
                unsafe { RemoveWindowSubclass(hwnd, Some(proc), SUBCLASS) };
            }
            _ => {}
        }
        // SAFETY: the arguments are the ones Windows passed for this window's message.
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod imp {
    //! AccessKit's own subclassing adapter on the window's view.

    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::panic::AssertUnwindSafe;

    use accesskit_macos::SubclassingAdapter;
    use iced::window;

    use super::{Actions, Activation, TreeUpdate};

    thread_local! {
        /// Each window's adapter.
        static ADAPTERS: RefCell<HashMap<window::Id, SubclassingAdapter>> =
            RefCell::new(HashMap::new());
    }

    /// See [`super::attach`].
    pub fn attach(id: window::Id, handle: isize) {
        // It panics when something already observes the view with key-value observing; the
        // window then just goes without a screen reader tree rather than taking the app down.
        let adapter = std::panic::catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: `handle` is the window's live `NSView`, and this runs on the main
            // thread, which owns it.
            unsafe {
                SubclassingAdapter::new(
                    handle as *mut std::ffi::c_void,
                    Activation(id),
                    Actions(id),
                )
            }
        }));
        if let Ok(adapter) = adapter {
            ADAPTERS.with(|a| a.borrow_mut().insert(id, adapter));
        }
    }

    /// See [`super::update`].
    pub fn update(id: window::Id, update: TreeUpdate) {
        // Raised after the borrow ends: raising can call back into the view.
        let events = ADAPTERS.with(|a| {
            a.borrow_mut()
                .get_mut(&id)
                .and_then(|adapter| adapter.update_if_active(|| update))
        });
        if let Some(events) = events {
            events.raise();
        }
    }

    /// See [`super::focus`].
    pub fn focus(id: window::Id, focused: bool) {
        let events = ADAPTERS.with(|a| {
            a.borrow_mut()
                .get_mut(&id)
                .and_then(|adapter| adapter.update_view_focus_state(focused))
        });
        if let Some(events) = events {
            events.raise();
        }
    }

    /// See [`super::detach`].
    pub fn detach(id: window::Id) {
        ADAPTERS.with(|a| a.borrow_mut().remove(&id));
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    //! No adapter here.

    use iced::window;

    use super::TreeUpdate;

    /// See [`super::attach`].
    pub fn attach(_: window::Id, _: isize) {}

    /// See [`super::update`].
    pub fn update(_: window::Id, _: TreeUpdate) {}

    /// See [`super::focus`].
    pub fn focus(_: window::Id, _: bool) {}

    /// See [`super::detach`].
    pub fn detach(_: window::Id) {}
}

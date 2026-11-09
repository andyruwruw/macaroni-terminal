//! Windows' modal move and resize loop for macaroni's frameless windows: starting it from the
//! custom title bar and edges, and holding a monitor's DPI change back until it ends.
//!
//! A drag reaches the window a moment after the click (through the app's update and winit's
//! thread executor). Started with the button already up, Windows' loop waits for a release that
//! never comes, so the window follows the cursor until the next click, snapping to wherever that
//! lands; or no loop starts, and winit, which only clears its own drag flag when a loop ends,
//! refuses every later drag of that window. [`drag`] starts the loop itself, only while the
//! button is still down, and ends it the way winit does.
//!
//! Dragged onto a monitor with a different scale, Windows sends `WM_DPICHANGED` mid-drag and
//! winit resizes the window around the cursor at once. The new size can tip the window back over
//! the boundary, rescaling it again, and it grows or shrinks and jumps under the cursor. Instead,
//! while a move is in progress the change is swallowed (the window keeps its size), and when the
//! move ends winit gets one `WM_DPICHANGED` for wherever it was dropped. A window that ended
//! snapped or maximized keeps the rectangle Windows gave it, which already fits that monitor.

#![allow(unsafe_code)]

use std::cell::RefCell;
use std::collections::HashMap;

use iced::window::Direction;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, ReleaseCapture, VK_LBUTTON, VK_RBUTTON,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, GetWindowRect, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT,
    HTCAPTION, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed, PostMessageW,
    SM_SWAPBUTTON, WM_DPICHANGED, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_LBUTTONUP, WM_NCDESTROY,
    WM_NCLBUTTONDOWN,
};

/// The subclass's id; the accessibility adapter uses 2.
const SUBCLASS: usize = 1;

/// One window's move loop state.
#[derive(Default)]
struct State {
    /// Inside the move or resize loop.
    moving: bool,
    /// The loop was started by [`drag`], so its end owes winit a button release.
    dragged: bool,
    /// The DPI winit last laid the window out for.
    applied: u32,
    /// The DPI Windows reported during the current move, 0 if none.
    pending: u32,
}

thread_local! {
    /// Each hooked window's state, by `HWND`. Only touched on the event loop's thread, which
    /// owns every window.
    static STATES: RefCell<HashMap<isize, State>> = RefCell::new(HashMap::new());
}

/// Subclasses a window so its drags start safely and DPI changes during them are deferred.
/// `window` is its `HWND`. Call once per window, on the thread that owns it (its event loop);
/// the app does it on `WindowOpened`.
pub fn install(window: isize) {
    let hwnd = window as HWND;
    // SAFETY: `hwnd` is a live window and this runs on its thread, as subclassing needs; the
    // subclass is removed on `WM_NCDESTROY`, before the window goes.
    unsafe {
        let applied = GetDpiForWindow(hwnd);
        STATES.with(|s| {
            s.borrow_mut().insert(
                window,
                State {
                    applied,
                    ..State::default()
                },
            )
        });
        SetWindowSubclass(hwnd, Some(proc), SUBCLASS, 0);
    }
}

/// Starts moving `window` (with `edge` `None`) or resizing it from `edge`, as a press on a
/// native title bar or border would, if the primary button is still down. Call on the event
/// loop's thread, from the press that asked for it.
pub fn drag(window: isize, edge: Option<Direction>) {
    if !primary_down() {
        return;
    }
    let hwnd = window as HWND;
    let hit = match edge {
        None => HTCAPTION,
        Some(Direction::North) => HTTOP,
        Some(Direction::South) => HTBOTTOM,
        Some(Direction::East) => HTRIGHT,
        Some(Direction::West) => HTLEFT,
        Some(Direction::NorthEast) => HTTOPRIGHT,
        Some(Direction::NorthWest) => HTTOPLEFT,
        Some(Direction::SouthEast) => HTBOTTOMRIGHT,
        Some(Direction::SouthWest) => HTBOTTOMLEFT,
    };
    with_state(window, |s| s.dragged = true);
    let mut cursor = POINT { x: 0, y: 0 };
    // SAFETY: `hwnd` is a live window owned by this thread, and the out pointer is valid.
    unsafe {
        GetCursorPos(&mut cursor);
        // The press gave the window the mouse capture, which would keep the loop from starting.
        ReleaseCapture();
        // `WM_NCLBUTTONDOWN` carries screen coordinates as two 16-bit words.
        let at = (cursor.x as u16 as u32 | ((cursor.y as u16 as u32) << 16)) as LPARAM;
        PostMessageW(hwnd, WM_NCLBUTTONDOWN, hit as WPARAM, at);
    }
}

/// Whether the primary mouse button (the left one, unless the buttons are swapped) is
/// physically down right now.
fn primary_down() -> bool {
    // SAFETY: both only read input state.
    unsafe {
        let button = if GetSystemMetrics(SM_SWAPBUTTON) != 0 {
            VK_RBUTTON
        } else {
            VK_LBUTTON
        };
        GetAsyncKeyState(button as i32) as u16 & 0x8000 != 0
    }
}

/// Runs `f` on `window`'s state, if it's hooked.
fn with_state<T>(window: isize, f: impl FnOnce(&mut State) -> T) -> Option<T> {
    STATES.with(|s| s.borrow_mut().get_mut(&window).map(f))
}

/// The subclass procedure: tracks the move loop, swallows `WM_DPICHANGED` during it, and on
/// `WM_EXITSIZEMOVE` replays one for the final DPI and releases the button for a loop [`drag`]
/// started. Everything else goes to the next procedure (winit's).
unsafe extern "system" fn proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    let key = hwnd as isize;
    // SAFETY: Windows calls this with valid arguments for `hwnd`'s messages, and `lparam` of
    // WM_DPICHANGED points at a RECT. This covers the whole function.
    unsafe {
        match msg {
            WM_ENTERSIZEMOVE => {
                with_state(key, |s| {
                    s.moving = true;
                    s.pending = 0;
                });
            }
            WM_DPICHANGED => {
                let dpi = wparam as u32 & 0xffff;
                let swallowed = with_state(key, |s| {
                    if s.moving {
                        s.pending = dpi;
                    } else {
                        s.applied = dpi;
                    }
                    s.moving
                });
                if swallowed == Some(true) {
                    return 0;
                }
            }
            WM_EXITSIZEMOVE => {
                let ended = with_state(key, |s| {
                    s.moving = false;
                    let dpi = std::mem::take(&mut s.pending);
                    let rescale = (dpi != 0 && dpi != s.applied).then_some((dpi, s.applied));
                    if rescale.is_some() {
                        s.applied = dpi;
                    }
                    (rescale, std::mem::take(&mut s.dragged))
                });
                // winit sees the end of the move first, then the DPI change, as if it had come
                // after the drop.
                let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                let Some((rescale, dragged)) = ended else {
                    return result;
                };
                if let Some((dpi, applied)) = rescale {
                    // Snapped or maximized, Windows already sized it for this monitor; scaling
                    // it again would pull it out of place.
                    let rect = if IsZoomed(hwnd) != 0 || is_arranged(hwnd) {
                        window_rect(hwnd)
                    } else {
                        scaled_around_cursor(hwnd, dpi as f64 / applied as f64)
                    };
                    // WM_DPICHANGED carries the DPI twice (x in the low word, y in the high).
                    let wparam = (dpi | (dpi << 16)) as WPARAM;
                    DefSubclassProc(hwnd, WM_DPICHANGED, wparam, &rect as *const RECT as LPARAM);
                }
                if dragged {
                    // The loop ate the release; without it iced would think the button was
                    // still held. winit does the same for its own drags.
                    PostMessageW(hwnd, WM_LBUTTONUP, 0, 0);
                }
                return result;
            }
            WM_NCDESTROY => {
                STATES.with(|s| s.borrow_mut().remove(&key));
                RemoveWindowSubclass(hwnd, Some(proc), SUBCLASS);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

/// Whether the window is snapped to part of its monitor. `IsWindowArranged` is looked up at
/// run time: only newer Windows 10 builds have it, and a static import would stop macaroni
/// starting on the others. Without it, this is false.
///
/// # Safety
///
/// `hwnd` must be a live window.
unsafe fn is_arranged(hwnd: HWND) -> bool {
    // SAFETY: user32 is always loaded in a GUI process, and the name is a NUL-terminated string;
    // the function, when found, has this signature.
    unsafe {
        let user32 = GetModuleHandleA(c"user32.dll".as_ptr().cast());
        let Some(found) = GetProcAddress(user32, c"IsWindowArranged".as_ptr().cast()) else {
            return false;
        };
        let arranged: unsafe extern "system" fn(HWND) -> i32 = std::mem::transmute(found);
        arranged(hwnd) != 0
    }
}

/// The window's rectangle on screen.
///
/// # Safety
///
/// `hwnd` must be a live window.
unsafe fn window_rect(hwnd: HWND) -> RECT {
    // SAFETY: `RECT` is plain integers, for which all zeroes is valid.
    let mut r: RECT = unsafe { std::mem::zeroed() };
    // SAFETY: valid window and out pointer.
    unsafe { GetWindowRect(hwnd, &mut r) };
    r
}

/// The window's rectangle scaled by `k`, keeping the point under the cursor in place: the
/// suggested rect a real `WM_DPICHANGED` would carry, so the window stays under the hand that
/// dropped it.
///
/// # Safety
///
/// `hwnd` must be a live window.
unsafe fn scaled_around_cursor(hwnd: HWND, k: f64) -> RECT {
    // SAFETY: the caller's guarantee.
    let r = unsafe { window_rect(hwnd) };
    let mut cursor = POINT { x: 0, y: 0 };
    // SAFETY: valid out pointer.
    unsafe { GetCursorPos(&mut cursor) };
    let scale = |edge: i32, anchor: i32| anchor + ((edge - anchor) as f64 * k).round() as i32;
    RECT {
        left: scale(r.left, cursor.x),
        top: scale(r.top, cursor.y),
        right: scale(r.right, cursor.x),
        bottom: scale(r.bottom, cursor.y),
    }
}

//! Holds a monitor's DPI change back until the user lets go of the window.
//!
//! Dragged onto a monitor with a different scale, Windows sends `WM_DPICHANGED` mid-drag and
//! winit resizes the window around the cursor at once. The new size can tip the window back over
//! the boundary, rescaling it again, and it grows or shrinks and jumps under the cursor. Instead,
//! while a move is in progress the change is swallowed (the window keeps its size), and when the
//! move ends winit gets one `WM_DPICHANGED` for wherever it was dropped.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowRect, WM_DPICHANGED, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE,
};

/// Only the main window is hooked, so one set of state is enough.
static MOVING: AtomicBool = AtomicBool::new(false);
/// The DPI winit last laid the window out for.
static APPLIED: AtomicU32 = AtomicU32::new(0);
/// The DPI Windows reported during the current move, 0 if none.
static PENDING: AtomicU32 = AtomicU32::new(0);

/// Subclasses the main window so DPI changes during a drag are deferred. `window` is its `HWND`.
/// Call once, on the thread that owns `window` (its event loop); the app does it on
/// `WindowOpened`.
pub fn install(window: isize) {
    let hwnd = window as HWND;
    // SAFETY: `hwnd` is the live main window and this runs on its thread, as subclassing needs.
    unsafe {
        APPLIED.store(GetDpiForWindow(hwnd), Ordering::Relaxed);
        SetWindowSubclass(hwnd, Some(proc), 1, 0);
    }
}

/// The subclass procedure: tracks the move loop, swallows `WM_DPICHANGED` during it, and on
/// `WM_EXITSIZEMOVE` replays one for the final DPI. Everything else goes to the next procedure
/// (winit's).
unsafe extern "system" fn proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    // SAFETY: Windows calls this with valid arguments for `hwnd`'s messages, and `lparam` of
    // WM_DPICHANGED points at a RECT. This covers the whole function.
    unsafe {
        match msg {
            WM_ENTERSIZEMOVE => {
                MOVING.store(true, Ordering::Relaxed);
                PENDING.store(0, Ordering::Relaxed);
            }
            WM_DPICHANGED if MOVING.load(Ordering::Relaxed) => {
                PENDING.store(wparam as u32 & 0xffff, Ordering::Relaxed);
                return 0;
            }
            WM_DPICHANGED => APPLIED.store(wparam as u32 & 0xffff, Ordering::Relaxed),
            WM_EXITSIZEMOVE => {
                MOVING.store(false, Ordering::Relaxed);
                // winit sees the end of the move first, then the DPI change, as if it had come
                // after the drop.
                let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                let dpi = PENDING.swap(0, Ordering::Relaxed);
                let applied = APPLIED.load(Ordering::Relaxed);
                if dpi != 0 && dpi != applied {
                    let rect = scaled_around_cursor(hwnd, dpi as f64 / applied as f64);
                    // WM_DPICHANGED carries the DPI twice (x in the low word, y in the high).
                    let wparam = (dpi | (dpi << 16)) as WPARAM;
                    APPLIED.store(dpi, Ordering::Relaxed);
                    DefSubclassProc(hwnd, WM_DPICHANGED, wparam, &rect as *const RECT as LPARAM);
                }
                return result;
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

/// The window's rectangle scaled by `k`, keeping the point under the cursor in place: the
/// suggested rect a real `WM_DPICHANGED` would carry, so the window stays under the hand that
/// dropped it.
///
/// # Safety
///
/// `hwnd` must be a live window.
unsafe fn scaled_around_cursor(hwnd: HWND, k: f64) -> RECT {
    // SAFETY: `RECT` is plain integers, for which all zeroes is valid.
    let mut r: RECT = unsafe { std::mem::zeroed() };
    let mut cursor = POINT { x: 0, y: 0 };
    // SAFETY: valid window and out pointers.
    unsafe {
        GetWindowRect(hwnd, &mut r);
        GetCursorPos(&mut cursor);
    }
    let scale = |edge: i32, anchor: i32| anchor + ((edge - anchor) as f64 * k).round() as i32;
    RECT {
        left: scale(r.left, cursor.x),
        top: scale(r.top, cursor.y),
        right: scale(r.right, cursor.x),
        bottom: scale(r.bottom, cursor.y),
    }
}

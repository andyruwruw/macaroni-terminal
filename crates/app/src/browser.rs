//! Browser tabs: the app side of `macaroni-browser`.
//!
//! A browser is a native child window laid over its pane, not something iced draws. The pane
//! body holds a [`BrowserHost`] placeholder that reports where it was laid out; after every
//! update [`crate::App::sync_browsers`] moves each browser over its placeholder, or hides it
//! when its tab isn't on screen. Like terminals, a browser starts the first time it's shown.
//!
//! Also here: [`prefer_gl`] (which `main` must call before iced creates its renderer), the
//! profile directory, the [`engine`] from the config, the address bar's [`resolve`], and
//! [`shortcuts`], which lets macaroni's keymap see keys pressed inside a page. Everything that
//! touches a web engine itself is in the `macaroni-browser` crate.

use std::sync::{Arc, Mutex};

use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Layout, Shell, layout, mouse, renderer};
use iced::keyboard::key::{Code, Named, Physical};
use iced::keyboard::{Key, Modifiers};
use iced::{Element, Event, Length, Rectangle, Size, Theme, window};
use macaroni_browser::{Bounds, Engine, KeyPress};
use macaroni_config::BrowserEngine;
use macaroni_core::BrowserId;

use crate::keymap::{Action, Keymap};

/// The engine `[browser] engine` picks for pages opened from now on.
pub fn engine(setting: BrowserEngine) -> Engine {
    match setting {
        BrowserEngine::System => Engine::System,
        BrowserEngine::Chromium => Engine::Chromium,
    }
}

/// Browsers are child windows, and on Windows DWM draws a DX12 or Vulkan (flip model) swap chain
/// over its window's children, hiding them. A GL swap chain leaves them visible, so iced renders
/// with GL there unless `WGPU_BACKEND` says otherwise.
pub fn prefer_gl() {
    if cfg!(windows) && std::env::var_os("WGPU_BACKEND").is_none() {
        // SAFETY: called first thing in main, before any other thread exists.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("WGPU_BACKEND", "gl");
        }
    }
}

/// Cookies and cache: `%LOCALAPPDATA%\macaroni\browser` on Windows, `~/Library/Application
/// Support/macaroni/browser` elsewhere (the temp dir if neither variable is set). A test run
/// with its own config dir gets its own profile too (`<config dir>/browser`): Chromium allows
/// one process per profile.
pub fn data_dir() -> std::path::PathBuf {
    local_data("browser")
}

/// `name` in macaroni's local (not roaming) data folder, as [`data_dir`] picks it.
pub(crate) fn local_data(name: &str) -> std::path::PathBuf {
    if std::env::var_os("MACARONI_CONFIG_DIR").is_some() {
        return macaroni_config::config_dir().join(name);
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)
    } else {
        std::env::var_os("HOME").map(|h| {
            std::path::PathBuf::from(h)
                .join("Library")
                .join("Application Support")
        })
    };
    base.unwrap_or_else(std::env::temp_dir)
        .join("macaroni")
        .join(name)
}

/// What a browser sends the app.
#[derive(Debug, Clone)]
pub enum BrowserMessage {
    /// Something happened in the page: navigation, title, loading state, focus, a new window.
    Event(macaroni_browser::Event),
    /// A macaroni shortcut pressed while the page had focus.
    Action(Action),
    /// Page text asked for over the control API: the request's id and the text.
    Text(u64, String),
}

/// The app's state for one browser tab, in `App.browsers`. Created when the tab is opened; the
/// browser itself starts later, the first time the tab is on screen.
pub struct Panel {
    /// The running browser; `None` until it starts, or if it couldn't. Dropping it closes the
    /// page.
    pub browser: Option<macaroni_browser::Browser>,
    /// The address bar's text: the page's address until the user edits it.
    pub address: String,
    /// Whether the page is loading (the reload button turns into a stop button).
    pub loading: bool,
    /// Whether there's history to go back to.
    pub back: bool,
    /// Whether there's history to go forward to.
    pub forward: bool,
    /// Where the pane body was laid out, in logical pixels.
    pub bounds: Option<Rectangle>,
    /// Why the browser couldn't start.
    pub error: Option<String>,
}

impl Panel {
    /// A tab for `url`, not started yet.
    pub fn new(url: &str) -> Self {
        Self {
            browser: None,
            address: url.to_owned(),
            loading: false,
            back: false,
            forward: false,
            bounds: None,
            error: None,
        }
    }
}

/// Logical `bounds` in physical pixels for placing the native window. Edges are rounded, not
/// sizes, so neighbouring rectangles still meet exactly.
pub fn physical(bounds: Rectangle, scale: f32) -> Bounds {
    let x = (bounds.x * scale).round() as i32;
    let y = (bounds.y * scale).round() as i32;
    Bounds {
        x,
        y,
        width: ((bounds.x + bounds.width) * scale).round() as i32 - x,
        height: ((bounds.y + bounds.height) * scale).round() as i32 - y,
    }
}

/// What the user typed in the address bar, as a URL: a search unless it looks like an address.
pub fn resolve(input: &str) -> String {
    let input = input.trim();
    // A scheme (`https://`, `about:`, `data:`), but not `localhost:3000`.
    let scheme = input.split_once(':').is_some_and(|(scheme, rest)| {
        !scheme.is_empty()
            && scheme.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
            && !rest.starts_with(|c: char| c.is_ascii_digit())
    });
    if scheme {
        return input.to_owned();
    }
    let host = input.split('/').next().unwrap_or_default();
    let looks_like_host = !input.contains(' ')
        && (host.contains('.') || host.starts_with("localhost") || host.contains(':'));
    if looks_like_host {
        let local = host.starts_with("localhost") || host.starts_with("127.");
        format!("{}://{input}", if local { "http" } else { "https" })
    } else {
        let query: String = input
            .bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                b' ' => "+".to_owned(),
                _ => format!("%{b:02X}"),
            })
            .collect();
        format!("https://www.google.com/search?q={query}")
    }
}

/// The handler a browser calls for keys pressed in the page: macaroni's shortcuts win, except
/// copy, paste and find, which belong to the page. Chromium runs it on its own UI thread, so the
/// keymap is shared behind a lock (`App.shared_keymap`, replaced on config reload) and a match goes back
/// through `send`. Returns whether the key was taken, so the page doesn't also see it.
pub fn shortcuts(
    keymap: Arc<Mutex<Keymap>>,
    send: impl Fn(Action) + Send + Sync + 'static,
) -> impl Fn(KeyPress) -> bool + Send + Sync + 'static {
    move |press| {
        let Some((key, physical)) = iced_key(press.code) else {
            return false;
        };
        let mut mods = Modifiers::empty();
        mods.set(Modifiers::CTRL, press.ctrl);
        mods.set(Modifiers::SHIFT, press.shift);
        mods.set(Modifiers::ALT, press.alt);
        mods.set(Modifiers::LOGO, press.meta);
        let action = keymap
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .lookup(&key, physical, mods);
        match action {
            None | Some(Action::Copy | Action::Paste | Action::Find) => false,
            Some(action) => {
                send(action);
                true
            }
        }
    }
}

/// A Windows virtual-key code as the iced key a US layout produces, for the keymap. Only the
/// keys default shortcuts use (letters, digits, Tab, arrows, `,` `[` `]`); `None` for the rest.
fn iced_key(vk: u32) -> Option<(Key, Physical)> {
    const LETTERS: [Code; 26] = [
        Code::KeyA,
        Code::KeyB,
        Code::KeyC,
        Code::KeyD,
        Code::KeyE,
        Code::KeyF,
        Code::KeyG,
        Code::KeyH,
        Code::KeyI,
        Code::KeyJ,
        Code::KeyK,
        Code::KeyL,
        Code::KeyM,
        Code::KeyN,
        Code::KeyO,
        Code::KeyP,
        Code::KeyQ,
        Code::KeyR,
        Code::KeyS,
        Code::KeyT,
        Code::KeyU,
        Code::KeyV,
        Code::KeyW,
        Code::KeyX,
        Code::KeyY,
        Code::KeyZ,
    ];
    const DIGITS: [Code; 10] = [
        Code::Digit0,
        Code::Digit1,
        Code::Digit2,
        Code::Digit3,
        Code::Digit4,
        Code::Digit5,
        Code::Digit6,
        Code::Digit7,
        Code::Digit8,
        Code::Digit9,
    ];
    let character =
        |c: char, code: Code| Some((Key::Character(c.to_string().into()), Physical::Code(code)));
    let named = |n: Named, code: Code| Some((Key::Named(n), Physical::Code(code)));
    match vk {
        0x41..=0x5A => {
            let i = (vk - 0x41) as usize;
            character((b'a' + i as u8) as char, LETTERS[i])
        }
        0x30..=0x39 => {
            let i = (vk - 0x30) as usize;
            character((b'0' + i as u8) as char, DIGITS[i])
        }
        0x09 => named(Named::Tab, Code::Tab),
        0x25 => named(Named::ArrowLeft, Code::ArrowLeft),
        0x26 => named(Named::ArrowUp, Code::ArrowUp),
        0x27 => named(Named::ArrowRight, Code::ArrowRight),
        0x28 => named(Named::ArrowDown, Code::ArrowDown),
        0xBC => character(',', Code::Comma),
        0xDB => character('[', Code::BracketLeft),
        0xDD => character(']', Code::BracketRight),
        _ => None,
    }
}

/// The empty pane body a browser window is laid over. Reports its bounds whenever they change.
///
/// It draws nothing; the page is a native window that covers it. Bounds are reported on
/// redraws, since `layout` can't publish messages.
pub struct BrowserHost<M> {
    /// The browser placed over this host.
    id: BrowserId,
    /// Builds the message (`Message::BrowserBounds`) from the host's logical bounds.
    on_bounds: Box<dyn Fn(Rectangle) -> M>,
}

impl<M> BrowserHost<M> {
    /// A placeholder for browser `id`, reporting its bounds with `on_bounds`.
    pub fn new(id: BrowserId, on_bounds: impl Fn(Rectangle) -> M + 'static) -> Self {
        Self {
            id,
            on_bounds: Box::new(on_bounds),
        }
    }
}

/// The host's state in the widget tree.
#[derive(Default)]
struct State {
    /// The last bounds reported, and for which browser: iced reuses state by tree position, so a
    /// different browser's host in this spot still reports.
    reported: Option<(BrowserId, Rectangle)>,
}

impl<M> Widget<M, Theme, iced::Renderer> for BrowserHost<M> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.max())
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        _viewport: &Rectangle,
    ) {
        if let Event::Window(window::Event::RedrawRequested(_)) = event {
            let state = tree.state.downcast_mut::<State>();
            let now = Some((self.id, layout.bounds()));
            if state.reported != now {
                state.reported = now;
                shell.publish((self.on_bounds)(layout.bounds()));
            }
        }
    }

    fn draw(
        &self,
        _tree: &Tree,
        _renderer: &mut iced::Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
    }
}

impl<'a, M: 'a> From<BrowserHost<M>> for Element<'a, M> {
    fn from(host: BrowserHost<M>) -> Self {
        Element::new(host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_bar_input_resolves_to_a_url() {
        assert_eq!(resolve("https://a.dev/x"), "https://a.dev/x");
        assert_eq!(resolve("example.com"), "https://example.com");
        assert_eq!(resolve("localhost:3000/app"), "http://localhost:3000/app");
        assert_eq!(resolve("127.0.0.1:8080"), "http://127.0.0.1:8080");
        assert_eq!(resolve("about:blank"), "about:blank");
        assert_eq!(
            resolve("data:text/html,<b>hi there</b>"),
            "data:text/html,<b>hi there</b>"
        );
        assert_eq!(
            resolve("rust iced & cef"),
            "https://www.google.com/search?q=rust+iced+%26+cef"
        );
    }

    #[test]
    fn physical_bounds_round_edges_not_sizes() {
        let b = physical(
            Rectangle::new((10.3, 20.6).into(), Size::new(100.4, 50.2)),
            1.5,
        );
        assert_eq!((b.x, b.y), (15, 31));
        assert_eq!(b.x + b.width, ((10.3 + 100.4) * 1.5f32).round() as i32);
    }
}

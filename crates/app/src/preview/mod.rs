//! Rendered Markdown, diagram files and media in a web page: how file tabs draw what iced
//! can't (diagrams and math with the downloaded [`crate::viewers`], raw HTML, video, sound,
//! animated pictures).
//!
//! The file tab builds the page's body off the UI thread ([`html`]) and hands it to the
//! [`server`], a small HTTP server on 127.0.0.1 behind a random token, which serves it with
//! the page's frame ([`head`] and [`tail`]), the viewers' scripts, the app's own script and
//! style (`assets/preview/`), and the files the document links to. The page itself is a
//! browser panel (`Panel::file`) laid over the tab like a browser tab's page.
//!
//! The page's script polls for a new version of the body and swaps it in place, keeping the
//! scroll position, and posts clicked links back to the app ([`crate::browser::BrowserMessage::Link`]).
//! A change to the frame (colors, fonts, installed viewers) reloads the page instead.

pub mod html;
pub mod server;

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::viewers::Extension;

/// Whether file tabs render in a web page here: where browser tabs exist.
pub const AVAILABLE: bool = cfg!(any(windows, target_os = "macos"));

/// The page's script, served at `<token>/app/preview.js`.
const SCRIPT: &str = include_str!("../../assets/preview/preview.js");

/// The page's style, served at `<token>/app/preview.css`.
const STYLE: &str = include_str!("../../assets/preview/preview.css");

/// The app's colors and fonts as the page uses them.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    /// The background is dark: diagrams use their dark themes.
    pub dark: bool,
    /// Body text.
    pub text: iced::Color,
    /// Quieter text (quotes, captions).
    pub muted: iced::Color,
    /// The page's background.
    pub background: iced::Color,
    /// Code blocks and table stripes.
    pub sunken: iced::Color,
    /// Rules and table lines.
    pub border: iced::Color,
    /// Links.
    pub link: iced::Color,
    /// Body text size in CSS pixels.
    pub size: f32,
    /// Code's size in CSS pixels.
    pub code_size: f32,
    /// The terminal's font family, for code.
    pub mono: String,
}

/// One page's content, as the file tab last published it.
#[derive(Debug, Clone)]
pub struct Page {
    /// The file shown; its folder is where relative links and images start from.
    pub document: PathBuf,
    /// The body's HTML ([`html::markdown`], [`html::diagram`] or [`html::media`]).
    pub body: String,
    /// The colors and fonts.
    pub look: Look,
    /// The viewers installed when the body was built.
    pub installed: Vec<Extension>,
    /// A diagram or media file: the drawing or player is the whole page.
    pub whole: bool,
}

impl Page {
    /// A fingerprint of everything in the frame: when it changes, the page reloads rather
    /// than swapping its body.
    pub fn frame_sig(&self) -> u64 {
        let mut hasher = std::hash::DefaultHasher::new();
        style_vars(&self.look).hash(&mut hasher);
        self.look.dark.hash(&mut hasher);
        self.installed.hash(&mut hasher);
        self.whole.hash(&mut hasher);
        self.folder().hash(&mut hasher);
        hasher.finish()
    }

    /// A fingerprint of the file shown: when it changes (the tab went to another file), the
    /// page starts again at the top.
    pub fn document_sig(&self) -> u64 {
        let mut hasher = std::hash::DefaultHasher::new();
        self.document.hash(&mut hasher);
        hasher.finish()
    }

    /// The document's folder.
    fn folder(&self) -> &Path {
        self.document.parent().unwrap_or(Path::new(""))
    }
}

/// `color` as CSS.
fn css(color: iced::Color) -> String {
    let [r, g, b, a] = color.into_rgba8();
    format!("rgba({r}, {g}, {b}, {:.3})", f32::from(a) / 255.0)
}

/// `color` as `#rrggbb`, which Graphviz reads (it ignores the alpha).
fn hex(color: iced::Color) -> String {
    let [r, g, b, _] = color.into_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The look as CSS custom properties on `:root`.
fn style_vars(look: &Look) -> String {
    // A family name with a quote or backslash in it can't break out of the string.
    let mono: String = look
        .mono
        .chars()
        .filter(|c| !matches!(c, '"' | '\\' | '<' | '>'))
        .collect();
    format!(
        ":root {{ --text: {}; --muted: {}; --background: {}; --sunken: {}; --border: {}; \
         --link: {}; --size: {:.1}px; --code-size: {:.1}px; --mono: \"{mono}\", ui-monospace, \
         Consolas, Menlo, monospace; color-scheme: {}; }}",
        css(look.text),
        css(look.muted),
        css(look.background),
        css(look.sunken),
        css(look.border),
        css(look.link),
        look.size,
        look.code_size,
        if look.dark { "dark" } else { "light" },
    )
}

/// `folder` as a URL path under `prefix/fs/`, ending in `/` so relative links resolve inside
/// it: each part percent-encoded, a Windows drive (`C:`) kept as the first part.
fn folder_url(prefix: &str, folder: &Path) -> String {
    let text = folder.to_string_lossy().replace('\\', "/");
    let mut url = format!("{prefix}/fs");
    for part in text.split('/').filter(|p| !p.is_empty()) {
        url.push('/');
        url.push_str(&encode(part));
    }
    url.push('/');
    url
}

/// `text` with everything but unreserved URL characters (and `:`, for drives) percent-encoded.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b':') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The page up to and including the body's opening: the base for relative links, the app's
/// style and the look. `prefix` is `/<token>`.
pub fn head(page: &Page, prefix: &str) -> String {
    let version = env!("CARGO_PKG_VERSION");
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <base href=\"{base}\">\
         <link rel=\"stylesheet\" href=\"{prefix}/app/preview.css?v={version}\">\
         <style>{vars}</style></head>\
         <body class=\"{class}\"><main id=\"mx-doc\">",
        base = html::escape(&folder_url(prefix, page.folder())),
        vars = style_vars(&page.look),
        class = if page.whole { "mx-only" } else { "mx-prose" },
    )
}

/// The page after the body: its settings and the app's script, both carrying `nonce` (the
/// only scripts the page's policy lets run). `page_path` is the page's own URL path.
pub fn tail(page: &Page, prefix: &str, page_path: &str, version: u64, nonce: &str) -> String {
    let viewers: serde_json::Map<String, serde_json::Value> = page
        .installed
        .iter()
        .map(|e| {
            (
                e.id().to_owned(),
                format!("{prefix}/ext/{}/", e.id()).into(),
            )
        })
        .collect();
    let settings = serde_json::json!({
        "page": page_path,
        "version": version.to_string(),
        "sig": page.frame_sig().to_string(),
        "doc": page.document_sig().to_string(),
        "dark": page.look.dark,
        "ink": hex(page.look.text),
        "viewers": viewers,
    });
    // `</` can't end the script early from inside the JSON.
    let settings = settings.to_string().replace("</", "<\\/");
    format!(
        "</main><script nonce=\"{nonce}\">window.MACARONI = {settings};</script>\
         <script nonce=\"{nonce}\" type=\"module\" src=\"{prefix}/app/preview.js?v={}\"></script>\
         </body></html>\n",
        env!("CARGO_PKG_VERSION"),
    )
}

/// The page's content security policy, letting only scripts carrying `nonce` (and what they
/// load) run, and keeping the page from reaching anything but this server, except images.
pub fn policy(nonce: &str) -> String {
    format!(
        "default-src 'none'; script-src 'nonce-{nonce}' 'strict-dynamic' 'unsafe-eval' \
         'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: \
         https: http:; media-src 'self' https:; font-src 'self' data:; connect-src 'self' \
         data: blob:; worker-src 'self' blob:; base-uri 'self'; form-action 'none'; \
         frame-ancestors 'none'"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page in `folder` with nothing installed.
    fn page(folder: &str) -> Page {
        Page {
            document: PathBuf::from(folder).join("README.md"),
            body: String::new(),
            look: Look {
                dark: true,
                text: iced::Color::WHITE,
                muted: iced::Color::WHITE,
                background: iced::Color::BLACK,
                sunken: iced::Color::BLACK,
                border: iced::Color::BLACK,
                link: iced::Color::WHITE,
                size: 14.0,
                code_size: 12.0,
                mono: "Fira \"Code\"".to_owned(),
            },
            installed: Vec::new(),
            whole: false,
        }
    }

    #[test]
    fn folders_become_base_urls() {
        assert_eq!(
            folder_url("/t", Path::new("C:\\Users\\me\\my docs")),
            "/t/fs/C:/Users/me/my%20docs/"
        );
        assert_eq!(
            folder_url("/t", Path::new("/Users/me/a#b")),
            "/t/fs/Users/me/a%23b/"
        );
    }

    #[test]
    fn the_frame_carries_the_nonce_and_settings() {
        let mut p = page("/docs");
        let head = head(&p, "/t");
        assert!(head.contains("<base href=\"/t/fs/docs/\">"));
        assert!(head.contains("--mono: \"Fira Code\""));
        let end = tail(&p, "/t", "/t/page/1", 3, "abc");
        assert!(end.contains("nonce=\"abc\""));
        assert!(end.contains("\"version\":\"3\""));
        let before = p.frame_sig();
        p.installed.push(Extension::Mermaid);
        assert_ne!(p.frame_sig(), before);
        assert!(tail(&p, "/t", "/t/page/1", 3, "abc").contains("/t/ext/mermaid/"));
        assert!(policy("abc").contains("'nonce-abc'"));
    }
}

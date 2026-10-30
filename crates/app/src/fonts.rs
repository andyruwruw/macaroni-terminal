//! Fonts: the bundled ones (JetBrains Mono and Symbols Nerd Font Mono, from `assets/fonts/`),
//! loaded into iced's global font system before anything measures text, and the lookup for a
//! configured `font.family`.
//!
//! [`crate::terminal`] draws with [`PRIMARY`] (or the configured family) and switches to
//! [`SYMBOLS`] for Private Use Area characters ([`is_private_use`]). Never rely on the system
//! "monospace" family: with a Japanese language pack, Windows resolves it to a font that draws
//! `\` as a yen sign.

use std::borrow::Cow;
use std::sync::Mutex;

use iced::Font;

/// The terminal's default font, bundled. Cell metrics are measured from it unless
/// `font.family` names another.
pub const PRIMARY: Font = Font::with_name("JetBrains Mono");
/// The bundled Nerd Font symbols, used for every Private Use Area character.
pub const SYMBOLS: Font = Font::with_name("Symbols Nerd Font Mono");
/// The app's name in the title bar: the UI's sans, semibold.
pub const LOGO: Font = Font {
    weight: iced::font::Weight::Semibold,
    ..Font::DEFAULT
};

/// The bundled font files, compiled in.
const BUNDLED: [&[u8]; 5] = [
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Italic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-BoldItalic.ttf"),
    include_bytes!("../assets/fonts/SymbolsNerdFontMono-Regular.ttf"),
];

/// Loads the bundled fonts synchronously, before anything measures text. `main` calls it first,
/// so [`crate::terminal::Metrics::measure`] sees them on the first frame.
///
/// # Panics
///
/// If iced's font system lock is poisoned.
pub fn load() {
    let mut fonts = iced::advanced::graphics::text::font_system()
        .write()
        .expect("font system lock");
    for bytes in BUNDLED {
        fonts.load_font(Cow::Borrowed(bytes));
    }
}

/// The font for `family`, or the bundled font with an explanation when it isn't installed.
/// `None` or a blank name means the bundled font with no message. Names match installed
/// families case-insensitively. The message joins the config errors shown in the banner.
///
/// # Panics
///
/// If iced's font system lock is poisoned.
pub fn resolve(family: Option<&str>) -> (Font, Option<String>) {
    let Some(family) = family.map(str::trim).filter(|f| !f.is_empty()) else {
        return (PRIMARY, None);
    };
    let installed = {
        let mut fonts = iced::advanced::graphics::text::font_system()
            .write()
            .expect("font system lock");
        fonts.raw().db().faces().any(|face| {
            face.families
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case(family))
        })
    };
    if installed {
        (Font::with_name(intern(family)), None)
    } else {
        (
            PRIMARY,
            Some(format!(
                "font.family: {family:?} isn't installed; using the bundled JetBrains Mono"
            )),
        )
    }
}

/// iced font names are `&'static str`. Each distinct name is leaked once, so reloading the
/// config repeatedly doesn't grow memory.
///
/// # Panics
///
/// If the name list's lock is poisoned.
fn intern(name: &str) -> &'static str {
    static NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut names = NAMES.lock().expect("font name lock");
    if let Some(existing) = names.iter().find(|n| **n == name) {
        return existing;
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    names.push(leaked);
    leaked
}

/// Nerd Font icons live in the Private Use Areas, where Windows icon fonts also put glyphs, so
/// fallback can't be trusted to pick the right font for them.
pub fn is_private_use(ch: char) -> bool {
    matches!(ch as u32, 0xe000..=0xf8ff | 0xf0000..=0x10ffff)
}

//! Fonts: the bundled ones (JetBrains Mono and Symbols Nerd Font Mono, from `assets/fonts/`),
//! loaded into iced's global font system before anything measures text, and the lookup for a
//! configured `font.family`.
//!
//! [`crate::terminal`] draws with [`PRIMARY`] (or the configured family), then for a character
//! that font lacks, the first `[font] fallback` family that has it ([`fallback_for`]), then
//! [`SYMBOLS`] for Private Use Area characters ([`is_private_use`]), then the system's fallback. Never rely on the system
//! "monospace" family: with a Japanese language pack, Windows resolves it to a font that draws
//! `\` as a yen sign.

use std::borrow::Cow;
use std::sync::{Arc, Mutex, RwLock};

use iced::advanced::graphics::text::cosmic_text;

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

/// `[font] fallback` resolved by [`set_fallback`]: the terminal font, then each listed family,
/// with the font data that says which characters it has. Empty without a list.
static FALLBACK: RwLock<Vec<(Font, Arc<cosmic_text::Font>)>> = RwLock::new(Vec::new());

/// Changes every time [`set_fallback`] does, so row layouts built with the old list are
/// rebuilt (it's part of [`crate::terminal::Metrics`]).
static FALLBACK_GENERATION: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Sets `[font] fallback` for terminal text drawn in `primary`: families tried, in order, for a
/// character `primary` lacks. Returns a warning for each family that isn't installed (it's
/// skipped).
///
/// # Panics
///
/// If iced's font system lock is poisoned.
pub fn set_fallback(primary: Font, families: &[String]) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut resolved = Vec::new();
    if !families.is_empty() {
        let mut fonts = iced::advanced::graphics::text::font_system()
            .write()
            .expect("font system lock");
        let system = fonts.raw();
        let mut load = |name: &str| {
            let face = system
                .db()
                .faces()
                .find(|face| {
                    face.families
                        .iter()
                        .any(|(family, _)| family.eq_ignore_ascii_case(name))
                })
                .map(|face| (face.id, face.weight))?;
            system.get_font(face.0, face.1)
        };
        let primary_name = match primary.family {
            iced::font::Family::Name(name) => name,
            _ => "JetBrains Mono",
        };
        if let Some(data) = load(primary_name) {
            resolved.push((primary, data));
        }
        for family in families.iter().map(|f| f.trim()).filter(|f| !f.is_empty()) {
            match load(family) {
                Some(data) => resolved.push((Font::with_name(intern(family)), data)),
                None => warnings.push(format!("font.fallback: {family:?} isn't installed")),
            }
        }
    }
    // Only the terminal font itself found: nothing to fall back to.
    if resolved.len() < 2 {
        resolved.clear();
    }
    let mut fallback = FALLBACK.write().unwrap_or_else(|e| e.into_inner());
    let changed = fallback.len() != resolved.len()
        || fallback
            .iter()
            .zip(&resolved)
            .any(|((a, _), (b, _))| a != b);
    *fallback = resolved;
    if changed {
        FALLBACK_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    warnings
}

/// Which [`set_fallback`] list is in effect, for telling cached layouts apart.
pub fn fallback_generation() -> u32 {
    FALLBACK_GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

/// The `[font] fallback` family to draw `ch` in: `None` when the terminal font has it, no
/// listed family does, or there's no list.
pub fn fallback_for(ch: char) -> Option<Font> {
    let fallback = FALLBACK.read().unwrap_or_else(|e| e.into_inner());
    let has = |data: &cosmic_text::Font| data.as_swash().charmap().map(ch) != 0;
    let (primary, rest) = fallback.split_first()?;
    if has(&primary.1) {
        return None;
    }
    rest.iter()
        .find(|(_, data)| has(data))
        .map(|(font, _)| *font)
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

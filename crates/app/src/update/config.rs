//! The config file, applied at launch and whenever it changes, and zooming the font.
//!
//! [`App::apply_config`] derives everything the config decides (keymap, palette, chrome, iced
//! theme, font metrics) in one place; `App::new` calls it at launch and
//! [`App::config_changed`] on every change the watcher reports.

use iced::Task;
use macaroni_config::{Config, CursorStyle};
use macaroni_term::CursorShape;

use crate::keymap::{Keymap, Zoom};
use crate::message::Message;
use crate::terminal::Metrics;
use crate::theme::{Chrome, Palette};
use crate::{App, MAX_FONT_SIZE, MIN_FONT_SIZE, fonts};

impl App {
    /// The config file changed: apply it, or show why it couldn't be loaded (keeping the config
    /// in effect).
    pub(crate) fn config_changed(&mut self) -> Task<Message> {
        match Config::load_or_init(&self.config_dir) {
            Ok(config) => self.apply_config(config),
            Err(err) => self.config_errors = vec![err.0],
        }
        Task::none()
    }

    /// Makes `config` current. Font and theme changes apply to every terminal immediately (the
    /// views re-measure and resize their sessions); shell and scrollback apply to new terminals.
    /// Replaces `config_errors` with the keymap's and font's problems.
    pub(crate) fn apply_config(&mut self, config: Config) {
        let mut errors = Vec::new();
        let (keymap, keymap_errors) = Keymap::new(&config.keybindings);
        errors.extend(keymap_errors);
        let (font, font_error) = fonts::resolve(config.font.family.as_deref());
        errors.extend(font_error);

        // A poisoned lock only means a browser thread panicked mid-lookup; the keymap inside is
        // still whole, and it's replaced wholesale here anyway.
        *self.shared_keymap.lock().unwrap_or_else(|e| e.into_inner()) = keymap.clone();
        self.keymap = keymap;
        // The chrome first: the palette's background follows the pane color it settles on.
        self.chrome = Chrome::new(&Palette::from_theme(&config.theme), &config);
        self.palette = Palette::from_theme(&config.theme).with_pane_background(&self.chrome);
        let colors = self.palette.default_colors();
        let cursor = (config.terminal.cursor_style != self.config.terminal.cursor_style)
            .then(|| cursor_shape(config.terminal.cursor_style));
        for running in self.running.values() {
            running.session.set_default_colors(colors.clone());
            if let Some(cursor) = cursor {
                running.session.set_default_cursor(cursor);
            }
        }
        self.iced_theme = self.chrome.iced_theme(&config.theme_name);
        self.metrics = Metrics::measure(
            font,
            zoomed(config.font.size, self.zoom),
            config.font.ligatures,
            config.font.line_height,
        );
        // A new configured width replaces the dragged one.
        if config.style.window.sidebar_width != self.config.style.window.sidebar_width {
            self.sidebar.width = None;
        }
        self.config = config;
        self.config_errors = errors;
        self.sync_sidebar();
    }

    /// Zooms every terminal's font a point in or out, or back to the configured size.
    pub(super) fn zoom_font(&mut self, zoom: Zoom) -> Task<Message> {
        self.zoom = match zoom {
            Zoom::In => self.zoom + 1.0,
            Zoom::Out => self.zoom - 1.0,
            Zoom::Reset => 0.0,
        };
        let size = zoomed(self.config.font.size, self.zoom);
        // Keep only the zoom that took effect, so zooming back from a limit responds at once.
        self.zoom = size - self.config.font.size;
        self.metrics = Metrics::measure(
            self.metrics.font,
            size,
            self.metrics.ligatures,
            self.metrics.line_height,
        );
        Task::none()
    }
}

/// The terminal's shape for a `[terminal] cursor_style`.
pub(super) fn cursor_shape(style: CursorStyle) -> CursorShape {
    match style {
        CursorStyle::Block => CursorShape::Block,
        CursorStyle::Underline => CursorShape::Underline,
        CursorStyle::Beam => CursorShape::Beam,
    }
}

/// The configured font size plus the zoom, kept to a usable size.
fn zoomed(size: f32, zoom: f32) -> f32 {
    (size + zoom).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
}

//! The config file, applied at launch and whenever it changes, and zooming the font.
//!
//! [`App::apply_config`] derives everything the config decides (keymap, palette, chrome, iced
//! theme, font metrics) in one place; `App::new` calls it at launch and
//! [`App::config_changed`] on every change the watcher reports, and
//! [`App::system_prefs_changed`] when the system settings `[accessibility]` follows change.

use iced::Task;
use macaroni_config::{Config, CursorStyle, HIGH_CONTRAST_THEME, Theme};
use macaroni_term::CursorShape;

use crate::accessibility::{Effective, SystemPrefs};
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

    /// The system's accessibility settings changed: apply the config again if they changed
    /// what's on.
    pub(crate) fn system_prefs_changed(&mut self, prefs: SystemPrefs) -> Task<Message> {
        self.system_prefs = prefs;
        if Effective::new(&self.config.accessibility, prefs) != self.access {
            self.apply_config(self.config.clone());
        }
        Task::none()
    }

    /// Makes `config` current. Font, theme and trigger changes apply to every terminal
    /// immediately (the views re-measure and resize their sessions); shell and scrollback apply
    /// to new terminals.
    /// Replaces `config_errors` with the keymap's and font's problems.
    pub(crate) fn apply_config(&mut self, config: Config) {
        let mut errors = Vec::new();
        let (keymap, keymap_errors) = Keymap::new(&config.keybindings);
        errors.extend(keymap_errors);
        let (font, font_error) = fonts::resolve(config.font.family.as_deref());
        errors.extend(font_error);
        // Before measuring: `Metrics` records which list is in effect.
        errors.extend(fonts::set_fallback(font, &config.font.fallback));

        // A poisoned lock only means a browser thread panicked mid-lookup; the keymap inside is
        // still whole, and it's replaced wholesale here anyway.
        *self.shared_keymap.lock().unwrap_or_else(|e| e.into_inner()) = keymap.clone();
        self.keymap = keymap;
        self.access = Effective::new(&config.accessibility, self.system_prefs);
        self.fades.instant = self.access.reduce_motion;
        let (theme, theme_name) = if self.access.high_contrast {
            // Can't fail: it's built in, and a test parses every built-in theme.
            let theme = Theme::builtin(HIGH_CONTRAST_THEME).expect("built-in theme");
            (theme, HIGH_CONTRAST_THEME)
        } else {
            (config.theme.clone(), config.theme_name.as_str())
        };
        // The chrome first: the palette's background follows the pane color it settles on.
        self.chrome = Chrome::new(&Palette::from_theme(&theme), &config);
        if self.access.high_contrast {
            self.chrome = self.chrome.high_contrast();
        }
        self.palette = Palette::from_theme(&theme).with_pane_background(&self.chrome);
        let colors = self.palette.default_colors();
        let cursor = (config.terminal.cursor_style != self.config.terminal.cursor_style)
            .then(|| cursor_shape(config.terminal.cursor_style));
        let triggers = (!self.triggers.same_rules(&config.triggers))
            .then(|| crate::triggers::Triggers::new(&config.triggers));
        for running in self.running.values() {
            running.session.set_default_colors(colors.clone());
            if let Some(cursor) = cursor {
                running.session.set_default_cursor(cursor);
            }
            if let Some(triggers) = &triggers {
                running.session.set_triggers(triggers.watched());
            }
        }
        if let Some(triggers) = triggers {
            self.triggers = triggers;
        }
        self.iced_theme = self.chrome.iced_theme(theme_name);
        let mut metrics = Metrics::measure(
            font,
            zoomed(config.font.size, self.zoom),
            config.font.ligatures,
            config.font.line_height,
        );
        // High contrast keeps text at AAA at least.
        metrics.minimum_contrast = if self.access.high_contrast {
            config.accessibility.minimum_contrast.max(7.0)
        } else {
            config.accessibility.minimum_contrast
        };
        metrics.cursor_thickness = config.accessibility.cursor_thickness;
        self.metrics = metrics;
        // iced reports no resize when only its scale changes, so windows' logical sizes and
        // positions are rescaled here.
        let rescale = self.config.accessibility.ui_scale / config.accessibility.ui_scale;
        if rescale != 1.0 {
            for win in self.wins.values_mut() {
                win.size = win.size * rescale;
                win.position = win
                    .position
                    .map(|p| iced::Point::new(p.x * rescale, p.y * rescale));
            }
        }
        // A new configured width replaces the dragged one.
        if config.style.window.sidebar_width != self.config.style.window.sidebar_width {
            self.sidebar.width = None;
        }
        self.config = config;
        self.config_errors = errors;
        self.config_errors
            .extend(self.config.plugin_problems.iter().cloned());
        // A plugin's command or settings may have changed.
        self.plugin_rows.stale();
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
        self.metrics = Metrics {
            minimum_contrast: self.metrics.minimum_contrast,
            cursor_thickness: self.metrics.cursor_thickness,
            ..Metrics::measure(
                self.metrics.font,
                size,
                self.metrics.ligatures,
                self.metrics.line_height,
            )
        };
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

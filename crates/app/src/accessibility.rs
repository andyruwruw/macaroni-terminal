//! Accessibility: what `[accessibility]` and the system's own settings turn on, and the screen
//! reader tree.
//!
//! - [`system`] reads the OS's reduce motion and high contrast settings, which `"system"`
//!   options follow. `App::subscription` polls them ([`POLL`]) while any option follows the
//!   system; a change reapplies the config.
//! - [`adapter`] and [`tree`] are screen reader support: a platform adapter per window, and
//!   the tree it's fed.
//! - [`bounds`] finds where items are on screen by their containers' widget ids.
//! - [`contrast`] keeps terminal text at `minimum_contrast` against its background.
//! - Reduced motion makes every fade finish at once (`anim::Fades`); high contrast swaps the
//!   terminal theme for [`macaroni_config::HIGH_CONTRAST_THEME`] and strengthens the chrome
//!   (`Chrome::high_contrast`); `ui_scale` is iced's scale factor for every window.

pub mod adapter;
pub mod bounds;
pub mod contrast;
pub mod system;
pub mod tree;

use std::time::Duration;

use macaroni_config::{AccessibilitySettings, Follow};

pub use system::SystemPrefs;

/// How often the system's settings are read while an option follows them. Reading them is a
/// couple of cheap system calls; a change shows up within this.
pub const POLL: Duration = Duration::from_secs(2);

/// What's on, with `"system"` options resolved against the system's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Effective {
    /// Fades and other movement finish at once.
    pub reduce_motion: bool,
    /// The high contrast chrome and terminal theme.
    pub high_contrast: bool,
}

impl Effective {
    /// `settings` with its `"system"` options answered by `system`.
    pub fn new(settings: &AccessibilitySettings, system: SystemPrefs) -> Self {
        Self {
            reduce_motion: settings.reduce_motion.resolve(system.reduce_motion),
            high_contrast: settings.high_contrast.resolve(system.high_contrast),
        }
    }
}

/// Whether any option follows the system, so its settings need watching.
pub fn follows_system(settings: &AccessibilitySettings) -> bool {
    settings.reduce_motion == Follow::System || settings.high_contrast == Follow::System
}

//! macaroni's settings: one TOML file (`config.toml`) plus theme files next to it, parsed,
//! checked and watched for changes.
//!
//! The crate knows nothing about the app. Keybinding actions stay strings and colors are its own
//! [`Rgb`] and [`Rgba`]; the app turns a [`Config`] into a keymap, a palette, chrome colors and
//! font metrics. [`config_dir`] finds the directory, [`Config::load_or_init`] reads it (writing
//! [`TEMPLATE`] on first run), [`watch()`] reports edits, and [`write_setting`] changes one key for
//! the settings dialog without disturbing the rest of the file. Panel plugins live in a
//! `plugins` folder next to the file (see [`Plugin`]).

mod chord;
mod edit;
mod plugins;
mod settings;
mod style;
mod theme;
mod watch;

pub use chord::{Chord, ChordError, ChordKey, Modifiers, NamedKey};
pub use edit::{Value, write_setting, write_ssh};
pub use plugins::{MAX_INTERVAL, MIN_INTERVAL, Plugin, Position, Run};
pub use settings::{
    AccessibilitySettings, AgentSettings, BrowserEngine, BrowserSettings, Config, CursorStyle,
    FileSettings, Follow, FontSettings, LoadError, NotificationSettings, OpenFiles, OpenLinks,
    SessionSettings, ShellSettings, SshProfile, TEMPLATE, TabItemSettings, TerminalSettings,
    Trigger, UiTheme, WorkspaceItemSettings, config_dir,
};
pub use style::{
    COLORS, ColorSetting, ColorValue, Colors, NumberSetting, SOLID_DEFAULTS, Style, THEME_COLORS,
    ThemeColor, Unit,
};
pub use theme::{BUILTIN_THEMES, DEFAULT_THEME, HIGH_CONTRAST_THEME, Rgb, Rgba, Theme};
pub use watch::{Watcher, watch};

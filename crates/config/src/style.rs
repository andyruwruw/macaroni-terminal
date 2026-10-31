//! The look of the app's own UI apart from the terminal theme: sizes, spacing, corner radii,
//! border widths and text sizes by part of the window (`[style.*]`), and color overrides for
//! each interface (`[colors.solid]`, `[colors.glass]`). Each table is declared once here, with
//! its defaults, limits and the labels the settings dialog shows.
//!
//! Three macros keep each declaration in one place. `numbers!` turns one table's list of
//! fields into its serde struct, `Default`, range check and [`NumberSetting`] rows;
//! `styles!` gathers the tables into [`Style`]; `colors!` turns the list of color names into
//! [`COLORS`]. [`crate::Config::parse`] runs the checks, the settings dialog lists
//! [`Style::settings`] and [`COLORS`], and the app reads the values when it builds its chrome.
//! Color defaults are not here: unset names keep the interface's own color, defined in the app.

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;

use crate::UiTheme;
use crate::theme::Rgba;

/// A number under `[style.*]`, as the settings dialog lists it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberSetting {
    /// Dotted, e.g. `style.tabs.height`.
    pub key: &'static str,
    /// The table's title, e.g. "Tabs".
    pub group: &'static str,
    /// The row's name in the dialog, e.g. "Height".
    pub label: &'static str,
    /// A longer explanation under the label; empty when the label says it all.
    pub description: &'static str,
    /// The value when the key isn't set.
    pub default: f32,
    /// The smallest value [`crate::Config::parse`] accepts.
    pub min: f32,
    /// The largest value [`crate::Config::parse`] accepts.
    pub max: f32,
    /// How much one step of the dialog's control changes the value.
    pub step: f32,
    /// What the value measures, which the dialog shows beside it.
    pub unit: Unit,
}

/// What a `[style.*]` number measures. The file always holds the raw value; [`Unit::scale`]
/// is only for showing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// Logical pixels.
    Px,
    /// A fraction from 0 to 1, shown as a percentage.
    Percent,
    /// A count of characters.
    Chars,
    /// A multiplier.
    Times,
    /// A count of lines.
    Lines,
    /// Points, for font sizes.
    Points,
}

impl Unit {
    /// What's shown after the value, e.g. `px`.
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Px => "px",
            Self::Percent => "%",
            Self::Chars => "chars",
            Self::Times => "\u{d7}",
            Self::Lines => "lines",
            Self::Points => "pt",
        }
    }

    /// What the stored value is multiplied by to show it: 100 for a percentage, else 1.
    pub fn scale(self) -> f32 {
        match self {
            Self::Percent => 100.0,
            _ => 1.0,
        }
    }
}

/// Declares one `[style.<table>]` table: `numbers! { Name, "table", "Title" { field: default,
/// min, max, step, Unit, "Label", "Description"; ... } }`, with optional attributes (docs)
/// before `Name`, where `Unit` is a [`Unit`] variant.
///
/// Generates a `pub struct Name` of `pub f32` fields (each documented from its label,
/// description and range), deserialized with `serde(default, deny_unknown_fields)` so a missing
/// key takes its default and a misspelled one is an error. Also generates `Default`,
/// `Name::SETTINGS` (one [`NumberSetting`] per field, in order), a private `get` by field name,
/// and a private `check` that reports the first field outside its range as
/// `style.<table>.<field> must be between <min> and <max>, got <value>`.
macro_rules! numbers {
    (
        $(#[$meta:meta])*
        $name:ident, $table:literal, $title:literal {
            $($field:ident: $default:expr, $min:expr, $max:expr, $step:expr, $unit:ident, $label:expr, $description:expr;)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        pub struct $name {
            $(
                #[doc = $label]
                #[doc = ""]
                #[doc = $description]
                #[doc = ""]
                #[doc = concat!(
                    "Default ", stringify!($default), ", from ", stringify!($min), " to ",
                    stringify!($max), "."
                )]
                pub $field: f32,
            )*
        }

        impl Default for $name {
            fn default() -> Self {
                Self {
                    $($field: $default,)*
                }
            }
        }

        impl $name {
            /// This table's numbers, in declaration order, as the settings dialog lists them.
            pub const SETTINGS: &'static [NumberSetting] = &[$(NumberSetting {
                key: concat!("style.", $table, ".", stringify!($field)),
                group: $title,
                label: $label,
                description: $description,
                default: $default,
                min: $min,
                max: $max,
                step: $step,
                unit: Unit::$unit,
            },)*];

            /// The value of the field named `field` (no table prefix); `None` if there's no such
            /// field.
            fn get(&self, field: &str) -> Option<f32> {
                match field {
                    $(stringify!($field) => Some(self.$field),)*
                    _ => None,
                }
            }

            /// Sets the field named `field` (no table prefix); ignored if there's no such field.
            fn set(&mut self, field: &str, value: f32) {
                match field {
                    $(stringify!($field) => self.$field = value,)*
                    _ => {}
                }
            }

            /// Fails with a readable message on the first field outside its range.
            fn check(&self) -> Result<(), String> {
                $(if !($min..=$max).contains(&self.$field) {
                    return Err(format!(
                        "style.{}.{} must be between {} and {}, got {}",
                        $table,
                        stringify!($field),
                        $min,
                        $max,
                        self.$field
                    ));
                })*
                Ok(())
            }
        }
    };
}

/// Declares [`Style`] from `field: TableType` pairs, where each `TableType` came from
/// `numbers!` and `field` is the table's name under `[style]` (so `tabs: TabStyle` is
/// `[style.tabs]`). Generates the struct, [`Style::settings`], [`Style::get`] and the crate's
/// range check over every table.
macro_rules! styles {
    ($($table:ident: $ty:ident,)*) => {
        /// Every `[style.*]` table.
        #[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        pub struct Style {
            $(
                #[doc = concat!("`[style.", stringify!($table), "]`.")]
                pub $table: $ty,
            )*
        }

        impl Style {
            /// Every number, table by table.
            pub fn settings() -> impl Iterator<Item = &'static NumberSetting> {
                [$($ty::SETTINGS,)*].into_iter().flatten()
            }

            /// The value for a dotted key, e.g. `style.tabs.height`; `None` for a key that isn't
            /// a style setting.
            pub fn get(&self, key: &str) -> Option<f32> {
                let (table, field) = key.strip_prefix("style.")?.split_once('.')?;
                match table {
                    $(stringify!($table) => self.$table.get(field),)*
                    _ => None,
                }
            }

            /// Sets the value for a dotted key; ignored for a key that isn't a style setting.
            fn set(&mut self, key: &str, value: f32) {
                let Some((table, field)) =
                    key.strip_prefix("style.").and_then(|k| k.split_once('.'))
                else {
                    return;
                };
                match table {
                    $(stringify!($table) => self.$table.set(field, value),)*
                    _ => {}
                }
            }

            /// Checks every table's ranges; the error names the first bad key.
            pub(crate) fn check(&self) -> Result<(), String> {
                $(self.$table.check()?;)*
                Ok(())
            }
        }
    };
}

/// The solid interface's own defaults, where they differ from the glass ones in the tables
/// below: everything touching, with one shared line between parts (no gaps under the title bar,
/// beside the sidebar, below or between tabs, between panes or around them), square tabs and
/// panes, and compact workspace rows.
pub const SOLID_DEFAULTS: &[(&str, f32)] = &[
    ("style.window.title_bar_gap", 0.0),
    ("style.sidebar.padding", 8.0),
    ("style.sidebar.panes_gap", 0.0),
    ("style.workspaces.padding", 4.0),
    ("style.workspaces.radius", 10.0),
    ("style.workspaces.add_radius", 2.0),
    ("style.workspaces.spacing", 0.0),
    ("style.workspaces.tile_size", 27.0),
    ("style.tabs.radius", 0.0),
    ("style.tabs.add_radius", 0.0),
    ("style.tabs.spacing", 0.0),
    ("style.tabs.gap", 0.0),
    ("style.panes.radius", 0.0),
    ("style.panes.gap", 0.0),
    ("style.panes.margin", 0.0),
    ("style.menus.radius", 4.0),
    ("style.controls.radius", 3.0),
];

impl NumberSetting {
    /// The value when the key isn't set, for interface `ui`: [`SOLID_DEFAULTS`] in solid,
    /// else [`NumberSetting::default`].
    pub fn default_for(&self, ui: UiTheme) -> f32 {
        match ui {
            UiTheme::Solid => SOLID_DEFAULTS
                .iter()
                .find(|(key, _)| *key == self.key)
                .map_or(self.default, |(_, value)| *value),
            UiTheme::Glass => self.default,
        }
    }
}

impl Style {
    /// Gives every [`SOLID_DEFAULTS`] key that `table` (the parsed file) doesn't set its solid
    /// value. Serde has already filled in the glass defaults, so only the file says which
    /// values the user chose.
    pub(crate) fn apply_solid_defaults(&mut self, table: &toml::Table) {
        for (key, value) in SOLID_DEFAULTS {
            let mut node = Some(table);
            let mut parts = key.split('.').peekable();
            let mut set = false;
            while let Some(part) = parts.next() {
                let Some(current) = node else { break };
                if parts.peek().is_none() {
                    set = current.contains_key(part);
                } else {
                    node = current.get(part).and_then(toml::Value::as_table);
                }
            }
            if !set {
                self.set(key, *value);
            }
        }
    }
}

/// A color under `[colors.solid]` and `[colors.glass]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorSetting {
    /// The key inside either table, e.g. `text`.
    pub name: &'static str,
    /// The row's name in the settings dialog.
    pub label: &'static str,
    /// The dialog section it's listed under, e.g. "Accents".
    pub group: &'static str,
    /// A longer explanation under the label.
    pub description: &'static str,
    /// The [`THEME_COLORS`] name it follows while it isn't set; empty when it keeps a color of
    /// its own.
    pub link: &'static str,
    /// Only the glass interface draws it.
    pub glass_only: bool,
    /// `colors.solid.<name>`, precomputed so [`ColorSetting::key`] can return a `&'static str`.
    solid_key: &'static str,
    /// `colors.glass.<name>`, as for `solid_key`.
    glass_key: &'static str,
}

impl ColorSetting {
    /// Its dotted key for `ui`'s table, e.g. `colors.solid.text`.
    pub fn key(&self, ui: UiTheme) -> &'static str {
        match ui {
            UiTheme::Solid => self.solid_key,
            UiTheme::Glass => self.glass_key,
        }
    }
}

/// A shared color that several [`COLORS`] follow (see [`ColorSetting::link`]), set in the same
/// `[colors.*]` tables under its own name. An element that's set itself, to a color or to
/// another theme color's name, no longer follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeColor {
    /// The key inside either table, e.g. `accent`. No [`COLORS`] name is the same.
    pub name: &'static str,
    /// The row's name in the settings dialog.
    pub label: &'static str,
    /// What follows it, under the label.
    pub description: &'static str,
    /// `colors.solid.<name>`.
    solid_key: &'static str,
    /// `colors.glass.<name>`.
    glass_key: &'static str,
}

impl ThemeColor {
    /// Its dotted key for `ui`'s table, e.g. `colors.solid.accent`.
    pub fn key(&self, ui: UiTheme) -> &'static str {
        match ui {
            UiTheme::Solid => self.solid_key,
            UiTheme::Glass => self.glass_key,
        }
    }
}

/// Declares [`COLORS`] from `name, "Label", "Group", "Description", "link", glass_only;` rows,
/// in the order the settings dialog lists them. The `name` is the key in both `[colors.*]`
/// tables.
macro_rules! colors {
    ($($name:ident, $label:expr, $group:expr, $description:expr, $link:expr, $glass:expr;)*) => {
        /// Every color the app's UI draws with, in the order the settings dialog lists them.
        pub const COLORS: &[ColorSetting] = &[$(ColorSetting {
            name: stringify!($name),
            label: $label,
            group: $group,
            description: $description,
            link: $link,
            glass_only: $glass,
            solid_key: concat!("colors.solid.", stringify!($name)),
            glass_key: concat!("colors.glass.", stringify!($name)),
        },)*];
    };
}

/// Declares [`THEME_COLORS`] from `name, "Label", "Description";` rows.
macro_rules! theme_colors {
    ($($name:ident, $label:expr, $description:expr;)*) => {
        /// The shared colors, in the order the settings dialog lists them.
        pub const THEME_COLORS: &[ThemeColor] = &[$(ThemeColor {
            name: stringify!($name),
            label: $label,
            description: $description,
            solid_key: concat!("colors.solid.", stringify!($name)),
            glass_key: concat!("colors.glass.", stringify!($name)),
        },)*];
    };
}

/// A value in a `[colors.*]` table: `"#rrggbb"` / `"#rrggbbaa"`, or the name of a
/// [`THEME_COLORS`] entry to follow (which [`crate::Config::parse`] checks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorValue {
    /// A color of its own.
    Rgba(Rgba),
    /// Follow this theme color.
    Theme(String),
}

impl fmt::Display for ColorValue {
    /// As it's written in the file: the hex color, or the theme color's name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rgba(rgba) => rgba.fmt(f),
            Self::Theme(name) => f.write_str(name),
        }
    }
}

impl<'de> Deserialize<'de> for ColorValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        if s.trim().starts_with('#') {
            Rgba::parse(&s).map(Self::Rgba).ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "expected a color like \"#1e222a\" or \"#1e222a80\", got {s:?}"
                ))
            })
        } else {
            Ok(Self::Theme(s.trim().to_owned()))
        }
    }
}

/// `[colors.solid]` and `[colors.glass]`: overrides by name, for each interface. Both
/// [`COLORS`] and [`THEME_COLORS`] names go here.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Colors {
    /// Overrides for the solid interface, by name.
    pub solid: BTreeMap<String, ColorValue>,
    /// Overrides for the glass interface, by name.
    pub glass: BTreeMap<String, ColorValue>,
}

impl Colors {
    /// The overrides for one interface.
    pub fn for_ui(&self, ui: UiTheme) -> &BTreeMap<String, ColorValue> {
        match ui {
            UiTheme::Solid => &self.solid,
            UiTheme::Glass => &self.glass,
        }
    }

    /// Fails on the first name in either table that isn't in [`COLORS`] or [`THEME_COLORS`],
    /// on a reference to a theme color that doesn't exist, and on a theme color that refers to
    /// another (they're the end of the chain). Serde can't catch these (the tables are maps),
    /// so a typo would otherwise be silently ignored.
    pub(crate) fn check(&self) -> Result<(), String> {
        let is_theme = |n: &str| THEME_COLORS.iter().any(|t| t.name == n);
        for (table, colors) in [("solid", &self.solid), ("glass", &self.glass)] {
            for (name, value) in colors {
                let element = COLORS.iter().any(|c| c.name == *name);
                if !element && !is_theme(name) {
                    return Err(format!("colors.{table}: unknown color {name:?}"));
                }
                if let ColorValue::Theme(target) = value {
                    if !element {
                        return Err(format!(
                            "colors.{table}.{name}: a theme color must be \"#rrggbb\", got {target:?}"
                        ));
                    }
                    if !is_theme(target) {
                        let names: Vec<&str> = THEME_COLORS.iter().map(|t| t.name).collect();
                        return Err(format!(
                            "colors.{table}.{name}: expected \"#rrggbb\" or one of {}, got {target:?}",
                            names.join(", ")
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

numbers! {
    /// `[style.window]`: the title bar, its window buttons, logo and workspace name, the sidebar's
    /// width and the resize border.
    WindowStyle, "window", "Window" {
        sidebar_width: 236.0, 120.0, 600.0, 1.0, Px, "Sidebar width", "How wide the workspace list on the left is.";
        title_bar_height: 36.0, 24.0, 72.0, 1.0, Px, "Title bar height", "Windows draws its own title bar.";
        title_bar_border_width: 1.0, 0.0, 6.0, 0.5, Px, "Title bar edge width", "The line along the title bar's bottom, in the title bar edge color (clear by default).";
        title_bar_gap: 12.0, 0.0, 40.0, 1.0, Px, "Space below the title bar", "Between the title bar and the tabs. 0 puts the tabs right under it.";
        caption_button_width: 42.0, 28.0, 80.0, 1.0, Px, "Window button width", "Settings, minimize, maximize and close.";
        caption_icon_size: 14.0, 8.0, 24.0, 0.5, Px, "Window button icon size", "The icons inside the window buttons.";
        logo_size: 13.0, 6.0, 32.0, 0.5, Px, "Logo size", "The macaroni logo at the left of the title bar.";
        logo_padding: 12.0, 0.0, 40.0, 1.0, Px, "Space beside the logo", "Left and right of the logo and app name.";
        app_name_size: 14.0, 8.0, 28.0, 0.5, Px, "App name text size", "The name next to the logo.";
        workspace_name_size: 14.0, 8.0, 28.0, 0.5, Px, "Workspace name text size", "The active workspace's name in the title bar.";
        workspace_icon_size: 16.0, 8.0, 32.0, 0.5, Px, "Workspace icon size", "Next to its name in the title bar.";
        resize_border: 5.0, 1.0, 16.0, 1.0, Px, "Resize border", "How far in from the window's edges you can grab to resize it.";
    }
}

numbers! {
    /// `[style.sidebar]`: the sidebar's padding and spacing.
    SidebarStyle, "sidebar", "Sidebar" {
        top_padding: 12.0, 0.0, 60.0, 1.0, Px, "Top padding", "Above the search bar, or the first workspace without one. As much as Space below the title bar lines the search bar up with the tabs.";
        padding: 12.0, 0.0, 40.0, 1.0, Px, "Side padding", "Left and right of the sidebar's contents.";
        bottom_padding: 8.0, 0.0, 60.0, 1.0, Px, "Bottom padding", "Below the New workspace button, at the end of the list.";
        search_height: 36.0, 16.0, 64.0, 1.0, Px, "Search bar height", "As tall as a tab by default.";
        search_padding: 10.0, 0.0, 40.0, 1.0, Px, "Search bar padding", "Inside the search bar, left and right of its icon and text.";
        search_gap: 10.0, 0.0, 40.0, 1.0, Px, "Space below the search bar", "Between the search bar and the first workspace. As much as Space below the tabs lines the workspaces up with the panes.";
        spacing: 6.0, 0.0, 30.0, 1.0, Px, "Space between items", "Between workspace rows and group headers.";
        new_workspace_gap: 8.0, 0.0, 40.0, 1.0, Px, "Space above New workspace", "Between the last workspace and the New workspace button.";
        panes_gap: 4.0, 0.0, 60.0, 1.0, Px, "Space before the panes", "Between the sidebar and the panes. 0 puts them right against it, and its edge and theirs become one line.";
        border_width: 1.0, 0.0, 6.0, 0.5, Px, "Edge width", "The line down the sidebar's right side, in the sidebar edge color (clear by default).";
    }
}

numbers! {
    /// `[style.workspaces]`: workspace rows in the sidebar.
    WorkspaceStyle, "workspaces", "Workspaces" {
        padding: 8.0, 0.0, 30.0, 1.0, Px, "Padding", "Inside each workspace row, around its contents.";
        radius: 8.0, 0.0, 24.0, 1.0, Px, "Corner radius", "How round a workspace row's corners are.";
        add_radius: 8.0, 0.0, 24.0, 1.0, Px, "+ button corner radius", "How round the New workspace button's corners are.";
        spacing: 8.0, 0.0, 24.0, 1.0, Px, "Space between parts", "Between the icon, the text, the badge and the close button.";
        line_spacing: 2.0, 0.0, 12.0, 1.0, Px, "Space between title and detail", "The gap between a row's name and the line under it.";
        tile_size: 32.0, 16.0, 64.0, 1.0, Px, "Icon tile size", "The square behind each workspace's icon.";
        tile_radius: 8.0, 0.0, 32.0, 1.0, Px, "Icon tile corner radius", "How round the icon tile's corners are.";
        icon_size: 16.0, 8.0, 40.0, 0.5, Px, "Icon size", "The workspace's icon inside its tile.";
        title_size: 13.0, 7.0, 24.0, 0.5, Px, "Title text size", "The workspace's name.";
        detail_size: 11.0, 7.0, 20.0, 0.5, Px, "Detail text size", "The line under the name.";
        badge_size: 11.0, 7.0, 20.0, 0.5, Px, "Badge text size", "The shortcut number or count on the right.";
        mark_size: 12.0, 6.0, 20.0, 0.5, Px, "Muted and pinned mark size", "The small bell and pin icons.";
        max_detail_chars: 30.0, 4.0, 120.0, 1.0, Chars, "Longest detail", "Longer ones are cut short with an ellipsis.";
    }
}

numbers! {
    /// `[style.groups]`: workspace group headers in the sidebar.
    GroupStyle, "groups", "Groups" {
        indent: 12.0, 0.0, 48.0, 1.0, Px, "Indent", "Between a group's line (under the start of its icon) and its workspaces.";
        padding: 6.0, 0.0, 20.0, 1.0, Px, "Header padding", "Above and below the header's name.";
        spacing: 8.0, 0.0, 24.0, 1.0, Px, "Space between parts", "Between the chevron, the name, the marks and the count.";
        name_size: 12.0, 7.0, 24.0, 0.5, Px, "Name text size", "The group's name in its header.";
        count_size: 11.0, 7.0, 20.0, 0.5, Px, "Mark size", "The bell and pin in a group header, sized like text this big.";
        chevron_size: 14.0, 6.0, 28.0, 0.5, Px, "Chevron size", "The arrow that folds and unfolds the group.";
    }
}

numbers! {
    /// `[style.tabs]`: the tab strip at the top of each pane.
    TabStyle, "tabs", "Tabs" {
        height: 36.0, 24.0, 64.0, 1.0, Px, "Height", "Pinned tabs are this wide too.";
        padding_left: 10.0, 0.0, 30.0, 1.0, Px, "Space before the icon", "Left padding, between the tab's left edge and its icon. The title bar's workspace name is indented to match.";
        padding_right: 6.0, 0.0, 30.0, 1.0, Px, "Space after the close button", "Right padding, between the close button and the tab's right edge.";
        radius: 8.0, 0.0, 24.0, 1.0, Px, "Corner radius", "How round a tab's corners are.";
        add_radius: 8.0, 0.0, 24.0, 1.0, Px, "+ button corner radius", "How round the New tab button's corners are.";
        accent_width: 2.0, 0.0, 8.0, 0.5, Px, "Open tab line", "How thick the colored line along the top of each pane's open tab is, in solid. 0 turns it off.";
        spacing: 6.0, 0.0, 24.0, 1.0, Px, "Space between tabs", "Also before the + button. At 0 in solid, two tabs' borders are one line.";
        gap: 10.0, 0.0, 40.0, 1.0, Px, "Space below the tabs", "Between the tabs and their pane. The scrollbar sits in it. At 0 in solid, with a pane border, the tabs sit on the pane's top border and the open tab opens into the pane.";
        part_spacing: 8.0, 0.0, 24.0, 1.0, Px, "Space between parts", "Between the icon, the title, the badge and the close button.";
        min_width: 110.0, 40.0, 400.0, 1.0, Px, "Narrowest tab", "Past this the tabs scroll.";
        max_title_chars: 28.0, 4.0, 120.0, 1.0, Chars, "Longest title", "Longer ones are cut short with an ellipsis.";
        icon_size: 16.0, 8.0, 32.0, 0.5, Px, "Icon size", "The terminal, browser or agent icon.";
        title_size: 12.5, 7.0, 24.0, 0.5, Px, "Title text size", "The tab's name.";
        badge_size: 11.0, 7.0, 20.0, 0.5, Px, "Badge text size", "The small text after the title.";
        status_size: 11.0, 7.0, 20.0, 0.5, Px, "Tooltip detail text size", "What a tab's agent is doing, under its name on hover.";
        scrollbar_width: 3.0, 1.0, 10.0, 0.5, Px, "Scrollbar thickness", "The bar under the tabs when they don't all fit.";
    }
}

numbers! {
    /// `[style.panes]`: pane cards, the gaps between them and their outlines.
    PaneStyle, "panes", "Panes" {
        gap: 8.0, 0.0, 40.0, 1.0, Px, "Space between panes", "Dividers are dragged from it. At 0 in solid, two panes' borders are one line.";
        margin: 8.0, 0.0, 40.0, 1.0, Px, "Space at the right and bottom", "Between the panes and the window's right and bottom edges. 0 puts them right against them.";
        radius: 16.0, 0.0, 40.0, 1.0, Px, "Corner radius", "How round pane cards' corners are.";
        padding: 6.0, 0.0, 30.0, 1.0, Px, "Padding", "Keeps terminal text and browser pages inside the rounded corners.";
        terminal_padding: 6.0, 0.0, 40.0, 1.0, Px, "Terminal padding", "Around the text, inside the pane's padding.";
        border_width: 1.0, 0.0, 6.0, 0.5, Px, "Border width", "The solid interface's border around each pane.";
        focus_border_width: 1.0, 0.0, 6.0, 0.5, Px, "Focused pane outline", "Shown when the workspace has more than one pane.";
        attention_border_width: 1.5, 0.0, 6.0, 0.5, Px, "Attention outline", "Around a pane with a tab that wants you.";
        address_size: 12.0, 8.0, 24.0, 0.5, Px, "Address bar text size", "In browser tabs.";
    }
}

numbers! {
    /// `[style.menus]`: context menus, submenus and the icon and color pickers.
    MenuStyle, "menus", "Menus" {
        width: 210.0, 120.0, 480.0, 1.0, Px, "Width", "How wide right-click menus are.";
        item_height: 30.0, 18.0, 60.0, 1.0, Px, "Item height", "The height of each entry.";
        padding: 5.0, 0.0, 20.0, 1.0, Px, "Padding", "Around a menu's entries, inside its edge.";
        item_padding: 10.0, 0.0, 30.0, 1.0, Px, "Item padding", "Left and right of each item and heading.";
        spacing: 10.0, 0.0, 30.0, 1.0, Px, "Space between icon and text", "In each entry.";
        radius: 10.0, 0.0, 24.0, 1.0, Px, "Corner radius", "How round menus' corners are.";
        border_width: 1.0, 0.0, 4.0, 0.5, Px, "Border width", "The solid interface's border.";
        text_size: 13.0, 8.0, 24.0, 0.5, Px, "Text size", "Menu entries' names.";
        icon_size: 15.0, 8.0, 28.0, 0.5, Px, "Icon size", "The icons beside menu entries.";
        header_size: 11.0, 7.0, 20.0, 0.5, Px, "Heading text size", "Headings inside menus, like Color.";
        header_height: 24.0, 12.0, 48.0, 1.0, Px, "Heading height", "The height of each heading.";
        separator_height: 9.0, 1.0, 30.0, 1.0, Px, "Separator height", "The room a separator line takes, line included.";
        grid_cell: 30.0, 20.0, 60.0, 1.0, Px, "Picker cell size", "The icon and color pickers.";
        picker_icon_size: 16.0, 8.0, 32.0, 0.5, Px, "Picker icon size", "The icons and color swatches in the icon and color pickers.";
        margin: 8.0, 0.0, 40.0, 1.0, Px, "Distance from the window's edges", "Menus are kept at least this far inside the window.";
        submenu_gap: 4.0, 0.0, 30.0, 1.0, Px, "Space before a submenu", "Between a menu and the submenu beside it.";
        shadow_offset: 6.0, 0.0, 30.0, 1.0, Px, "Shadow offset", "How far below a menu its shadow falls.";
        shadow_blur: 18.0, 0.0, 60.0, 1.0, Px, "Shadow blur", "How soft a menu's shadow is.";
    }
}

numbers! {
    /// `[style.controls]`: buttons, text fields, drop outlines and drop lines.
    ControlStyle, "controls", "Buttons, fields and outlines" {
        radius: 8.0, 0.0, 24.0, 1.0, Px, "Corner radius", "Buttons, text fields and hover highlights.";
        border_width: 1.0, 0.0, 4.0, 0.5, Px, "Border width", "Text fields, the + buttons and outlines.";
        highlight_width: 1.5, 0.0, 6.0, 0.5, Px, "Drop target outline", "Around where a dragged tab or workspace would go.";
        insertion_line_width: 2.0, 0.5, 8.0, 0.5, Px, "Drop line thickness", "The line between items where a dragged one would land.";
        icon_button_size: 14.0, 8.0, 28.0, 0.5, Px, "Small button icon size", "Close buttons, + buttons and the find bar's arrows.";
        field_text_size: 12.0, 8.0, 24.0, 0.5, Px, "Field text size", "The find bar and rename fields.";
    }
}

numbers! {
    /// `[style.effects]`: opacities, hover and drag strengths, shadow scale and the glass
    /// interface's blur, shine, lift and refraction.
    EffectStyle, "effects", "Effects" {
        dim: 0.55, 0.0, 1.0, 0.05, Percent, "Unselected opacity", "How strongly unselected tabs and workspaces show their text and icons.";
        hover: 0.5, 0.0, 1.0, 0.05, Percent, "Hover highlight", "How far a hovered item goes toward the selected look.";
        dragged: 0.35, 0.0, 1.0, 0.05, Percent, "Dragged item opacity", "The item left in place while it's dragged.";
        drag_preview: 0.92, 0.0, 1.0, 0.01, Percent, "Drag preview opacity", "The tab or workspace that follows the cursor while dragging.";
        drop_tint: 0.12, 0.0, 1.0, 0.01, Percent, "Drop target tint", "How strongly the area or group where a dragged tab or workspace would land is filled.";
        shadow: 1.0, 0.0, 4.0, 0.1, Times, "Shadow size", "Scales the shadows under items and panes.";
        glass_idle: 0.45, 0.0, 1.0, 0.05, Percent, "Unselected glass strength", "Glass interface: how frosted unselected items are.";
        blur: 40.0, 0.0, 120.0, 1.0, Px, "Glass blur", "Glass interface: how much the backdrop blurs behind sheets.";
        shine: 0.6, 0.0, 1.0, 0.05, Percent, "Glass edge shine", "Glass interface: the highlight along sheets' edges.";
        lift: 1.0, 0.0, 6.0, 0.5, Px, "Selected lift", "Glass interface: how much a selected item grows and rises.";
        lift_clear: 0.45, 0.0, 1.0, 0.05, Percent, "Selected clarity", "Glass interface: how much less a selected item blurs.";
        refraction: 1.0, 0.0, 2.0, 0.05, Times, "Selected refraction", "Glass interface: how much a selected item bends the backdrop at its edges.";
    }
}

// One row per shared theme color: name, label, description.
theme_colors! {
    accent, "Accent", "Attention, focus, switches that are on and drop lines.";
    success, "Success", "A coding agent at work.";
    error, "Error", "Error banners and failures.";
    foreground, "Foreground", "Primary text, the logo and switch knobs.";
    lines, "Lines", "Separators and pane borders.";
    shade, "Shade", "Menu shadows and the backdrop behind dialogs.";
}

// One row per color: name, label, group, description, the theme color it follows unless it's
// set ("" for none), whether only glass draws it.
colors! {
    background, "Window background", "Window", "Behind the sidebar and panes. The glass interface draws its backdrop instead.", "", false;
    title_bar, "Title bar", "Window", "Across the top of the window, behind the window buttons.", "", false;
    title_bar_border, "Title bar edge", "Solid theme", "The line along the title bar's bottom. With no space below the title bar it's the tabs' top border too.", "", false;
    sidebar, "Sidebar", "Window", "Behind the sidebar, up to the title bar. Clear in glass.", "", false;
    close_hover, "Close button hover", "Window", "The close window button under the cursor.", "", false;
    logo, "Logo", "Window", "The macaroni logo in the title bar.", "foreground", false;
    separator, "Separators", "Window", "Thin lines between parts, and field borders in the settings.", "lines", false;
    outline, "Outlines", "Window", "The focused pane, selected picker cells and, in glass, the + buttons.", "", false;
    text, "Text", "Text", "Names, titles and other primary text.", "foreground", false;
    label, "Labels", "Text", "Icons and secondary text.", "", false;
    muted, "Muted text", "Text", "Quieter text, like descriptions.", "", false;
    faint, "Faint text", "Text", "Details, badges and disabled items.", "", false;
    attention, "Attention", "Accents", "Notifications, focus and switches that are on.", "accent", false;
    working, "Agent working", "Accents", "Shown while a coding agent is working.", "success", false;
    danger, "Errors", "Accents", "Error banners and failures.", "error", false;
    insertion, "Drop line", "Accents", "Where a dragged tab or workspace would land.", "accent", false;
    surface, "Cards", "Workspaces and tabs", "Banners, and in glass the tint of open workspaces and tabs.", "", false;
    item, "Highlighted items", "Workspaces and tabs", "Hovered group headers and the settings' cards.", "", false;
    raised, "Selected choices", "Workspaces and tabs", "Selected choices and primary buttons.", "", false;
    hover, "Hovered items", "Workspaces and tabs", "Buttons and menu items under the cursor.", "", false;
    item_edge, "Menu borders", "Workspaces and tabs", "Menu borders and dividers, and the edges of dialog fields.", "", false;
    sunken, "Fields", "Workspaces and tabs", "Text fields, switches that are off and the title bar.", "", false;
    workspace_icon, "Workspace icon", "Workspaces and tabs", "The tile behind each workspace's icon in the sidebar.", "", false;
    shadow, "Shadows", "Workspaces and tabs", "Under workspaces, tabs and panes. Solid draws it under panes only, clear by default.", "", false;
    border, "Borders", "Solid theme", "Every line between parts: the panes', tabs', + button's, sidebar edge and title bar edge colors take it unless set themselves. Setting it clears theirs.", "", false;
    search, "Search bar", "Solid theme", "Behind the sidebar's search field.", "", false;
    sidebar_border, "Sidebar edge", "Solid theme", "The line down the sidebar's right side, style.sidebar.border_width wide.", "", false;
    workspace, "Workspace", "Solid theme", "Behind workspaces that aren't open.", "", false;
    workspace_active, "Workspace, open", "Solid theme", "Behind the workspace that's open.", "", false;
    workspace_hover, "Workspace, hovered", "Solid theme", "Behind a workspace under the cursor.", "", false;
    workspace_border, "Workspace border", "Solid theme", "Around workspaces that aren't open.", "", false;
    workspace_border_active, "Workspace border, open", "Solid theme", "Around the workspace that's open.", "", false;
    workspace_border_hover, "Workspace border, hovered", "Solid theme", "Around a workspace under the cursor.", "", false;
    tab_strip, "Tab slot", "Solid theme", "Behind each pane's tab strip, under and between the tabs.", "", false;
    tab, "Tab", "Solid theme", "Behind tabs that aren't open.", "", false;
    tab_active, "Tab, open", "Solid theme", "Behind each pane's open tab.", "", false;
    tab_hover, "Tab, hovered", "Solid theme", "Behind a tab under the cursor.", "", false;
    tab_border, "Tab border", "Solid theme", "Around tabs that aren't open.", "", false;
    tab_border_active, "Tab border, open", "Solid theme", "Around each pane's open tab.", "", false;
    tab_border_hover, "Tab border, hovered", "Solid theme", "Around a tab under the cursor.", "", false;
    tab_accent, "Open tab line", "Solid theme", "Along the top of each pane's open tab, style.tabs.accent_width thick.", "accent", false;
    add_workspace, "+ button background", "Solid theme", "Behind the New workspace button. Hovered, it takes the hovered workspace's colors.", "", false;
    add_workspace_border, "+ button border", "Solid theme", "Around the New workspace button.", "", false;
    add_tab, "+ button background", "Solid theme", "Behind the New tab button. Hovered, it takes the hovered tab's colors.", "", false;
    add_tab_border, "+ button border", "Solid theme", "Around the New tab button.", "", false;
    card, "Pane background", "Panes", "The solid interface uses the terminal theme's background unless this is set, and then the terminal takes this color too.", "", false;
    card_border, "Pane border", "Panes", "The thin border around each pane.", "lines", false;
    popup, "Menus", "Menus and dialogs", "Behind right-click menus, drop-downs and the settings.", "", false;
    menu_shadow, "Menu shadow", "Menus and dialogs", "Under menus and dialogs.", "shade", false;
    overlay, "Dialog backdrop", "Menus and dialogs", "Over the app behind the settings.", "shade", false;
    knob, "Switch knob", "Menus and dialogs", "The round knob on switches.", "foreground", false;
    glass_light, "Sheet tint", "Glass", "The light end of each sheet's gradient.", "", true;
    glass_dark, "Sheet tint, far end", "Glass", "The other end of each sheet's gradient.", "", true;
    glass_rim, "Sheet rim", "Glass", "The faint edge all round each sheet.", "", true;
    backdrop_top, "Backdrop, top left", "Backdrop", "The gradient behind everything.", "", true;
    backdrop_bottom, "Backdrop, bottom right", "Backdrop", "The other end of the gradient.", "", true;
    glow_1, "Glow, top left", "Backdrop", "Alpha is its strength.", "", true;
    glow_2, "Glow, top right", "Backdrop", "Alpha is its strength.", "", true;
    glow_3, "Glow, bottom", "Backdrop", "Alpha is its strength.", "", true;
    glow_4, "Glow, middle left", "Backdrop", "Alpha is its strength.", "", true;
    glow_5, "Glow, middle", "Backdrop", "Alpha is its strength.", "", true;
}

styles! {
    window: WindowStyle,
    sidebar: SidebarStyle,
    workspaces: WorkspaceStyle,
    groups: GroupStyle,
    tabs: TabStyle,
    panes: PaneStyle,
    menus: MenuStyle,
    controls: ControlStyle,
    effects: EffectStyle,
}

//! Colors: the terminal's [`Palette`] (from the configured terminal theme) and the interface's
//! [`Chrome`] (from `ui_theme`, `[colors.*]` and `[style.*]`).
//!
//! The app builds both on start and on every config reload and passes `Chrome` by value to every
//! view function. [`crate::terminal`] resolves cell colors through [`Palette::resolve`]; the
//! glass look's sheets ([`crate::glass`]) and backdrop shader ([`crate::backdrop`]) read
//! [`Chrome::glass`] and [`Chrome::backdrop`]. Also here: the workspace color presets and the
//! custom `#rrggbb` field's parser.

use std::collections::BTreeMap;

use macaroni_config::{COLORS, ColorValue, Config, Rgba, Style, UiTheme};

use crate::glass::Glass;
use macaroni_term::{Color as TermColor, DefaultColors, Rgb};

/// The terminal's colors, from the configured terminal theme. Programs can override any of them
/// at runtime (OSC 4/10/11); those overrides travel with each snapshot and are applied in
/// [`Palette::resolve`].
pub struct Palette {
    /// Default text color (`TermColor::Foreground`).
    pub foreground: Rgb,
    /// Default background color (`TermColor::Background`); solid pane cards use it too.
    pub background: Rgb,
    /// The cursor block.
    pub cursor: Rgb,
    /// Background of selected cells: the theme's `selection`, else a blend of background and blue.
    pub selection: Rgb,
    /// The 16 ANSI colors (normal, then bright), indexes 0 to 15 of the 256-color palette.
    pub ansi: [Rgb; 16],
}

/// Converts a config color to the terminal crate's.
fn rgb(c: macaroni_config::Rgb) -> Rgb {
    Rgb::new(c.0, c.1, c.2)
}

impl Palette {
    /// In solid, the terminal's default background made the pane background
    /// (`Chrome.card`), so setting the pane color recolors the terminal too and the two never
    /// meet in a seam. A translucent pane color keeps the theme's background, since cells are
    /// painted opaque. Glass leaves it be: its terminals don't paint the default background.
    pub fn with_pane_background(mut self, chrome: &Chrome) -> Self {
        if chrome.glass.is_none() && chrome.card.a >= 1.0 {
            let [r, g, b, _] = chrome.card.into_rgba8();
            self.background = Rgb { r, g, b };
        }
        self
    }

    /// Builds the palette from a terminal theme. Without a `selection` color, selected cells get
    /// the background blended 35% of the way to the theme's blue (ANSI 4).
    pub fn from_theme(theme: &macaroni_config::Theme) -> Self {
        let background = rgb(theme.background);
        let blue = rgb(theme.ansi[4]);
        let blend = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * 0.35) as u8;
        Self {
            foreground: rgb(theme.foreground),
            background,
            cursor: rgb(theme.cursor()),
            selection: theme.selection.map(rgb).unwrap_or(Rgb::new(
                blend(background.r, blue.r),
                blend(background.g, blue.g),
                blend(background.b, blue.b),
            )),
            ansi: theme.ansi.map(rgb),
        }
    }

    /// The color a cell's `color` stands for. `overrides` are the program's palette changes, keyed
    /// by index (0 to 255, then 256 for the foreground and 257 for the background, as the emulator
    /// reports them); they win over the theme. Direct RGB colors pass through.
    pub fn resolve(&self, color: TermColor, overrides: &[(u16, Rgb)]) -> Rgb {
        let index = match color {
            TermColor::Rgb(rgb) => return rgb,
            TermColor::Indexed(i) => u16::from(i),
            TermColor::Foreground => 256,
            TermColor::Background => 257,
        };
        if let Some(&(_, rgb)) = overrides.iter().find(|(i, _)| *i == index) {
            return rgb;
        }
        match color {
            TermColor::Indexed(i) => self.indexed(i),
            TermColor::Background => self.background,
            _ => self.foreground,
        }
    }

    /// What programs are told when they ask a terminal for a color they haven't set.
    pub fn default_colors(&self) -> DefaultColors {
        DefaultColors {
            palette: std::array::from_fn(|i| self.indexed(i as u8)),
            foreground: self.foreground,
            background: self.background,
            cursor: self.cursor,
        }
    }

    /// The xterm 256-color palette: 16 theme colors, a 6x6x6 cube, then a 24-step gray ramp.
    fn indexed(&self, i: u8) -> Rgb {
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        match i {
            0..=15 => self.ansi[usize::from(i)],
            16..=231 => {
                let i = i - 16;
                Rgb::new(
                    LEVELS[usize::from(i / 36)],
                    LEVELS[usize::from(i / 6 % 6)],
                    LEVELS[usize::from(i % 6)],
                )
            }
            _ => {
                let v = 8 + (i - 232) * 10;
                Rgb::new(v, v, v)
            }
        }
    }
}

/// A terminal color as an iced color with `alpha` (0 to 1).
pub fn to_iced(rgb: Rgb, alpha: f32) -> iced::Color {
    iced::Color::from_rgba8(rgb.r, rgb.g, rgb.b, alpha)
}

/// App UI colors, by `ui_theme`, with the user's `[colors.*]` overrides, and the `[style.*]`
/// sizes. Solid is a fixed dark palette of layered greys and one accent. Glass draws the window
/// over the [`crate::backdrop`] shader and its surfaces as translucent sheets
/// ([`crate::glass`]), with terminals on them instead of on their own background. In solid, pane
/// cards follow the terminal theme, so a terminal's background meets its card without a seam.
///
/// Built by [`Chrome::new`]; `Copy`, so view functions take it by value. Each color field is
/// also a `[colors.*]` setting of the same name (see [`Chrome::color_mut`]).
#[derive(Debug, Clone, Copy)]
pub struct Chrome {
    /// The window, sidebar and tab rows.
    pub background: iced::Color,
    /// The custom title bar (Windows); transparent in glass, where it's a frosted sheet.
    pub title_bar: iced::Color,
    /// Behind the sidebar; transparent by default, so the window background shows.
    pub sidebar: iced::Color,
    /// The window's close button, hovered.
    pub close_hover: iced::Color,
    /// The logo in the title bar.
    pub logo: iced::Color,
    /// Inactive sidebar items (and, in glass, tabs).
    pub surface: iced::Color,
    /// Solid: behind tabs that aren't open.
    pub tab: iced::Color,
    /// The active tab, and selected controls.
    pub raised: iced::Color,
    /// Hovered items, buttons and menu entries.
    pub hover: iced::Color,
    /// Text fields, icon tiles and the solid title bar.
    pub sunken: iced::Color,
    /// The tile behind each workspace's icon in the sidebar.
    pub workspace_icon: iced::Color,
    /// Pane cards: the terminal's background, with a fine border.
    pub card: iced::Color,
    /// The pane card's 1px border.
    pub card_border: iced::Color,
    /// The active sidebar item.
    pub item: iced::Color,
    /// Behind the sidebar's search field in solid (glass derives it from the pane cards).
    pub search: iced::Color,
    /// Solid: every line between parts, unless each sets its own ([`BORDER_FOLLOWERS`]).
    pub border: iced::Color,
    /// Solid: behind each pane's tab strip.
    pub tab_strip: iced::Color,
    /// Solid: the line down the sidebar's right side ([`crate::view::sidebar_edge`]).
    pub sidebar_border: iced::Color,
    /// Solid: the line along the title bar's bottom ([`crate::view::with_title_line`]).
    pub title_bar_border: iced::Color,
    /// Solid: behind workspaces that aren't open.
    pub workspace: iced::Color,
    /// Solid: behind the open workspace.
    pub workspace_active: iced::Color,
    /// Solid: behind a hovered workspace.
    pub workspace_hover: iced::Color,
    /// Solid: around workspaces that aren't open.
    pub workspace_border: iced::Color,
    /// Solid: around the open workspace.
    pub workspace_border_active: iced::Color,
    /// Solid: around a hovered workspace.
    pub workspace_border_hover: iced::Color,
    /// Solid: behind each pane's open tab.
    pub tab_active: iced::Color,
    /// Solid: behind a hovered tab.
    pub tab_hover: iced::Color,
    /// Solid: around tabs that aren't open.
    pub tab_border: iced::Color,
    /// Solid: around each pane's open tab.
    pub tab_border_active: iced::Color,
    /// Solid: around a hovered tab.
    pub tab_border_hover: iced::Color,
    /// Solid: the line along the top of each pane's open tab.
    pub tab_accent: iced::Color,
    /// Solid: behind the New workspace `+` button.
    pub add_workspace: iced::Color,
    /// Solid: around the New workspace `+` button.
    pub add_workspace_border: iced::Color,
    /// Solid: behind the New tab `+` button.
    pub add_tab: iced::Color,
    /// Solid: around the New tab `+` button.
    pub add_tab_border: iced::Color,
    /// Menu borders and dividers, and dialog field edges.
    pub item_edge: iced::Color,
    /// Separator lines and the settings dialog's field borders.
    pub separator: iced::Color,
    /// The focused pane's outline when there's more than one.
    pub outline: iced::Color,
    /// Primary text.
    pub text: iced::Color,
    /// Icons and secondary text.
    pub label: iced::Color,
    /// Quieter text than `label`.
    pub muted: iced::Color,
    /// Details, badges and disabled items.
    pub faint: iced::Color,
    /// Tabs, panes and workspaces waiting for the user.
    pub attention: iced::Color,
    /// An agent at work.
    pub working: iced::Color,
    /// Errors.
    pub danger: iced::Color,
    /// Where a dragged tab or workspace would land.
    pub insertion: iced::Color,
    /// Right-click menus: opaque enough to read over anything.
    pub popup: iced::Color,
    /// The shadow under menus and dialogs.
    pub menu_shadow: iced::Color,
    /// Over the app behind a dialog.
    pub overlay: iced::Color,
    /// The knob of the settings dialog's switches.
    pub knob: iced::Color,
    /// Under sidebar items, tabs and pane cards.
    pub shadow: iced::Shadow,
    /// Set for the glass look: sidebar items, tabs, pane cards and popups are glass sheets
    /// ([`crate::glass::sheet`]), and the window draws `backdrop` instead of `background`.
    pub glass: Option<Glass>,
    /// The glass backdrop: its gradient's two ends, then its glows (alpha is each one's
    /// strength), as `backdrop.wgsl` places them. Passed to the shader as its shared
    /// [`crate::backdrop::Scene`] uniform. Solid carries glass's, unused.
    pub backdrop: [iced::Color; 7],
    /// Sizes, spacing, radii and text sizes (`[style.*]`).
    pub s: Style,
}
/// The colors a workspace can have, by the names saved with it.
pub const WORKSPACE_COLORS: [(&str, u32); 9] = [
    ("red", 0xe5484d),
    ("orange", 0xf76b15),
    ("amber", 0xffb224),
    ("green", 0x68c6a9),
    ("teal", 0x12a594),
    ("blue", 0x009ced),
    ("purple", 0x8e4ec6),
    ("pink", 0xe93d82),
    ("gray", 0x8b8b8b),
];

/// The color picker's palette: one column per hue (gray, brown, cyan, blue, violet, grape,
/// pink, green, yellow, orange, red), each from lightest to darkest. Picked ones are saved as
/// their `#rrggbb`, which [`workspace_color`] reads like any custom color.
pub const PALETTE: [[&str; 5]; 11] = [
    // Gray
    ["#ffffff", "#e9ecef", "#ced4da", "#868e96", "#343a40"],
    // Brown
    ["#f8f1ee", "#eaddd7", "#d2bab0", "#a18072", "#846358"],
    // Cyan
    ["#e3fafc", "#99e9f2", "#3bc9db", "#15aabf", "#0c8599"],
    // Blue
    ["#e7f5ff", "#a5d8ff", "#4dabf7", "#228be6", "#1971c2"],
    // Violet
    ["#f3f0ff", "#d0bfff", "#9775fa", "#7950f2", "#6741d9"],
    // Grape
    ["#f8f0fc", "#eebefa", "#da77f2", "#be4bdb", "#9c36b5"],
    // Pink
    ["#fff0f6", "#fcc2d7", "#f783ac", "#e64980", "#c2255c"],
    // Green
    ["#ebfbee", "#b2f2bb", "#69db7c", "#40c057", "#2f9e44"],
    // Yellow
    ["#fff9db", "#ffec99", "#ffd43b", "#fab005", "#f08c00"],
    // Orange
    ["#fff4e6", "#ffd8a8", "#ffa94d", "#fd7e14", "#e8590c"],
    // Red
    ["#fff5f5", "#ffc9c9", "#ff8787", "#fa5252", "#e03131"],
];

/// An opaque color from `0xrrggbb`.
pub fn hex(v: u32) -> iced::Color {
    iced::Color::from_rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// A workspace's chosen color: one of [`WORKSPACE_COLORS`] by name, or a custom `#rrggbb`.
pub fn workspace_color(name: Option<&str>) -> Option<iced::Color> {
    let name = name?;
    if let Some(rgb) = macaroni_config::Rgb::parse(name) {
        return Some(iced::Color::from_rgb8(rgb.0, rgb.1, rgb.2));
    }
    WORKSPACE_COLORS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, v)| hex(*v))
}

/// What's typed in the custom color field as a `#rrggbb` to save, if it is one: the `#` is
/// optional and case doesn't matter.
pub fn custom_color(input: &str) -> Option<String> {
    let input = input.trim();
    let digits = input.strip_prefix('#').unwrap_or(input);
    let color = format!("#{}", digits.to_ascii_lowercase());
    macaroni_config::Rgb::parse(&color).map(|_| color)
}

/// A config color as an iced one.
pub fn from_rgba(rgba: Rgba) -> iced::Color {
    iced::Color::from_rgba8(rgba.0, rgba.1, rgba.2, f32::from(rgba.3) / 255.0)
}

/// The shared theme colors (`macaroni_config::THEME_COLORS`) for one interface: its own, with
/// the user's overrides. [`Chrome::new`] paints every color linked to one with it; the settings
/// dialog shows them.
#[derive(Debug, Clone)]
pub struct ThemeColors(BTreeMap<&'static str, iced::Color>);

impl ThemeColors {
    /// `ui`'s theme colors with `overrides` (its `[colors.*]` table) applied. Only plain colors
    /// apply: [`macaroni_config::Config::parse`] rejects a theme color set to another's name.
    pub fn new(ui: UiTheme, overrides: &BTreeMap<String, ColorValue>) -> Self {
        let mut colors = Self::base(ui).0;
        for (name, color) in &mut colors {
            if let Some(ColorValue::Rgba(rgba)) = overrides.get(*name) {
                *color = from_rgba(*rgba);
            }
        }
        Self(colors)
    }

    /// `ui`'s own theme colors. Each is the same as the interface's own color for every element
    /// linked to it, so following a theme color changes nothing until it's set (a test checks).
    pub fn base(ui: UiTheme) -> Self {
        let accent = match ui {
            UiTheme::Glass => hex(0x0aa6ff),
            UiTheme::Solid => hex(0x009ced),
        };
        let lines = match ui {
            UiTheme::Glass => white(0.08),
            UiTheme::Solid => hex(0x2a2b2c),
        };
        Self(BTreeMap::from([
            ("accent", accent),
            ("success", hex(0x68c6a9)),
            ("error", hex(0xe5484d)),
            ("foreground", hex(0xffffff)),
            ("lines", lines),
            ("shade", black(0.45)),
        ]))
    }

    /// The theme color called `name`; `None` for an empty or unknown name.
    pub fn get(&self, name: &str) -> Option<iced::Color> {
        self.0.get(name).copied()
    }
}

/// White at `alpha`: glass's tints are white over the dark backdrop.
fn white(alpha: f32) -> iced::Color {
    iced::Color::from_rgba(1.0, 1.0, 1.0, alpha)
}

/// Black at `alpha`, for shadows and dimming.
fn black(alpha: f32) -> iced::Color {
    iced::Color::from_rgba(0.0, 0.0, 0.0, alpha)
}

/// The colors of the lines between parts, which follow `[colors.solid] border` while it's set
/// (and which setting it in the settings clears, so they all follow it).
pub const BORDER_FOLLOWERS: &[&str] = &[
    "card_border",
    "tab_border",
    "tab_border_active",
    "tab_border_hover",
    "add_tab_border",
    "sidebar_border",
    "title_bar_border",
];

impl Chrome {
    /// The look `config` asks for: [`Chrome::base`] for its `ui_theme`, then each color from
    /// `[colors.*]` for that interface: its own value if it's set (a color, or a theme color it
    /// follows), else the theme color it's linked to ([`ThemeColors`], with their overrides),
    /// else the interface's own. Then the shadow scaled by `style.effects.shadow`, glass's shine
    /// from `style.effects.shine`, and the whole `[style.*]` in [`Chrome::s`].
    pub fn new(p: &Palette, config: &Config) -> Self {
        let ui = config.ui_theme;
        let mut chrome = Self::base(p, ui);
        let overrides = config.colors.for_ui(ui);
        let theme = ThemeColors::new(ui, overrides);
        for setting in COLORS {
            let value = match overrides.get(setting.name) {
                Some(ColorValue::Rgba(rgba)) => Some(from_rgba(*rgba)),
                Some(ColorValue::Theme(name)) => theme.get(name),
                None => theme.get(setting.link),
            };
            if let (Some(value), Some(color)) = (value, chrome.color_mut(setting.name)) {
                *color = value;
            }
        }
        // The lines between parts follow the one border color while it's set, each unless it
        // has its own.
        // Unset, it shows the pane border's color, the one most of the lines have.
        if !overrides.contains_key("border") {
            chrome.border = chrome.card_border;
        } else {
            let border = chrome.border;
            for name in BORDER_FOLLOWERS {
                if !overrides.contains_key(*name)
                    && let Some(color) = chrome.color_mut(name)
                {
                    *color = border;
                }
            }
        }
        let style = config.style;
        let scale = style.effects.shadow;
        chrome.shadow.blur_radius *= scale;
        chrome.shadow.offset *= scale;
        if let Some(glass) = &mut chrome.glass {
            glass.shine = style.effects.shine;
        }
        chrome.s = style;
        chrome
    }

    /// `ui`'s own colors, before the user's overrides, with the default sizes.
    pub fn base(p: &Palette, ui: UiTheme) -> Self {
        match ui {
            UiTheme::Solid => Self::solid(p),
            UiTheme::Glass => Self::glass(),
        }
    }

    /// The color named `name` in `[colors.*]`, or `None` if there's no such color in this look
    /// (the glass-only names under solid). The settings dialog uses it on [`Chrome::base`] for
    /// placeholders and swatches.
    pub fn color(&self, name: &str) -> Option<iced::Color> {
        let mut chrome = *self;
        chrome.color_mut(name).map(|c| *c)
    }

    /// The field behind the `[colors.*]` setting `name`: one arm per name in
    /// `macaroni_config::COLORS` (a test checks every name has one). `None` for unknown names, and
    /// for the glass-only ones (`glass_*`) when [`Chrome::glass`] is unset.
    fn color_mut(&mut self, name: &str) -> Option<&mut iced::Color> {
        Some(match name {
            "background" => &mut self.background,
            "title_bar" => &mut self.title_bar,
            "sidebar" => &mut self.sidebar,
            "close_hover" => &mut self.close_hover,
            "logo" => &mut self.logo,
            "separator" => &mut self.separator,
            "outline" => &mut self.outline,
            "text" => &mut self.text,
            "label" => &mut self.label,
            "muted" => &mut self.muted,
            "faint" => &mut self.faint,
            "attention" => &mut self.attention,
            "working" => &mut self.working,
            "danger" => &mut self.danger,
            "insertion" => &mut self.insertion,
            "surface" => &mut self.surface,
            "tab" => &mut self.tab,
            "item" => &mut self.item,
            "raised" => &mut self.raised,
            "hover" => &mut self.hover,
            "search" => &mut self.search,
            "border" => &mut self.border,
            "tab_strip" => &mut self.tab_strip,
            "sidebar_border" => &mut self.sidebar_border,
            "title_bar_border" => &mut self.title_bar_border,
            "workspace" => &mut self.workspace,
            "workspace_active" => &mut self.workspace_active,
            "workspace_hover" => &mut self.workspace_hover,
            "workspace_border" => &mut self.workspace_border,
            "workspace_border_active" => &mut self.workspace_border_active,
            "workspace_border_hover" => &mut self.workspace_border_hover,
            "tab_active" => &mut self.tab_active,
            "tab_hover" => &mut self.tab_hover,
            "tab_border" => &mut self.tab_border,
            "tab_border_active" => &mut self.tab_border_active,
            "tab_border_hover" => &mut self.tab_border_hover,
            "tab_accent" => &mut self.tab_accent,
            "add_workspace" => &mut self.add_workspace,
            "add_workspace_border" => &mut self.add_workspace_border,
            "add_tab" => &mut self.add_tab,
            "add_tab_border" => &mut self.add_tab_border,
            "item_edge" => &mut self.item_edge,
            "sunken" => &mut self.sunken,
            "workspace_icon" => &mut self.workspace_icon,
            "shadow" => &mut self.shadow.color,
            "card" => &mut self.card,
            "card_border" => &mut self.card_border,
            "popup" => &mut self.popup,
            "menu_shadow" => &mut self.menu_shadow,
            "overlay" => &mut self.overlay,
            "knob" => &mut self.knob,
            "glass_light" => &mut self.glass.as_mut()?.light,
            "glass_dark" => &mut self.glass.as_mut()?.dark,
            "glass_rim" => &mut self.glass.as_mut()?.rim,
            "backdrop_top" => &mut self.backdrop[0],
            "backdrop_bottom" => &mut self.backdrop[1],
            "glow_1" => &mut self.backdrop[2],
            "glow_2" => &mut self.backdrop[3],
            "glow_3" => &mut self.backdrop[4],
            "glow_4" => &mut self.backdrop[5],
            "glow_5" => &mut self.backdrop[6],
            _ => return None,
        })
    }

    /// The glass look's own colors: white and black tints over the backdrop, a stronger shadow.
    fn glass() -> Self {
        Self {
            background: hex(0x1b1b1b),
            title_bar: iced::Color::TRANSPARENT,
            sidebar: iced::Color::TRANSPARENT,
            close_hover: hex(0xc42b1c),
            logo: hex(0xffffff),
            surface: white(0.035),
            tab: white(0.035),
            raised: white(0.09),
            hover: white(0.06),
            sunken: black(0.22),
            workspace_icon: black(0.22),
            card: iced::Color::TRANSPARENT,
            card_border: white(0.08),
            item: white(0.075),
            search: iced::Color::TRANSPARENT,
            border: iced::Color::TRANSPARENT,
            tab_strip: iced::Color::TRANSPARENT,
            sidebar_border: iced::Color::TRANSPARENT,
            title_bar_border: iced::Color::TRANSPARENT,
            workspace: iced::Color::TRANSPARENT,
            workspace_active: iced::Color::TRANSPARENT,
            workspace_hover: iced::Color::TRANSPARENT,
            workspace_border: iced::Color::TRANSPARENT,
            workspace_border_active: iced::Color::TRANSPARENT,
            workspace_border_hover: iced::Color::TRANSPARENT,
            tab_active: iced::Color::TRANSPARENT,
            tab_hover: iced::Color::TRANSPARENT,
            tab_border: iced::Color::TRANSPARENT,
            tab_border_active: iced::Color::TRANSPARENT,
            tab_border_hover: iced::Color::TRANSPARENT,
            tab_accent: hex(0x0aa6ff),
            add_workspace: iced::Color::TRANSPARENT,
            add_workspace_border: iced::Color::TRANSPARENT,
            add_tab: iced::Color::TRANSPARENT,
            add_tab_border: iced::Color::TRANSPARENT,
            item_edge: white(0.24),
            separator: white(0.08),
            outline: white(0.2),
            text: hex(0xffffff),
            label: white(0.68),
            muted: white(0.55),
            faint: white(0.45),
            attention: hex(0x0aa6ff),
            working: hex(0x68c6a9),
            danger: hex(0xe5484d),
            insertion: hex(0x0aa6ff),
            popup: iced::Color::from_rgba8(28, 28, 40, 0.97),
            menu_shadow: black(0.45),
            overlay: black(0.45),
            knob: hex(0xffffff),
            shadow: iced::Shadow {
                color: black(0.18),
                offset: iced::Vector::new(0.0, 1.0),
                blur_radius: 5.0,
            },
            glass: Some(Glass {
                light: white(0.07),
                dark: white(0.0),
                rim: white(0.12),
                shine: 0.6,
            }),
            backdrop: [
                iced::Color::from_rgb(0.125, 0.125, 0.145),
                iced::Color::from_rgb(0.075, 0.075, 0.086),
                iced::Color::from_rgba(0.20, 0.38, 1.00, 0.15),
                iced::Color::from_rgba(0.58, 0.28, 0.95, 0.12),
                iced::Color::from_rgba(0.10, 0.65, 0.60, 0.10),
                iced::Color::from_rgba(0.92, 0.32, 0.52, 0.09),
                iced::Color::from_rgba(0.32, 0.62, 1.00, 0.08),
            ],
            s: Style::default(),
        }
    }

    /// The solid look's own colors: flat near-black greys, close together. The title bar is the
    /// darkest; the window, pane cards (the terminal theme's background, `#121314` in the
    /// default theme) and the open tab are one color, so the open tab opens into its pane, and
    /// the sidebar and the tab slot are a step lighter. Every line between parts is one
    /// `#2a2b2c`. Workspaces have the sidebar's fill, lighter when open; panes have no shadow.
    /// Text runs from white to grey.
    fn solid(p: &Palette) -> Self {
        Self {
            background: hex(0x121314),
            title_bar: hex(0x0e0e0f),
            sidebar: hex(0x191a1b),
            close_hover: hex(0xc42b1c),
            logo: hex(0xffffff),
            surface: iced::Color::TRANSPARENT,
            tab: hex(0x202122),
            raised: hex(0x303030),
            hover: hex(0x2c2d31),
            sunken: hex(0x151515),
            workspace_icon: hex(0x151515),
            card: to_iced(p.background, 1.0),
            card_border: hex(0x2a2b2c),
            item: hex(0x333438),
            search: hex(0x121314),
            border: hex(0x2a2b2c),
            tab_strip: hex(0x202122),
            sidebar_border: hex(0x2a2b2c),
            title_bar_border: hex(0x2a2b2c),
            workspace: hex(0x191a1b),
            workspace_active: hex(0x2c2d2e),
            workspace_hover: hex(0x131314),
            workspace_border: iced::Color::TRANSPARENT,
            workspace_border_active: iced::Color::TRANSPARENT,
            workspace_border_hover: iced::Color::TRANSPARENT,
            tab_active: hex(0x121314),
            tab_hover: iced::Color::TRANSPARENT,
            tab_border: hex(0x2a2b2c),
            tab_border_active: hex(0x2a2b2c),
            tab_border_hover: hex(0x2a2b2c),
            tab_accent: hex(0x009ced),
            add_workspace: iced::Color::TRANSPARENT,
            add_workspace_border: hex(0x1f1f1f),
            add_tab: iced::Color::TRANSPARENT,
            add_tab_border: hex(0x2a2b2c),
            item_edge: hex(0x4a4a4a),
            separator: hex(0x2a2b2c),
            outline: hex(0x555555),
            text: hex(0xffffff),
            label: hex(0xa3a3a3),
            muted: hex(0x737373),
            faint: hex(0x5c5c5c),
            attention: hex(0x009ced),
            working: hex(0x68c6a9),
            danger: hex(0xe5484d),
            insertion: hex(0x009ced),
            popup: hex(0x222222),
            menu_shadow: black(0.45),
            overlay: black(0.45),
            knob: hex(0xffffff),
            // Clear: a flat look, and a pane's shadow would spill past the tabs above it.
            shadow: iced::Shadow {
                color: iced::Color::TRANSPARENT,
                offset: iced::Vector::new(0.0, 1.0),
                blur_radius: 2.0,
            },
            glass: None,
            backdrop: Self::glass().backdrop,
            s: Style::default(),
        }
    }

    /// The iced theme for built-in widgets (scrollbars, text selection in inputs).
    pub fn iced_theme(&self, name: &str) -> iced::Theme {
        iced::Theme::custom(
            name.to_owned(),
            iced::theme::Palette {
                background: self.background,
                text: self.text,
                primary: self.attention,
                success: self.working,
                warning: self.attention,
                danger: self.danger,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use macaroni_config::THEME_COLORS;

    fn palette() -> Palette {
        Palette::from_theme(
            &macaroni_config::Theme::builtin(macaroni_config::DEFAULT_THEME).unwrap(),
        )
    }

    #[test]
    fn cube_and_gray_ramp() {
        let p = palette();
        assert_eq!(p.indexed(16), Rgb::new(0, 0, 0));
        assert_eq!(p.indexed(196), Rgb::new(255, 0, 0));
        assert_eq!(p.indexed(231), Rgb::new(255, 255, 255));
        assert_eq!(p.indexed(232), Rgb::new(8, 8, 8));
        assert_eq!(p.indexed(255), Rgb::new(238, 238, 238));
    }

    #[test]
    fn overrides_win() {
        let p = palette();
        let red = Rgb::new(1, 0, 0);
        assert_eq!(p.resolve(TermColor::Background, &[(257, red)]), red);
        assert_eq!(p.resolve(TermColor::Indexed(1), &[(1, red)]), red);
        assert_eq!(p.resolve(TermColor::Indexed(1), &[]), p.ansi[1]);
    }

    #[test]
    fn custom_colors() {
        assert_eq!(custom_color(" 12AB9f "), Some("#12ab9f".into()));
        assert_eq!(custom_color("#12ab9f"), Some("#12ab9f".into()));
        assert_eq!(custom_color("#12ab9"), None);
        assert_eq!(custom_color("blue"), None);
        assert!(workspace_color(Some("#ff0000")).is_some());
        assert!(workspace_color(Some("blue")).is_some());
        assert!(workspace_color(Some("nope")).is_none());
    }

    #[test]
    fn palette_colors_parse() {
        for hue in PALETTE {
            for value in hue {
                assert!(workspace_color(Some(value)).is_some(), "{value}");
            }
        }
    }

    #[test]
    fn every_color_setting_has_a_color() {
        let chrome = Chrome::base(&palette(), UiTheme::Glass);
        for color in macaroni_config::COLORS {
            assert!(chrome.color(color.name).is_some(), "{}", color.name);
        }
    }

    #[test]
    fn overrides_apply_to_their_interface() {
        let mut config = Config {
            ui_theme: UiTheme::Solid,
            ..Config::default()
        };
        config
            .colors
            .solid
            .insert("text".into(), ColorValue::Rgba(Rgba(255, 0, 0, 128)));
        config
            .colors
            .glass
            .insert("muted".into(), ColorValue::Rgba(Rgba(0, 255, 0, 255)));
        let chrome = Chrome::new(&palette(), &config);
        assert_eq!(
            chrome.text,
            iced::Color::from_rgba8(255, 0, 0, 128.0 / 255.0)
        );
        assert_eq!(chrome.muted, Chrome::base(&palette(), UiTheme::Solid).muted);
    }

    #[test]
    fn linked_colors_start_as_their_theme_color() {
        for ui in [UiTheme::Glass, UiTheme::Solid] {
            let base = Chrome::base(&palette(), ui);
            let theme = ThemeColors::base(ui);
            for setting in COLORS.iter().filter(|c| !c.link.is_empty()) {
                assert_eq!(
                    base.color(setting.name),
                    theme.get(setting.link),
                    "{} in {ui:?}",
                    setting.name
                );
            }
            for name in THEME_COLORS.iter().map(|t| t.name) {
                assert!(theme.get(name).is_some(), "{name}");
            }
        }
    }

    #[test]
    fn theme_colors_reach_what_follows_them() {
        let mut config = Config {
            ui_theme: UiTheme::Glass,
            ..Config::default()
        };
        let red = Rgba(255, 0, 0, 255);
        let glass = &mut config.colors.glass;
        glass.insert("accent".into(), ColorValue::Rgba(red));
        glass.insert("insertion".into(), ColorValue::Rgba(Rgba(0, 0, 255, 255)));
        glass.insert("label".into(), ColorValue::Theme("accent".into()));
        let chrome = Chrome::new(&palette(), &config);
        assert_eq!(chrome.attention, from_rgba(red));
        assert_eq!(chrome.label, from_rgba(red));
        assert_eq!(chrome.insertion, iced::Color::from_rgb8(0, 0, 255));
    }

    #[test]
    fn cards_follow_the_terminal_background() {
        let light =
            Palette::from_theme(&macaroni_config::Theme::builtin("macaroni-light").unwrap());
        assert_eq!(
            Chrome::base(&light, UiTheme::Solid).card,
            to_iced(light.background, 1.0)
        );
    }
}

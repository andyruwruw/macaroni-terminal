//! Every row the settings dialog shows, built from the current config.
//!
//! [`App::settings_list`] is rebuilt on every draw of the dialog, so each row's value is what's
//! in effect. The functional pages (General through Keyboard Shortcuts) are written out here.
//! The Appearance pages come from [`LOOK`], which places every color (`macaroni_config::COLORS`,
//! `THEME_COLORS`), every `[style.*]` number and the item toggles and templates on exactly one
//! card, each part's colors beside its sizes. A test checks nothing is left out or listed twice,
//! so a new color or number fails it until it's given a card.

use iced::Color;
use macaroni_config::{
    BrowserEngine, COLORS, Chord, ChordKey, ColorSetting, ColorValue, Config, CursorStyle,
    NamedKey, NumberSetting, OpenLinks, Rgba, Style, THEME_COLORS, Theme, ThemeColor, UiTheme,
    Unit, Value,
};

use super::{Control, Section, Setting, SettingsMessage};
use crate::theme::{Chrome, Palette, ThemeColors};
use crate::{App, MAX_FONT_SIZE, MIN_FONT_SIZE, Message, keymap};

/// `[font] size`, as a number row.
static FONT_SIZE: NumberSetting = NumberSetting {
    key: "font.size",
    group: "Font",
    label: "Font size",
    description: "Zoom (Ctrl+= and Ctrl+-) adds to it for the session.",
    default: 14.0,
    min: MIN_FONT_SIZE,
    max: MAX_FONT_SIZE,
    step: 1.0,
    unit: Unit::Points,
};

/// `[font] line_height`.
static LINE_HEIGHT: NumberSetting = NumberSetting {
    key: "font.line_height",
    group: "Font",
    label: "Line height",
    description: "Each row's height as a multiple of the font size.",
    default: 1.3,
    min: 1.0,
    max: 2.5,
    step: 0.05,
    unit: Unit::Times,
};

/// `[terminal] scroll_lines`.
static SCROLL_LINES: NumberSetting = NumberSetting {
    key: "terminal.scroll_lines",
    group: "Mouse",
    label: "Scroll speed",
    description: "Lines one notch of the mouse wheel scrolls.",
    default: 3.0,
    min: 1.0,
    max: 20.0,
    step: 1.0,
    unit: Unit::Lines,
};

/// The number rows that aren't `[style.*]`.
static OTHER_NUMBERS: [&NumberSetting; 3] = [&FONT_SIZE, &LINE_HEIGHT, &SCROLL_LINES];

/// The number setting with dotted `key`, from `[style.*]` or the others.
pub(super) fn number_setting(key: &str) -> Option<&'static NumberSetting> {
    Style::settings()
        .chain(OTHER_NUMBERS.iter().copied())
        .find(|s| s.key == key)
}

/// The `COLORS` group of the colors only the solid interface draws, left off the pages in glass.
const SOLID_GROUP: &str = "Solid theme";

/// The `[style.effects]` numbers only the glass interface uses, left off the pages in solid.
const GLASS_NUMBERS: &[&str] = &[
    "style.effects.glass_idle",
    "style.effects.blur",
    "style.effects.shine",
    "style.effects.lift",
    "style.effects.lift_clear",
    "style.effects.refraction",
];

/// A card on an Appearance page: its title, then its rows' keys (see [`LOOK`]).
type Card = (&'static str, &'static [&'static str]);

/// The Appearance pages: each page's cards in order, and each card's rows by key: a [`COLORS`]
/// or [`THEME_COLORS`] name, a `[style.*]` number's dotted key, or the key of one of
/// [`item_rows`]. A row the interface in use doesn't draw ([`SOLID_GROUP`], [`GLASS_NUMBERS`],
/// glass-only colors) is left out, and a card left with no rows isn't shown.
const LOOK: &[(Section, &[Card])] = &[
    (
        Section::Theme,
        &[
            ("Themes", &["ui_theme", "theme"]),
            (
                "Theme colors",
                &["accent", "success", "error", "foreground", "lines", "shade"],
            ),
            ("Text", &["text", "label", "muted", "faint"]),
            ("Accents", &["attention", "working", "danger"]),
            ("Surfaces", &["surface", "item"]),
        ],
    ),
    (
        Section::Window,
        &[
            (
                "Window",
                &[
                    "background",
                    "border",
                    "separator",
                    "style.window.resize_border",
                ],
            ),
            (
                "Title bar",
                &[
                    "title_bar",
                    "title_bar_border",
                    "style.window.title_bar_height",
                    "style.window.title_bar_border_width",
                    "style.window.title_bar_gap",
                ],
            ),
            (
                "Window buttons",
                &[
                    "close_hover",
                    "style.window.caption_button_width",
                    "style.window.caption_icon_size",
                ],
            ),
            (
                "Logo and names",
                &[
                    "logo",
                    "style.window.logo_size",
                    "style.window.logo_padding",
                    "style.window.app_name_size",
                    "style.window.workspace_name_size",
                    "style.window.workspace_icon_size",
                ],
            ),
        ],
    ),
    (
        Section::Sidebar,
        &[
            (
                "Sidebar",
                &[
                    "sidebar",
                    "sidebar_border",
                    "style.window.sidebar_width",
                    "style.sidebar.border_width",
                    "style.sidebar.top_padding",
                    "style.sidebar.padding",
                    "style.sidebar.bottom_padding",
                    "style.sidebar.spacing",
                    "style.sidebar.panes_gap",
                ],
            ),
            (
                "Search bar",
                &[
                    "workspace_items.search",
                    "search",
                    "style.sidebar.search_height",
                    "style.sidebar.search_padding",
                    "style.sidebar.search_gap",
                ],
            ),
            (
                "What workspaces show",
                &[
                    "workspace_items.title",
                    "workspace_items.detail",
                    "workspace_items.badge",
                    "workspace_items.status_icons",
                    "workspace_items.close_button",
                ],
            ),
            (
                "Workspaces",
                &[
                    "workspace",
                    "workspace_active",
                    "workspace_hover",
                    "workspace_border",
                    "workspace_border_active",
                    "workspace_border_hover",
                    "style.workspaces.padding",
                    "style.workspaces.radius",
                    "style.workspaces.spacing",
                    "style.workspaces.line_spacing",
                ],
            ),
            (
                "Workspace icon",
                &[
                    "workspace_items.icon",
                    "workspace_items.icon_background",
                    "workspace_icon",
                    "style.workspaces.tile_size",
                    "style.workspaces.tile_radius",
                    "style.workspaces.icon_size",
                ],
            ),
            (
                "Workspace text",
                &[
                    "style.workspaces.title_size",
                    "style.workspaces.detail_size",
                    "style.workspaces.badge_size",
                    "style.workspaces.mark_size",
                    "style.workspaces.max_detail_chars",
                ],
            ),
            (
                "Groups",
                &[
                    "style.groups.indent",
                    "style.groups.padding",
                    "style.groups.spacing",
                    "style.groups.name_size",
                    "style.groups.count_size",
                    "style.groups.chevron_size",
                ],
            ),
            (
                "New workspace button",
                &[
                    "add_workspace",
                    "add_workspace_border",
                    "style.workspaces.add_radius",
                    "style.sidebar.new_workspace_gap",
                ],
            ),
        ],
    ),
    (
        Section::Tabs,
        &[
            (
                "What tabs show",
                &[
                    "tab_items.icon",
                    "tab_items.title",
                    "tab_items.badge",
                    "tab_items.status_icons",
                    "tab_items.close_button",
                    "tab_items.new_buttons",
                ],
            ),
            (
                "Tabs",
                &[
                    "tab",
                    "tab_active",
                    "tab_hover",
                    "tab_border",
                    "tab_border_active",
                    "tab_border_hover",
                    "style.tabs.height",
                    "style.tabs.min_width",
                    "style.tabs.radius",
                    "style.tabs.padding_left",
                    "style.tabs.padding_right",
                    "style.tabs.spacing",
                    "style.tabs.part_spacing",
                ],
            ),
            ("Open tab line", &["tab_accent", "style.tabs.accent_width"]),
            (
                "Tab text and icons",
                &[
                    "style.tabs.icon_size",
                    "style.tabs.title_size",
                    "style.tabs.badge_size",
                    "style.tabs.max_title_chars",
                ],
            ),
            (
                "Tab strip",
                &[
                    "tab_strip",
                    "style.tabs.gap",
                    "style.tabs.scrollbar_width",
                    "style.tabs.status_size",
                ],
            ),
            (
                "New tab button",
                &["add_tab", "add_tab_border", "style.tabs.add_radius"],
            ),
        ],
    ),
    (
        Section::Panes,
        &[
            (
                "Panes",
                &[
                    "card",
                    "card_border",
                    "shadow",
                    "style.panes.border_width",
                    "style.panes.radius",
                    "style.panes.gap",
                    "style.panes.margin",
                ],
            ),
            (
                "Inside panes",
                &[
                    "style.panes.padding",
                    "style.panes.terminal_padding",
                    "style.panes.address_size",
                ],
            ),
            (
                "Outlines",
                &[
                    "outline",
                    "style.panes.focus_border_width",
                    "style.panes.attention_border_width",
                ],
            ),
        ],
    ),
    (
        Section::Menus,
        &[
            (
                "Menus",
                &[
                    "popup",
                    "item_edge",
                    "menu_shadow",
                    "style.menus.width",
                    "style.menus.radius",
                    "style.menus.border_width",
                    "style.menus.padding",
                    "style.menus.margin",
                    "style.menus.submenu_gap",
                    "style.menus.shadow_offset",
                    "style.menus.shadow_blur",
                ],
            ),
            (
                "Menu entries",
                &[
                    "hover",
                    "style.menus.item_height",
                    "style.menus.item_padding",
                    "style.menus.spacing",
                    "style.menus.text_size",
                    "style.menus.icon_size",
                    "style.menus.header_size",
                    "style.menus.header_height",
                    "style.menus.separator_height",
                ],
            ),
            (
                "Icon and color pickers",
                &["style.menus.grid_cell", "style.menus.picker_icon_size"],
            ),
            (
                "Buttons and fields",
                &[
                    "raised",
                    "sunken",
                    "knob",
                    "style.controls.radius",
                    "style.controls.border_width",
                    "style.controls.icon_button_size",
                    "style.controls.field_text_size",
                ],
            ),
            ("Dialogs", &["overlay"]),
        ],
    ),
    (
        Section::Effects,
        &[
            (
                "Items",
                &[
                    "style.effects.dim",
                    "style.effects.hover",
                    "style.effects.shadow",
                ],
            ),
            (
                "Dragging",
                &[
                    "insertion",
                    "style.controls.insertion_line_width",
                    "style.controls.highlight_width",
                    "style.effects.dragged",
                    "style.effects.drag_preview",
                    "style.effects.drop_tint",
                ],
            ),
            (
                "Glass",
                &[
                    "glass_light",
                    "glass_dark",
                    "glass_rim",
                    "style.effects.glass_idle",
                    "style.effects.blur",
                    "style.effects.shine",
                    "style.effects.lift",
                    "style.effects.lift_clear",
                    "style.effects.refraction",
                ],
            ),
            (
                "Backdrop",
                &[
                    "backdrop_top",
                    "backdrop_bottom",
                    "glow_1",
                    "glow_2",
                    "glow_3",
                    "glow_4",
                    "glow_5",
                ],
            ),
        ],
    ),
];

/// The Quick Theme page's one card: its title, then its colors by name.
const QUICK_CARD: (&str, &[&str]) = (
    "Colors",
    &[
        "background",
        "tab_strip",
        "tab",
        "tab_hover",
        "tab_active",
        "card",
        "sidebar",
        "border",
        "workspace",
        "workspace_active",
        "workspace_hover",
    ],
);

/// What a color row shows while its color is unset, worked out once per list.
struct ColorDefaults {
    /// The interface's own colors, before the config.
    base: Chrome,
    /// The theme colors in effect.
    theme: ThemeColors,
    /// The theme colors before the config.
    theme_base: ThemeColors,
}

impl App {
    /// Every row, section by section. Rows of a section and group must be next to each other,
    /// since the dialog starts a new card whenever the (section, group) changes.
    pub(super) fn settings_list(&self) -> Vec<Setting> {
        let config = &self.config;
        let mut list = self.general_settings();
        list.extend(self.terminal_settings());
        list.extend(self.notification_settings());
        list.extend(keymap::named_bindings(&config.keybindings).into_iter().map(
            |(chord, name)| {
                Setting::new(
                    Section::Shortcuts,
                    "Shortcuts",
                    &action_label(&name),
                    "",
                    Control::Shortcut(chord_label(&chord)),
                )
            },
        ));
        list.extend(self.agent_settings());
        list.extend(self.browser_settings());
        list.extend(self.vscode_settings());
        list.extend(self.look_settings());
        // Dev-only (`Section::shown_in_build`), left out so search can't reach it.
        if cfg!(debug_assertions) {
            list.extend(self.quick_settings());
        }
        list
    }

    /// General: restoring the session.
    fn general_settings(&self) -> Vec<Setting> {
        vec![Setting::new(
            Section::General,
            "Session",
            "Restore session",
            "Reopen the last layout on launch: workspaces, splits, tabs and directories.",
            Control::Toggle("session.restore", self.config.session.restore),
        )]
    }

    /// Browser: where terminal links open, and the home page.
    fn browser_settings(&self) -> Vec<Setting> {
        use Section::Browser;
        let config = &self.config;
        let mut list = vec![
            Setting::new(
                Browser,
                "Browser",
                "Open links in",
                "Where links clicked in a terminal open.",
                choice(
                    "browser.open_links",
                    vec![
                        ("Browser panel".into(), str_value("panel")),
                        ("System browser".into(), str_value("system")),
                    ],
                    &str_value(match config.browser.open_links {
                        OpenLinks::Panel => "panel",
                        OpenLinks::System => "system",
                    }),
                ),
            ),
            Setting::new(
                Browser,
                "Browser",
                "Home page",
                "The page a new browser tab opens. Press Enter to apply.",
                Control::Text {
                    key: "browser.home",
                    value: config.browser.home.clone(),
                    placeholder: "https://www.google.com/",
                },
            ),
        ];
        // Chromium is only an option on Windows.
        if cfg!(windows) {
            let engine = match config.browser.engine {
                BrowserEngine::System => "system",
                BrowserEngine::Chromium => "chromium",
            };
            let engines = vec![
                ("System (Edge)".into(), str_value("system")),
                ("Chromium".into(), str_value("chromium")),
            ];
            list.insert(
                1,
                Setting::new(
                    Browser,
                    "Browser",
                    "Engine",
                    "What shows pages: Edge's WebView2, built into Windows, or Chromium once \
                     it's downloaded below. Applies to pages opened afterwards.",
                    choice("browser.engine", engines, &str_value(engine)),
                ),
            );
        }
        if crate::chromium::available() {
            list.push(self.chromium_setting());
        }
        list
    }

    /// The Chromium download: its state, and a button to download or remove it.
    fn chromium_setting(&self) -> Setting {
        let state = &self.chromium;
        let (mut description, button, message) = if state.busy {
            (
                "Working\u{2026}".to_owned(),
                "Please wait",
                SettingsMessage::Chromium(true),
            )
        } else if state.installed {
            (
                format!(
                    "Downloaded to {}. Set the engine to Chromium to use it.",
                    crate::chromium::dir().display()
                ),
                "Remove",
                SettingsMessage::Chromium(false),
            )
        } else {
            (
                "Embedded Chromium, for pages that need Chrome's engine. Downloads the Chromium \
                 Embedded Framework build this version of macaroni uses (about 175 MB)."
                    .to_owned(),
                "Download",
                SettingsMessage::Chromium(true),
            )
        };
        if let Some(error) = &state.error {
            description = format!("{description}\n{error}");
        }
        Setting::new(
            Section::Browser,
            "Chromium",
            "Chromium",
            &description,
            Control::Button(button.to_owned(), Message::Settings(message)),
        )
    }

    /// VS Code: download or remove it, and its server while it runs. Nothing where browser
    /// tabs don't exist.
    fn vscode_settings(&self) -> Vec<Setting> {
        if !crate::vscode::available() {
            return Vec::new();
        }
        let web = &self.vscode_web;
        let (mut description, button, message) = if web.busy {
            (
                "Working\u{2026}".to_owned(),
                "Please wait",
                SettingsMessage::VsCode(true),
            )
        } else if web.installed {
            (
                format!(
                    "Installed in {}. Open a VS Code tab from a pane's + menu or the tab strip.",
                    crate::vscode::dir().display()
                ),
                "Remove",
                SettingsMessage::VsCode(false),
            )
        } else {
            (
                format!(
                    "Opt in to run VS Code for the web in browser tabs. Downloads Microsoft's VS \
                     Code CLI (about 10 MB); the first tab then fetches the VS Code server. \
                     Downloading accepts its license: {}",
                    crate::vscode::LICENSE_URL
                ),
                "Download",
                SettingsMessage::VsCode(true),
            )
        };
        if let Some(error) = &web.error {
            description = format!("{description}\n{error}");
        }
        let mut rows = vec![Setting::new(
            Section::VsCode,
            "Download",
            "VS Code in tabs",
            &description,
            Control::Button(button.to_owned(), Message::Settings(message)),
        )];
        if let Some(server) = &web.server {
            let state = if web.ready { "Running" } else { "Starting" };
            rows.push(Setting::new(
                Section::VsCode,
                "Server",
                "Server",
                &format!(
                    "{state} on 127.0.0.1:{}, for this machine only. Stopping it disconnects \
                     open VS Code tabs; the next one starts it again.",
                    server.port
                ),
                Control::Button(
                    "Stop".to_owned(),
                    Message::Settings(SettingsMessage::VsCodeStop),
                ),
            ));
        }
        rows
    }

    /// The Appearance pages, laid out by [`LOOK`].
    fn look_settings(&self) -> Vec<Setting> {
        let themes: Vec<(String, Value)> = Theme::names(&self.config_dir)
            .into_iter()
            .map(|name| (name.clone(), Value::Str(name)))
            .collect();
        let mut items = item_rows(&self.config, themes);
        let defaults = self.color_defaults();
        let mut list = Vec::new();
        for &(section, cards) in LOOK {
            for &(card, keys) in cards {
                for &key in keys {
                    let row = match items.iter().position(|s| s.control.key() == Some(key)) {
                        Some(at) => Some(items.swap_remove(at)),
                        None => self.look_row(&defaults, key),
                    };
                    if let Some(mut row) = row {
                        row.section = section;
                        row.group = card;
                        list.push(row);
                    }
                }
            }
        }
        list
    }

    /// The row for a [`LOOK`] key that's a color, theme color or `[style.*]` number; `None` for
    /// one the interface in use doesn't draw.
    fn look_row(&self, defaults: &ColorDefaults, key: &str) -> Option<Setting> {
        let ui = self.config.ui_theme;
        if let Some(setting) = number_setting(key) {
            if ui == UiTheme::Solid && GLASS_NUMBERS.contains(&key) {
                return None;
            }
            return Some(self.style_row(Section::Theme, setting));
        }
        if let Some(theme) = THEME_COLORS.iter().find(|t| t.name == key) {
            return self.theme_color(defaults, theme);
        }
        let color = COLORS.iter().find(|c| c.name == key)?;
        self.element_color(defaults, color)
    }

    /// The shared values [`App::element_color`] and [`App::theme_color`] fall back to.
    fn color_defaults(&self) -> ColorDefaults {
        let ui = self.config.ui_theme;
        ColorDefaults {
            base: Chrome::base(&Palette::from_theme(&self.config.theme), ui),
            theme: ThemeColors::new(ui, self.config.colors.for_ui(ui)),
            theme_base: ThemeColors::base(ui),
        }
    }

    /// A theme color's row: a swatch and a field, without the theme options (theme colors are
    /// the end of the chain).
    fn theme_color(&self, defaults: &ColorDefaults, t: &ThemeColor) -> Option<Setting> {
        let ui = self.config.ui_theme;
        Some(Setting::new(
            Section::Theme,
            "",
            t.label,
            t.description,
            Control::Color {
                key: t.key(ui),
                value: defaults.theme.get(t.name)?,
                default: defaults.theme_base.get(t.name)?,
                link: None,
                themed: false,
                set: self.config.colors.for_ui(ui).get(t.name).cloned(),
            },
        ))
    }

    /// An element color's row; `None` for one the interface in use doesn't draw.
    fn element_color(&self, defaults: &ColorDefaults, color: &ColorSetting) -> Option<Setting> {
        let ui = self.config.ui_theme;
        let drawn = match ui {
            UiTheme::Solid => !color.glass_only,
            UiTheme::Glass => color.group != SOLID_GROUP,
        };
        if !drawn {
            return None;
        }
        // While unset a color is the interface's own, or its theme color's.
        let link = THEME_COLORS.iter().find(|t| t.name == color.link);
        Some(Setting::new(
            Section::Theme,
            "",
            color.label,
            color.description,
            Control::Color {
                key: color.key(ui),
                value: self.chrome.color(color.name)?,
                default: match link {
                    Some(t) => defaults.theme.get(t.name)?,
                    None => defaults.base.color(color.name)?,
                },
                link: link.map(|t| t.label),
                themed: true,
                set: self.config.colors.for_ui(ui).get(color.name).cloned(),
            },
        ))
    }

    /// A `[style.*]` number's row.
    fn style_row(&self, section: Section, setting: &'static NumberSetting) -> Setting {
        let value = self
            .config
            .style
            .get(setting.key)
            .unwrap_or(setting.default_for(self.config.ui_theme));
        number_row(section, setting, value)
    }

    /// Terminal: the shell, font, cursor, history and mouse.
    fn terminal_settings(&self) -> Vec<Setting> {
        use Section::Terminal;
        let config = &self.config;
        let terminal = &config.terminal;
        let font = &config.font;
        let mut scrollback: Vec<(String, Value)> = [1_000, 5_000, 10_000, 50_000, 100_000]
            .into_iter()
            .map(|n| (format!("{} lines", thousands(n)), Value::Int(n)))
            .collect();
        let lines = terminal.scrollback as i64;
        if !scrollback.iter().any(|(_, v)| *v == Value::Int(lines)) {
            scrollback.push((format!("{} lines", thousands(lines)), Value::Int(lines)));
        }
        let (options, default) = self
            .settings
            .as_ref()
            .map(|s| (s.shells.clone(), s.default_shell.clone()))
            .unwrap_or_default();
        let mut rows = vec![
            Setting::new(
                Terminal,
                "Shell",
                "Shell program",
                "What new terminals run. Right-click a pane's + to open any shell once.",
                Control::Shell {
                    program: config.shell.program.clone().unwrap_or_default(),
                    args: config.shell.args.clone(),
                    options,
                    default,
                },
            ),
            Setting::new(
                Terminal,
                "Shell",
                "Arguments",
                "Passed to a shell you set yourself, separated by spaces. Press Enter to apply.",
                Control::List {
                    key: "shell.args",
                    value: config.shell.args.clone(),
                    placeholder: "None",
                },
            ),
            Setting::new(
                Terminal,
                "Shell",
                "Default folder",
                "Where terminals start in a workspace without a folder of its own. Empty is your \
                 home folder; ~ stands for it. Press Enter to apply.",
                Control::Text {
                    key: "terminal.default_dir",
                    value: terminal.default_dir.clone(),
                    placeholder: "Home folder",
                },
            ),
            Setting::new(
                Terminal,
                "Font",
                "Font family",
                "Any installed monospace font. Empty uses the bundled JetBrains Mono.",
                Control::Text {
                    key: "font.family",
                    value: font.family.clone().unwrap_or_default(),
                    placeholder: "JetBrains Mono",
                },
            ),
            number_row(Terminal, &FONT_SIZE, font.size),
            number_row(Terminal, &LINE_HEIGHT, font.line_height),
            Setting::new(
                Terminal,
                "Font",
                "Ligatures",
                "Join character pairs like => and != where the font supports it.",
                Control::Toggle("font.ligatures", font.ligatures),
            ),
            Setting::new(
                Terminal,
                "Cursor",
                "Cursor shape",
                "Until a program asks for another, as editors often do.",
                choice(
                    "terminal.cursor_style",
                    vec![
                        ("Block".into(), str_value("block")),
                        ("Underline".into(), str_value("underline")),
                        ("Bar".into(), str_value("beam")),
                    ],
                    &str_value(match terminal.cursor_style {
                        CursorStyle::Block => "block",
                        CursorStyle::Underline => "underline",
                        CursorStyle::Beam => "beam",
                    }),
                ),
            ),
            Setting::new(
                Terminal,
                "History",
                "Scrollback",
                "Lines each terminal keeps. Applies to new terminals.",
                choice("terminal.scrollback", scrollback, &Value::Int(lines)),
            ),
            number_row(Terminal, &SCROLL_LINES, terminal.scroll_lines as f32),
            Setting::new(
                Terminal,
                "Mouse",
                "Copy on select",
                "Copy text to the clipboard as soon as it's selected.",
                Control::Toggle("terminal.copy_on_select", terminal.copy_on_select),
            ),
        ];
        // After the two shell rows.
        rows.splice(2..2, self.ssh_settings());
        rows
    }

    /// The SSH profiles card: one row per `[[ssh]]` profile, then fields to add one.
    fn ssh_settings(&self) -> Vec<Setting> {
        const GROUP: &str = "SSH profiles";
        let mut rows: Vec<Setting> = self
            .config
            .ssh
            .iter()
            .enumerate()
            .map(|(i, profile)| {
                Setting::new(
                    Section::Terminal,
                    GROUP,
                    profile.label(),
                    &format!("ssh {}", profile.ssh_args().join(" ")),
                    Control::Button(
                        "Remove".to_owned(),
                        Message::Settings(SettingsMessage::RemoveSsh(i)),
                    ),
                )
            })
            .collect();
        rows.push(Setting::new(
            Section::Terminal,
            GROUP,
            "New profile",
            "Opens from a pane's + menu (right-click it). Type a host, user@host or a Host from \
             ~/.ssh/config, and press Enter. A key file and more ssh arguments go in config.toml.",
            Control::NewSsh,
        ));
        rows
    }

    /// Notifications: when a tab asks for attention, and the bell.
    fn notification_settings(&self) -> Vec<Setting> {
        use Section::Notifications;
        let n = &self.config.notifications;
        vec![
            Setting::new(
                Notifications,
                "Attention",
                "System notifications",
                "Show an OS notification when a tab that isn't in view asks for attention.",
                Control::Toggle("notifications.os", n.os),
            ),
            Setting::new(
                Notifications,
                "Bell",
                "Bell marks the tab",
                "Treat the terminal bell as a request for attention.",
                Control::Toggle("notifications.bell", n.bell),
            ),
            Setting::new(
                Notifications,
                "Bell",
                "Bell sound",
                "What the bell sounds like. A file is a .wav on Windows; anything afplay plays on \
                 macOS. Muted tabs stay quiet.",
                Control::Template {
                    key: "notifications.bell_sound",
                    value: n.bell_sound.clone(),
                    presets: vec![
                        ("Off".into(), "off".into()),
                        ("System sound".into(), "system".into()),
                    ],
                    placeholder: "Path to a sound file",
                },
            ),
        ]
    }

    /// Agents: Claude Code's hooks, then how agents show up.
    fn agent_settings(&self) -> Vec<Setting> {
        use Section::Agents;
        let config = &self.config;
        let agents = &config.agents;
        let (hooks, result) = self
            .settings
            .as_ref()
            .map(|s| (s.hooks, s.hooks_result.clone()))
            .unwrap_or_default();
        let mut status = match hooks {
            Some(true) => "Installed: Claude Code reports what it's doing and when it needs you.",
            _ => "Not installed: Claude Code's state and notifications won't show in macaroni.",
        }
        .to_owned();
        if let Some(result) = result {
            status = format!("{status} {result}");
        }
        let installed = hooks == Some(true);
        let mut hooks_row = Setting::new(
            Agents,
            "Claude Code",
            "Hooks and status line",
            "",
            Control::Button(
                if installed { "Remove" } else { "Install" }.to_owned(),
                Message::Settings(SettingsMessage::Hooks(!installed)),
            ),
        );
        hooks_row.description = status;
        vec![
            hooks_row,
            Setting::new(
                Agents,
                "Claude Code",
                "Resume after restart",
                "In restored tabs that were running Claude Code, run claude --resume.",
                Control::Toggle("session.resume_agents", config.session.resume_agents),
            ),
            Setting::new(
                Agents,
                "Notifications",
                "When an agent finishes",
                "Mark the tab, and notify if it's out of view.",
                Control::Toggle("agents.notify_finished", agents.notify_finished),
            ),
            Setting::new(
                Agents,
                "Notifications",
                "When an agent needs you",
                "Asking for input or permission.",
                Control::Toggle("agents.notify_input", agents.notify_input),
            ),
            Setting::new(
                Agents,
                "In tabs",
                "Agent icon",
                "Tabs running an agent show sparkles, tinted by its state.",
                Control::Toggle("agents.tab_icon", agents.tab_icon),
            ),
        ]
    }

    /// Quick Theme: [`QUICK_CARD`]'s colors, all copies of rows on the Appearance pages (so left
    /// out of search). Glass is its own look, so there the page only offers to switch.
    fn quick_settings(&self) -> Vec<Setting> {
        use Section::Quick;
        let (card, names) = QUICK_CARD;
        if self.config.ui_theme == UiTheme::Glass {
            let mut row = Setting::new(
                Quick,
                card,
                "Use the solid interface",
                "These colors shape the solid interface, so switch to it to try them.",
                Control::Button(
                    "Use Solid".into(),
                    Message::Settings(SettingsMessage::Set("ui_theme", Some(str_value("solid")))),
                ),
            );
            row.repeat = true;
            return vec![row];
        }
        let defaults = self.color_defaults();
        names
            .iter()
            .filter_map(|name| COLORS.iter().find(|c| c.name == *name))
            .filter_map(|color| self.element_color(&defaults, color))
            .map(|mut row| {
                row.section = Quick;
                row.group = card;
                row.repeat = true;
                row
            })
            .collect()
    }
}

/// The Interface choice.
fn interface_choice(ui: UiTheme) -> Control {
    Control::Dropdown(
        "ui_theme",
        vec![
            ("Glass".into(), str_value("glass")),
            ("Solid".into(), str_value("solid")),
        ],
        Some(usize::from(ui == UiTheme::Solid)),
    )
}

/// A number's row in `section`, under its own group.
fn number_row(section: Section, setting: &'static NumberSetting, value: f32) -> Setting {
    Setting::new(
        section,
        setting.group,
        setting.label,
        setting.description,
        Control::Number { setting, value },
    )
}

/// A `[workspace_items]` or `[tab_items]` template's control: its presets, or a custom one.
fn template(key: &'static str, value: &str, presets: &[(&str, &str)]) -> Control {
    Control::Template {
        key,
        value: value.to_owned(),
        presets: presets
            .iter()
            .map(|(l, t)| ((*l).to_owned(), (*t).to_owned()))
            .collect(),
        placeholder: "Empty hides it",
    }
}

/// The Appearance rows that aren't colors or numbers: the interface and terminal themes
/// (`themes` are the terminal themes found), `[workspace_items]` and `[tab_items]`. Their
/// section and card come from where [`LOOK`] lists their keys.
fn item_rows(config: &Config, themes: Vec<(String, Value)>) -> Vec<Setting> {
    let ws = &config.workspace_items;
    let tab = &config.tab_items;
    let ui = config.ui_theme;
    let row =
        |label, description, control| Setting::new(Section::Theme, "", label, description, control);
    vec![
        row(
            "Interface",
            "How the sidebar, tabs and panes are drawn: frosted Glass, or flat Solid. Each keeps its own colors and sizes.",
            interface_choice(ui),
        ),
        row(
            "Terminal colors",
            "The terminal's color theme. Add your own to the themes folder next to config.toml.",
            choice("theme", themes, &str_value(&config.theme_name)),
        ),
        row(
            "Show search bar",
            "The search field at the top of the sidebar.",
            Control::Toggle("workspace_items.search", ws.search(ui)),
        ),
        row(
            "Show icon",
            "The icon at the left of each workspace.",
            Control::Toggle("workspace_items.icon", ws.icon),
        ),
        row(
            "Icon background",
            "The tile behind each workspace's icon. Off, the icon shows on its own.",
            Control::Toggle("workspace_items.icon_background", ws.icon_background(ui)),
        ),
        row(
            "Title",
            "Custom: {name}, {title}, {dir}, {index}, or any detail placeholder.",
            template(
                "workspace_items.title",
                &ws.title,
                &[
                    ("Name", "{name}"),
                    ("Focused tab", "{title}"),
                    ("Folder", "{dir}"),
                    ("Hidden", ""),
                ],
            ),
        ),
        row(
            "Detail",
            "The line under the title; one that comes out empty is hidden. Custom: {activity}, \
             {status}, {summary}, {tabs}, {panes}, {notice}, {agent}, {dir}, {path}, {title}.",
            template(
                "workspace_items.detail",
                &ws.detail,
                &[
                    ("Activity", "{activity}"),
                    ("Status", "{status}"),
                    ("Summary", "{summary}"),
                    ("Tab count", "{tabs} tabs"),
                    ("Pane count", "{panes} panes"),
                    ("Notifications", "{notice}"),
                    ("Agent", "{agent}"),
                    ("Directory", "{path}"),
                    ("Hidden", ""),
                ],
            ),
        ),
        row(
            "Badge",
            "Small text on the right. Custom: {shortcut}, {index}, {tabs}, {panes}.",
            template(
                "workspace_items.badge",
                &ws.badge,
                &[
                    ("Shortcut", "{shortcut}"),
                    ("Position", "{index}"),
                    ("Tab count", "{tabs}"),
                    ("Pane count", "{panes}"),
                    ("Hidden", ""),
                ],
            ),
        ),
        row(
            "Muted and pinned marks",
            "The bell and pin beside a muted or pinned workspace's name.",
            Control::Toggle("workspace_items.status_icons", ws.status_icons),
        ),
        row(
            "Close button",
            "Shown on the hovered and selected workspaces.",
            Control::Toggle("workspace_items.close_button", ws.close_button),
        ),
        row(
            "Icon",
            "Pinned tabs always show theirs.",
            Control::Toggle("tab_items.icon", tab.icon),
        ),
        row(
            "Title",
            "Custom: {title}, {name}, {dir}, {path}, {url}, {kind}, {index}, {notice}, {agent}.",
            template(
                "tab_items.title",
                &tab.title,
                &[
                    ("Title", "{title}"),
                    ("Folder", "{dir}"),
                    ("Kind", "{kind}"),
                    ("Hidden", ""),
                ],
            ),
        ),
        row(
            "Badge",
            "Small text after the title, with the same placeholders.",
            template(
                "tab_items.badge",
                &tab.badge,
                &[("Hidden", ""), ("Position", "{index}"), ("Folder", "{dir}")],
            ),
        ),
        row(
            "Muted mark",
            "The crossed-out bell on a muted tab.",
            Control::Toggle("tab_items.status_icons", tab.status_icons),
        ),
        row(
            "Close button",
            "The x at the right of each tab.",
            Control::Toggle("tab_items.close_button", tab.close_button),
        ),
        row(
            "New tab buttons",
            "Terminal, browser and VS Code (when installed) buttons at the right of the strip.",
            Control::Toggle("tab_items.new_buttons", tab.new_buttons),
        ),
    ]
}

/// A string setting's value.
fn str_value(v: &str) -> Value {
    Value::Str(v.to_owned())
}

/// A choice among `options`, with whichever is `current` selected.
fn choice(key: &'static str, options: Vec<(String, Value)>, current: &Value) -> Control {
    let at = options.iter().position(|(_, v)| v == current);
    Control::Choice(key, options, at)
}

/// `color` as `#rrggbb`, with `aa` when it isn't opaque.
pub(super) fn to_hex(color: Color) -> String {
    let [r, g, b, a] = color.into_rgba8();
    Rgba(r, g, b, a).to_string()
}

/// The theme color a `[colors.*]` value follows, by label; `None` for a plain color.
pub(super) fn theme_label(value: &ColorValue) -> Option<&'static str> {
    match value {
        ColorValue::Theme(name) => THEME_COLORS
            .iter()
            .find(|t| t.name == name)
            .map(|t| t.label),
        ColorValue::Rgba(_) => None,
    }
}

/// `v` as the TOML value to write: an integer when it's whole (so `14` isn't written `14.0`),
/// else a float rounded to 3 places, dropping `f32` noise like `0.550000011920929`.
pub(super) fn number(v: f32) -> Value {
    if v.fract() == 0.0 {
        Value::Int(v as i64)
    } else {
        Value::Float((f64::from(v) * 1000.0).round() / 1000.0)
    }
}

/// `n` with commas between thousands, as `100,000`. Only for non-negative counts.
fn thousands(n: i64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// `new_tab` as "New tab", `workspace_3` as "Workspace 3".
fn action_label(name: &str) -> String {
    let words = name.replace('_', " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// A chord as it's shown, e.g. Ctrl+Shift+T.
fn chord_label(chord: &Chord) -> String {
    let mut parts: Vec<String> = Vec::new();
    let logo = if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Win"
    };
    for (on, name) in [
        (chord.mods.ctrl, "Ctrl"),
        (
            chord.mods.alt,
            if cfg!(target_os = "macos") {
                "Option"
            } else {
                "Alt"
            },
        ),
        (chord.mods.shift, "Shift"),
        (chord.mods.logo, logo),
    ] {
        if on {
            parts.push(name.to_owned());
        }
    }
    parts.push(match chord.key {
        ChordKey::Char(c) => c.to_uppercase().to_string(),
        ChordKey::Digit(d) => d.to_string(),
        ChordKey::Named(NamedKey::F(n)) => format!("F{n}"),
        ChordKey::Named(NamedKey::PageUp) => "PageUp".to_owned(),
        ChordKey::Named(NamedKey::PageDown) => "PageDown".to_owned(),
        ChordKey::Named(named) => format!("{named:?}"),
    });
    parts.join("+")
}

/// `14`, `12.5` or `0.55`: no trailing zeros, no float noise.
pub(super) fn format_number(v: f32) -> String {
    let rounded = (f64::from(v) * 1000.0).round() / 1000.0;
    format!("{rounded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(action_label("new_tab"), "New tab");
        assert_eq!(action_label("workspace_3"), "Workspace 3");
        assert_eq!(thousands(100_000), "100,000");
        assert_eq!(thousands(500), "500");
        assert_eq!(format_number(14.0), "14");
        assert_eq!(format_number(12.5), "12.5");
        assert_eq!(format_number(0.55), "0.55");
        assert_eq!(to_hex(Color::from_rgb8(1, 2, 255)), "#0102ff");
        assert_eq!(to_hex(Color::from_rgba8(1, 2, 255, 0.5)), "#0102ff80");
        let chord: Chord = "ctrl+shift+t".parse().unwrap();
        assert_eq!(chord_label(&chord), "Ctrl+Shift+T");
    }

    #[test]
    fn number_settings_are_found() {
        assert_eq!(
            number_setting("font.size").map(|s| s.unit),
            Some(Unit::Points)
        );
        assert!(number_setting("style.tabs.height").is_some());
        assert!(number_setting("nope").is_none());
    }

    #[test]
    fn other_numbers_match_the_config_defaults() {
        let config = Config::default();
        assert_eq!(FONT_SIZE.default, config.font.size);
        assert_eq!(LINE_HEIGHT.default, config.font.line_height);
        assert_eq!(SCROLL_LINES.default, config.terminal.scroll_lines as f32);
    }

    #[test]
    fn every_color_and_number_has_one_card() {
        let items: Vec<&str> = item_rows(&Config::default(), Vec::new())
            .iter()
            .filter_map(|s| s.control.key())
            .collect();
        let placed: Vec<&str> = LOOK
            .iter()
            .flat_map(|(_, cards)| cards.iter())
            .flat_map(|(_, keys)| keys.iter().copied())
            .collect();
        for key in &placed {
            let known = items.contains(key)
                || number_setting(key).is_some()
                || THEME_COLORS.iter().any(|t| t.name == *key)
                || COLORS.iter().any(|c| c.name == *key);
            assert!(known, "LOOK lists {key}, which isn't a setting");
            let count = placed.iter().filter(|k| *k == key).count();
            assert_eq!(count, 1, "LOOK lists {key} {count} times");
        }
        let wanted = items
            .iter()
            .copied()
            .chain(Style::settings().map(|s| s.key))
            .chain(THEME_COLORS.iter().map(|t| t.name))
            .chain(COLORS.iter().map(|c| c.name));
        for key in wanted {
            assert!(placed.contains(&key), "{key} isn't on any Appearance card");
        }
    }
}

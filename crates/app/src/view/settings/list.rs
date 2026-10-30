//! Every row the settings dialog shows, built from the current config.
//!
//! [`App::settings_list`] is rebuilt on every draw of the dialog, so each row's value is what's
//! in effect. To add a setting, add a row here (and read it somewhere). The look's rows come
//! from config metadata instead: `macaroni_config::COLORS`, `THEME_COLORS` and
//! `Style::settings`, so a new color or `[style.*]` number shows up without touching this file.
//! Appearance holds everything visual, each part's sizes next to its colors; a few colors are
//! repeated, as copies left out of search, on the functional pages that use them.

use iced::Color;
use macaroni_config::{
    COLORS, Chord, ChordKey, ColorValue, Config, CursorStyle, NamedKey, NumberSetting, OpenLinks,
    Rgba, Style, THEME_COLORS, Theme, UiTheme, Unit, Value,
};

use super::{Control, Section, Setting, SettingsMessage};
use crate::theme::{Chrome, Palette, ThemeColors};
use crate::{App, MAX_FONT_SIZE, MIN_FONT_SIZE, Message, keymap};

/// `[font] size`, as a number row.
static FONT_SIZE: NumberSetting = NumberSetting {
    key: "font.size",
    group: "Terminal font",
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
    group: "Terminal font",
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

/// Where the colors of each `COLORS` group are repeated besides Appearance: its `group`, then
/// the section and card title it's repeated under.
const COLOR_REPEATS: &[(&str, Section, &str)] = &[
    ("Accents", Section::Notifications, "Colors"),
    ("Workspaces and tabs", Section::Items, "Colors"),
];

/// The `COLORS` group of the colors only the solid interface has, listed only on the Solid
/// Theme page.
const SOLID_GROUP: &str = "Solid theme";

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

/// The Solid Theme page's cards: each title, then its colors by name and numbers by dotted key
/// (a key [`number_setting`] knows is a number), in order.
const THEME_CARDS: &[(&str, &[&str])] = &[
    (
        "Window",
        &[
            "background",
            "title_bar",
            "border",
            "title_bar_border",
            "style.window.title_bar_border_width",
            "style.window.title_bar_gap",
        ],
    ),
    (
        "Sidebar",
        &[
            "sidebar",
            "sidebar_border",
            "style.sidebar.border_width",
            "style.sidebar.top_padding",
            "style.sidebar.padding",
            "style.sidebar.bottom_padding",
            "style.sidebar.panes_gap",
        ],
    ),
    (
        "Search bar",
        &[
            SEARCH_TOGGLE,
            "search",
            "style.sidebar.search_height",
            "style.sidebar.search_padding",
            "style.sidebar.search_gap",
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
            "workspace_icon",
            ICON_BACKGROUND_TOGGLE,
            "style.workspaces.tile_size",
            "style.workspaces.tile_radius",
            "style.workspaces.padding",
            "style.workspaces.radius",
        ],
    ),
    (
        "Tabs",
        &[
            "tab_strip",
            "tab",
            "tab_active",
            "tab_hover",
            "tab_border",
            "tab_border_active",
            "tab_border_hover",
            "tab_accent",
            "style.tabs.accent_width",
            "style.tabs.radius",
            "style.tabs.gap",
        ],
    ),
    (
        "Panels",
        &[
            "card",
            "card_border",
            "shadow",
            "style.panes.radius",
            "style.panes.border_width",
            "style.panes.gap",
            "style.panes.margin",
        ],
    ),
    (
        "New workspace button",
        &[
            "add_workspace",
            "add_workspace_border",
            "style.workspaces.add_radius",
        ],
    ),
    (
        "New tab button",
        &["add_tab", "add_tab_border", "style.tabs.add_radius"],
    ),
];

impl App {
    /// Every row, section by section. Rows of a section and group must be next to each other,
    /// since the dialog starts a new card whenever the (section, group) changes.
    pub(super) fn settings_list(&self) -> Vec<Setting> {
        let config = &self.config;
        let mut list = self.general_settings();
        list.extend(self.appearance_settings());
        // Dev-only pages (`Section::shown_in_build`), left out so search can't reach them.
        if cfg!(debug_assertions) {
            list.extend(self.quick_settings());
            list.extend(self.theme_settings());
        }
        list.extend(self.terminal_settings());
        list.extend(self.notification_settings());
        list.extend(self.agent_settings());
        list.extend(self.item_settings());
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
        list
    }

    /// General: the session and the browser.
    fn general_settings(&self) -> Vec<Setting> {
        use Section::General;
        let config = &self.config;
        vec![
            Setting::new(
                General,
                "Session",
                "Restore session",
                "Reopen the last layout on launch: workspaces, splits, tabs and directories.",
                Control::Toggle("session.restore", config.session.restore),
            ),
            Setting::new(
                General,
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
                General,
                "Browser",
                "Home page",
                "The page a new browser tab opens. Press Enter to apply.",
                Control::Text {
                    key: "browser.home",
                    value: config.browser.home.clone(),
                    placeholder: "https://www.google.com/",
                },
            ),
        ]
    }

    /// Appearance: everything visual, one card per part, each part's sizes followed by its
    /// colors: the interface and terminal themes, the terminal font, the theme colors, then
    /// every `[style.*]` table, then the effects and glass colors.
    fn appearance_settings(&self) -> Vec<Setting> {
        use Section::Appearance;
        let themes: Vec<(String, Value)> = Theme::names(&self.config_dir)
            .into_iter()
            .map(|name| (name.clone(), Value::Str(name)))
            .collect();
        let font = &self.config.font;
        let mut list = vec![
            Setting::new(
                Appearance,
                "Interface",
                "Theme",
                "How the sidebar, tabs and panes are drawn. Glass is a fixed frosted preset; Solid is flat, and the Solid Theme page sets its colors, borders and spacing.",
                interface_choice(self.config.ui_theme),
            ),
            Setting::new(
                Appearance,
                "Terminal colors",
                "Color theme",
                "Terminal colors. Add your own to the themes folder next to config.toml.",
                choice("theme", themes, &str_value(&self.config.theme_name)),
            ),
            Setting::new(
                Appearance,
                "Terminal font",
                "Font family",
                "Any installed monospace font. Empty uses the bundled JetBrains Mono.",
                Control::Text {
                    key: "font.family",
                    value: font.family.clone().unwrap_or_default(),
                    placeholder: "JetBrains Mono",
                },
            ),
            number_row(Appearance, &FONT_SIZE, font.size),
            number_row(Appearance, &LINE_HEIGHT, font.line_height),
            Setting::new(
                Appearance,
                "Terminal font",
                "Ligatures",
                "Join character pairs like => and != where the font supports it.",
                Control::Toggle("font.ligatures", font.ligatures),
            ),
        ];
        list.extend(self.theme_colors());
        list.extend(self.colors_card("Accents", "Accent colors"));
        list.extend(self.colors_card("Text", "Text colors"));
        let mut table = "";
        for setting in Style::settings() {
            if setting.group != table {
                list.extend(self.colors_after(table));
                table = setting.group;
            }
            list.push(self.style_row(Appearance, setting));
        }
        list.extend(self.colors_after(table));
        list
    }

    /// The theme colors' card: each a swatch and a field.
    fn theme_colors(&self) -> Vec<Setting> {
        let ui = self.config.ui_theme;
        let overrides = self.config.colors.for_ui(ui);
        let base = ThemeColors::base(ui);
        let theme = ThemeColors::new(ui, overrides);
        THEME_COLORS
            .iter()
            .filter_map(|t| {
                Some(Setting::new(
                    Section::Appearance,
                    "Theme colors",
                    t.label,
                    t.description,
                    Control::Color {
                        key: t.key(ui),
                        value: theme.get(t.name)?,
                        default: base.get(t.name)?,
                        link: None,
                        themed: false,
                        set: overrides.get(t.name).cloned(),
                    },
                ))
            })
            .collect()
    }

    /// The colors of `COLORS` group `group`, as an Appearance card titled `title`.
    fn colors_card(&self, group: &str, title: &'static str) -> Vec<Setting> {
        self.element_colors(
            |c| c.group == group,
            |_| (Section::Appearance, title),
            false,
        )
    }

    /// The colors card that follows the `[style.*]` table titled `table`, if it has one.
    fn colors_after(&self, table: &str) -> Vec<Setting> {
        match table {
            "Window" => self.colors_card("Window", "Window colors"),
            "Tabs" => self.colors_card("Workspaces and tabs", "Workspace and tab colors"),
            "Panes" => self.colors_card("Panes", "Pane colors"),
            "Menus" => self.colors_card("Menus and dialogs", "Menu and dialog colors"),
            "Effects" => {
                let mut list = self.colors_card("Glass", "Glass colors");
                list.extend(self.colors_card("Backdrop", "Backdrop colors"));
                list
            }
            _ => Vec::new(),
        }
    }

    /// The rows for the element colors `keep` picks, under the section and card `place` gives
    /// each; `repeat` marks them as copies of the Appearance page's.
    fn element_colors(
        &self,
        keep: impl Fn(&macaroni_config::ColorSetting) -> bool,
        place: impl Fn(&macaroni_config::ColorSetting) -> (Section, &'static str),
        repeat: bool,
    ) -> Vec<Setting> {
        // Defaults show what a color is while unset: the interface's own, or its theme color's.
        let ui = self.config.ui_theme;
        let overrides = self.config.colors.for_ui(ui);
        let base = Chrome::base(&Palette::from_theme(&self.config.theme), ui);
        let theme = ThemeColors::new(ui, overrides);
        COLORS
            .iter()
            .filter(|color| ui == UiTheme::Glass || !color.glass_only)
            .filter(|color| keep(color))
            .filter_map(|color| {
                let link = THEME_COLORS.iter().find(|t| t.name == color.link);
                let (section, group) = place(color);
                let mut row = Setting::new(
                    section,
                    group,
                    color.label,
                    color.description,
                    Control::Color {
                        key: color.key(ui),
                        value: self.chrome.color(color.name)?,
                        default: match link {
                            Some(t) => theme.get(t.name)?,
                            None => base.color(color.name)?,
                        },
                        link: link.map(|t| t.label),
                        themed: true,
                        set: overrides.get(color.name).cloned(),
                    },
                );
                row.repeat = repeat;
                Some(row)
            })
            .collect()
    }

    /// The colors repeated on `section`'s page, by [`COLOR_REPEATS`].
    fn color_repeats(&self, section: Section) -> Vec<Setting> {
        let mut list = Vec::new();
        for &(group, s, title) in COLOR_REPEATS {
            if s == section {
                list.extend(self.element_colors(|c| c.group == group, |_| (s, title), true));
            }
        }
        list
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

    /// Terminal: the shell, history, cursor and mouse.
    fn terminal_settings(&self) -> Vec<Setting> {
        use Section::Terminal;
        let config = &self.config;
        let terminal = &config.terminal;
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
        vec![
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
                "History",
                "Scrollback",
                "Lines each terminal keeps. Applies to new terminals.",
                choice("terminal.scrollback", scrollback, &Value::Int(lines)),
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
            number_row(Terminal, &SCROLL_LINES, terminal.scroll_lines as f32),
            Setting::new(
                Terminal,
                "Mouse",
                "Copy on select",
                "Copy text to the clipboard as soon as it's selected.",
                Control::Toggle("terminal.copy_on_select", terminal.copy_on_select),
            ),
        ]
    }

    /// Notifications: when a tab asks for attention, and the bell.
    fn notification_settings(&self) -> Vec<Setting> {
        use Section::Notifications;
        let n = &self.config.notifications;
        let mut list = vec![
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
        ];
        list.extend(self.color_repeats(Notifications));
        list
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
                "Status in the tab strip",
                "The agent's state and status line, at the right of its pane's tabs.",
                Control::Toggle("agents.tab_status", agents.tab_status),
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

    /// Solid Theme: the solid interface's window, workspace, tab and `+` button colors, each
    /// card's borders, corners and gaps beside them ([`THEME_CARDS`]). Rows also on Appearance
    /// are repeats, left out of search; the solid-only colors are only here. Glass is a fixed
    /// preset, so there the page only offers to switch.
    fn theme_settings(&self) -> Vec<Setting> {
        use Section::Theme;
        if self.config.ui_theme == UiTheme::Glass {
            return vec![Setting::new(
                Theme,
                "Solid theme",
                "Use the solid interface",
                "Glass is a fixed preset. These settings shape the solid interface, so switch to it to try them.",
                Control::Button(
                    "Use Solid".into(),
                    Message::Settings(SettingsMessage::Set("ui_theme", Some(str_value("solid")))),
                ),
            )];
        }
        let mut list = Vec::new();
        for &(card, entries) in THEME_CARDS {
            for &entry in entries {
                if let Some(mut row) = item_toggle(&self.config, entry) {
                    row.section = Theme;
                    row.group = card;
                    row.repeat = true;
                    list.push(row);
                } else if let Some(setting) = number_setting(entry) {
                    let mut row = self.style_row(Theme, setting);
                    row.group = card;
                    row.repeat = true;
                    list.push(row);
                } else {
                    let repeat = COLORS
                        .iter()
                        .any(|c| c.name == entry && c.group != SOLID_GROUP);
                    list.extend(self.element_colors(
                        |c| c.name == entry,
                        |_| (Theme, card),
                        repeat,
                    ));
                }
            }
        }
        list
    }

    /// Quick Theme: [`QUICK_CARD`]'s colors, all copies of the Solid Theme page's (so left out
    /// of search). In glass, like Solid Theme, only the switch to solid.
    fn quick_settings(&self) -> Vec<Setting> {
        use Section::Quick;
        if self.config.ui_theme == UiTheme::Glass {
            let mut list = self.theme_settings();
            for row in &mut list {
                row.section = Quick;
                row.repeat = true;
            }
            return list;
        }
        let (card, names) = QUICK_CARD;
        let mut list = Vec::new();
        for &name in names {
            list.extend(self.element_colors(|c| c.name == name, |_| (Quick, card), true));
        }
        list
    }

    /// Sidebar & Tabs: what each workspace item and tab shows, then their colors.
    fn item_settings(&self) -> Vec<Setting> {
        let mut list = workspace_item_settings(&self.config);
        list.extend(tab_item_settings(&self.config));
        list.extend(self.color_repeats(Section::Items));
        list
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

/// `[workspace_items]`.
fn workspace_item_settings(config: &Config) -> Vec<Setting> {
    let ws = &config.workspace_items;
    let item = |label, description, control| {
        Setting::new(Section::Items, "Workspaces", label, description, control)
    };
    vec![
        item(
            "Icon",
            "The tile on the left.",
            Control::Toggle("workspace_items.icon", ws.icon),
        ),
        icon_background_toggle(config),
        item(
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
        item(
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
        item(
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
        item(
            "Muted and pinned marks",
            "The bell and pin beside a muted or pinned workspace's name.",
            Control::Toggle("workspace_items.status_icons", ws.status_icons),
        ),
        item(
            "Close button",
            "Shown on the hovered and selected workspaces.",
            Control::Toggle("workspace_items.close_button", ws.close_button),
        ),
        search_toggle(config),
    ]
}

/// The `[workspace_items]` key of the icon tile's background toggle, also an entry in
/// [`THEME_CARDS`].
const ICON_BACKGROUND_TOGGLE: &str = "workspace_items.icon_background";

/// The workspace icon tile's background on/off row, on Sidebar & Tabs.
fn icon_background_toggle(config: &Config) -> Setting {
    Setting::new(
        Section::Items,
        "Workspaces",
        "Icon background",
        "The tile behind each workspace's icon. Off, the icon shows on its own.",
        Control::Toggle(
            ICON_BACKGROUND_TOGGLE,
            config.workspace_items.icon_background(config.ui_theme),
        ),
    )
}

/// The `[workspace_items]` toggle row for `key`, if it's one [`THEME_CARDS`] lists.
fn item_toggle(config: &Config, key: &str) -> Option<Setting> {
    match key {
        SEARCH_TOGGLE => Some(search_toggle(config)),
        ICON_BACKGROUND_TOGGLE => Some(icon_background_toggle(config)),
        _ => None,
    }
}

/// The `[workspace_items]` key of the search bar's toggle, also an entry in [`THEME_CARDS`].
const SEARCH_TOGGLE: &str = "workspace_items.search";

/// The sidebar search bar's on/off row, on Sidebar & Tabs.
fn search_toggle(config: &Config) -> Setting {
    Setting::new(
        Section::Items,
        "Workspaces",
        "Search bar",
        "The search field at the top of the sidebar.",
        Control::Toggle(
            SEARCH_TOGGLE,
            config.workspace_items.search(config.ui_theme),
        ),
    )
}

/// `[tab_items]`.
fn tab_item_settings(config: &Config) -> Vec<Setting> {
    let tab = &config.tab_items;
    let item = |label, description, control| {
        Setting::new(Section::Items, "Tabs", label, description, control)
    };
    vec![
        item(
            "Icon",
            "Pinned tabs always show theirs.",
            Control::Toggle("tab_items.icon", tab.icon),
        ),
        item(
            "Title",
            "Custom: {title}, {name}, {dir}, {path}, {url}, {kind}, {index}, {notice}, \
                 {agent}.",
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
        item(
            "Badge",
            "Small text after the title, with the same placeholders.",
            template(
                "tab_items.badge",
                &tab.badge,
                &[("Hidden", ""), ("Position", "{index}"), ("Folder", "{dir}")],
            ),
        ),
        item(
            "Muted mark",
            "The crossed-out bell on a muted tab.",
            Control::Toggle("tab_items.status_icons", tab.status_icons),
        ),
        item(
            "Close button",
            "The x at the right of each tab.",
            Control::Toggle("tab_items.close_button", tab.close_button),
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
}

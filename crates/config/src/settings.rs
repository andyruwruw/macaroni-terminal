//! The schema of `config.toml` and the parsed [`Config`].
//!
//! Each TOML table is a struct here (`[font]` is [`FontSettings`], and so on), all with
//! `deny_unknown_fields` so a typo is an error naming the valid keys. The private [`File`] is the
//! file as written; [`Config::parse`] checks ranges, resolves the theme through
//! [`Theme::resolve`] and parses the keybinding chords, producing a [`Config`]. Looks
//! (`[style.*]`, `[colors.*]`) live in [`crate::style`], themes in [`crate::theme`].

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::chord::Chord;
use crate::style::{Colors, Style};
use crate::theme::{DEFAULT_THEME, Theme};

/// The settings file's name inside the config directory.
pub const FILE_NAME: &str = "config.toml";

/// Written to the config directory on first run. Every line is commented out, so it parses to
/// the defaults; a test keeps it in sync.
pub const TEMPLATE: &str = include_str!("template.toml");

/// `[font]`: the terminal font.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FontSettings {
    /// `None` uses the bundled JetBrains Mono.
    pub family: Option<String>,
    /// In points; [`Config::parse`] rejects anything outside 6 to 72.
    pub size: f32,
    /// Programming ligatures (`=>`, `!=`) in terminal text.
    pub ligatures: bool,
    /// A row's height as a multiple of `size`; [`Config::parse`] rejects anything outside 1 to
    /// 2.5.
    pub line_height: f32,
}

impl Default for FontSettings {
    fn default() -> Self {
        Self {
            family: None,
            size: 14.0,
            ligatures: false,
            line_height: 1.3,
        }
    }
}

/// `[shell]`: what new terminals run.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShellSettings {
    /// `None` runs the platform default shell.
    pub program: Option<String>,
    /// Arguments passed to `program`.
    pub args: Vec<String>,
}

/// `[terminal] cursor_style`: the cursor's shape until a program asks for another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorStyle {
    /// A filled cell.
    #[default]
    Block,
    /// A line under the cell.
    Underline,
    /// A thin bar before the cell.
    Beam,
}

/// `[terminal]`: emulator settings.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminalSettings {
    /// Lines of history kept above the screen.
    pub scrollback: usize,
    /// The cursor's shape until a program asks for another.
    pub cursor_style: CursorStyle,
    /// Lines one notch of the mouse wheel scrolls; [`Config::parse`] rejects anything outside 1
    /// to 20.
    pub scroll_lines: u32,
    /// Copy text to the clipboard as soon as it's selected.
    pub copy_on_select: bool,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            scrollback: 10_000,
            cursor_style: CursorStyle::Block,
            scroll_lines: 3,
            copy_on_select: false,
        }
    }
}

/// `[notifications]`: when tabs ask for attention.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationSettings {
    /// Show an OS notification when a tab that isn't in view asks for attention.
    pub os: bool,
    /// Treat the terminal bell as a request for attention.
    pub bell: bool,
    /// What the terminal bell sounds like: `"off"`, `"system"` (the system's alert sound) or
    /// the path of a sound file (`.wav` on Windows; anything `afplay` plays on macOS).
    pub bell_sound: String,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            os: true,
            bell: false,
            bell_sound: "off".to_owned(),
        }
    }
}

/// `[agents]`: how coding agents (Claude Code, through `macaroni-cli setup claude`) show up.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentSettings {
    /// Mark the tab and notify when an agent finishes responding.
    pub notify_finished: bool,
    /// Mark the tab and notify when an agent needs input or permission.
    pub notify_input: bool,
    /// Show an agent's state and status line at the right of its pane's tab strip.
    pub tab_status: bool,
    /// Give a tab running an agent the sparkles icon.
    pub tab_icon: bool,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            notify_finished: true,
            notify_input: true,
            tab_status: false,
            tab_icon: true,
        }
    }
}

/// `[session]`: saving and restoring the layout between runs.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionSettings {
    /// Reopen the last layout (workspaces, splits, tabs, directories) on launch.
    pub restore: bool,
    /// In restored tabs that were running Claude Code, run `claude --resume <session>`.
    pub resume_agents: bool,
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            restore: true,
            resume_agents: true,
        }
    }
}

/// How the app's own UI (sidebar, tabs, pane cards) is drawn, apart from the terminal theme:
/// `ui_theme = "solid"` or `"glass"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiTheme {
    /// Flat layered greys.
    Solid,
    /// Translucent sheets with lit edges over a deep gradient.
    #[default]
    Glass,
}

/// `[browser] open_links`: where a link clicked in a terminal opens (`"panel"` or `"system"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenLinks {
    /// In a browser split next to the terminal.
    Panel,
    /// In the system's default browser.
    System,
}

/// `[browser]`: browser tabs.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrowserSettings {
    /// The page a new browser tab opens.
    pub home: String,
    /// Where links clicked in a terminal open.
    pub open_links: OpenLinks,
}

impl Default for BrowserSettings {
    fn default() -> Self {
        Self {
            home: "https://www.google.com/".to_owned(),
            open_links: OpenLinks::Panel,
        }
    }
}

/// `[workspace_items]`: what a workspace's row in the sidebar shows. Text fields are templates:
/// `{placeholder}`s are filled in by the app, and one that comes out empty hides its part, so
/// `""` is a real value (hidden), not the default.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspaceItemSettings {
    /// The workspace's icon tile.
    pub icon: bool,
    /// The tile's background behind the icon; off, the icon shows on its own. Unset, on in
    /// glass and off in solid ([`WorkspaceItemSettings::icon_background`]).
    #[serde(rename = "icon_background")]
    pub icon_background_set: Option<bool>,
    /// The first line; `{name}` by default.
    pub title: String,
    /// The second line; `{status}` by default.
    pub detail: String,
    /// Small text on the right.
    pub badge: String,
    /// The muted and pinned marks.
    pub status_icons: bool,
    /// The row's close button.
    pub close_button: bool,
    /// The search bar at the top of the sidebar. Unset, on in glass and off in solid
    /// ([`WorkspaceItemSettings::search`]).
    #[serde(rename = "search")]
    pub search_set: Option<bool>,
}

impl WorkspaceItemSettings {
    /// Whether the sidebar has its search bar in interface `ui`: as set, else only in glass.
    pub fn search(&self, ui: crate::UiTheme) -> bool {
        self.search_set.unwrap_or(ui == crate::UiTheme::Glass)
    }

    /// Whether workspace icons sit on their tile's background in interface `ui`: as set, else
    /// only in glass.
    pub fn icon_background(&self, ui: crate::UiTheme) -> bool {
        self.icon_background_set
            .unwrap_or(ui == crate::UiTheme::Glass)
    }
}

impl Default for WorkspaceItemSettings {
    fn default() -> Self {
        Self {
            icon: true,
            icon_background_set: None,
            title: "{name}".to_owned(),
            detail: "{activity}".to_owned(),
            badge: "{shortcut}".to_owned(),
            status_icons: true,
            close_button: true,
            search_set: None,
        }
    }
}

/// `[tab_items]`: what a tab in a pane's strip shows; templates as for
/// [`WorkspaceItemSettings`]. Pinned tabs show only their icon whatever this says.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TabItemSettings {
    /// The tab's icon.
    pub icon: bool,
    /// The tab's text; `{title}` by default.
    pub title: String,
    /// Small text after the title.
    pub badge: String,
    /// The muted mark.
    pub status_icons: bool,
    /// The tab's close button.
    pub close_button: bool,
}

impl Default for TabItemSettings {
    fn default() -> Self {
        Self {
            icon: true,
            title: "{title}".to_owned(),
            badge: String::new(),
            status_icons: true,
            close_button: true,
        }
    }
}

/// `config.toml` exactly as written, before checks. Every table is optional (`serde(default)`),
/// so an empty file is all defaults. Fields mirror [`Config`]'s, except that the theme is still a
/// name and keybindings are still strings.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct File {
    /// `[font]`.
    font: FontSettings,
    /// `theme = "<name>"`: a file in `themes/` or a built-in theme.
    theme: String,
    /// `ui_theme`.
    ui_theme: UiTheme,
    /// `[shell]`.
    shell: ShellSettings,
    /// `[terminal]`.
    terminal: TerminalSettings,
    /// `[notifications]`.
    notifications: NotificationSettings,
    /// `[session]`.
    session: SessionSettings,
    /// `[agents]`.
    agents: AgentSettings,
    /// `[browser]`.
    browser: BrowserSettings,
    /// `[workspace_items]`.
    workspace_items: WorkspaceItemSettings,
    /// `[tab_items]`.
    tab_items: TabItemSettings,
    /// `[style.*]`.
    style: Style,
    /// `[colors.solid]` and `[colors.glass]`.
    colors: Colors,
    /// `[keybindings]`: chord string to action name. The map sorts by chord string, and that is
    /// the order [`Config::keybindings`] lists (and the app applies) them in.
    keybindings: BTreeMap<String, String>,
}

impl Default for File {
    fn default() -> Self {
        Self {
            font: FontSettings::default(),
            theme: DEFAULT_THEME.to_owned(),
            ui_theme: UiTheme::default(),
            shell: ShellSettings::default(),
            terminal: TerminalSettings::default(),
            notifications: NotificationSettings::default(),
            session: SessionSettings::default(),
            agents: AgentSettings::default(),
            browser: BrowserSettings::default(),
            workspace_items: WorkspaceItemSettings::default(),
            tab_items: TabItemSettings::default(),
            style: Style::default(),
            colors: Colors::default(),
            keybindings: BTreeMap::new(),
        }
    }
}

/// Settings with the theme resolved and key chords parsed. Actions stay as names: the app
/// decides what they mean.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// `[font]`.
    pub font: FontSettings,
    /// The `theme` name as written, for the settings dialog.
    pub theme_name: String,
    /// The theme `theme_name` resolved to.
    pub theme: Theme,
    /// `ui_theme`.
    pub ui_theme: UiTheme,
    /// `[shell]`.
    pub shell: ShellSettings,
    /// `[terminal]`.
    pub terminal: TerminalSettings,
    /// `[notifications]`.
    pub notifications: NotificationSettings,
    /// `[session]`.
    pub session: SessionSettings,
    /// `[agents]`.
    pub agents: AgentSettings,
    /// `[browser]`.
    pub browser: BrowserSettings,
    /// `[workspace_items]`.
    pub workspace_items: WorkspaceItemSettings,
    /// `[tab_items]`.
    pub tab_items: TabItemSettings,
    /// `[style.*]`, already range-checked.
    pub style: Style,
    /// `[colors.*]`, every name checked against [`crate::COLORS`].
    pub colors: Colors,
    /// User overrides, applied on top of the app's defaults. The action `"none"` unbinds a chord.
    pub keybindings: Vec<(Chord, String)>,
}

impl Default for Config {
    fn default() -> Self {
        let file = File::default();
        Self {
            font: file.font,
            // Can't fail: `DEFAULT_THEME` is built in and a test parses every built-in theme.
            theme: Theme::builtin(&file.theme).expect("default theme is built in"),
            theme_name: file.theme,
            ui_theme: file.ui_theme,
            shell: file.shell,
            terminal: file.terminal,
            notifications: file.notifications,
            session: file.session,
            agents: file.agents,
            browser: file.browser,
            workspace_items: file.workspace_items,
            tab_items: file.tab_items,
            style: file.style,
            colors: file.colors,
            keybindings: Vec::new(),
        }
    }
}

/// Why `config.toml` couldn't be used: the file's path followed by the parse or check error,
/// ready for the app's error banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError(pub String);

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Config {
    /// `<dir>/config.toml`.
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(FILE_NAME)
    }

    /// Loads `<dir>/config.toml`, writing the commented template first if there's no file yet.
    ///
    /// Creating the directory and writing the template are best effort: if the file still can't
    /// be read, this returns the defaults rather than an error, so an unwritable location
    /// doesn't stop the app. A file that reads but doesn't parse is a [`LoadError`].
    pub fn load_or_init(dir: &Path) -> Result<Self, LoadError> {
        let path = Self::path(dir);
        if !path.exists() {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(&path, TEMPLATE);
        }
        let source = match std::fs::read_to_string(&path) {
            Ok(source) => source,
            Err(_) => return Ok(Self::default()),
        };
        Self::parse(&source, dir).map_err(|e| LoadError(format!("{}: {e}", path.display())))
    }

    /// Parses and checks a config file's text. `dir` is the config directory, where user themes
    /// are looked up.
    ///
    /// Errors are readable messages: TOML errors with line, column and the valid key names, an
    /// out-of-range number with its key and limits, or an unknown color, theme or chord. Unknown
    /// actions are not errors here; the app skips them with a warning.
    pub fn parse(source: &str, dir: &Path) -> Result<Self, String> {
        let mut file: File = toml::from_str(source).map_err(|e| e.to_string())?;
        if file.ui_theme == UiTheme::Solid {
            // It parsed as a `File`, so it parses as a table.
            let table: toml::Table = source.parse().map_err(|e: toml::de::Error| e.to_string())?;
            file.style.apply_solid_defaults(&table);
        }
        if !(6.0..=72.0).contains(&file.font.size) {
            return Err(format!(
                "font.size must be between 6 and 72, got {}",
                file.font.size
            ));
        }
        if !(1.0..=2.5).contains(&file.font.line_height) {
            return Err(format!(
                "font.line_height must be between 1 and 2.5, got {}",
                file.font.line_height
            ));
        }
        if !(1..=20).contains(&file.terminal.scroll_lines) {
            return Err(format!(
                "terminal.scroll_lines must be between 1 and 20, got {}",
                file.terminal.scroll_lines
            ));
        }
        file.style.check()?;
        file.colors.check()?;
        let theme = Theme::resolve(&file.theme, dir)?;
        let keybindings = file
            .keybindings
            .into_iter()
            .map(|(chord, action)| {
                let chord: Chord = chord.parse().map_err(|e| format!("keybindings: {e}"))?;
                Ok((chord, action.trim().to_owned()))
            })
            .collect::<Result<_, String>>()?;
        Ok(Self {
            font: file.font,
            theme_name: file.theme,
            theme,
            ui_theme: file.ui_theme,
            shell: file.shell,
            terminal: file.terminal,
            notifications: file.notifications,
            session: file.session,
            agents: file.agents,
            browser: file.browser,
            workspace_items: file.workspace_items,
            tab_items: file.tab_items,
            style: file.style,
            colors: file.colors,
            keybindings,
        })
    }
}

/// `MACARONI_CONFIG_DIR` if set (handy for testing), else `%APPDATA%\macaroni` on Windows and
/// `$XDG_CONFIG_HOME/macaroni` or `~/.config/macaroni` elsewhere (including macOS, where terminal
/// users expect dotfile paths). Falls back to `./macaroni` when none of those variables is set.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MACARONI_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.unwrap_or_else(|| PathBuf::from(".")).join("macaroni")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chord::ChordKey;

    #[test]
    fn solid_has_its_own_radii_unless_set() {
        let dir = Path::new(".");
        let glass = Config::parse("", dir).expect("empty config parses");
        assert_eq!(glass.style.tabs.radius, 8.0);
        let solid = Config::parse("ui_theme = \"solid\"", dir).expect("solid parses");
        assert_eq!(solid.style.tabs.radius, 0.0);
        assert_eq!(solid.style.panes.radius, 0.0);
        assert!(!solid.workspace_items.search(solid.ui_theme));
        assert!(glass.workspace_items.search(glass.ui_theme));
        let chosen = Config::parse("ui_theme = \"solid\"\n[style.tabs]\nradius = 8", dir)
            .expect("solid with a radius parses");
        assert_eq!(chosen.style.tabs.radius, 8.0);
        assert_eq!(chosen.style.workspaces.radius, 10.0);
    }

    fn parse(source: &str) -> Result<Config, String> {
        Config::parse(source, Path::new("does-not-exist"))
    }

    #[test]
    fn template_parses_to_defaults() {
        assert_eq!(parse(TEMPLATE).unwrap(), Config::default());
    }

    #[test]
    fn template_lists_every_style_setting() {
        for setting in Style::settings() {
            let (table, field) = setting.key["style.".len()..].split_once('.').unwrap();
            let section = TEMPLATE
                .split(&format!("[style.{table}]\n"))
                .nth(1)
                .unwrap_or_else(|| panic!("no [style.{table}] in the template"));
            // A value with a solid default of its own notes it after the glass one.
            let default = format!("# {field} = {}", setting.default);
            assert!(
                section.contains(&format!("{default}\n"))
                    || section.contains(&format!("{default}  (solid: ")),
                "{} isn't in the template",
                setting.key
            );
            assert_eq!(Style::default().get(setting.key), Some(setting.default));
        }
        for color in crate::COLORS {
            assert!(
                TEMPLATE.contains(color.name),
                "{} isn't in the template",
                color.name
            );
        }
    }

    #[test]
    fn empty_file_is_all_defaults() {
        assert_eq!(parse("").unwrap(), Config::default());
    }

    #[test]
    fn full_example() {
        let config = parse(
            r##"
            theme = "macaroni-light"
            ui_theme = "solid"
            [font]
            family = "Cascadia Mono"
            size = 16
            ligatures = true
            [shell]
            program = "pwsh.exe"
            args = ["-NoLogo"]
            [terminal]
            scrollback = 500
            [notifications]
            os = false
            bell = true
            [workspace_items]
            icon = false
            detail = ""
            [tab_items]
            title = "{dir}"
            close_button = false
            [style.tabs]
            height = 40
            radius = 4.5
            [colors.solid]
            text = "#eeeeee"
            surface = "#30303080"
            insertion = "success"
            accent = "#ff0000"
            [keybindings]
            "ctrl+shift+t" = "none"
            "alt+t" = "new_tab"
            "##,
        )
        .unwrap();
        assert_eq!(config.font.family.as_deref(), Some("Cascadia Mono"));
        assert_eq!(config.font.size, 16.0);
        assert!(config.font.ligatures);
        assert_eq!(config.theme_name, "macaroni-light");
        assert_eq!(config.ui_theme, UiTheme::Solid);
        assert_eq!(config.shell.args, vec!["-NoLogo"]);
        assert_eq!(config.terminal.scrollback, 500);
        assert!(!config.notifications.os && config.notifications.bell);
        assert!(!config.workspace_items.icon && config.workspace_items.detail.is_empty());
        assert_eq!(config.workspace_items.title, "{name}");
        assert_eq!(config.tab_items.title, "{dir}");
        assert!(!config.tab_items.close_button && config.tab_items.status_icons);
        assert_eq!(config.style.tabs.height, 40.0);
        assert_eq!(config.style.tabs.radius, 4.5);
        assert_eq!(config.style.get("style.tabs.height"), Some(40.0));
        // Solid, so the panes' radius, gap and margin are solid's own defaults.
        assert_eq!(
            config.style.panes,
            crate::style::PaneStyle {
                radius: 0.0,
                gap: 0.0,
                margin: 0.0,
                ..crate::style::PaneStyle::default()
            }
        );
        assert_eq!(
            config.colors.solid.get("surface"),
            Some(&crate::ColorValue::Rgba(crate::Rgba(
                0x30, 0x30, 0x30, 0x80
            )))
        );
        assert_eq!(
            config.colors.solid.get("insertion"),
            Some(&crate::ColorValue::Theme("success".into()))
        );
        assert!(config.colors.glass.is_empty());
        assert_eq!(config.keybindings.len(), 2);
        assert!(
            config
                .keybindings
                .iter()
                .any(|(c, a)| c.key == ChordKey::Char('t') && c.mods.alt && a == "new_tab")
        );
    }

    #[test]
    fn mistakes_are_reported() {
        assert!(
            parse("[font]\nsize = 200")
                .unwrap_err()
                .contains("font.size")
        );
        assert!(
            parse("[font]\nfamliy = \"x\"")
                .unwrap_err()
                .contains("famliy")
        );
        assert!(
            parse("theme = \"nope\"")
                .unwrap_err()
                .contains("unknown theme")
        );
        assert!(
            parse("[keybindings]\n\"ctrl+banana\" = \"new_tab\"")
                .unwrap_err()
                .contains("banana")
        );
        assert!(parse("[font\nsize = 1").is_err());
        assert!(
            parse("[style.tabs]\nheight = 2")
                .unwrap_err()
                .contains("style.tabs.height")
        );
        assert!(
            parse("[style.tabs]\nhieght = 30")
                .unwrap_err()
                .contains("hieght")
        );
        assert!(
            parse("[colors.solid]\nnope = \"#000000\"")
                .unwrap_err()
                .contains("nope")
        );
        assert!(parse("[colors.glass]\ntext = \"white\"").is_err());
        assert!(parse("[colors.glass]\ntext = \"#nothex\"").is_err());
        assert!(parse("[colors.glass]\naccent = \"lines\"").is_err());
        assert!(
            parse("[terminal]\nscroll_lines = 0")
                .unwrap_err()
                .contains("scroll_lines")
        );
        assert!(
            parse("[font]\nline_height = 4")
                .unwrap_err()
                .contains("line_height")
        );
    }

    #[test]
    fn first_run_writes_the_template() {
        let dir = std::env::temp_dir().join(format!("macaroni-config-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = Config::load_or_init(&dir).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(
            std::fs::read_to_string(Config::path(&dir)).unwrap(),
            TEMPLATE
        );
        std::fs::write(Config::path(&dir), "[font]\nsize = 3").unwrap();
        let err = Config::load_or_init(&dir).unwrap_err();
        assert!(err.0.contains("config.toml") && err.0.contains("font.size"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

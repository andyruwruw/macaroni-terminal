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
use crate::plugins::Plugin;
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
    /// Installed families tried in order for a character `family` lacks, before the bundled
    /// symbols font and the system's fallback.
    pub fallback: Vec<String>,
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
            fallback: Vec::new(),
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

/// One `[[ssh]]` entry: a shell on another machine over macaroni's own SSH client, opened from
/// the new-tab menu. What it leaves out comes from `~/.ssh/config`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SshProfile {
    /// Shown in the new-tab menu and saved with the tabs it opens. Empty uses `host`.
    pub name: String,
    /// The destination: a host name or address, `user@host`, or a `Host` from `~/.ssh/config`.
    pub host: String,
    /// The login name, when `host` doesn't include one.
    pub user: Option<String>,
    /// The port.
    pub port: Option<u16>,
    /// A private key file, tried before the configured ones.
    pub identity_file: Option<String>,
    /// Hosts to connect through first, `[user@]host[:port]`, comma-separated.
    pub proxy_jump: Option<String>,
    /// A tmux session to attach to (made if it doesn't exist) in control mode, so its windows
    /// open as tabs, instead of a plain shell.
    pub tmux: Option<String>,
}

/// One `[[trigger]]`: a regex run over each line of terminal output, and what a match does.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Trigger {
    /// The regex (Rust `regex` syntax), matched against each line of output with its colors and
    /// other escape sequences taken out.
    pub pattern: String,
    /// Colors the matching text on screen: a workspace color name or `#rrggbb`.
    pub highlight: Option<String>,
    /// Marks the tab and sends a notification, with the matching line as its text.
    pub notify: bool,
    /// Sets the tab's badge to this until the tab is looked at. `$1` or `${name}` put in the
    /// match's groups.
    pub badge: Option<String>,
    /// The notification's title; empty uses the tab's.
    pub name: String,
}

impl SshProfile {
    /// The profile's name, or its host when it has none.
    pub fn label(&self) -> &str {
        if self.name.trim().is_empty() {
            &self.host
        } else {
            &self.name
        }
    }

    /// Where it connects, for showing: `user@host:port`, and the jump hosts after `via`.
    pub fn destination(&self) -> String {
        let mut out = match &self.user {
            Some(user) if !self.host.contains('@') => format!("{user}@{}", self.host),
            _ => self.host.clone(),
        };
        if let Some(port) = self.port {
            out.push_str(&format!(":{port}"));
        }
        if let Some(jump) = self.proxy_jump.as_deref().filter(|j| !j.trim().is_empty()) {
            out.push_str(&format!(" via {jump}"));
        }
        if let Some(session) = self.tmux_session() {
            out.push_str(&format!(", tmux {session}"));
        }
        out
    }

    /// The tmux session to attach, when one is named.
    pub fn tmux_session(&self) -> Option<&str> {
        self.tmux
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    /// What runs on the host instead of the login shell: tmux in control mode, attached to the
    /// named session (made if missing). The name is quoted for the remote shell.
    pub fn remote_command(&self) -> Option<String> {
        let session = self.tmux_session()?;
        Some(format!(
            "tmux -CC new-session -A -s '{}'",
            session.replace('\'', r"'\''")
        ))
    }
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
    /// Where terminals start in a workspace without a folder of its own; blank is the home
    /// folder, and a leading `~` stands for it.
    pub default_dir: String,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            scrollback: 10_000,
            cursor_style: CursorStyle::Block,
            scroll_lines: 3,
            copy_on_select: false,
            default_dir: String::new(),
        }
    }
}

/// `[notifications]`: when tabs ask for attention.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationSettings {
    /// Show an OS notification when a tab that isn't in view asks for attention.
    pub os: bool,
    /// What a tab that isn't in view asking for attention (an agent finishing) sounds like:
    /// `"off"`, `"system"` (the OS notification's own sound), one of macaroni's sounds
    /// (the app's `sounds::BUILT_IN`, such as `"chime"`, `"ding"` or `"chord"`) or the path of
    /// a sound file.
    pub sound: String,
    /// Treat the terminal bell as a request for attention.
    pub bell: bool,
    /// What the terminal bell sounds like: `"off"`, `"system"` (the system's alert sound), one
    /// of macaroni's sounds (as for `sound`) or the path of a sound file (`.wav` on Windows;
    /// anything `afplay` plays on macOS).
    pub bell_sound: String,
    /// Seconds a command the shell marked (OSC 133) has to run for its finish to ask for
    /// attention; 0 never does. [`Config::parse`] rejects more than a day.
    pub long_command: u32,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            os: true,
            sound: "chime".to_owned(),
            bell: false,
            bell_sound: "off".to_owned(),
            long_command: 30,
        }
    }
}

/// `[agents]`: how coding agents (Claude Code and Codex, through `macaroni-cli setup`) show up.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentSettings {
    /// Mark the tab and notify when an agent finishes responding.
    pub notify_finished: bool,
    /// Mark the tab and notify when an agent needs input or permission.
    pub notify_input: bool,
    /// Give a tab running an agent the sparkles icon.
    pub tab_icon: bool,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            notify_finished: true,
            notify_input: true,
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
    /// In restored tabs that were running Claude Code or Codex, run `claude --resume <session>`
    /// or `codex resume <session>`.
    pub resume_agents: bool,
    /// Save each terminal's text on quit and show it again, dimmed, in its restored tab.
    pub keep_scrollback: bool,
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            restore: true,
            resume_agents: true,
            keep_scrollback: true,
        }
    }
}

/// An accessibility option that can follow the system's own setting: `"system"`, `"on"` or
/// `"off"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Follow {
    /// Whatever the operating system's accessibility settings say, checked as they change.
    #[default]
    System,
    /// Always on.
    On,
    /// Always off.
    Off,
}

impl Follow {
    /// Whether the option is on, given whether the system's setting is.
    pub fn resolve(self, system: bool) -> bool {
        match self {
            Follow::System => system,
            Follow::On => true,
            Follow::Off => false,
        }
    }
}

/// `[accessibility]`: scaling, contrast, motion and screen readers.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AccessibilitySettings {
    /// Scales the whole window (interface and terminal text), on top of the display's own
    /// scaling. 0.5 to 3.
    pub ui_scale: f32,
    /// Turns off fades and other movement.
    pub reduce_motion: Follow,
    /// Stronger borders and text in the interface, and the high contrast terminal theme.
    pub high_contrast: Follow,
    /// The least contrast terminal text may have against its background, as a WCAG ratio from 1
    /// (off) to 21 (black on white). Text below it is lightened or darkened until it reaches it.
    pub minimum_contrast: f32,
    /// How thick the bar and underline cursors are, in pixels, from 1 to 8.
    pub cursor_thickness: f32,
    /// Have screen readers read out new output in the focused terminal.
    pub announce_output: bool,
}

impl Default for AccessibilitySettings {
    fn default() -> Self {
        Self {
            ui_scale: 1.0,
            reduce_motion: Follow::System,
            high_contrast: Follow::System,
            minimum_contrast: 1.0,
            cursor_thickness: 2.0,
            announce_output: true,
        }
    }
}

/// How the app's own UI (sidebar, tabs, pane cards) is drawn, apart from the terminal theme:
/// `ui_theme = "solid"` or `"glass"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiTheme {
    /// Flat layered greys.
    #[default]
    Solid,
    /// Translucent sheets with lit edges over a deep gradient.
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

/// `[browser] engine`: what renders browser tabs (`"system"` or `"chromium"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserEngine {
    /// The system's web view: Edge's WebView2 on Windows, Safari's WebKit on macOS.
    System,
    /// Chromium, embedded (CEF). Windows only; elsewhere the system engine is used.
    Chromium,
}

/// `[files] open`: where a file Ctrl+clicked (Cmd+clicked on macOS) in a terminal opens
/// (`"viewer"`, `"vscode"` or `"app"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenFiles {
    /// In a file tab, in a split next to the terminal.
    Viewer,
    /// In a VS Code tab (VS Code for the web, Windows only), in a split next to the terminal.
    Vscode,
    /// With the program in `[files] app`.
    App,
}

/// `[files]`: opening files named in a terminal's output.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileSettings {
    /// Where a Ctrl+clicked file opens.
    pub open: OpenFiles,
    /// The program `open = "app"` runs with the file: a path, or a command on `PATH`. Empty
    /// means VS Code when its `code` command is installed, else the system's text editor.
    pub app: String,
}

impl Default for FileSettings {
    fn default() -> Self {
        Self {
            open: OpenFiles::Viewer,
            app: String::new(),
        }
    }
}

/// `[browser]`: browser tabs.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrowserSettings {
    /// The page a new browser tab opens.
    pub home: String,
    /// Where links clicked in a terminal open.
    pub open_links: OpenLinks,
    /// What renders newly opened pages.
    pub engine: BrowserEngine,
}

impl Default for BrowserSettings {
    fn default() -> Self {
        Self {
            home: "https://www.google.com/".to_owned(),
            open_links: OpenLinks::Panel,
            engine: BrowserEngine::System,
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
    /// A large faded label in the top right of each of the workspace's panes; empty shows none.
    pub pane_badge: String,
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
            pane_badge: String::new(),
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
    /// Buttons at the right of each tab strip that open a terminal, a browser or VS Code.
    pub new_buttons: bool,
}

impl Default for TabItemSettings {
    fn default() -> Self {
        Self {
            icon: true,
            title: "{title}".to_owned(),
            badge: String::new(),
            status_icons: true,
            close_button: true,
            new_buttons: true,
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
    /// `[[ssh]]`.
    ssh: Vec<SshProfile>,
    /// `[[trigger]]`.
    trigger: Vec<Trigger>,
    /// `[terminal]`.
    terminal: TerminalSettings,
    /// `[notifications]`.
    notifications: NotificationSettings,
    /// `[session]`.
    session: SessionSettings,
    /// `[accessibility]`.
    accessibility: AccessibilitySettings,
    /// `[agents]`.
    agents: AgentSettings,
    /// `[browser]`.
    browser: BrowserSettings,
    /// `[files]`.
    files: FileSettings,
    /// `[workspace_items]`.
    workspace_items: WorkspaceItemSettings,
    /// `[tab_items]`.
    tab_items: TabItemSettings,
    /// `[plugins.<id>]`: overrides and settings for each plugin, by id.
    plugins: BTreeMap<String, toml::Table>,
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
            ssh: Vec::new(),
            trigger: Vec::new(),
            terminal: TerminalSettings::default(),
            notifications: NotificationSettings::default(),
            session: SessionSettings::default(),
            accessibility: AccessibilitySettings::default(),
            agents: AgentSettings::default(),
            browser: BrowserSettings::default(),
            files: FileSettings::default(),
            workspace_items: WorkspaceItemSettings::default(),
            tab_items: TabItemSettings::default(),
            plugins: BTreeMap::new(),
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
    /// `[[ssh]]`, in file order; each has a host.
    pub ssh: Vec<SshProfile>,
    /// `[[trigger]]`, in file order; each pattern compiles and each does something.
    pub triggers: Vec<Trigger>,
    /// `[terminal]`.
    pub terminal: TerminalSettings,
    /// `[notifications]`.
    pub notifications: NotificationSettings,
    /// `[session]`.
    pub session: SessionSettings,
    /// `[accessibility]`.
    pub accessibility: AccessibilitySettings,
    /// `[agents]`.
    pub agents: AgentSettings,
    /// `[browser]`.
    pub browser: BrowserSettings,
    /// `[files]`.
    pub files: FileSettings,
    /// `[workspace_items]`.
    pub workspace_items: WorkspaceItemSettings,
    /// `[tab_items]`.
    pub tab_items: TabItemSettings,
    /// The plugins: built in, then from the plugins folder, with `[plugins.<id>]` applied.
    pub plugins: Vec<Plugin>,
    /// What was wrong with plugin folders or `[plugins]` (those parts are left out); the rest of
    /// the config still applies.
    pub plugin_problems: Vec<String>,
    /// `[style.*]`, already range-checked.
    pub style: Style,
    /// `[colors.*]`, every name checked against [`crate::COLORS`].
    pub colors: Colors,
    /// User overrides, applied on top of the app's defaults. The action `"none"` unbinds a chord.
    pub keybindings: Vec<(Chord, String)>,
}

impl Default for Config {
    fn default() -> Self {
        let mut file = File::default();
        // Style defaults are glass's; the default interface is solid, so it gets solid's values
        // just as a file that sets nothing does in `Config::parse`.
        if file.ui_theme == UiTheme::Solid {
            file.style.apply_solid_defaults(&toml::Table::new());
        }
        Self {
            font: file.font,
            // Can't fail: `DEFAULT_THEME` is built in and a test parses every built-in theme.
            theme: Theme::builtin(&file.theme).expect("default theme is built in"),
            theme_name: file.theme,
            ui_theme: file.ui_theme,
            shell: file.shell,
            ssh: file.ssh,
            triggers: file.trigger,
            terminal: file.terminal,
            notifications: file.notifications,
            session: file.session,
            accessibility: file.accessibility,
            agents: file.agents,
            browser: file.browser,
            files: file.files,
            workspace_items: file.workspace_items,
            tab_items: file.tab_items,
            plugins: crate::plugins::built_in(),
            plugin_problems: Vec::new(),
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
    /// and plugins are looked up.
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
        if file.notifications.long_command > 86_400 {
            return Err(format!(
                "notifications.long_command must be at most 86400 seconds, got {}",
                file.notifications.long_command
            ));
        }
        if !(1..=20).contains(&file.terminal.scroll_lines) {
            return Err(format!(
                "terminal.scroll_lines must be between 1 and 20, got {}",
                file.terminal.scroll_lines
            ));
        }
        let a = &file.accessibility;
        if !(0.5..=3.0).contains(&a.ui_scale) {
            return Err(format!(
                "accessibility.ui_scale must be between 0.5 and 3, got {}",
                a.ui_scale
            ));
        }
        if !(1.0..=21.0).contains(&a.minimum_contrast) {
            return Err(format!(
                "accessibility.minimum_contrast must be between 1 and 21, got {}",
                a.minimum_contrast
            ));
        }
        if !(1.0..=8.0).contains(&a.cursor_thickness) {
            return Err(format!(
                "accessibility.cursor_thickness must be between 1 and 8, got {}",
                a.cursor_thickness
            ));
        }
        if let Some(n) = file.ssh.iter().position(|p| p.host.trim().is_empty()) {
            return Err(format!("ssh entry {} has no host", n + 1));
        }
        for (n, trigger) in file.trigger.iter().enumerate() {
            if trigger.pattern.is_empty() {
                return Err(format!("trigger {} has no pattern", n + 1));
            }
            if let Err(e) = regex::Regex::new(&trigger.pattern) {
                return Err(format!("trigger {}: {e}", n + 1));
            }
            if trigger.highlight.is_none() && !trigger.notify && trigger.badge.is_none() {
                return Err(format!(
                    "trigger {} has no highlight, notify or badge",
                    n + 1
                ));
            }
        }
        file.style.check()?;
        file.colors.check()?;
        let theme = Theme::resolve(&file.theme, dir)?;
        let (plugins, plugin_problems) = crate::plugins::load(dir, &file.plugins);
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
            ssh: file.ssh,
            triggers: file.trigger,
            terminal: file.terminal,
            notifications: file.notifications,
            session: file.session,
            accessibility: file.accessibility,
            agents: file.agents,
            browser: file.browser,
            files: file.files,
            workspace_items: file.workspace_items,
            tab_items: file.tab_items,
            plugins,
            plugin_problems,
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
        let glass = Config::parse("ui_theme = \"glass\"", dir).expect("glass parses");
        assert_eq!(glass.style.tabs.radius, 8.0);
        let solid = Config::parse("ui_theme = \"solid\"", dir).expect("solid parses");
        assert_eq!(solid.style.tabs.radius, 0.0);
        // Solid is the default, with or without a file.
        let empty = Config::parse("", dir).expect("empty config parses");
        assert_eq!(empty.ui_theme, UiTheme::Solid);
        assert_eq!(empty.style.tabs.radius, 0.0);
        assert_eq!(Config::default().style, empty.style);
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
            // The template shows the default interface's (solid's) value; where glass differs,
            // glass's follows it.
            let solid = crate::SOLID_DEFAULTS
                .iter()
                .find(|(key, _)| *key == setting.key)
                .map(|(_, v)| *v);
            let shown = match solid {
                Some(solid) => format!("# {field} = {solid}  (glass: {})\n", setting.default),
                None => format!("# {field} = {}\n", setting.default),
            };
            assert!(
                section.contains(&shown),
                "{} isn't in the template as {shown:?}",
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
    fn triggers_are_checked() {
        let config =
            parse("[[trigger]]\npattern = '\\berror\\b'\nnotify = true\nbadge = \"$0\"").unwrap();
        assert_eq!(config.triggers.len(), 1);
        assert_eq!(config.triggers[0].pattern, "\\berror\\b");
        assert!(
            parse("[[trigger]]\npattern = \"(\"\nnotify = true")
                .unwrap_err()
                .starts_with("trigger 1:")
        );
        assert!(
            parse("[[trigger]]\npattern = \"x\"")
                .unwrap_err()
                .contains("no highlight")
        );
        assert!(
            parse("[[trigger]]\nnotify = true")
                .unwrap_err()
                .contains("no pattern")
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

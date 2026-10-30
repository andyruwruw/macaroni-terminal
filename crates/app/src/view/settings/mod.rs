//! The settings dialog: a modal over the app with pages down the left, a search field over
//! them, and each page's settings in collapsible cards, with Quick Access to jump between them. Every change
//! is written straight into `config.toml` (`macaroni_config::write_setting`), and the config
//! watcher applies it like a hand edit, so the file stays the one source of truth.
//!
//! This module holds the dialog's state ([`Settings`], kept in `App.settings` while it's open),
//! its messages ([`SettingsMessage`], handled by [`App::settings_update`]) and the row model
//! ([`Setting`], [`Control`]). `list` builds every row from `App.config` on each draw, so the
//! rows always show what's in effect; `dialog` draws them ([`App::settings_view`], the top
//! `stack` layer in `App::view`), and `preview` the live previews at the top of some cards.

mod color_picker;
mod dialog;
mod list;
mod preview;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use iced::advanced::widget::operation::{Operation, Outcome};
use iced::widget::Id;
use iced::widget::operation::AbsoluteOffset;
use iced::{Color, Point, Rectangle, Task, Vector};
use macaroni_config::{ColorValue, NumberSetting, Rgba, UiTheme, Value};
use macaroni_term::shell::ShellOption;

use crate::icons::Icon;
use crate::theme::{BORDER_FOLLOWERS, Chrome, from_rgba};
use crate::{App, Message, home_dir};
use color_picker::{ColorPicker, PickerStart};
use list::{number, number_setting};

/// The widget id of the dialog's search field, focused when the dialog opens.
pub const SEARCH_INPUT: &str = "settings-search";

/// The widget id of the scrolling column of settings, which Quick Access scrolls.
const BODY_SCROLL: &str = "settings-body";

/// A page of the dialog, listed down its left side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Section {
    /// The session and the browser.
    #[default]
    General,
    /// Everything visual: the interface and terminal themes, the font, every color and size,
    /// and the effects.
    Appearance,
    /// "Quick Theme": the handful of solid colors that set the overall look, on one card under
    /// one preview.
    Quick,
    /// "Solid Theme": the colors, borders, corners and gaps of the solid interface's window,
    /// workspaces, tabs and `+` buttons, together on one page to try out.
    Theme,
    /// The shell, scrollback, cursor and mouse.
    Terminal,
    /// OS notifications and the terminal bell.
    Notifications,
    /// Claude Code's hooks and `[agents]`.
    Agents,
    /// "Sidebar & Tabs": what workspaces and tabs show (`[workspace_items]`, `[tab_items]`).
    Items,
    /// "Keyboard Shortcuts": the bindings in effect, read-only.
    Shortcuts,
}

impl Section {
    /// Every section, in the order the dialog lists them.
    const ALL: [Self; 9] = [
        Self::General,
        Self::Appearance,
        Self::Quick,
        Self::Theme,
        Self::Terminal,
        Self::Notifications,
        Self::Agents,
        Self::Items,
        Self::Shortcuts,
    ];

    /// Whether the dialog lists the section: Quick Theme and Solid Theme are tools for working
    /// on the solid look, so only debug builds (`cargo xtask dev`) have them; release builds
    /// set the look from Appearance.
    fn shown_in_build(self) -> bool {
        cfg!(debug_assertions) || !matches!(self, Self::Quick | Self::Theme)
    }

    /// The sections the dialog lists, in order ([`Section::shown_in_build`]).
    fn shown() -> impl Iterator<Item = Self> {
        Self::ALL.into_iter().filter(|s| s.shown_in_build())
    }

    /// The section's name, in the dialog's list and over its rows.
    fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Quick => "Quick Theme",
            Self::Theme => "Solid Theme",
            Self::Terminal => "Terminal",
            Self::Notifications => "Notifications",
            Self::Agents => "Agents",
            Self::Items => "Sidebar & Tabs",
            Self::Shortcuts => "Keyboard Shortcuts",
        }
    }

    /// The icon beside the section's name.
    fn icon(self) -> Icon {
        match self {
            Self::General => Icon::Sliders,
            Self::Appearance => Icon::Palette,
            Self::Quick => Icon::Zap,
            Self::Theme => Icon::SwatchBook,
            Self::Terminal => Icon::Terminal,
            Self::Notifications => Icon::Bell,
            Self::Agents => Icon::Bot,
            Self::Items => Icon::PanelLeft,
            Self::Shortcuts => Icon::Keyboard,
        }
    }
}

/// The open dialog. Text fields keep what's typed in `drafts` until Enter writes it.
#[derive(Debug, Default)]
pub struct Settings {
    /// The section shown while there's no search.
    section: Section,
    /// The search field's text; non-empty, it searches every section instead.
    query: String,
    /// Text typed into fields, by dotted config key, until Enter writes (or drops) it.
    drafts: BTreeMap<&'static str, String>,
    /// Pickers switched to Custom (templates, the shell, the bell sound, colors), which show
    /// their text field even while the value matches one of the options.
    custom: BTreeSet<&'static str>,
    /// Cards folded shut, by section and group.
    collapsed: BTreeSet<(Section, &'static str)>,
    /// Whether the Quick Access list of the page's cards is open.
    quick: bool,
    /// The color picker popup, while one is open.
    picker: Option<ColorPicker>,
    /// Whether Claude Code's settings have macaroni's hooks, read when the dialog opens and after
    /// installing or removing them; `None` when there's no Claude Code settings file to read.
    hooks: Option<bool>,
    /// What the last install or removal of the hooks reported, shown under its row.
    hooks_result: Option<String>,
    /// The shells found on this machine, looked up when the dialog opens (it searches `PATH`).
    shells: Vec<ShellOption>,
    /// What new terminals run while `shell.program` isn't set, by name.
    default_shell: String,
}

/// What the dialog's controls send, wrapped in [`Message::Settings`].
#[derive(Debug, Clone)]
pub enum SettingsMessage {
    /// Show this section, clearing any search.
    Section(Section),
    /// The search field changed.
    Query(String),
    /// Write a setting, or (`None`) remove it, back to its default.
    Set(&'static str, Option<Value>),
    /// Write several settings at once (a shell's program and arguments).
    SetMany(Vec<(&'static str, Option<Value>)>),
    /// Typing in a field: keep it as the key's draft.
    Draft(&'static str, String),
    /// Enter in a text field: write its draft (empty means the default).
    Submit(&'static str),
    /// Enter in a list's field: write its draft split at spaces (empty means the default).
    SubmitList(&'static str),
    /// Show a picker's text field.
    Custom(&'static str),
    /// Enter in a template's field: write its draft as is (empty hides what it shows).
    SubmitTemplate(&'static str),
    /// Enter in a number's field: write it, kept within its limits (empty means the default).
    /// Something that isn't a number stays in the field to fix.
    SubmitNumber(&'static str),
    /// Fold or unfold a card.
    Collapse(Section, &'static str),
    /// Quick Access: scroll to the shown page's `n`th card, unfolding it, and close the list.
    Jump(usize, Section, &'static str),
    /// Open or close Quick Access.
    Quick,
    /// Ctrl+F (Cmd+F): put the keyboard in the search field, its text selected.
    FocusSearch,
    /// Install (`true`) or remove macaroni's Claude Code hooks with `macaroni-cli setup claude`.
    Hooks(bool),
    /// A color row's button was clicked at this window position: open the menu for its key,
    /// starting from what the row shows.
    Pick(&'static str, Point, PickerStart),
    /// A Default, theme color or palette cell was picked in the menu (`None` for Default),
    /// drawn as this color: write it and keep the menu open.
    PickerChoose(Option<Value>, Color),
    /// The picker's square was dragged to this saturation and value.
    PickerSaturationValue(f32, f32),
    /// The picker's hue bar was dragged to this hue.
    PickerHue(f32),
    /// The picker's alpha bar was dragged to this alpha.
    PickerAlpha(f32),
    /// A drag in the picker ended: write the picked color.
    PickerCommit,
    /// Typing in the picker's hex field; a whole hex applies and writes at once.
    PickerHex(String),
    /// Close the picker.
    PickerClose,
}

/// How a row's setting is shown and changed. Each holds the dotted config key it writes and
/// its value in effect.
enum Control {
    /// A switch for a boolean, and whether it's on.
    Toggle(&'static str, bool),
    /// Options as labels and values, and which one is current (`None` when the value in effect
    /// isn't one of them). Up to 3 draw as segments, more as a drop-down.
    Choice(&'static str, Vec<(String, Value)>, Option<usize>),
    /// A [`Control::Choice`] that's always a drop-down, for picking from presets.
    Dropdown(&'static str, Vec<(String, Value)>, Option<usize>),
    /// A text field that writes on Enter; emptied, it removes the key.
    Text {
        /// The dotted config key.
        key: &'static str,
        /// The value in effect, shown until something is typed.
        value: String,
        /// Shown while the field is empty.
        placeholder: &'static str,
    },
    /// A list of words in one field (`shell.args`), split at spaces on Enter.
    List {
        /// The dotted config key.
        key: &'static str,
        /// The value in effect.
        value: Vec<String>,
        /// Shown while the field is empty.
        placeholder: &'static str,
    },
    /// A read-only binding, its chord as shown (e.g. "Ctrl+Shift+T").
    Shortcut(String),
    /// Presets as labels and values, or a custom value typed in: `[workspace_items]` and
    /// `[tab_items]` templates and the bell sound.
    Template {
        /// The dotted config key.
        key: &'static str,
        /// The value in effect.
        value: String,
        /// Each preset's label and value.
        presets: Vec<(String, String)>,
        /// The custom field's placeholder.
        placeholder: &'static str,
    },
    /// `[shell] program` and `args`: a shell found on this machine, the default, or one typed in.
    Shell {
        /// `shell.program` in effect; empty for the default.
        program: String,
        /// `shell.args` in effect.
        args: Vec<String>,
        /// The shells found on this machine.
        options: Vec<ShellOption>,
        /// What the default runs, named in its option.
        default: String,
    },
    /// A number: typed, or stepped, with its unit.
    Number {
        /// Its key, default, limits, step and unit.
        setting: &'static NumberSetting,
        /// The value in effect.
        value: f32,
    },
    /// A `[colors.*]` color: what's drawn now, and what it falls back to.
    Color {
        /// The dotted key, under the interface in use (`colors.glass.<name>`).
        key: &'static str,
        /// The color in effect, from the live [`crate::theme::Chrome`].
        value: Color,
        /// What it is while unset: the interface's own color, or its theme color's.
        default: Color,
        /// The theme color it follows while unset, by label; `None` for a color that keeps its
        /// own, and for the theme colors themselves (which don't offer the theme options).
        link: Option<&'static str>,
        /// Whether it can be set to a theme color: every element color, not the theme colors.
        themed: bool,
        /// What the config sets it to: `None` while unset.
        set: Option<macaroni_config::ColorValue>,
    },
    /// A button, with its label and what it sends.
    Button(String, Message),
}

/// The template drop-down's entry for typing one's own.
const CUSTOM: &str = "Custom\u{2026}";

/// One row of the dialog.
struct Setting {
    /// The section it's listed under.
    section: Section,
    /// The card it's grouped into within its section.
    group: &'static str,
    /// Its name, on the left of the row.
    label: String,
    /// The faint line under the label; empty for none.
    description: String,
    /// What's on the right of the row.
    control: Control,
    /// A copy of a row listed elsewhere too (colors repeated where they're used), left out of
    /// search results so each setting is found once.
    repeat: bool,
}

impl Setting {
    /// A row from its parts.
    fn new(
        section: Section,
        group: &'static str,
        label: &str,
        description: &str,
        control: Control,
    ) -> Self {
        Self {
            section,
            group,
            label: label.to_owned(),
            description: description.to_owned(),
            control,
            repeat: false,
        }
    }

    /// Whether every word of the search `query` (already trimmed and lowercased) is somewhere
    /// in the row's section, group, label or description, so "tab padding" finds the Tabs
    /// card's Padding row though neither name has both words. Repeats never match.
    fn matches(&self, query: &str) -> bool {
        if self.repeat {
            return false;
        }
        let haystack = self.search_text();
        query.split_whitespace().all(|word| haystack.contains(word))
    }

    /// What search looks in: the section, group, label and description, lowercased.
    fn search_text(&self) -> String {
        format!(
            "{} {} {} {}",
            self.section.title(),
            self.group,
            self.label,
            self.description
        )
        .to_lowercase()
    }
}

/// The widget id of the shown page's `n`th card, for Quick Access.
fn group_id(n: usize) -> Id {
    Id::from(format!("settings-group-{n}"))
}

/// Finds how far down the settings column the card with id `target` starts, for
/// [`SettingsMessage::Jump`]. Children of a scrollable are laid out unscrolled, so the card's
/// top minus the column's top is the offset to scroll to.
struct CardOffset {
    /// The card's id.
    target: Id,
    /// The top of the scrollable's content, once passed.
    content_top: Option<f32>,
    /// The card's top, once found.
    card_top: Option<f32>,
}

impl Operation<f32> for CardOffset {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<f32>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        _bounds: Rectangle,
        content_bounds: Rectangle,
        _translation: Vector,
        _state: &mut dyn iced::advanced::widget::operation::Scrollable,
    ) {
        if id == Some(&Id::new(BODY_SCROLL)) {
            self.content_top = Some(content_bounds.y);
        }
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id == Some(&self.target) {
            self.card_top = Some(bounds.y);
        }
    }

    fn finish(&self) -> Outcome<f32> {
        match (self.content_top, self.card_top) {
            (Some(content), Some(card)) => Outcome::Some((card - content).max(0.0)),
            _ => Outcome::None,
        }
    }
}

/// The name of what new terminals run while no shell is set: Git Bash or PowerShell on
/// Windows, the login shell elsewhere.
fn default_shell() -> String {
    match macaroni_term::shell::windows_default() {
        Some(shell) => shell.name,
        None => std::env::var("SHELL")
            .ok()
            .and_then(|s| s.rsplit('/').next().map(str::to_owned))
            .unwrap_or_default(),
    }
}

/// `~/.claude/settings.json`, where `macaroni-cli setup claude` installs its hooks.
fn claude_settings() -> Option<PathBuf> {
    Some(home_dir()?.join(".claude").join("settings.json"))
}

/// Whether Claude Code's settings run `macaroni-cli hook`; `None` when there are no settings.
fn hooks_installed() -> Option<bool> {
    let text = std::fs::read_to_string(claude_settings()?).ok()?;
    Some(text.contains("macaroni-cli") && text.contains(" hook"))
}

/// Runs `macaroni-cli setup claude --yes` (with `--remove` unless `install`) from next to this
/// executable, and returns what it printed, or why it couldn't run.
fn run_setup(install: bool) -> String {
    let cli = std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name(format!("macaroni-cli{}", std::env::consts::EXE_SUFFIX)));
    let Some(cli) = cli.filter(|p| p.is_file()) else {
        return "macaroni-cli wasn't found next to macaroni.".to_owned();
    };
    let mut command = std::process::Command::new(cli);
    command.args(["setup", "claude", "--yes"]);
    if !install {
        command.arg("--remove");
    }
    match command.output() {
        Ok(out) => {
            let text = if out.status.success() {
                String::from_utf8_lossy(&out.stdout).into_owned()
            } else {
                String::from_utf8_lossy(&out.stderr).into_owned()
            };
            // The last line says what happened; the rest is the plan it followed.
            text.lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("Done.")
                .trim()
                .to_owned()
        }
        Err(err) => format!("macaroni-cli didn't run: {err}"),
    }
}

/// `color` as a `[colors.*]` value of its own.
fn rgba_value(color: Color) -> ColorValue {
    let [r, g, b, a] = color.into_rgba8();
    ColorValue::Rgba(Rgba(r, g, b, a))
}

/// `text` typed into `setting`'s field, as shown (a percentage as 55), its unit optional; `None`
/// unless it's a finite number.
fn parse_number(setting: &NumberSetting, text: &str) -> Option<f32> {
    let text = text.trim().trim_end_matches(setting.unit.suffix()).trim();
    text.parse::<f32>().ok().filter(|n| n.is_finite())
}

/// `text` as a color: `#rrggbb` or `#rrggbbaa`, the `#` optional; `None` until it's whole.
fn parse_hex(text: &str) -> Option<Rgba> {
    let text = text.trim();
    Rgba::parse(&format!("#{}", text.strip_prefix('#').unwrap_or(text)))
}

impl Settings {
    /// Closes the color picker; whether one was open.
    pub fn close_picker(&mut self) -> bool {
        self.picker.take().is_some()
    }
}

impl App {
    /// Opens the dialog fresh (General, no search, no drafts), focusing its search field.
    pub fn open_settings(&mut self) -> Task<Message> {
        self.settings = Some(Settings {
            hooks: hooks_installed(),
            shells: macaroni_term::shell::available(),
            default_shell: default_shell(),
            ..Settings::default()
        });
        iced::widget::operation::focus(SEARCH_INPUT)
    }

    /// Applies a dialog message. Writes go straight to `config.toml` (see
    /// [`App::write_setting`]); the config watcher then reloads it, and the next draw shows the
    /// new value. Ignored while the dialog is closed.
    pub fn settings_update(&mut self, message: SettingsMessage) -> Task<Message> {
        let Some(settings) = &mut self.settings else {
            return Task::none();
        };
        match message {
            SettingsMessage::Section(section) => {
                settings.section = section;
                settings.query.clear();
                settings.quick = false;
                return iced::widget::operation::scroll_to(
                    BODY_SCROLL,
                    AbsoluteOffset { x: 0.0, y: 0.0 },
                );
            }
            SettingsMessage::Query(query) => {
                settings.query = query;
                settings.quick = false;
            }
            SettingsMessage::Quick => settings.quick = !settings.quick,
            SettingsMessage::FocusSearch => {
                return iced::widget::operation::focus(SEARCH_INPUT)
                    .chain(iced::widget::operation::select_all(SEARCH_INPUT));
            }
            SettingsMessage::Draft(key, value) => {
                // A number applies as it's typed, not only on Enter, once it's whole and in
                // range (Enter clamps one that isn't). The draft stays, so the field keeps what
                // was typed.
                let typed = number_setting(key).and_then(|setting| {
                    let raw = parse_number(setting, &value)? / setting.unit.scale();
                    (setting.min..=setting.max).contains(&raw).then_some(raw)
                });
                settings.drafts.insert(key, value);
                if let Some(raw) = typed {
                    self.write_setting(key, Some(number(raw)));
                }
            }
            SettingsMessage::Submit(key) => {
                if let Some(draft) = settings.drafts.remove(key) {
                    let draft = draft.trim();
                    let value = (!draft.is_empty()).then(|| Value::Str(draft.to_owned()));
                    self.write_setting(key, value);
                }
            }
            SettingsMessage::SubmitList(key) => {
                if let Some(draft) = settings.drafts.remove(key) {
                    let words: Vec<String> = draft.split_whitespace().map(str::to_owned).collect();
                    let value = (!words.is_empty()).then_some(Value::List(words));
                    self.write_setting(key, value);
                }
            }
            SettingsMessage::Custom(key) => {
                settings.custom.insert(key);
            }
            SettingsMessage::SubmitTemplate(key) => {
                if let Some(draft) = settings.drafts.remove(key) {
                    self.write_setting(key, Some(Value::Str(draft.trim().to_owned())));
                }
            }
            SettingsMessage::SubmitNumber(key) => {
                // Read, not taken: like a color, a draft that isn't a number stays to fix.
                let (Some(draft), Some(setting)) = (settings.drafts.get(key), number_setting(key))
                else {
                    return Task::none();
                };
                if draft.trim().is_empty() {
                    settings.drafts.remove(key);
                    self.write_setting(key, None);
                } else if let Some(shown) = parse_number(setting, draft) {
                    settings.drafts.remove(key);
                    // Typed as shown (a percentage as 55), stored raw (0.55). The config would
                    // reject an out-of-range value and keep the old one, so clamp it here.
                    let value = (shown / setting.unit.scale()).clamp(setting.min, setting.max);
                    self.write_setting(key, Some(number(value)));
                }
            }
            SettingsMessage::Set(key, value) => {
                // Picking an option leaves Custom.
                settings.custom.remove(key);
                settings.drafts.remove(key);
                self.write_setting(key, value);
            }
            SettingsMessage::SetMany(writes) => {
                for (key, _) in &writes {
                    settings.custom.remove(key);
                    settings.drafts.remove(key);
                }
                for (key, value) in writes {
                    self.write_setting(key, value);
                }
            }
            SettingsMessage::Collapse(section, group) => {
                if !settings.collapsed.remove(&(section, group)) {
                    settings.collapsed.insert((section, group));
                }
            }
            SettingsMessage::Jump(n, section, group) => {
                // The list floats over the page, so closing it moves no card.
                settings.quick = false;
                settings.collapsed.remove(&(section, group));
                let find = CardOffset {
                    target: group_id(n),
                    content_top: None,
                    card_top: None,
                };
                return iced::advanced::widget::operate(find).then(|y| {
                    iced::widget::operation::scroll_to(BODY_SCROLL, AbsoluteOffset { x: 0.0, y })
                });
            }
            SettingsMessage::Pick(key, at, start) => {
                settings.picker = Some(ColorPicker::new(key, at, start));
                return iced::widget::operation::focus(color_picker::HEX_INPUT)
                    .chain(iced::widget::operation::select_all(color_picker::HEX_INPUT));
            }
            SettingsMessage::PickerChoose(value, color) => {
                if let Some(picker) = &mut settings.picker {
                    picker.set_color(color);
                    // The menu only ever writes a hex or a theme color's name.
                    let set = match &value {
                        Some(Value::Str(s)) => Some(match Rgba::parse(s) {
                            Some(rgba) => ColorValue::Rgba(rgba),
                            None => ColorValue::Theme(s.clone()),
                        }),
                        _ => None,
                    };
                    picker.set = set.clone();
                    let key = picker.key;
                    self.preview_override(key, set);
                    self.write_setting(key, value);
                }
            }
            SettingsMessage::PickerSaturationValue(saturation, value) => {
                if let Some(picker) = &mut settings.picker {
                    picker.set_saturation_value(saturation, value);
                    let (key, color) = (picker.key, picker.color());
                    picker.set = Some(rgba_value(color));
                    self.preview_color(key, color);
                }
            }
            SettingsMessage::PickerHue(hue) => {
                if let Some(picker) = &mut settings.picker {
                    picker.set_hue(hue);
                    let (key, color) = (picker.key, picker.color());
                    picker.set = Some(rgba_value(color));
                    self.preview_color(key, color);
                }
            }
            SettingsMessage::PickerAlpha(alpha) => {
                if let Some(picker) = &mut settings.picker {
                    picker.set_alpha(alpha);
                    let (key, color) = (picker.key, picker.color());
                    picker.set = Some(rgba_value(color));
                    self.preview_color(key, color);
                }
            }
            SettingsMessage::PickerCommit => {
                if let Some(picker) = &settings.picker {
                    let (key, hex) = (picker.key, picker.hex.clone());
                    settings.drafts.remove(key);
                    self.write_setting(key, Some(Value::Str(hex)));
                }
            }
            SettingsMessage::PickerHex(text) => {
                if let Some(picker) = &mut settings.picker {
                    let rgba = parse_hex(&text);
                    if let Some(rgba) = rgba {
                        picker.set_color(from_rgba(rgba));
                    }
                    // The field keeps what's typed; only the color follows it.
                    picker.hex = text;
                    let key = picker.key;
                    if let Some(rgba) = rgba {
                        picker.set = Some(ColorValue::Rgba(rgba));
                        settings.drafts.remove(key);
                        self.preview_color(key, from_rgba(rgba));
                        self.write_setting(key, Some(Value::Str(rgba.to_string())));
                    }
                }
            }
            SettingsMessage::PickerClose => settings.picker = None,
            SettingsMessage::Hooks(install) => {
                // Quick (it edits one JSON file), so it runs here rather than on a thread.
                settings.hooks_result = Some(run_setup(install));
                settings.hooks = hooks_installed();
            }
        }
        Task::none()
    }

    /// Shows `color` as the `[colors.*]` setting `key` (`colors.<interface>.<name>`) right away,
    /// in the config in memory, ahead of the file write and reload that make it stick, so the
    /// app and the dialog's previews follow a drag in the color picker as it happens.
    fn preview_color(&mut self, key: &str, color: Color) {
        self.preview_override(key, Some(rgba_value(color)));
    }

    /// Like [`App::preview_color`], for any value `key` can have: `None` shows its default.
    fn preview_override(&mut self, key: &str, value: Option<ColorValue>) {
        let Some((ui, name)) = key
            .strip_prefix("colors.")
            .and_then(|rest| rest.split_once('.'))
        else {
            return;
        };
        let overrides = match ui {
            "solid" => &mut self.config.colors.solid,
            "glass" => &mut self.config.colors.glass,
            _ => return,
        };
        match value {
            Some(value) => overrides.insert(name.to_owned(), value),
            None => overrides.remove(name),
        };
        self.chrome = Chrome::new(&self.palette, &self.config);
        self.iced_theme = self.chrome.iced_theme(&self.config.theme_name);
    }

    /// Writes one dotted `key` into `config.toml` (`None` removes it), keeping the rest of the
    /// file as it is. A failure (say, the file can't be written) joins the config error banner.
    /// Setting the solid border color also clears the border colors set on their own
    /// ([`BORDER_FOLLOWERS`]), so every line between parts follows it.
    fn write_setting(&mut self, key: &str, value: Option<Value>) {
        if key == "colors.solid.border" && value.is_some() {
            let set: Vec<_> = BORDER_FOLLOWERS
                .iter()
                .filter(|name| {
                    self.config
                        .colors
                        .for_ui(UiTheme::Solid)
                        .contains_key(**name)
                })
                .map(|name| format!("colors.solid.{name}"))
                .collect();
            for follower in set {
                self.write_one(&follower, None);
            }
        }
        self.write_one(key, value);
    }

    /// Writes one dotted `key` into `config.toml`, as [`App::write_setting`] does.
    fn write_one(&mut self, key: &str, value: Option<Value>) {
        if let Err(error) = macaroni_config::write_setting(&self.config_dir, key, value) {
            self.config_errors.push(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_words_across_fields() {
        let row = Setting::new(
            Section::Appearance,
            "Tabs",
            "Padding",
            "Inside each tab.",
            Control::Toggle("style.tabs.padding", true),
        );
        assert!(row.matches("tab padding"));
        assert!(row.matches("padding tabs"));
        assert!(row.matches("appearance inside"));
        assert!(!row.matches("sidebar padding"));
    }
}

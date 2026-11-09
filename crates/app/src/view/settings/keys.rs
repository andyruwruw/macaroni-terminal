//! The settings dialog from the keyboard. Keys nothing captured (so not typing in a field) move
//! a ring along the rows of the page (or the search's results) and change the ringed one:
//!
//! - Up and Down move between rows, skipping folded cards and read-only shortcuts, and scroll
//!   the row into view. Ctrl+Page Up and Ctrl+Page Down (Cmd on macOS) go to the page before
//!   or after.
//! - Space or Enter flips a switch, picks a choice's next option, presses a button, plays a
//!   sound, or puts the keyboard in a field (Enter writes it, as when clicked). Left and Right
//!   pick a choice's or sound's option before or after, step a number, or turn a switch off or
//!   on. Delete puts the row
//!   back to its default.

use iced::advanced::widget::operation::{Operation, Outcome, Scrollable};
use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use iced::widget::Id;
use iced::widget::operation::AbsoluteOffset;
use iced::{Rectangle, Task, Vector};
use macaroni_config::Value;

use super::list::number;
use super::{BODY_SCROLL, Control, Section, Setting, SettingsMessage};
use crate::{App, Message};

/// Room kept above and below a row scrolled into view.
const SCROLL_MARGIN: f32 = 24.0;

/// A row, as the keyboard remembers it: its page, card and label (rows are rebuilt on every
/// draw, so there's nothing longer-lived to point at).
pub type RowKey = (Section, &'static str, String);

/// The key of `setting`'s row.
pub(super) fn row_key(setting: &Setting) -> RowKey {
    (setting.section, setting.group, setting.label.clone())
}

/// The widget id of a row's container, to scroll it into view.
pub(super) fn row_id(key: &RowKey) -> Id {
    Id::from(format!("setting-{}-{}-{}", key.0.title(), key.1, key.2))
}

/// The widget id of a row's text field, to put the keyboard in it.
pub(super) fn field_id(key: &'static str) -> Id {
    Id::from(format!("field-{key}"))
}

impl App {
    /// A key nothing captured while the settings dialog is open.
    pub(super) fn settings_key(&mut self, key: &Key, mods: Modifiers) -> Task<Message> {
        let Some(settings) = &self.settings else {
            return Task::none();
        };
        if mods.command()
            && let Key::Named(named @ (Named::PageUp | Named::PageDown)) = key.as_ref()
        {
            let pages: Vec<Section> = Section::shown().collect();
            let at = pages
                .iter()
                .position(|s| *s == settings.section)
                .unwrap_or(0);
            let next = if named == Named::PageDown {
                (at + 1) % pages.len()
            } else {
                (at + pages.len() - 1) % pages.len()
            };
            return self.settings_update(SettingsMessage::Section(pages[next]));
        }
        let rows = self.keyed_rows();
        let at = settings
            .row
            .as_ref()
            .and_then(|r| rows.iter().position(|s| row_key(s) == *r));
        let mut moved = None;
        let task = match key.as_ref() {
            Key::Named(Named::ArrowDown) => {
                moved = Some(at.map_or(0, |i| (i + 1).min(rows.len().saturating_sub(1))));
                Task::none()
            }
            Key::Named(Named::ArrowUp) => {
                moved = Some(at.map_or(rows.len().saturating_sub(1), |i| i.saturating_sub(1)));
                Task::none()
            }
            Key::Named(Named::Home) => {
                moved = Some(0);
                Task::none()
            }
            Key::Named(Named::End) => {
                moved = Some(rows.len().saturating_sub(1));
                Task::none()
            }
            Key::Named(named) => match at.and_then(|i| rows.get(i)) {
                Some(row) => change(&row.control, named),
                None => Task::none(),
            },
            _ => Task::none(),
        };
        let Some(i) = moved.filter(|i| rows.get(*i).is_some()) else {
            return task;
        };
        let key = row_key(&rows[i]);
        let id = row_id(&key);
        if let Some(settings) = &mut self.settings {
            settings.row = Some(key);
        }
        // Out of the search field (Up and Down pass through it), so Space and the arrows
        // reach the row.
        let unfocus = iced::advanced::widget::operate(
            iced::advanced::widget::operation::focusable::unfocus(),
        );
        let scroll = iced::advanced::widget::operate(RowSpan::new(id)).then(
            |(top, bottom, view, scrolled)| {
                // Only when it's out of view, and just far enough to show it.
                let y = if top - SCROLL_MARGIN < scrolled {
                    top - SCROLL_MARGIN
                } else if bottom + SCROLL_MARGIN > scrolled + view {
                    bottom + SCROLL_MARGIN - view
                } else {
                    return Task::none();
                };
                iced::widget::operation::scroll_to(
                    BODY_SCROLL,
                    AbsoluteOffset {
                        x: 0.0,
                        y: y.max(0.0),
                    },
                )
            },
        );
        Task::batch([unfocus, scroll])
    }

    /// The rows the keyboard moves through, in order: the page's (or the search's), minus
    /// folded cards and read-only shortcuts.
    fn keyed_rows(&self) -> Vec<Setting> {
        let Some(settings) = &self.settings else {
            return Vec::new();
        };
        let query = settings.query.trim().to_lowercase();
        self.settings_list()
            .into_iter()
            .filter(|s| {
                if query.is_empty() {
                    s.section == settings.section
                        && !settings.collapsed.contains(&(s.section, s.group))
                } else {
                    s.matches(&query)
                }
            })
            .filter(|s| !matches!(s.control, Control::Shortcut(_)))
            .collect()
    }
}

/// What `key` does to the ringed row's `control`.
fn change(control: &Control, key: Named) -> Task<Message> {
    let set =
        |key: &'static str, value| Task::done(Message::Settings(SettingsMessage::Set(key, value)));
    let activate = matches!(key, Named::Enter | Named::Space);
    match (control, key) {
        (Control::Toggle(k, on), _) if activate => set(k, Some(Value::Bool(!on))),
        (Control::Toggle(k, _), Named::ArrowLeft) => set(k, Some(Value::Bool(false))),
        (Control::Toggle(k, _), Named::ArrowRight) => set(k, Some(Value::Bool(true))),
        (
            Control::Choice(k, options, current) | Control::Dropdown(k, options, current),
            Named::Enter | Named::Space | Named::ArrowRight | Named::ArrowLeft,
        ) if !options.is_empty() => {
            let n = options.len();
            let next = match (current, key) {
                (Some(i), Named::ArrowLeft) => (i + n - 1) % n,
                (Some(i), _) => (i + 1) % n,
                (None, _) => 0,
            };
            set(k, Some(options[next].1.clone()))
        }
        (Control::Number { setting, value }, Named::ArrowLeft | Named::ArrowRight) => {
            let delta = if key == Named::ArrowLeft {
                -setting.step
            } else {
                setting.step
            };
            // Kept on the step's grid, as the dialog's own buttons do.
            let next = (((value + delta) / setting.step).round() * setting.step)
                .clamp(setting.min, setting.max);
            set(setting.key, Some(number(next)))
        }
        // Left and Right step through the sounds, which plays each as it's picked; Enter or
        // Space plays the one picked.
        (
            Control::Sound {
                key: k,
                value,
                presets,
            },
            Named::ArrowLeft | Named::ArrowRight,
        ) if !presets.is_empty() => {
            let n = presets.len();
            let next = match (presets.iter().position(|(_, v)| v == value), key) {
                (Some(i), Named::ArrowLeft) => (i + n - 1) % n,
                (Some(i), _) => (i + 1) % n,
                (None, _) => 0,
            };
            set(k, Some(Value::Str(presets[next].1.clone())))
        }
        (Control::Sound { value, .. }, _) if activate => {
            Task::done(Message::Settings(SettingsMessage::SoundPlay(value.clone())))
        }
        (Control::Button(_, message), _) if activate => Task::done(message.clone()),
        (
            Control::Text { key: k, .. }
            | Control::List { key: k, .. }
            | Control::Template { key: k, .. },
            _,
        ) if activate => iced::widget::operation::focus(field_id(k)),
        (Control::Number { setting, .. }, _) if activate => {
            iced::widget::operation::focus(field_id(setting.key))
        }
        (_, Named::Delete) => match control.key() {
            Some(k) => set(k, None),
            None => Task::none(),
        },
        _ => Task::none(),
    }
}

/// Finds a row in the dialog's scrolling body: its top and bottom within the body's content,
/// the body's height, and how far it's scrolled.
struct RowSpan {
    /// The row's container.
    target: Id,
    /// The body's viewport height, its content's top, and how far it's scrolled.
    body: Option<(f32, f32, f32)>,
    /// The row's top and bottom, once found.
    row: Option<(f32, f32)>,
}

impl RowSpan {
    /// Looks for the row with id `target`.
    fn new(target: Id) -> Self {
        Self {
            target,
            body: None,
            row: None,
        }
    }
}

impl Operation<(f32, f32, f32, f32)> for RowSpan {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<(f32, f32, f32, f32)>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        if id == Some(&Id::new(BODY_SCROLL)) {
            self.body = Some((bounds.height, content_bounds.y, translation.y));
        }
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id == Some(&self.target) {
            self.row = Some((bounds.y, bounds.y + bounds.height));
        }
    }

    fn finish(&self) -> Outcome<(f32, f32, f32, f32)> {
        match (self.body, self.row) {
            // Children of a scrollable are laid out unscrolled, so the row's place within the
            // content is its top minus the content's.
            (Some((view, content, scrolled)), Some((top, bottom))) => {
                Outcome::Some((top - content, bottom - content, view, scrolled))
            }
            _ => Outcome::None,
        }
    }
}

/// A settings row, as a screen reader is told about it.
pub(crate) struct AccessRow {
    /// Its label.
    pub(crate) label: String,
    /// The line under it.
    pub(crate) description: String,
    /// What it is and its value.
    pub(crate) kind: AccessKind,
    /// Whether the keyboard's ring is on it.
    pub(crate) ringed: bool,
}

/// What kind of control a settings row is, with its value.
pub(crate) enum AccessKind {
    /// A switch.
    Switch(bool),
    /// A choice, with the option in effect.
    Choice(String),
    /// A number, with its value, limits, step and unit.
    Number(f32, f32, f32, f32, &'static str),
    /// A text field and what it holds.
    Text(String),
    /// A button.
    Button,
    /// Anything else, described by its value.
    Other(String),
}

impl App {
    /// The open settings dialog for a screen reader: its page title and the rows the keyboard
    /// moves through.
    pub(crate) fn settings_access(&self) -> Option<(String, Vec<AccessRow>)> {
        let settings = self.settings.as_ref()?;
        let title = if settings.query.trim().is_empty() {
            format!("Settings, {}", settings.section.title())
        } else {
            format!("Settings, results for {}", settings.query.trim())
        };
        let rows = self
            .keyed_rows()
            .into_iter()
            .map(|s| {
                let ringed = settings.row.as_ref() == Some(&row_key(&s));
                let kind = match &s.control {
                    Control::Toggle(_, on) => AccessKind::Switch(*on),
                    Control::Choice(_, options, current)
                    | Control::Dropdown(_, options, current) => AccessKind::Choice(
                        current
                            .and_then(|i| options.get(i))
                            .map(|(label, _)| label.clone())
                            .unwrap_or_default(),
                    ),
                    Control::Number { setting, value } => AccessKind::Number(
                        *value * setting.unit.scale(),
                        setting.min * setting.unit.scale(),
                        setting.max * setting.unit.scale(),
                        setting.step * setting.unit.scale(),
                        setting.unit.suffix(),
                    ),
                    Control::Sound { value, presets, .. } => AccessKind::Choice(
                        presets
                            .iter()
                            .find(|(_, v)| v == value)
                            .map_or_else(|| value.clone(), |(label, _)| label.clone()),
                    ),
                    Control::Text { value, .. } | Control::Template { value, .. } => {
                        AccessKind::Text(value.clone())
                    }
                    Control::List { value, .. } => AccessKind::Text(value.join(" ")),
                    Control::Button(label, _) => {
                        return AccessRow {
                            label: format!("{}: {label}", s.label),
                            description: s.description.clone(),
                            kind: AccessKind::Button,
                            ringed,
                        };
                    }
                    Control::Shell { program, .. } => AccessKind::Other(if program.is_empty() {
                        "Default".to_owned()
                    } else {
                        program.clone()
                    }),
                    Control::Color { value, .. } => {
                        let [r, g, b, _] = value.into_rgba8();
                        AccessKind::Other(format!("#{r:02x}{g:02x}{b:02x}"))
                    }
                    Control::Shortcut(chord) => AccessKind::Other(chord.clone()),
                    Control::NewSsh => AccessKind::Other(String::new()),
                };
                AccessRow {
                    label: s.label.clone(),
                    description: s.description.clone(),
                    kind,
                    ringed,
                }
            })
            .collect();
        Some((title, rows))
    }
}

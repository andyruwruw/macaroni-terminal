//! The command palette (Ctrl+Shift+P, Cmd+Shift+P on macOS): one field that finds and runs
//! anything by name: keymap actions, workspaces, tabs, SSH profiles and settings pages.
//!
//! [`App::palette_items`] lists every target with its label and a detail line (an action's
//! shortcut, a tab's workspace); [`fuzzy`] scores each against what's typed and the best
//! [`MAX_SHOWN`] show, best first. Up and Down move the highlight (`palette_keys` in `main.rs`),
//! Enter or a click runs it, and Escape or a click outside closes the palette. Running an item
//! closes the palette first, so the item can open a dialog of its own.
//!
//! Opened as the attention inbox ([`PaletteMessage::OpenWaiting`], `show_waiting`), it lists
//! only the tabs waiting for the user, oldest notice first, each with its notice.
//!
//! Opened as scrollback search ([`PaletteMessage::OpenSearch`], `search_workspace` and
//! `search_all`), it copies the text of the workspace's (or every) terminal when it opens and
//! lists the lines containing what's typed, newest first. Running one shows its tab with the
//! find bar on that line.

use iced::widget::{button, column, container, mouse_area, opaque, row, text_input};
use iced::{Border, Color, Element, Fill, Task, alignment};
use macaroni_core::{Command, TabId, WorkspaceId};

use super::settings::{Section, SettingsMessage};
use super::widgets::{flat_button, input_style, one_line, shadow};
use crate::icons::{Icon, icon};
use crate::keymap;
use crate::{App, Message, glass};

/// The widget id of the palette's field, focused when it opens.
const PALETTE_INPUT: &str = "command-palette-input";

/// The most matches listed at once.
const MAX_SHOWN: usize = 12;

/// The palette's width when the window has room.
const WIDTH: f32 = 560.0;

/// The least room the palette leaves at the window's sides, together.
const MARGIN: f32 = 48.0;

/// How far below the window's top the palette sits.
const TOP: f32 = 72.0;

/// Corner radius of the palette.
const RADIUS: f32 = 12.0;

/// The text size of the field and the items.
const TEXT_SIZE: f32 = 13.0;

/// The text size of an item's detail.
const DETAIL_SIZE: f32 = 11.5;

/// The open palette, kept in `App.palette`.
#[derive(Debug, Default)]
pub struct Palette {
    /// What it lists.
    mode: Mode,
    /// What's typed.
    query: String,
    /// The highlighted match, counting from the best.
    selected: usize,
}

/// What the palette lists.
#[derive(Debug, Default)]
enum Mode {
    /// Actions, workspaces, tabs, SSH profiles and settings pages.
    #[default]
    Commands,
    /// The tabs waiting for the user (the attention inbox).
    Waiting,
    /// Lines of terminals' scrollback, copied when it opened.
    Search(Vec<Scrollback>),
}

/// One terminal's text, for scrollback search.
#[derive(Debug)]
struct Scrollback {
    /// Its tab.
    tab: TabId,
    /// Where the result says it is: the tab's title and its workspace.
    place: String,
    /// Its lines, oldest first.
    lines: Vec<String>,
}

/// What the palette sends, wrapped in [`Message::Palette`].
#[derive(Debug, Clone)]
pub enum PaletteMessage {
    /// Open it, empty (closing any menu).
    Open,
    /// Open it as the attention inbox: only the waiting tabs.
    OpenWaiting,
    /// Open it as scrollback search: every terminal (`true`) or the active workspace's.
    OpenSearch(bool),
    /// The field changed.
    Input(String),
    /// Move the highlight by this many matches (Up is -1, Down 1), stopping at the ends.
    Move(isize),
    /// Run the match at this position, counting from the best.
    Run(usize),
    /// Enter: run the highlighted match.
    Submit,
}

/// Something the palette can run.
#[derive(Debug, Clone, PartialEq)]
enum Target {
    /// A keymap action, by config name.
    Action(&'static str),
    /// Switch to a workspace.
    Workspace(WorkspaceId),
    /// Show a tab.
    Tab(TabId),
    /// Open the SSH profile at this index, in the focused pane.
    Ssh(usize),
    /// Show a tab with its find bar on a match of `query`: the `back`th match counting up from
    /// the newest (0 is the newest).
    Found {
        /// The terminal's tab.
        tab: TabId,
        /// What was searched for.
        query: String,
        /// How many matches are newer than this one.
        back: usize,
    },
    /// Open the settings dialog at a page.
    Settings(Section),
}

/// One entry: what it runs, what it's called, what's shown beside it, and its icon.
#[derive(Debug, Clone)]
struct Item {
    /// What running it does.
    target: Target,
    /// The name it's matched by.
    label: String,
    /// Shown dimmed at the right: a shortcut, a workspace, a host.
    detail: String,
    /// Its icon.
    icon: Icon,
}

impl App {
    /// Applies a palette message. Ignored while it's closed, except [`PaletteMessage::Open`].
    pub(crate) fn palette_update(&mut self, message: PaletteMessage) -> Task<Message> {
        let opened = match &message {
            PaletteMessage::Open => Some(Mode::Commands),
            PaletteMessage::OpenWaiting => Some(Mode::Waiting),
            PaletteMessage::OpenSearch(everywhere) => {
                Some(Mode::Search(self.scrollbacks(*everywhere)))
            }
            _ => None,
        };
        if let Some(mode) = opened {
            self.menu = None;
            self.command_palette = Some(Palette {
                mode,
                ..Palette::default()
            });
            return iced::widget::operation::focus(PALETTE_INPUT);
        }
        let Some(palette) = &mut self.command_palette else {
            return Task::none();
        };
        match message {
            PaletteMessage::Open | PaletteMessage::OpenWaiting | PaletteMessage::OpenSearch(_) => {
                Task::none()
            }
            PaletteMessage::Input(query) => {
                palette.query = query;
                palette.selected = 0;
                Task::none()
            }
            PaletteMessage::Move(by) => {
                let count = self.palette_matches().len();
                if let Some(palette) = &mut self.command_palette {
                    palette.selected = palette
                        .selected
                        .saturating_add_signed(by)
                        .min(count.saturating_sub(1));
                }
                Task::none()
            }
            PaletteMessage::Submit => {
                let selected = palette.selected;
                self.palette_run(selected)
            }
            PaletteMessage::Run(at) => self.palette_run(at),
        }
    }

    /// Closes the palette and runs the match at `at`, if there's one.
    fn palette_run(&mut self, at: usize) -> Task<Message> {
        let target = self.palette_matches().into_iter().nth(at).map(|i| i.target);
        self.command_palette = None;
        match target {
            None => Task::none(),
            Some(Target::Action(name)) => match keymap::action(name) {
                Some(action) => self.perform(action),
                None => Task::none(),
            },
            Some(Target::Workspace(id)) => self.apply(Command::SelectWorkspace(id)),
            Some(Target::Tab(tab)) => self.apply(Command::SelectTab(tab)),
            Some(Target::Ssh(index)) => {
                let pane = self.model.active_workspace().focused;
                self.new_ssh_tab(pane, index)
            }
            Some(Target::Found { tab, query, back }) => {
                let shown = self.apply(Command::SelectTab(tab));
                let found = self.find_in_tab(&query, back);
                Task::batch([shown, found])
            }
            Some(Target::Settings(section)) => Task::batch([
                self.open_settings(),
                self.settings_update(SettingsMessage::Section(section)),
            ]),
        }
    }

    /// Every running terminal's text (just the active workspace's unless `everywhere`), in
    /// sidebar order, for scrollback search.
    fn scrollbacks(&self, everywhere: bool) -> Vec<Scrollback> {
        let active = self.model.active_workspace().id;
        let mut found = Vec::new();
        for ws in self.model.all_workspaces() {
            if !everywhere && ws.id != active {
                continue;
            }
            for pane in ws.layout.leaves() {
                for &tab in self
                    .model
                    .pane(pane)
                    .map(|p| p.tabs.as_slice())
                    .unwrap_or_default()
                {
                    let Some(t) = self.model.tab(tab) else {
                        continue;
                    };
                    let macaroni_core::TabKind::Terminal(session) = t.kind else {
                        continue;
                    };
                    let Some(running) = self.running.get(&session) else {
                        continue;
                    };
                    found.push(Scrollback {
                        tab,
                        place: format!("{} \u{b7} {}", super::tabs::tab_title(t), ws.name),
                        lines: running
                            .session
                            .all_text()
                            .lines()
                            .map(str::to_owned)
                            .collect(),
                    });
                }
            }
        }
        found
    }

    /// Scrollback search's items: lines containing `query` (smart case, like the find bar),
    /// newest first within each terminal, at most [`MAX_SHOWN`] in all.
    fn search_items(scrollbacks: &[Scrollback], query: &str) -> Vec<Item> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        let exact = query.chars().any(char::is_uppercase);
        let count = |line: &str| {
            if exact {
                line.matches(query).count()
            } else {
                line.to_lowercase().matches(&query.to_lowercase()).count()
            }
        };
        let mut items = Vec::new();
        for scrollback in scrollbacks {
            // Matches newer than the line in hand, counted from the bottom up, which is how
            // far the find bar has to step back from the newest.
            let mut newer = 0;
            for line in scrollback.lines.iter().rev() {
                let here = count(line);
                if here == 0 {
                    continue;
                }
                items.push(Item {
                    target: Target::Found {
                        tab: scrollback.tab,
                        query: query.to_owned(),
                        back: newer + here - 1,
                    },
                    label: line.trim().to_owned(),
                    detail: scrollback.place.clone(),
                    icon: Icon::Search,
                });
                newer += here;
                if items.len() >= MAX_SHOWN {
                    return items;
                }
            }
        }
        items
    }

    /// The attention inbox's items: each waiting tab, oldest notice first, with its notice.
    fn waiting_items(&self) -> Vec<Item> {
        self.model
            .waiting_tabs()
            .into_iter()
            .filter_map(|tab| {
                let t = self.model.tab(tab)?;
                let notice = t.attention.as_ref()?;
                let ws = self
                    .workspace_of_tab(tab)
                    .and_then(|w| self.model.workspace(w))
                    .map(|w| w.name.clone())
                    .unwrap_or_default();
                Some(Item {
                    target: Target::Tab(tab),
                    label: format!("{}: {}", super::tabs::tab_title(t), notice.body),
                    detail: ws,
                    icon: Icon::Bell,
                })
            })
            .collect()
    }

    /// Every item, before matching: actions, then workspaces, tabs, SSH profiles and settings
    /// pages, each in the order the app shows them. The attention inbox lists only
    /// [`App::waiting_items`].
    fn palette_items(&self) -> Vec<Item> {
        if self
            .command_palette
            .as_ref()
            .is_some_and(|p| matches!(p.mode, Mode::Waiting))
        {
            return self.waiting_items();
        }
        let bindings = keymap::named_bindings(&self.config.keybindings);
        let shortcut = |name: &str| {
            bindings
                .iter()
                .find(|(_, n)| n == name)
                .map(|(chord, _)| keymap::chord_label(chord))
                .unwrap_or_default()
        };
        let mut items: Vec<Item> = keymap::NAMES
            .iter()
            .map(|&name| Item {
                target: Target::Action(name),
                label: keymap::label(name),
                detail: shortcut(name),
                icon: Icon::Forward,
            })
            .collect();
        // Every window's: going to one in another window brings that window up.
        for ws in self.model.all_workspaces() {
            items.push(Item {
                target: Target::Workspace(ws.id),
                label: ws.name.clone(),
                detail: "Workspace".to_owned(),
                icon: Icon::Terminal,
            });
        }
        for ws in self.model.all_workspaces() {
            for pane in ws.layout.leaves() {
                let Some(pane) = self.model.pane(pane) else {
                    continue;
                };
                for &tab in &pane.tabs {
                    let Some(t) = self.model.tab(tab) else {
                        continue;
                    };
                    items.push(Item {
                        target: Target::Tab(tab),
                        label: super::tabs::tab_title(t),
                        detail: ws.name.clone(),
                        icon: match t.kind {
                            macaroni_core::TabKind::Terminal(_) => Icon::Terminal,
                            macaroni_core::TabKind::Browser(_) => Icon::Globe,
                        },
                    });
                }
            }
        }
        for (index, profile) in self.config.ssh.iter().enumerate() {
            items.push(Item {
                target: Target::Ssh(index),
                label: format!("SSH: {}", profile.label()),
                detail: profile.host.clone(),
                icon: Icon::Terminal,
            });
        }
        for section in Section::shown() {
            items.push(Item {
                target: Target::Settings(section),
                label: format!("Settings: {}", section.title()),
                detail: "Settings".to_owned(),
                icon: Icon::Settings,
            });
        }
        items
    }

    /// The items matching what's typed, best first, at most [`MAX_SHOWN`]. Ties keep
    /// [`App::palette_items`]' order.
    fn palette_matches(&self) -> Vec<Item> {
        let query = self
            .command_palette
            .as_ref()
            .map_or("", |p| p.query.as_str());
        // Search results are already in their order and limited; fuzzy matching would reorder
        // them by the line's text.
        if let Some(Palette {
            mode: Mode::Search(scrollbacks),
            ..
        }) = &self.command_palette
        {
            return Self::search_items(scrollbacks, query);
        }
        let mut scored: Vec<(i32, Item)> = self
            .palette_items()
            .into_iter()
            .filter_map(|item| fuzzy(query, &item.label).map(|score| (score, item)))
            .collect();
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        scored
            .into_iter()
            .take(MAX_SHOWN)
            .map(|(_, item)| item)
            .collect()
    }

    /// The palette over a dimmed app, or nothing while it's closed.
    pub(crate) fn palette_view(&self) -> Option<Element<'_, Message>> {
        let palette = self.command_palette.as_ref()?;
        let c = self.chrome;
        let (placeholder, empty) = match &palette.mode {
            Mode::Waiting => ("Tabs waiting for you", "Nothing is waiting."),
            Mode::Search(_) if palette.query.trim().is_empty() => {
                ("Search terminals' scrollback", "Type to search.")
            }
            Mode::Search(_) => ("Search terminals' scrollback", "No line matches."),
            Mode::Commands => (
                "Run a command, or go to a workspace, tab or setting",
                "Nothing matches.",
            ),
        };
        let field = text_input(placeholder, &palette.query)
            .id(PALETTE_INPUT)
            .on_input(|value| Message::Palette(PaletteMessage::Input(value)))
            .on_submit(Message::Palette(PaletteMessage::Submit))
            .size(TEXT_SIZE)
            .padding([8, 10])
            .style(move |_, status| input_style(c, status));
        let mut list = column![].spacing(1);
        let matches = self.palette_matches();
        if matches.is_empty() {
            list = list.push(container(one_line(empty, TEXT_SIZE, c.muted)).padding([6, 10]));
        }
        for (at, item) in matches.into_iter().enumerate() {
            let selected = at == palette.selected;
            let line = row![
                icon(item.icon, 14.0, c.label),
                one_line(item.label, TEXT_SIZE, c.text).width(Fill),
                one_line(item.detail, DETAIL_SIZE, c.muted),
            ]
            .spacing(10)
            .align_y(alignment::Vertical::Center);
            list = list.push(
                button(line)
                    .on_press(Message::Palette(PaletteMessage::Run(at)))
                    .width(Fill)
                    .padding([6, 10])
                    .style(move |_, status| {
                        let status = if selected {
                            button::Status::Hovered
                        } else {
                            status
                        };
                        flat_button(c, status, None)
                    }),
            );
        }
        let card = container(column![field, list].spacing(8))
            .padding(10)
            .width((self.window_size().width - MARGIN).clamp(280.0, WIDTH))
            .style(move |_| container::Style {
                background: Some(
                    if c.glass.is_some() {
                        c.popup
                    } else {
                        c.background
                    }
                    .into(),
                ),
                border: Border {
                    color: if c.glass.is_some() {
                        Color::TRANSPARENT
                    } else {
                        c.item_edge
                    },
                    width: 1.0,
                    radius: RADIUS.into(),
                },
                shadow: shadow(c.menu_shadow, 12.0, 40.0),
                ..container::Style::default()
            });
        let backdrop = mouse_area(
            container(opaque(glass::sheet(c, card, RADIUS, 1.0)))
                .width(Fill)
                .height(Fill)
                .align_x(alignment::Horizontal::Center)
                .padding(iced::Padding::default().top(TOP))
                .style(move |_| container::Style {
                    background: Some(c.overlay.into()),
                    ..container::Style::default()
                }),
        )
        .on_press(Message::Dismiss);
        Some(backdrop.into())
    }
}

/// How well `query` matches `text`, or `None` when it doesn't: every character of the query
/// must appear in order (ignoring case and spaces). Higher is better: characters that follow
/// each other, start a word, or start the text count more, and shorter texts win ties. An
/// empty query matches everything equally.
fn fuzzy(query: &str, text: &str) -> Option<i32> {
    let query: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if query.is_empty() {
        return Some(0);
    }
    let text: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let mut score = 0;
    let mut at = 0;
    let mut previous: Option<usize> = None;
    for wanted in query {
        let found = (at..text.len()).find(|&i| text[i] == wanted)?;
        score += 1;
        if previous.is_some_and(|p| p + 1 == found) {
            score += 5;
        }
        if found == 0 {
            score += 8;
        } else if !text[found - 1].is_alphanumeric() {
            score += 4;
        }
        previous = Some(found);
        at = found + 1;
    }
    Some(score * 100 - text.len() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_needs_every_letter_in_order() {
        assert!(fuzzy("spr", "Split right").is_some());
        assert!(fuzzy("rs", "Split right").is_none());
        assert_eq!(fuzzy("", "anything"), Some(0));
    }

    #[test]
    fn fuzzy_prefers_word_starts_and_runs() {
        let start = fuzzy("split", "Split right").unwrap();
        let scattered = fuzzy("split", "Settings: Panes, lights and tabs").unwrap();
        assert!(start > scattered);
        assert!(
            fuzzy("sr", "Split right").unwrap()
                > fuzzy("sr", "Close tab after search").unwrap_or(0)
        );
    }

    #[test]
    fn scrollback_search_lists_newest_first_with_steps_back() {
        let scrollbacks = vec![Scrollback {
            tab: TabId(7),
            place: "tab · ws".into(),
            lines: vec![
                "error one".into(),
                "fine".into(),
                "error two, error three".into(),
            ],
        }];
        let items = App::search_items(&scrollbacks, "error");
        let found: Vec<_> = items
            .iter()
            .map(|i| match &i.target {
                Target::Found { back, .. } => (i.label.as_str(), *back),
                _ => unreachable!(),
            })
            .collect();
        // The newest line has two matches, so the older line is three steps back.
        assert_eq!(found, vec![("error two, error three", 1), ("error one", 2)]);
        assert!(
            App::search_items(&scrollbacks, "Error").is_empty(),
            "capitals match exactly"
        );
        assert!(App::search_items(&scrollbacks, " ").is_empty());
    }

    #[test]
    fn every_palette_action_exists() {
        for name in keymap::NAMES {
            assert!(keymap::action(name).is_some(), "{name}");
        }
    }
}

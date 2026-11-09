//! Working the interface from the keyboard: F6 moves between the focused terminal, its pane's
//! tab strip and the sidebar, arrows move a focus ring along them, and menus answer arrows.
//!
//! - `App.key_focus` is the focused stop ([`KeyFocus`]), `None` while a terminal or page has the
//!   keyboard. While it's set, terminals don't take keys and [`App::focus_key`] gets every key
//!   nothing else captured (`focus_keys` in `main.rs`).
//! - A region's stops are [`App::sidebar_stops`] (group headers and workspaces as the sidebar
//!   lists them, then New workspace) and [`App::strip_stops`] (a pane's tabs, its `+`, then its
//!   new tab buttons, [`App::strip_buttons`]). The views draw a ring on the focused one.
//! - Enter activates and hands the keyboard back to the terminal; Space activates and stays;
//!   F2 renames; Delete closes a tab; Shift+F10 or the menu key opens the item's menu where the
//!   item is on screen (found by its widget id, [`KeyFocus::widget_id`]); Escape leaves.
//! - In an open menu, `Menu.cursor` is the highlighted item: Up and Down move it, Enter or Space
//!   picks it, Right opens a submenu and Left (or Escape, in `dismiss`) closes it.

use iced::keyboard::key::{Named, Physical};
use iced::keyboard::{Key, Modifiers};
use iced::{Point, Rectangle, Task};
use macaroni_core::{Command, PaneId, Rect};

use crate::accessibility::bounds;
use crate::icons::Icon;
use crate::message::Message;
use crate::state::{KeyFocus, MenuCursor, MenuTarget, Renaming};
use crate::view::MenuEntryKind;
use crate::{App, GroupAction, MenuPage};

/// One of a tab strip's new tab buttons: what it shows, its name, and what pressing it sends
/// (`None` while it's unavailable, like Git outside a repository).
pub(crate) struct StripButton {
    /// Its icon.
    pub(crate) icon: Icon,
    /// Its name, on hover and for screen readers.
    pub(crate) name: &'static str,
    /// What pressing it does.
    pub(crate) message: Option<Message>,
}

/// Which way a key moves the focus ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// One back.
    Back,
    /// One on.
    On,
    /// To the first.
    First,
    /// To the last.
    Last,
}

impl App {
    /// F6 (`true`) or Shift+F6: the next region after the one with the keyboard, in the order
    /// terminal, tab strip, sidebar (skipped while it's folded away).
    pub(crate) fn cycle_region(&mut self, forward: bool) -> Task<Message> {
        let strip = self.strip_stop_for(self.model.active_workspace().focused);
        let sidebar = (!self.sidebar.collapsed).then(|| self.sidebar_home());
        let regions = [None, strip, sidebar.flatten()];
        let here = match self.key_focus {
            None => 0,
            Some(f) if f.in_sidebar() => 2,
            Some(_) => 1,
        };
        let mut next = here;
        // Regions with nothing to stop on are passed over; the terminal always has one.
        for _ in 0..regions.len() {
            next = if forward {
                (next + 1) % regions.len()
            } else {
                (next + regions.len() - 1) % regions.len()
            };
            if next == 0 || regions[next].is_some() {
                break;
            }
        }
        self.key_focus = regions[next];
        self.menu = None;
        Task::none()
    }

    /// A key while `key_focus` is set or a menu is open. Keys the ring doesn't use are looked
    /// up as shortcuts, since no terminal takes them meanwhile.
    pub(crate) fn focus_key(
        &mut self,
        key: Key,
        physical: Physical,
        mods: Modifiers,
    ) -> Task<Message> {
        if self.menu.is_some() {
            return self.menu_key(&key);
        }
        let Some(focus) = self.key_focus else {
            return Task::none();
        };
        // A stop that's gone (its tab closed, its workspace filtered out) falls back to its
        // region's first.
        let stops = self.stops_around(focus);
        let Some(at) = stops.iter().position(|s| *s == focus) else {
            self.key_focus = stops.first().copied();
            return Task::none();
        };
        let step = |step| match step {
            Step::Back => at.saturating_sub(1),
            Step::On => (at + 1).min(stops.len() - 1),
            Step::First => 0,
            Step::Last => stops.len() - 1,
        };
        let sidebar = focus.in_sidebar();
        match key.as_ref() {
            Key::Named(Named::ArrowUp) if sidebar => self.key_focus = Some(stops[step(Step::Back)]),
            Key::Named(Named::ArrowDown) if sidebar => self.key_focus = Some(stops[step(Step::On)]),
            Key::Named(Named::ArrowLeft) if !sidebar => {
                self.key_focus = Some(stops[step(Step::Back)]);
            }
            Key::Named(Named::ArrowRight) if !sidebar => {
                self.key_focus = Some(stops[step(Step::On)]);
            }
            // A group header folds and unfolds with Left and Right, like a tree.
            Key::Named(named @ (Named::ArrowLeft | Named::ArrowRight)) => {
                if let KeyFocus::Group(group) = focus {
                    let collapse = named == Named::ArrowLeft;
                    return Task::done(Message::Group(group, GroupAction::Collapse(collapse)));
                }
            }
            // In a strip, Up and Down go to the pane before or after this one.
            Key::Named(named @ (Named::ArrowUp | Named::ArrowDown)) => {
                return self.next_strip(focus, named == Named::ArrowDown);
            }
            Key::Named(Named::Home) => self.key_focus = Some(stops[step(Step::First)]),
            Key::Named(Named::End) => self.key_focus = Some(stops[step(Step::Last)]),
            Key::Named(Named::Tab) => return self.cycle_region(!mods.shift()),
            Key::Named(Named::Enter) => return self.activate(focus, false),
            Key::Named(Named::Space) => return self.activate(focus, true),
            Key::Named(Named::Escape) => self.key_focus = None,
            Key::Named(Named::F2) => return self.rename_focused(focus),
            Key::Named(Named::Delete) => {
                if let KeyFocus::Tab(tab) = focus {
                    // The next stop takes the ring; the strip's `+` is always there.
                    self.key_focus = stops
                        .get(at + 1)
                        .or(at.checked_sub(1).and_then(|i| stops.get(i)))
                        .copied();
                    return self.apply(Command::CloseTab(tab));
                }
            }
            Key::Named(Named::ContextMenu) => return self.open_focus_menu(focus),
            Key::Named(Named::F10) if mods.shift() => return self.open_focus_menu(focus),
            _ => {
                if let Some(action) = self.keymap.lookup(&key, physical, mods) {
                    return self.perform(action);
                }
            }
        }
        Task::none()
    }

    /// The stops in `focus`'s region: the sidebar's, or its pane's strip's.
    fn stops_around(&self, focus: KeyFocus) -> Vec<KeyFocus> {
        if focus.in_sidebar() {
            return self.sidebar_stops();
        }
        let pane = match focus {
            KeyFocus::Tab(tab) => self.model.pane_of_tab(tab),
            KeyFocus::Add(pane) | KeyFocus::Button(pane, _) => Some(pane),
            _ => None,
        };
        pane.map(|p| self.strip_stops(p)).unwrap_or_default()
    }

    /// The sidebar's stops, top to bottom as it lists them: each group's header, its
    /// workspaces unless it's folded, ungrouped workspaces, then New workspace. A search shows
    /// only what matches, folded groups' members included.
    pub(crate) fn sidebar_stops(&self) -> Vec<KeyFocus> {
        let query = if self.sidebar_searching() {
            self.sidebar_query.trim()
        } else {
            ""
        };
        let mut stops = Vec::new();
        let mut previous = None;
        for ws in self.shown().workspaces {
            if !self.workspace_matches(ws, query) {
                continue;
            }
            let group = ws.group.and_then(|g| self.model.group(g));
            if ws.group != previous {
                previous = ws.group;
                if let Some(group) = group {
                    stops.push(KeyFocus::Group(group.id));
                }
            }
            if group.is_none_or(|g| !g.collapsed || !query.is_empty()) {
                stops.push(KeyFocus::Workspace(ws.id));
            }
        }
        stops.push(KeyFocus::NewWorkspace);
        stops
    }

    /// Where the sidebar's ring starts: the open workspace.
    fn sidebar_home(&self) -> Option<KeyFocus> {
        let stops = self.sidebar_stops();
        let open = KeyFocus::Workspace(self.shown().active);
        stops
            .contains(&open)
            .then_some(open)
            .or(stops.first().copied())
    }

    /// A pane's strip's stops, left to right: its tabs, its `+`, and its new tab buttons while
    /// they show.
    pub(crate) fn strip_stops(&self, pane: PaneId) -> Vec<KeyFocus> {
        let Some(p) = self.model.pane(pane) else {
            return Vec::new();
        };
        let mut stops: Vec<KeyFocus> = p.tabs.iter().map(|t| KeyFocus::Tab(*t)).collect();
        stops.push(KeyFocus::Add(pane));
        if self.strip_buttons_shown(pane) {
            stops.extend((0..self.strip_buttons(pane).len()).map(|i| KeyFocus::Button(pane, i)));
        }
        stops
    }

    /// Where a pane's strip's ring starts: its open tab.
    fn strip_stop_for(&self, pane: PaneId) -> Option<KeyFocus> {
        self.model.pane(pane).map(|p| KeyFocus::Tab(p.active))
    }

    /// Whether `pane`'s strip shows its new tab buttons (`[tab_items] new_buttons`, and the
    /// find bar isn't in their place).
    pub(crate) fn strip_buttons_shown(&self, pane: PaneId) -> bool {
        self.config.tab_items.new_buttons
            && self.model.pane(pane).is_none_or(|p| !self.search_shown(p))
    }

    /// A pane's new tab buttons, left to right: a terminal, a browser, a Git tab (unavailable
    /// outside a repository) and VS Code when it can open.
    pub(crate) fn strip_buttons(&self, pane: PaneId) -> Vec<StripButton> {
        let mut buttons = vec![
            StripButton {
                icon: Icon::Terminal,
                name: "New terminal",
                message: Some(Message::Commands(vec![
                    Command::FocusPane(pane),
                    Command::NewTab,
                ])),
            },
            StripButton {
                icon: Icon::Globe,
                name: "New browser",
                message: Some(Message::NewBrowserIn(pane)),
            },
            if self.pane_in_repository(pane) {
                StripButton {
                    icon: Icon::GitCompare,
                    name: "New Git tab",
                    message: Some(Message::GitTab(pane)),
                }
            } else {
                StripButton {
                    icon: Icon::GitCompare,
                    name: "Git: not in a repository",
                    message: None,
                }
            },
        ];
        // A tab when VS Code for the web is downloaded, else the desktop app.
        if self.vscode_web.installed {
            buttons.push(StripButton {
                icon: Icon::Code,
                name: "New VS Code tab",
                message: Some(Message::VsCodeTab(pane)),
            });
        } else if self.vscode.is_some() {
            buttons.push(StripButton {
                icon: Icon::Code,
                name: "Open in VS Code",
                message: Some(Message::OpenVsCode(pane)),
            });
        }
        buttons
    }

    /// Up or Down in a strip: the open tab of the pane before or after, in the order the
    /// workspace shows them (pane letters' order).
    fn next_strip(&mut self, focus: KeyFocus, down: bool) -> Task<Message> {
        let pane = match focus {
            KeyFocus::Tab(tab) => self.model.pane_of_tab(tab),
            KeyFocus::Add(pane) | KeyFocus::Button(pane, _) => Some(pane),
            _ => None,
        };
        let panes: Vec<PaneId> = self
            .model
            .active_workspace()
            .shown_panes(Rect::UNIT)
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        let Some(at) = pane.and_then(|p| panes.iter().position(|q| *q == p)) else {
            return Task::none();
        };
        let next = if down {
            panes.get(at + 1)
        } else {
            at.checked_sub(1).and_then(|i| panes.get(i))
        };
        if let Some(stop) = next.and_then(|p| self.strip_stop_for(*p)) {
            self.key_focus = Some(stop);
        }
        Task::none()
    }

    /// Enter (`stay` false: the keyboard goes back to the terminal) or Space on a stop.
    pub(crate) fn activate(&mut self, focus: KeyFocus, stay: bool) -> Task<Message> {
        if !stay {
            self.key_focus = None;
        }
        match focus {
            KeyFocus::Workspace(ws) => self.apply(Command::SelectWorkspace(ws)),
            KeyFocus::Group(group) => {
                let collapsed = self.model.group(group).is_some_and(|g| g.collapsed);
                // A header stays focused: there's nothing to go back to it for.
                self.key_focus = Some(focus);
                Task::done(Message::Group(group, GroupAction::Collapse(!collapsed)))
            }
            KeyFocus::NewWorkspace => self.apply(Command::NewWorkspace),
            KeyFocus::Tab(tab) => self.apply(Command::SelectTab(tab)),
            KeyFocus::Add(pane) => self.apply_all(vec![Command::FocusPane(pane), Command::NewTab]),
            KeyFocus::Button(pane, n) => {
                match self
                    .strip_buttons(pane)
                    .into_iter()
                    .nth(n)
                    .and_then(|b| b.message)
                {
                    Some(message) => Task::done(message),
                    None => Task::none(),
                }
            }
        }
    }

    /// F2: renames the focused workspace, group or tab. The field takes the keyboard.
    fn rename_focused(&mut self, focus: KeyFocus) -> Task<Message> {
        let target = match focus {
            KeyFocus::Workspace(ws) => Renaming::Workspace(ws),
            KeyFocus::Group(group) => Renaming::Group(group),
            KeyFocus::Tab(tab) => Renaming::Tab(tab),
            _ => return Task::none(),
        };
        self.key_focus = None;
        self.start_rename(target)
    }

    /// Shift+F10 or the menu key: the focused item's menu, under the item, with its first
    /// item highlighted.
    pub(crate) fn open_focus_menu(&mut self, focus: KeyFocus) -> Task<Message> {
        let target = match focus {
            KeyFocus::Workspace(ws) => MenuTarget::Workspace(ws),
            KeyFocus::Group(group) => MenuTarget::Group(group),
            KeyFocus::Tab(tab) => MenuTarget::Tab(tab),
            KeyFocus::Add(pane) => MenuTarget::NewTab(pane),
            KeyFocus::NewWorkspace | KeyFocus::Button(..) => return Task::none(),
        };
        bounds::find(focus.widget_id()).map(move |at| Message::KeyMenu(target, at))
    }

    /// Opens `target`'s menu from the keyboard, at the bottom left of the item (`at`, where it
    /// was found on screen), else at the window's top left.
    pub(crate) fn key_menu(&mut self, target: MenuTarget, at: Option<Rectangle>) -> Task<Message> {
        let at = at.map_or(Point::ORIGIN, |b| Point::new(b.x, b.y + b.height));
        let task = self.menu_open(target, at);
        let first = self.first_item(target, MenuPage::Main);
        if let Some(menu) = &mut self.menu {
            menu.cursor = first.map(|index| MenuCursor { sub: false, index });
        }
        task
    }

    /// A key while a menu is open. Escape is `dismiss`'s.
    fn menu_key(&mut self, key: &Key) -> Task<Message> {
        let Some(menu) = self.menu else {
            return Task::none();
        };
        let in_sub = menu.cursor.is_some_and(|c| c.sub) && menu.sub.is_some();
        let page = match menu.sub {
            Some(sub) if in_sub => sub.page,
            _ => MenuPage::Main,
        };
        let items = self.menu_items(menu.target, page);
        let current = menu
            .cursor
            .filter(|c| c.sub == in_sub)
            .and_then(|c| items.iter().position(|(i, _)| *i == c.index));
        let set = |app: &mut Self, at: Option<usize>| {
            if let Some(menu) = &mut app.menu {
                menu.cursor = at
                    .and_then(|at| items.get(at))
                    .map(|(index, _)| MenuCursor {
                        sub: in_sub,
                        index: *index,
                    });
            }
        };
        let last = items.len().checked_sub(1);
        match key.as_ref() {
            Key::Named(Named::ArrowDown) => {
                set(
                    self,
                    current.map_or(Some(0), |c| Some((c + 1).min(last.unwrap_or(0)))),
                );
            }
            Key::Named(Named::ArrowUp) => {
                set(self, current.map_or(last, |c| Some(c.saturating_sub(1))));
            }
            Key::Named(Named::Home) => set(self, Some(0)),
            Key::Named(Named::End) => set(self, last),
            Key::Named(Named::ArrowLeft) if in_sub => return self.close_key_submenu(),
            Key::Named(Named::ArrowRight | Named::Enter) | Key::Named(Named::Space) => {
                let Some((_, kind)) = current.and_then(|c| items.get(c)).cloned() else {
                    return Task::none();
                };
                let arrow = *key == Key::Named(Named::ArrowRight);
                match kind {
                    MenuEntryKind::Submenu(page) if !in_sub => {
                        // Always `Task::none()`.
                        let _ = self.menu_pin(page);
                        let first = self.first_item(menu.target, page);
                        if let Some(menu) = &mut self.menu {
                            menu.cursor = first.map(|index| MenuCursor { sub: true, index });
                        }
                    }
                    MenuEntryKind::Action(message) if !arrow => return Task::done(message),
                    _ => {}
                }
            }
            _ => {}
        }
        Task::none()
    }

    /// Closes the open submenu and puts the highlight back on the item that opened it.
    pub(crate) fn close_key_submenu(&mut self) -> Task<Message> {
        let Some(menu) = self.menu else {
            return Task::none();
        };
        let Some(sub) = menu.sub else {
            return Task::none();
        };
        let opener = self
            .menu_items(menu.target, MenuPage::Main)
            .into_iter()
            .find(|(_, k)| matches!(k, MenuEntryKind::Submenu(p) if *p == sub.page))
            .map(|(i, _)| i);
        if let Some(menu) = &mut self.menu {
            menu.sub = None;
            menu.cursor = opener.map(|index| MenuCursor { sub: false, index });
        }
        Task::none()
    }

    /// The first item the keyboard can highlight on `page` of `target`'s menu.
    fn first_item(&self, target: MenuTarget, page: MenuPage) -> Option<usize> {
        self.menu_items(target, page).first().map(|(i, _)| *i)
    }
}

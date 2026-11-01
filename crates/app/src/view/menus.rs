//! The right-click menus: their entries for workspaces, groups, tabs and the new tab
//! button, and how their lists and submenus are laid out.
//!
//! [`App::menu_entries`] is the one description of a menu page, as a list of [`MenuEntry`]s.
//! Both drawing ([`App::menu_list`], placed by `App::menu_layers` in the parent module) and
//! sizing ([`App::menu_bounds`], from [`MenuEntry::height`]) read it, so a list's computed height
//! always matches what's drawn. `menu_bounds` is also what `sync_browsers` uses to hide browser
//! pages under an open menu. The entries' messages ([`Message::Workspace`], [`Message::Tab`],
//! [`Message::Group`], [`Message::MenuPin`] and so on) are handled in `update`.

use iced::widget::{Space, button, column, container, mouse_area, row, text_input};
use iced::{Background, Border, Color, Element, Fill, Point, Rectangle, Size, alignment};
use macaroni_core::{
    Direction, Group, GroupId, MoveTo, PaneId, Shift, Tab, TabId, Workspace, WorkspaceId,
};

use super::tabs::tab_kind_icon;
use super::widgets::{FIELD_PADDING, card_style, flat_button, input_style, one_line, shadow};
use crate::hair_line::HairLine;
use crate::icons::{Icon, WORKSPACE_ICONS, icon};
use crate::theme::{Chrome, PALETTE, WORKSPACE_COLORS, custom_color, hex, workspace_color};
use crate::update::same_dir;
use crate::view::folder_dialog::FolderMessage;
use crate::view::icon_picker::{IconPickerMessage, IconTarget};
use crate::{
    App, ColorTarget, GroupAction, MenuPage, MenuTarget, Message, TabAction, WorkspaceAction, glass,
};

/// The height of the custom color row, fixed so [`MenuEntry::height`] knows it without layout.
const COLOR_INPUT_HEIGHT: f32 = 36.0;

/// Between the custom color field's preview and the field.
const COLOR_INPUT_SPACING: f32 = 4.0;

/// How many cells to a row in the icon and color pickers.
const GRID_COLUMNS: usize = 6;

/// Between a picker's cells, across and down.
const GRID_SPACING: f32 = 2.0;

/// The color palette's columns: one per hue in [`PALETTE`].
const PALETTE_COLUMNS: usize = PALETTE.len();

/// A palette cell's height; its width is the menu's share.
const PALETTE_CELL: f32 = 18.0;

/// A palette swatch's size inside its cell.
const PALETTE_SWATCH: f32 = 13.0;

/// Between palette cells.
const PALETTE_SPACING: f32 = 1.0;

/// Around a separator's line, above and below then left and right.
const SEPARATOR_PADDING: [f32; 2] = [4.0, 6.0];

/// How much smaller a submenu's chevron is drawn than the item's icon.
const CHEVRON_SHRINK: f32 = 1.0;

impl App {
    /// Where the right-click menu's lists are drawn, kept inside the window: the main one, then
    /// the open submenu beside its item (to the right, or the left if there's no room). Browser
    /// pages under them are hidden while they're open, since nothing iced draws can cover them.
    pub fn menu_bounds(&self) -> Vec<Rectangle> {
        let c = self.chrome;
        let Some(menu) = self.menu else {
            return Vec::new();
        };
        let window = self.window_size;
        let main = self.menu_entries(menu.target, MenuPage::Main);
        let height = c.s.menus.padding * 2.0 + main.iter().map(|e| e.height(c)).sum::<f32>();
        // Slid left to stay in the window, but never past its left margin. Vertically it flips
        // to open upward from the cursor instead, like a system menu.
        let x = menu
            .at
            .x
            .min(window.width - c.s.menus.width - c.s.menus.margin)
            .max(c.s.menus.margin);
        let y = if menu.at.y + height > window.height - c.s.menus.margin {
            (menu.at.y - height).max(c.s.menus.margin)
        } else {
            menu.at.y
        };
        let mut bounds = vec![Rectangle::new(
            Point::new(x, y),
            Size::new(c.s.menus.width, height),
        )];
        let Some(page) = menu.sub.map(|s| s.page) else {
            return bounds;
        };
        // The members prompt replaces the Color submenu, so it lines up with the Color item.
        let opener = match page {
            MenuPage::MembersColor => MenuPage::Color,
            page => page,
        };
        // The opener's top within the list. Both lists have the same padding, so offsetting the
        // submenu by this puts its first item level with the opener.
        let item_y: f32 = main
            .iter()
            .take_while(|e| !matches!(e, MenuEntry::Item { submenu: Some(p), .. } if *p == opener))
            .map(|e| e.height(c))
            .sum();
        let sub_height = c.s.menus.padding * 2.0
            + self
                .menu_entries(menu.target, page)
                .iter()
                .map(|e| e.height(c))
                .sum::<f32>();
        let right = x + c.s.menus.width + c.s.menus.submenu_gap;
        let sub_x = if right + c.s.menus.width <= window.width - c.s.menus.margin {
            right
        } else {
            (x - c.s.menus.width - c.s.menus.submenu_gap).max(c.s.menus.margin)
        };
        // Its first item level with the one that opened it.
        let sub_y = (y + item_y)
            .min(window.height - c.s.menus.margin - sub_height)
            .max(c.s.menus.margin);
        bounds.push(Rectangle::new(
            Point::new(sub_x, sub_y),
            Size::new(c.s.menus.width, sub_height),
        ));
        bounds
    }

    /// The entries of `target`'s menu on `page`. Pages a target doesn't have fall back to its
    /// main list (or nothing, for a workspace's [`MenuPage::MembersColor`]).
    fn menu_entries(&self, target: MenuTarget, page: MenuPage) -> Vec<MenuEntry> {
        match target {
            MenuTarget::NewTab(pane) => self.new_tab_menu_entries(pane),
            MenuTarget::Tab(tab) => self.tab_menu_entries(tab, page),
            MenuTarget::Workspace(ws) => self.workspace_menu_entries(ws, page),
            MenuTarget::Group(group) => self.group_menu_entries(group, page),
        }
    }

    /// A workspace's menu on `page`; empty if the workspace is gone. Inside a group, Move goes
    /// up and down among the group's members; outside, among the blocks ([`App::block_room`]).
    fn workspace_menu_entries(&self, id: WorkspaceId, page: MenuPage) -> Vec<MenuEntry> {
        let c = self.chrome;
        let Some(ws) = self.model.workspace(id) else {
            return Vec::new();
        };
        let action = |action| Some(Message::Workspace(id, action));
        match page {
            MenuPage::Main => self.workspace_main_entries(ws),
            MenuPage::Move => {
                let (up, down) = match ws.group {
                    Some(group) => {
                        let members: Vec<WorkspaceId> = self
                            .model
                            .workspaces()
                            .iter()
                            .filter(|w| w.group == Some(group))
                            .map(|w| w.id)
                            .collect();
                        let at = members.iter().position(|w| *w == id).unwrap_or(0);
                        (at > 0, at + 1 < members.len())
                    }
                    None => self.block_room(|w| w.id == id),
                };
                move_entries(up, down, |shift| {
                    Message::Workspace(id, WorkspaceAction::Shift(shift))
                })
            }
            MenuPage::Icon => vec![
                icon_grid(
                    Some(ws.icon.as_deref().unwrap_or("terminal")),
                    workspace_color(ws.color.as_deref()).unwrap_or(c.label),
                    |name| action(WorkspaceAction::Icon(Some(name))),
                ),
                MenuEntry::Separator,
                more_icons(IconTarget::Workspace(id)),
                MenuEntry::item(
                    Icon::Terminal,
                    "Default icon",
                    action(WorkspaceAction::Icon(None)),
                ),
            ],
            MenuPage::Color => {
                color_entries(ws.color.as_deref(), ColorTarget::Workspace(id), |color| {
                    action(WorkspaceAction::Color(color))
                })
            }
            // Only groups ask about their members.
            MenuPage::MembersColor => Vec::new(),
            MenuPage::Group => self.workspace_group_entries(ws),
            MenuPage::Combine => self.combine_entries(ws),
            // Group pages.
            MenuPage::Merge | MenuPage::Delete => Vec::new(),
        }
    }

    /// A workspace's main menu, in groups: pin (first, it's the most used), what it is and where
    /// (rename, move, group, combine), how it looks (icon, color), its folder, then mute, its
    /// id, and closing it. A grouped workspace is pinned with its group.
    fn workspace_main_entries(&self, ws: &Workspace) -> Vec<MenuEntry> {
        let id = ws.id;
        let action = |action| Some(Message::Workspace(id, action));
        let folder = self.workspace_folder(id);
        let mut entries = vec![
            pin_entry(
                ws.pinned,
                if ws.group.is_some() {
                    "Group"
                } else {
                    "Workspace"
                },
                |pin| action(WorkspaceAction::Pin(pin)),
            ),
            MenuEntry::Separator,
            MenuEntry::item(
                Icon::Pencil,
                "Rename Workspace",
                action(WorkspaceAction::Rename),
            ),
            MenuEntry::submenu(Icon::Move, "Move Workspace", MenuPage::Move),
            MenuEntry::submenu(Icon::Folder, "Group", MenuPage::Group),
            // Only with another workspace to combine.
            if self.model.workspaces().len() > 1 {
                MenuEntry::submenu(Icon::Combine, "Combine Workspaces", MenuPage::Combine)
            } else {
                MenuEntry::item(Icon::Combine, "Combine Workspaces", None)
            },
            MenuEntry::Separator,
            MenuEntry::submenu(Icon::Star, "Icon", MenuPage::Icon),
            MenuEntry::submenu(Icon::Palette, "Color", MenuPage::Color),
            MenuEntry::Separator,
            MenuEntry::Header(format!("Folder: {}", short_path(&folder))),
            MenuEntry::item(
                Icon::FolderPen,
                "Set Active Folder\u{2026}",
                Some(Message::Folder(FolderMessage::Open(id))),
            ),
            MenuEntry::Separator,
            mute_entry(ws.muted, "Workspace", |mute| {
                action(WorkspaceAction::Mute(mute))
            }),
            MenuEntry::item(
                Icon::Copy,
                "Copy Workspace ID",
                action(WorkspaceAction::CopyId),
            ),
            MenuEntry::Separator,
        ];
        entries.push(MenuEntry::item(
            Icon::Close,
            "Close Workspace",
            action(WorkspaceAction::Close),
        ));
        entries
    }

    /// A tab's main menu, grouped like a workspace's: pin; rename, move; icon, color; the
    /// workspace's folder; mute and its id; then closing its neighbours (pinned tabs are never
    /// closed in bulk) and, last, the tab itself.
    fn tab_main_entries(&self, tab: &Tab) -> Vec<MenuEntry> {
        let id = tab.id;
        let action = |action| Some(Message::Tab(id, action));
        let ws = self.workspace_of_tab(id);
        let folder = ws.map(|w| self.workspace_folder(w)).unwrap_or_default();
        let ws_cwd = ws.and_then(|w| self.model.workspace(w)?.cwd.clone());
        let pane = self.model.pane_of_tab(id).and_then(|p| self.model.pane(p));
        let at = pane.and_then(|p| p.tabs.iter().position(|t| *t == id));
        let closable = |range: &[TabId]| {
            range
                .iter()
                .any(|t| *t != id && self.model.tab(*t).is_some_and(|t| !t.pinned))
        };
        let (left, right, others) = match (pane, at) {
            (Some(p), Some(at)) => (
                closable(&p.tabs[..at]),
                closable(&p.tabs[at + 1..]),
                closable(&p.tabs),
            ),
            _ => (false, false, false),
        };
        let when = |on: bool, a| if on { action(a) } else { None };
        vec![
            pin_entry(tab.pinned, "Tab", |pin| action(TabAction::Pin(pin))),
            MenuEntry::Separator,
            MenuEntry::item(Icon::Pencil, "Rename Tab", action(TabAction::Rename)),
            MenuEntry::submenu(Icon::Move, "Move Tab", MenuPage::Move),
            MenuEntry::Separator,
            MenuEntry::submenu(Icon::Star, "Icon", MenuPage::Icon),
            MenuEntry::submenu(Icon::Palette, "Color", MenuPage::Color),
            MenuEntry::Separator,
            MenuEntry::Header(format!("Workspace folder: {}", short_path(&folder))),
            MenuEntry::item(
                Icon::FolderPlus,
                "Use as Workspace Folder",
                when(
                    tab.cwd
                        .as_deref()
                        .is_some_and(|cwd| !ws_cwd.as_deref().is_some_and(|f| same_dir(f, cwd))),
                    TabAction::UseAsFolder,
                ),
            ),
            MenuEntry::item(
                Icon::Folder,
                "Go to Workspace Folder",
                when(self.can_cd(id, &folder), TabAction::GoToFolder),
            ),
            MenuEntry::Separator,
            mute_entry(tab.muted, "Tab", |mute| action(TabAction::Mute(mute))),
            MenuEntry::item(Icon::Copy, "Copy Tab ID", action(TabAction::CopyId)),
            MenuEntry::Separator,
            MenuEntry::item(
                Icon::Close,
                "Close Other Tabs",
                when(others, TabAction::CloseOthers),
            ),
            MenuEntry::item(
                Icon::Back,
                "Close Tabs to the Left",
                when(left, TabAction::CloseLeft),
            ),
            MenuEntry::item(
                Icon::Forward,
                "Close Tabs to the Right",
                when(right, TabAction::CloseRight),
            ),
            MenuEntry::Separator,
            MenuEntry::item(Icon::Close, "Close Tab", action(TabAction::Close)),
        ]
    }

    /// A workspace's Combine submenu: every other workspace, whose panes would move into it.
    fn combine_entries(&self, ws: &Workspace) -> Vec<MenuEntry> {
        let mut entries = vec![MenuEntry::Header(format!("Move into {}", ws.name))];
        entries.extend(
            self.model
                .workspaces()
                .iter()
                .filter(|w| w.id != ws.id)
                .map(|w| {
                    MenuEntry::item(
                        Icon::Terminal,
                        w.name.clone(),
                        Some(Message::Workspace(ws.id, WorkspaceAction::Combine(w.id))),
                    )
                }),
        );
        entries
    }

    /// A workspace's Group submenu: a new group, the other groups, and out of its own.
    fn workspace_group_entries(&self, ws: &Workspace) -> Vec<MenuEntry> {
        let action = |action| Some(Message::Workspace(ws.id, action));
        let mut entries = vec![MenuEntry::item(
            Icon::FolderPlus,
            "New group",
            action(WorkspaceAction::NewGroup),
        )];
        for group in self.model.groups() {
            if Some(group.id) != ws.group {
                entries.push(MenuEntry::item(
                    Icon::Folder,
                    group.name.clone(),
                    action(WorkspaceAction::Group(Some(group.id))),
                ));
            }
        }
        if ws.group.is_some() {
            entries.push(MenuEntry::Separator);
            entries.push(MenuEntry::item(
                Icon::FolderMinus,
                "Remove from group",
                action(WorkspaceAction::Group(None)),
            ));
        }
        entries
    }

    /// A group's header menu on `page`; empty if the group is gone. Its Pin entry reads
    /// "Unpin" when any member is pinned.
    fn group_menu_entries(&self, id: GroupId, page: MenuPage) -> Vec<MenuEntry> {
        let Some(group) = self.model.group(id) else {
            return Vec::new();
        };
        let action = |action| Some(Message::Group(id, action));
        match page {
            MenuPage::Color => {
                color_entries(group.color.as_deref(), ColorTarget::Group(id), |color| {
                    action(GroupAction::Color(color.map(str::to_owned)))
                })
            }
            MenuPage::MembersColor => self.members_color_entries(id),
            MenuPage::Icon => vec![
                icon_grid(
                    group.icon.as_deref(),
                    workspace_color(group.color.as_deref()).unwrap_or(self.chrome.label),
                    |name| action(GroupAction::Icon(Some(name))),
                ),
                MenuEntry::Separator,
                more_icons(IconTarget::Group(id)),
                MenuEntry::item(
                    Icon::Folder,
                    "Default icon",
                    action(GroupAction::Icon(None)),
                ),
            ],
            MenuPage::Merge => {
                let mut entries = vec![MenuEntry::Header(format!("Move into {}", group.name))];
                entries.extend(self.model.groups().iter().filter(|g| g.id != id).map(|g| {
                    MenuEntry::item(
                        Icon::Folder,
                        g.name.clone(),
                        action(GroupAction::Merge(g.id)),
                    )
                }));
                entries
            }
            MenuPage::Delete => {
                let count = self
                    .model
                    .workspaces()
                    .iter()
                    .filter(|w| w.group == Some(id))
                    .count();
                vec![
                    MenuEntry::Header(if count == 1 {
                        "Close its workspace and its tabs?".to_owned()
                    } else {
                        format!("Close its {count} workspaces and their tabs?")
                    }),
                    MenuEntry::item(Icon::Close, "Delete Group", action(GroupAction::Delete)),
                    MenuEntry::item(Icon::Back, "Keep it", Some(Message::MenuClose)),
                ]
            }
            MenuPage::Move => {
                let (up, down) = self.block_room(|w| w.group == Some(id));
                move_entries(up, down, |shift| {
                    Message::Group(id, GroupAction::Shift(shift))
                })
            }
            _ => {
                let pinned = self
                    .model
                    .workspaces()
                    .iter()
                    .any(|w| w.group == Some(id) && w.pinned);
                group_main_entries(group, pinned, self.model.groups().len() > 1)
            }
        }
    }

    /// After a group's color is picked: whether to color its workspaces too.
    fn members_color_entries(&self, id: GroupId) -> Vec<MenuEntry> {
        let count = self
            .model
            .workspaces()
            .iter()
            .filter(|w| w.group == Some(id))
            .count();
        let (label, members) = if count == 1 {
            ("Color its workspace too?".to_owned(), "Color the workspace")
        } else {
            (
                format!("Color its {count} workspaces too?"),
                "Color the workspaces",
            )
        };
        vec![
            MenuEntry::Header(label),
            MenuEntry::item(
                Icon::Palette,
                members,
                Some(Message::Group(id, GroupAction::ColorMembers)),
            ),
            MenuEntry::item(Icon::Folder, "Just the group", Some(Message::MenuClose)),
        ]
    }

    /// Whether a block (a group, or a workspace outside groups) can move up and down: whether
    /// another block with the same pinned state is above or below it.
    fn block_room(&self, is_block: impl Fn(&Workspace) -> bool) -> (bool, bool) {
        // One entry per block: a group's members are contiguous, so only its first counts.
        // Pinned and unpinned blocks don't mix, so a block only moves past its own kind.
        let mut blocks: Vec<(bool, bool)> = Vec::new();
        let mut previous = None;
        for w in self.model.workspaces() {
            if w.group.is_some() && w.group == previous {
                continue;
            }
            previous = w.group;
            blocks.push((w.pinned, is_block(w)));
        }
        let Some(i) = blocks.iter().position(|(_, this)| *this) else {
            return (false, false);
        };
        let pinned = blocks[i].0;
        (
            blocks[..i].iter().any(|(p, _)| *p == pinned),
            blocks[i + 1..].iter().any(|(p, _)| *p == pinned),
        )
    }

    /// The new tab button's menu for `pane`: each installed shell (the first is the default),
    /// then a browser tab and, when VS Code's `code` was found, "Open in VS Code".
    fn new_tab_menu_entries(&self, pane: PaneId) -> Vec<MenuEntry> {
        let mut entries = vec![MenuEntry::Header("Terminal".to_owned())];
        entries.extend(self.shells.iter().enumerate().map(|(i, shell)| {
            let label = if i == 0 {
                format!("{} (default)", shell.name)
            } else {
                shell.name.clone()
            };
            MenuEntry::item(Icon::Terminal, label, Some(Message::NewTabWith(pane, i)))
        }));
        if !self.config.ssh.is_empty() {
            entries.push(MenuEntry::Separator);
            entries.push(MenuEntry::Header("SSH".to_owned()));
            entries.extend(self.config.ssh.iter().enumerate().map(|(i, profile)| {
                MenuEntry::item(
                    Icon::Server,
                    profile.label(),
                    Some(Message::NewSshTab(pane, i)),
                )
            }));
        }
        entries.push(MenuEntry::Separator);
        entries.push(MenuEntry::Header("Other".to_owned()));
        entries.push(MenuEntry::item(
            Icon::Globe,
            "Browser",
            Some(Message::NewBrowserIn(pane)),
        ));
        if self.vscode_web.installed {
            entries.push(MenuEntry::item(
                Icon::Code,
                "VS Code tab",
                Some(Message::VsCodeTab(pane)),
            ));
        }
        if self.vscode.is_some() {
            entries.push(MenuEntry::item(
                Icon::Code,
                "Open in VS Code",
                Some(Message::OpenVsCode(pane)),
            ));
        }
        entries
    }

    /// A tab's menu on `page`; empty if the tab is gone. The icon picker only marks a choice
    /// when the tab has its own icon, not its kind's default.
    fn tab_menu_entries(&self, tab: TabId, page: MenuPage) -> Vec<MenuEntry> {
        let c = self.chrome;
        let Some(tab) = self.model.tab(tab) else {
            return Vec::new();
        };
        let id = tab.id;
        let action = |action| Some(Message::Tab(id, action));
        match page {
            MenuPage::Icon => vec![
                icon_grid(
                    tab.icon.as_deref(),
                    workspace_color(tab.color.as_deref()).unwrap_or(c.label),
                    |name| action(TabAction::Icon(Some(name))),
                ),
                MenuEntry::Separator,
                more_icons(IconTarget::Tab(id)),
                MenuEntry::item(
                    tab_kind_icon(tab),
                    "Default icon",
                    action(TabAction::Icon(None)),
                ),
            ],
            MenuPage::Color => color_entries(tab.color.as_deref(), ColorTarget::Tab(id), |color| {
                action(TabAction::Color(color))
            }),
            MenuPage::Move => self.tab_move_entries(id),
            _ => self.tab_main_entries(tab),
        }
    }

    /// A tab's Move submenu: to a new workspace, a split of its pane, or another workspace.
    /// Moves that would change nothing are disabled: splitting a pane's only tab off itself,
    /// or a new workspace for the only tab of a single-pane workspace.
    fn tab_move_entries(&self, id: TabId) -> Vec<MenuEntry> {
        let pane = self.model.pane_of_tab(id);
        let alone = pane
            .and_then(|p| self.model.pane(p))
            .is_none_or(|p| p.tabs.len() == 1);
        let home = pane.and_then(|p| self.model.workspace_of_pane(p));
        let only_pane = home
            .and_then(|w| self.model.workspace(w))
            .is_some_and(|w| w.layout.leaves().len() == 1);
        let move_to = |to, enabled: bool| enabled.then_some(Message::Tab(id, TabAction::Move(to)));
        let split = |direction| {
            pane.and_then(|pane| move_to(MoveTo::SplitPane { pane, direction }, !alone))
        };
        let mut entries = vec![
            MenuEntry::item(
                Icon::Plus,
                "New workspace",
                move_to(MoveTo::NewWorkspace, !(alone && only_pane)),
            ),
            MenuEntry::item(Icon::Forward, "Split right", split(Direction::Right)),
            MenuEntry::item(Icon::Down, "Split down", split(Direction::Down)),
        ];
        let others: Vec<_> = self
            .model
            .workspaces()
            .iter()
            .filter(|w| Some(w.id) != home)
            .collect();
        if !others.is_empty() {
            entries.push(MenuEntry::Separator);
        }
        for ws in others {
            entries.push(MenuEntry::item(
                Icon::Terminal,
                ws.name.clone(),
                move_to(MoveTo::Workspace(ws.id), true),
            ));
        }
        entries
    }

    /// One of a menu's lists; `open` is the submenu showing, whose item stays highlighted.
    pub(super) fn menu_list(
        &self,
        target: MenuTarget,
        page: MenuPage,
        open: Option<MenuPage>,
    ) -> Element<'_, Message> {
        let c = self.chrome;
        let entries = self
            .menu_entries(target, page)
            .into_iter()
            .map(|entry| match entry {
                MenuEntry::Item {
                    icon,
                    label,
                    message,
                    submenu,
                } => {
                    let item = menu_item(c, icon, label, message, submenu, open);
                    // Only the main list's items open and close submenus.
                    if page == MenuPage::Main {
                        mouse_area(item)
                            .on_enter(Message::MenuHover(submenu))
                            .into()
                    } else {
                        item
                    }
                }
                MenuEntry::Separator => menu_separator(c, self.scale),
                MenuEntry::Header(label) => menu_header(c, label),
                MenuEntry::Grid(cells) => menu_grid(c, cells),
                MenuEntry::Palette(cells) => menu_palette(c, cells),
                MenuEntry::ColorInput(target) => self.color_input(target),
            });
        menu_card(c, column(entries).into())
    }

    /// The custom color field, with a preview of what's typed, or the no-color sign until it's
    /// a color.
    fn color_input(&self, target: ColorTarget) -> Element<'_, Message> {
        let c = self.chrome;
        let size = c.s.menus.picker_icon_size;
        let typed = custom_color(&self.color_input);
        let preview: Element<'_, Message> = match workspace_color(typed.as_deref()) {
            Some(color) => swatch(color, size),
            None => icon(Icon::Ban, size, c.faint).into(),
        };
        container(
            row![
                container(preview).center(c.s.menus.grid_cell),
                text_input("Custom  #RRGGBB", &self.color_input)
                    .on_input(Message::ColorInput)
                    .on_submit(Message::ColorSubmit(target))
                    .size(c.s.menus.text_size)
                    .padding(FIELD_PADDING)
                    .style(move |_, status| input_style(c, status)),
            ]
            .spacing(COLOR_INPUT_SPACING)
            .align_y(alignment::Vertical::Center),
        )
        .height(COLOR_INPUT_HEIGHT)
        .align_y(alignment::Vertical::Center)
        .into()
    }
}

/// A group's main menu: pin first; rename, move, merge, then collapse or expand; icon, color;
/// ungroup and delete. `pinned` is whether its workspaces are, `others` whether there's
/// another group to merge.
fn group_main_entries(group: &Group, pinned: bool, others: bool) -> Vec<MenuEntry> {
    let id = group.id;
    let action = |action| Some(Message::Group(id, action));
    vec![
        pin_entry(pinned, "Group", |pin| action(GroupAction::Pin(pin))),
        MenuEntry::Separator,
        MenuEntry::item(Icon::Pencil, "Rename Group", action(GroupAction::Rename)),
        MenuEntry::submenu(Icon::Move, "Move Group", MenuPage::Move),
        if others {
            MenuEntry::submenu(Icon::Combine, "Merge Groups", MenuPage::Merge)
        } else {
            MenuEntry::item(Icon::Combine, "Merge Groups", None)
        },
        if group.collapsed {
            MenuEntry::item(
                Icon::ChevronDown,
                "Expand Group",
                action(GroupAction::Collapse(false)),
            )
        } else {
            MenuEntry::item(
                Icon::ChevronRight,
                "Collapse Group",
                action(GroupAction::Collapse(true)),
            )
        },
        MenuEntry::Separator,
        MenuEntry::submenu(Icon::Star, "Icon", MenuPage::Icon),
        MenuEntry::submenu(Icon::Palette, "Color", MenuPage::Color),
        MenuEntry::Separator,
        MenuEntry::item(Icon::FolderMinus, "Ungroup", action(GroupAction::Ungroup)),
        MenuEntry::submenu(Icon::Close, "Delete Group", MenuPage::Delete),
    ]
}

/// "Pin `noun`", or "Unpin `noun`" when it's `pinned`; `pin` is the message for either.
fn pin_entry(pinned: bool, noun: &str, pin: impl Fn(bool) -> Option<Message>) -> MenuEntry {
    if pinned {
        MenuEntry::item(Icon::PinOff, format!("Unpin {noun}"), pin(false))
    } else {
        MenuEntry::item(Icon::Pin, format!("Pin {noun}"), pin(true))
    }
}

/// "Mute `noun`", or "Unmute `noun`" when it's `muted`; `mute` is the message for either.
fn mute_entry(muted: bool, noun: &str, mute: impl Fn(bool) -> Option<Message>) -> MenuEntry {
    if muted {
        MenuEntry::item(Icon::Bell, format!("Unmute {noun}"), mute(false))
    } else {
        MenuEntry::item(Icon::BellOff, format!("Mute {noun}"), mute(true))
    }
}

/// A Move submenu: up and down by one, and to either end, where there's room (`up`, `down`).
fn move_entries(up: bool, down: bool, shift: impl Fn(Shift) -> Message) -> Vec<MenuEntry> {
    let shift = |to, enabled: bool| enabled.then(|| shift(to));
    vec![
        MenuEntry::item(Icon::Up, "Move up", shift(Shift::Up, up)),
        MenuEntry::item(Icon::Down, "Move down", shift(Shift::Down, down)),
        MenuEntry::item(Icon::Up, "Move to top", shift(Shift::Top, up)),
        MenuEntry::item(Icon::Down, "Move to bottom", shift(Shift::Bottom, down)),
    ]
}

/// "More Icons…": opens the icon picker for `target`, with the whole library and the user's
/// own icons.
fn more_icons(target: IconTarget) -> MenuEntry {
    MenuEntry::item(
        Icon::Search,
        "More Icons\u{2026}",
        Some(Message::IconPicker(IconPickerMessage::Open(target))),
    )
}

/// The icon picker, every icon in `tint`, with the one saved as `current` (if any) selected.
fn icon_grid(
    current: Option<&str>,
    tint: Color,
    pick: impl Fn(&'static str) -> Option<Message>,
) -> MenuEntry {
    MenuEntry::Grid(
        WORKSPACE_ICONS
            .iter()
            .map(|&(name, glyph)| GridCell {
                kind: CellKind::Icon(glyph, tint),
                selected: current == Some(name),
                message: pick(name),
            })
            .collect(),
    )
}

/// The color picker: no color and each of [`WORKSPACE_COLORS`], then the [`PALETTE`] (a column
/// per hue, a row per brightness), then a field for a custom one. `current` is the color's
/// saved name or `#rrggbb`; `pick` is the message for each choice.
fn color_entries(
    current: Option<&str>,
    target: ColorTarget,
    pick: impl Fn(Option<&'static str>) -> Option<Message>,
) -> Vec<MenuEntry> {
    let colors = std::iter::once(GridCell {
        kind: CellKind::Swatch(None),
        selected: current.is_none(),
        message: pick(None),
    })
    .chain(WORKSPACE_COLORS.iter().map(|&(name, value)| GridCell {
        kind: CellKind::Swatch(Some(hex(value))),
        selected: current == Some(name),
        message: pick(Some(name)),
    }))
    .collect();
    let same = |hex: &str| current.is_some_and(|c| c.eq_ignore_ascii_case(hex));
    // Row by row: each row is one brightness across every hue.
    let palette = (0..PALETTE[0].len())
        .flat_map(|shade| PALETTE.iter().map(move |hue| hue[shade]))
        .map(|value| GridCell {
            kind: CellKind::Swatch(workspace_color(Some(value))),
            selected: same(value),
            message: pick(Some(value)),
        })
        .collect();
    vec![
        MenuEntry::Grid(colors),
        MenuEntry::Separator,
        MenuEntry::Palette(palette),
        MenuEntry::Separator,
        MenuEntry::ColorInput(target),
    ]
}

/// An item: its icon and label, dimmed without a `message`, with a chevron if it opens a
/// `submenu`, highlighted while that submenu is `open`.
fn menu_item<'a>(
    c: Chrome,
    glyph: Icon,
    label: String,
    message: Option<Message>,
    submenu: Option<MenuPage>,
    open: Option<MenuPage>,
) -> Element<'a, Message> {
    let color = if message.is_some() { c.text } else { c.faint };
    let mut content = row![
        icon(
            glyph,
            c.s.menus.icon_size,
            if message.is_some() { c.label } else { c.faint }
        ),
        one_line(label, c.s.menus.text_size, color).width(Fill),
    ]
    .spacing(c.s.menus.spacing)
    // The button's fixed height would otherwise leave the row at its top.
    .height(Fill)
    .align_y(alignment::Vertical::Center);
    if submenu.is_some() {
        content = content.push(icon(
            Icon::ChevronRight,
            c.s.menus.icon_size - CHEVRON_SHRINK,
            c.muted,
        ));
    }
    let open = submenu.is_some() && submenu == open;
    button(content)
        .on_press_maybe(message)
        .style(move |_, status| {
            let status = if open {
                button::Status::Hovered
            } else {
                status
            };
            flat_button(c, status, None)
        })
        .width(Fill)
        .height(c.s.menus.item_height)
        .padding([0.0, c.s.menus.item_padding])
        .into()
}

/// A thin line across the menu between groups of items, `style.menus.separator_height` tall.
fn menu_separator<'a>(c: Chrome, scale: f32) -> Element<'a, Message> {
    container(HairLine::new(c.item_edge, scale))
        .height(c.s.menus.separator_height)
        .padding(SEPARATOR_PADDING)
        .into()
}

/// A small faint label over the items after it, `style.menus.header_height` tall.
fn menu_header<'a>(c: Chrome, label: String) -> Element<'a, Message> {
    container(one_line(label, c.s.menus.header_size, c.faint))
        .height(c.s.menus.header_height)
        .padding([0.0, c.s.menus.item_padding])
        .align_y(alignment::Vertical::Center)
        .into()
}

/// A picker's cells, [`GRID_COLUMNS`] to a row.
fn menu_grid<'a>(c: Chrome, cells: Vec<GridCell>) -> Element<'a, Message> {
    let mut rows = Vec::new();
    let mut cells = cells.into_iter().peekable();
    while cells.peek().is_some() {
        let mut row_cells: Vec<Element<'_, Message>> = cells
            .by_ref()
            .take(GRID_COLUMNS)
            .map(|cell| grid_cell(c, cell, c.s.menus.grid_cell, c.s.menus.picker_icon_size))
            .collect();
        // Keep a short last row's cells the same width as the others.
        while row_cells.len() < GRID_COLUMNS {
            row_cells.push(Space::new().width(Fill).into());
        }
        rows.push(row(row_cells).spacing(GRID_SPACING).into());
    }
    column(rows).spacing(GRID_SPACING).into()
}

/// The color palette: rows of [`PALETTE_COLUMNS`] small swatches.
fn menu_palette<'a>(c: Chrome, cells: Vec<GridCell>) -> Element<'a, Message> {
    let mut rows = Vec::new();
    let mut cells = cells.into_iter().peekable();
    while cells.peek().is_some() {
        let row_cells: Vec<Element<'_, Message>> = cells
            .by_ref()
            .take(PALETTE_COLUMNS)
            .map(|cell| grid_cell(c, cell, PALETTE_CELL, PALETTE_SWATCH))
            .collect();
        rows.push(row(row_cells).spacing(PALETTE_SPACING).into());
    }
    column(rows).spacing(PALETTE_SPACING).into()
}

/// A menu list's popup card around its `entries`.
fn menu_card<'a>(c: Chrome, entries: Element<'a, Message>) -> Element<'a, Message> {
    let list = container(entries)
        .width(c.s.menus.width)
        .padding(c.s.menus.padding)
        .style(move |_| container::Style {
            background: Some(c.popup.into()),
            border: Border {
                color: if c.glass.is_some() {
                    Color::TRANSPARENT
                } else {
                    c.item_edge
                },
                width: c.s.menus.border_width,
                radius: c.s.menus.radius.into(),
            },
            shadow: shadow(
                c.menu_shadow,
                c.s.menus.shadow_offset,
                c.s.menus.shadow_blur,
            ),
            ..container::Style::default()
        });
    // A plain sheet, not frosted: the blur only sees the backdrop, not the UI under a popup.
    glass::sheet(c, list, c.s.menus.radius, 1.0)
}

/// A round swatch of `color`, `size` across.
fn swatch<'a>(color: Color, size: f32) -> Element<'a, Message> {
    container(Space::new())
        .width(size)
        .height(size)
        .style(move |_| card_style(color, size / 2.0))
        .into()
}

/// One row of a menu list. Every kind has a height known without layout
/// ([`MenuEntry::height`]), which [`App::menu_bounds`] relies on.
enum MenuEntry {
    /// A clickable row: an icon and a label, plus a chevron when it opens a submenu.
    Item {
        /// Drawn before the label.
        icon: Icon,
        /// The item's text.
        label: String,
        /// `None` shows the item disabled.
        message: Option<Message>,
        /// Opens this list beside the menu rather than acting.
        submenu: Option<MenuPage>,
    },
    /// A line between groups of items.
    Separator,
    /// A small label over the items after it.
    Header(String),
    /// A picker: cells [`GRID_COLUMNS`] to a row.
    Grid(Vec<GridCell>),
    /// Swatches in [`PALETTE_COLUMNS`] smaller columns, a row per brightness.
    Palette(Vec<GridCell>),
    /// A field for a custom `#rrggbb` color for the workspace, group or tab.
    ColorInput(ColorTarget),
}

impl MenuEntry {
    /// A plain item that sends `message` when clicked (disabled when `None`).
    fn item(icon: Icon, label: impl Into<String>, message: Option<Message>) -> Self {
        Self::Item {
            icon,
            label: label.into(),
            message,
            submenu: None,
        }
    }

    /// An item that opens `page` beside the menu: hovering opens it, clicking pins it open
    /// ([`Message::MenuPin`]).
    fn submenu(icon: Icon, label: &str, page: MenuPage) -> Self {
        Self::Item {
            icon,
            label: label.into(),
            message: Some(Message::MenuPin(page)),
            submenu: Some(page),
        }
    }

    /// How tall the entry draws, from the same style values its widget uses, so menus can be
    /// placed before they're laid out.
    fn height(&self, c: Chrome) -> f32 {
        match self {
            Self::Item { .. } => c.s.menus.item_height,
            Self::Separator => c.s.menus.separator_height,
            Self::Header(_) => c.s.menus.header_height,
            Self::Grid(cells) => {
                let rows = cells.len().div_ceil(GRID_COLUMNS) as f32;
                rows * c.s.menus.grid_cell + (rows - 1.0).max(0.0) * GRID_SPACING
            }
            Self::Palette(cells) => {
                let rows = cells.len().div_ceil(PALETTE_COLUMNS) as f32;
                rows * PALETTE_CELL + (rows - 1.0).max(0.0) * PALETTE_SPACING
            }
            Self::ColorInput(_) => COLOR_INPUT_HEIGHT,
        }
    }
}

/// One cell of a picker grid.
struct GridCell {
    /// What the cell shows.
    kind: CellKind,
    /// Whether it's the current choice, drawn raised and outlined.
    selected: bool,
    /// Sent when it's clicked; `None` disables it.
    message: Option<Message>,
}

/// What a picker cell shows.
enum CellKind {
    /// An icon in the given tint.
    Icon(Icon, Color),
    /// A color to pick, or `None` for no color.
    Swatch(Option<Color>),
}

/// A picker cell as a button `height` tall, filling its column, its icon or swatch `size`.
fn grid_cell<'a>(c: Chrome, cell: GridCell, height: f32, size: f32) -> Element<'a, Message> {
    let content: Element<'a, Message> = match cell.kind {
        CellKind::Icon(glyph, color) => icon(glyph, size, color).into(),
        CellKind::Swatch(Some(color)) => swatch(color, size),
        CellKind::Swatch(None) => icon(Icon::Ban, size, c.label).into(),
    };
    let selected = cell.selected;
    button(container(content).center(Fill))
        .on_press_maybe(cell.message)
        .width(Fill)
        .height(height)
        .padding(0)
        .style(move |_, status| {
            let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: (selected || hover).then_some(Background::Color(if selected {
                    c.raised
                } else {
                    c.hover
                })),
                border: Border {
                    color: if selected {
                        c.outline
                    } else {
                        Color::TRANSPARENT
                    },
                    width: c.s.controls.border_width,
                    radius: c.s.controls.radius.into(),
                },
                ..button::Style::default()
            }
        })
        .into()
}

/// `path` with the home directory shown as `~`, for menu headings.
fn short_path(path: &str) -> String {
    let Some(home) = crate::home_dir() else {
        return path.to_owned();
    };
    let home = home.to_string_lossy().replace('\\', "/");
    let unix = path.replace('\\', "/");
    let lower = |s: &str| {
        if cfg!(windows) {
            s.to_lowercase()
        } else {
            s.to_owned()
        }
    };
    match lower(&unix).strip_prefix(&lower(&home)) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            format!("~{}", &unix[home.len()..])
        }
        _ => unix,
    }
}

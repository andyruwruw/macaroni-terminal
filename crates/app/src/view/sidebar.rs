//! The workspace sidebar: group headers, workspace items, "New workspace" and (where the
//! system draws the title bar) the Settings button.
//!
//! Each header and item is a `mouse_area` wrapped in [`Pointer`] (which reports the half of the
//! item under the cursor, and gap exits, as [`Message::SidebarZone`]) and [`RightClick`] (which
//! opens its menu at the cursor). Those hover reports, not geometry, are what
//! [`App::workspace_drop`] and [`App::drop_target`] use for targets in the sidebar; the
//! insertion line comes from [`App::workspace_line`] and is drawn with [`line_above`]. What an
//! item shows comes from the `[workspace_items]` templates, filled from [`WorkspaceValues`].

use iced::widget::{
    Space, button, column, container, mouse_area, pin, row, scrollable, stack, text, text_input,
    tooltip,
};
use iced::{Border, Color, Element, Fill, alignment, mouse};
use macaroni_core::{Agent, AgentState, Command, Group, GroupId, Tab, Workspace, WorkspaceId};

use super::drag::{GROUPED_LINE_INSET, LineAbove, line_above};
use super::tabs::{strip_scrollbar, tab_dir, tab_title};
use super::widgets::{
    AddBox, LINE_HEIGHT, MARK_OVER_TEXT, add_box, card_style, faded_icon_button, flat_button,
    glass_strength, icon_button, icon_tile, input_style, item_ease, item_fill, one_line, truncate,
};
use super::{CUSTOM_FRAME, RENAME_INPUT};
use crate::anim::mix;
use crate::icons::{Glyph, Icon, glyph as draw_glyph, icon, named_glyph, workspace_icon};
use crate::keymap::Action;
use crate::pointer::{Halves, Pointer};
use crate::right_click::RightClick;
use crate::side_line::SideLine;
use crate::theme::{Chrome, workspace_color};
use crate::{
    App, Dragged, DropTarget, GroupAction, MenuTarget, Message, Renaming, SidebarHover, anim,
    glass, label,
};

/// The Settings button at the foot of the sidebar, where the system draws the title bar: its
/// icon size.
const SETTINGS_ICON_SIZE: f32 = 16.0;

/// The Settings button's text size.
const SETTINGS_TEXT_SIZE: f32 = 13.0;

/// Between the Settings button's icon and text.
const SETTINGS_SPACING: f32 = 10.0;

/// Inside the Settings button, above and below then left and right.
const SETTINGS_PADDING: [f32; 2] = [8.0, 16.0];

/// A group header's padding left and right of its contents (and of its name field).
const GROUP_PADDING_X: f32 = 10.0;

/// Between a workspace's muted and pinned marks.
const MARK_SPACING: f32 = 4.0;

/// Narrower than this, workspaces keep their cards but show only the first letter of their
/// title (see [`SidebarSize`]).
const NARROW_BELOW: f32 = 150.0;

/// How wide the sidebar is drawn: three sizes, by its width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SidebarSize {
    /// Everything `[workspace_items]` asks for, and the search field.
    Full,
    /// The cards stay, with the icon tile and the title's first letter, capitalized; no detail,
    /// marks, badge, close button or search.
    Narrow,
    /// Too narrow for a card: icons only (`compact_workspace_item`).
    Tiny,
}

/// The narrowest a [`SidebarSize::Narrow`] card fits: the sidebar's and the card's padding
/// around an icon tile and one letter, plus a resting glass sheet's inset.
fn narrow_width(c: Chrome) -> f32 {
    let inset = glass::lift_padding(c, iced::Padding::ZERO, 0.0).0;
    compact_width(c)
        + c.s.workspaces.padding * 2.0
        + c.s.workspaces.spacing
        + c.s.workspaces.title_size
        + inset.left
        + inset.right
}

/// The scrolling list of workspaces.
const SIDEBAR_SCROLL: &str = "sidebar-workspaces";

/// The widget id of the sidebar's search field.
pub const SIDEBAR_SEARCH: &str = "sidebar-search";

/// The search field's icons' size.
const SEARCH_ICON_SIZE: f32 = 12.0;

/// How far the glass search bar's color is mixed toward black, past the pane cards'.
const SEARCH_DARKEN: f32 = 0.06;

/// Between the search field's icon, text and clear button.
const SEARCH_SPACING: f32 = 6.0;

/// The sidebar scrollbar's margin from the sidebar's right edge.
const SIDEBAR_SCROLLBAR_MARGIN: f32 = 2.0;

/// The width of the strip at the sidebar's right edge that drags its width.
const HANDLE_WIDTH: f32 = 6.0;

/// The line beside a group's workspaces: its width.
const GROUP_LINE_WIDTH: f32 = 1.0;

/// How strongly the line beside a group's workspaces shows (of `Chrome.outline`).
const GROUP_LINE_ALPHA: f32 = 0.6;

/// A compact workspace's close button: its icon size, and the padding around it.
const COMPACT_CLOSE: (f32, f32) = (8.0, 3.0);

/// The narrowest the sidebar can be dragged without folding away: one icon tile and its
/// padding.
pub fn compact_width(c: Chrome) -> f32 {
    c.s.workspaces.tile_size + c.s.sidebar.padding * 2.0
}

/// A workspace item's height: its padding around [`workspace_content_height`].
fn workspace_item_height(c: Chrome) -> f32 {
    c.s.workspaces.padding * 2.0 + workspace_content_height(c)
}

/// How tall a workspace item's contents are: the icon tile, or the title and detail lines if
/// they're taller. Every row gets this height whether its detail shows or not, so a row doesn't
/// grow when its agent starts working and drop targets can count on one row height.
fn workspace_content_height(c: Chrome) -> f32 {
    let w = &c.s.workspaces;
    let lines = (w.title_size + w.detail_size) * LINE_HEIGHT + w.line_spacing;
    w.tile_size.max(lines.ceil())
}

impl App {
    /// The workspaces under their group headers, "New workspace", and (where the system draws
    /// the title bar) the Settings button.
    pub(super) fn sidebar(&self) -> Element<'_, Message> {
        let c = self.chrome;
        if self.sidebar.collapsed {
            return sidebar_rail(c);
        }
        let line = self.workspace_line();
        let compact = self.sidebar_compact();
        let pad = c.s.sidebar.padding;
        // The sidebar's padding (but the bottom) is inside the scrolling part, so the viewport
        // spans the whole sidebar: lifted glass items, their shadows and the insertion line above
        // the first one aren't clipped, and the scrollbar floats in the right padding instead of
        // over the items.
        let searching = self.sidebar_searching();
        let list = self.sidebar_list(self.sidebar_items(line), line, searching);
        let list = scrollable(list)
            .id(SIDEBAR_SCROLL)
            .direction(scrollable::Direction::Vertical(
                scrollable::Scrollbar::new()
                    .width(c.s.tabs.scrollbar_width)
                    .scroller_width(c.s.tabs.scrollbar_width)
                    .margin(SIDEBAR_SCROLLBAR_MARGIN),
            ))
            .width(Fill)
            .height(Fill)
            .style(move |theme, status| strip_scrollbar(c, theme, status));
        let search = searching.then(|| self.sidebar_search(pad));
        let content = container(
            column![]
                .push(search)
                .push(list)
                // Windows has it in the title bar.
                .push((!CUSTOM_FRAME).then(|| {
                    container(
                        column![
                            sidebar_separator(c),
                            if compact {
                                icon_button(
                                    c,
                                    Icon::Settings,
                                    Message::Action(Action::OpenSettings),
                                )
                            } else {
                                settings_button(c)
                            },
                        ]
                        .spacing(c.s.sidebar.new_workspace_gap),
                    )
                    .padding([0.0, pad])
                })),
        )
        .padding(iced::Padding {
            // The list ends in the bottom padding; below the Settings button it's needed again.
            bottom: if CUSTOM_FRAME {
                0.0
            } else {
                c.s.sidebar.bottom_padding
            },
            ..iced::Padding::ZERO
        })
        .width(c.s.window.sidebar_width)
        .height(Fill)
        .style(move |_| card_style(c.sidebar, 0.0));
        // Over the sidebar's last few pixels, so dragging it never moves the panes' origin.
        let dragging = self.sidebar_drag;
        let handle = mouse_area(
            container(Space::new().width(HANDLE_WIDTH).height(Fill)).style(move |_| {
                card_style(
                    if dragging {
                        c.outline
                    } else {
                        Color::TRANSPARENT
                    },
                    0.0,
                )
            }),
        )
        .on_press(Message::SidebarDragStart)
        .interaction(mouse::Interaction::ResizingHorizontally);
        stack![
            content,
            container(handle)
                .width(Fill)
                .height(Fill)
                .align_x(alignment::Horizontal::Right),
        ]
        .width(c.s.window.sidebar_width)
        .height(Fill)
        .into()
    }

    /// Which of the three sizes the sidebar's width calls for.
    pub(super) fn sidebar_size(&self) -> SidebarSize {
        let width = self.chrome.s.window.sidebar_width;
        if width < narrow_width(self.chrome) {
            SidebarSize::Tiny
        } else if width < NARROW_BELOW {
            SidebarSize::Narrow
        } else {
            SidebarSize::Full
        }
    }

    /// Whether the sidebar shows its search field: wide enough, and `workspace_items.search`.
    pub(super) fn sidebar_searching(&self) -> bool {
        self.sidebar_size() == SidebarSize::Full
            && self.config.workspace_items.search(self.config.ui_theme)
    }

    /// Whether the sidebar is narrow enough to show workspaces as icons only.
    pub(super) fn sidebar_compact(&self) -> bool {
        self.sidebar_size() == SidebarSize::Tiny
    }

    /// Every group header and workspace, in order, with the insertion `line` for a dragged one.
    pub(super) fn sidebar_items(
        &self,
        line: Option<(LineAbove, bool)>,
    ) -> Vec<Element<'_, Message>> {
        let c = self.chrome;
        let compact = self.sidebar_compact();
        // Without the field nothing filters, whatever was typed in it before.
        let query = if self.sidebar_searching() {
            self.sidebar_query.trim()
        } else {
            ""
        };
        let mut items: Vec<Element<'_, Message>> = Vec::new();
        // The open group's members so far, drawn as one indented block beside a line.
        let mut members: Vec<Element<'_, Message>> = Vec::new();
        let mut previous = None;
        // A group's members are contiguous in the model's order, so a header goes before the
        // first workspace whose group differs from the one before it.
        for (i, ws) in self.model.workspaces().iter().enumerate() {
            if !self.workspace_matches(ws, query) {
                continue;
            }
            let group = ws.group.and_then(|g| self.model.group(g));
            if ws.group != previous {
                if !members.is_empty() {
                    items.push(group_block(c, std::mem::take(&mut members)));
                }
                previous = ws.group;
                if let Some(group) = group {
                    items.push(line_above(
                        c,
                        self.group_header(group),
                        line_inset(c, line, LineAbove::Group(group.id)),
                        c.s.sidebar.spacing,
                        0.0,
                    ));
                }
            }
            let item = || {
                line_above(
                    c,
                    if compact {
                        self.compact_workspace_item(ws)
                    } else {
                        self.workspace_item(i, ws)
                    },
                    // Grouped workspaces are already indented.
                    line_inset(c, line, LineAbove::Workspace(ws.id)).map(|inset| {
                        if ws.group.is_some() {
                            inset - group_inset(c)
                        } else {
                            inset
                        }
                    }),
                    c.s.sidebar.spacing,
                    // A resting glass item sits this far inside its bounds (`lift_padding`'s
                    // outer top), so the line centers on the visible gap, not the layout gap.
                    // Headers and "New workspace" don't lift, so theirs is 0.
                    glass::lift_padding(c, iced::Padding::ZERO, 0.0).0.top,
                )
            };
            match group {
                // A search shows matches in collapsed groups too.
                Some(group) if group.collapsed && query.is_empty() => {}
                // Icons only, there's no room to indent.
                Some(_) if compact => items.push(item()),
                Some(_) => members.push(item()),
                None => items.push(item()),
            }
        }
        if !members.is_empty() {
            items.push(group_block(c, members));
        }
        if items.is_empty() && !query.is_empty() {
            items.push(
                container(one_line(
                    "No workspaces match",
                    c.s.groups.name_size,
                    c.faint,
                ))
                .padding([c.s.groups.padding, GROUP_PADDING_X])
                .into(),
            );
        }
        items
    }

    /// Whether `ws` matches the sidebar search `query` (trimmed; empty matches everything): its
    /// name, its folder, or any of its tabs' titles or directories contain it, ignoring case.
    pub(crate) fn workspace_matches(&self, ws: &Workspace, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        let has = |s: &str| s.to_lowercase().contains(&query);
        has(&ws.name)
            || ws.cwd.as_deref().is_some_and(has)
            || ws
                .layout
                .leaves()
                .iter()
                .filter_map(|p| self.model.pane(*p))
                .flat_map(|p| p.tabs.iter().filter_map(|t| self.model.tab(*t)))
                .any(|t| has(&tab_title(t)) || t.cwd.as_deref().is_some_and(has))
    }

    /// The sidebar's rows (`items`) and the New workspace button under them, padded as in the
    /// sidebar: below the search field when `searching`, else `style.sidebar.top_padding` below
    /// the title bar or the window's top. The settings dialog's screen preview lays out its few rows with it too.
    pub(super) fn sidebar_list<'a>(
        &'a self,
        items: Vec<Element<'a, Message>>,
        line: Option<(LineAbove, bool)>,
        searching: bool,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let pad = c.s.sidebar.padding;
        container(
            column![
                column(items).spacing(c.s.sidebar.spacing),
                self.new_workspace_button(line),
            ]
            .spacing(c.s.sidebar.new_workspace_gap),
        )
        .padding(iced::Padding {
            // Under the search bar, a resting glass item already sits its inset below its slot's
            // top, so that comes off `search_gap` to keep the visible gap what's set.
            top: if searching {
                (c.s.sidebar.search_gap - glass::lift_padding(c, iced::Padding::ZERO, 0.0).0.top)
                    .max(0.0)
            } else {
                c.s.sidebar.top_padding
            },
            bottom: c.s.sidebar.bottom_padding,
            ..iced::Padding::new(pad)
        })
        .into()
    }

    /// The search field at the top of the sidebar, `pad` in from its sides and
    /// `style.sidebar.top_padding` down: a search icon, the field, and while something's typed
    /// an x that empties it. Its slot is `style.sidebar.search_height` tall and, like a resting
    /// glass tab in its slot, the bar sits `lift_padding`'s outer inset inside it, so with the
    /// default sizes it lines up with the first tab strip beside it.
    pub(super) fn sidebar_search(&self, pad: f32) -> Element<'_, Message> {
        let c = self.chrome;
        let size = c.s.groups.name_size;
        let field = text_input("Search workspaces", &self.sidebar_query)
            .id(SIDEBAR_SEARCH)
            .on_input(Message::SidebarSearch)
            .on_submit(Message::SidebarSearchSubmit)
            .size(size)
            .padding(0)
            .style(move |_, status| text_input::Style {
                background: Color::TRANSPARENT.into(),
                border: Border::default(),
                ..input_style(c, status)
            });
        let clear = (!self.sidebar_query.is_empty()).then(|| {
            button(icon(Icon::Close, SEARCH_ICON_SIZE, c.muted))
                .on_press(Message::SidebarSearchClear)
                .style(move |_, status| flat_button(c, status, None))
                .padding(2)
        });
        let focused = self.sidebar_search_focused;
        let rest = glass::lift_padding(c, iced::Padding::ZERO, 0.0).0;
        let bar = container(
            row![icon(Icon::Search, SEARCH_ICON_SIZE, c.muted), field]
                .push(clear)
                .spacing(SEARCH_SPACING)
                .align_y(alignment::Vertical::Center),
        )
        .padding([0.0, c.s.sidebar.search_padding])
        .width(Fill)
        .height((c.s.sidebar.search_height - rest.top - rest.bottom).max(0.0))
        .align_y(alignment::Vertical::Center)
        .style(move |_| container::Style {
            // Glass: the pane cards' color, a shade darker, on their frosted sheet, below.
            // Solid: a field.
            background: Some(
                if c.glass.is_some() {
                    mix(c.card, Color::BLACK, SEARCH_DARKEN)
                } else {
                    c.search
                }
                .into(),
            ),
            border: Border {
                color: if focused {
                    c.outline
                } else {
                    Color::TRANSPARENT
                },
                width: c.s.controls.border_width,
                radius: c.s.controls.radius.into(),
            },
            ..container::Style::default()
        });
        // Sized as the window, not the bar: a panel's fill dims as it grows, and a bar-sized
        // one would be milky where the pane cards beside it are clear. Its rim is as quiet as a
        // resting sidebar item's, so the shine doesn't outshine the workspaces.
        let bar = glass::quiet_panel(
            c,
            bar,
            c.s.controls.radius,
            self.window_size,
            c.s.effects.glass_idle,
        );
        container(bar)
            .padding(iced::Padding {
                top: c.s.sidebar.top_padding + rest.top,
                left: pad,
                right: pad,
                bottom: 0.0,
            })
            .into()
    }

    /// The `+` box after the workspaces, a drop target for tabs and workspaces too.
    fn new_workspace_button(&self, line: Option<(LineAbove, bool)>) -> Element<'_, Message> {
        let c = self.chrome;
        line_above(
            c,
            mouse_area(add_box(
                c,
                AddBox::Workspace,
                Fill,
                if self.sidebar_compact() {
                    c.s.workspaces.tile_size
                } else {
                    workspace_item_height(c)
                },
                Message::Command(Command::NewWorkspace),
            ))
            .on_enter(Message::SidebarHover(SidebarHover::NewWorkspace, true))
            .on_exit(Message::SidebarHover(SidebarHover::NewWorkspace, false))
            .into(),
            line_inset(c, line, LineAbove::End),
            c.s.sidebar.new_workspace_gap,
            0.0,
        )
    }

    /// A group's header: click to collapse or expand, right-click for its menu.
    fn group_header<'a>(&'a self, group: &'a Group) -> Element<'a, Message> {
        let c = self.chrome;
        let id = group.id;
        if let Some((Renaming::Group(renaming), value)) = &self.renaming
            && *renaming == id
        {
            return group_rename_field(c, id, value);
        }
        let members: Vec<&Workspace> = self
            .model
            .workspaces()
            .iter()
            .filter(|w| w.group == Some(id))
            .collect();
        let holds_active = members
            .iter()
            .any(|w| w.id == self.model.active_workspace().id);
        // Collapsed, the header stands in for the workspaces it hides.
        let lit = group.collapsed && holds_active;
        // Anything dropped into the group marks it, not just a drop onto the header.
        let drop_here = self.workspace_drop().is_some_and(|d| d.group == Some(id));
        let content = if self.sidebar_compact() {
            // Just the chevron, in the group's color; the name shows on hover.
            let mark = draw_glyph(
                group_glyph(group),
                c.s.groups.chevron_size,
                workspace_color(group.color.as_deref()).unwrap_or(c.label),
                1.0,
            );
            tooltip(
                container(mark).center_x(Fill),
                name_tip(c, group.name.clone()),
                tooltip::Position::Right,
            )
            .into()
        } else {
            self.group_header_row(group, &members, lit)
        };
        let header = group_header_card(c, content, lit, drop_here, self.sidebar_compact());
        let hover = SidebarHover::Group(id);
        let focus = Renaming::Group(id);
        RightClick::new(
            Pointer::new(
                mouse_area(header)
                    .on_press(Message::Group(id, GroupAction::Collapse(!group.collapsed)))
                    .on_enter(Message::SidebarHover(hover, true))
                    .on_exit(Message::SidebarHover(hover, false))
                    .interaction(mouse::Interaction::Pointer),
            )
            .on_zone(Halves::TopBottom, move |zone| {
                Message::SidebarZone(hover, zone)
            })
            .on_press_outside_maybe(
                (self.item_focus == Some(focus)).then_some(Message::ItemBlur(focus)),
            ),
            move |at| Message::MenuOpen(MenuTarget::Group(id), at),
        )
        .into()
    }

    /// What a group's header shows: its icon (a folder unless it has its own) in its color,
    /// its name, a bell while any of its `members` waits, a pin if any is pinned, and the
    /// chevron at the right (never tinted). `lit` shows the name brighter.
    fn group_header_row<'a>(
        &self,
        group: &'a Group,
        members: &[&Workspace],
        lit: bool,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let waiting = members
            .iter()
            .any(|w| self.model.workspace_notice(w.id).is_some());
        let mark_size = c.s.groups.count_size + MARK_OVER_TEXT;
        row![]
            .push(draw_glyph(
                group_glyph(group),
                c.s.groups.chevron_size,
                workspace_color(group.color.as_deref()).unwrap_or(c.label),
                1.0,
            ))
            .push(
                one_line(
                    group.name.as_str(),
                    c.s.groups.name_size,
                    if lit { c.text } else { c.label },
                )
                .width(Fill),
            )
            .push(waiting.then(|| icon(Icon::Bell, mark_size, c.attention)))
            .push(
                members
                    .iter()
                    .any(|w| w.pinned)
                    .then(|| icon(Icon::Pin, mark_size, c.faint)),
            )
            .push(icon(
                if group.collapsed {
                    Icon::ChevronRight
                } else {
                    Icon::ChevronDown
                },
                c.s.groups.chevron_size,
                c.muted,
            ))
            .spacing(c.s.groups.spacing)
            .align_y(alignment::Vertical::Center)
            .into()
    }

    /// A workspace, drawn like a pane's tabs: icon, name, a detail line and a close button.
    fn workspace_item<'a>(&'a self, index: usize, ws: &'a Workspace) -> Element<'a, Message> {
        let c = self.chrome;
        if let Some((Renaming::Workspace(id), value)) = &self.renaming
            && *id == ws.id
        {
            return workspace_rename_field(c, *id, value);
        }
        let levels = self.fades.get(anim::Item::Workspace(ws.id));
        let ghost = if self.dragging(Dragged::Workspace(ws.id)) {
            c.s.effects.dragged
        } else {
            1.0
        };
        // Text dims by alpha; icons by opacity, since iced's svg tint ignores alpha.
        let fade = (c.s.effects.dim + (1.0 - c.s.effects.dim) * levels.emphasis(c.s.effects.hover))
            * ghost;
        let drop_here = matches!(
            self.drop_target(),
            Some((DropTarget::Workspace(target), _)) if target == ws.id
        );
        let item = workspace_card(
            c,
            self.workspace_body(index, ws, fade, ghost),
            levels,
            ghost,
            drop_here,
        );
        self.workspace_interactions(ws.id, item)
    }

    /// A workspace as just its icon, for a narrow sidebar: no card, a fill when it's open or
    /// hovered, a small close button at its top right while hovered, and its name on hover.
    fn compact_workspace_item<'a>(&'a self, ws: &'a Workspace) -> Element<'a, Message> {
        let c = self.chrome;
        let levels = self.fades.get(anim::Item::Workspace(ws.id));
        let ghost = if self.dragging(Dragged::Workspace(ws.id)) {
            c.s.effects.dragged
        } else {
            1.0
        };
        let fade = (c.s.effects.dim + (1.0 - c.s.effects.dim) * levels.emphasis(c.s.effects.hover))
            * ghost;
        let notice = self.model.workspace_notice(ws.id).is_some();
        let agent = self.workspace_agent(ws);
        let (glyph, glyph_color, glyph_fade) = workspace_glyph(c, ws, notice, agent, fade, ghost);
        let drop_here = matches!(
            self.drop_target(),
            Some((DropTarget::Workspace(target), _)) if target == ws.id
        );
        let fill = mix(
            mix(Color::TRANSPARENT, c.hover, levels.hovered),
            c.raised,
            levels.selected,
        );
        let tile_size = c.s.workspaces.tile_size;
        let tile = container(draw_glyph(
            glyph,
            c.s.workspaces.icon_size,
            glyph_color,
            glyph_fade,
        ))
        .center(tile_size)
        .style(move |_| container::Style {
            border: Border {
                color: if drop_here {
                    c.attention
                } else {
                    Color::TRANSPARENT
                },
                width: c.s.controls.highlight_width,
                radius: c.s.workspaces.tile_radius.into(),
            },
            ..card_style(fill, c.s.workspaces.tile_radius)
        });
        let (close_icon, close_pad) = COMPACT_CLOSE;
        let close_size = close_icon + close_pad * 2.0;
        let close = (levels.hovered > 0.5 && self.config.workspace_items.close_button).then(|| {
            pin(button(icon(Icon::Close, close_icon, c.text))
                .on_press(Message::Command(Command::CloseWorkspace(ws.id)))
                .padding(close_pad)
                .style(move |_, status| {
                    let mut style = flat_button(c, status, Some(c.raised));
                    style.border.radius = (close_size / 2.0).into();
                    style
                }))
            .x(tile_size - close_size)
        });
        let tile = tooltip(
            stack![tile].push(close).width(tile_size).height(tile_size),
            name_tip(c, ws.name.clone()),
            tooltip::Position::Right,
        );
        self.workspace_interactions(ws.id, container(tile).center_x(Fill).into())
    }

    /// A workspace `item` with what makes it one: press to select and drag, hover to light it
    /// and aim drops, right-click for its menu.
    fn workspace_interactions<'a>(
        &'a self,
        id: WorkspaceId,
        item: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let hover = SidebarHover::Workspace(id);
        RightClick::new(
            Pointer::new(
                mouse_area(item)
                    .on_press(Message::WorkspacePress(id))
                    .on_enter(Message::SidebarHover(hover, true))
                    .on_exit(Message::SidebarHover(hover, false))
                    .interaction(mouse::Interaction::Pointer),
            )
            .on_zone(Halves::TopBottom, move |zone| {
                Message::SidebarZone(hover, zone)
            })
            .on_press_outside_maybe(
                (self.item_focus == Some(Renaming::Workspace(id)))
                    .then_some(Message::ItemBlur(Renaming::Workspace(id))),
            ),
            move |at| Message::MenuOpen(MenuTarget::Workspace(id), at),
        )
        .into()
    }

    /// What a workspace item shows inside its card, by `[workspace_items]`: its icon tile, title
    /// and detail lines, marks, badge and close button, text faded to `fade`. In a
    /// [`SidebarSize::Narrow`] sidebar, just the tile and the title's first letter.
    fn workspace_body<'a>(
        &'a self,
        index: usize,
        ws: &'a Workspace,
        fade: f32,
        ghost: f32,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let items = &self.config.workspace_items;
        let dim = |color: Color| color.scale_alpha(fade);
        let notice = self.model.workspace_notice(ws.id).is_some();
        let agent = self.workspace_agent(ws);
        let values = WorkspaceValues::new(self, index, ws);
        let (detail, tone) = workspace_detail(c, &items.detail, &values, notice, agent);
        let detail_color = tone.map_or(dim(c.faint), |c| c.scale_alpha(ghost));
        let narrow = self.sidebar_size() == SidebarSize::Narrow;
        let title = label::fill(&items.title, |key| values.get(key));
        let badge = label::fill(&items.badge, |key| values.get(key));
        let (title, detail, badge) = if narrow {
            let letter = title.chars().next().map(|l| l.to_uppercase().collect());
            (letter.unwrap_or_default(), String::new(), String::new())
        } else {
            (title, detail, badge)
        };
        let (glyph, glyph_color, glyph_fade) = workspace_glyph(c, ws, notice, agent, fade, ghost);

        let glyph = draw_glyph(glyph, c.s.workspaces.icon_size, glyph_color, glyph_fade);
        // Without its tile's background the icon is just a glyph, so it goes on the first line
        // (centered on it, in a tile-wide box so it stays where the tile was) rather than
        // centered between the lines, where it would sit below the name.
        let in_lines = items.icon
            && !items.icon_background(self.config.ui_theme)
            && !(title.is_empty() && detail.is_empty());
        let (mut lead, mut tile) = if in_lines {
            (Some(glyph), None)
        } else {
            (None, items.icon.then_some(glyph))
        };
        let tile_size = c.s.workspaces.tile_size;
        // A line of text, after the icon if it's the first line, else after the icon's room.
        let mut line = |size: f32, content: Element<'a, Message>| -> Element<'a, Message> {
            if !in_lines {
                return content;
            }
            let room: Element<'a, Message> = match lead.take() {
                Some(icon) => container(icon)
                    .center_x(tile_size)
                    .center_y(size * LINE_HEIGHT)
                    .into(),
                None => Space::new().width(tile_size).into(),
            };
            row![room, content]
                .spacing(c.s.workspaces.spacing)
                .align_y(alignment::Vertical::Center)
                .into()
        };
        let mut lines = column![]
            .spacing(c.s.workspaces.line_spacing)
            .width(Fill)
            .clip(true);
        if !title.is_empty() {
            let size = c.s.workspaces.title_size;
            lines = lines.push(line(size, one_line(title, size, dim(c.text)).into()));
        }
        if !detail.is_empty() {
            let size = c.s.workspaces.detail_size;
            let text = one_line(
                truncate(&detail, c.s.workspaces.max_detail_chars as usize),
                size,
                detail_color,
            );
            lines = lines.push(line(size, text.into()));
        }
        let mark = |glyph| icon(glyph, c.s.workspaces.mark_size, c.faint).opacity(fade);
        let marks = row![]
            .push((items.status_icons && !narrow && ws.muted).then(|| mark(Icon::BellOff)))
            .push((items.status_icons && !narrow && ws.pinned).then(|| mark(Icon::Pin)))
            .spacing(MARK_SPACING);
        row![]
            .push(
                tile.take()
                    .map(|icon| icon_tile(c, icon, items.icon_background(self.config.ui_theme))),
            )
            // The same height whatever lines are shown.
            .push(
                container(lines)
                    .height(workspace_content_height(c))
                    .align_y(alignment::Vertical::Center),
            )
            .push(marks)
            .push((!badge.is_empty()).then(|| {
                text(badge)
                    .size(c.s.workspaces.badge_size)
                    .color(dim(c.faint))
            }))
            .push((items.close_button && !narrow).then(|| {
                faded_icon_button(
                    c,
                    Icon::Close,
                    Message::Command(Command::CloseWorkspace(ws.id)),
                    fade,
                )
            }))
            .spacing(c.s.workspaces.spacing)
            .align_y(alignment::Vertical::Center)
            .into()
    }
}

/// The inset of the insertion `line` for a dragged workspace, if it goes above `above`.
fn line_inset(c: Chrome, line: Option<(LineAbove, bool)>, above: LineAbove) -> Option<f32> {
    line.filter(|(at, _)| *at == above).map(|(_, grouped)| {
        if grouped {
            group_inset(c) + GROUPED_LINE_INSET
        } else {
            0.0
        }
    })
}

/// The thin line above the Settings button.
fn sidebar_separator<'a>(c: Chrome) -> Element<'a, Message> {
    container(Space::new().width(Fill).height(1))
        .style(move |_| card_style(c.separator, 0.0))
        .into()
}

/// The Settings button at the foot of the sidebar.
fn settings_button<'a>(c: Chrome) -> Element<'a, Message> {
    button(
        row![
            icon(Icon::Settings, SETTINGS_ICON_SIZE, c.muted),
            text("Settings").size(SETTINGS_TEXT_SIZE).color(c.muted)
        ]
        .spacing(SETTINGS_SPACING)
        .align_y(alignment::Vertical::Center),
    )
    .on_press(Message::Action(Action::OpenSettings))
    .style(move |_, status| flat_button(c, status, None))
    .width(Fill)
    .padding(SETTINGS_PADDING)
    .into()
}

/// A group's name field while it's renamed.
fn group_rename_field<'a>(c: Chrome, id: GroupId, value: &'a str) -> Element<'a, Message> {
    Pointer::new(
        text_input("Group name", value)
            .id(RENAME_INPUT)
            .on_input(Message::RenameInput)
            .on_submit(Message::RenameCommit)
            .size(c.s.groups.name_size)
            .padding([c.s.groups.padding, GROUP_PADDING_X])
            .style(move |_, status| input_style(c, status)),
    )
    .on_press_outside(Message::RenameCancel(Renaming::Group(id)))
    .into()
}

/// Where a group's line runs, from the sidebar's content edge: under the left edge of the
/// header's icon, which `group_header_card` centers where an ungrouped workspace's icon is.
fn group_line_x(c: Chrome) -> f32 {
    (c.s.workspaces.padding + (c.s.workspaces.tile_size - c.s.groups.chevron_size) / 2.0).round()
}

/// How far a group's workspaces are indented: past its line, then `style.groups.indent` more,
/// so they never run into the line.
fn group_inset(c: Chrome) -> f32 {
    group_line_x(c) + GROUP_LINE_WIDTH + c.s.groups.indent
}

/// A group's workspaces, indented by [`group_inset`] with a thin line at [`group_line_x`],
/// under the start of the header's icon, so the group reads as one block under its header.
fn group_block<'a>(c: Chrome, members: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    SideLine::new(
        column(members).spacing(c.s.sidebar.spacing).width(Fill),
        group_inset(c),
        group_line_x(c),
        GROUP_LINE_WIDTH,
        c.outline.scale_alpha(GROUP_LINE_ALPHA),
    )
    .into()
}

/// A group's icon: its own, else a folder.
fn group_glyph(group: &Group) -> Glyph {
    named_glyph(group.icon.as_deref()).unwrap_or(Glyph::Icon(Icon::Folder))
}

/// The name shown beside a compact sidebar item on hover.
pub(super) fn name_tip<'a>(c: Chrome, name: String) -> Element<'a, Message> {
    container(one_line(name, c.s.workspaces.title_size, c.text))
        .padding([4, 8])
        .style(move |_| container::Style {
            border: Border {
                color: c.item_edge,
                width: c.s.menus.border_width,
                radius: c.s.controls.radius.into(),
            },
            ..card_style(c.popup, c.s.controls.radius)
        })
        .into()
}

/// The folded sidebar where the system draws the title bar: just a button that brings it back.
/// (With the custom title bar the button is there, and nothing is left here.)
fn sidebar_rail<'a>(c: Chrome) -> Element<'a, Message> {
    container(icon_button(
        c,
        Icon::PanelLeft,
        Message::Action(Action::ToggleSidebar),
    ))
    .center_x(c.s.window.sidebar_width)
    .padding([c.s.sidebar.padding, 0.0])
    .height(Fill)
    .into()
}

/// A group header's background: `lit` while it stands in for the active workspace, tinted and
/// outlined while a dragged workspace would drop into the group.
fn group_header_card<'a>(
    c: Chrome,
    header: Element<'a, Message>,
    lit: bool,
    drop_here: bool,
    compact: bool,
) -> Element<'a, Message> {
    // The group's icon centers on the workspace icons below: a workspace's tile starts its
    // padding in (glass splits that padding around the sheet, but the sum is the same), and its
    // icon sits in the tile's middle. Compact headers are centered instead.
    let left = if compact {
        GROUP_PADDING_X
    } else {
        c.s.workspaces.padding + (c.s.workspaces.tile_size - c.s.groups.chevron_size) / 2.0
    };
    container(header)
        .padding(iced::Padding {
            left,
            ..iced::Padding::from([c.s.groups.padding, GROUP_PADDING_X])
        })
        .width(Fill)
        .style(move |_| container::Style {
            background: if drop_here {
                Some(c.attention.scale_alpha(c.s.effects.drop_tint).into())
            } else {
                lit.then(|| c.item.into())
            },
            border: Border {
                color: if drop_here {
                    c.attention
                } else {
                    Color::TRANSPARENT
                },
                width: c.s.controls.highlight_width,
                radius: c.s.workspaces.radius.into(),
            },
            ..card_style(c.item, c.s.workspaces.radius)
        })
        .into()
}

/// A workspace's name field while it's renamed, as tall as the item it replaces.
fn workspace_rename_field<'a>(c: Chrome, id: WorkspaceId, value: &'a str) -> Element<'a, Message> {
    Pointer::new(
        text_input("Workspace name", value)
            .id(RENAME_INPUT)
            .on_input(Message::RenameInput)
            .on_submit(Message::RenameCommit)
            .size(c.s.workspaces.title_size)
            // Padding that centers one line of text in the item's height, so swapping the item
            // for the field doesn't move anything below it.
            .padding(
                ((workspace_item_height(c) - c.s.workspaces.title_size * LINE_HEIGHT) / 2.0)
                    .max(0.0),
            )
            .style(move |_, status| input_style(c, status)),
    )
    .on_press_outside(Message::RenameCancel(Renaming::Workspace(id)))
    .into()
}

/// A workspace's detail line from `template`, and the color of what it shows first: the
/// attention color for a waiting notification, else its agent's.
fn workspace_detail(
    c: Chrome,
    template: &str,
    values: &WorkspaceValues<'_>,
    notice: bool,
    agent: Option<&Agent>,
) -> (String, Option<Color>) {
    // The first non-empty placeholder that carries a color decides the line's tone.
    let tone = std::cell::Cell::new(None);
    let detail = label::fill(template, |key| {
        let value = values.get(key)?;
        if !value.is_empty() {
            let notice = notice.then_some(c.attention);
            let shows = match key {
                "notice" => notice,
                "agent" => agent.map(|a| agent_line(c, a).1),
                "status" | "activity" => notice.or_else(|| agent.map(|a| agent_line(c, a).1)),
                _ => None,
            };
            if tone.get().is_none() {
                tone.set(shows);
            }
        }
        Some(value)
    });
    (detail, tone.get())
}

/// A workspace's icon, its color and opacity: a waiting notification shows over everything; an
/// agent over the default icon only.
fn workspace_glyph(
    c: Chrome,
    ws: &Workspace,
    notice: bool,
    agent: Option<&Agent>,
    fade: f32,
    ghost: f32,
) -> (Glyph, Color, f32) {
    match (notice, agent) {
        (true, _) => (Icon::Bell.into(), c.attention, ghost),
        (false, Some(agent)) if ws.icon.is_none() => {
            (Icon::Sparkles.into(), agent_line(c, agent).1, ghost)
        }
        _ => (
            workspace_icon(ws.icon.as_deref()),
            workspace_color(ws.color.as_deref()).unwrap_or(c.text),
            fade,
        ),
    }
}

/// A workspace item's card around `body`: filled by how selected and hovered it is (`levels`),
/// faded to `ghost` while dragged, outlined while a dragged tab would drop onto it. Glass lifts
/// it when selected; solid gives it a lighter top edge.
fn workspace_card<'a>(
    c: Chrome,
    body: Element<'a, Message>,
    levels: anim::Levels,
    ghost: f32,
    drop_here: bool,
) -> Element<'a, Message> {
    // `outer` and `padding` always add up to the same slot, so lifting grows the sheet into its
    // margin without moving anything around it.
    let lift = levels.selected;
    let (outer, padding) = glass::lift_padding(c, iced::Padding::new(c.s.workspaces.padding), lift);
    let inner = container(body)
        .padding(padding)
        .width(Fill)
        .style(move |_| container::Style {
            background: Some(
                item_fill(
                    c,
                    levels,
                    c.workspace,
                    c.workspace_active,
                    c.workspace_hover,
                )
                .scale_alpha(ghost)
                .into(),
            ),
            border: Border {
                color: if drop_here {
                    c.attention
                } else {
                    item_ease(
                        levels,
                        c.workspace_border,
                        c.workspace_border_active,
                        c.workspace_border_hover,
                    )
                    .scale_alpha(ghost)
                },
                width: if drop_here {
                    c.s.controls.highlight_width
                } else {
                    c.s.controls.border_width
                },
                radius: c.s.workspaces.radius.into(),
            },
            ..container::Style::default()
        });
    if c.glass.is_some() {
        return container(
            container(glass::frosted(
                c,
                inner,
                c.s.workspaces.radius,
                glass_strength(c, levels) * ghost,
                lift,
            ))
            .style(move |_| container::Style {
                border: Border {
                    radius: c.s.workspaces.radius.into(),
                    ..Border::default()
                },
                shadow: glass::lift_shadow(c, c.shadow, lift),
                ..container::Style::default()
            }),
        )
        .padding(outer)
        .into();
    }
    // Solid: just the outlined card; a shadow would darken around an unfilled one.
    inner.into()
}

/// What a workspace item's templates can show (`[workspace_items]`).
/// Gathered once per item so each placeholder is a lookup, not a model walk.
struct WorkspaceValues<'a> {
    /// The workspace itself, for `{name}`.
    ws: &'a Workspace,
    /// Its position in the sidebar from 0; `{index}` and `{shortcut}` show it from 1.
    index: usize,
    /// How many panes its layout has.
    panes: usize,
    /// How many tabs across all its panes.
    tabs: usize,
    /// The focused pane's active tab, for `{title}`, `{dir}` and `{path}`.
    focused: Option<&'a Tab>,
    /// The body of its waiting notification, if any.
    notice: Option<&'a str>,
    /// Its most urgent agent's status line ([`agent_line`]), if any.
    agent: Option<String>,
}

impl<'a> WorkspaceValues<'a> {
    /// Collects the values for `ws`, the `index`th workspace in the sidebar.
    fn new(app: &'a App, index: usize, ws: &'a Workspace) -> Self {
        let panes = ws.layout.leaves();
        Self {
            ws,
            index,
            panes: panes.len(),
            tabs: panes
                .iter()
                .filter_map(|p| app.model.pane(*p))
                .map(|p| p.tabs.len())
                .sum(),
            focused: app
                .model
                .pane(ws.focused)
                .and_then(|p| app.model.tab(p.active)),
            notice: app.model.workspace_notice(ws.id).map(|n| n.body.as_str()),
            agent: app.workspace_agent(ws).map(|a| agent_line(app.chrome, a).0),
        }
    }

    /// `{summary}`: "1 terminal", "3 tabs", or "2 panes, 3 tabs".
    fn summary(&self) -> String {
        match (self.panes, self.tabs) {
            (1, 1) => "1 terminal".to_owned(),
            (1, t) => format!("{t} tabs"),
            (p, t) => format!("{p} panes, {t} tabs"),
        }
    }

    /// The value of placeholder `key`; `None` for an unknown name, which [`label::fill`] leaves
    /// as typed. `{shortcut}` is empty past the ninth workspace, which has no 1-9 shortcut, and
    /// `{status}` falls back from the notice to the agent to the summary, `{activity}` from the
    /// notice to the agent to nothing.
    fn get(&self, key: &str) -> Option<String> {
        Some(match key {
            "name" => self.ws.name.clone(),
            "index" => (self.index + 1).to_string(),
            "shortcut" if self.index < 9 => (self.index + 1).to_string(),
            "shortcut" => String::new(),
            "title" => self.focused.map(tab_title).unwrap_or_default(),
            "tabs" => self.tabs.to_string(),
            "panes" => self.panes.to_string(),
            "summary" => self.summary(),
            "dir" => self.focused.map(tab_dir).unwrap_or_default(),
            "path" => self.focused.and_then(|t| t.cwd.clone()).unwrap_or_default(),
            "notice" => self.notice.unwrap_or_default().to_owned(),
            "agent" => self.agent.clone().unwrap_or_default(),
            "status" => match (self.notice, &self.agent) {
                (Some(notice), _) => notice.to_owned(),
                (None, Some(agent)) => agent.clone(),
                (None, None) => self.summary(),
            },
            "activity" => match (self.notice, &self.agent) {
                (Some(notice), _) => notice.to_owned(),
                (None, agent) => agent.clone().unwrap_or_default(),
            },
            _ => return None,
        })
    }
}

/// What an agent is doing, and the color for its state: green working, attention color when it
/// needs the user, muted when idle.
pub(super) fn agent_line(c: Chrome, agent: &Agent) -> (String, Color) {
    match agent.state {
        AgentState::Working => (
            format!("Claude: {}", agent.detail.as_deref().unwrap_or("working")),
            c.working,
        ),
        AgentState::NeedsInput => (
            match agent.detail.as_deref() {
                // Claude Code's own messages already say "Claude needs ...".
                Some(detail) if detail.starts_with("Claude") => detail.to_owned(),
                Some(detail) => format!("Claude: {detail}"),
                None => "Claude needs your input".to_owned(),
            },
            c.attention,
        ),
        AgentState::Idle => (
            format!("Claude: {}", agent.detail.as_deref().unwrap_or("idle")),
            c.muted,
        ),
    }
}

impl App {
    /// The most urgent agent in a workspace: one that needs input, else one working, else idle.
    fn workspace_agent(&self, ws: &Workspace) -> Option<&Agent> {
        let rank = |a: &Agent| match a.state {
            AgentState::NeedsInput => 0,
            AgentState::Working => 1,
            AgentState::Idle => 2,
        };
        ws.layout
            .leaves()
            .into_iter()
            .filter_map(|p| self.model.pane(p))
            .flat_map(|p| p.tabs.iter())
            .filter_map(|t| self.model.tab(*t)?.agent.as_ref())
            .min_by_key(|a| rank(a))
    }
}

//! Tabs and sidebar items under the pointer: pressing, hovering, dragging and dropping them.
//!
//! A press arms `App.item_drag`; the subscription then sends cursor moves and the release. The
//! hover state kept here (`hovered_tab`, `sidebar_hover`, `hover_zone`) is what `view/drag.rs`
//! reads to work out where a drop would land, both for drawing markers and for the drop itself.

use std::time::{Duration, Instant};

use iced::{Point, Task};
use macaroni_core::{Command, MoveTo, PaneId, TabId, WorkspaceId};

use crate::message::Message;
use crate::state::{Dragged, DropTarget, ItemDrag, Renaming, SidebarHover};
use crate::{App, pointer, view};

/// How far (logical pixels) a pressed tab or workspace must move before it's dragged rather
/// than clicked, so a slightly shaky click doesn't move anything.
const TAB_DRAG_THRESHOLD: f32 = 6.0;
/// How long a dragged tab hovers a sidebar workspace before that workspace opens.
const WORKSPACE_HOVER_OPEN: Duration = Duration::from_millis(600);
/// How far the tab strip scrolls per wheel line, in pixels.
const TAB_SCROLL_LINE: f32 = 60.0;

impl App {
    /// A tab was pressed: select it, focus it for renaming, and arm a drag.
    pub(crate) fn tab_press(&mut self, tab: TabId) -> Task<Message> {
        self.item_focus = Some(Renaming::Tab(tab));
        self.press_item(Dragged::Tab(tab));
        self.apply(Command::SelectTab(tab))
    }

    /// A sidebar workspace was pressed: select it, focus it for renaming, and arm a drag.
    pub(crate) fn workspace_press(&mut self, ws: WorkspaceId) -> Task<Message> {
        self.item_focus = Some(Renaming::Workspace(ws));
        self.press_item(Dragged::Workspace(ws));
        self.apply(Command::SelectWorkspace(ws))
    }

    /// Arms a drag of a pressed item; it starts once the pointer moves far enough.
    fn press_item(&mut self, item: Dragged) {
        self.item_drag = Some(ItemDrag {
            item,
            from: None,
            at: Point::ORIGIN,
            dragging: false,
        });
    }

    /// Whether a pressed item is being dragged, not just held.
    pub(crate) fn dragging_item(&self) -> bool {
        self.item_drag.is_some_and(|d| d.dragging)
    }

    /// Follows the pointer with a pressed item, which becomes a drag once it moved far enough.
    pub(crate) fn item_drag_move(&mut self, at: Point) -> Task<Message> {
        if let Some(drag) = &mut self.item_drag {
            // The press itself carries no position, so the first move is the starting point.
            let from = *drag.from.get_or_insert(at);
            drag.at = at;
            drag.dragging |= from.distance(at) > TAB_DRAG_THRESHOLD;
        }
        Task::none()
    }

    /// The button was released: drop the dragged tab or workspace where it is. A press that
    /// never became a drag does nothing more (the press already selected the item).
    pub(crate) fn item_drag_end(&mut self) -> Task<Message> {
        // Both read `item_drag` and the hover state, so they run before either is cleared.
        // Each is `None` unless an item of its kind is being dragged.
        let target = self.drop_target().map(|(target, _)| target);
        let to = self.workspace_drop();
        // What was kept under the cursor through gaps; hover picks up again on the next
        // enter.
        if self.dragging_item() {
            self.sidebar_hover = None;
            self.hovered_tab = None;
        }
        match self.item_drag.take() {
            Some(ItemDrag {
                item: Dragged::Tab(tab),
                ..
            }) => match target {
                Some(target) => self.drop_tab(tab, target),
                None => Task::none(),
            },
            Some(ItemDrag {
                item: Dragged::Workspace(ws),
                ..
            }) => match to {
                Some(drop) => self.apply(Command::PlaceWorkspace {
                    id: ws,
                    before: drop.before,
                    group: drop.group,
                }),
                None => Task::none(),
            },
            None => Task::none(),
        }
    }

    /// Moves a dropped tab to where it was dropped.
    fn drop_tab(&mut self, tab: TabId, target: DropTarget) -> Task<Message> {
        let to = match target {
            DropTarget::Strip { pane, index } => MoveTo::Pane { pane, index },
            // Joining a pane appends; the model clamps the index.
            DropTarget::Pane { pane, side: None } => MoveTo::Pane {
                pane,
                index: usize::MAX,
            },
            DropTarget::Pane {
                pane,
                side: Some(direction),
            } => MoveTo::SplitPane { pane, direction },
            DropTarget::Edge(direction) => MoveTo::SplitWorkspace {
                workspace: self.model.active_workspace().id,
                direction,
            },
            DropTarget::Workspace(ws) => MoveTo::Workspace(ws),
            DropTarget::NewWorkspace => MoveTo::NewWorkspace,
        };
        self.apply(Command::MoveTab { tab, to })
    }

    /// The pointer entered or left a tab.
    pub(crate) fn tab_hover(&mut self, pane: PaneId, index: usize, entered: bool) -> Task<Message> {
        // While dragging, leaving an item keeps it as the target, so the gap after it still
        // shows where the drop would go.
        if entered {
            self.hovered_tab = Some((pane, index));
        } else if self.hovered_tab == Some((pane, index)) && !self.dragging_item() {
            self.hovered_tab = None;
        }
        Task::none()
    }

    /// The pointer entered or left a sidebar item.
    pub(crate) fn sidebar_hover(&mut self, hover: SidebarHover, entered: bool) -> Task<Message> {
        // Re-entering the same item keeps its time, so the hover-to-open delay isn't reset.
        // While dragging, leaving keeps it, as for tabs.
        if entered {
            if self.sidebar_hover.is_none_or(|(h, _)| h != hover) {
                self.sidebar_hover = Some((hover, Instant::now()));
            }
        } else if self.sidebar_hover.is_some_and(|(h, _)| h == hover) && !self.dragging_item() {
            self.sidebar_hover = None;
        }
        Task::none()
    }

    /// The pointer moved to `zone` of a sidebar item.
    pub(crate) fn sidebar_zone(
        &mut self,
        hover: SidebarHover,
        zone: pointer::Zone,
    ) -> Task<Message> {
        // Inside an item, it's now the one under the cursor; a gap only counts beside the item
        // it was left from.
        let current = self.sidebar_hover.is_some_and(|(h, _)| h == hover);
        if !zone.in_gap() && !current {
            self.sidebar_hover = Some((hover, Instant::now()));
        }
        if !zone.in_gap() || current {
            self.hover_zone = zone;
        }
        Task::none()
    }

    /// The pointer moved to `zone` of a tab.
    pub(crate) fn tab_zone(
        &mut self,
        pane: PaneId,
        index: usize,
        zone: pointer::Zone,
    ) -> Task<Message> {
        // As for sidebar items: a gap only counts for the tab it was left from.
        let current = self.hovered_tab == Some((pane, index));
        if !zone.in_gap() {
            self.hovered_tab = Some((pane, index));
        }
        if !zone.in_gap() || current {
            self.hover_zone = zone;
        }
        Task::none()
    }

    /// Opens the sidebar workspace a dragged tab has hovered long enough, so the drag can go on
    /// into its panes. Only while the cursor is still over the sidebar: the hover is kept when
    /// the cursor leaves, and moving out into the panes shouldn't switch workspaces.
    pub(crate) fn drag_tick(&mut self) -> Task<Message> {
        match self.sidebar_hover {
            Some((SidebarHover::Workspace(ws), since))
                if since.elapsed() >= WORKSPACE_HOVER_OPEN
                    && self
                        .item_drag
                        .is_some_and(|d| d.at.x < self.chrome.s.window.sidebar_width)
                    && self.model.active_workspace().id != ws =>
            {
                self.apply(Command::SelectWorkspace(ws))
            }
            _ => Task::none(),
        }
    }

    /// Remembers where a pane's tab strip is scrolled to, and if its open tab changed since it
    /// last reported (see [`App::reveal_tab`]), scrolls that tab into view now that the strip
    /// has its new width.
    pub(crate) fn tab_viewport(
        &mut self,
        pane: PaneId,
        viewport: iced::widget::scrollable::Viewport,
    ) -> Task<Message> {
        self.tab_views.retain(|p, _| self.model.pane(*p).is_some());
        self.tab_views.insert(pane, viewport);
        let before = self.tab_reveals.len();
        self.tab_reveals.retain(|p| *p != pane);
        if self.tab_reveals.len() == before {
            return Task::none();
        }
        self.scroll_to_open_tab(pane)
    }

    /// Scrolls `pane`'s strip just far enough to show its open tab, and again once the strip
    /// next reports its viewport, since a tab that was just added only widens it on the next
    /// layout. Scrolling past the current end is fine: the scrollable clamps the offset to its
    /// content when it lays out.
    pub(crate) fn reveal_tab(&mut self, pane: PaneId) -> Task<Message> {
        if !self.tab_reveals.contains(&pane) {
            self.tab_reveals.push(pane);
        }
        self.scroll_to_open_tab(pane)
    }

    /// Scrolls `pane`'s strip, as last reported, just far enough to show its open tab: its left
    /// edge at the strip's left if it's off to the left, its right edge at the strip's right if
    /// off to the right. Nothing while it's in view or the strip hasn't reported.
    fn scroll_to_open_tab(&self, pane: PaneId) -> Task<Message> {
        let (Some(viewport), Some(p)) = (self.tab_views.get(&pane), self.model.pane(pane)) else {
            return Task::none();
        };
        let pinned: Vec<bool> = p
            .tabs
            .iter()
            .map(|t| self.model.tab(*t).is_some_and(|t| t.pinned))
            .collect();
        let Some(index) = p.tabs.iter().position(|t| *t == p.active) else {
            return Task::none();
        };
        let content = viewport.content_bounds().width;
        let Some((left, right)) = view::tab_span(self.chrome, &pinned, index, content) else {
            return Task::none();
        };
        let offset = viewport.absolute_offset().x;
        let shown = viewport.bounds().width;
        let x = if left < offset {
            left
        } else if right > offset + shown {
            right - shown
        } else {
            return Task::none();
        };
        iced::widget::operation::scroll_to(
            view::tab_strip_id(pane),
            iced::widget::scrollable::AbsoluteOffset { x, y: 0.0 },
        )
    }

    /// Scrolls a pane's tab strip sideways with the wheel. Vertical and horizontal wheel motion
    /// both scroll it; wheel down or right moves toward the later tabs.
    pub(crate) fn tab_scroll(
        &self,
        pane: PaneId,
        delta: iced::mouse::ScrollDelta,
    ) -> Task<Message> {
        let by = match delta {
            iced::mouse::ScrollDelta::Lines { x, y } => -(x + y) * TAB_SCROLL_LINE,
            iced::mouse::ScrollDelta::Pixels { x, y } => -(x + y),
        };
        iced::widget::operation::scroll_by(
            view::tab_strip_id(pane),
            iced::widget::scrollable::AbsoluteOffset { x: by, y: 0.0 },
        )
    }
}

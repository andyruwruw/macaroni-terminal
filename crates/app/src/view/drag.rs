//! Drag and drop: where a dragged tab or workspace would land, and the insertion lines
//! that mark it.
//!
//! [`App::drop_target`] (tabs) and [`App::workspace_drop`] (workspaces) are read both by the view,
//! to draw markers and lines, and by `update` on release, to apply the move, so what's shown is
//! what happens. Tab targets over panes are hit tested here from the same `SplitNode::panes`
//! geometry the panes are drawn with; targets in a strip or the sidebar come from what the
//! cursor is over (`App.hovered_tab`, `App.sidebar_hover`, `App.hover_zone`, reported by
//! [`crate::pointer::Pointer`]), since the strip scrolls and the sidebar's layout isn't known
//! here. The lines are [`GapLine`]s, which draw in the gap without taking room.

use iced::{Element, Point, Rectangle, Size};
use macaroni_core::{
    Command, Direction, GroupId, Model, Pane, Rect, TabId, Workspace, WorkspaceId,
};

use super::tabs::tab_spacing;
use super::{pane_area, pane_overlap, workspace_origin};
use crate::gap_line::{Gap, GapLine};
use crate::pointer::Zone;
use crate::theme::Chrome;
use crate::{App, Dragged, DropTarget, Message, SidebarHover, WorkspaceDrop};

/// How close to the workspace's outer edge a dragged tab snaps to a whole-workspace split.
const EDGE_SNAP: f32 = 28.0;

/// The part of a pane, from each edge, where a dragged tab splits it rather than joins it.
const SPLIT_ZONE: f32 = 0.25;

/// How much further in than a group's workspaces the line for a drop into the group starts,
/// so it reads as inside the group rather than after it.
pub(super) const GROUPED_LINE_INSET: f32 = 16.0;

impl App {
    /// Where the dragged tab would land, and the area to mark for it (none for a tab strip or
    /// the sidebar, which mark the item under the cursor instead). Pane geometry is the same
    /// `SplitNode::panes` math the panes are drawn with; positions in a strip and sidebar
    /// entries come from what the cursor is over.
    pub fn drop_target(&self) -> Option<(DropTarget, Option<Rectangle>)> {
        let c = self.chrome;
        let drag = self.item_drag.filter(|d| d.dragging)?;
        let at = drag.at;
        let tab = drag.tab()?;
        let source = self.model.pane_of_tab(tab)?;
        let home = self.model.workspace_of_pane(source)?;
        let alone = self.model.pane(source).is_some_and(|p| p.tabs.len() == 1);

        if at.x < c.s.window.sidebar_width {
            return match self.sidebar_hover?.0 {
                SidebarHover::Workspace(ws) if ws != home => {
                    Some((DropTarget::Workspace(ws), None))
                }
                SidebarHover::Workspace(_) | SidebarHover::Group(_) => None,
                SidebarHover::NewWorkspace => Some((DropTarget::NewWorkspace, None)),
            };
        }

        let ws = self.model.active_workspace();
        let origin = workspace_origin(c);
        // The workspace area's padding (`App::workspace_area`) takes a gap off the right and
        // bottom; `window_size` is used instead of the `responsive` size, which isn't kept.
        let area = Rect {
            x: 0.0,
            y: 0.0,
            width: self.window_size.width - origin.x - c.s.panes.margin,
            height: self.window_size.height - origin.y - c.s.panes.margin,
        };
        let local = Point::new(at.x - origin.x, at.y - origin.y);
        let panes = ws
            .layout
            .panes(pane_area(c, Size::new(area.width, area.height)));

        // Near the workspace's outer edge: a pane as tall or wide as the whole workspace.
        if panes.len() > 1
            && let Some(edge) = edge_drop(origin, area, local)
        {
            return Some(edge);
        }
        // The same half-gap inset `App::workspace` pins each pane with, inside `pane_area`.
        let inset = c.s.panes.gap / 2.0;
        for (pane, r) in panes {
            let (x, y) = (origin.x + r.x + inset, origin.y + r.y + inset);
            let grow = pane_overlap(c) - c.s.panes.gap;
            let (width, height) = (r.width + grow, r.height + grow);
            if at.x < x || at.x > x + width || at.y < y || at.y > y + height {
                continue;
            }
            // The strip counts down to the middle of the gap above the card, so there's no dead
            // band between it and the body. The scrollbar room under the tabs is taken out of
            // that gap, so the card still starts `height + gap` down.
            if at.y < y + c.s.tabs.height + c.s.tabs.gap / 2.0 {
                // Before or after the hovered tab, by which half (or gap) the cursor is in. A
                // drag doesn't clear the hover on exit, so moving past the last tab keeps it as
                // "after the last"; with no tab of this pane hovered yet, it appends.
                let index = match self.hovered_tab {
                    Some((p, i)) if p == pane => i + usize::from(self.hover_zone.after()),
                    _ => usize::MAX,
                };
                return Some((DropTarget::Strip { pane, index }, None));
            }
            let card = Rectangle::new(
                Point::new(x, y + c.s.tabs.height + c.s.tabs.gap),
                Size::new(width, height - c.s.tabs.height - c.s.tabs.gap),
            );
            let nearest = nearest_side(card, at);
            // Nothing to do dropping a tab into its own pane, or splitting a lone tab off itself.
            if pane == source && (nearest.is_none() || alone) {
                return None;
            }
            return Some((
                DropTarget::Pane {
                    pane,
                    side: nearest,
                },
                Some(nearest.map_or(card, |side| half(card, side))),
            ));
        }
        None
    }

    /// Where in `pane`'s strip a tab dropped at `index` would show up, as the gap before that
    /// tab (`tabs.len()` is after the last), following `MoveTo::Pane`'s rules; `None` when it
    /// wouldn't move.
    pub(super) fn insertion_gap(&self, pane: &Pane, tab: TabId, index: usize) -> Option<usize> {
        let pinned_tab = self.model.tab(tab)?.pinned;
        let from = pane.tabs.iter().position(|t| *t == tab);
        let others: Vec<TabId> = pane.tabs.iter().copied().filter(|t| *t != tab).collect();
        let index = match from {
            Some(at) if index > at => index - 1,
            _ => index,
        };
        let pinned = others
            .iter()
            .take_while(|t| self.model.tab(**t).is_some_and(|t| t.pinned))
            .count();
        let to = if pinned_tab {
            index.min(pinned)
        } else {
            index.clamp(pinned, others.len())
        };
        match from {
            Some(at) if to == at => None,
            Some(at) if to > at => Some(to + 1),
            _ => Some(to),
        }
    }

    /// Where the dragged workspace would go, unless that wouldn't change anything. Over a
    /// workspace: before or after it by which half the cursor is over, in its group; out
    /// through the bottom of a group's last one, after the group. Over a group's header: its
    /// top half is before the group, its bottom half into its end; out through its bottom,
    /// the group's start (or after the group, collapsed). Over "New workspace", the end.
    pub fn workspace_drop(&self) -> Option<WorkspaceDrop> {
        let c = self.chrome;
        let drag = self
            .item_drag
            .filter(|d| d.dragging && d.at.x < c.s.window.sidebar_width)?;
        let Dragged::Workspace(ws) = drag.item else {
            return None;
        };
        let list = self.model.workspaces();
        let place = |before, group| WorkspaceDrop {
            before,
            group,
            onto_group: false,
        };
        // The first workspace after `id`, skipping the dragged one and any `past` matches.
        let after = |id: WorkspaceId, past: &dyn Fn(&Workspace) -> bool| {
            list.iter()
                .skip_while(|w| w.id != id)
                .skip(1)
                .find(|w| w.id != ws && !past(w))
                .map(|w| w.id)
        };
        let zone = self.hover_zone;
        let drop = match self.sidebar_hover?.0 {
            SidebarHover::Workspace(over) => {
                let group = self.model.workspace(over)?.group;
                let last_in_group = group.is_some()
                    && list.iter().rev().find(|w| w.group == group).map(|w| w.id) == Some(over);
                match zone {
                    Zone::GapAfter if last_in_group => {
                        place(after(over, &|w| w.group == group), None)
                    }
                    _ if over == ws => return None,
                    zone if zone.after() => place(after(over, &|_| false), group),
                    _ => place(Some(over), group),
                }
            }
            SidebarHover::Group(group) => {
                let collapsed = self.model.group(group)?.collapsed;
                let members: Vec<WorkspaceId> = list
                    .iter()
                    .filter(|w| w.group == Some(group))
                    .map(|w| w.id)
                    .collect();
                let first = *members.first()?;
                let others = members.iter().copied().find(|id| *id != ws);
                match zone {
                    Zone::GapBefore | Zone::Before => match others {
                        Some(_) => place(Some(first), None),
                        // Its only member: leaving it where it is, out of the group.
                        None => place(after(first, &|_| false), None),
                    },
                    Zone::After => WorkspaceDrop {
                        before: None,
                        group: Some(group),
                        onto_group: true,
                    },
                    Zone::GapAfter if collapsed => {
                        place(after(first, &|w| w.group == Some(group)), None)
                    }
                    Zone::GapAfter => place(others, Some(group)),
                }
            }
            SidebarHover::NewWorkspace => place(None, None),
        };
        // Try it on a copy: several drops can mean the same final order, and a no-op shows no
        // line rather than one that does nothing on release.
        let mut moved = self.model.clone();
        moved.apply(Command::PlaceWorkspace {
            id: ws,
            before: drop.before,
            group: drop.group,
        });
        let order = |m: &Model| {
            m.workspaces()
                .iter()
                .map(|w| (w.id, w.group))
                .collect::<Vec<_>>()
        };
        (order(&moved) != order(&self.model)).then_some(drop)
    }

    /// Whether `item` is being dragged (past the threshold, not just pressed), so it draws at
    /// `style.effects.dragged` opacity.
    pub(super) fn dragging(&self, item: Dragged) -> bool {
        self.item_drag.is_some_and(|d| d.dragging && d.item == item)
    }

    /// The sidebar element the insertion line for a dragged workspace goes above, and whether
    /// it lands in a group (the line is indented like the group's workspaces). None when it's
    /// dropped onto a header, or into a collapsed group, which marks the header instead.
    pub(super) fn workspace_line(&self) -> Option<(LineAbove, bool)> {
        let drop = self.workspace_drop().filter(|d| !d.onto_group)?;
        let Dragged::Workspace(ws) = self.item_drag?.item else {
            return None;
        };
        if drop
            .group
            .and_then(|g| self.model.group(g))
            .is_some_and(|g| g.collapsed)
        {
            return None;
        }
        // The line goes above whatever follows the workspace once moved, so apply the move to a
        // copy and look, rather than redoing `PlaceWorkspace`'s rules here.
        let mut moved = self.model.clone();
        moved.apply(Command::PlaceWorkspace {
            id: ws,
            before: drop.before,
            group: drop.group,
        });
        let list = moved.workspaces();
        let at = list.iter().position(|w| w.id == ws)?;
        let group = list[at].group;
        let above = match list.get(at + 1) {
            None => LineAbove::End,
            // The first of another group: above its header.
            Some(next) => match next.group {
                Some(g) if next.group != group => LineAbove::Group(g),
                _ => LineAbove::Workspace(next.id),
            },
        };
        Some((above, group.is_some()))
    }
}

/// The sidebar element a dragged workspace's insertion line goes above.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum LineAbove {
    /// Above this workspace's item.
    Workspace(WorkspaceId),
    /// Above this group's header, when the workspace lands just before the group.
    Group(GroupId),
    /// Above "New workspace", after the last workspace.
    End,
}

/// The insertion line above a sidebar element, `inset` from its left (`None` for no line),
/// across a gap `spacing` tall. `top` is how far below its bounds the element starts to show
/// (glass items sit a little inside theirs), so the line sits midway between what's seen.
pub(super) fn line_above(
    c: Chrome,
    el: Element<'_, Message>,
    inset: Option<f32>,
    spacing: f32,
    top: f32,
) -> Element<'_, Message> {
    // The gap runs from `spacing` above the bounds to `top` inside them (the item above shows
    // down to its bounds' bottom, since a resting sheet only rises), so its middle is
    // `(top - spacing) / 2` from the top edge.
    match inset {
        Some(inset) => GapLine::new(
            el,
            Gap::Top,
            (top - spacing) / 2.0,
            c.s.controls.insertion_line_width,
            c.insertion,
        )
        .inset(inset)
        .into(),
        None => el,
    }
}

/// Marks the gap before `el` (a tab, or the new tab button after the last) as where a dragged
/// tab would go. It's drawn over the gap rather than taking room, so the tabs don't shift under
/// the cursor.
pub(super) fn insertion_line(
    c: Chrome,
    el: Element<'_, Message>,
    line: bool,
) -> Element<'_, Message> {
    if !line {
        return el;
    }
    // Centered in the `tabs.spacing` gap to the left of `el`.
    GapLine::new(
        el,
        Gap::Left,
        -tab_spacing(c) / 2.0,
        c.s.controls.insertion_line_width,
        c.insertion,
    )
    .into()
}

/// A whole-workspace split when the cursor (`local`, from the pane area's `origin`) is within
/// [`EDGE_SNAP`] of the left, right or bottom of `area`, with its marker in window
/// coordinates. The top edge is left out: it's where the tab strips are.
fn edge_drop(origin: Point, area: Rect, local: Point) -> Option<(DropTarget, Option<Rectangle>)> {
    let direction = if local.x < EDGE_SNAP {
        Direction::Left
    } else if local.x > area.width - EDGE_SNAP {
        Direction::Right
    } else if local.y > area.height - EDGE_SNAP {
        Direction::Down
    } else {
        return None;
    };
    // The marker covers half of what the panes cover together, which is the whole area (see
    // `pane_area`).
    let (w, h) = (area.width, area.height);
    let (x, y, width, height) = match direction {
        Direction::Left => (0.0, 0.0, w / 2.0, h),
        Direction::Right => (w / 2.0, 0.0, w / 2.0, h),
        Direction::Up => (0.0, 0.0, w, h / 2.0),
        Direction::Down => (0.0, h / 2.0, w, h / 2.0),
    };
    let marker = Rectangle::new(
        Point::new(origin.x + x, origin.y + y),
        Size::new(width, height),
    );
    Some((DropTarget::Edge(direction), Some(marker)))
}

/// The side of `card` that `at` is nearest, if it's within [`SPLIT_ZONE`] of it; `None` for the
/// middle, which joins the pane.
fn nearest_side(card: Rectangle, at: Point) -> Option<Direction> {
    // In fractions of the card's size, so the split zone scales with the pane; clamped because
    // the cursor can be in the strip-to-card gap or a pixel outside.
    let u = ((at.x - card.x) / card.width).clamp(0.0, 1.0);
    let v = ((at.y - card.y) / card.height).clamp(0.0, 1.0);
    [
        (u, Direction::Left),
        (1.0 - u, Direction::Right),
        (v, Direction::Up),
        (1.0 - v, Direction::Down),
    ]
    .into_iter()
    .min_by(|a, b| a.0.total_cmp(&b.0))
    .filter(|(d, _)| *d < SPLIT_ZONE)
    .map(|(_, direction)| direction)
}

/// The half of `card` on `side`.
fn half(card: Rectangle, side: Direction) -> Rectangle {
    match side {
        Direction::Left => Rectangle {
            width: card.width / 2.0,
            ..card
        },
        Direction::Right => Rectangle {
            x: card.x + card.width / 2.0,
            width: card.width / 2.0,
            ..card
        },
        Direction::Up => Rectangle {
            height: card.height / 2.0,
            ..card
        },
        Direction::Down => Rectangle {
            y: card.y + card.height / 2.0,
            height: card.height / 2.0,
            ..card
        },
    }
}

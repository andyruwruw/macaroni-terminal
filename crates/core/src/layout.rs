//! The split tree of a workspace and its geometry.
//!
//! A workspace's layout is a binary tree ([`SplitNode`]): leaves are panes, and each split
//! divides its area between two children along an [`Axis`] at a `ratio`. This module owns the
//! tree surgery the model needs (splitting a leaf, removing one and collapsing its parent,
//! finding the pane to focus next) and the geometry the app and the model share:
//!
//! - [`SplitNode::panes`] lays the tree out into one [`Rect`] per pane. The model does this in
//!   [`Rect::UNIT`] for directional focus; the app does it in pixels to place pane widgets.
//! - [`SplitNode::dividers`] lists every split's boundary with its [`Side`] path from the root,
//!   which is what [`Command::SetSplitRatio`](crate::Command::SetSplitRatio) addresses.
//! - [`neighbour`] picks the pane past an edge for
//!   [`Command::FocusDirection`](crate::Command::FocusDirection).
//!
//! Nothing here enforces model invariants beyond the tree shape; [`crate::Model`] keeps ratios in
//! range and panes unique.

use crate::ids::PaneId;

/// How a split arranges its children: `Row` puts them side by side, `Column` stacks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    /// Children side by side: `first` on the left, `second` on the right, split along x.
    Row,
    /// Children stacked: `first` on top, `second` below, split along y.
    Column,
}

/// One child of a split. A path of these from the root addresses a node in the tree (see
/// [`Divider::path`]); a new pane's side decides whether it goes left/above or right/below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// The left child of an [`Axis::Row`] split, or the top one of an [`Axis::Column`] split.
    First,
    /// The right child of an [`Axis::Row`] split, or the bottom one of an [`Axis::Column`] split.
    Second,
}

/// A workspace's layout: a binary tree whose leaves are panes.
///
/// Every pane appears once. Leaf order (depth first, `first` before `second`) is the left to
/// right, top to bottom order used by [`SplitNode::leaves`] and by saved layouts
/// ([`crate::WorkspaceSpec::focused`]).
#[derive(Debug, Clone, PartialEq)]
pub enum SplitNode {
    /// A single pane filling the node's whole area.
    Leaf(PaneId),
    /// Two subtrees sharing the node's area.
    Split {
        /// Which way the area is divided.
        axis: Axis,
        /// Share of the space given to `first`, strictly between 0 and 1.
        ratio: f32,
        /// The left (row) or top (column) subtree.
        first: Box<SplitNode>,
        /// The right (row) or bottom (column) subtree.
        second: Box<SplitNode>,
    },
}

/// A rectangle in whatever units the caller passes in (the model works in a unit square).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge (y grows downward).
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

impl Rect {
    /// The unit square at the origin, the area the model lays layouts out in.
    pub const UNIT: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };

    /// The right edge, `x + width`.
    fn right(self) -> f32 {
        self.x + self.width
    }

    /// The bottom edge, `y + height`.
    fn bottom(self) -> f32 {
        self.y + self.height
    }
}

/// A split's draggable boundary. `path` addresses the split node from the root.
#[derive(Debug, Clone, PartialEq)]
pub struct Divider {
    /// The sides to take from the root to reach this split; empty for the root itself. Pass it
    /// back in [`Command::SetSplitRatio`](crate::Command::SetSplitRatio).
    pub path: Vec<Side>,
    /// The split's axis: an [`Axis::Row`] divider is a vertical line, an [`Axis::Column`] one a
    /// horizontal line.
    pub axis: Axis,
    /// The whole area the split divides; the boundary sits at `ratio` along `axis`.
    pub area: Rect,
    /// The split's current ratio, the share of `area` given to its first child.
    pub ratio: f32,
}

/// The smallest share a split may give either child. Ratios are clamped to
/// `MIN_RATIO..=1.0 - MIN_RATIO` so a dragged divider can't make a pane vanish.
pub const MIN_RATIO: f32 = 0.05;

impl SplitNode {
    /// Every pane in the tree, in leaf order (left to right, top to bottom).
    pub fn leaves(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    /// Appends the tree's panes to `out` in leaf order.
    fn collect_leaves(&self, out: &mut Vec<PaneId>) {
        match self {
            Self::Leaf(pane) => out.push(*pane),
            Self::Split { first, second, .. } => {
                first.collect_leaves(out);
                second.collect_leaves(out);
            }
        }
    }

    /// Whether `pane` is a leaf of this tree.
    pub fn contains(&self, pane: PaneId) -> bool {
        match self {
            Self::Leaf(p) => *p == pane,
            Self::Split { first, second, .. } => first.contains(pane) || second.contains(pane),
        }
    }

    /// Replaces the leaf `target` with an even (0.5) split holding it and `new`. `new_side`
    /// picks which side the new pane lands on. Returns whether `target` was found; the tree is
    /// unchanged when it wasn't.
    pub(crate) fn split_leaf(
        &mut self,
        target: PaneId,
        axis: Axis,
        new: PaneId,
        new_side: Side,
    ) -> bool {
        match self {
            Self::Leaf(p) if *p == target => {
                let (first, second) = match new_side {
                    Side::First => (new, target),
                    Side::Second => (target, new),
                };
                *self = Self::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(Self::Leaf(first)),
                    second: Box::new(Self::Leaf(second)),
                };
                true
            }
            Self::Leaf(_) => false,
            Self::Split { first, second, .. } => {
                first.split_leaf(target, axis, new, new_side)
                    || second.split_leaf(target, axis, new, new_side)
            }
        }
    }

    /// Removes the leaf `target`, collapsing its parent split into the sibling, which keeps its
    /// own subtree and takes the parent's whole area. Returns `None` when the tree becomes
    /// empty (it was just `Leaf(target)`). A tree without `target` comes back unchanged.
    pub(crate) fn without(self, target: PaneId) -> Option<Self> {
        match self {
            Self::Leaf(p) if p == target => None,
            Self::Leaf(_) => Some(self),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => match (first.without(target), second.without(target)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    axis,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            },
        }
    }

    /// The pane that should take focus when `target` is removed: the nearest leaf of its sibling.
    ///
    /// "Nearest" is the sibling subtree's leaf on `target`'s side: its first leaf when `target`
    /// is a first child, its last leaf when `target` is a second child. Returns `None` when
    /// `target` isn't in the tree or is the whole tree.
    pub(crate) fn neighbour_of(&self, target: PaneId) -> Option<PaneId> {
        let Self::Split { first, second, .. } = self else {
            return None;
        };
        // Only a direct child can match here; deeper leaves are found by recursing below.
        if **first == Self::Leaf(target) {
            return second.leaves().first().copied();
        }
        if **second == Self::Leaf(target) {
            return first.leaves().last().copied();
        }
        first
            .neighbour_of(target)
            .or_else(|| second.neighbour_of(target))
    }

    /// The node reached by following `path` from this one, or `None` if the path runs past a
    /// leaf. An empty path is this node.
    pub(crate) fn node_at_mut(&mut self, path: &[Side]) -> Option<&mut Self> {
        let Some((step, rest)) = path.split_first() else {
            return Some(self);
        };
        match self {
            Self::Leaf(_) => None,
            Self::Split { first, second, .. } => match step {
                Side::First => first.node_at_mut(rest),
                Side::Second => second.node_at_mut(rest),
            },
        }
    }

    /// Lays the tree out in `area`: each pane with its rectangle, in leaf order. The rects tile
    /// `area` exactly (see [`split_rect`]).
    pub fn panes(&self, area: Rect) -> Vec<(PaneId, Rect)> {
        let mut out = Vec::new();
        self.collect_panes(area, &mut out);
        out
    }

    /// Appends each pane's rect within `area` to `out`, in leaf order.
    fn collect_panes(&self, area: Rect, out: &mut Vec<(PaneId, Rect)>) {
        match self {
            Self::Leaf(pane) => out.push((*pane, area)),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (a, b) = split_rect(area, *axis, *ratio);
                first.collect_panes(a, out);
                second.collect_panes(b, out);
            }
        }
    }

    /// Every split in the tree laid out in `area`, parents before children (pre-order), each
    /// with its path from the root.
    pub fn dividers(&self, area: Rect) -> Vec<Divider> {
        let mut out = Vec::new();
        self.collect_dividers(area, &mut Vec::new(), &mut out);
        out
    }

    /// Appends this subtree's dividers to `out`. `path` is this node's path from the root; it is
    /// pushed and popped around each child, so it's unchanged on return.
    fn collect_dividers(&self, area: Rect, path: &mut Vec<Side>, out: &mut Vec<Divider>) {
        if let Self::Split {
            axis,
            ratio,
            first,
            second,
        } = self
        {
            out.push(Divider {
                path: path.clone(),
                axis: *axis,
                area,
                ratio: *ratio,
            });
            let (a, b) = split_rect(area, *axis, *ratio);
            path.push(Side::First);
            first.collect_dividers(a, path, out);
            path.pop();
            path.push(Side::Second);
            second.collect_dividers(b, path, out);
            path.pop();
        }
    }
}

/// Splits `area` in two along `axis`: the first part gets `ratio` of the width (row) or height
/// (column), the second the rest.
///
/// The second rect starts exactly where the first ends and its size is the remainder, so the
/// two always tile `area` with no gap or overlap.
pub fn split_rect(area: Rect, axis: Axis, ratio: f32) -> (Rect, Rect) {
    match axis {
        Axis::Row => {
            let w = area.width * ratio;
            (
                Rect { width: w, ..area },
                Rect {
                    x: area.x + w,
                    width: area.width - w,
                    ..area
                },
            )
        }
        Axis::Column => {
            let h = area.height * ratio;
            (
                Rect { height: h, ..area },
                Rect {
                    y: area.y + h,
                    height: area.height - h,
                    ..area
                },
            )
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// A screen direction, for directional focus and for which side of a pane a new one goes.
pub enum Direction {
    /// Toward smaller x.
    Left,
    /// Toward larger x.
    Right,
    /// Toward smaller y.
    Up,
    /// Toward larger y.
    Down,
}

/// The pane adjacent to `from` in `direction`: among panes past the edge that overlap it on the
/// other axis, the closest one, preferring the largest overlap.
///
/// For each other pane this measures the `gap` from `from`'s edge to the candidate's facing edge
/// (0 for panes that touch, as every neighbour does in a tiled layout) and the length `shared`
/// that the two overlap on the perpendicular axis. A candidate needs `gap > -EPS` (it isn't
/// behind the edge) and `shared > EPS` (a real overlap, not just a shared corner). The smallest
/// gap wins; ties go to the largest overlap. Returns `None` when `from` isn't in `panes` or
/// nothing lies that way.
pub fn neighbour(panes: &[(PaneId, Rect)], from: PaneId, direction: Direction) -> Option<PaneId> {
    /// Tolerance for float error in edges computed through nested ratios.
    const EPS: f32 = 1e-4;
    let (_, here) = panes.iter().find(|(id, _)| *id == from)?;
    let overlap = |a0: f32, a1: f32, b0: f32, b1: f32| a1.min(b1) - a0.max(b0);

    panes
        .iter()
        .filter(|(id, _)| *id != from)
        .filter_map(|(id, r)| {
            let (gap, shared) = match direction {
                Direction::Left => (
                    here.x - r.right(),
                    overlap(here.y, here.bottom(), r.y, r.bottom()),
                ),
                Direction::Right => (
                    r.x - here.right(),
                    overlap(here.y, here.bottom(), r.y, r.bottom()),
                ),
                Direction::Up => (
                    here.y - r.bottom(),
                    overlap(here.x, here.right(), r.x, r.right()),
                ),
                Direction::Down => (
                    r.y - here.bottom(),
                    overlap(here.x, here.right(), r.x, r.right()),
                ),
            };
            (gap > -EPS && shared > EPS).then_some((*id, gap, shared))
        })
        .min_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
        })
        .map(|(id, _, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: PaneId = PaneId(1);
    const B: PaneId = PaneId(2);
    const C: PaneId = PaneId(3);

    /// A | (B over C)
    fn three() -> SplitNode {
        let mut tree = SplitNode::Leaf(A);
        tree.split_leaf(A, Axis::Row, B, Side::Second);
        tree.split_leaf(B, Axis::Column, C, Side::Second);
        tree
    }

    #[test]
    fn splitting_and_rects() {
        let rects = three().panes(Rect::UNIT);
        assert_eq!(
            rects[0],
            (
                A,
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.5,
                    height: 1.0
                }
            )
        );
        assert_eq!(
            rects[1],
            (
                B,
                Rect {
                    x: 0.5,
                    y: 0.0,
                    width: 0.5,
                    height: 0.5
                }
            )
        );
        assert_eq!(
            rects[2],
            (
                C,
                Rect {
                    x: 0.5,
                    y: 0.5,
                    width: 0.5,
                    height: 0.5
                }
            )
        );
    }

    #[test]
    fn removing_collapses_the_parent() {
        let tree = three().without(B).unwrap();
        assert_eq!(tree.leaves(), vec![A, C]);
        assert!(matches!(
            tree,
            SplitNode::Split {
                axis: Axis::Row,
                ..
            }
        ));
        assert_eq!(SplitNode::Leaf(A).without(A), None);
    }

    #[test]
    fn neighbour_of_removed_pane_is_its_sibling() {
        let tree = three();
        assert_eq!(tree.neighbour_of(C), Some(B));
        assert_eq!(tree.neighbour_of(B), Some(C));
        assert_eq!(tree.neighbour_of(A), Some(B));
    }

    #[test]
    fn directional_focus() {
        let rects = three().panes(Rect::UNIT);
        assert_eq!(neighbour(&rects, A, Direction::Right), Some(B));
        assert_eq!(neighbour(&rects, C, Direction::Left), Some(A));
        assert_eq!(neighbour(&rects, B, Direction::Down), Some(C));
        assert_eq!(neighbour(&rects, C, Direction::Up), Some(B));
        assert_eq!(neighbour(&rects, A, Direction::Left), None);
    }

    #[test]
    fn dividers_have_paths() {
        let dividers = three().dividers(Rect::UNIT);
        assert_eq!(dividers.len(), 2);
        assert_eq!(dividers[0].path, Vec::<Side>::new());
        assert_eq!(dividers[1].path, vec![Side::Second]);
        assert_eq!(dividers[1].axis, Axis::Column);
    }
}

//! Eased selection and hover for the sidebar's workspaces and the strips' tabs. `App::sync_fades`
//! sets every item's targets after each message ([`Fades::sync`]); the view reads the eased
//! levels ([`Fades::get`]), and while any is moving ([`Fades::animating`]) the app subscribes to
//! `window::frames` so it keeps redrawing. Levels are computed from the time of the last `sync`,
//! so a frame message (which runs `sync` again) is what advances them.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use iced::Color;
use macaroni_core::{TabId, WorkspaceId};

/// How long a fade from 0 to 1 (or back) takes.
const DURATION: Duration = Duration::from_millis(160);

/// What a fade belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Item {
    /// A workspace's row in the sidebar.
    Workspace(WorkspaceId),
    /// A tab in a pane's strip.
    Tab(TabId),
}

/// An item's eased levels, 0 to 1.
#[derive(Debug, Clone, Copy, Default)]
pub struct Levels {
    /// How selected the item is: 1 when it's the active workspace or its pane's active tab.
    pub selected: f32,
    /// How hovered the item is.
    pub hovered: f32,
}

impl Levels {
    /// How prominent the item is: full when selected, partway when only hovered. `hover` is how
    /// far toward selected a full hover goes (`style.effects.hover`).
    pub fn emphasis(self, hover: f32) -> f32 {
        self.selected + (1.0 - self.selected) * self.hovered * hover
    }
}

/// One level easing from `from` to `to` over [`DURATION`], starting at `start`.
#[derive(Debug, Clone, Copy)]
struct Fade {
    /// The level when the fade started.
    from: f32,
    /// The target level, 0 or 1.
    to: f32,
    /// When the fade started.
    start: Instant,
}

impl Fade {
    /// The level at `now`, eased out (cubic), and `to` once the fade is over.
    fn at(&self, now: Instant) -> f32 {
        let t = (now.saturating_duration_since(self.start).as_secs_f32() / DURATION.as_secs_f32())
            .min(1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        self.from + (self.to - self.from) * eased
    }

    /// Starts easing toward `to` from wherever the fade is at `now`, so a reversal mid-fade doesn't
    /// jump. Does nothing if `to` is already the target.
    fn retarget(&mut self, to: f32, now: Instant) {
        if self.to != to {
            *self = Fade {
                from: self.at(now),
                to,
                start: now,
            };
        }
    }
}

/// Every visible workspace's and tab's selection and hover fades. Lives in `App`.
#[derive(Debug, Default)]
pub struct Fades {
    /// Each item's selection fade, then its hover fade.
    items: HashMap<Item, (Fade, Fade)>,
    /// The time of the last [`Fades::sync`], what [`Fades::get`] reads levels at; `None` before
    /// the first.
    now: Option<Instant>,
}

impl Fades {
    /// Sets each item's targets, as `(item, selected, hovered)`; items left out are forgotten.
    /// New items start at their targets, so nothing animates in when a workspace or tab appears.
    pub fn sync(&mut self, now: Instant, targets: impl IntoIterator<Item = (Item, bool, bool)>) {
        let level = |on: bool| if on { 1.0 } else { 0.0 };
        let mut items = HashMap::with_capacity(self.items.len());
        for (item, selected, hovered) in targets {
            let (selected, hovered) = (level(selected), level(hovered));
            let fades = match self.items.remove(&item) {
                Some((mut s, mut h)) => {
                    s.retarget(selected, now);
                    h.retarget(hovered, now);
                    (s, h)
                }
                None => {
                    let still = |to| Fade {
                        from: to,
                        to,
                        start: now,
                    };
                    (still(selected), still(hovered))
                }
            };
            items.insert(item, fades);
        }
        self.items = items;
        self.now = Some(now);
    }

    /// The item's levels as of the last [`Fades::sync`]; all 0 for an item it doesn't know.
    pub fn get(&self, item: Item) -> Levels {
        match (self.items.get(&item), self.now) {
            (Some((s, h)), Some(now)) => Levels {
                selected: s.at(now),
                hovered: h.at(now),
            },
            _ => Levels::default(),
        }
    }

    /// Whether any fade was still moving at the last [`Fades::sync`], so another frame is needed.
    pub fn animating(&self) -> bool {
        let Some(now) = self.now else {
            return false;
        };
        self.items
            .values()
            .any(|(s, h)| now < s.start + DURATION || now < h.start + DURATION)
    }
}

/// Linear interpolation from `a` (`t` = 0) to `b` (`t` = 1), alpha included, in sRGB.
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    // Premultiplied, so a clear end (often `TRANSPARENT`, which is clear black) only fades the
    // other color out rather than darkening it on the way: straight alpha would pass through
    // half-clear dark grey.
    let alpha = a.a + (b.a - a.a) * t;
    if alpha <= 0.0 {
        return Color::TRANSPARENT;
    }
    let channel = |x: f32, y: f32| (x * a.a + (y * b.a - x * a.a) * t) / alpha;
    Color {
        r: channel(a.r, b.r),
        g: channel(a.g, b.g),
        b: channel(a.b, b.b),
        a: alpha,
    }
}

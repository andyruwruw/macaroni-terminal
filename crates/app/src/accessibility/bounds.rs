//! Where widgets are on screen, by the ids of their containers: for opening an item's menu
//! from the keyboard under the item, and for telling screen readers where each item is.
//!
//! An [`Operation`] walks the laid-out tree. Children of a scrollable are laid out unscrolled,
//! so each scrollable's offset is taken off the bounds of what's inside it, and they're clipped
//! to what it shows.

use std::collections::HashMap;

use iced::advanced::widget::operation::{Operation, Outcome, Scrollable};
use iced::widget::Id;
use iced::{Rectangle, Task, Vector};

/// Finds where the container with id `id` is, in window coordinates; `None` when it isn't
/// laid out (or is scrolled out of sight).
pub fn find(id: Id) -> Task<Option<Rectangle>> {
    iced::advanced::widget::operate(Collect::new([id.clone()]))
        .map(move |mut found| found.remove(&id))
}

/// Finds where each of the containers with `ids` is; those not laid out (or scrolled out of
/// sight) are left out.
pub fn find_all(ids: impl IntoIterator<Item = Id>) -> Task<HashMap<Id, Rectangle>> {
    iced::advanced::widget::operate(Collect::new(ids))
}

/// The operation behind [`find`] and [`find_all`].
struct Collect {
    /// The ids looked for.
    wanted: HashMap<Id, ()>,
    /// What's been found, in window coordinates.
    found: HashMap<Id, Rectangle>,
    /// What to add to layout bounds here for window coordinates: the scrolling so far.
    offset: Vector,
    /// What's visible here: the scrollables' viewports so far.
    clip: Option<Rectangle>,
    /// The last scrollable's offset and viewport, which apply to the children it traverses next.
    entering: Option<(Vector, Rectangle)>,
}

impl Collect {
    /// Looks for `ids`.
    fn new(ids: impl IntoIterator<Item = Id>) -> Self {
        Self {
            wanted: ids.into_iter().map(|id| (id, ())).collect(),
            found: HashMap::new(),
            offset: Vector::ZERO,
            clip: None,
            entering: None,
        }
    }
}

impl Operation<HashMap<Id, Rectangle>> for Collect {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<HashMap<Id, Rectangle>>)) {
        let (offset, clip) = (self.offset, self.clip);
        if let Some((translation, viewport)) = self.entering.take() {
            let viewport = viewport + offset;
            self.clip = Some(match clip {
                Some(clip) => clip.intersection(&viewport).unwrap_or_default(),
                None => viewport,
            });
            self.offset = offset - translation;
        }
        operate(self);
        (self.offset, self.clip) = (offset, clip);
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        let Some(id) = id.filter(|id| self.wanted.contains_key(id)) else {
            return;
        };
        let bounds = bounds + self.offset;
        let shown = match self.clip {
            Some(clip) => clip.intersection(&bounds),
            None => Some(bounds),
        };
        if let Some(shown) = shown {
            self.found.insert(id.clone(), shown);
        }
    }

    fn scrollable(
        &mut self,
        _id: Option<&Id>,
        bounds: Rectangle,
        _content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        self.entering = Some((translation, bounds));
    }

    fn finish(&self) -> Outcome<HashMap<Id, Rectangle>> {
        Outcome::Some(self.found.clone())
    }
}

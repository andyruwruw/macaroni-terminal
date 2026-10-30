//! Wraps a widget to report where the pointer is relative to it, which `mouse_area` doesn't:
//! which half of it the cursor is over, and which edge it left through (for where a dragged
//! item would be inserted), and presses anywhere outside it (to cancel what it's editing).
//!
//! The view wraps each strip tab and sidebar workspace or group header in [`Pointer::on_zone`];
//! the app keeps the latest [`Zone`] (`App.hover_zone`) and `view::drop_target` /
//! `view::workspace_drop` turn it into a drop position. Gap zones are what let a drop land in
//! the space between two items. Presses outside ([`Pointer::on_press_outside`]) let go of a
//! clicked item waiting for Enter (`Message::ItemBlur`) and drop an open rename field
//! (`Message::RenameCancel`). Otherwise the wrapper is transparent: layout, drawing, operations and
//! overlays all go to the content.

use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::{Element, Event, Length, Rectangle, Size, Vector};

/// Which axis a widget is split along for [`Zone`]s.
#[derive(Debug, Clone, Copy)]
pub enum Halves {
    /// Left and right halves, for tabs in a strip.
    LeftRight,
    /// Top and bottom halves, for sidebar rows.
    TopBottom,
}

/// Where the cursor is along a widget's axis (`Halves`): in its first or second half, or just
/// gone out through its leading or trailing edge, into the gap beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// Just left through the leading edge (left or top).
    GapBefore,
    /// Over the leading half.
    Before,
    /// Over the trailing half.
    After,
    /// Just left through the trailing edge (right or bottom).
    GapAfter,
}

impl Zone {
    /// Whether the zone is on the trailing side, inside or in the gap.
    pub fn after(self) -> bool {
        matches!(self, Self::After | Self::GapAfter)
    }

    /// Whether the cursor has left the widget into a gap beside it.
    pub fn in_gap(self) -> bool {
        matches!(self, Self::GapBefore | Self::GapAfter)
    }
}

/// Builds the message for a [`Zone`] change.
type OnZone<'a, Message> = Box<dyn Fn(Zone) -> Message + 'a>;

/// The wrapper widget; build with [`Pointer::new`] and the `on_*` methods.
pub struct Pointer<'a, Message> {
    /// The wrapped widget.
    content: Element<'a, Message>,
    /// The axis and the message for zone changes, if reported.
    zones: Option<(Halves, OnZone<'a, Message>)>,
    /// The message for a press outside, if reported.
    outside: Option<Message>,
}

/// Per-widget state kept in the tree.
#[derive(Default)]
struct State {
    /// The half the cursor was last over, while it's over the widget.
    inside: Option<Zone>,
}

impl<'a, Message> Pointer<'a, Message> {
    /// Wraps `content`, reporting nothing until an `on_*` method is called.
    pub fn new(content: impl Into<Element<'a, Message>>) -> Self {
        Self {
            content: content.into(),
            zones: None,
            outside: None,
        }
    }

    /// Reports the cursor's `Zone` whenever it changes: which half it's over, and the edge it
    /// leaves through (leaving through a side along the axis reports nothing).
    pub fn on_zone(mut self, halves: Halves, on_zone: impl Fn(Zone) -> Message + 'a) -> Self {
        self.zones = Some((halves, Box::new(on_zone)));
        self
    }

    /// Reports a press of any mouse button outside the widget.
    pub fn on_press_outside(self, message: Message) -> Self {
        self.on_press_outside_maybe(Some(message))
    }

    /// Like [`Pointer::on_press_outside`], or reports nothing for `None`.
    pub fn on_press_outside_maybe(mut self, message: Option<Message>) -> Self {
        self.outside = message;
        self
    }
}

impl<Message: Clone> Widget<Message, iced::Theme, iced::Renderer> for Pointer<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        // Before the content, which may capture the press.
        if let (Some(message), Event::Mouse(mouse::Event::ButtonPressed(_))) =
            (&self.outside, event)
            && !cursor.is_over(bounds)
        {
            shell.publish(message.clone());
        }
        // A zone message is published only on a change, so the app isn't flooded on every move.
        // The gap is reported once, on the move that leaves, and only through an edge along
        // the axis (leaving a tab through its top or bottom reports nothing).
        if let (Some((halves, on_zone)), Event::Mouse(mouse::Event::CursorMoved { position })) =
            (&self.zones, event)
        {
            let state = tree.state.downcast_mut::<State>();
            let (at, start, end) = match halves {
                Halves::LeftRight => (position.x, bounds.x, bounds.x + bounds.width),
                Halves::TopBottom => (position.y, bounds.y, bounds.y + bounds.height),
            };
            if cursor.is_over(bounds) {
                let zone = if at > (start + end) / 2.0 {
                    Zone::After
                } else {
                    Zone::Before
                };
                if state.inside != Some(zone) {
                    state.inside = Some(zone);
                    shell.publish(on_zone(zone));
                }
            } else if state.inside.take().is_some() {
                if at < start {
                    shell.publish(on_zone(Zone::GapBefore));
                } else if at > end {
                    shell.publish(on_zone(Zone::GapAfter));
                }
            }
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message: Clone + 'a> From<Pointer<'a, Message>> for Element<'a, Message> {
    fn from(widget: Pointer<'a, Message>) -> Self {
        Element::new(widget)
    }
}

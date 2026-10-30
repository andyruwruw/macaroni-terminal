//! Wraps a widget to report a left-button drag across it as fractions of its size, which
//! `mouse_area` can't: its `on_move` stops at the widget's edge and its presses carry no
//! position.
//!
//! The settings dialog's color picker draws its saturation and value square and its hue and
//! alpha bars inside [`DragArea`]s. A press starts the drag, every move while held reports the
//! cursor clamped to the area (so dragging past an edge pins it there), and the release reports
//! once more so the caller can save what was picked.

use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector};

/// The wrapper widget.
pub struct DragArea<'a, Message> {
    /// The wrapped widget, drawn as is.
    content: Element<'a, Message>,
    /// Builds the message from the cursor's place across and down the area, each 0 to 1.
    on_drag: Box<dyn Fn(f32, f32) -> Message + 'a>,
    /// Sent when the button is let go after a drag.
    on_release: Option<Message>,
}

/// Whether a drag that started on the area is under way.
#[derive(Debug, Default)]
struct State {
    /// The left button went down on the area and hasn't come up yet.
    dragging: bool,
}

impl<'a, Message> DragArea<'a, Message> {
    /// Wraps `content`, reporting presses and drags on it with `on_drag`.
    pub fn new(
        content: impl Into<Element<'a, Message>>,
        on_drag: impl Fn(f32, f32) -> Message + 'a,
    ) -> Self {
        Self {
            content: content.into(),
            on_drag: Box::new(on_drag),
            on_release: None,
        }
    }

    /// Sends `message` when a drag ends.
    pub fn on_release(self, message: Message) -> Self {
        Self {
            on_release: Some(message),
            ..self
        }
    }

    /// The `on_drag` message for `position`, clamped into `bounds`.
    fn drag(&self, bounds: Rectangle, position: Point) -> Message {
        let x = ((position.x - bounds.x) / bounds.width.max(1.0)).clamp(0.0, 1.0);
        let y = ((position.y - bounds.y) / bounds.height.max(1.0)).clamp(0.0, 1.0);
        (self.on_drag)(x, y)
    }
}

impl<Message: Clone> Widget<Message, iced::Theme, iced::Renderer> for DragArea<'_, Message> {
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
        let bounds = layout.bounds();
        let dragging = tree.state.downcast_ref::<State>().dragging;
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(position) = cursor.position_over(bounds) {
                    tree.state.downcast_mut::<State>().dragging = true;
                    shell.publish(self.drag(bounds, position));
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if dragging => {
                shell.publish(self.drag(bounds, *position));
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if dragging => {
                tree.state.downcast_mut::<State>().dragging = false;
                if let Some(message) = self.on_release.clone() {
                    shell.publish(message);
                }
                shell.capture_event();
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        if tree.state.downcast_ref::<State>().dragging || cursor.is_over(layout.bounds()) {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::default()
        }
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

impl<'a, Message: Clone + 'a> From<DragArea<'a, Message>> for Element<'a, Message> {
    fn from(widget: DragArea<'a, Message>) -> Self {
        Element::new(widget)
    }
}

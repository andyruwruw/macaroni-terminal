//! Indents a widget and draws a thin line in the indent, as tall as the widget: the line beside
//! a sidebar group's workspaces.
//!
//! A line drawn as its own `height(Fill)` element can't do this: a `Fill` child makes the row
//! holding it ask for all the height its column has, which squeezes every other sidebar row.
//! [`SideLine`] lays its content out as usual (indented by its inset) and paints the line
//! behind it from the content's own bounds, so it never changes the layout.

use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::{Border, Color, Element, Event, Length, Rectangle, Size, Vector};

/// The wrapper widget.
pub struct SideLine<'a, Message> {
    /// What's indented.
    content: Element<'a, Message>,
    /// How far the content is indented.
    inset: f32,
    /// The line's left edge, from the widget's left.
    x: f32,
    /// The line's width.
    width: f32,
    /// The line's color.
    color: Color,
}

impl<'a, Message> SideLine<'a, Message> {
    /// `content`, indented by `inset`, with a `width` line of `color` at `x` inside the indent.
    pub fn new(
        content: impl Into<Element<'a, Message>>,
        inset: f32,
        x: f32,
        width: f32,
        color: Color,
    ) -> Self {
        Self {
            content: content.into(),
            inset,
            x,
            width,
            color,
        }
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for SideLine<'_, Message> {
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
        let inner = limits.shrink(Size::new(self.inset, 0.0));
        let child = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &inner)
            .move_to((self.inset, 0.0));
        let size = child.size();
        layout::Node::with_children(Size::new(size.width + self.inset, size.height), vec![child])
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        let child = layout
            .children()
            .next()
            .expect("SideLine lays out its content");
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], child, renderer, operation);
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
        let child = layout
            .children()
            .next()
            .expect("SideLine lays out its content");
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            child,
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
        let child = layout
            .children()
            .next()
            .expect("SideLine lays out its content");
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            child,
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
        use iced::advanced::Renderer as _;
        let bounds = layout.bounds();
        renderer.fill_quad(
            renderer::Quad {
                bounds: Rectangle {
                    x: bounds.x + self.x,
                    y: bounds.y,
                    width: self.width,
                    height: bounds.height,
                },
                border: Border::default(),
                ..renderer::Quad::default()
            },
            self.color,
        );
        let child = layout
            .children()
            .next()
            .expect("SideLine lays out its content");
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            child,
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
        let child = layout.children().next()?;
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            child,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message: 'a> From<SideLine<'a, Message>> for Element<'a, Message> {
    fn from(widget: SideLine<'a, Message>) -> Self {
        Element::new(widget)
    }
}

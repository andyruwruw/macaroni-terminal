//! An insertion line in the gap before a widget, where a dragged tab or workspace would land.
//! It's drawn outside the widget's bounds, so it takes no room and nothing shifts under the
//! cursor. `pin` can't do this: it hands its own bounds down as the viewport, so anything
//! placed outside them is culled.
//!
//! The view wraps the tab or sidebar row after the drop position in a [`GapLine`]
//! (`insertion_line` in the strip, `line_above` in the sidebar). Apart from drawing the line,
//! it passes everything to its content.

use iced::advanced::Renderer as _;
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::{Border, Color, Element, Event, Length, Rectangle, Size, Vector};

/// Which gap the line goes in, and so its orientation.
#[derive(Debug, Clone, Copy)]
pub enum Gap {
    /// Before it in a row: a vertical line left of the widget.
    Left,
    /// Before it in a column: a horizontal line above the widget.
    Top,
}

/// Draws `content` and an insertion line beside it; see the module docs.
pub struct GapLine<'a, Message> {
    /// The widget after the gap.
    content: Element<'a, Message>,
    /// Which gap the line goes in.
    gap: Gap,
    /// Where the line's middle is, from the widget's leading edge (negative is before it).
    center: f32,
    /// How far in from the widget's start the line begins, along the line (the sidebar indents
    /// it for drops into a group).
    inset: f32,
    /// The line's thickness.
    width: f32,
    /// The line's color (`Chrome.insertion`).
    color: Color,
}

impl<'a, Message> GapLine<'a, Message> {
    /// A line `width` thick across the gap before `content`, centered `center` from its edge.
    pub fn new(
        content: impl Into<Element<'a, Message>>,
        gap: Gap,
        center: f32,
        width: f32,
        color: Color,
    ) -> Self {
        Self {
            content: content.into(),
            gap,
            center,
            inset: 0.0,
            width,
            color,
        }
    }

    /// Starts the line `inset` in from the widget's start (top for [`Gap::Left`], left for
    /// [`Gap::Top`]); it still runs to the far end.
    pub fn inset(self, inset: f32) -> Self {
        Self { inset, ..self }
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for GapLine<'_, Message> {
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
        let bounds = layout.bounds();
        let start = self.center - self.width / 2.0;
        let line = match self.gap {
            Gap::Left => Rectangle {
                x: bounds.x + start,
                y: bounds.y + self.inset,
                width: self.width,
                height: bounds.height - self.inset,
            },
            Gap::Top => Rectangle {
                x: bounds.x + self.inset,
                y: bounds.y + start,
                width: bounds.width - self.inset,
                height: self.width,
            },
        };
        // Its own layer, over the sheets either side of the gap. The layer is clipped to the
        // parent's viewport, not this widget's bounds, which is what lets the line sit outside
        // them.
        renderer.with_layer(*viewport, |renderer| {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: line,
                    border: Border::default().rounded(self.width / 2.0),
                    ..renderer::Quad::default()
                },
                self.color,
            );
        });
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

impl<'a, Message: 'a> From<GapLine<'a, Message>> for Element<'a, Message> {
    fn from(widget: GapLine<'a, Message>) -> Self {
        Element::new(widget)
    }
}

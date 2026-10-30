//! A horizontal divider exactly one device pixel thick (more on high-density screens), across
//! the middle of its bounds: the menus' separators.
//!
//! A 1-unit-tall container isn't this: at fractional scale factors (125%, 150%) and at the
//! fractional positions menus open at, it covers one pixel in one place and blends across two
//! in another, so dividers in the same menu look different thicknesses. [`HairLine`] sizes the
//! line in device pixels and asks the renderer to snap it to the pixel grid.

use iced::advanced::widget::Tree;
use iced::advanced::{Layout, Widget, layout, mouse, renderer};
use iced::{Border, Color, Element, Length, Rectangle, Size};

/// The divider widget.
pub struct HairLine {
    /// The line's color.
    color: Color,
    /// Device pixels per logical unit, from the window.
    scale: f32,
}

impl HairLine {
    /// A `color` line for a window at `scale`, filling its parent's width and height (the line
    /// itself is drawn across the middle).
    pub fn new(color: Color, scale: f32) -> Self {
        Self { color, scale }
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for HairLine {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.resolve(Length::Fill, Length::Fill, Size::ZERO))
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        use iced::advanced::Renderer as _;
        let bounds = layout.bounds();
        let scale = self.scale.max(1.0);
        // Whole device pixels: one at 100% to 199%, two from 200%.
        let height = scale.floor() / scale;
        renderer.fill_quad(
            renderer::Quad {
                bounds: Rectangle {
                    y: bounds.center_y() - height / 2.0,
                    height,
                    ..bounds
                },
                border: Border::default(),
                // Snapped, a whole number of device pixels tall stays exactly that many.
                snap: true,
                ..renderer::Quad::default()
            },
            self.color,
        );
    }
}

impl<'a, Message: 'a> From<HairLine> for Element<'a, Message> {
    fn from(widget: HairLine) -> Self {
        Element::new(widget)
    }
}

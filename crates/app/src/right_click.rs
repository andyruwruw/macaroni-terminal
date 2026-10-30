//! Wraps a widget to report right clicks on it with the cursor's window position, which
//! `mouse_area` doesn't give: context menus open where the click was.
//!
//! The view wraps strip tabs, sidebar workspaces and group headers in [`RightClick`]; the
//! message opens `App.menu` at that point. Everything else (layout, drawing, operations,
//! overlays) goes straight to the content.

use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector};

/// The wrapper widget.
pub struct RightClick<'a, Message> {
    /// The wrapped widget.
    content: Element<'a, Message>,
    /// Builds the message from the click's position in window coordinates.
    on_right_click: Box<dyn Fn(Point) -> Message + 'a>,
    /// A left click opens it too (the title bar's workspace name).
    left: bool,
}

impl<'a, Message> RightClick<'a, Message> {
    /// Wraps `content`, reporting right presses on it with `on_right_click`.
    pub fn new(
        content: impl Into<Element<'a, Message>>,
        on_right_click: impl Fn(Point) -> Message + 'a,
    ) -> Self {
        Self {
            content: content.into(),
            on_right_click: Box::new(on_right_click),
            left: false,
        }
    }

    /// Reports left clicks the same way, for something whose only action is its menu.
    pub fn on_left_click(self) -> Self {
        Self { left: true, ..self }
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for RightClick<'_, Message> {
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
        // The content goes first, so a right click a child handles (a text field) stays its own.
        if shell.is_event_captured() {
            return;
        }
        if let Event::Mouse(mouse::Event::ButtonPressed(button)) = event
            && (*button == mouse::Button::Right || (self.left && *button == mouse::Button::Left))
            && let Some(position) = cursor.position_over(layout.bounds())
        {
            shell.publish((self.on_right_click)(position));
            shell.capture_event();
        }
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

impl<'a, Message: 'a> From<RightClick<'a, Message>> for Element<'a, Message> {
    fn from(widget: RightClick<'a, Message>) -> Self {
        Element::new(widget)
    }
}

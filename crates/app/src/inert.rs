//! Wraps a widget so it's drawn but can't be used: the settings dialog's live previews show the
//! real sidebar items, tabs and menus, built by the same view code, without clicks on them
//! reaching the app.
//!
//! [`Inert`] lays out and draws its content as usual and forwards nothing else: no events, no
//! operations (so a preview's text field can't take focus, and its scrollables don't answer for
//! the real ones by id), no overlays, and the default cursor.

use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse, renderer};
use iced::{Element, Event, Length, Rectangle, Size};

/// The wrapper widget.
pub struct Inert<'a, Message> {
    /// The widget shown.
    content: Element<'a, Message>,
}

impl<'a, Message> Inert<'a, Message> {
    /// Wraps `content`, shown but not interactive.
    pub fn new(content: impl Into<Element<'a, Message>>) -> Self {
        Self {
            content: content.into(),
        }
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Inert<'_, Message> {
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
        _tree: &mut Tree,
        _layout: Layout<'_>,
        _renderer: &iced::Renderer,
        _operation: &mut dyn Operation,
    ) {
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        _event: &Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        _shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        // No cursor, so nothing in the preview looks hovered.
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            mouse::Cursor::Unavailable,
            viewport,
        );
    }
}

impl<'a, Message: 'a> From<Inert<'a, Message>> for Element<'a, Message> {
    fn from(widget: Inert<'a, Message>) -> Self {
        Element::new(widget)
    }
}

//! The live preview pinned over a settings page's cards: the real window, sidebar rows, tab
//! strip, pane or menu, drawn by the same view code as the app around the dialog, so it changes
//! as soon as a setting is written and the config reloads (or, for a color being dragged in the
//! picker, as it's dragged).
//!
//! Each is wrapped in [`Inert`], so it can't be clicked and its widgets don't answer operations
//! meant for the real ones. [`App::settings_preview`] picks one by page; pages with nothing to
//! show get none.

use iced::widget::text::{LineHeight, Shaping};
use iced::widget::{Space, column, container, row, stack, text, text_input};
use iced::{Border, Color, Degrees, Element, Fill, Length, Size, alignment, gradient};
use macaroni_config::CursorStyle;
use macaroni_core::TabKind;

use super::Section;
use crate::icons::Icon;
use crate::inert::Inert;
use crate::state::{MenuPage, MenuTarget};
use crate::theme::{Chrome, to_iced};
use crate::view::widgets::{AddBox, add_box, card_style, icon_button, input_style};
use crate::view::{CUSTOM_FRAME, TitleLine, sidebar_edge, sidebar_margin, with_title_line};
use crate::{App, Message};

/// The most sidebar rows a preview shows.
const PREVIEW_ITEMS: usize = 3;

/// The width the tab strip preview is laid out for.
const STRIP_WIDTH: f32 = 640.0;

/// The pane preview's size.
const PANE_SIZE: Size = Size::new(560.0, 190.0);

/// What the font preview shows: prompts, code with ligature pairs, and box drawing.
const FONT_SAMPLE: [&str; 3] = [
    "~/projects/macaroni $ cargo test --workspace",
    "fn main() { let x = a >= b && c != d; } // => -> <=",
    "\u{2500}\u{2500} 0123456789 {}[]() @#$%&* \u{2514}\u{2500}\u{2518}",
];

/// The size of the window preview: the window's top left.
const SCREEN_SIZE: Size = Size::new(640.0, 240.0);

/// Inside a terminal-colored preview, around its text.
const TERMINAL_PADDING: f32 = 14.0;

/// What a page's preview shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Preview {
    /// The window's top left: the title bar, the sidebar's first rows and the focused pane.
    Screen,
    /// The sidebar's top: the search bar, the first few rows and the New workspace button.
    Sidebar,
    /// The focused pane's tab strip.
    Tabs,
    /// The sidebar rows over the tab strip, for settings that touch both.
    Items,
    /// The focused pane, tabs and all.
    Pane,
    /// A tab's right-click menu, beside a text field and the small buttons.
    Menus,
    /// Lines in the terminal font, and the cursor.
    Terminal,
}

/// How the dialog frames a preview at the top of its card.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Frame {
    /// Padded, on the interface's background.
    Padded,
    /// Edge to edge on this color (the terminal's background), as a terminal would be.
    Fill(Color),
}

/// `preview` with a 1 px `Chrome.outline` outline drawn over it, so it shows whatever the
/// preview's own colors and borders are (a container's own border would be covered by what's
/// inside it, such as the title bar or sidebar). Not `separator`, which follows the theme's
/// lines and can be as dark as the greys around it.
fn outlined<'a>(c: Chrome, preview: Element<'a, Message>) -> Element<'a, Message> {
    let outline =
        container(Space::new().width(Fill).height(Fill)).style(move |_| container::Style {
            border: Border {
                color: c.outline,
                width: 1.0,
                radius: c.s.controls.radius.into(),
            },
            ..container::Style::default()
        });
    // The stack takes the preview's size (its first layer), and the outline fills it.
    stack![preview, outline].into()
}

/// The preview for `section`'s page, if it has one.
fn preview_for(section: Section) -> Option<Preview> {
    use Preview::*;
    Some(match section {
        Section::Theme | Section::Window | Section::Quick => Screen,
        Section::Sidebar => Sidebar,
        Section::Tabs | Section::Agents => Tabs,
        Section::Panes => Pane,
        Section::Menus => Menus,
        Section::Effects => Items,
        Section::Terminal => Terminal,
        Section::General
        | Section::Notifications
        | Section::Shortcuts
        | Section::Browser
        | Section::VsCode => return None,
    })
}

impl App {
    /// The live preview for `section`'s page, if it has one, and how to frame it.
    pub(super) fn settings_preview(
        &self,
        section: Section,
    ) -> Option<(Element<'_, Message>, Frame)> {
        let c = self.chrome;
        let terminal = Frame::Fill(to_iced(self.palette.background, 1.0));
        let (content, frame): (Element<'_, Message>, Frame) = match preview_for(section)? {
            Preview::Screen => (self.screen_preview(), Frame::Padded),
            Preview::Sidebar => (self.sidebar_preview(), Frame::Padded),
            Preview::Tabs => (self.tabs_preview()?, Frame::Padded),
            Preview::Items => (
                column![self.sidebar_preview()]
                    .push(self.tabs_preview())
                    .spacing(16)
                    .into(),
                Frame::Padded,
            ),
            Preview::Pane => (self.pane_preview(PANE_SIZE)?, Frame::Padded),
            Preview::Menus => {
                let tab = self.model.focused_tab().id;
                (
                    row![
                        self.menu_list(MenuTarget::Tab(tab), MenuPage::Main, None),
                        controls_preview(c),
                    ]
                    .spacing(32)
                    .into(),
                    Frame::Padded,
                )
            }
            Preview::Terminal => (self.terminal_preview(), terminal),
        };
        Some((Inert::new(content).into(), frame))
    }

    /// The window's top left, as a screenshot of it would show, laid out as `App::window` lays
    /// out the real one: the title bar (where macaroni draws one), the sidebar with its search
    /// field, first rows and New workspace button, and the focused pane, on the window's
    /// background.
    fn screen_preview(&self) -> Element<'_, Message> {
        let c = self.chrome;
        let gap = if CUSTOM_FRAME {
            c.s.window.title_bar_gap
        } else {
            0.0
        };
        let title = if CUSTOM_FRAME {
            c.s.window.title_bar_height
        } else {
            0.0
        };
        // The width in effect, dragged or configured, as the window has it.
        let side = c.s.window.sidebar_width;
        let margin = sidebar_margin(c);
        let pane_size = Size::new(
            SCREEN_SIZE.width - side - margin,
            SCREEN_SIZE.height - title - gap,
        );
        let pane = self
            .pane_preview(pane_size)
            .or_else(|| self.tabs_preview())
            .unwrap_or_else(|| Space::new().into());
        let sidebar = container(self.sidebar_top())
            .width(side)
            .height(Fill)
            .style(move |_| card_style(c.sidebar, 0.0));
        let pane = container(pane).padding(iced::Padding {
            top: gap,
            left: margin,
            ..iced::Padding::ZERO
        });
        let sidebar = with_title_line(c, sidebar.into(), TitleLine::Sidebar);
        let pane = with_title_line(c, pane.into(), TitleLine::Panes);
        let body = column![]
            .push(CUSTOM_FRAME.then(|| with_title_line(c, self.title_bar(), TitleLine::TitleBar)))
            .push(stack![row![sidebar, pane]].push(sidebar_edge(c)));
        let background: iced::Background = match c.glass {
            // The backdrop shader can't be drawn inside the dialog; its gradient stands in.
            Some(_) => gradient::Linear::new(Degrees(135.0))
                .add_stop(0.0, c.backdrop[0])
                .add_stop(1.0, c.backdrop[1])
                .into(),
            None => c.background.into(),
        };
        outlined(
            c,
            container(body)
                .width(SCREEN_SIZE.width)
                .height(SCREEN_SIZE.height)
                .clip(true)
                .style(move |_| container::Style {
                    background: Some(background),
                    border: Border::default().rounded(c.s.controls.radius),
                    ..container::Style::default()
                })
                .into(),
        )
    }

    /// The sidebar's top as the sidebar lays it out: the search bar when it's shown, the first
    /// [`PREVIEW_ITEMS`] rows, and the New workspace button.
    fn sidebar_top(&self) -> Element<'_, Message> {
        let c = self.chrome;
        let items: Vec<_> = self
            .sidebar_items(None)
            .into_iter()
            .take(PREVIEW_ITEMS)
            .collect();
        let searching = self.sidebar_searching();
        column![]
            .push(searching.then(|| self.sidebar_search(c.s.sidebar.padding)))
            .push(self.sidebar_list(items, None, searching))
            .into()
    }

    /// [`App::sidebar_top`] on the sidebar's color, at its configured width.
    fn sidebar_preview(&self) -> Element<'_, Message> {
        let c = self.chrome;
        outlined(
            c,
            container(self.sidebar_top())
                // The configured width: the real sidebar may be dragged narrow or folded away.
                .width(self.config.style.window.sidebar_width)
                .style(move |_| card_style(c.sidebar, c.s.controls.radius))
                .into(),
        )
    }

    /// The focused pane's tab strip.
    fn tabs_preview(&self) -> Option<Element<'_, Message>> {
        let pane = self.model.pane(self.model.active_workspace().focused)?;
        Some(
            container(self.tab_strip(pane, STRIP_WIDTH))
                .width(Length::Fixed(STRIP_WIDTH))
                .into(),
        )
    }

    /// The focused pane, when its tab is a terminal (a browser page is a native window laid
    /// over the app, which a preview can't draw).
    fn pane_preview(&self, size: Size) -> Option<Element<'_, Message>> {
        let ws = self.model.active_workspace();
        let pane = self.model.pane(ws.focused)?;
        let tab = self.model.tab(pane.active)?;
        if matches!(tab.kind, TabKind::Browser(_)) {
            return None;
        }
        Some(
            container(self.pane(pane.id, true, None, size))
                .width(size.width)
                .height(size.height)
                .clip(true)
                .into(),
        )
    }

    /// A few lines in the terminal font, size, line height and colors, then a prompt with the
    /// cursor after it in the configured shape.
    fn terminal_preview(&self) -> Element<'_, Message> {
        let m = self.metrics;
        let p = &self.palette;
        let shaping = if m.ligatures {
            Shaping::Advanced
        } else {
            Shaping::Basic
        };
        let colors = [p.ansi[2], p.foreground, p.ansi[6]];
        let lines = FONT_SAMPLE.iter().zip(colors).map(|(line, color)| {
            text(*line)
                .font(m.font)
                .size(m.font_size)
                .line_height(LineHeight::Relative(m.line_height))
                .shaping(shaping)
                .color(to_iced(color, 1.0))
                .wrapping(text::Wrapping::None)
                .into()
        });
        let cell = Size::new(m.cell_width, m.cell_height);
        let cursor = to_iced(p.cursor, 1.0);
        let (width, height) = match self.config.terminal.cursor_style {
            CursorStyle::Block => (cell.width, cell.height),
            CursorStyle::Underline => (cell.width, 2.0),
            CursorStyle::Beam => (2.0, cell.height),
        };
        let shape = container(
            container(Space::new().width(width).height(height))
                .style(move |_| card_style(cursor, 0.0)),
        )
        .width(cell.width)
        .height(cell.height)
        .align_y(alignment::Vertical::Bottom);
        let prompt = text("~ $ ")
            .font(m.font)
            .size(m.font_size)
            .line_height(LineHeight::Absolute(m.cell_height.into()))
            .color(to_iced(p.foreground, 1.0));
        // The frame paints the terminal's background, edge to edge.
        container(column(lines).push(row![prompt, shape].align_y(alignment::Vertical::Center)))
            .padding(TERMINAL_PADDING)
            .width(Fill)
            .clip(true)
            .into()
    }
}

/// What the preview's buttons send: nothing, as [`Inert`] never lets them be pressed.
fn unsent() -> Message {
    Message::Commands(Vec::new())
}

/// A text field, a `+` box and small icon buttons.
fn controls_preview<'a>(c: Chrome) -> Element<'a, Message> {
    let field = text_input("A text field", "")
        .size(c.s.controls.field_text_size)
        .padding([6, 10])
        .width(200)
        .style(move |_, status| input_style(c, status));
    row![
        field,
        add_box(c, AddBox::Tab, c.s.tabs.height, c.s.tabs.height, unsent()),
        icon_button(c, Icon::Close, unsent()),
        icon_button(c, Icon::Up, unsent()),
        icon_button(c, Icon::Down, unsent()),
    ]
    .spacing(10)
    .align_y(alignment::Vertical::Center)
    .into()
}

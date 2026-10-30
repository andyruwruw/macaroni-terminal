//! Live previews at the top of the settings dialog's cards: the real sidebar items, tab strip,
//! pane, menu and title bar, drawn by the same view code as the app around the dialog, so they
//! change as soon as a setting is written and the config reloads.
//!
//! Each is wrapped in [`Inert`], so it can't be clicked and its widgets don't answer operations
//! meant for the real ones. [`App::settings_preview`] picks one by (section, card); cards with
//! nothing to show get none.

use iced::widget::text::{LineHeight, Shaping};
use iced::widget::{Space, column, container, row, stack, text, text_input};
use iced::{Border, Color, Degrees, Element, Fill, Length, Size, alignment, gradient};
use macaroni_config::CursorStyle;
use macaroni_core::TabKind;

use super::Section;
use crate::icons::{Icon, icon};
use crate::inert::Inert;
use crate::state::{MenuPage, MenuTarget};
use crate::theme::{Chrome, to_iced};
use crate::view::widgets::{AddBox, add_box, card_style, icon_button, input_style, one_line};
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

/// The size of the Interface preview: the top left of the window.
const SCREEN_SIZE: Size = Size::new(640.0, 300.0);

/// Inside a terminal-colored preview, around its text.
const TERMINAL_PADDING: f32 = 14.0;

/// What a card's preview shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Preview {
    /// The window's top left: the title bar, the sidebar's first rows and the focused pane.
    Screen,
    /// The first few sidebar rows: group headers and workspaces.
    Sidebar,
    /// The focused pane's tab strip.
    Tabs,
    /// The sidebar rows over the tab strip, for settings that touch both.
    Items,
    /// The focused pane, tabs and all.
    Pane,
    /// A tab's right-click menu.
    Menu,
    /// The title bar (Windows, where macaroni draws it).
    TitleBar,
    /// A text field and the small buttons.
    Controls,
    /// Lines in the terminal font.
    Font,
    /// The interface's text colors.
    Text,
    /// The accent colors.
    Accents,
    /// The cursor's shape.
    Cursor,
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

/// The preview for `section`'s card `group`, if it has one.
fn preview_for(section: Section, group: &str) -> Option<Preview> {
    use Preview::*;
    Some(match (section, group) {
        (Section::Appearance, "Interface" | "Theme colors") => Screen,
        (Section::Appearance, "Terminal colors" | "Terminal font") => Font,
        (Section::Appearance, "Accent colors") | (Section::Notifications, "Colors") => Accents,
        (Section::Appearance, "Text colors") => Text,
        (Section::Appearance, "Window" | "Window colors") if CUSTOM_FRAME => TitleBar,
        (Section::Appearance, "Window" | "Window colors") => Screen,
        (Section::Appearance, "Sidebar" | "Workspaces" | "Groups") => Sidebar,
        (Section::Appearance, "Tabs") => Tabs,
        (Section::Appearance, "Workspace and tab colors") => Items,
        (Section::Appearance, "Panes" | "Pane colors") => Pane,
        (Section::Appearance, "Menus" | "Menu and dialog colors") => Menu,
        (Section::Appearance, "Buttons, fields and outlines") => Controls,
        (Section::Appearance, "Effects" | "Glass colors" | "Backdrop colors") => Items,
        (Section::Theme | Section::Quick, _) => Screen,
        (Section::Terminal, "Cursor") => Cursor,
        (Section::Items, "Workspaces") => Sidebar,
        (Section::Items, "Tabs") | (Section::Agents, "In tabs") => Tabs,
        (Section::Items, "Colors") => Items,
        _ => return None,
    })
}

impl App {
    /// The live preview at the top of `section`'s card `group`, if it has one, and how to frame
    /// it.
    pub(super) fn settings_preview(
        &self,
        section: Section,
        group: &str,
    ) -> Option<(Element<'_, Message>, Frame)> {
        let c = self.chrome;
        let terminal = Frame::Fill(to_iced(self.palette.background, 1.0));
        let (content, frame): (Element<'_, Message>, Frame) = match preview_for(section, group)? {
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
            Preview::Menu => {
                let tab = self.model.focused_tab().id;
                (
                    self.menu_list(MenuTarget::Tab(tab), MenuPage::Main, None),
                    Frame::Padded,
                )
            }
            Preview::TitleBar => (
                outlined(
                    c,
                    container(self.title_bar())
                        .width(Fill)
                        .style(move |_| card_style(c.background, c.s.controls.radius))
                        .into(),
                ),
                Frame::Padded,
            ),
            Preview::Controls => (controls_preview(c), Frame::Padded),
            Preview::Font => (self.font_preview(), terminal),
            Preview::Text => (text_preview(c), Frame::Padded),
            Preview::Accents => (accents_preview(c), Frame::Padded),
            Preview::Cursor => (self.cursor_preview(), terminal),
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
        let items: Vec<_> = self
            .sidebar_items(None)
            .into_iter()
            .take(PREVIEW_ITEMS)
            .collect();
        let searching = self.sidebar_searching();
        let sidebar = container(
            column![]
                .push(searching.then(|| self.sidebar_search(c.s.sidebar.padding)))
                .push(self.sidebar_list(items, None, searching)),
        )
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

    /// The first [`PREVIEW_ITEMS`] sidebar rows, as wide and spaced as in the sidebar.
    fn sidebar_preview(&self) -> Element<'_, Message> {
        let c = self.chrome;
        let items: Vec<_> = self
            .sidebar_items(None)
            .into_iter()
            .take(PREVIEW_ITEMS)
            .collect();
        outlined(
            c,
            container(column(items).spacing(c.s.sidebar.spacing))
                .padding(c.s.sidebar.padding)
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

    /// A few lines in the terminal font, size, line height and colors.
    fn font_preview(&self) -> Element<'_, Message> {
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
        // The frame paints the terminal's background, edge to edge.
        container(column(lines))
            .padding(TERMINAL_PADDING)
            .width(Fill)
            .clip(true)
            .into()
    }

    /// A prompt with the cursor after it, in the configured shape.
    fn cursor_preview(&self) -> Element<'_, Message> {
        let m = self.metrics;
        let p = &self.palette;
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
        container(row![prompt, shape].align_y(alignment::Vertical::Center))
            .padding(TERMINAL_PADDING)
            .width(Fill)
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

/// Each of the interface's text colors, named in itself.
fn text_preview<'a>(c: Chrome) -> Element<'a, Message> {
    row![
        one_line("Text", 14.0, c.text),
        one_line("Labels", 14.0, c.label),
        one_line("Muted text", 14.0, c.muted),
        one_line("Faint text", 14.0, c.faint),
    ]
    .spacing(24)
    .into()
}

/// The accent colors where they show: an attention bell, a working agent, an error and a drop
/// line.
fn accents_preview<'a>(c: Chrome) -> Element<'a, Message> {
    let line = container(
        Space::new()
            .width(60)
            .height(c.s.controls.insertion_line_width),
    )
    .style(move |_| card_style(c.insertion, 1.0));
    let chip = |glyph, label, color| {
        row![icon(glyph, 15.0, color), one_line(label, 13.0, color)]
            .spacing(6)
            .align_y(alignment::Vertical::Center)
    };
    let error = container(one_line("Something went wrong", 12.0, c.text))
        .padding([4, 10])
        .style(move |_| container::Style {
            border: Border {
                color: c.danger,
                width: 1.0,
                radius: c.s.controls.radius.into(),
            },
            ..container::Style::default()
        });
    row![
        chip(Icon::Bell, "Needs you", c.attention),
        chip(Icon::Sparkles, "Working", c.working),
        error,
        line,
    ]
    .spacing(24)
    .align_y(alignment::Vertical::Center)
    .into()
}

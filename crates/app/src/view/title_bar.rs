//! The custom title bar, the banners over the workspace, and the window's resize edges.
//!
//! Only with [`super::CUSTOM_FRAME`] (Windows) does `App::window` draw [`App::title_bar`] and
//! `App::view` lay [`resize_edges`] over everything: without the system frame nothing else
//! moves or resizes the window. The buttons send [`Message::Window`], which `update` turns into
//! `iced::window` calls. The banners are the Git Bash offer (a row above the panes) and the
//! config error banner (floating over the panes' bottom edge, placed by `App::workspace_area`).

use iced::widget::{Space, button, column, container, hover, mouse_area, row, text};
use iced::{Background, Border, Color, Element, Fill, Length, alignment, mouse, window};

use super::widgets::{
    NOTICE_PADDING, card_style, flat_button, icon_button, notice_card, one_line, shadow,
};
use super::workspace_origin;
use crate::icons::{Icon, glyph, icon, workspace_icon};
use crate::keymap::Action;
use crate::right_click::RightClick;
use crate::state::MenuTarget;
use crate::theme::{Chrome, workspace_color};
use crate::{App, Message, WindowOp, glass};

/// How far inside the workspace's edges the config error banner floats.
pub(super) const BANNER_MARGIN: f32 = 12.0;

/// The config error banner's widest, centered in the workspace.
const BANNER_WIDTH: f32 = 640.0;

/// The banners' text size.
const BANNER_TEXT_SIZE: f32 = 12.0;

/// Inside the Git Bash offer's buttons, above and below then left and right.
const BANNER_BUTTON_PADDING: [f32; 2] = [4.0, 10.0];

/// Between the config error banner's messages.
const BANNER_LINE_SPACING: f32 = 2.0;

/// The config error banner's shadow: its alpha, how far it falls, and its blur.
const BANNER_SHADOW: (f32, f32, f32) = (0.25, 2.0, 8.0);

/// The logo column's least width, so the logo and the sidebar button fit while the sidebar
/// is narrow or folded away.
const LOGO_COLUMN_MIN: f32 = 150.0;

/// The sidebar button's icon size.
const SIDEBAR_BUTTON_ICON: f32 = 15.0;

/// Inside the workspace name's hover highlight, above and below then left and right.
const NAME_PADDING: [f32; 2] = [4.0, 8.0];

/// Between the parts of the title bar's logo and workspace name, and of a banner.
const TITLE_SPACING: f32 = 8.0;

/// "macaroni" has no ascenders, so its middle is below its line's: the logo drops this much to
/// line up with it.
const LOGO_DROP: f32 = 1.0;

/// The caption icon size the glyph sizes below are drawn at; they scale with
/// `[style.window] caption_icon_size` from it.
const CAPTION_DESIGN_SIZE: f32 = 14.0;

/// The settings gear's size at [`CAPTION_DESIGN_SIZE`]. Lucide's square and copy fill more of
/// their box than its x, so they draw smaller to look the same size; the gear and x draw a
/// little bigger.
const SETTINGS_GLYPH: f32 = 15.0;

/// The minimize dash's size at [`CAPTION_DESIGN_SIZE`].
const MINIMIZE_GLYPH: f32 = 14.0;

/// The maximize square's size at [`CAPTION_DESIGN_SIZE`].
const MAXIMIZE_GLYPH: f32 = 11.5;

/// The restore glyph's (two squares) size at [`CAPTION_DESIGN_SIZE`].
const RESTORE_GLYPH: f32 = 12.0;

/// The close x's size at [`CAPTION_DESIGN_SIZE`].
const CLOSE_GLYPH: f32 = 15.0;

impl App {
    /// The logo and app name, the active workspace's name over the panes, room to drag the
    /// window by, and the window buttons. Glass draws it as a square-cornered frosted sheet.
    pub(super) fn title_bar(&self) -> Element<'_, Message> {
        let c = self.chrome;
        let grip = mouse_area(
            row![
                // As wide as the sidebar plus the first pane's half-gap inset, so what follows
                // starts where the pane card does; never too narrow for the logo and the
                // sidebar button.
                container(
                    row![title_logo(c), Space::new().width(Fill)]
                        // Only while the sidebar is folded away (Ctrl+B also brings it back).
                        .push(self.sidebar.collapsed.then(|| sidebar_button(c)))
                        .align_y(alignment::Vertical::Center),
                )
                .padding(iced::Padding {
                    left: c.s.window.logo_padding,
                    right: c.s.window.logo_padding / 2.0,
                    ..iced::Padding::ZERO
                })
                .width(workspace_origin(c).x.max(LOGO_COLUMN_MIN)),
                // Lined up with the icon and title of the first pane's first tab below (the
                // hover highlight's padding comes out of the indent).
                container(self.title_workspace_name())
                    .padding(iced::Padding {
                        left: (c.s.tabs.padding_left - NAME_PADDING[1]).max(0.0),
                        ..iced::Padding::ZERO
                    })
                    .width(Fill)
                    .clip(true),
            ]
            .height(Fill)
            .align_y(alignment::Vertical::Center),
        )
        .on_press(Message::Window(WindowOp::Drag))
        .on_double_click(Message::Window(WindowOp::ToggleMaximize));
        let (maximize, maximize_size) = if self.maximized {
            (Icon::Restore, RESTORE_GLYPH)
        } else {
            (Icon::Maximize, MAXIMIZE_GLYPH)
        };
        let bar: Element<'_, Message> = row![
            grip,
            caption_button(
                c,
                Icon::Settings,
                SETTINGS_GLYPH,
                Message::Action(Action::OpenSettings),
                Some(c.hover),
            ),
            caption_button(
                c,
                Icon::Minimize,
                MINIMIZE_GLYPH,
                Message::Window(WindowOp::Minimize),
                Some(c.hover),
            ),
            caption_button(
                c,
                maximize,
                maximize_size,
                Message::Window(WindowOp::ToggleMaximize),
                Some(c.hover),
            ),
            caption_button(
                c,
                Icon::Close,
                CLOSE_GLYPH,
                Message::CloseRequested,
                Some(c.close_hover),
            ),
        ]
        .height(c.s.window.title_bar_height)
        .into();
        let bar = container(bar)
            .width(Fill)
            .style(move |_| card_style(c.title_bar, 0.0));
        if c.glass.is_some() {
            glass::frosted(c, bar, 0.0, 1.0, 0.0)
        } else {
            bar.into()
        }
    }

    /// The active workspace's icon and name, lit on hover. A click (either button) opens the
    /// workspace's menu, the same one as in the sidebar.
    fn title_workspace_name(&self) -> Element<'_, Message> {
        let c = self.chrome;
        let ws = self.model.active_workspace();
        let id = ws.id;
        let name = |lit: bool| -> Element<'_, Message> {
            container(
                row![
                    glyph(
                        workspace_icon(ws.icon.as_deref()),
                        c.s.window.workspace_icon_size,
                        workspace_color(ws.color.as_deref()).unwrap_or(c.label),
                        1.0,
                    ),
                    one_line(ws.name.as_str(), c.s.window.workspace_name_size, c.text),
                ]
                .spacing(TITLE_SPACING)
                .align_y(alignment::Vertical::Center),
            )
            .padding(NAME_PADDING)
            .style(move |_| {
                card_style(
                    if lit { c.hover } else { Color::TRANSPARENT },
                    c.s.controls.radius,
                )
            })
            .into()
        };
        // Not a button: a press on it is the menu's, and must not start dragging the window.
        RightClick::new(
            mouse_area(hover(name(false), name(true))).interaction(mouse::Interaction::Pointer),
            move |at| Message::MenuOpen(MenuTarget::Workspace(id), at),
        )
        .on_left_click()
        .into()
    }

    /// On Windows without Git Bash: offer to install it, since it's the default shell when
    /// present. `None` once dismissed or when there's nothing to offer.
    pub(super) fn git_bash_offer(&self) -> Option<Element<'_, Message>> {
        if !self.offer_git_bash {
            return None;
        }
        let c = self.chrome;
        let action = |label: &'static str, message: Message, primary: bool| {
            button(text(label).size(BANNER_TEXT_SIZE).color(c.text))
                .on_press(message)
                .style(move |_, status| flat_button(c, status, primary.then_some(c.hover)))
                .padding(BANNER_BUTTON_PADDING)
        };
        Some(
            notice_card(
                c,
                c.attention,
                row![
                    text(
                        "Want Unix commands (ls, mkdir, rm -rf, grep)? macaroni uses Git Bash for new \
                         terminals once Git for Windows is installed."
                    )
                    .size(BANNER_TEXT_SIZE)
                    .color(c.text)
                    .width(Fill),
                    action("Install Git Bash", Message::InstallGitBash, true),
                    action("Not now", Message::DismissShellOffer, false),
                ]
                .spacing(TITLE_SPACING)
                .align_y(alignment::Vertical::Center),
            )
            .into(),
        )
    }

    /// Config problems, shown until fixed or dismissed; `None` when there are none. The last
    /// good config stays in effect.
    pub(super) fn banner(&self) -> Option<Element<'_, Message>> {
        if self.config_errors.is_empty() {
            return None;
        }
        let c = self.chrome;
        let messages = column(self.config_errors.iter().map(|e| {
            text(e.as_str())
                .size(BANNER_TEXT_SIZE)
                .font(crate::fonts::PRIMARY)
                .color(c.text)
                .into()
        }))
        .spacing(BANNER_LINE_SPACING)
        .width(Fill);
        let (alpha, y, blur) = BANNER_SHADOW;
        Some(
            container(
                row![
                    messages,
                    icon_button(c, Icon::Close, Message::DismissConfigErrors)
                ]
                .spacing(TITLE_SPACING)
                .align_y(alignment::Vertical::Center),
            )
            .padding(NOTICE_PADDING)
            .max_width(BANNER_WIDTH)
            .style(move |_| container::Style {
                background: Some(c.surface.into()),
                border: Border {
                    color: c.danger,
                    width: c.s.controls.border_width,
                    radius: c.s.controls.radius.into(),
                },
                shadow: shadow(Color::from_rgba(0.0, 0.0, 0.0, alpha), y, blur),
                ..container::Style::default()
            })
            .into(),
        )
    }
}

/// The logo and app name.
fn title_logo<'a>(c: Chrome) -> Element<'a, Message> {
    row![
        container(icon(Icon::Logo, c.s.window.logo_size, c.logo)).padding(iced::Padding {
            top: LOGO_DROP,
            ..iced::Padding::ZERO
        }),
        one_line("macaroni", c.s.window.app_name_size, c.text).font(crate::fonts::LOGO)
    ]
    .spacing(TITLE_SPACING)
    .align_y(alignment::Vertical::Center)
    .into()
}

/// Folds the sidebar away or brings it back (Ctrl+B).
fn sidebar_button<'a>(c: Chrome) -> Element<'a, Message> {
    button(icon(Icon::PanelLeft, SIDEBAR_BUTTON_ICON, c.label))
        .on_press(Message::Action(Action::ToggleSidebar))
        .style(move |_, status| flat_button(c, status, None))
        .padding(5)
        .into()
}

/// A window button with `glyph` drawn `size` at the default caption icon size. `hover` is its
/// hovered background; `None` lights it like other flat buttons.
fn caption_button<'a>(
    c: Chrome,
    glyph: Icon,
    size: f32,
    message: Message,
    hover: Option<Color>,
) -> Element<'a, Message> {
    // The tuned sizes keep their proportions at any `caption_icon_size`.
    let size = size * c.s.window.caption_icon_size / CAPTION_DESIGN_SIZE;
    button(container(icon(glyph, size, c.label)).center(Fill))
        .on_press(message)
        .width(c.s.window.caption_button_width)
        .height(Fill)
        .style(move |_, status| match hover {
            Some(color) => button::Style {
                background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                    .then_some(Background::Color(color)),
                ..button::Style::default()
            },
            None => flat_button(c, status, None),
        })
        .into()
}

/// Mouse areas along the window's edges and corners that resize it: without a system frame,
/// nothing else does.
pub(super) fn resize_edges<'a>(c: Chrome) -> Element<'a, Message> {
    use window::Direction as D;
    let edge = |direction: D, width: Length, height: Length, interaction| {
        mouse_area(Space::new().width(width).height(height))
            .on_press(Message::Window(WindowOp::Resize(direction)))
            .interaction(interaction)
    };
    let e = Length::Fixed(c.s.window.resize_border);
    let corner = |direction, interaction| edge(direction, e, e, interaction);
    let horizontal = mouse::Interaction::ResizingHorizontally;
    let vertical = mouse::Interaction::ResizingVertically;
    let down = mouse::Interaction::ResizingDiagonallyDown;
    let up = mouse::Interaction::ResizingDiagonallyUp;
    // A 3x3 grid over the window; the middle cell is an empty `Space`, which reports no
    // interaction, so the `stack` passes clicks there through to the window under it.
    column![
        row![
            corner(D::NorthWest, down),
            edge(D::North, Fill, e, vertical),
            corner(D::NorthEast, up),
        ],
        row![
            edge(D::West, e, Fill, horizontal),
            Space::new().width(Fill).height(Fill),
            edge(D::East, e, Fill, horizontal),
        ]
        .height(Fill),
        row![
            corner(D::SouthWest, up),
            edge(D::South, Fill, e, vertical),
            corner(D::SouthEast, down),
        ],
    ]
    .into()
}

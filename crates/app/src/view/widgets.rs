//! Small styling helpers shared by the view's parts: button, field and card styles, the `+` box,
//! icon buttons, item fills for the selection and hover fades, and one-line text.
//!
//! Everything takes the [`Chrome`] (`c`) for its colors and `[style.*]` sizes, so the sidebar,
//! tab strip, menus and banners look alike in both interfaces.

use iced::widget::{button, container, text, text_input};
use iced::{Background, Border, Color, Element, Fill, Length, Padding, Shadow, Vector};

use crate::anim::mix;
use crate::icons::{Icon, icon};
use crate::theme::Chrome;
use crate::{Message, anim, glass};

/// iced's default line height, as a multiple of the text size.
pub(super) const LINE_HEIGHT: f32 = 1.3;

/// Around a small icon button's icon.
pub(super) const ICON_BUTTON_PADDING: f32 = 5.0;

/// How much bigger a mark is drawn than the text beside it, so the two look the same size.
pub(super) const MARK_OVER_TEXT: f32 = 1.0;

/// Inside a text field, above and below then left and right.
pub(super) const FIELD_PADDING: [f32; 2] = [6.0, 10.0];

/// Inside a banner card, above and below then left and right.
pub(super) const NOTICE_PADDING: [f32; 2] = [8.0, 12.0];

/// How strongly the `+` box's outline shows, hovered.
const ADD_OUTLINE_HOVER: f32 = 0.5;

/// How strongly the `+` box's outline shows, not hovered.
const ADD_OUTLINE_IDLE: f32 = 0.25;

/// How strongly selected text in a field is tinted with the attention color.
const SELECTION_ALPHA: f32 = 0.35;

/// A drop shadow `blur` wide, falling `y` below what casts it.
pub(super) fn shadow(color: Color, y: f32, blur: f32) -> Shadow {
    Shadow {
        color,
        offset: Vector::new(0.0, y),
        blur_radius: blur,
    }
}

/// A borderless button's style: `base` as its background (none if `None`), `hover` while
/// hovered or pressed.
pub(super) fn flat_button(c: Chrome, status: button::Status, base: Option<Color>) -> button::Style {
    let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: if hover {
            Some(Background::Color(c.hover))
        } else {
            base.map(Background::Color)
        },
        text_color: c.text,
        border: Border {
            radius: c.s.controls.radius.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// Which `+` box [`add_box`] draws, for its colors and corners.
#[derive(Debug, Clone, Copy)]
pub(super) enum AddBox {
    /// New workspace, under the sidebar's workspaces.
    Workspace,
    /// New tab, after a pane's tabs.
    Tab,
}

/// A `+` box in a `width` by `height` slot like the items it adds. In glass it's a dim outline
/// that sits as far inside the slot as a resting item's sheet does (`glass::lift_padding` at
/// rest), so it's exactly as tall as the tabs or workspaces beside it, not the slot. In solid it
/// takes its own background and border (`add_workspace`, `add_tab` and their `_border`s), and
/// hovered, the hovered workspace's or tab's.
pub(super) fn add_box<'a>(
    c: Chrome,
    kind: AddBox,
    width: impl Into<Length>,
    height: f32,
    message: Message,
) -> Element<'a, Message> {
    let inset = glass::lift_padding(c, Padding::ZERO, 0.0).0;
    let (radius, fill, border, hover_fill, hover_border) = match kind {
        AddBox::Workspace => (
            c.s.workspaces.add_radius,
            c.add_workspace,
            c.add_workspace_border,
            c.workspace_hover,
            c.workspace_border_hover,
        ),
        AddBox::Tab => (
            c.s.tabs.add_radius,
            c.add_tab,
            c.add_tab_border,
            c.tab_hover,
            c.tab_border_hover,
        ),
    };
    let button = button(
        container(
            icon(Icon::Plus, c.s.controls.icon_button_size, c.muted).opacity(c.s.effects.dim),
        )
        .center(Fill),
    )
    .on_press(message)
    .width(Fill)
    .height(Fill)
    .padding(0)
    .style(move |_, status| {
        let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let (background, color) = match (c.glass, hover) {
            (Some(_), _) => (
                hover.then_some(c.hover),
                c.outline.scale_alpha(if hover {
                    ADD_OUTLINE_HOVER
                } else {
                    ADD_OUTLINE_IDLE
                }),
            ),
            (None, false) => (Some(fill), border),
            (None, true) => (Some(hover_fill), hover_border),
        };
        button::Style {
            background: background.map(Background::Color),
            border: Border {
                color,
                width: c.s.controls.border_width,
                radius: radius.into(),
            },
            ..button::Style::default()
        }
    });
    container(button)
        .padding(inset)
        .width(width)
        .height(height)
        .into()
}

/// A small flat button showing `glyph`, sending `message` when clicked.
pub(super) fn icon_button<'a>(c: Chrome, glyph: Icon, message: Message) -> Element<'a, Message> {
    faded_icon_button(c, glyph, message, 1.0)
}

/// [`icon_button`] with its icon at `opacity`, for buttons on items that dim when unselected.
/// Opacity, not the color's alpha, since iced's svg tint ignores alpha.
pub(super) fn faded_icon_button<'a>(
    c: Chrome,
    glyph: Icon,
    message: Message,
    opacity: f32,
) -> Element<'a, Message> {
    button(icon(glyph, c.s.controls.icon_button_size, c.muted).opacity(opacity))
        .on_press(message)
        .style(move |_, status| flat_button(c, status, None))
        .padding(ICON_BUTTON_PADDING)
        .into()
}

/// A sidebar item's or tab's background. Solid: `base`, eased to `selected` as it's selected
/// and to `hover` as it's hovered ([`item_ease`]). Glass: only a faint `surface` tint, lit on
/// hover, over the frosted sheet.
pub(super) fn item_fill(
    c: Chrome,
    levels: anim::Levels,
    base: Color,
    selected: Color,
    hover: Color,
) -> Color {
    match c.glass {
        Some(_) => Color {
            a: c.surface.a * levels.selected + c.hover.a * hover_level(levels),
            ..c.hover
        },
        None => item_ease(levels, base, selected, hover),
    }
}

/// `base`, eased to `selected` by the item's selection and to `hover` by its hover: a solid
/// item's fill or border.
pub(super) fn item_ease(levels: anim::Levels, base: Color, selected: Color, hover: Color) -> Color {
    mix(
        mix(base, selected, levels.selected),
        hover,
        hover_level(levels),
    )
}

/// How hovered an item looks: hover fades out as selection fades in, so a selected item under
/// the cursor doesn't light up further.
fn hover_level(levels: anim::Levels) -> f32 {
    levels.hovered * (1.0 - levels.selected)
}

/// How strong a frosted item's sheet is: `style.effects.glass_idle` unselected, eased up to 1
/// as it's hovered (partly, by `style.effects.hover`) and selected.
pub(super) fn glass_strength(c: Chrome, levels: anim::Levels) -> f32 {
    c.s.effects.glass_idle + (1.0 - c.s.effects.glass_idle) * levels.emphasis(c.s.effects.hover)
}

/// An icon on a small tile (`Chrome.workspace_icon`, or clear without a `background`), as in
/// the sidebar. The tile keeps its size either way, so rows don't move.
pub(super) fn icon_tile<'a>(
    c: Chrome,
    icon: impl Into<Element<'a, Message>>,
    background: bool,
) -> Element<'a, Message> {
    let fill = if background {
        c.workspace_icon
    } else {
        Color::TRANSPARENT
    };
    container(icon)
        .center(c.s.workspaces.tile_size)
        .style(move |_| card_style(fill, c.s.workspaces.tile_radius))
        .into()
}

/// A plain container style: `background` with corners rounded by `radius`, no border.
pub(super) fn card_style(background: Color, radius: f32) -> container::Style {
    container::Style {
        background: Some(background.into()),
        border: Border {
            radius: radius.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// A banner in a card, edged in `tone`, with the workspace's gap on its right and below it, so it
/// lines up with the pane cards under it.
pub(super) fn notice_card<'a>(
    c: Chrome,
    tone: Color,
    content: impl Into<Element<'a, Message>>,
) -> container::Container<'a, Message> {
    let card = container(content)
        .padding(NOTICE_PADDING)
        .width(Fill)
        .style(move |_| container::Style {
            background: Some(c.surface.into()),
            border: Border {
                color: tone,
                width: c.s.controls.border_width,
                radius: c.s.controls.radius.into(),
            },
            ..container::Style::default()
        });
    container(card).padding(Padding {
        top: 0.0,
        right: c.s.panes.margin,
        bottom: c.s.panes.margin,
        left: 0.0,
    })
}

/// A text field's style: sunken, outlined only while focused.
pub(super) fn input_style(c: Chrome, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: c.sunken.into(),
        border: Border {
            color: if focused {
                c.outline
            } else {
                Color::TRANSPARENT
            },
            width: c.s.controls.border_width,
            radius: c.s.controls.radius.into(),
        },
        icon: c.muted,
        placeholder: c.faint,
        value: c.text,
        selection: c.attention.scale_alpha(SELECTION_ALPHA),
    }
}

/// One line of text that never wraps; the caller clips it.
pub(super) fn one_line<'a>(
    content: impl text::IntoFragment<'a>,
    size: f32,
    color: Color,
) -> text::Text<'a> {
    text(content)
        .size(size)
        .color(color)
        .wrapping(text::Wrapping::None)
}

/// `title` cut to at most `max` characters, the last an ellipsis when it's cut. Counts chars,
/// not widths, so it's only a cap; the caller still clips.
///
/// # Panics
///
/// When `max` is 0 and `title` isn't empty (subtraction overflow, in debug builds). Callers pass
/// constants or `[style.*]` limits whose minimum is 4.
pub(super) fn truncate(title: &str, max: usize) -> String {
    if title.chars().count() <= max {
        title.to_owned()
    } else {
        let mut short: String = title.chars().take(max - 1).collect();
        short.push('…');
        short
    }
}

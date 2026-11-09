//! The settings dialog's sound menu: a popup opened by clicking a sound row's button
//! (`[notifications] sound` and `bell_sound`). Off, the system's sound, then macaroni's own
//! under a heading per kind ([`crate::sounds::KINDS`]), then a custom file.
//!
//! Each sound has a play button that only plays it, so sounds can be tried one after another
//! with the menu open; clicking a sound's name picks it and closes the menu. [`SoundMenu`] (in
//! `Settings.sounds` while open) keeps what the row showed when it was opened. The popup sits
//! by the button, kept inside the window, over a layer that closes it on a click elsewhere (or
//! Escape).

use iced::widget::{Space, button, column, container, mouse_area, opaque, pin, row, stack};
use iced::{Element, Fill, Point, Size, alignment};
use macaroni_config::Value;

use super::SettingsMessage;
use crate::Message;
use crate::icons::{Icon, icon};
use crate::theme::Chrome;
use crate::view::widgets::{card_style, flat_button, one_line, shadow, thin_scroll};

/// The popup's width.
const WIDTH: f32 = 240.0;

/// The tallest the popup gets; the list scrolls past it.
const MAX_HEIGHT: f32 = 420.0;

/// Around the popup's list.
const PADDING: f32 = 6.0;

/// Kept between the popup and the window's edges.
const MARGIN: f32 = 12.0;

/// How far below the clicked button the popup opens.
const OFFSET: f32 = 12.0;

/// A sound's row height.
const ROW_HEIGHT: f32 = 28.0;

/// A heading's height.
const HEADING_HEIGHT: f32 = 26.0;

/// The menu's text size.
const TEXT_SIZE: f32 = 12.5;

/// The play and check icons' size.
const ICON_SIZE: f32 = 13.0;

/// What a sound row hands the menu when it opens.
#[derive(Debug, Clone)]
pub struct SoundMenu {
    /// The dotted config key it writes.
    pub key: &'static str,
    /// Where the row's button was clicked, in window coordinates.
    pub at: Point,
    /// The value in effect.
    pub value: String,
    /// Each choice's label and value: Off, the system's sound, then the built-in sounds in
    /// [`crate::sounds::BUILT_IN`]'s order.
    pub presets: Vec<(String, String)>,
}

/// A message for the dialog.
fn send(message: SettingsMessage) -> Message {
    Message::Settings(message)
}

/// The popup over a layer that closes it on a click elsewhere, kept inside `window`.
pub fn menu_layer<'a>(c: Chrome, menu: &SoundMenu, window: Size) -> Element<'a, Message> {
    let height = content_height(menu)
        .min(MAX_HEIGHT)
        .min(window.height - MARGIN * 2.0)
        .max(ROW_HEIGHT);
    let x = menu.at.x.min(window.width - WIDTH - MARGIN).max(MARGIN);
    let below = menu.at.y + OFFSET;
    let y = if below + height + MARGIN <= window.height {
        below
    } else {
        (menu.at.y - OFFSET - height).max(MARGIN)
    };
    let backdrop = mouse_area(Space::new().width(Fill).height(Fill))
        .on_press(send(SettingsMessage::SoundMenuClose));
    stack![backdrop, pin(opaque(popup(c, menu, height))).x(x).y(y)].into()
}

/// The list's full height: every row and heading, and the padding around them.
fn content_height(menu: &SoundMenu) -> f32 {
    // The presets, the custom row, and a heading per kind.
    let rows = menu.presets.len() + 1;
    rows as f32 * ROW_HEIGHT + crate::sounds::KINDS.len() as f32 * HEADING_HEIGHT + PADDING * 2.0
}

/// The popup itself, `height` tall.
fn popup<'a>(c: Chrome, menu: &SoundMenu, height: f32) -> Element<'a, Message> {
    let key = menu.key;
    let preset = |(label, value): &(String, String)| {
        sound_row(c, key, label.clone(), value.clone(), *value == menu.value)
    };
    // Off and the system's sound come before the first kind.
    let (first, built_in) = menu.presets.split_at(menu.presets.len().min(2));
    let mut list = column(first.iter().map(preset));
    let mut rest = built_in;
    for &(kind, count) in crate::sounds::KINDS {
        let (sounds, after) = rest.split_at(count.min(rest.len()));
        list = list
            .push(heading(c, kind))
            .extend(sounds.iter().map(preset));
        rest = after;
    }
    let custom = !menu.presets.iter().any(|(_, v)| *v == menu.value);
    list = list.push(choice(
        c,
        Space::new().width(ICON_SIZE + 10.0).into(),
        "Custom file…".to_owned(),
        custom,
        send(SettingsMessage::Custom(key)),
    ));
    container(thin_scroll(c, container(list).padding(PADDING)).height(Fill))
        .width(WIDTH)
        .height(height)
        .style(move |_| container::Style {
            border: iced::Border {
                color: c.item_edge,
                width: 1.0,
                radius: c.s.menus.radius.into(),
            },
            shadow: shadow(c.menu_shadow, 4.0, 16.0),
            ..card_style(c.popup, c.s.menus.radius)
        })
        .into()
}

/// A kind's heading.
fn heading<'a>(c: Chrome, label: &'static str) -> Element<'a, Message> {
    container(one_line(label, TEXT_SIZE - 1.5, c.muted))
        .height(HEADING_HEIGHT)
        .padding([0, 8])
        .align_y(alignment::Vertical::Bottom)
        .into()
}

/// A sound: its play button (none for Off), then its name, which picks it.
fn sound_row<'a>(
    c: Chrome,
    key: &'static str,
    label: String,
    value: String,
    selected: bool,
) -> Element<'a, Message> {
    let play: Element<'a, Message> = if value == "off" {
        Space::new().width(ICON_SIZE + 10.0).into()
    } else {
        button(icon(Icon::Play, ICON_SIZE, c.label))
            .on_press(send(SettingsMessage::SoundPlay(value.clone())))
            .padding(5)
            .style(move |_, status| flat_button(c, status, None))
            .into()
    };
    let pick = send(SettingsMessage::Set(key, Some(Value::Str(value))));
    choice(c, play, label, selected, pick)
}

/// A row: `lead` (a play button or room for one), then a button with `label`, checked while
/// it's the `selected` one, that sends `message`.
fn choice<'a>(
    c: Chrome,
    lead: Element<'a, Message>,
    label: String,
    selected: bool,
    message: Message,
) -> Element<'a, Message> {
    let check: Element<'a, Message> = if selected {
        icon(Icon::Check, ICON_SIZE, c.text).into()
    } else {
        Space::new().width(ICON_SIZE).into()
    };
    let name = button(
        row![one_line(label, TEXT_SIZE, c.text).width(Fill), check]
            .spacing(8)
            .align_y(alignment::Vertical::Center),
    )
    .on_press(message)
    .width(Fill)
    .padding([5, 8])
    .style(move |_, status| flat_button(c, status, None));
    row![lead, name]
        .spacing(2)
        .height(ROW_HEIGHT)
        .align_y(alignment::Vertical::Center)
        .into()
}

//! The settings dialog's color menu: a popup opened by clicking any color row's button. From the
//! top: Default (what the color is while unset), the theme colors for colors that can follow
//! one, a row of neutral greys, the palette (a column per hue, a row per brightness, as in the
//! workspace color menu), then a custom picker: a saturation and value square, a hue bar, an
//! alpha bar and a hex field.
//!
//! Picking a cell writes it at once and keeps the menu open, so colors can be tried one after
//! another.
//! [`ColorPicker`] (in `Settings.picker` while open) keeps the color as HSV, so dragging to
//! grey or black doesn't lose the hue. Every drag applies the color to the running app at once
//! ([`App::preview_color`](crate::App)), and letting go writes it to `config.toml`, so the file
//! isn't rewritten on every mouse move. A whole hex typed into the field applies and writes at
//! once. The popup sits by the button, kept inside the window, over a layer that closes it on a
//! click elsewhere (or Escape).

use iced::widget::{
    Space, button, column, container, mouse_area, opaque, pin, row, stack, text_input,
};
use iced::{Background, Border, Color, Degrees, Element, Fill, Point, Size, alignment, gradient};
use macaroni_config::{ColorValue, Value};

use super::SettingsMessage;
use super::list::to_hex;
use crate::Message;
use crate::drag_area::DragArea;
use crate::theme::{Chrome, PALETTE};
use crate::view::widgets::{card_style, input_style, one_line, shadow};

/// The widget id of the hex field, focused when the menu opens.
pub const HEX_INPUT: &str = "settings-color-hex";

/// The popup's size, for keeping it inside the window.
const POPUP: Size = Size::new(244.0, 500.0);

/// The saturation and value square's height (it fills the popup's width).
const SQUARE_HEIGHT: f32 = 120.0;

/// A palette cell's height; cells fill their column's width.
const CELL: f32 = 18.0;

/// The swatch inside a palette cell.
const CELL_SWATCH: f32 = 13.0;

/// A Default or theme color button's height.
const CHOICE_HEIGHT: f32 = 26.0;

/// The swatch on a Default or theme color button.
const CHOICE_SWATCH: f32 = 12.0;

/// The menu's text size.
const TEXT_SIZE: f32 = 12.0;

/// Neutral greys, black to light, for the window's own colors: the same count as the palette's
/// hues, so the row lines up with its columns.
const GREYS: [u32; 11] = [
    0x000000, 0x0e0e10, 0x141414, 0x191919, 0x1f1f1f, 0x242529, 0x2c2d31, 0x333438, 0x3a3a3a,
    0x555555, 0xa3a3a3,
];

/// The hue and alpha bars' height.
const BAR_HEIGHT: f32 = 12.0;

/// Around the popup's contents, and between its parts.
const PADDING: f32 = 12.0;

/// The round marker on the square, and the width of the bars' markers.
const MARKER: f32 = 12.0;

/// How far below the clicked swatch the popup opens.
const OFFSET: f32 = 12.0;

/// The swatch beside the hex field.
const SWATCH: f32 = 28.0;

/// What a color row hands the menu when it opens.
#[derive(Debug, Clone)]
pub struct PickerStart {
    /// The color drawn now.
    pub value: Color,
    /// What it is while unset.
    pub default: Color,
    /// The theme color it follows while unset, by label.
    pub link: Option<&'static str>,
    /// Whether it can follow a theme color (every element color, not the theme colors).
    pub themed: bool,
    /// What the config sets it to, if anything.
    pub set: Option<ColorValue>,
}

/// The open picker: which color it edits, where it opened, and the color as HSV and alpha.
#[derive(Debug, Clone)]
pub struct ColorPicker {
    /// The dotted `[colors.*]` key it writes.
    pub key: &'static str,
    /// Where the row's button was clicked, in window coordinates.
    at: Point,
    /// What it is while unset.
    default: Color,
    /// The theme color it follows while unset, by label.
    link: Option<&'static str>,
    /// Whether it offers the theme colors.
    themed: bool,
    /// What the config sets it to, for marking the current choice.
    pub set: Option<ColorValue>,
    /// Hue, 0 to 1 around the wheel.
    hue: f32,
    /// Saturation, 0 to 1.
    saturation: f32,
    /// Value (brightness), 0 to 1.
    value: f32,
    /// Alpha, 0 to 1.
    alpha: f32,
    /// The hex field's text: the color's hex, or what's being typed.
    pub hex: String,
}

impl ColorPicker {
    /// A picker for `key`, opened at `at`, starting from what the row shows.
    pub fn new(key: &'static str, at: Point, start: PickerStart) -> Self {
        let mut picker = Self {
            key,
            at,
            default: start.default,
            link: start.link,
            themed: start.themed,
            set: start.set,
            hue: 0.0,
            saturation: 0.0,
            value: 0.0,
            alpha: 1.0,
            hex: String::new(),
        };
        picker.set_color(start.value);
        picker
    }

    /// The picked color.
    pub fn color(&self) -> Color {
        let [r, g, b] = hsv_to_rgb(self.hue, self.saturation, self.value);
        Color::from_rgba(r, g, b, self.alpha)
    }

    /// Takes `color` as the picked one, its hex in the field. Greys keep the hue they had.
    pub fn set_color(&mut self, color: Color) {
        let (hue, saturation, value) = rgb_to_hsv(color.r, color.g, color.b);
        if saturation > 0.0 && value > 0.0 {
            self.hue = hue;
        }
        self.saturation = saturation;
        self.value = value;
        self.alpha = color.a;
        self.hex = to_hex(color);
    }

    /// Sets saturation and value from the square, keeping the hex field in step.
    pub fn set_saturation_value(&mut self, saturation: f32, value: f32) {
        self.saturation = saturation;
        self.value = value;
        self.hex = to_hex(self.color());
    }

    /// Sets the hue from its bar.
    pub fn set_hue(&mut self, hue: f32) {
        self.hue = hue;
        self.hex = to_hex(self.color());
    }

    /// Sets the alpha from its bar.
    pub fn set_alpha(&mut self, alpha: f32) {
        self.alpha = alpha;
        self.hex = to_hex(self.color());
    }
}

/// `h`, `s`, `v` (each 0 to 1) as red, green and blue.
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = (h.fract() + 1.0).fract() * 6.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

/// Red, green and blue as hue, saturation and value, each 0 to 1.
fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let hue = if delta == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / delta).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };
    let saturation = if max == 0.0 { 0.0 } else { delta / max };
    (hue, saturation, max)
}

/// A message for the open picker.
fn send(message: SettingsMessage) -> Message {
    Message::Settings(message)
}

/// The popup over a layer that closes it on a click elsewhere, kept inside `window`. `themes`
/// are the theme colors (name, label, color) a themed color can follow.
pub fn picker_layer<'a>(
    c: Chrome,
    picker: &ColorPicker,
    themes: &[(&'static str, &'static str, Color)],
    window: Size,
) -> Element<'a, Message> {
    let x = picker
        .at
        .x
        .min(window.width - POPUP.width - PADDING)
        .max(0.0);
    let below = picker.at.y + OFFSET;
    let y = if below + POPUP.height <= window.height {
        below
    } else {
        (picker.at.y - OFFSET - POPUP.height).max(0.0)
    };
    let backdrop = mouse_area(Space::new().width(Fill).height(Fill))
        .on_press(send(SettingsMessage::PickerClose));
    stack![backdrop, pin(opaque(popup(c, picker, themes))).x(x).y(y)].into()
}

/// Picking `value` (`None` for the default), drawn as `color`.
fn choose(value: Option<Value>, color: Color) -> Message {
    send(SettingsMessage::PickerChoose(value, color))
}

/// A palette cell: `color`'s swatch, outlined while it's the `selected` one.
fn cell<'a>(c: Chrome, color: Color, selected: bool) -> Element<'a, Message> {
    let swatch = container(Space::new())
        .width(CELL_SWATCH)
        .height(CELL_SWATCH)
        .style(move |_| container::Style {
            border: Border {
                color: c.separator,
                width: 1.0,
                radius: 3.0.into(),
            },
            ..card_style(color, 3.0)
        });
    button(container(swatch).center(Fill))
        .on_press(choose(Some(Value::Str(to_hex(color))), color))
        .width(Fill)
        .height(CELL)
        .padding(0)
        .style(move |_, status| choice_style(c, status, selected))
        .into()
}

/// A palette cell's or choice button's style: lit while hovered, raised and outlined while it's
/// the `selected` one.
fn choice_style(c: Chrome, status: button::Status, selected: bool) -> button::Style {
    let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: (selected || hover).then_some(Background::Color(if selected {
            c.raised
        } else {
            c.hover
        })),
        text_color: c.text,
        border: Border {
            color: if selected {
                c.outline
            } else {
                Color::TRANSPARENT
            },
            width: c.s.controls.border_width,
            radius: c.s.controls.radius.into(),
        },
        ..button::Style::default()
    }
}

/// A button with a small swatch and a label: Default, or a theme color.
fn choice<'a>(
    c: Chrome,
    label: String,
    color: Color,
    selected: bool,
    message: Message,
) -> Element<'a, Message> {
    let swatch = container(Space::new())
        .width(CHOICE_SWATCH)
        .height(CHOICE_SWATCH)
        .style(move |_| container::Style {
            border: Border {
                color: c.separator,
                width: 1.0,
                radius: 3.0.into(),
            },
            ..card_style(color, 3.0)
        });
    button(
        row![swatch, one_line(label, TEXT_SIZE, c.text).width(Fill)]
            .spacing(8)
            .height(Fill)
            .align_y(alignment::Vertical::Center),
    )
    .on_press(message)
    .width(Fill)
    .height(CHOICE_HEIGHT)
    .padding([0, 8])
    .style(move |_, status| choice_style(c, status, selected))
    .into()
}

/// `cells` in rows of `columns`.
fn grid<'a>(cells: Vec<Element<'a, Message>>, columns: usize) -> Element<'a, Message> {
    let mut rows = column![];
    let mut cells = cells.into_iter().peekable();
    while cells.peek().is_some() {
        rows = rows.push(row(cells.by_ref().take(columns)).width(Fill));
    }
    rows.into()
}

/// A faint heading over a part of the menu.
fn heading<'a>(c: Chrome, label: &'static str) -> Element<'a, Message> {
    one_line(label, TEXT_SIZE - 1.0, c.muted).into()
}

/// The popup itself.
fn popup<'a>(
    c: Chrome,
    picker: &ColorPicker,
    themes: &[(&'static str, &'static str, Color)],
) -> Element<'a, Message> {
    let inner = POPUP.width - PADDING * 2.0;
    let color = picker.color();
    let opaque_color = Color { a: 1.0, ..color };
    let [hr, hg, hb] = hsv_to_rgb(picker.hue, 1.0, 1.0);
    let pure = Color::from_rgb(hr, hg, hb);

    // The square: the pure hue, whitened toward the left and darkened toward the bottom.
    let whiten = gradient::Linear::new(Degrees(90.0))
        .add_stop(0.0, Color::WHITE)
        .add_stop(1.0, Color::WHITE.scale_alpha(0.0));
    let darken = gradient::Linear::new(Degrees(180.0))
        .add_stop(0.0, Color::BLACK.scale_alpha(0.0))
        .add_stop(1.0, Color::BLACK);
    let ring = pin(marker(MARKER, MARKER, MARKER / 2.0))
        .x(picker.saturation * inner - MARKER / 2.0)
        .y((1.0 - picker.value) * SQUARE_HEIGHT - MARKER / 2.0);
    let square = stack![
        fill(pure.into()),
        fill(whiten.into()),
        fill(darken.into()),
        ring
    ]
    .width(inner)
    .height(SQUARE_HEIGHT);
    let square = DragArea::new(container(square).clip(true), |x, y| {
        send(SettingsMessage::PickerSaturationValue(x, 1.0 - y))
    })
    .on_release(send(SettingsMessage::PickerCommit));

    let mut hues = gradient::Linear::new(Degrees(90.0));
    for i in 0..=6 {
        let [r, g, b] = hsv_to_rgb(i as f32 / 6.0, 1.0, 1.0);
        hues = hues.add_stop(i as f32 / 6.0, Color::from_rgb(r, g, b));
    }
    let hue = bar(hues.into(), picker.hue, inner);
    let hue = DragArea::new(hue, |x, _| send(SettingsMessage::PickerHue(x)))
        .on_release(send(SettingsMessage::PickerCommit));

    let fade = gradient::Linear::new(Degrees(90.0))
        .add_stop(0.0, opaque_color.scale_alpha(0.0))
        .add_stop(1.0, opaque_color);
    let alpha = bar(fade.into(), picker.alpha, inner);
    let alpha = DragArea::new(alpha, |x, _| send(SettingsMessage::PickerAlpha(x)))
        .on_release(send(SettingsMessage::PickerCommit));

    let swatch = container(Space::new())
        .width(SWATCH)
        .height(SWATCH)
        .style(move |_| container::Style {
            border: Border {
                color: c.separator,
                width: 1.0,
                radius: c.s.controls.radius.into(),
            },
            ..card_style(color, c.s.controls.radius)
        });
    let hex = text_input("#rrggbb", &picker.hex)
        .id(HEX_INPUT)
        .on_input(|v| send(SettingsMessage::PickerHex(v)))
        .size(13)
        .padding([6, 10])
        .width(Fill)
        .style(move |_, status| input_style(c, status));

    // The current choice is marked: Default while unset, a theme color while one is followed,
    // else whichever cell has the color.
    let current = to_hex(color);
    let rgba = matches!(picker.set, Some(ColorValue::Rgba(_)));
    let default_label = match picker.link {
        Some(theme) => format!("Default ({theme})"),
        None => "Default".to_owned(),
    };
    let mut menu = column![choice(
        c,
        default_label,
        picker.default,
        picker.set.is_none(),
        choose(None, picker.default),
    )]
    .spacing(6);
    if picker.themed {
        let theme_cells = themes
            .iter()
            .map(|&(name, label, value)| {
                let selected = matches!(&picker.set, Some(ColorValue::Theme(t)) if t == name);
                choice(
                    c,
                    label.to_owned(),
                    value,
                    selected,
                    choose(Some(Value::Str(name.to_owned())), value),
                )
            })
            .collect();
        menu = menu
            .push(heading(c, "Theme colors"))
            .push(grid(theme_cells, 2));
    }
    let greys = GREYS
        .iter()
        .map(|&value| {
            let value = crate::theme::hex(value);
            cell(c, value, rgba && to_hex(value) == current)
        })
        .collect();
    // Row by row: each row is one brightness across every hue.
    let palette = (0..PALETTE[0].len())
        .flat_map(|shade| PALETTE.iter().map(move |hue| hue[shade]))
        .filter_map(|value| {
            let rgba = macaroni_config::Rgba::parse(value)?;
            Some(crate::theme::from_rgba(rgba))
        })
        .map(|value| cell(c, value, rgba && to_hex(value) == current))
        .collect();
    menu = menu
        .push(heading(c, "Colors"))
        .push(grid(greys, GREYS.len()))
        .push(grid(palette, PALETTE.len()))
        .push(heading(c, "Custom"))
        .push(square)
        .push(hue)
        .push(alpha)
        .push(
            row![swatch, hex]
                .spacing(8)
                .align_y(alignment::Vertical::Center),
        );
    container(menu)
        .padding(PADDING)
        .width(POPUP.width)
        .style(move |_| container::Style {
            border: Border {
                color: c.item_edge,
                width: 1.0,
                radius: c.s.menus.radius.into(),
            },
            shadow: shadow(c.menu_shadow, 4.0, 16.0),
            ..card_style(c.popup, c.s.menus.radius)
        })
        .into()
}

/// A layer filling the square with `background`.
fn fill<'a>(background: iced::Background) -> Element<'a, Message> {
    container(Space::new())
        .width(Fill)
        .height(Fill)
        .style(move |_| container::Style {
            background: Some(background),
            ..container::Style::default()
        })
        .into()
}

/// A hollow white ring or bar marker, `width` by `height`, outlined dark so it shows on white.
fn marker<'a>(width: f32, height: f32, radius: f32) -> Element<'a, Message> {
    container(Space::new())
        .width(width)
        .height(height)
        .style(move |_| container::Style {
            border: Border {
                color: Color::WHITE,
                width: 2.0,
                radius: radius.into(),
            },
            shadow: shadow(Color::BLACK.scale_alpha(0.6), 0.0, 2.0),
            ..container::Style::default()
        })
        .into()
}

/// A bar `width` wide showing `background`, with a marker at `at` (0 to 1) across it.
fn bar<'a>(background: iced::Background, at: f32, width: f32) -> Element<'a, Message> {
    let thumb = MARKER / 2.0;
    let handle = pin(marker(thumb, BAR_HEIGHT, 2.0)).x(at * (width - thumb));
    let track = container(Space::new())
        .width(width)
        .height(BAR_HEIGHT)
        .style(move |_| container::Style {
            background: Some(background),
            border: Border {
                radius: (BAR_HEIGHT / 2.0).into(),
                ..Border::default()
            },
            ..container::Style::default()
        });
    stack![track, handle].width(width).height(BAR_HEIGHT).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trips() {
        for &(r, g, b) in &[
            (1.0, 0.0, 0.0),
            (0.2, 0.6, 0.4),
            (0.13, 0.13, 0.15),
            (0.0, 0.0, 0.0),
        ] {
            let (h, s, v) = rgb_to_hsv(r, g, b);
            let [r2, g2, b2] = hsv_to_rgb(h, s, v);
            assert!((r - r2).abs() < 1e-4 && (g - g2).abs() < 1e-4 && (b - b2).abs() < 1e-4);
        }
    }

    #[test]
    fn greys_keep_their_hue() {
        let mut picker = ColorPicker::new(
            "colors.solid.tab",
            Point::ORIGIN,
            PickerStart {
                value: Color::from_rgb(1.0, 0.0, 0.0),
                default: Color::BLACK,
                link: None,
                themed: true,
                set: None,
            },
        );
        picker.set_hue(0.5);
        picker.set_color(Color::from_rgb(0.5, 0.5, 0.5));
        assert_eq!(picker.hue, 0.5);
    }
}

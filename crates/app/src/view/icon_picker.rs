//! The icon picker: a modal for choosing a workspace's or tab's icon from the whole bundled
//! library (searchable by name and tag), from icons the user added, or from a new picture.
//!
//! "More Icons…" in an Icon submenu opens it ([`IconPickerMessage::Open`]) for an
//! [`IconTarget`]. Upload asks for a PNG, JPEG or SVG with the system file dialog (`rfd`, run
//! as a task so the UI keeps drawing), then shows the crop step: the picture with a square over
//! it that's dragged into place and sized with a slider. Saving writes the square into the
//! custom icons folder (`icons/` next to `config.toml`, [`icons::custom_dir`]) under a new
//! name, so a re-crop never changes another item's icon: a picture is cut and scaled to
//! [`SAVED_SIZE`] pixels as a PNG; an SVG keeps its shapes and gets a new `viewBox`, so it stays
//! sharp and is still tinted by the item's color like every other icon. The saved name is
//! `file:<name>`, which [`icons::named_glyph`] resolves.

use std::path::{Path, PathBuf};

use iced::widget::{
    Space, button, column, container, mouse_area, opaque, pin, row, scrollable, slider, stack, svg,
    text, text_input,
};
use iced::{Border, Color, Element, Fill, Length, Point, Task, alignment};
use macaroni_core::{Command, GroupId, TabId, WorkspaceId};

use super::widgets::{card_style, flat_button, input_style, one_line, shadow};
use crate::icons::{self, Glyph, Icon, icon};
use crate::theme::{Chrome, workspace_color};
use crate::{App, Message, glass};

/// The widget id of the picker's search field, focused when it opens.
const SEARCH_INPUT: &str = "icon-picker-search";

/// The most library icons shown at once; a search narrows them down.
const MAX_SHOWN: usize = 400;

/// The picker's size when the window has room.
const PICKER_SIZE: iced::Size = iced::Size::new(600.0, 620.0);

/// The least room the picker leaves around it, across both sides together.
const PICKER_MARGIN: f32 = 48.0;

/// A grid cell's size.
const CELL: f32 = 38.0;

/// An icon's size in its cell.
const CELL_ICON: f32 = 20.0;

/// The largest side the picture is shown at in the crop step.
const CROP_BOX: f32 = 320.0;

/// The smallest square the crop can be, as a share of the picture's shorter side.
const MIN_CROP_SHARE: f32 = 0.1;

/// A saved picture's width and height, in pixels: sharp at any icon size and zoom.
const SAVED_SIZE: u32 = 256;

/// Corner radius of the picker and its cells.
const RADIUS: f32 = 12.0;

/// Text sizes: the title, and everything else.
const TITLE_SIZE: f32 = 16.0;

/// Body text.
const TEXT_SIZE: f32 = 12.5;

/// Whose icon the picker sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconTarget {
    /// A workspace's.
    Workspace(WorkspaceId),
    /// A tab's.
    Tab(TabId),
    /// A sidebar group's.
    Group(GroupId),
}

/// The open picker, kept in `App.icon_picker`.
#[derive(Debug)]
pub struct IconPicker {
    /// Whose icon it sets.
    target: IconTarget,
    /// The search field's text.
    query: String,
    /// The icons the user added, listed when the picker opens and after each save.
    custom: Vec<String>,
    /// The crop step, while a picture is being cut to a square.
    crop: Option<Crop>,
    /// Why the last upload or save failed, shown until the next try.
    error: Option<String>,
}

/// A picture being cropped. Positions are in the picture's own units: pixels, or the SVG's
/// `viewBox` units.
#[derive(Debug)]
struct Crop {
    /// The file the user picked.
    source: PathBuf,
    /// An SVG's text and `viewBox` (x, y, width, height); `None` for a picture.
    svg: Option<(String, [f32; 4])>,
    /// What's drawn in the crop step.
    preview: Glyph,
    /// The picture's width and height.
    size: (f32, f32),
    /// The square's left and top edges.
    x: f32,
    /// See `x`.
    y: f32,
    /// The square's side.
    side: f32,
    /// The cursor's last position over the picture, in display pixels.
    cursor: Point,
    /// While dragging: where the press was (display pixels) and the square's corner then.
    drag: Option<(Point, f32, f32)>,
}

impl Crop {
    /// Display pixels per picture unit.
    fn scale(&self) -> f32 {
        CROP_BOX / self.size.0.max(self.size.1)
    }

    /// Keeps the square inside the picture.
    fn clamp(&mut self) {
        let short = self.size.0.min(self.size.1);
        self.side = self.side.clamp(short * MIN_CROP_SHARE, short);
        self.x = self.x.clamp(0.0, self.size.0 - self.side);
        self.y = self.y.clamp(0.0, self.size.1 - self.side);
    }
}

/// What the picker's controls send, wrapped in [`Message::IconPicker`].
#[derive(Debug, Clone)]
pub enum IconPickerMessage {
    /// Open it for this workspace or tab (closing the menu it was opened from).
    Open(IconTarget),
    /// Close it.
    Close,
    /// The search field changed.
    Query(String),
    /// Use the icon saved under this name, and close.
    Pick(String),
    /// Ask for a picture to add.
    Upload,
    /// The file dialog closed, with the file picked, if any.
    Picked(Option<PathBuf>),
    /// The cursor moved over the picture in the crop step (display pixels).
    CropMove(Point),
    /// The button went down over the picture: start dragging the square.
    CropPress,
    /// The button came up: stop dragging.
    CropRelease,
    /// The size slider moved, as a share of the picture's shorter side.
    CropSize(f32),
    /// Save the square as a new icon and use it.
    CropSave,
    /// Leave the crop step without saving.
    CropCancel,
}

impl App {
    /// Applies a picker message. Ignored while it's closed, except [`IconPickerMessage::Open`].
    pub(crate) fn icon_picker_update(&mut self, message: IconPickerMessage) -> Task<Message> {
        if let IconPickerMessage::Open(target) = message {
            self.menu = None;
            self.icon_picker = Some(IconPicker {
                target,
                query: String::new(),
                custom: icons::custom_icons(),
                crop: None,
                error: None,
            });
            return iced::widget::operation::focus(SEARCH_INPUT);
        }
        let Some(picker) = &mut self.icon_picker else {
            return Task::none();
        };
        match message {
            IconPickerMessage::Open(_) => {}
            IconPickerMessage::Close => self.icon_picker = None,
            IconPickerMessage::Query(query) => picker.query = query,
            IconPickerMessage::Pick(name) => {
                let target = picker.target;
                self.icon_picker = None;
                return self.set_icon(target, Some(name));
            }
            IconPickerMessage::Upload => {
                picker.error = None;
                let dialog = rfd::AsyncFileDialog::new()
                    .set_title("Choose an icon")
                    .add_filter("Pictures", &["png", "jpg", "jpeg", "svg"]);
                return Task::perform(
                    async move { dialog.pick_file().await.map(|f| f.path().to_path_buf()) },
                    |path| Message::IconPicker(IconPickerMessage::Picked(path)),
                );
            }
            IconPickerMessage::Picked(None) => {}
            IconPickerMessage::Picked(Some(path)) => match load_crop(&path) {
                Ok(crop) => picker.crop = Some(crop),
                Err(error) => picker.error = Some(error),
            },
            IconPickerMessage::CropMove(at) => {
                if let Some(crop) = &mut picker.crop {
                    crop.cursor = at;
                    if let Some((from, x, y)) = crop.drag {
                        let scale = crop.scale();
                        crop.x = x + (at.x - from.x) / scale;
                        crop.y = y + (at.y - from.y) / scale;
                        crop.clamp();
                    }
                }
            }
            IconPickerMessage::CropPress => {
                if let Some(crop) = &mut picker.crop {
                    crop.drag = Some((crop.cursor, crop.x, crop.y));
                }
            }
            IconPickerMessage::CropRelease => {
                if let Some(crop) = &mut picker.crop {
                    crop.drag = None;
                }
            }
            IconPickerMessage::CropSize(share) => {
                if let Some(crop) = &mut picker.crop {
                    // Resized around its center, so the part being framed stays put.
                    let center = (crop.x + crop.side / 2.0, crop.y + crop.side / 2.0);
                    crop.side = crop.size.0.min(crop.size.1) * share;
                    crop.x = center.0 - crop.side / 2.0;
                    crop.y = center.1 - crop.side / 2.0;
                    crop.clamp();
                }
            }
            IconPickerMessage::CropCancel => picker.crop = None,
            IconPickerMessage::CropSave => {
                let Some(crop) = &picker.crop else {
                    return Task::none();
                };
                match save_crop(crop) {
                    Ok(name) => {
                        let target = picker.target;
                        self.icon_picker = None;
                        return self.set_icon(target, Some(name));
                    }
                    Err(error) => picker.error = Some(error),
                }
            }
        }
        Task::none()
    }

    /// Sets `target`'s icon by saved name (`None` for the default).
    fn set_icon(&mut self, target: IconTarget, name: Option<String>) -> Task<Message> {
        self.apply(match target {
            IconTarget::Workspace(id) => Command::SetWorkspaceIcon(id, name),
            IconTarget::Tab(id) => Command::SetTabIcon(id, name),
            IconTarget::Group(id) => Command::SetGroupIcon(id, name),
        })
    }

    /// The picker over a dimmed app, or nothing while it's closed.
    pub(crate) fn icon_picker_view(&self) -> Option<Element<'_, Message>> {
        let picker = self.icon_picker.as_ref()?;
        let c = self.chrome;
        let tint = self.target_color(picker.target).unwrap_or(c.label);
        let body = match &picker.crop {
            Some(crop) => crop_view(c, crop, tint),
            None => self.choose_view(c, picker, tint),
        };
        let mut content = column![header(c, picker.crop.is_some())].spacing(14);
        if let Some(error) = &picker.error {
            content = content.push(text(error.clone()).size(TEXT_SIZE).color(c.danger));
        }
        content = content.push(body);
        let window = self.window_size;
        let card = container(content)
            .padding(20)
            .width((window.width - PICKER_MARGIN).clamp(320.0, PICKER_SIZE.width))
            .height((window.height - PICKER_MARGIN).clamp(240.0, PICKER_SIZE.height))
            .style(move |_| container::Style {
                background: Some(
                    if c.glass.is_some() {
                        c.popup
                    } else {
                        c.background
                    }
                    .into(),
                ),
                border: Border {
                    color: if c.glass.is_some() {
                        Color::TRANSPARENT
                    } else {
                        c.item_edge
                    },
                    width: 1.0,
                    radius: RADIUS.into(),
                },
                shadow: shadow(c.menu_shadow, 12.0, 40.0),
                ..container::Style::default()
            });
        let backdrop = mouse_area(
            container(opaque(glass::sheet(c, card, RADIUS, 1.0)))
                .center(Fill)
                .style(move |_| container::Style {
                    background: Some(c.overlay.into()),
                    ..container::Style::default()
                }),
        )
        .on_press(Message::IconPicker(IconPickerMessage::Close));
        Some(backdrop.into())
    }

    /// The color `target` is drawn in, if it has one.
    fn target_color(&self, target: IconTarget) -> Option<Color> {
        let color = match target {
            IconTarget::Workspace(id) => self.model.workspace(id)?.color.clone(),
            IconTarget::Tab(id) => self.model.tab(id)?.color.clone(),
            IconTarget::Group(id) => self.model.group(id)?.color.clone(),
        };
        workspace_color(color.as_deref())
    }

    /// The search field, the user's own icons with Upload, and the library's matches.
    fn choose_view<'a>(
        &self,
        c: Chrome,
        picker: &'a IconPicker,
        tint: Color,
    ) -> Element<'a, Message> {
        let search = text_input("Search icons by name or tag", &picker.query)
            .id(SEARCH_INPUT)
            .on_input(|q| Message::IconPicker(IconPickerMessage::Query(q)))
            .size(TEXT_SIZE)
            .padding([7, 10])
            .style(move |_, status| input_style(c, status));
        let upload = button(
            row![
                icon(Icon::Plus, 14.0, c.label),
                one_line("Upload PNG, JPG or SVG\u{2026}", TEXT_SIZE, c.text)
            ]
            .spacing(6)
            .align_y(alignment::Vertical::Center),
        )
        .on_press(Message::IconPicker(IconPickerMessage::Upload))
        .style(move |_, status| flat_button(c, status, Some(c.sunken)))
        .padding([6, 10]);
        let mut yours = vec![upload.into()];
        yours.extend(picker.custom.iter().filter_map(|name| {
            let glyph = icons::named_glyph(Some(name))?;
            Some(cell(c, glyph, tint, name.clone()))
        }));

        let query = picker.query.trim().to_lowercase();
        let matches: Vec<&icons::LibraryIcon> = icons::library()
            .iter()
            .filter(|i| query.is_empty() || i.matches(&query))
            .collect();
        let total = matches.len();
        let cells: Vec<Element<'a, Message>> = matches
            .into_iter()
            .take(MAX_SHOWN)
            .filter_map(|i| {
                let glyph = icons::named_glyph(Some(i.name))?;
                Some(cell(c, glyph, tint, i.name.to_owned()))
            })
            .collect();
        let count = if total > MAX_SHOWN {
            format!("Showing {MAX_SHOWN} of {total}; search to find the rest.")
        } else {
            format!("{total} icons")
        };
        let library = column![
            row(cells).spacing(2).wrap().vertical_spacing(2),
            one_line(count, TEXT_SIZE, c.faint),
        ]
        .spacing(10);
        column![
            search,
            section(c, "Your icons"),
            row(yours)
                .spacing(4)
                .align_y(alignment::Vertical::Center)
                .wrap()
                .vertical_spacing(4),
            section(c, "Library"),
            scrollable(library).height(Fill).width(Fill),
        ]
        .spacing(10)
        .into()
    }
}

/// The picker's title and close button; `cropping` changes the title.
fn header<'a>(c: Chrome, cropping: bool) -> Element<'a, Message> {
    row![
        one_line(
            if cropping {
                "Crop your icon"
            } else {
                "Choose an icon"
            },
            TITLE_SIZE,
            c.text
        ),
        Space::new().width(Fill),
        button(icon(Icon::Close, 14.0, c.muted))
            .on_press(Message::IconPicker(IconPickerMessage::Close))
            .style(move |_, status| flat_button(c, status, None))
            .padding(5),
    ]
    .align_y(alignment::Vertical::Center)
    .into()
}

/// A small heading over part of the picker.
fn section<'a>(c: Chrome, title: &'a str) -> Element<'a, Message> {
    one_line(title, TEXT_SIZE, c.muted).into()
}

/// One icon to pick, saved as `name`.
fn cell<'a>(c: Chrome, glyph: Glyph, tint: Color, name: String) -> Element<'a, Message> {
    button(container(icons::glyph(glyph, CELL_ICON, tint, 1.0)).center(CELL))
        .on_press(Message::IconPicker(IconPickerMessage::Pick(name)))
        .padding(0)
        .style(move |_, status| flat_button(c, status, None))
        .into()
}

/// The crop step: the picture with the square over it (the rest shaded), the size slider, a
/// preview of the result, and Cancel and Save.
fn crop_view<'a>(c: Chrome, crop: &'a Crop, tint: Color) -> Element<'a, Message> {
    let scale = crop.scale();
    let (width, height) = (crop.size.0 * scale, crop.size.1 * scale);
    let (x, y, side) = (crop.x * scale, crop.y * scale, crop.side * scale);
    let picture = icons::glyph(crop.preview.clone(), 1.0, tint, 1.0);
    // `glyph` makes a square; the crop step shows the picture at its own aspect instead.
    let picture: Element<'a, Message> = match &crop.preview {
        Glyph::Image(handle) => iced::widget::image(handle.clone())
            .width(width)
            .height(height)
            .into(),
        Glyph::Svg(handle) => svg(handle.clone())
            .width(width)
            .height(height)
            .style(move |_, _| svg::Style { color: Some(tint) })
            .into(),
        Glyph::Icon(_) => picture,
    };
    let shade = |w: f32, h: f32, px: f32, py: f32| {
        pin(container(Space::new().width(w.max(0.0)).height(h.max(0.0)))
            .style(|_| card_style(Color::from_rgba(0.0, 0.0, 0.0, 0.55), 0.0)))
        .x(px)
        .y(py)
    };
    let frame = pin(
        container(Space::new().width(side).height(side)).style(move |_| container::Style {
            border: Border {
                color: Color::WHITE,
                width: 1.5,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        }),
    )
    .x(x)
    .y(y);
    let layers = stack![
        picture,
        shade(width, y, 0.0, 0.0),
        shade(width, height - y - side, 0.0, y + side),
        shade(x, side, 0.0, y),
        shade(width - x - side, side, x + side, y),
        frame,
    ]
    .width(width)
    .height(height);
    let area = mouse_area(layers)
        .on_move(|p| Message::IconPicker(IconPickerMessage::CropMove(p)))
        .on_press(Message::IconPicker(IconPickerMessage::CropPress))
        .on_release(Message::IconPicker(IconPickerMessage::CropRelease))
        .interaction(iced::mouse::Interaction::Grab);
    let short = crop.size.0.min(crop.size.1);
    let size = slider(MIN_CROP_SHARE..=1.0, crop.side / short, |share| {
        Message::IconPicker(IconPickerMessage::CropSize(share))
    })
    .step(0.01_f32)
    .width(240);
    let action = |label: &'a str, message, primary: bool| {
        button(one_line(label, TEXT_SIZE, c.text))
            .on_press(Message::IconPicker(message))
            .padding([6, 14])
            .style(move |_, status| {
                let mut style =
                    flat_button(c, status, Some(if primary { c.raised } else { c.sunken }));
                style.border = Border {
                    color: c.separator,
                    width: 1.0,
                    radius: c.s.controls.radius.into(),
                };
                style
            })
    };
    column![
        one_line(
            "Drag the square over the part to keep; the slider sizes it.",
            TEXT_SIZE,
            c.muted
        ),
        container(area)
            .center_x(Fill)
            .height(Length::Fixed(CROP_BOX)),
        row![one_line("Size", TEXT_SIZE, c.label), size]
            .spacing(10)
            .align_y(alignment::Vertical::Center),
        row![
            Space::new().width(Fill),
            action("Cancel", IconPickerMessage::CropCancel, false),
            action("Use icon", IconPickerMessage::CropSave, true),
        ]
        .spacing(8),
    ]
    .spacing(14)
    .into()
}

/// Opens `path` for the crop step, with the largest centered square selected. Fails with a
/// readable message on a file that isn't a picture this can read.
fn load_crop(path: &Path) -> Result<Crop, String> {
    let (svg, preview, size) = if icons::is_svg(path) {
        let source = std::fs::read_to_string(path).map_err(|e| format!("Couldn't read it: {e}"))?;
        let view_box = view_box(&source).ok_or("That SVG has no size or viewBox to crop.")?;
        let preview = Glyph::Svg(svg::Handle::from_memory(source.clone().into_bytes()));
        let size = (view_box[2], view_box[3]);
        (Some((source, view_box)), preview, size)
    } else {
        let (w, h) = image::image_dimensions(path).map_err(|e| format!("Couldn't open it: {e}"))?;
        let preview = Glyph::Image(iced::widget::image::Handle::from_path(path));
        (None, preview, (w as f32, h as f32))
    };
    if size.0 <= 0.0 || size.1 <= 0.0 {
        return Err("That picture is empty.".to_owned());
    }
    let side = size.0.min(size.1);
    Ok(Crop {
        source: path.to_path_buf(),
        svg,
        preview,
        size,
        x: (size.0 - side) / 2.0,
        y: (size.1 - side) / 2.0,
        side,
        cursor: Point::ORIGIN,
        drag: None,
    })
}

/// Saves the crop as a new file in the custom icons folder and returns the name to save with
/// the item (`file:<name>`).
fn save_crop(crop: &Crop) -> Result<String, String> {
    let dir = icons::custom_dir().ok_or("There's no folder to save icons in.")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't make {}: {e}", dir.display()))?;
    // Milliseconds since the epoch: new every time, so re-cropping never changes an icon that's
    // already in use.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let file = match &crop.svg {
        Some((source, view_box)) => {
            let file = format!("icon-{stamp}.svg");
            let cut = [
                view_box[0] + crop.x,
                view_box[1] + crop.y,
                crop.side,
                crop.side,
            ];
            std::fs::write(dir.join(&file), with_view_box(source, cut))
                .map_err(|e| format!("Couldn't save it: {e}"))?;
            file
        }
        None => {
            let file = format!("icon-{stamp}.png");
            let picture =
                image::open(&crop.source).map_err(|e| format!("Couldn't open it: {e}"))?;
            let cut = picture
                .crop_imm(
                    crop.x.round() as u32,
                    crop.y.round() as u32,
                    crop.side.round().max(1.0) as u32,
                    crop.side.round().max(1.0) as u32,
                )
                .resize_exact(
                    SAVED_SIZE,
                    SAVED_SIZE,
                    image::imageops::FilterType::Lanczos3,
                );
            cut.save(dir.join(&file))
                .map_err(|e| format!("Couldn't save it: {e}"))?;
            file
        }
    };
    Ok(format!("{}{file}", icons::CUSTOM_PREFIX))
}

/// The `<svg ...>` start tag of `source`: its start and end (just past the `>`).
fn svg_tag(source: &str) -> Option<(usize, usize)> {
    let start = source.find("<svg")?;
    let end = start + source[start..].find('>')? + 1;
    Some((start, end))
}

/// The value of attribute `name` in `tag`, quoted with `"` or `'`.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    while let Some(at) = rest.find(name) {
        let before = rest[..at].chars().last();
        let after = rest[at + name.len()..].trim_start();
        rest = &rest[at + name.len()..];
        // A whole attribute name, not the end of another (`stroke-width` vs `width`).
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let Some(value) = after.strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        let quote = value.chars().next()?;
        if quote != '"' && quote != '\'' {
            continue;
        }
        let value = &value[1..];
        return value.find(quote).map(|end| &value[..end]);
    }
    None
}

/// An SVG's `viewBox`, else `0 0 width height` from its size (in user units, so `px` is
/// dropped); `None` when it has neither.
fn view_box(source: &str) -> Option<[f32; 4]> {
    let (start, end) = svg_tag(source)?;
    let tag = &source[start..end];
    if let Some(value) = attribute(tag, "viewBox") {
        let numbers: Vec<f32> = value
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        if let [x, y, w, h] = numbers[..] {
            return Some([x, y, w, h]);
        }
    }
    let number = |name| {
        attribute(tag, name)?
            .trim_end_matches("px")
            .trim()
            .parse::<f32>()
            .ok()
    };
    Some([0.0, 0.0, number("width")?, number("height")?])
}

/// `source` with its root `viewBox` set to `cut` and its `width` and `height` removed, so it
/// shows exactly that square at whatever size it's drawn.
fn with_view_box(source: &str, cut: [f32; 4]) -> String {
    let Some((start, end)) = svg_tag(source) else {
        return source.to_owned();
    };
    let mut tag = source[start..end].to_owned();
    for name in ["viewBox", "width", "height"] {
        if let Some(value) = attribute(&tag, name) {
            for quote in ['"', '\''] {
                let whole = format!("{name}={quote}{value}{quote}");
                if let Some(at) = tag.find(&whole) {
                    tag.replace_range(at..at + whole.len(), "");
                    break;
                }
            }
        }
    }
    let [x, y, w, h] = cut;
    let insert = format!(" viewBox=\"{x} {y} {w} {h}\"");
    tag.insert_str("<svg".len(), &insert);
    format!("{}{tag}{}", &source[..start], &source[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_boxes() {
        let svg = r#"<?xml?><svg xmlns="x" stroke-width="2" width="48px" height='24'><g/></svg>"#;
        assert_eq!(view_box(svg), Some([0.0, 0.0, 48.0, 24.0]));
        let svg = r#"<svg viewBox="0,0 10 20" width="5"><g/></svg>"#;
        assert_eq!(view_box(svg), Some([0.0, 0.0, 10.0, 20.0]));
        assert_eq!(view_box("<svg><g/></svg>"), None);
    }

    #[test]
    fn cropping_an_svg_sets_its_view_box() {
        let svg = r#"<svg xmlns="x" width="48" height="24" viewBox="0 0 48 24"><g/></svg>"#;
        let cut = with_view_box(svg, [12.0, 0.0, 24.0, 24.0]);
        assert_eq!(view_box(&cut), Some([12.0, 0.0, 24.0, 24.0]));
        assert!(!cut.contains("width=\"48\""));
        assert!(cut.ends_with("<g/></svg>"));
    }

    #[test]
    fn crop_stays_inside() {
        let mut crop = Crop {
            source: PathBuf::new(),
            svg: None,
            preview: Glyph::Icon(Icon::Terminal),
            size: (200.0, 100.0),
            x: 150.0,
            y: -5.0,
            side: 500.0,
            cursor: Point::ORIGIN,
            drag: None,
        };
        crop.clamp();
        assert_eq!((crop.x, crop.y, crop.side), (100.0, 0.0, 100.0));
    }
}

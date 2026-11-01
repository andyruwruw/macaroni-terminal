//! Draws the settings dialog: its section sidebar, each page's title row, its live preview,
//! Quick Access and cards, the rows, and their controls.
//!
//! [`App::settings_view`] is the last layer of `App::view`'s `stack`: a dimmed backdrop that
//! closes the dialog on a click, and the dialog over it (`opaque`, so clicks inside don't reach
//! the backdrop). The rows come from [`App::settings_list`]; every control sends a
//! [`SettingsMessage`]. The dialog's own layout is fixed here, not read from `[style.*]`.

use iced::widget::{
    Space, button, column, container, mouse_area, opaque, pick_list, row, scrollable, stack, text,
    text_input,
};
use iced::{Border, Color, Element, Fill, Shadow, Vector, alignment};
use macaroni_config::{ColorValue, NumberSetting, THEME_COLORS, UiTheme, Value};
use macaroni_term::shell::ShellOption;

use super::color_picker::{self, PickerStart};
use super::list::{format_number, number, theme_label, to_hex};
use super::preview::Frame;
use super::{
    BODY_SCROLL, CUSTOM, Control, SEARCH_INPUT, SSH_HOST, SSH_NAME, Section, Setting, Settings,
    SettingsMessage, group_id,
};
use crate::anim::mix;
use crate::icons::{Icon, icon};
use crate::keymap::Action;
use crate::right_click::RightClick;
use crate::theme::{Chrome, ThemeColors};
use crate::view::widgets::{card_style, flat_button, input_style, one_line, shadow};
use crate::{App, Message, glass};

/// The dialog's width when the window has room.
const DIALOG_WIDTH: f32 = 1040.0;

/// The dialog's height when the window has room.
const DIALOG_HEIGHT: f32 = 760.0;

/// The least room the dialog leaves around it in a small window, across both sides together
/// (it's centered, so half on each).
const DIALOG_MARGIN: f32 = 48.0;

/// The narrowest the dialog gets, even if the window is narrower.
const DIALOG_MIN_WIDTH: f32 = 320.0;

/// The shortest the dialog gets, even if the window is shorter.
const DIALOG_MIN_HEIGHT: f32 = 240.0;

/// The dialog's corner radius.
const DIALOG_RADIUS: f32 = 14.0;

/// The dialog's shadow: its alpha, how far it falls, and its blur.
const DIALOG_SHADOW: (f32, f32, f32) = (0.5, 12.0, 40.0);

/// The width of the dialog's section list.
const SIDEBAR_WIDTH: f32 = 210.0;

/// The section list's shadow over the page beside it: its alpha, how far right it falls, and
/// its blur.
const SIDEBAR_SHADOW: (f32, f32, f32) = (0.22, 2.0, 16.0);

/// The width of most controls (segments, drop-downs, fields), so the rows' right edges line up.
const CONTROL_WIDTH: f32 = 220.0;

/// The section title over the rows.
const TITLE_SIZE: f32 = 20.0;

/// Section names, setting labels and other body text.
const TEXT_SIZE: f32 = 13.0;

/// What's in controls: choices, fields and values.
const CONTROL_TEXT_SIZE: f32 = 12.5;

/// Group titles, Quick Access and shortcuts.
const SMALL_TEXT_SIZE: f32 = 12.0;

/// The text size of a setting's description.
const DESCRIPTION_SIZE: f32 = 11.5;

/// The search and Open config.toml icons.
const SMALL_ICON_SIZE: f32 = 14.0;

/// The icons beside the section names.
const SECTION_ICON_SIZE: f32 = 15.0;

/// The steppers' -/+ icons, the reset icon and the cards' chevrons.
const STEP_ICON_SIZE: f32 = 13.0;

/// Inside the sidebar's search field and buttons, above and below then left and right.
const SIDEBAR_ITEM_PADDING: [f32; 2] = [7.0, 10.0];

/// A group of settings' card corner radius.
const GROUP_RADIUS: f32 = 10.0;

/// How strongly a group's card shows as glass.
const GROUP_GLASS: f32 = 0.6;

/// Small controls' corner radius: segments and shortcut keys.
const SMALL_RADIUS: f32 = 6.0;

/// The width of a number's field between its steppers.
const NUMBER_FIELD_WIDTH: f32 = 52.0;

/// The width of a number's unit after its field, so the steppers line up whatever the unit.
const UNIT_WIDTH: f32 = 34.0;

/// A color's swatch: its size.
const SWATCH_SIZE: f32 = 20.0;

/// A color swatch's corner radius.
const SWATCH_RADIUS: f32 = 5.0;

/// How much narrower a color's field is than other controls, for its reset button and swatch.
const COLOR_FIELD_INSET: f32 = 52.0;

/// How strongly a reset with nothing to reset shows.
const RESET_IDLE: f32 = 0.45;

/// The tallest a page's preview gets; a taller one (a long menu) is cut off at the bottom.
const PREVIEW_MAX_HEIGHT: f32 = 280.0;

/// The shortest dialog that pins the preview over the cards. In a shorter one the preview would
/// leave the cards no room, so it scrolls with them as the page's first item.
const PIN_MIN_HEIGHT: f32 = 600.0;

/// How many cards a page needs before it gets Quick Access.
const QUICK_MIN_GROUPS: usize = 2;

/// The widest the Quick Access list gets; its chips wrap within it.
const QUICK_WIDTH: f32 = 520.0;

/// Quick Access's shadow: its alpha, how far it falls, and its blur.
const QUICK_SHADOW: (f32, f32, f32) = (0.35, 6.0, 20.0);

/// Inside a Quick Access chip, above and below then left and right.
const CHIP_PADDING: [f32; 2] = [7.0, 16.0];

/// How far a chip's fill is mixed toward the accent: a hint, not a color.
const CHIP_TINT: f32 = 0.12;

/// How far a chip's outline is mixed toward the accent.
const CHIP_EDGE_TINT: f32 = 0.3;

impl App {
    /// The dialog over a dimmed app, or nothing while it's closed.
    pub fn settings_view(&self) -> Option<Element<'_, Message>> {
        let settings = self.settings.as_ref()?;
        let c = self.chrome;
        // Lowercased once here, since `Setting::matches` expects it.
        let query = settings.query.trim().to_lowercase();
        let size = dialog_size(self.window_size);
        // Pinned over the cards it stays in view while they scroll, so a color can be watched as
        // it's dragged. Outside the scrollable it also never moves a card, which `Jump` relies on.
        let preview = (query.is_empty())
            .then(|| self.settings_preview(settings.section))
            .flatten()
            .map(|(preview, frame)| preview_frame(c, preview, frame));
        let (pinned, inline) = if size.height >= PIN_MIN_HEIGHT {
            (preview, None)
        } else {
            (None, preview)
        };
        let (header, quick, body) = self.settings_body(settings, &query, inline);
        let scroller = scrollable(container(body).padding(iced::Padding {
            top: 14.0,
            right: 28.0,
            bottom: 28.0,
            left: 28.0,
        }))
        .id(BODY_SCROLL)
        .width(Fill)
        .height(Fill);
        // The title row stays put over the scrolling cards, and Quick Access floats over their
        // top right, so opening and closing it never moves a card (which `Jump` relies on).
        let mut page = stack![scroller].width(Fill).height(Fill);
        if let Some(quick) = quick {
            page = page.push(
                container(quick)
                    .width(Fill)
                    .padding([4.0, 28.0])
                    .align_x(alignment::Horizontal::Right),
            );
        }
        let content = column![container(header).padding(iced::Padding {
            top: 22.0,
            right: 28.0,
            bottom: 0.0,
            left: 28.0,
        })]
        .push(pinned.map(|preview| {
            container(preview).padding(iced::Padding {
                top: 14.0,
                right: 28.0,
                bottom: 0.0,
                left: 28.0,
            })
        }))
        .push(page);
        // The page comes after the sidebar, so the sidebar's shadow falls over the page's
        // (transparent) background; the line is its right border.
        let edge = container(Space::new().width(1).height(Fill)).style(move |_| container::Style {
            background: Some(c.separator.into()),
            ..container::Style::default()
        });
        let dialog = dialog_frame(
            c,
            size,
            row![settings_sidebar(c, settings, &query), edge, content].into(),
        );
        let backdrop = mouse_area(
            container(opaque(glass::sheet(c, dialog, DIALOG_RADIUS, 1.0)))
                .center(Fill)
                .style(move |_| container::Style {
                    background: Some(c.overlay.into()),
                    ..container::Style::default()
                }),
        )
        .on_press(Message::SettingsClose);
        // Always a stack, the picker an optional second layer: swapping the root between the
        // backdrop and a stack would rebuild the widget tree and lose the page's scroll.
        let picker = settings.picker.as_ref().map(|picker| {
            let ui = self.config.ui_theme;
            let theme = ThemeColors::new(ui, self.config.colors.for_ui(ui));
            let themes: Vec<_> = THEME_COLORS
                .iter()
                .filter_map(|t| Some((t.name, t.label, theme.get(t.name)?)))
                .collect();
            color_picker::picker_layer(c, picker, &themes, self.window_size)
        });
        Some(stack![backdrop].push(picker).into())
    }

    /// The page: its title row (the section's title and what it covers, or the search's, and the
    /// Quick Access button), the Quick Access list while it's open, and its settings in a card
    /// per group, after `preview` when it isn't pinned. Searching looks through every section;
    /// otherwise the chosen one.
    fn settings_body<'a>(
        &'a self,
        settings: &'a Settings,
        query: &str,
        preview: Option<Element<'a, Message>>,
    ) -> (
        Element<'a, Message>,
        Option<Element<'a, Message>>,
        Element<'a, Message>,
    ) {
        let c = self.chrome;
        let searching = !query.is_empty();
        let all = self.settings_list();
        // Cards in order: a new one each time the (section, group) changes, so a group's rows
        // must be adjacent in `settings_list`.
        let mut groups: Vec<(Section, &'static str, Vec<&Setting>)> = Vec::new();
        for setting in all.iter().filter(|s| {
            if searching {
                s.matches(query)
            } else {
                s.section == settings.section
            }
        }) {
            match groups.last_mut() {
                Some((section, group, rows))
                    if *section == setting.section && *group == setting.group =>
                {
                    rows.push(setting);
                }
                _ => groups.push((setting.section, setting.group, vec![setting])),
            }
        }
        let has_quick = !searching && groups.len() >= QUICK_MIN_GROUPS;
        let (title, blurb) = if searching {
            let found: usize = groups.iter().map(|(_, _, rows)| rows.len()).sum();
            (
                format!("Results for \u{201c}{}\u{201d}", settings.query.trim()),
                match found {
                    0 => String::new(),
                    1 => "1 setting. Click a card's title to see it on its page.".to_owned(),
                    n => format!("{n} settings. Click a card's title to see it on its page."),
                },
            )
        } else {
            (
                settings.section.title().to_owned(),
                settings.section.blurb().to_owned(),
            )
        };
        // Clipped: a long search would otherwise run the title past the dialog's edge.
        let title = container(
            column![one_line(title, TITLE_SIZE, c.text)]
                .push((!blurb.is_empty()).then(|| one_line(blurb, DESCRIPTION_SIZE, c.muted)))
                .spacing(4),
        )
        .width(Fill)
        .clip(true);
        let header = row![title]
            .push(has_quick.then(|| quick_access_button(c, settings.quick)))
            .spacing(12)
            .align_y(alignment::Vertical::Center)
            .into();
        let quick = (has_quick && settings.quick).then(|| quick_access(c, &groups));
        let mut body = column![].spacing(14).push(preview);
        if groups.is_empty() {
            body = body.push(text("No settings match.").size(TEXT_SIZE).color(c.muted));
        }
        let cards = groups
            .into_iter()
            .enumerate()
            .map(|(n, (section, group, rows))| {
                self.settings_group(settings, n, section, group, searching, rows)
            });
        (header, quick, body.push(column(cards).spacing(14)).into())
    }

    /// A card of settings under its group's name (and section's, when searching across them).
    /// The header folds the card; while searching, when every result shows, it opens the card's
    /// page instead.
    fn settings_group<'a>(
        &'a self,
        settings: &'a Settings,
        n: usize,
        section: Section,
        group: &'static str,
        searching: bool,
        rows: Vec<&Setting>,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let collapsed = !searching && settings.collapsed.contains(&(section, group));
        let title = if searching {
            format!("{} \u{203a} {group}", section.title())
        } else {
            group.to_owned()
        };
        let header: Element<'a, Message> = if searching {
            button(
                row![
                    one_line(title, SMALL_TEXT_SIZE, c.label),
                    icon(Icon::ChevronRight, STEP_ICON_SIZE, c.muted),
                ]
                .spacing(6)
                .align_y(alignment::Vertical::Center),
            )
            .on_press(Message::Settings(SettingsMessage::Reveal(section, group)))
            .style(move |_, status| flat_button(c, status, None))
            .padding([4, 4])
            .into()
        } else {
            button(
                row![
                    icon(
                        if collapsed {
                            Icon::ChevronRight
                        } else {
                            Icon::ChevronDown
                        },
                        STEP_ICON_SIZE,
                        c.muted
                    ),
                    one_line(title, SMALL_TEXT_SIZE, c.label),
                    Space::new().width(Fill),
                ]
                .spacing(6)
                .align_y(alignment::Vertical::Center),
            )
            .on_press(Message::Settings(SettingsMessage::Collapse(section, group)))
            .style(move |_, status| flat_button(c, status, None))
            .padding([4, 4])
            .width(Fill)
            .into()
        };
        let mut card = column![header].spacing(6);
        if !collapsed {
            let mut list = column![];
            let count = rows.len();
            for (i, setting) in rows.into_iter().enumerate() {
                list = list.push(self.setting_row(settings, setting));
                if i + 1 < count {
                    list = list.push(separator(c));
                }
            }
            card = card.push(glass::sheet(
                c,
                // In glass the sheet is the card; a background here would hide its gradient.
                // Solid: the window's pane cards.
                container(list).clip(true).style(move |_| match c.glass {
                    Some(_) => container::Style::default(),
                    None => container::Style {
                        border: Border {
                            color: c.card_border,
                            width: c.s.panes.border_width,
                            radius: GROUP_RADIUS.into(),
                        },
                        ..card_style(c.card, GROUP_RADIUS)
                    },
                }),
                GROUP_RADIUS,
                GROUP_GLASS,
            ));
        }
        container(card).id(group_id(n)).into()
    }

    /// A setting's label and description, and its control.
    fn setting_row<'a>(
        &'a self,
        settings: &'a Settings,
        setting: &Setting,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let mut label =
            column![text(setting.label.clone()).size(TEXT_SIZE).color(c.text)].spacing(3);
        if !setting.description.is_empty() {
            label = label.push(
                text(setting.description.clone())
                    .size(DESCRIPTION_SIZE)
                    .color(c.muted),
            );
        }
        let control: Element<'a, Message> = match &setting.control {
            Control::Toggle(key, on) => toggle(c, *on, set(key, Some(Value::Bool(!on)))),
            Control::Choice(key, options, current) if options.len() <= 3 => {
                segments(c, key, options, *current)
            }
            Control::Choice(key, options, current) | Control::Dropdown(key, options, current) => {
                choice_picker(c, key, options, *current)
            }
            Control::Template {
                key,
                value,
                presets,
                placeholder,
            } => template_control(c, settings, key, value, presets, placeholder),
            Control::Text {
                key,
                value,
                placeholder,
            } => {
                let key = *key;
                let shown = settings.drafts.get(key).unwrap_or(value);
                field(c, placeholder, shown, key)
                    .on_submit(Message::Settings(SettingsMessage::Submit(key)))
                    .into()
            }
            Control::List {
                key,
                value,
                placeholder,
            } => {
                let key = *key;
                let joined = value.join(" ");
                let shown = settings.drafts.get(key).cloned().unwrap_or(joined);
                field(c, placeholder, &shown, key)
                    .on_submit(Message::Settings(SettingsMessage::SubmitList(key)))
                    .into()
            }
            Control::Shell {
                program,
                args,
                options,
                default,
            } => shell_control(c, settings, program, args, options, default),
            Control::Number { setting, value } => number_control(c, settings, setting, *value),
            Control::Color {
                key,
                value,
                default,
                link,
                themed,
                set,
            } => {
                let color = ColorRow {
                    key,
                    value: *value,
                    default: *default,
                    link: *link,
                    set: set.as_ref(),
                };
                color_control(c, color, *themed)
            }
            Control::Shortcut(chord) => shortcut(c, chord.clone()),
            Control::Button(label, message) => action_button(c, label.clone(), message.clone()),
            Control::NewSsh => {
                let add = Message::Settings(SettingsMessage::AddSsh);
                let draft = |key| settings.drafts.get(key).map_or("", String::as_str);
                column![
                    field(c, "Name (optional)", draft(SSH_NAME), SSH_NAME).on_submit(add.clone()),
                    field(c, "user@host:port", draft(SSH_HOST), SSH_HOST).on_submit(add),
                ]
                .spacing(6)
                .into()
            }
        };
        // Shortcut rows have no description and a short control, so they're packed tighter.
        let padding = if matches!(setting.control, Control::Shortcut(_)) {
            [8, 14]
        } else {
            [12, 14]
        };
        row![label.width(Fill), control]
            .spacing(20)
            .padding(padding)
            .align_y(alignment::Vertical::Center)
            .into()
    }
}

/// Writes `value` to `key`, or (`None`) puts it back to its default.
fn set(key: &'static str, value: Option<Value>) -> Message {
    Message::Settings(SettingsMessage::Set(key, value))
}

/// A hairline between rows, inset from the card's edges.
fn separator<'a>(c: Chrome) -> Element<'a, Message> {
    container(Space::new().width(Fill).height(1))
        .padding([0, 14])
        .style(move |_| card_style(c.separator, 0.0))
        .into()
}

/// The "Quick Access" button at the right of the title row; `open` turns its chevron.
fn quick_access_button<'a>(c: Chrome, open: bool) -> Element<'a, Message> {
    button(
        row![
            one_line("Quick Access", SMALL_TEXT_SIZE, c.label),
            icon(
                if open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                },
                STEP_ICON_SIZE,
                c.muted
            ),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center),
    )
    .on_press(Message::Settings(SettingsMessage::Quick))
    .style(move |_, status| {
        let mut style = flat_button(c, status, open.then_some(c.raised));
        style.border = Border {
            color: c.separator,
            width: 1.0,
            radius: SMALL_RADIUS.into(),
        };
        style
    })
    .padding([5, 10])
    .into()
}

/// The Quick Access list: the page's cards as chips that wrap, on a popup card; each scrolls
/// to its card. The chips are tinted a little toward the accent so they read as links.
fn quick_access<'a>(
    c: Chrome,
    groups: &[(Section, &'static str, Vec<&Setting>)],
) -> Element<'a, Message> {
    let chip = mix(c.sunken, c.attention, CHIP_TINT);
    let edge = mix(c.separator, c.attention, CHIP_EDGE_TINT);
    let chips = groups.iter().enumerate().map(|(n, (section, group, _))| {
        button(one_line(*group, SMALL_TEXT_SIZE, c.text))
            .on_press(Message::Settings(SettingsMessage::Jump(n, *section, group)))
            .style(move |_, status| {
                let mut style = flat_button(c, status, Some(chip));
                style.border = Border {
                    color: edge,
                    width: 1.0,
                    radius: 999.0.into(),
                };
                style
            })
            .padding(CHIP_PADDING)
            .into()
    });
    let (alpha, y, blur) = QUICK_SHADOW;
    container(row(chips).spacing(8).wrap().vertical_spacing(8))
        .padding(12)
        .max_width(QUICK_WIDTH)
        .style(move |_| container::Style {
            background: Some(c.popup.into()),
            border: Border {
                color: c.separator,
                width: 1.0,
                radius: GROUP_RADIUS.into(),
            },
            shadow: shadow(Color::from_rgba(0.0, 0.0, 0.0, alpha), y, blur),
            ..container::Style::default()
        })
        .into()
}

/// A page's live preview, centered on its own rounded card: padded on the interface's background,
/// or edge to edge on the terminal's. At most [`PREVIEW_MAX_HEIGHT`] tall.
fn preview_frame<'a>(
    c: Chrome,
    preview: Element<'a, Message>,
    frame: Frame,
) -> Element<'a, Message> {
    let (background, padding) = match frame {
        Frame::Padded => (
            if c.glass.is_some() {
                c.sunken
            } else {
                c.background
            },
            16.0,
        ),
        Frame::Fill(color) => (color, 0.0),
    };
    container(container(preview).center_x(Fill).padding(padding))
        .width(Fill)
        .max_height(PREVIEW_MAX_HEIGHT)
        .clip(true)
        .style(move |_| container::Style {
            background: Some(background.into()),
            border: Border {
                color: c.separator,
                width: 1.0,
                radius: GROUP_RADIUS.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// The dialog's left column: the search field, the sections under their headings (scrolling
/// when the dialog is short), and Open config.toml.
fn settings_sidebar<'a>(c: Chrome, settings: &'a Settings, query: &str) -> Element<'a, Message> {
    let mut sections = column![].spacing(2);
    for section in Section::shown() {
        if let Some(heading) = section.heading() {
            sections = sections.push(
                container(one_line(heading, SMALL_TEXT_SIZE, c.muted)).padding(iced::Padding {
                    top: 14.0,
                    right: 10.0,
                    bottom: 4.0,
                    left: 10.0,
                }),
            );
        }
        let active = query.is_empty() && section == settings.section;
        let color = if active { c.text } else { c.label };
        sections = sections.push(
            button(
                row![
                    icon(section.icon(), SECTION_ICON_SIZE, color),
                    one_line(section.title(), TEXT_SIZE, color),
                ]
                .spacing(10)
                .align_y(alignment::Vertical::Center),
            )
            .on_press(Message::Settings(SettingsMessage::Section(section)))
            .style(move |_, status| section_style(c, status, active))
            .width(Fill)
            .padding(SIDEBAR_ITEM_PADDING),
        );
    }
    let open_file = button(
        row![
            icon(Icon::ExternalLink, SMALL_ICON_SIZE, c.muted),
            one_line("Open config.toml", CONTROL_TEXT_SIZE, c.label),
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center),
    )
    .on_press(Message::Action(Action::OpenConfig))
    .style(move |_, status| flat_button(c, status, None))
    .width(Fill)
    .padding(SIDEBAR_ITEM_PADDING);
    let (alpha, x, blur) = SIDEBAR_SHADOW;
    container(
        column![
            settings_search(c, settings),
            scrollable(sections).height(Fill),
            open_file
        ]
        .spacing(14),
    )
    .width(SIDEBAR_WIDTH)
    .height(Fill)
    .padding(12)
    .style(move |_| match c.glass {
        Some(_) => container::Style {
            background: Some(c.sunken.into()),
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, alpha),
                offset: Vector::new(x, 0.0),
                blur_radius: blur,
            },
            ..container::Style::default()
        },
        // Solid: the window's sidebar, which has no shadow either.
        None => card_style(c.sidebar, 0.0),
    })
    .into()
}

/// A section's button in the dialog's sidebar, `active` when its page is open: in solid a
/// sidebar workspace's fills, borders and corners, in glass a flat button.
fn section_style(c: Chrome, status: button::Status, active: bool) -> button::Style {
    if c.glass.is_some() {
        return flat_button(c, status, active.then_some(c.raised));
    }
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let (fill, edge) = if active {
        (c.workspace_active, c.workspace_border_active)
    } else if hovered {
        (c.workspace_hover, c.workspace_border_hover)
    } else {
        (c.workspace, c.workspace_border)
    };
    button::Style {
        background: Some(fill.into()),
        border: Border {
            color: edge,
            width: c.s.controls.border_width,
            radius: c.s.workspaces.radius.into(),
        },
        ..flat_button(c, status, None)
    }
}

/// The search field over the sections.
fn settings_search<'a>(c: Chrome, settings: &'a Settings) -> Element<'a, Message> {
    container(
        row![
            icon(Icon::Search, SMALL_ICON_SIZE, c.muted),
            text_input("Search settings", &settings.query)
                .id(SEARCH_INPUT)
                .on_input(|q| Message::Settings(SettingsMessage::Query(q)))
                .size(TEXT_SIZE)
                .padding(0)
                .width(Fill)
                .style(move |_, status| text_input::Style {
                    background: Color::TRANSPARENT.into(),
                    border: Border::default(),
                    ..input_style(c, status)
                }),
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center),
    )
    .padding(SIDEBAR_ITEM_PADDING)
    .width(Fill)
    .clip(true)
    .style(move |_| match c.glass {
        Some(_) => container::Style {
            border: Border {
                color: c.separator,
                width: 1.0,
                radius: c.s.controls.radius.into(),
            },
            ..card_style(c.sunken, c.s.controls.radius)
        },
        // Solid: the window sidebar's search bar.
        None => card_style(c.search, c.s.controls.radius),
    })
    .into()
}

/// The dialog's size in a `window` this big, within its limits. `clamp`, not `min`: below the
/// minimum the dialog overflows the window rather than squeezing its rows into nothing.
fn dialog_size(window: iced::Size) -> iced::Size {
    iced::Size::new(
        (window.width - DIALOG_MARGIN).clamp(DIALOG_MIN_WIDTH, DIALOG_WIDTH),
        (window.height - DIALOG_MARGIN).clamp(DIALOG_MIN_HEIGHT, DIALOG_HEIGHT),
    )
}

/// The dialog's card around `content`, `size` big.
fn dialog_frame<'a>(
    c: Chrome,
    size: iced::Size,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    let (alpha, y, blur) = DIALOG_SHADOW;
    container(content)
        .width(size.width)
        .height(size.height)
        .clip(true)
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
                radius: DIALOG_RADIUS.into(),
            },
            shadow: shadow(Color::from_rgba(0.0, 0.0, 0.0, alpha), y, blur),
            ..container::Style::default()
        })
        .into()
}

/// A raised button, as a segment that's selected looks.
fn raised_button(c: Chrome, status: button::Status) -> button::Style {
    let mut style = flat_button(c, status, Some(c.raised));
    style.border = Border {
        color: c.item_edge,
        width: 1.0,
        radius: SMALL_RADIUS.into(),
    };
    style
}

/// A few options side by side, the `current` one raised.
fn segments<'a>(
    c: Chrome,
    key: &'static str,
    options: &[(String, Value)],
    current: Option<usize>,
) -> Element<'a, Message> {
    let segments = options.iter().enumerate().map(|(i, (name, value))| {
        let selected = current == Some(i);
        button(
            container(one_line(
                name.clone(),
                CONTROL_TEXT_SIZE,
                if selected { c.text } else { c.label },
            ))
            .center_x(Fill),
        )
        .on_press(set(key, Some(value.clone())))
        .style(move |_, status| {
            if selected {
                raised_button(c, status)
            } else {
                let mut style = flat_button(c, status, None);
                style.border.radius = SMALL_RADIUS.into();
                style
            }
        })
        .width(Fill)
        .padding([5, 10])
        .into()
    });
    container(row(segments).spacing(2))
        .width(CONTROL_WIDTH)
        .padding(2)
        .style(move |_| card_style(c.sunken, c.s.controls.radius))
        .into()
}

/// A button that runs something rather than setting a value.
fn action_button<'a>(c: Chrome, label: String, message: Message) -> Element<'a, Message> {
    button(
        container(one_line(label, CONTROL_TEXT_SIZE, c.text))
            .center_x(Fill)
            .width(Fill),
    )
    .on_press(message)
    .style(move |_, status| raised_button(c, status))
    .width(CONTROL_WIDTH)
    .padding([6, 10])
    .into()
}

/// A drop-down of options, the `current` one shown. Picks are matched back to their value by
/// label, so labels must be distinct.
///
/// # Panics
///
/// If `current` is past the end of `options`. [`Control::Choice`] rows get it from a position
/// in those same options.
fn choice_picker<'a>(
    c: Chrome,
    key: &'static str,
    options: &[(String, Value)],
    current: Option<usize>,
) -> Element<'a, Message> {
    let labels: Vec<String> = options.iter().map(|(l, _)| l.clone()).collect();
    let selected = current.map(|i| labels[i].clone());
    let options = options.to_vec();
    picker(c, labels, selected, CONTROL_WIDTH, move |label: String| {
        let value = options
            .iter()
            .find(|(l, _)| *l == label)
            .map(|(_, v)| v.clone());
        set(key, value)
    })
}

/// A picker's options, then its text field while it's custom (or matches none of them).
fn template_control<'a>(
    c: Chrome,
    settings: &'a Settings,
    key: &'static str,
    value: &str,
    presets: &[(String, String)],
    placeholder: &'static str,
) -> Element<'a, Message> {
    // Custom when the user picked it or when the value in effect isn't a preset, so a
    // hand-edited value always shows its text.
    let preset = presets.iter().find(|(_, t)| t == value);
    let custom = settings.custom.contains(key) || preset.is_none();
    let mut labels: Vec<String> = presets.iter().map(|(l, _)| l.clone()).collect();
    labels.push(CUSTOM.to_owned());
    let selected = match preset {
        Some((l, _)) if !custom => l.clone(),
        _ => CUSTOM.to_owned(),
    };
    let presets = presets.to_vec();
    let pick = picker(
        c,
        labels,
        Some(selected),
        CONTROL_WIDTH,
        move |label: String| match presets.iter().find(|(l, _)| *l == label) {
            Some((_, t)) => set(key, Some(Value::Str(t.clone()))),
            None => Message::Settings(SettingsMessage::Custom(key)),
        },
    );
    if !custom {
        return pick;
    }
    let shown = settings.drafts.get(key).map_or(value, String::as_str);
    column![
        pick,
        field(c, placeholder, shown, key)
            .on_submit(Message::Settings(SettingsMessage::SubmitTemplate(key)))
    ]
    .spacing(6)
    .into()
}

/// The shell: the default, a shell found on this machine, or a program typed in. Picking a
/// shell writes its arguments too, so Git Bash gets `--login -i`.
fn shell_control<'a>(
    c: Chrome,
    settings: &'a Settings,
    program: &str,
    args: &[String],
    options: &[ShellOption],
    default: &str,
) -> Element<'a, Message> {
    const KEY: &str = "shell.program";
    let default_label = if default.is_empty() {
        "Default".to_owned()
    } else {
        format!("Default ({default})")
    };
    let found = options
        .iter()
        .find(|o| o.program == program && o.args == args);
    let custom = settings.custom.contains(KEY) || (!program.is_empty() && found.is_none());
    let mut labels = vec![default_label.clone()];
    labels.extend(options.iter().map(|o| o.name.clone()));
    labels.push(CUSTOM.to_owned());
    let selected = match found {
        _ if custom => CUSTOM.to_owned(),
        Some(o) => o.name.clone(),
        None => default_label.clone(),
    };
    let options = options.to_vec();
    let pick = picker(c, labels, Some(selected), CONTROL_WIDTH, move |label| {
        if label == default_label {
            return Message::Settings(SettingsMessage::SetMany(vec![
                ("shell.program", None),
                ("shell.args", None),
            ]));
        }
        match options.iter().find(|o| o.name == label) {
            Some(o) => Message::Settings(SettingsMessage::SetMany(vec![
                ("shell.program", Some(Value::Str(o.program.clone()))),
                (
                    "shell.args",
                    (!o.args.is_empty()).then(|| Value::List(o.args.clone())),
                ),
            ])),
            None => Message::Settings(SettingsMessage::Custom(KEY)),
        }
    });
    if !custom {
        return pick;
    }
    let shown = settings.drafts.get(KEY).map_or(program, String::as_str);
    column![
        pick,
        field(c, "Program, e.g. nu.exe", shown, KEY)
            .on_submit(Message::Settings(SettingsMessage::Submit(KEY)))
    ]
    .spacing(6)
    .into()
}

/// A button that sets `key` to `next`, disabled when that's the `value` already.
fn step_button<'a>(
    c: Chrome,
    key: &'static str,
    value: f32,
    next: f32,
    glyph: Icon,
) -> Element<'a, Message> {
    button(icon(glyph, STEP_ICON_SIZE, c.label))
        .on_press_maybe((next != value).then(|| set(key, Some(number(next)))))
        .style(move |_, status| flat_button(c, status, None))
        .padding(6)
        .into()
}

/// A number: typed in its field (as shown, so a percentage as 55), or stepped on its step's
/// grid, with its unit after it and a reset.
fn number_control<'a>(
    c: Chrome,
    settings: &'a Settings,
    setting: &'static NumberSetting,
    value: f32,
) -> Element<'a, Message> {
    // Solid has some defaults of its own (squarer corners), so the reset compares against the
    // default of the interface in use.
    let default = setting.default_for(if c.glass.is_some() {
        UiTheme::Glass
    } else {
        UiTheme::Solid
    });
    let key = setting.key;
    let scale = setting.unit.scale();
    let step = |delta: f32, glyph| {
        // Kept on the step's grid, so repeated steps don't drift.
        let next = ((value + delta) / setting.step).round() * setting.step;
        step_button(c, key, value, next.clamp(setting.min, setting.max), glyph)
    };
    let shown = settings
        .drafts
        .get(key)
        .cloned()
        .unwrap_or_else(|| format_number(value * scale));
    let field = text_input(&format_number(default * scale), &shown)
        .on_input(move |v| Message::Settings(SettingsMessage::Draft(key, v)))
        .on_submit(Message::Settings(SettingsMessage::SubmitNumber(key)))
        .size(CONTROL_TEXT_SIZE)
        .padding([4, 2])
        .width(NUMBER_FIELD_WIDTH)
        .align_x(alignment::Horizontal::Right)
        .style(move |_, status| text_input::Style {
            background: Color::TRANSPARENT.into(),
            border: Border::default(),
            ..input_style(c, status)
        });
    let unit = container(one_line(setting.unit.suffix(), SMALL_TEXT_SIZE, c.muted))
        .width(UNIT_WIDTH)
        .padding([0, 4]);
    row![
        reset(c, key, value != default),
        container(
            row![
                step(-setting.step, Icon::Minimize),
                field,
                unit,
                step(setting.step, Icon::Plus)
            ]
            .align_y(alignment::Vertical::Center),
        )
        .padding(1)
        .style(move |_| card_style(c.sunken, c.s.controls.radius)),
    ]
    .spacing(4)
    .align_y(alignment::Vertical::Center)
    .into()
}

/// What a color row shows, gathered from its [`Control::Color`].
struct ColorRow<'a> {
    /// The dotted key.
    key: &'static str,
    /// What's drawn now.
    value: Color,
    /// What it is while unset.
    default: Color,
    /// The theme color it follows while unset, by label.
    link: Option<&'static str>,
    /// What the config sets it to.
    set: Option<&'a ColorValue>,
}

/// A color's swatch.
fn swatch<'a>(c: Chrome, color: Color) -> Element<'a, Message> {
    container(Space::new())
        .width(SWATCH_SIZE)
        .height(SWATCH_SIZE)
        .style(move |_| container::Style {
            background: Some(color.into()),
            border: Border {
                color: c.separator,
                width: 1.0,
                radius: SWATCH_RADIUS.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// A color: a reset, and a button showing its swatch and what it's set to (Default, a theme
/// color's name, or its hex) that opens the color menu where it's clicked. `themed` colors can
/// follow a theme color.
fn color_control<'a>(c: Chrome, color: ColorRow<'_>, themed: bool) -> Element<'a, Message> {
    let key = color.key;
    let label = match color.set {
        None => match color.link {
            Some(theme) => format!("Default ({theme})"),
            None => "Default".to_owned(),
        },
        Some(value) => theme_label(value)
            .map(str::to_owned)
            .unwrap_or_else(|| to_hex(color.value)),
    };
    let start = PickerStart {
        value: color.value,
        default: color.default,
        link: color.link,
        themed,
        set: color.set.cloned(),
    };
    let face = container(
        row![
            swatch(c, color.value),
            one_line(label, CONTROL_TEXT_SIZE, c.text).width(Fill),
            icon(Icon::Down, CONTROL_TEXT_SIZE, c.muted),
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center),
    )
    .padding([4, 10])
    .width(CONTROL_WIDTH - COLOR_FIELD_INSET)
    .style(move |_| container::Style {
        border: Border {
            color: c.separator,
            width: 1.0,
            radius: c.s.controls.radius.into(),
        },
        ..card_style(c.sunken, c.s.controls.radius)
    });
    let open = RightClick::new(face, move |at| {
        Message::Settings(SettingsMessage::Pick(key, at, start.clone()))
    })
    .on_left_click();
    row![reset(c, key, color.set.is_some()), open]
        .spacing(8)
        .align_y(alignment::Vertical::Center)
        .into()
}

/// A shortcut's chord, drawn as a key.
fn shortcut<'a>(c: Chrome, chord: String) -> Element<'a, Message> {
    container(one_line(chord, SMALL_TEXT_SIZE, c.label))
        .padding([3, 8])
        .style(move |_| container::Style {
            border: Border {
                color: c.separator,
                width: 1.0,
                radius: SMALL_RADIUS.into(),
            },
            ..card_style(c.sunken, SMALL_RADIUS)
        })
        .into()
}

/// A drop-down list of `labels`, `width` wide.
fn picker<'a>(
    c: Chrome,
    labels: Vec<String>,
    selected: Option<String>,
    width: f32,
    on_pick: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message> {
    pick_list(labels, selected, on_pick)
        .text_size(CONTROL_TEXT_SIZE)
        .padding([6, 10])
        .width(width)
        .style(move |_, status| {
            let hover = matches!(
                status,
                pick_list::Status::Hovered | pick_list::Status::Opened { .. }
            );
            pick_list::Style {
                text_color: c.text,
                placeholder_color: c.faint,
                handle_color: c.muted,
                background: if hover { c.hover } else { c.sunken }.into(),
                border: Border {
                    color: c.separator,
                    width: 1.0,
                    radius: c.s.controls.radius.into(),
                },
            }
        })
        .menu_style(move |_| iced::overlay::menu::Style {
            background: c.popup.into(),
            border: Border {
                color: c.item_edge,
                width: 1.0,
                radius: c.s.controls.radius.into(),
            },
            text_color: c.text,
            selected_text_color: c.text,
            selected_background: c.hover.into(),
            shadow: Shadow::default(),
        })
        .into()
}

/// A text field whose typing goes to the draft for `key`.
fn field<'a>(
    c: Chrome,
    placeholder: &str,
    value: &str,
    key: &'static str,
) -> iced::widget::TextInput<'a, Message> {
    text_input(placeholder, value)
        .on_input(move |v| Message::Settings(SettingsMessage::Draft(key, v)))
        .size(CONTROL_TEXT_SIZE)
        .padding([6, 10])
        .width(CONTROL_WIDTH)
        .style(move |_, status| {
            let mut style = input_style(c, status);
            if !matches!(status, text_input::Status::Focused { .. }) {
                style.border.color = c.separator;
            }
            style
        })
}

/// An on/off switch: off, its track is a field's (sunken and outlined); on, the accent.
fn toggle<'a>(c: Chrome, on: bool, message: Message) -> Element<'a, Message> {
    let knob = container(Space::new().width(14).height(14)).style(move |_| card_style(c.knob, 7.0));
    let track = container(knob)
        .width(34)
        .height(20)
        .padding(2)
        .align_x(if on {
            alignment::Horizontal::Right
        } else {
            alignment::Horizontal::Left
        })
        .align_y(alignment::Vertical::Center)
        .style(move |_| container::Style {
            border: Border {
                color: if on { c.attention } else { c.separator },
                width: 1.0,
                radius: 10.0.into(),
            },
            ..card_style(if on { c.attention } else { c.sunken }, 10.0)
        });
    button(track)
        .on_press(message)
        .padding(0)
        .style(|_, _| button::Style::default())
        .into()
}

/// Puts a setting back to its default. Always there, so the controls beside it never shift:
/// faint while there's nothing to reset (an svg tint can't be transparent, so it's dimmed by
/// opacity rather than hidden).
fn reset<'a>(c: Chrome, key: &'static str, active: bool) -> Element<'a, Message> {
    let glyph = if active {
        icon(Icon::Reload, STEP_ICON_SIZE, c.label)
    } else {
        icon(Icon::Reload, STEP_ICON_SIZE, c.faint).opacity(RESET_IDLE)
    };
    button(glyph)
        .on_press_maybe(active.then_some(set(key, None)))
        .style(move |_, status| flat_button(c, status, None))
        .padding(5)
        .into()
}

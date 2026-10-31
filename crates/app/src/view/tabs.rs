//! A pane's tab strip, its find bar, and what a tab shows.
//!
//! [`App::tab_strip`] is called by `App::pane` (in `panes`) for the top of each pane card. The
//! strip is always one line: [`App::fit_tabs`] estimates the tabs' natural widths and, when they
//! don't fit, gives each an equal fixed share down to `style.tabs.min_width`, past which the
//! tabs scroll horizontally in a `scrollable` ([`tab_strip_id`]) while the `+` stays outside it.
//! The strip is [`strip_scrollbar_room`] taller than the tabs so the scrollbar sits under them,
//! not over them. Each tab reports hover and the half under the cursor through
//! [`Pointer`], which [`App::drop_target`] reads for drops into the strip. What a tab shows
//! comes from the `[tab_items]` templates, filled by [`tab_value`].

use std::time::Duration;

use iced::widget::{
    Space, column, container, mouse_area, row, scrollable, stack, text, text_input, tooltip,
};
use iced::{Border, Color, Element, Fill, Length, alignment};
use macaroni_core::{Command, Pane, PaneId, Tab, TabId, TabKind};

use super::drag::insertion_line;
use super::sidebar::{agent_line, name_tip};
use super::widgets::{
    AddBox, FIELD_PADDING, ICON_BUTTON_PADDING, LINE_HEIGHT, MARK_OVER_TEXT, add_box, card_style,
    faded_icon_button, glass_strength, icon_button, input_style, item_ease, item_fill, one_line,
    truncate,
};
use super::{RENAME_INPUT, SEARCH_INPUT};
use crate::icons::{Glyph, Icon, glyph as draw_glyph, icon, named_glyph};
use crate::pointer::{Halves, Pointer};
use crate::right_click::RightClick;
use crate::theme::{Chrome, workspace_color};
use crate::{App, Dragged, DropTarget, MenuTarget, Message, Renaming, anim, glass, label};

/// How long the pointer rests on a tab before its tooltip shows.
const TIP_DELAY: Duration = Duration::from_millis(500);

/// The widest a tab's tooltip grows before its lines wrap.
const TIP_WIDTH: f32 = 360.0;

/// The gap between the new tab buttons at the right of a strip.
const NEW_BUTTON_SPACING: f32 = 2.0;

/// Roughly how wide a title character is, as a multiple of the text size, for fitting tabs.
const CHAR_WIDTH: f32 = 0.56;

/// Slack added to each tab's estimated width, so a tab that nearly fits isn't cut short.
const TAB_WIDTH_SLACK: f32 = 8.0;

/// Slack kept free at the end of the strip, past the `+` button.
const STRIP_SLACK: f32 = 6.0;

/// A tab's name field while it's renamed: its width.
const RENAME_WIDTH: f32 = 180.0;

/// The rename field's padding left and right.
const RENAME_PADDING_X: f32 = 10.0;

/// The scrollbar's margin from the strip's edge, on each side.
const SCROLLBAR_MARGIN: f32 = 1.0;

/// How strongly the scroller shows while the cursor is over the tabs but not the scrollbar.
const SCROLLER_ALPHA: f32 = 0.6;

/// Between the agent's status and its summary, and either side of them.
const STATUS_SPACING: f32 = 4.0;

/// The find bar's field width.
const SEARCH_WIDTH: f32 = 220.0;

/// Between the find bar's parts.
const SEARCH_SPACING: f32 = 4.0;

/// The height of what's in a tab (and of its close button), inside its padding.
fn tab_row(c: Chrome) -> f32 {
    c.s.controls.icon_button_size + ICON_BUTTON_PADDING * 2.0
}

/// A tab's padding above and below `tab_row`, so it's `[style.tabs] height` tall.
fn tab_padding_y(c: Chrome) -> f32 {
    ((c.s.tabs.height - tab_row(c)) / 2.0).max(0.0)
}

/// Whether tabs sit right on their pane's top border: solid, no `tabs.gap`, and a pane border
/// to sit on. Each tab (and the `+`) is then a pane border taller, so its bottom border lands on
/// the card's top border (the strip is drawn over the card, see `App::pane`) and the two are
/// one line; the open tab covers that line with its fill, so it opens into its pane.
pub(super) fn tabs_flush(c: Chrome) -> bool {
    c.glass.is_none() && c.s.tabs.gap == 0.0 && c.s.panes.border_width > 0.0
}

/// The spacing between tabs: `tabs.spacing`, or with none in solid less a tab border, so two
/// tabs' borders are one line rather than a doubled one.
pub(super) fn tab_spacing(c: Chrome) -> f32 {
    if c.glass.is_none() && c.s.tabs.spacing == 0.0 {
        -c.s.controls.border_width
    } else {
        c.s.tabs.spacing
    }
}

/// The part of `[style.tabs] gap` the strip takes, for its scrollbar to sit under the tabs.
/// `App::pane` takes the same amount off the spacing to the card, so the card still starts
/// `height + gap` down, which drop targets and browser bounds rely on. Capped at the gap so
/// that spacing never goes negative.
pub(super) fn strip_scrollbar_room(c: Chrome) -> f32 {
    (c.s.tabs.scrollbar_width + SCROLLBAR_MARGIN * 2.0).min(c.s.tabs.gap)
}

/// `template` from `[tab_items]` filled in for `tab`, the `index`th in its pane.
fn tab_text(c: Chrome, template: &str, tab: &Tab, index: usize) -> String {
    label::fill(template, |key| tab_value(c, tab, index, key))
}

impl App {
    /// A pane's tabs and `+` button, then the find bar when shown, and the new tab buttons at
    /// the right end.
    pub(super) fn tab_strip(&self, pane: &Pane, width: f32) -> Element<'_, Message> {
        let c = self.chrome;
        let (tab_width, overflow) = self.fit_tabs(pane, width);
        let gap = match (self.drop_target(), self.item_drag.and_then(|d| d.tab())) {
            (Some((DropTarget::Strip { pane: p, index }, _)), Some(tab)) if p == pane.id => {
                self.insertion_gap(pane, tab, index)
            }
            _ => None,
        };
        let tabs = pane.tabs.iter().enumerate().filter_map(|(index, &id)| {
            let tab = self.model.tab(id)?;
            Some(self.tab_item(pane.id, index, tab, tab_width, gap == Some(index)))
        });
        // The line before tab `gap` is drawn by that tab; `tabs.len()` (after the last) is drawn
        // by the `+` below.
        // Top-aligned: flush, the open tab's slot hangs lower than the rest.
        let tabs = row(tabs)
            .spacing(tab_spacing(c))
            .align_y(alignment::Vertical::Top);
        // Outside the scrolling part, so it stays at the end of the strip.
        let add = insertion_line(c, new_tab_button(c, pane.id), gap == Some(pane.tabs.len()));
        // Top-aligned: the scroller is `strip_scrollbar_room` taller than everything else, and
        // that room hangs below the tabs.
        let mut strip = row![tab_scroller(c, pane.id, tabs.into(), overflow), add]
            .spacing(tab_spacing(c))
            .align_y(alignment::Vertical::Top);
        let search = self.search_bar(pane);
        let buttons = self.config.tab_items.new_buttons;
        // Pushes the find bar and the buttons to the right.
        if search.is_some() || buttons {
            strip = strip.push(Space::new().width(Fill));
        }
        if let Some(bar) = search {
            strip = strip.push(
                container(bar)
                    .height(c.s.tabs.height)
                    .align_y(alignment::Vertical::Center),
            );
        }
        if buttons {
            strip = strip.push(self.new_tab_buttons(pane.id));
        }
        strip.into()
    }

    /// Icon buttons that open a terminal (the default shell), a browser, and VS Code at the
    /// active tab's folder (a VS Code tab once downloaded, else the desktop app if installed),
    /// each in `pane` and named on hover.
    fn new_tab_buttons(&self, pane: PaneId) -> Element<'_, Message> {
        let c = self.chrome;
        let button = |glyph, name: &str, message| -> Element<'_, Message> {
            tooltip(
                icon_button(c, glyph, message),
                name_tip(c, name.to_owned()),
                tooltip::Position::Bottom,
            )
            .into()
        };
        let mut buttons = row![
            button(
                Icon::Terminal,
                "New terminal",
                Message::Commands(vec![Command::FocusPane(pane), Command::NewTab]),
            ),
            button(Icon::Globe, "New browser", Message::NewBrowserIn(pane)),
        ];
        // A tab when VS Code for the web is downloaded, else the desktop app.
        if self.vscode_web.installed {
            buttons = buttons.push(button(
                Icon::Code,
                "New VS Code tab",
                Message::VsCodeTab(pane),
            ));
        } else if self.vscode.is_some() {
            buttons = buttons.push(button(
                Icon::Code,
                "Open in VS Code",
                Message::OpenVsCode(pane),
            ));
        }
        container(buttons.spacing(NEW_BUTTON_SPACING))
            .height(c.s.tabs.height)
            .align_y(alignment::Vertical::Center)
            .into()
    }

    /// How wide [`App::new_tab_buttons`] is (none while they're off), for fitting the tabs.
    fn new_buttons_width(&self) -> f32 {
        if !self.config.tab_items.new_buttons {
            return 0.0;
        }
        let c = self.chrome;
        let code = self.vscode_web.installed || self.vscode.is_some();
        let count = if code { 3.0 } else { 2.0 };
        count * tab_row(c) + (count - 1.0) * NEW_BUTTON_SPACING + tab_spacing(c)
    }

    /// How wide each unpinned tab is, and whether they overflow the strip and scroll: as wide as
    /// its contents when they all fit in `width`, else an equal share of what pinned tabs leave,
    /// but no narrower than `[style.tabs] min_width`. Widths are estimated from character counts rather than
    /// measured, since the strip has to be sized before layout.
    fn fit_tabs(&self, pane: &Pane, width: f32) -> (Length, bool) {
        let c = self.chrome;
        let items = &self.config.tab_items;
        // Icon-only, about square, plus the spacing after it.
        let pinned_width = c.s.tabs.height + tab_spacing(c);
        let tabs: Vec<_> = pane
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(index, &id)| Some((index, self.model.tab(id)?)))
            .collect();
        let pinned = tabs.iter().filter(|(_, tab)| tab.pinned).count();
        let natural: f32 = tabs
            .iter()
            .map(|&(index, tab)| {
                if tab.pinned {
                    return pinned_width;
                }
                let text = truncate(
                    &tab_text(c, &items.title, tab, index),
                    c.s.tabs.max_title_chars as usize,
                )
                .chars()
                .count()
                    + tab_text(c, &items.badge, tab, index).chars().count();
                // Rough widths: per title character, and for the icon, padding, close button
                // and spacing.
                text as f32 * c.s.tabs.title_size * CHAR_WIDTH
                    + c.s.tabs.padding_left
                    + c.s.tabs.padding_right
                    + c.s.tabs.icon_size
                    + tab_row(c)
                    + c.s.tabs.part_spacing * 2.0
                    + tab_spacing(c)
                    + TAB_WIDTH_SLACK
            })
            .sum();
        // The `+` button (a `tabs.height` square) and its spacing, and the new tab buttons.
        let room =
            width - c.s.tabs.height - tab_spacing(c) - self.new_buttons_width() - STRIP_SLACK;
        // Each unpinned tab's share of what the pinned ones leave, less the spacing after it
        // (`natural` counts that spacing per tab too).
        let unpinned = (tabs.len() - pinned).max(1) as f32;
        let shared = (room - pinned as f32 * pinned_width) / unpinned - tab_spacing(c);
        let overflow = natural > room && shared < c.s.tabs.min_width;
        let tab_width = if natural <= room {
            Length::Shrink
        } else {
            Length::Fixed(shared.max(c.s.tabs.min_width))
        };
        (tab_width, overflow)
    }

    /// One tab in `pane`'s strip, the `index`th, `tab_width` wide unless pinned, with the
    /// insertion line before it if `line`; or its name field while it's renamed.
    fn tab_item(
        &self,
        pane: PaneId,
        index: usize,
        tab: &Tab,
        tab_width: Length,
        line: bool,
    ) -> Element<'_, Message> {
        let c = self.chrome;
        let id = tab.id;
        if let Some((Renaming::Tab(renaming), value)) = &self.renaming
            && *renaming == id
        {
            return tab_rename_field(c, id, value);
        }
        let levels = self.fades.get(anim::Item::Tab(id));
        let ghost = if self.dragging(Dragged::Tab(id)) {
            c.s.effects.dragged
        } else {
            1.0
        };
        let fade = (c.s.effects.dim + (1.0 - c.s.effects.dim) * levels.emphasis(c.s.effects.hover))
            * ghost;
        let body = self.tab_body(tab, index, tab_width != Length::Shrink, fade, ghost);
        let width = if tab.pinned {
            Length::Shrink
        } else {
            tab_width
        };
        // Flush, whether it's its pane's open tab (see `tabs_flush`).
        let hang = tabs_flush(c).then(|| self.model.pane(pane).is_some_and(|p| p.active == id));
        let card = tab_card(c, body, levels, ghost, tab.pinned, width, hang);
        // Pressing selects the tab and can start dragging it, so it's a mouse area rather
        // than a button (which only reports the click once released).
        let tab_area = Pointer::new(
            mouse_area(card)
                .on_press(Message::TabPress(id))
                .on_enter(Message::TabHover(pane, index, true))
                .on_exit(Message::TabHover(pane, index, false)),
        )
        .on_zone(Halves::LeftRight, move |zone| {
            Message::TabZone(pane, index, zone)
        })
        .on_press_outside_maybe(
            (self.item_focus == Some(Renaming::Tab(id)))
                .then_some(Message::ItemBlur(Renaming::Tab(id))),
        );
        // No tooltip while something is dragged: it would trail the drag around.
        let tab_area: Element<'_, Message> = if self.item_drag.is_some() {
            tab_area.into()
        } else {
            tooltip(tab_area, self.tab_tip(tab), tooltip::Position::Bottom)
                .delay(TIP_DELAY)
                .into()
        };
        insertion_line(
            c,
            RightClick::new(tab_area, move |at| {
                Message::MenuOpen(MenuTarget::Tab(id), at)
            })
            .into(),
            line,
        )
    }

    /// A tab's tooltip: its icon and whole title, then what its agent is doing and the agent's
    /// status line (model, context) when one runs.
    fn tab_tip(&self, tab: &Tab) -> Element<'_, Message> {
        let c = self.chrome;
        let tint = workspace_color(tab.color.as_deref()).unwrap_or(c.text);
        let title = row![
            draw_glyph(tab_icon(tab), c.s.tabs.icon_size, tint, 1.0),
            text(tab_title(tab)).size(c.s.tabs.title_size).color(c.text),
        ]
        .spacing(c.s.tabs.part_spacing)
        .align_y(alignment::Vertical::Center);
        let mut lines = column![title].spacing(STATUS_SPACING);
        if let Some(agent) = &tab.agent {
            let (line, color) = agent_line(c, agent);
            lines = lines.push(text(line).size(c.s.tabs.status_size).color(color));
            if let Some(summary) = &agent.summary {
                lines = lines.push(
                    text(summary.clone())
                        .size(c.s.tabs.status_size)
                        .color(c.faint),
                );
            }
        }
        container(lines)
            .max_width(TIP_WIDTH)
            .padding([6, 10])
            .style(move |_| container::Style {
                border: Border {
                    color: c.item_edge,
                    width: c.s.menus.border_width,
                    radius: c.s.controls.radius.into(),
                },
                ..card_style(c.popup, c.s.controls.radius)
            })
            .into()
    }

    /// What a tab shows inside its card: its icon (all a pinned tab shows), title, badge,
    /// muted mark and close button. `fill` makes the title take the tab's fixed width.
    fn tab_body(
        &self,
        tab: &Tab,
        index: usize,
        fill: bool,
        fade: f32,
        ghost: f32,
    ) -> Element<'_, Message> {
        let c = self.chrome;
        let items = &self.config.tab_items;
        let tint = workspace_color(tab.color.as_deref());
        let (glyph, glyph_color, glyph_fade) = match (&tab.attention, &tab.agent) {
            (Some(_), _) => (Icon::Bell.into(), c.attention, ghost),
            (None, Some(agent)) if self.config.agents.tab_icon => {
                (Icon::Sparkles.into(), agent_line(c, agent).1, ghost)
            }
            (None, _) => (tab_icon(tab), tint.unwrap_or(c.text), fade),
        };
        // Pinned tabs are just their icon, and can't be closed from the strip.
        if tab.pinned {
            // As tall as a close button, so pinning never changes the strip's height.
            return container(draw_glyph(
                glyph,
                c.s.tabs.icon_size,
                glyph_color,
                glyph_fade,
            ))
            .center(tab_row(c))
            .into();
        }
        // One line, clipped: tabs shrink to share the strip (and then scroll) instead of
        // wrapping, which would make the header taller and resize everything below it.
        let title = tab_text(c, &items.title, tab, index);
        let badge = tab_text(c, &items.badge, tab, index);
        let title = (!title.is_empty()).then(|| {
            container(one_line(
                truncate(&title, c.s.tabs.max_title_chars as usize),
                c.s.tabs.title_size,
                c.text.scale_alpha(fade),
            ))
            .width(if fill { Fill } else { Length::Shrink })
            .clip(true)
        });
        let id = tab.id;
        row![]
            .push(
                items
                    .icon
                    .then(|| draw_glyph(glyph, c.s.tabs.icon_size, glyph_color, glyph_fade)),
            )
            .push(title)
            .push(
                (!badge.is_empty())
                    .then(|| one_line(badge, c.s.tabs.badge_size, c.faint.scale_alpha(fade))),
            )
            .push((items.status_icons && tab.muted).then(|| {
                icon(Icon::BellOff, c.s.tabs.badge_size + MARK_OVER_TEXT, c.faint).opacity(fade)
            }))
            .push(items.close_button.then(|| {
                faded_icon_button(
                    c,
                    Icon::Close,
                    Message::Command(Command::CloseTab(id)),
                    fade,
                )
            }))
            // As tall as a close button, so the strip keeps its height without one.
            .height(tab_row(c))
            .spacing(c.s.tabs.part_spacing)
            .align_y(alignment::Vertical::Center)
            .into()
    }
}

/// A tab's name field while it's renamed.
fn tab_rename_field<'a>(c: Chrome, id: TabId, value: &'a str) -> Element<'a, Message> {
    Pointer::new(
        text_input("Tab name", value)
            .id(RENAME_INPUT)
            .on_input(Message::RenameInput)
            .on_submit(Message::RenameCommit)
            .size(c.s.tabs.title_size)
            // Vertical padding that makes the field exactly `tabs.height` tall, like the tab.
            .padding([
                (c.s.tabs.height - c.s.tabs.title_size * LINE_HEIGHT) / 2.0,
                RENAME_PADDING_X,
            ])
            .width(RENAME_WIDTH)
            .style(move |_, status| input_style(c, status)),
    )
    .on_press_outside(Message::RenameCancel(Renaming::Tab(id)))
    .into()
}

/// A tab's card around `body`: filled by how selected and hovered it is (`levels`), faded to
/// `ghost` while dragged, and lifted when selected on glass. `hang` is set when tabs are flush
/// ([`tabs_flush`]), to whether the tab is open: the tab then reaches a pane border lower, over
/// the card's top border, and an open one covers its bottom border and the card's with its fill
/// (between its side borders).
fn tab_card<'a>(
    c: Chrome,
    body: Element<'a, Message>,
    levels: anim::Levels,
    ghost: f32,
    pinned: bool,
    width: Length,
    hang: Option<bool>,
) -> Element<'a, Message> {
    let lift = levels.selected;
    // Tabs keep their size when selected (a lift would make the open one taller than the rest);
    // they still clear and refract the backdrop and deepen their shadow, below.
    let (outer, mut padding) = glass::lift_padding(
        c,
        if pinned {
            iced::Padding::new(tab_padding_y(c))
        } else {
            iced::Padding {
                top: tab_padding_y(c),
                right: c.s.tabs.padding_right,
                bottom: tab_padding_y(c),
                left: c.s.tabs.padding_left,
            }
        },
        0.0,
    );
    // Taller by the pane border it lies over. The text stays where it was, since only the
    // bottom padding grows.
    if hang.is_some() {
        padding.bottom += c.s.panes.border_width;
    }
    let fill = item_fill(c, levels, c.tab, c.tab_active, c.tab_hover).scale_alpha(ghost);
    // The fixed width goes on the outer slot (`width`), so the card inside fills it less the
    // lift's outer padding; a shrinking tab stays shrunk inside too.
    let card = container(body)
        .style(move |_| container::Style {
            background: Some(fill.into()),
            border: Border {
                color: item_ease(
                    levels,
                    c.tab_border,
                    c.tab_border_active,
                    c.tab_border_hover,
                )
                .scale_alpha(ghost),
                width: c.s.controls.border_width,
                radius: c.s.tabs.radius.into(),
            },
            // Solid tabs are outlined, not filled, so a shadow would darken around them.
            shadow: if c.glass.is_some() {
                glass::lift_shadow(c, c.shadow, lift)
            } else {
                iced::Shadow::default()
            },
            ..container::Style::default()
        })
        .padding(padding)
        .width(if width == Length::Shrink {
            Length::Shrink
        } else {
            Fill
        });
    let card = accent_line(c, card.into(), levels.selected * ghost, width);
    let slot = container(glass::frosted(
        c,
        card,
        c.s.tabs.radius,
        glass_strength(c, levels) * ghost,
        lift,
    ))
    .padding(outer)
    .width(width);
    if hang != Some(true) {
        return slot.into();
    }
    // The open tab's fill over the bottom row, where its border and the card's lie, between its
    // side borders.
    let side = c.s.controls.border_width;
    let seam = side.max(c.s.panes.border_width);
    let cover = row![
        Space::new().width(side),
        container(Space::new().width(Fill).height(seam)).style(move |_| card_style(fill, 0.0)),
        Space::new().width(side),
    ];
    stack![
        slot,
        container(cover)
            .width(Fill)
            .height(Fill)
            .align_y(alignment::Vertical::Bottom),
    ]
    .width(width)
    .into()
}

/// `card` with the open tab's line along its top in solid (`Chrome.tab_accent`,
/// `style.tabs.accent_width` thick), faded in by `selected`; the card as it is otherwise. The
/// line is laid over the card's top border rather than taking room, so a tab is the same height
/// open or not, and its top corners are the tab's.
fn accent_line<'a>(
    c: Chrome,
    card: Element<'a, Message>,
    selected: f32,
    width: Length,
) -> Element<'a, Message> {
    let thickness = c.s.tabs.accent_width;
    if c.glass.is_some() || thickness <= 0.0 || selected <= 0.0 {
        return card;
    }
    let radius = c.s.tabs.radius;
    let line =
        container(Space::new().width(Fill).height(thickness)).style(move |_| container::Style {
            background: Some(c.tab_accent.scale_alpha(selected).into()),
            border: Border {
                radius: iced::border::Radius {
                    top_left: radius,
                    top_right: radius,
                    bottom_right: 0.0,
                    bottom_left: 0.0,
                },
                ..Border::default()
            },
            ..container::Style::default()
        });
    // The stack takes the card's size (its first layer), so the line spans the tab.
    stack![card, line]
        .width(if width == Length::Shrink {
            Length::Shrink
        } else {
            Fill
        })
        .into()
}

/// The strip's tabs in a horizontal scrollable, filling the strip when they `overflow` it.
fn tab_scroller<'a>(
    c: Chrome,
    pane: PaneId,
    tabs: Element<'a, Message>,
    overflow: bool,
) -> Element<'a, Message> {
    // The wheel scrolls it as is, not just with Shift: `on_scroll` gets the wheel events the
    // scrollable didn't use and `update` turns them into `scroll_by`. The bottom padding is
    // the room the scrollbar floats in, under the tabs.
    mouse_area(
        scrollable(container(tabs).padding(iced::Padding {
            bottom: strip_scrollbar_room(c),
            ..iced::Padding::ZERO
        }))
        .id(tab_strip_id(pane))
        .on_scroll(move |viewport| Message::TabViewport(pane, viewport))
        .direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::new()
                .width(c.s.tabs.scrollbar_width)
                .scroller_width(c.s.tabs.scrollbar_width)
                .margin(SCROLLBAR_MARGIN),
        ))
        // Shrink while the tabs fit, so the `+` follows the last tab.
        .width(if overflow { Fill } else { Length::Shrink })
        .style(move |theme, status| strip_scrollbar(c, theme, status)),
    )
    .on_scroll(move |delta| Message::TabScroll(pane, delta))
    .into()
}

/// The `+` box after a pane's tabs: a new tab, or on right-click a menu of what to open.
fn new_tab_button<'a>(c: Chrome, pane: PaneId) -> Element<'a, Message> {
    // Flush, like a tab that isn't open: a pane border taller, its bottom border on the card's.
    let hang = if tabs_flush(c) {
        c.s.panes.border_width
    } else {
        0.0
    };
    RightClick::new(
        add_box(
            c,
            AddBox::Tab,
            Length::Fixed(c.s.tabs.height),
            c.s.tabs.height + hang,
            Message::Commands(vec![Command::FocusPane(pane), Command::NewTab]),
        ),
        move |at| Message::MenuOpen(MenuTarget::NewTab(pane), at),
    )
    .into()
}

/// The tab strip's (and the sidebar's) scrollbar: a thin scroller in the UI's own colors, shown
/// only while the cursor is over what scrolls, brighter over the scrollbar itself.
pub(super) fn strip_scrollbar(
    c: Chrome,
    theme: &iced::Theme,
    status: scrollable::Status,
) -> scrollable::Style {
    let scroller = match status {
        scrollable::Status::Active { .. } => Color::TRANSPARENT,
        scrollable::Status::Hovered {
            is_horizontal_scrollbar_hovered: false,
            is_vertical_scrollbar_hovered: false,
            ..
        } => c.faint.scale_alpha(SCROLLER_ALPHA),
        _ => c.muted,
    };
    let rail = scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller {
            background: scroller.into(),
            border: Border::default().rounded(c.s.tabs.scrollbar_width / 2.0),
        },
    };
    scrollable::Style {
        horizontal_rail: rail,
        vertical_rail: rail,
        ..scrollable::default(theme, status)
    }
}

/// Where the `index`th of a strip's tabs (`pinned` says which are) starts and ends in the
/// scrolling strip, `content` wide, while the tabs overflow it: unpinned tabs are then all
/// `style.tabs.min_width` (see `App::fit_tabs`), and pinned ones share what's left. `None` past
/// the last tab.
pub fn tab_span(c: Chrome, pinned: &[bool], index: usize, content: f32) -> Option<(f32, f32)> {
    if index >= pinned.len() {
        return None;
    }
    let spacing = tab_spacing(c);
    let count = pinned.len() as f32;
    let pins = pinned.iter().filter(|p| **p).count() as f32;
    let unpinned = count - pins;
    let pinned_width = if pins > 0.0 {
        (content - unpinned * c.s.tabs.min_width - (count - 1.0) * spacing) / pins
    } else {
        0.0
    };
    let width = |p: bool| if p { pinned_width } else { c.s.tabs.min_width };
    let left: f32 = pinned[..index].iter().map(|&p| width(p) + spacing).sum();
    Some((left, left + width(pinned[index])))
}

/// The scrollable holding a pane's tabs.
pub fn tab_strip_id(pane: PaneId) -> iced::widget::Id {
    iced::widget::Id::from(format!("tab-strip-{}", pane.0))
}

/// A tab's icon: the user's, else its kind's.
pub(super) fn tab_icon(tab: &Tab) -> Glyph {
    named_glyph(tab.icon.as_deref()).unwrap_or_else(|| tab_kind_icon(tab).into())
}

/// The default icon for a tab's kind: a globe for a browser, a terminal prompt otherwise.
pub(super) fn tab_kind_icon(tab: &Tab) -> Icon {
    match tab.kind {
        TabKind::Browser(_) => Icon::Globe,
        TabKind::Terminal(_) => Icon::Terminal,
    }
}

/// What a tab's templates can show (`[tab_items]`); `index` is its place in its pane. `None`
/// for an unknown placeholder, which [`label::fill`] leaves as typed.
fn tab_value(c: Chrome, tab: &Tab, index: usize, key: &str) -> Option<String> {
    Some(match key {
        "title" => tab_title(tab),
        "name" => tab.name.clone().unwrap_or_default(),
        "index" => (index + 1).to_string(),
        "kind" => match tab.kind {
            TabKind::Terminal(_) => "Terminal".to_owned(),
            TabKind::Browser(_) => "Browser".to_owned(),
        },
        "dir" => tab_dir(tab),
        "path" => tab.cwd.clone().unwrap_or_default(),
        "url" => tab.url.clone().unwrap_or_default(),
        "notice" => tab
            .attention
            .as_ref()
            .map(|n| n.body.clone())
            .unwrap_or_default(),
        "agent" => tab
            .agent
            .as_ref()
            .map(|a| agent_line(c, a).0)
            .unwrap_or_default(),
        _ => return None,
    })
}

/// The last part of a tab's working directory.
pub(super) fn tab_dir(tab: &Tab) -> String {
    tab.cwd
        .as_deref()
        .map(|cwd| {
            let trimmed = cwd.trim_end_matches(['/', '\\']);
            trimmed
                .rsplit(['/', '\\'])
                .next()
                .filter(|name| !name.is_empty())
                .unwrap_or(cwd)
                .to_owned()
        })
        .unwrap_or_default()
}

/// A tab's label: the user's name for it, the program's or page's title, else what it is.
pub fn tab_title(tab: &Tab) -> String {
    if let Some(label) = tab.label() {
        return label.to_owned();
    }
    match tab.kind {
        TabKind::Terminal(_) => "Terminal".to_owned(),
        TabKind::Browser(_) => tab
            .url
            .as_deref()
            .and_then(host)
            .unwrap_or("New tab")
            .to_owned(),
    }
}

/// The host part of `url` (between `://` and the next `/`), or `None` without a scheme or with
/// an empty host.
fn host(url: &str) -> Option<&str> {
    url.split("://")
        .nth(1)?
        .split('/')
        .next()
        .filter(|host| !host.is_empty())
}

impl App {
    /// The find bar, shown in the header of the pane whose active tab it searches.
    fn search_bar(&self, pane: &Pane) -> Option<Element<'_, Message>> {
        let search = self.search.as_ref()?;
        let tab = self.model.tab(pane.active)?;
        let TabKind::Terminal(session) = tab.kind else {
            return None;
        };
        // Only the pane whose active tab is the session being searched; switching tabs hides
        // the bar without closing it.
        if session != search.session {
            return None;
        }
        let c = self.chrome;
        let step = |glyph: Icon, direction| icon_button(c, glyph, Message::SearchStep(direction));
        let status = if search.found { "" } else { "No matches" };
        Some(
            row![
                text(status)
                    .size(c.s.controls.field_text_size - 1.0)
                    .color(c.danger),
                text_input("Find", &search.query)
                    .id(SEARCH_INPUT)
                    .on_input(Message::SearchInput)
                    .on_submit(Message::SearchStep(macaroni_term::SearchDirection::Older))
                    .size(c.s.controls.field_text_size)
                    .padding(FIELD_PADDING)
                    .style(move |_, status| input_style(c, status))
                    .width(SEARCH_WIDTH),
                step(Icon::Up, macaroni_term::SearchDirection::Older),
                step(Icon::Down, macaroni_term::SearchDirection::Newer),
                icon_button(c, Icon::Close, Message::SearchClose),
            ]
            .spacing(SEARCH_SPACING)
            .align_y(alignment::Vertical::Center)
            .into(),
        )
    }
}

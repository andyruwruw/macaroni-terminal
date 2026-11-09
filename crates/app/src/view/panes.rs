//! The active workspace's panes: each one's card, the divider handles between them, and the
//! body of a browser tab.
//!
//! [`App::workspace`] runs inside `App::workspace_area`'s `responsive`, so it gets the pane
//! area's real size and lays panes out with `SplitNode::panes`, the same geometry
//! `App::drop_target` and divider dragging use. Each pane is a `pin` in one `stack`, inset by
//! half of `style.panes.gap` so the window background shows between panes. A terminal tab's
//! body is a [`TerminalView`]; a browser tab's is an address bar over a [`BrowserHost`], the
//! placeholder the native CEF window is laid over. Plugins' rows go above and below a tab's
//! body ([`App::with_plugin_rows`]).

use iced::widget::{
    Space, button, column, container, mouse_area, pin, rich_text, row, span, stack, text,
    text_input,
};
use iced::{Border, Color, Element, Fill, Size, alignment, mouse};
use macaroni_config::Position;
use macaroni_core::{Axis, BrowserId, PaneId, Tab, TabKind};

use super::tabs::strip_scrollbar_room;
use super::tabs::tabs_flush;
use super::widgets::{card_style, flat_button, icon_button, input_style};
use super::{pane_area, pane_overlap};
use crate::browser::BrowserHost;
use crate::icons::{Icon, icon};
use crate::plugins::{Row, SpanColor};
use crate::terminal::TerminalView;
use crate::theme::to_iced;
use crate::{App, Message, Nav, glass};

/// The narrowest a divider's handle gets. Solid panes have no gap between them, so a handle
/// only as wide as the gap couldn't be grabbed; this one reaches over both panes' edges.
const MIN_HANDLE: f32 = 8.0;

/// A browser tab's back, forward, reload and dev tools buttons: their icon size.
const NAV_ICON_SIZE: f32 = 16.0;

/// Around a browser navigation button's icon.
const NAV_PADDING: f32 = 6.0;

/// Inside the address field, above and below then left and right.
const ADDRESS_PADDING: [f32; 2] = [6.0, 12.0];

/// Between the address bar's buttons and field.
const ADDRESS_SPACING: f32 = 4.0;

/// Around the address bar.
const ADDRESS_BAR_PADDING: f32 = 2.0;

/// Between the address bar and the page.
const ADDRESS_BAR_GAP: f32 = 6.0;

/// Inside the page find card, around its row.
const FIND_CARD_PADDING: f32 = 6.0;

/// The size of the letters `jump_to_pane` shows over the panes.
const PANE_LETTER_SIZE: f32 = 44.0;

/// The text size of `[workspace_items] pane_badge`.
const PANE_BADGE_SIZE: f32 = 40.0;

/// How strongly the pane badge shows over the terminal.
const PANE_BADGE_ALPHA: f32 = 0.14;

/// Between the pane badge and the pane's top and right edges.
const PANE_BADGE_MARGIN: f32 = 16.0;

/// How strongly dimmed plugin text shows.
const DIM_ALPHA: f32 = 0.55;

/// The text size of the message shown when a page can't open.
const ERROR_TEXT_SIZE: f32 = 13.0;

impl App {
    /// The active workspace's panes and divider handles, laid out in `size` (the pane area,
    /// from `responsive`); a maximized pane alone, without handles. The focused pane gets a ring
    /// only when there's more than one pane on screen to tell apart; a waiting tab's attention
    /// ring wins over it.
    pub(super) fn workspace(&self, size: Size) -> Element<'_, Message> {
        let c = self.chrome;
        let ws = self.shown_workspace();
        let area = pane_area(c, size);

        let panes = ws.shown_panes(area);
        let split = panes.len() > 1;
        let inset = c.s.panes.gap / 2.0;
        let grow = pane_overlap(c) - c.s.panes.gap;
        let mut layers: Vec<Element<'_, Message>> = panes
            .into_iter()
            .map(|(pane, r)| {
                let ring = if self.model.pane_attention(pane) {
                    Some((c.attention, c.s.panes.attention_border_width))
                } else if split && pane == ws.focused {
                    Some((c.outline, c.s.panes.focus_border_width))
                } else {
                    None
                };
                pin(container(self.pane(
                    pane,
                    pane == ws.focused,
                    ring,
                    Size::new(r.width, r.height),
                ))
                .width((r.width + grow).max(0.0))
                .height((r.height + grow).max(0.0)))
                .x(r.x + inset)
                .y(r.y + inset)
                .into()
            })
            .collect();

        // Each handle fills the gap at its boundary (centered on `ratio`, a gap wide), so it's
        // grabbable where the window background shows between the panes; at least
        // `MIN_HANDLE` wide, over the panes' edges, where the gap is narrower or none.
        let thickness = c.s.panes.gap.max(MIN_HANDLE);
        for divider in ws.shown_dividers(area) {
            let a = divider.area;
            let (x, y, width, height, interaction) = match divider.axis {
                Axis::Row => (
                    a.x + a.width * divider.ratio - thickness / 2.0,
                    a.y,
                    thickness,
                    a.height,
                    mouse::Interaction::ResizingHorizontally,
                ),
                Axis::Column => (
                    a.x,
                    a.y + a.height * divider.ratio - thickness / 2.0,
                    a.width,
                    thickness,
                    mouse::Interaction::ResizingVertically,
                ),
            };
            let handle = mouse_area(Space::new().width(Fill).height(Fill))
                .on_press(Message::DragStart(ws.id, divider))
                .interaction(interaction);
            layers.push(
                pin(container(handle).width(width).height(height))
                    .x(x)
                    .y(y)
                    .into(),
            );
        }

        // `[workspace_items] pane_badge`: drawn over the terminal, faded, since an opaque terminal
        // background would hide anything behind it.
        let template = &self.config.workspace_items.pane_badge;
        if !template.trim().is_empty() {
            let index = self
                .shown()
                .workspaces
                .iter()
                .position(|w| w.id == ws.id)
                .unwrap_or(0);
            let values = super::sidebar::WorkspaceValues::new(self, index, ws);
            let label = crate::label::fill(template, |key| values.get(key));
            if !label.trim().is_empty() {
                // Below the tab strip, so it sits on the terminal.
                let top = c.s.tabs.height + c.s.tabs.gap + PANE_BADGE_MARGIN;
                for (_, r) in ws.shown_panes(area) {
                    layers.push(
                        pin(container(
                            text(label.clone())
                                .size(PANE_BADGE_SIZE)
                                .color(c.text.scale_alpha(PANE_BADGE_ALPHA)),
                        )
                        .align_right(r.width - PANE_BADGE_MARGIN * 2.0)
                        .clip(true))
                        .x(r.x + inset + PANE_BADGE_MARGIN)
                        .y(r.y + inset + top)
                        .into(),
                    );
                }
            }
        }

        // `jump_to_pane`: a letter in the middle of each pane, in `shown_panes` order.
        if self.pane_letters {
            for (index, (_, r)) in ws.shown_panes(area).into_iter().enumerate() {
                let Some(letter) = crate::update::shortcuts::pane_letter(index) else {
                    break;
                };
                let badge = container(
                    text(letter.to_ascii_uppercase().to_string())
                        .size(PANE_LETTER_SIZE)
                        .color(c.text),
                )
                .padding([6, 22])
                .style(move |_| container::Style {
                    background: Some(c.popup.into()),
                    border: iced::Border {
                        color: c.outline,
                        width: 2.0,
                        radius: 12.0.into(),
                    },
                    ..container::Style::default()
                });
                layers.push(
                    pin(container(badge)
                        .center(Fill)
                        .width(r.width)
                        .height(r.height))
                    .x(r.x + inset)
                    .y(r.y + inset)
                    .into(),
                );
            }
        }

        stack(layers).width(Fill).height(Fill).into()
    }

    /// The tab strip, and below it the active tab in a rounded card. `ring` outlines the card:
    /// the attention color when a tab in the pane is waiting, a light outline on the focused
    /// pane when there's more than one to tell apart.
    /// `size` is the pane's slot before the gap inset; the strip gets the inset width.
    pub(super) fn pane(
        &self,
        id: PaneId,
        focused: bool,
        ring: Option<(Color, f32)>,
        size: Size,
    ) -> Element<'_, Message> {
        let c = self.chrome;
        let width = size.width - c.s.panes.gap + pane_overlap(c);
        let Some(pane) = self.model.pane(id) else {
            return Space::new().into();
        };
        let Some(tab) = self.model.tab(pane.active) else {
            return Space::new().into();
        };
        let body: Element<'_, Message> = match tab.kind {
            TabKind::Terminal(session) => {
                let snapshot = self
                    .running
                    .get(&session)
                    .map_or(&self.empty, |r| &r.snapshot);
                self.terminal_view(session, snapshot, focused, c.s.panes.terminal_padding)
            }
            TabKind::Browser(id) => self.browser_body(id),
            TabKind::Git => self.git_body(tab.id, width),
            TabKind::AgentSetup => self.agent_body(tab.id, width),
            TabKind::File => self.file_body(tab.id),
            TabKind::Tmux => self.tmux_body(tab.id, focused),
        };
        let body = self.with_plugin_rows(tab, body);

        let card = container(body)
            .width(Fill)
            .height(Fill)
            .padding(c.s.panes.padding)
            .style(move |_| container::Style {
                background: Some(c.card.into()),
                // iced draws borders over the padding without taking room, so a ring (of any
                // width) never changes the terminal's size and makes it reflow. In glass the
                // sheet's rim outlines an unringed card instead of `card_border`.
                border: Border {
                    color: match (ring, c.glass) {
                        (Some((color, _)), _) => color,
                        (None, Some(_)) => Color::TRANSPARENT,
                        (None, None) => c.card_border,
                    },
                    width: ring.map_or(c.s.panes.border_width, |(_, width)| width),
                    radius: c.s.panes.radius.into(),
                },
                shadow: c.shadow,
                ..container::Style::default()
            });
        let card = glass::panel(c, card, c.s.panes.radius, size);
        let strip = self.tab_strip(pane, width);
        // Flush, the strip is drawn over the card so the open tab can hang down over the card's
        // top border; the card still starts `tabs.height` down.
        // The tab slot's color goes behind the strip, not over the card's top border.
        let slot = move |_: &iced::Theme| card_style(c.tab_strip, 0.0);
        if tabs_flush(c) {
            let above = container(Space::new().width(Fill).height(c.s.tabs.height)).style(slot);
            return stack![column![above, card], strip].into();
        }
        // The strip is `strip_scrollbar_room` taller than the tabs, so the spacing gives that
        // back: the card starts `tabs.height + tabs.gap` down, where drop targets and browser
        // bounds expect it.
        column![container(strip).width(Fill).style(slot), card]
            .spacing(c.s.tabs.gap - strip_scrollbar_room(c))
            .into()
    }

    /// `body` with the rows of the plugins on in `tab` above and below it, each plugin's in
    /// config order. The body keeps whatever room is left, so a terminal loses rows to them.
    fn with_plugin_rows<'a>(
        &'a self,
        tab: &Tab,
        body: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let plugins = self.tab_plugins(tab);
        if plugins
            .iter()
            .all(|(_, rows)| rows.is_none_or(|r| r.rows.is_empty()))
        {
            return body;
        }
        let rows = |position| {
            let rows: Vec<Element<'a, Message>> = plugins
                .iter()
                .filter(|(plugin, _)| plugin.position == position)
                .flat_map(|(_, rows)| rows.map(|r| r.rows.as_slice()).unwrap_or_default())
                .map(|row| self.plugin_row(row))
                .collect();
            (!rows.is_empty()).then(|| {
                container(column(rows))
                    .padding([0.0, self.chrome.s.panes.terminal_padding])
                    .width(Fill)
            })
        };
        let top = rows(Position::Top);
        let bottom = rows(Position::Bottom);
        column![]
            .push(top)
            .push(container(body).width(Fill).height(Fill))
            .push(bottom)
            .into()
    }

    /// One plugin row in the terminal's font, its colors from the terminal's palette; text
    /// without a color is in the label color.
    fn plugin_row<'a>(&self, row: &'a Row) -> Element<'a, Message> {
        let c = self.chrome;
        let m = self.metrics;
        let spans = row.0.iter().map(|s| {
            let color = match s.color {
                Some(SpanColor::Indexed(i)) => to_iced(self.palette.indexed(i), 1.0),
                Some(SpanColor::Rgb(rgb)) => to_iced(rgb, 1.0),
                None => c.label,
            };
            let mut font = m.font;
            if s.bold {
                font.weight = iced::font::Weight::Bold;
            }
            if s.italic {
                font.style = iced::font::Style::Italic;
            }
            span(s.text.as_str())
                .color(Color {
                    a: if s.dim { color.a * DIM_ALPHA } else { color.a },
                    ..color
                })
                .font(font)
                .underline(s.underline)
        });
        rich_text::<(), Message, _, _>(spans.collect::<Vec<_>>())
            .size(m.font_size)
            .line_height(iced::widget::text::LineHeight::Relative(m.line_height))
            .wrapping(iced::widget::text::Wrapping::None)
            .width(Fill)
            .into()
    }

    /// A terminal's view of `session`, with `padding` around its grid. It takes the keyboard
    /// when `focused` and nothing else wants typing.
    pub(crate) fn terminal_view<'a>(
        &'a self,
        session: macaroni_core::SessionId,
        snapshot: &'a macaroni_term::Snapshot,
        focused: bool,
        padding: f32,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let view = TerminalView::new(
            session,
            snapshot,
            &self.palette,
            self.metrics,
            &self.keymap,
            focused
                && self.renaming.is_none()
                && self.search.is_none()
                && self.menu.is_none()
                && self.settings.is_none()
                && self.icon_picker.is_none()
                && self.folder_dialog.is_none()
                && self.command_palette.is_none()
                && self.worktree_dialog.is_none()
                && !self.dashboard
                && !self.pane_letters
                && self.env_dialog.is_none()
                && !self.sidebar_search_focused
                && self.key_focus.is_none(),
            move |event| Message::View(session, event),
        )
        .transparent(c.glass.is_some())
        .padding(padding)
        .enter_renames(self.item_focus.is_some())
        .wheel_lines(self.config.terminal.scroll_lines as f32)
        .copy_on_select(self.config.terminal.copy_on_select)
        .highlights(self.triggers.highlights());
        // An id, so screen readers can be told where its lines are.
        iced::widget::container(view)
            .id(crate::accessibility::tree::terminal_id(session))
            .into()
    }

    /// The address bar, and below it the space the browser window is laid over (or the error
    /// when the page couldn't open). Reload and dev tools are disabled until the browser has
    /// started; while a page loads, reload becomes stop.
    fn browser_body(&self, id: BrowserId) -> Element<'_, Message> {
        let c = self.chrome;
        let Some(panel) = self.browsers.get(&id) else {
            return Space::new().into();
        };
        let nav = |glyph: Icon, nav: Nav, enabled: bool| {
            button(icon(
                glyph,
                NAV_ICON_SIZE,
                if enabled { c.label } else { c.faint },
            ))
            .on_press_maybe(enabled.then_some(Message::Nav(id, nav)))
            .style(move |_, status| flat_button(c, status, None))
            .padding(NAV_PADDING)
        };
        let started = panel.browser.is_some();
        let bar = row![
            nav(Icon::Back, Nav::Back, panel.back),
            nav(Icon::Forward, Nav::Forward, panel.forward),
            if panel.loading {
                nav(Icon::Close, Nav::Stop, true)
            } else {
                nav(Icon::Reload, Nav::Reload, started)
            },
            text_input("Search or enter an address", &panel.address)
                .on_input(move |text| Message::AddressInput(id, text))
                .on_submit(Message::AddressSubmit(id))
                .size(c.s.panes.address_size)
                .padding(ADDRESS_PADDING)
                .style(move |_, status| input_style(c, status))
                .width(Fill),
            nav(Icon::DevTools, Nav::DevTools, started),
        ]
        .spacing(ADDRESS_SPACING)
        .align_y(alignment::Vertical::Center);

        let page: Element<'_, Message> = match &panel.error {
            Some(err) => container(text(err.as_str()).size(ERROR_TEXT_SIZE).color(c.danger))
                .center(Fill)
                .into(),
            None => BrowserHost::new(id, move |bounds| Message::BrowserBounds(id, bounds)).into(),
        };
        column![container(bar).padding(ADDRESS_BAR_PADDING)]
            .push(self.page_find(id))
            .push(page)
            .spacing(ADDRESS_BAR_GAP)
            .into()
    }

    /// The find card for page `id`, between the address bar and the page while the find bar
    /// searches it; `None` otherwise. It can't float over the page: the page is a native window
    /// that covers anything the app draws there, so the page gives up the card's height instead.
    fn page_find(&self, id: BrowserId) -> Option<Element<'_, Message>> {
        use macaroni_term::SearchDirection;
        let search = self
            .search
            .as_ref()
            .filter(|s| s.target == crate::state::SearchTarget::Page(id))?;
        let c = self.chrome;
        let status = if search.found { "" } else { "No matches" };
        let card = row![
            icon(Icon::Globe, NAV_ICON_SIZE, c.label),
            text("Find in page")
                .size(c.s.controls.field_text_size)
                .color(c.label),
            text_input("Text on this page", &search.query)
                .id(super::SEARCH_INPUT)
                .on_input(Message::SearchInput)
                .on_submit(Message::SearchStep(SearchDirection::Newer))
                .size(c.s.controls.field_text_size)
                .padding(ADDRESS_PADDING)
                .style(move |_, status| input_style(c, status))
                .width(Fill),
            text(status)
                .size(c.s.controls.field_text_size - 1.0)
                .color(c.danger),
            icon_button(c, Icon::Up, Message::SearchStep(SearchDirection::Older)),
            icon_button(c, Icon::Down, Message::SearchStep(SearchDirection::Newer)),
            icon_button(c, Icon::Close, Message::SearchClose),
        ]
        .spacing(ADDRESS_SPACING)
        .align_y(alignment::Vertical::Center);
        Some(
            container(container(card).padding(FIND_CARD_PADDING).style(move |_| {
                container::Style {
                    background: Some(c.popup.into()),
                    border: Border {
                        color: c.outline,
                        width: 1.0,
                        radius: c.s.controls.radius.into(),
                    },
                    ..container::Style::default()
                }
            }))
            .padding(ADDRESS_BAR_PADDING)
            .into(),
        )
    }
}

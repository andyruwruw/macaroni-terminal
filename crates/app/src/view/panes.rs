//! The active workspace's panes: each one's card, the divider handles between them, and the
//! body of a browser tab.
//!
//! [`App::workspace`] runs inside `App::workspace_area`'s `responsive`, so it gets the pane
//! area's real size and lays panes out with `SplitNode::panes`, the same geometry
//! `App::drop_target` and divider dragging use. Each pane is a `pin` in one `stack`, inset by
//! half of `style.panes.gap` so the window background shows between panes. A terminal tab's
//! body is a [`TerminalView`]; a browser tab's is an address bar over a [`BrowserHost`], the
//! placeholder the native CEF window is laid over.

use iced::widget::{
    Space, button, column, container, mouse_area, pin, row, stack, text, text_input,
};
use iced::{Border, Color, Element, Fill, Size, alignment, mouse};
use macaroni_core::{Axis, BrowserId, PaneId, TabKind};

use super::tabs::strip_scrollbar_room;
use super::tabs::tabs_flush;
use super::widgets::{card_style, flat_button, input_style};
use super::{pane_area, pane_overlap};
use crate::browser::BrowserHost;
use crate::icons::{Icon, icon};
use crate::terminal::TerminalView;
use crate::{App, Message, Nav, glass};

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

/// The text size of the message shown when a page can't open.
const ERROR_TEXT_SIZE: f32 = 13.0;

impl App {
    /// The active workspace's panes and divider handles, laid out in `size` (the pane area,
    /// from `responsive`). The focused pane gets a ring only when there's more than one pane
    /// to tell apart; a waiting tab's attention ring wins over it.
    pub(super) fn workspace(&self, size: Size) -> Element<'_, Message> {
        let c = self.chrome;
        let ws = self.model.active_workspace();
        let area = pane_area(c, size);

        let panes = ws.layout.panes(area);
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
        // grabbable exactly where the window background shows between the panes.
        for divider in ws.layout.dividers(area) {
            let a = divider.area;
            let (x, y, width, height, interaction) = match divider.axis {
                Axis::Row => (
                    a.x + a.width * divider.ratio - c.s.panes.gap / 2.0,
                    a.y,
                    c.s.panes.gap,
                    a.height,
                    mouse::Interaction::ResizingHorizontally,
                ),
                Axis::Column => (
                    a.x,
                    a.y + a.height * divider.ratio - c.s.panes.gap / 2.0,
                    a.width,
                    c.s.panes.gap,
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
                TerminalView::new(
                    session,
                    snapshot,
                    &self.palette,
                    self.metrics,
                    &self.keymap,
                    // Keyboard focus only while nothing else wants typing.
                    focused
                        && self.renaming.is_none()
                        && self.search.is_none()
                        && self.menu.is_none()
                        && self.settings.is_none()
                        && self.icon_picker.is_none()
                        && self.folder_dialog.is_none()
                        && !self.sidebar_search_focused,
                    move |event| Message::View(session, event),
                )
                .transparent(c.glass.is_some())
                .padding(c.s.panes.terminal_padding)
                .enter_renames(self.item_focus.is_some())
                .wheel_lines(self.config.terminal.scroll_lines as f32)
                .copy_on_select(self.config.terminal.copy_on_select)
                .into()
            }
            TabKind::Browser(id) => self.browser_body(id),
        };

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
        column![container(bar).padding(ADDRESS_BAR_PADDING), page]
            .spacing(ADDRESS_BAR_GAP)
            .into()
    }
}

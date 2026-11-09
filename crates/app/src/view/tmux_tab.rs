//! A tmux tab's body: its window's panes where tmux puts them, each a terminal view, with a
//! line through each border between them. tmux lays panes out in cells, a cell apart, so the
//! body places them at cell positions rather than sharing out the space like macaroni's own
//! splits. The body reports its bounds so tmux can size the window to fit
//! (`App::tmux_bounds`).

use iced::widget::{Space, container, pin, stack};
use iced::{Element, Fill};
use macaroni_core::TabId;
use macaroni_term::{Layout, LayoutKind};

use crate::browser::BrowserHost;
use crate::{App, Message};

impl App {
    /// The panes of the tmux window tab `tab` shows; `focused` when its pane has the focus.
    pub(crate) fn tmux_body(&self, tab: TabId, focused: bool) -> Element<'_, Message> {
        let c = self.chrome;
        let host: Element<'_, Message> =
            BrowserHost::new(tab, move |bounds| Message::TmuxBounds(tab, bounds)).into();
        let Some((gateway, window)) = self.tmux_window_of(tab) else {
            return host;
        };
        let Some(t) = self.tmux.get(&gateway) else {
            return host;
        };
        let Some(w) = t.windows.get(&window) else {
            return host;
        };
        let (cw, ch) = (self.metrics.cell_width, self.metrics.cell_height);
        let active = w
            .active_pane
            .or_else(|| w.layout.panes().first().map(|p| p.0));
        let mut layers = vec![host];
        for (x, y, width, height) in borders(&w.layout) {
            layers.push(
                pin(
                    container(Space::new().width(width * cw).height(height * ch)).style(
                        move |_| container::Style {
                            background: Some(c.separator.into()),
                            ..container::Style::default()
                        },
                    ),
                )
                .x(x * cw)
                .y(y * ch)
                .into(),
            );
        }
        for (pane, x, y, width, height) in w.layout.panes() {
            let Some(id) = t.panes.get(&pane) else {
                continue;
            };
            let snapshot = self.running.get(id).map_or(&self.empty, |r| &r.snapshot);
            let view = self.terminal_view(*id, snapshot, focused && active == Some(pane), 0.0);
            layers.push(
                // Half a pixel over, so the grid's division doesn't round a column or row away;
                // the next pane is a cell further on.
                pin(container(view)
                    .width(f32::from(width) * cw + 0.5)
                    .height(f32::from(height) * ch + 0.5))
                .x(f32::from(x) * cw)
                .y(f32::from(y) * ch)
                .into(),
            );
        }
        container(stack(layers).width(Fill).height(Fill))
            .clip(true)
            .into()
    }
}

/// A thin line in the middle of each one-cell gap between a split's children, in cells:
/// `(x, y, width, height)`.
fn borders(layout: &Layout) -> Vec<(f32, f32, f32, f32)> {
    /// How thick a border line is, in cells.
    const LINE: f32 = 0.12;
    let mut out = Vec::new();
    let (children, side_by_side) = match &layout.kind {
        LayoutKind::Pane(_) => return out,
        LayoutKind::Row(children) => (children, true),
        LayoutKind::Column(children) => (children, false),
    };
    for pair in children.windows(2) {
        let first = &pair[0];
        if side_by_side {
            let x = f32::from(first.x) + f32::from(first.width) + 0.5 - LINE / 2.0;
            out.push((x, f32::from(layout.y), LINE, f32::from(layout.height)));
        } else {
            let y = f32::from(first.y) + f32::from(first.height) + 0.5 - LINE / 2.0;
            out.push((f32::from(layout.x), y, f32::from(layout.width), LINE));
        }
    }
    for child in children {
        out.extend(borders(child));
    }
    out
}

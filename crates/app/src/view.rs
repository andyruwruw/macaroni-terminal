//! The window layout: the title bar across the top, the workspace sidebar on the left, the
//! active workspace's panes on the right, and the layers drawn over them (resize edges, drop
//! markers, the drag ghost, right-click menus and the settings dialog).
//!
//! [`App::view`] is iced's view function. Each part lives in a submodule that adds methods to
//! [`App`]: [`title_bar`], [`sidebar`], [`panes`] (cards, dividers), [`tabs`] (the strip and
//! find bar), [`drag`] (drop targets for dragged tabs and workspaces), [`menus`] and
//! [`settings`]; [`widgets`] holds the small shared pieces. Every function reads its sizes and
//! colors from [`Chrome`] (passed or taken as `c`), so nothing here is a fixed size.
//!
//! Pane geometry comes from `SplitNode::panes`, so what's drawn matches what directional focus,
//! divider dragging and drop targets compute. Each pane and divider handle is a `pin` in one
//! `stack`.

pub mod folder_dialog;

pub use sidebar::SIDEBAR_SEARCH;
pub mod icon_picker;

pub use sidebar::compact_width as sidebar_compact_width;
pub mod settings;

mod drag;
mod menus;
mod panes;
mod sidebar;
mod tabs;
mod title_bar;
mod widgets;

use iced::widget::{Space, column, container, mouse_area, opaque, pin, responsive, row, stack};
use iced::{Border, Element, Fill, Point, Size, Vector, alignment};

use crate::icons::{self, workspace_icon};
use crate::theme::Chrome;
use crate::{App, Dragged, MenuPage, Message, backdrop};
pub use tabs::tab_title;
pub use tabs::{tab_span, tab_strip_id};
use title_bar::{BANNER_MARGIN, resize_edges};
use widgets::{card_style, one_line, shadow, truncate};

/// macaroni draws its own title bar on Windows; elsewhere the system's stays.
pub const CUSTOM_FRAME: bool = cfg!(windows);

/// The widget id of the in-place rename field, shared by workspaces, tabs and groups (only one
/// rename is open at a time), so `update::rename` can focus it and select the old name.
pub const RENAME_INPUT: &str = "workspace-rename";

/// The widget id of the terminal find bar's field, focused when the bar opens.
pub const SEARCH_INPUT: &str = "terminal-search";

/// Where the drag preview sits from the cursor, clear of the pointer.
const GHOST_OFFSET: Vector = Vector::new(12.0, 8.0);

/// Inside the drag preview, above and below then left and right.
const GHOST_PADDING: [f32; 2] = [8.0, 12.0];

/// Between the drag preview's icon and title.
const GHOST_SPACING: f32 = 8.0;

/// The drag preview's shadow: how far it falls, and its blur.
const GHOST_SHADOW: (f32, f32) = (4.0, 12.0);

/// Where the pane area starts in the window: right of the sidebar and, with [`CUSTOM_FRAME`],
/// below the title bar and its gap. Divider dragging and drop targets subtract it from the
/// cursor's window position to get pane-area coordinates, so it must match [`App::window`]'s
/// layout. Banners above the panes aren't counted.
pub fn workspace_origin(c: Chrome) -> Point {
    Point::new(
        c.s.window.sidebar_width + sidebar_margin(c),
        if CUSTOM_FRAME {
            c.s.window.title_bar_height + c.s.window.title_bar_gap
        } else {
            0.0
        },
    )
}

/// The rectangle `SplitNode::panes` lays a workspace's panes out in, for a pane area `size`
/// big. Each pane is pinned half a gap in from its share on every side, so the gap between two
/// panes is a whole one; laying out in an area half a gap bigger all round puts the outer panes'
/// edges exactly on the pane area's, so the margins around the panes are only their own settings
/// (`style.window.title_bar_gap`, `style.sidebar.panes_gap`, and a gap on the right and bottom).
///
/// With [`pane_overlap`] each pane is pinned that much wider and taller than its share, over its
/// neighbours' borders, so the area is that much smaller to keep the last ones inside.
pub fn pane_area(c: Chrome, size: Size) -> macaroni_core::Rect {
    let inset = c.s.panes.gap / 2.0;
    let overlap = pane_overlap(c);
    macaroni_core::Rect {
        x: -inset,
        y: -inset,
        width: size.width + c.s.panes.gap - overlap,
        height: size.height + c.s.panes.gap - overlap,
    }
}

/// How far each pane reaches over its right and lower neighbours: with no gap between panes in
/// solid, by its border, so two panes' borders are one line rather than a doubled one.
pub fn pane_overlap(c: Chrome) -> f32 {
    if c.glass.is_none() && c.s.panes.gap == 0.0 {
        c.s.panes.border_width
    } else {
        0.0
    }
}

/// Which part of the window [`with_title_line`] draws the title bar's bottom line on.
#[derive(Clone, Copy)]
pub enum TitleLine {
    /// The title bar, over its last pixels.
    TitleBar,
    /// The sidebar, over its first pixels.
    Sidebar,
    /// The pane area, under the panes, so their tabs' top borders lie on it.
    Panes,
}

/// `el`, one part of the window, with the title bar's bottom line (`Chrome.title_bar_border`,
/// `style.window.title_bar_border_width` thick) where that part has it. With space below the
/// title bar the line is the title bar's last pixels. Without, it's the first pixels of what's
/// under the title bar instead, so the tabs' top borders and it are one line rather than a
/// doubled one: over the sidebar, and under the panes, where tab fills and borders draw over
/// it and a tab's accent line stays whole. Only where macaroni draws the title bar, in solid.
pub fn with_title_line<'a>(
    c: Chrome,
    el: Element<'a, Message>,
    part: TitleLine,
) -> Element<'a, Message> {
    let width = c.s.window.title_bar_border_width;
    if !CUSTOM_FRAME || c.glass.is_some() || c.title_bar_border.a == 0.0 || width <= 0.0 {
        return el;
    }
    let line = || {
        container(Space::new().width(Fill).height(width))
            .style(move |_| card_style(c.title_bar_border, 0.0))
    };
    match (c.s.window.title_bar_gap > 0.0, part) {
        (true, TitleLine::TitleBar) => stack![
            el,
            container(line())
                .height(Fill)
                .align_y(alignment::Vertical::Bottom)
        ]
        .into(),
        (false, TitleLine::Sidebar) => stack![el, line()].into(),
        (false, TitleLine::Panes) => stack![line(), el].width(Fill).height(Fill).into(),
        _ => el,
    }
}

/// The line down the sidebar's right side (`Chrome.sidebar_border`, `style.sidebar.border_width`
/// wide), laid over the sidebar and panes so it runs their whole height. Its own pixels are the
/// sidebar's last ones, unless the panes are right against the sidebar with a border of their
/// own: then it's drawn on their first ones, over their left border, so the two are one line
/// rather than a doubled one. `None` without a sidebar or with nothing to draw.
pub fn sidebar_edge<'a>(c: Chrome) -> Option<Element<'a, Message>> {
    let width = c.s.sidebar.border_width;
    let side = c.s.window.sidebar_width;
    if c.glass.is_some() || c.sidebar_border.a == 0.0 || width <= 0.0 || side <= 0.0 {
        return None;
    }
    let shared = c.s.sidebar.panes_gap == 0.0 && c.s.panes.border_width > 0.0;
    let x = if shared { side } else { side - width };
    Some(
        row![
            Space::new().width(x),
            container(Space::new().width(width).height(Fill))
                .style(move |_| card_style(c.sidebar_border, 0.0)),
        ]
        .height(Fill)
        .into(),
    )
}

/// The room between the sidebar and the panes (`style.sidebar.panes_gap`), or none with the
/// sidebar folded away.
pub fn sidebar_margin(c: Chrome) -> f32 {
    if c.s.window.sidebar_width > 0.0 {
        c.s.sidebar.panes_gap
    } else {
        0.0
    }
}

impl App {
    /// iced's view function: the window, then each overlay as a higher `stack` layer, in the
    /// order they must cover each other (the settings dialog over everything).
    pub fn view(&self) -> Element<'_, Message> {
        let c = self.chrome;
        let mut layers: Vec<Element<'_, Message>> = vec![self.window()];
        if CUSTOM_FRAME && !self.maximized {
            layers.push(resize_edges(c));
        }
        layers.extend(self.drop_marker());
        layers.extend(self.drag_ghost());
        layers.extend(self.menu_layers());
        layers.extend(self.icon_picker_view());
        layers.extend(self.folder_dialog_view());
        layers.extend(self.settings_view());
        stack(layers).into()
    }

    /// The title bar, sidebar and panes, over the glass backdrop if there is one.
    fn window(&self) -> Element<'_, Message> {
        let c = self.chrome;
        let main: Element<'_, Message> = column![]
            .push(self.git_bash_offer())
            .push(self.workspace_area())
            .into();
        // The title bar's gap is padding inside the panes rather than column spacing, so a
        // colored sidebar runs up to the title bar; the sidebar has its own top padding.
        let gap = if CUSTOM_FRAME {
            c.s.window.title_bar_gap
        } else {
            0.0
        };
        let main = container(main).padding(iced::Padding {
            top: gap,
            left: sidebar_margin(c),
            ..iced::Padding::ZERO
        });
        let panes = row![]
            // Folded under the custom title bar, the sidebar is gone entirely.
            .push((self.chrome.s.window.sidebar_width > 0.0).then(|| {
                with_title_line(
                    c,
                    container(self.sidebar())
                        .height(Fill)
                        .style(move |_| card_style(c.sidebar, 0.0))
                        .into(),
                    TitleLine::Sidebar,
                )
            }))
            .push(with_title_line(c, main.into(), TitleLine::Panes));
        let body = column![]
            .push(CUSTOM_FRAME.then(|| with_title_line(c, self.title_bar(), TitleLine::TitleBar)))
            .push(stack![panes].push(sidebar_edge(c)));
        // In glass the container stays clear so the backdrop shader under it shows through.
        let window = container(body)
            .width(Fill)
            .height(Fill)
            .style(move |_| container::Style {
                background: c.glass.is_none().then(|| c.background.into()),
                ..container::Style::default()
            });
        if c.glass.is_some() {
            stack([backdrop::window(c.backdrop), window.into()]).into()
        } else {
            window.into()
        }
    }

    /// The panes, with the config error banner floating over their bottom edge.
    fn workspace_area(&self) -> Element<'_, Message> {
        let c = self.chrome;
        // The outer panes reach the edges of what `responsive` reports (see `pane_area`); this
        // keeps `style.panes.margin` on the right and bottom, where the panes meet the window's
        // edge (the sidebar and title bar border the other two sides, with their own margins).
        // The padded size is the same area `drop_target` and divider dragging lay out.
        let workspace =
            container(responsive(move |size| self.workspace(size))).padding(iced::Padding {
                right: c.s.panes.margin,
                bottom: c.s.panes.margin,
                ..iced::Padding::ZERO
            });
        let mut workspace = stack([workspace.into()]);
        if let Some(banner) = self.banner() {
            workspace = workspace.push(
                container(opaque(banner))
                    .width(Fill)
                    .height(Fill)
                    .align_x(alignment::Horizontal::Center)
                    .align_bottom(Fill)
                    .padding(iced::Padding {
                        right: c.s.panes.margin + BANNER_MARGIN,
                        bottom: c.s.panes.margin + BANNER_MARGIN,
                        left: BANNER_MARGIN,
                        top: BANNER_MARGIN,
                    }),
            );
        }
        workspace.into()
    }

    /// The area a dragged tab would land in, tinted and outlined.
    fn drop_marker(&self) -> Option<Element<'_, Message>> {
        let c = self.chrome;
        let (_, Some(marker)) = self.drop_target()? else {
            return None;
        };
        Some(
            pin(container(Space::new())
                .width(marker.width)
                .height(marker.height)
                .style(move |_| container::Style {
                    background: Some(c.attention.scale_alpha(c.s.effects.drop_tint).into()),
                    border: Border {
                        color: c.attention,
                        width: c.s.controls.highlight_width,
                        radius: c.s.panes.radius.into(),
                    },
                    ..container::Style::default()
                }))
            .x(marker.x)
            .y(marker.y)
            .into(),
        )
    }

    /// What follows the cursor while a tab or workspace is dragged: its icon and title.
    fn drag_ghost(&self) -> Option<Element<'_, Message>> {
        let c = self.chrome;
        let drag = self.item_drag.filter(|d| d.dragging)?;
        let (glyph, title) = match drag.item {
            Dragged::Tab(tab) => self
                .model
                .tab(tab)
                .map(|t| (tabs::tab_icon(t), tab_title(t))),
            Dragged::Workspace(ws) => self
                .model
                .workspace(ws)
                .map(|w| (workspace_icon(w.icon.as_deref()), w.name.clone())),
        }?;
        let ghost = container(
            row![
                icons::glyph(glyph, c.s.tabs.icon_size, c.text, 1.0),
                one_line(
                    truncate(&title, c.s.tabs.max_title_chars as usize),
                    c.s.tabs.title_size,
                    c.text
                ),
            ]
            .spacing(GHOST_SPACING)
            .align_y(alignment::Vertical::Center),
        )
        .padding(GHOST_PADDING)
        .style(move |_| container::Style {
            background: Some(c.raised.scale_alpha(c.s.effects.drag_preview).into()),
            border: Border {
                color: c.outline,
                width: c.s.controls.border_width,
                radius: c.s.tabs.radius.into(),
            },
            shadow: shadow(c.menu_shadow, GHOST_SHADOW.0, GHOST_SHADOW.1),
            ..container::Style::default()
        });
        let at = drag.at + GHOST_OFFSET;
        Some(pin(ghost).x(at.x).y(at.y).into())
    }

    /// The open right-click menu's lists, over a layer that closes it on a click elsewhere.
    fn menu_layers(&self) -> Vec<Element<'_, Message>> {
        let Some(menu) = self.menu else {
            return Vec::new();
        };
        let mut layers: Vec<Element<'_, Message>> = vec![
            mouse_area(Space::new().width(Fill).height(Fill))
                .on_press(Message::MenuClose)
                .on_right_press(Message::MenuClose)
                .into(),
        ];
        let open = menu.sub.map(|s| s.page);
        let pages = std::iter::once(MenuPage::Main).chain(open);
        for (page, bounds) in pages.zip(self.menu_bounds()) {
            layers.push(
                pin(self.menu_list(menu.target, page, open))
                    .x(bounds.x)
                    .y(bounds.y)
                    .into(),
            );
        }
        layers
    }
}

//! The agent dashboard (Ctrl+Shift+A, Cmd+Shift+A on macOS): a card for every coding agent in
//! every workspace, most urgent first, each showing what it's doing, its status line and how
//! long it's been in its state. Clicking a card shows its tab.
//!
//! Agents come from the model (`Tab.agent`, set by Claude Code's hooks); how long each has been
//! in its state is the app's own bookkeeping, `App.agent_since`, updated when a hook reports a
//! new state (an agent restored after a restart has none until then). While the dashboard is
//! open a one-second tick redraws it, so the times count up.

use iced::widget::{button, column, container, mouse_area, opaque, row, scrollable, text};
use iced::{Border, Color, Element, Fill, Task, alignment};
use macaroni_core::{AgentState, Command, TabId};

use super::sidebar::agent_line;
use super::widgets::{flat_button, one_line, shadow};
use crate::icons::{Icon, icon};
use crate::{App, Message, glass, label};

/// The dashboard's width when the window has room.
const WIDTH: f32 = 900.0;

/// The least room it leaves at the window's sides, together.
const MARGIN: f32 = 48.0;

/// How far below the window's top it sits.
const TOP: f32 = 72.0;

/// Corner radius of the dashboard and its cards.
const RADIUS: f32 = 12.0;

/// Each card's width.
const CARD_WIDTH: f32 = 270.0;

/// The title's text size.
const TITLE_SIZE: f32 = 16.0;

/// A card's main text size.
const TEXT_SIZE: f32 = 13.0;

/// A card's smaller lines.
const DETAIL_SIZE: f32 = 11.5;

/// What the dashboard sends, wrapped in [`Message::Dashboard`].
#[derive(Debug, Clone, Copy)]
pub enum DashboardMessage {
    /// Open it, or close it when it's open.
    Toggle,
    /// Close it and show this agent's tab.
    Show(TabId),
}

impl App {
    /// Applies a dashboard message.
    pub(crate) fn dashboard_update(&mut self, message: DashboardMessage) -> Task<Message> {
        match message {
            DashboardMessage::Toggle => {
                self.menu = None;
                self.dashboard = !self.dashboard;
                Task::none()
            }
            DashboardMessage::Show(tab) => {
                self.dashboard = false;
                self.apply(Command::SelectTab(tab))
            }
        }
    }

    /// Every tab with an agent, in every window, needing input first, then working, then
    /// idle; within each, in window then sidebar order.
    fn dashboard_agents(&self) -> Vec<TabId> {
        let mut agents: Vec<(u8, usize, TabId)> = Vec::new();
        for (at, ws) in self.model.all_workspaces().into_iter().enumerate() {
            for pane in ws.layout.leaves() {
                for &tab in self
                    .model
                    .pane(pane)
                    .map(|p| p.tabs.as_slice())
                    .unwrap_or_default()
                {
                    let Some(agent) = self.model.tab(tab).and_then(|t| t.agent.as_ref()) else {
                        continue;
                    };
                    let rank = match agent.state {
                        AgentState::NeedsInput => 0,
                        AgentState::Working => 1,
                        AgentState::Idle => 2,
                    };
                    agents.push((rank, at, tab));
                }
            }
        }
        agents.sort_by_key(|(rank, at, _)| (*rank, *at));
        agents.into_iter().map(|(_, _, tab)| tab).collect()
    }

    /// The dashboard over a dimmed app, or nothing while it's closed.
    pub(crate) fn dashboard_view(&self) -> Option<Element<'_, Message>> {
        if !self.dashboard {
            return None;
        }
        let c = self.chrome;
        let agents = self.dashboard_agents();
        let header = row![
            one_line("Agents", TITLE_SIZE, c.text).width(Fill),
            one_line(
                match agents.len() {
                    1 => "1 agent".to_owned(),
                    n => format!("{n} agents"),
                },
                DETAIL_SIZE,
                c.muted
            ),
        ]
        .align_y(alignment::Vertical::Center);
        let body: Element<'_, Message> = if agents.is_empty() {
            text(
                "No coding agents are running. Start Claude Code in a terminal; its hooks report \
                 here once they're installed (Settings > Integrations > Agents).",
            )
            .size(TEXT_SIZE)
            .color(c.muted)
            .into()
        } else {
            let cards = agents.into_iter().filter_map(|tab| self.agent_card(tab));
            scrollable(row(cards).spacing(10).wrap().vertical_spacing(10))
                .height(iced::Length::Shrink)
                .into()
        };
        let card = container(column![header, body].spacing(14))
            .padding(18)
            .width((self.window_size().width - MARGIN).clamp(300.0, WIDTH))
            .max_height((self.window_size().height - TOP * 2.0).max(200.0))
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
                .width(Fill)
                .height(Fill)
                .align_x(alignment::Horizontal::Center)
                .padding(iced::Padding::default().top(TOP))
                .style(move |_| container::Style {
                    background: Some(c.overlay.into()),
                    ..container::Style::default()
                }),
        )
        .on_press(Message::Dismiss);
        Some(backdrop.into())
    }

    /// One agent's card: its tab and workspace, what it's doing (in its state's color), its
    /// status line and how long it's been in its state. A button that shows the tab.
    fn agent_card(&self, tab: TabId) -> Option<Element<'_, Message>> {
        let c = self.chrome;
        let t = self.model.tab(tab)?;
        let agent = t.agent.as_ref()?;
        let workspace = self
            .workspace_of_tab(tab)
            .and_then(|w| self.model.workspace(w))
            .map(|w| w.name.clone())
            .unwrap_or_default();
        let (doing, color) = agent_line(c, agent);
        let since = self
            .agent_since
            .get(&tab)
            .filter(|(state, _)| *state == agent.state)
            .map(|(_, at)| format!("for {}", label::duration(at.elapsed())))
            .unwrap_or_default();
        let mut lines = column![
            row![
                icon(Icon::Sparkles, 14.0, color),
                one_line(super::tabs::tab_title(t), TEXT_SIZE, c.text).width(Fill),
                one_line(since, DETAIL_SIZE, c.faint),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center),
            one_line(workspace, DETAIL_SIZE, c.muted),
            text(doing).size(DETAIL_SIZE).color(color),
        ]
        .spacing(4);
        if let Some(summary) = &agent.summary {
            lines = lines.push(one_line(summary.clone(), DETAIL_SIZE, c.muted));
        }
        if self.asks_permission(tab) {
            let answer = |label: &'static str, allow: bool| {
                button(one_line(label, DETAIL_SIZE, c.text))
                    .on_press(Message::PermissionAnswer(tab, allow))
                    .padding([4, 12])
                    .style(move |_, status| {
                        let mut style =
                            flat_button(c, status, Some(if allow { c.raised } else { c.sunken }));
                        style.border = Border {
                            color: c.separator,
                            width: 1.0,
                            radius: c.s.controls.radius.into(),
                        };
                        style
                    })
            };
            lines = lines.push(row![answer("Allow", true), answer("Deny", false)].spacing(6));
        }
        let card = button(lines)
            .on_press(Message::Dashboard(DashboardMessage::Show(tab)))
            .width(CARD_WIDTH)
            .padding(12)
            .style(move |_, status| {
                let mut style = flat_button(c, status, Some(c.sunken));
                style.border = Border {
                    color: c.separator,
                    width: 1.0,
                    radius: (RADIUS - 4.0).into(),
                };
                style
            });
        Some(card.into())
    }
}

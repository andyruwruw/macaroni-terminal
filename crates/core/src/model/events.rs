//! What programs report about their tabs: titles, working directories, notifications, coding
//! agents and browser pages.
//!
//! These commands arrive from the app's terminal sniffers, the control socket (agent hooks) and
//! browser callbacks, addressed by session, browser or tab id. They only edit fields on the tab
//! and never change structure or emit effects, so a report for a tab that has already closed is
//! silently dropped.

use super::{Agent, AgentState, Model, Notice, Tab, TabKind};
use crate::ids::{BrowserId, SessionId, TabId};

impl Model {
    /// Runs `edit` on the tab running the session, if there is one.
    pub(super) fn edit_session_tab(&mut self, session: SessionId, edit: impl FnOnce(&mut Tab)) {
        if let Some(tab) = self
            .tabs
            .values_mut()
            .find(|t| t.kind == TabKind::Terminal(session))
        {
            edit(tab);
        }
    }

    /// Runs `edit` on the tab showing the browser, if there is one.
    pub(super) fn edit_browser_tab(&mut self, browser: BrowserId, edit: impl FnOnce(&mut Tab)) {
        if let Some(tab) = self
            .tabs
            .values_mut()
            .find(|t| t.kind == TabKind::Browser(browser))
        {
            edit(tab);
        }
    }

    /// Marks the session's tab as wanting attention, unless it or its workspace is muted.
    ///
    /// The notice replaces any earlier one and takes its `seq` from the shared id counter, so
    /// [`Model::workspace_notice`] can pick the newest across tabs.
    pub(super) fn notify(&mut self, session: SessionId, title: Option<String>, body: String) {
        if let Some(id) = self.tab_for_session(session)
            && !self.is_muted(id)
        {
            let seq = self.next_id();
            self.tabs
                .get_mut(&id)
                .expect("the session's tab is in the model")
                .attention = Some(Notice { title, body, seq });
        }
    }

    /// Records the state an agent in the tab reported, keeping its session id and working
    /// directory when they're `None`. Creates the tab's [`Agent`] on the first report; `detail`
    /// is always replaced (blank clears it), and the summary is left alone.
    pub(super) fn set_agent(
        &mut self,
        tab: TabId,
        state: AgentState,
        detail: Option<String>,
        session_id: Option<String>,
        cwd: Option<String>,
    ) {
        self.edit_tab(tab, |tab| {
            let agent = tab.agent.get_or_insert(Agent {
                state,
                detail: None,
                session_id: None,
                cwd: None,
                summary: None,
            });
            agent.state = state;
            agent.detail = non_blank(detail);
            if session_id.is_some() {
                agent.session_id = session_id;
            }
            if cwd.is_some() {
                agent.cwd = cwd;
            }
        });
    }

    /// Sets the status-line summary of the agent in the tab, if one is running there; a blank
    /// summary clears it. Never creates an agent.
    pub(super) fn set_agent_summary(&mut self, tab: TabId, summary: Option<String>) {
        if let Some(agent) = self.tabs.get_mut(&tab).and_then(|t| t.agent.as_mut()) {
            agent.summary = non_blank(summary);
        }
    }
}

/// `None` for a missing or blank (all whitespace) string; otherwise the string unchanged.
pub(super) fn non_blank(s: Option<String>) -> Option<String> {
    s.filter(|s| !s.trim().is_empty())
}

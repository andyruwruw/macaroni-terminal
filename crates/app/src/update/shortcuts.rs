//! Keymap actions, from terminals, pages and keys nothing else handled.
//!
//! Every [`Action`] ends up in [`App::perform`], whichever way it was pressed: in a terminal
//! (`ViewEvent::Action`), in a page (`BrowserMessage::Action`), as a key no widget took while
//! a browser tab is focused (`Message::Key`), or from a button (`Message::Action`).

use iced::Task;
use iced::keyboard::{Key, Modifiers, key};
use macaroni_config::Config;

use crate::keymap::Action;
use crate::message::Message;
use crate::{App, open_in_editor};

/// The letter shown over the `index`th pane (from 0) while choosing one: `a` to `z`, then
/// nothing (past 26 panes, the rest can't be jumped to).
pub(crate) fn pane_letter(index: usize) -> Option<char> {
    u8::try_from(index)
        .ok()
        .filter(|i| *i < 26)
        .map(|i| char::from(b'a' + i))
}

impl App {
    /// Zooms a browser tab's page a step in or out, or back to 100%.
    fn zoom_page(&mut self, id: macaroni_core::BrowserId, zoom: crate::keymap::Zoom) {
        use crate::keymap::Zoom;
        let Some(panel) = self.browsers.get_mut(&id) else {
            return;
        };
        let step = match zoom {
            Zoom::In => Some(true),
            Zoom::Out => Some(false),
            Zoom::Reset => None,
        };
        panel.zoom = crate::browser::next_zoom(panel.zoom, step);
        if let Some(browser) = &panel.browser {
            browser.zoom(panel.zoom);
        }
    }

    /// Carries out a keymap action. Opening the config spawns the system's editor.
    pub(crate) fn perform(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::Command(command) => self.apply(command),
            Action::OpenSettings => self.open_settings(),
            Action::Zoom(zoom) => match self.model.focused_tab().kind {
                macaroni_core::TabKind::Browser(id) => {
                    self.zoom_page(id, zoom);
                    Task::none()
                }
                macaroni_core::TabKind::Terminal(_) => self.zoom_font(zoom),
            },
            Action::OpenConfig => {
                open_in_editor(&Config::path(&self.config_dir));
                Task::none()
            }
            Action::Copy => self.copy_selection(),
            Action::Paste => self.paste_clipboard(),
            Action::Find => self.open_search(),
            Action::ToggleSidebar => self.toggle_sidebar(),
            Action::CommandPalette => {
                self.palette_update(crate::view::palette::PaletteMessage::Open)
            }
            Action::Prompt(up) => {
                if let Some(id) = self.focused_session()
                    && let Some(r) = self.running.get_mut(&id)
                    && r.session.jump_to_prompt(up)
                {
                    r.snapshot = r.session.snapshot();
                }
                Task::none()
            }
            Action::ReopenTab => self.reopen_tab(),
            Action::SearchScrollback(everywhere) => {
                self.palette_update(crate::view::palette::PaletteMessage::OpenSearch(everywhere))
            }
            Action::SaveScrollback => {
                let text = self
                    .focused_session()
                    .and_then(|id| self.running.get(&id))
                    .map(|r| r.session.all_text());
                let Some(text) = text else {
                    return Task::none();
                };
                // The dialog runs as a task, so the UI keeps drawing; writing failures are
                // dropped (the user picked the place and sees the result there).
                Task::perform(
                    async move {
                        let file = rfd::AsyncFileDialog::new()
                            .set_title("Save the terminal's text")
                            .set_file_name("scrollback.txt")
                            .save_file()
                            .await;
                        if let Some(file) = file {
                            let _ = file.write(text.as_bytes()).await;
                        }
                    },
                    |()| Message::Frame,
                )
            }
            Action::JumpToPane => {
                self.menu = None;
                // With one pane there's nothing to choose between.
                self.pane_letters = self
                    .model
                    .active_workspace()
                    .shown_panes(macaroni_core::Rect::UNIT)
                    .len()
                    > 1;
                Task::none()
            }
            Action::AgentDashboard => {
                self.dashboard_update(crate::view::dashboard::DashboardMessage::Toggle)
            }
            Action::CopyOutput => {
                let output = self
                    .focused_session()
                    .and_then(|id| self.running.get(&id))
                    .and_then(|r| r.session.last_output());
                match output {
                    Some(text) => iced::clipboard::write(text),
                    None => Task::none(),
                }
            }
            Action::NewWorktree => {
                let ws = self.model.active_workspace().id;
                self.worktree_update(crate::view::worktree_dialog::WorktreeMessage::Open(ws))
            }
            Action::ShowWaiting => {
                self.palette_update(crate::view::palette::PaletteMessage::OpenWaiting)
            }
        }
    }

    /// The key pressed while pane letters show: its pane (by [`pane_letter`]) gets focus, and
    /// anything else just hides the letters.
    pub(crate) fn pane_letter(&mut self, letter: char) -> Task<Message> {
        self.pane_letters = false;
        let panes = self
            .model
            .active_workspace()
            .shown_panes(macaroni_core::Rect::UNIT);
        let at = (letter.to_ascii_lowercase() as usize).wrapping_sub('a' as usize);
        match panes.get(at) {
            Some(&(pane, _)) => self.apply(macaroni_core::Command::FocusPane(pane)),
            None => Task::none(),
        }
    }

    /// A key nothing else handled while a browser tab is focused: Enter renames the focused
    /// item, anything else may be a shortcut.
    pub(crate) fn unhandled_key(
        &mut self,
        key: Key,
        physical: key::Physical,
        mods: Modifiers,
    ) -> Task<Message> {
        if key == Key::Named(key::Named::Enter)
            && mods.is_empty()
            && let Some(target) = self.item_focus
        {
            return self.start_rename(target);
        }
        // Any other key lets go of the focused item, as typing in a terminal does.
        self.item_focus = None;
        match self.keymap.lookup(&key, physical, mods) {
            Some(action) => self.perform(action),
            None => Task::none(),
        }
    }

    /// Copies the focused terminal's selection to the clipboard, then clears it.
    fn copy_selection(&mut self) -> Task<Message> {
        let Some(id) = self.focused_session() else {
            return Task::none();
        };
        let Some(r) = self.running.get_mut(&id) else {
            return Task::none();
        };
        let Some(text) = r.session.selection_text() else {
            return Task::none();
        };
        r.session.clear_selection();
        r.snapshot = r.session.snapshot();
        iced::clipboard::write(text)
    }

    /// Reads the clipboard, to paste into the focused terminal. The text comes back as
    /// [`Message::Paste`], bound to the terminal focused now.
    fn paste_clipboard(&self) -> Task<Message> {
        let Some(id) = self.focused_session() else {
            return Task::none();
        };
        iced::clipboard::read().map(move |text| Message::Paste(id, text))
    }
}

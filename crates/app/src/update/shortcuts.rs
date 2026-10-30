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

impl App {
    /// Carries out a keymap action. Opening the config spawns the system's editor.
    pub(crate) fn perform(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::Command(command) => self.apply(command),
            Action::OpenSettings => self.open_settings(),
            Action::Zoom(zoom) => self.zoom_font(zoom),
            Action::OpenConfig => {
                open_in_editor(&Config::path(&self.config_dir));
                Task::none()
            }
            Action::Copy => self.copy_selection(),
            Action::Paste => self.paste_clipboard(),
            Action::Find => self.open_search(),
            Action::ToggleSidebar => self.toggle_sidebar(),
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

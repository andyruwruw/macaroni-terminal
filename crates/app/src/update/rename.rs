//! Renaming tabs, workspaces and groups in place, and the item focus that Enter renames.
//!
//! `App.renaming` holds the item and the text typed so far; the view draws a text field
//! ([`view::RENAME_INPUT`]) in the item's place. `App.item_focus` is the item last clicked:
//! a plain Enter renames it, caught by the terminal view (`ViewEvent::RenameItem`) or, with a
//! browser tab focused, by `App::unhandled_key`.

use iced::Task;
use macaroni_core::Command;

use crate::message::Message;
use crate::state::Renaming;
use crate::{App, view};

impl App {
    /// Puts a rename field in place of the item, holding its current name, and focuses it with
    /// the name selected. Does nothing if the item is gone.
    pub(crate) fn start_rename(&mut self, target: Renaming) -> Task<Message> {
        let name = match target {
            Renaming::Workspace(id) => self.model.workspace(id).map(|w| w.name.clone()),
            Renaming::Tab(id) => self.model.tab(id).map(view::tab_title),
            Renaming::Group(id) => self.model.group(id).map(|g| g.name.clone()),
        };
        let Some(name) = name else {
            return Task::none();
        };
        self.item_focus = None;
        self.renaming = Some((target, name));
        start_editing_name()
    }

    /// Renames the item last clicked, if there is one.
    pub(crate) fn rename_focused_item(&mut self) -> Task<Message> {
        match self.item_focus {
            Some(target) => self.start_rename(target),
            None => Task::none(),
        }
    }

    /// Typing in the rename field.
    pub(crate) fn rename_input(&mut self, value: String) -> Task<Message> {
        if let Some((_, current)) = &mut self.renaming {
            *current = value;
        }
        Task::none()
    }

    /// Enter in the rename field: give the item its new name.
    pub(crate) fn rename_commit(&mut self) -> Task<Message> {
        match self.renaming.take() {
            Some((Renaming::Workspace(id), name)) => self.apply(Command::RenameWorkspace(id, name)),
            Some((Renaming::Tab(id), name)) => self.apply(Command::RenameTab(id, Some(name))),
            Some((Renaming::Group(id), name)) => self.apply(Command::RenameGroup(id, name)),
            None => Task::none(),
        }
    }

    /// A press outside the rename field drops the edit.
    pub(crate) fn rename_cancel(&mut self, target: Renaming) -> Task<Message> {
        if self.renaming.as_ref().is_some_and(|(t, _)| *t == target) {
            self.renaming = None;
        }
        Task::none()
    }

    /// A press outside the focused item lets go of it.
    pub(crate) fn item_blur(&mut self, target: Renaming) -> Task<Message> {
        if self.item_focus == Some(target) {
            self.item_focus = None;
        }
        Task::none()
    }
}

/// Focuses the rename field with its text selected, so typing replaces the old name.
fn start_editing_name<T: Send + 'static>() -> Task<T> {
    iced::widget::operation::focus(view::RENAME_INPUT)
        .chain(iced::widget::operation::select_all(view::RENAME_INPUT))
}

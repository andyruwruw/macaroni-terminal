//! The pure app model: workspaces, their split layouts, panes and tabs, as plain data.
//!
//! [`Model`] holds the state and [`Model::apply`] is the only way to change it: every input
//! (keybindings, menus, the sidebar, the control socket) becomes a [`Command`], and `apply`
//! returns the [`Effect`]s (spawn or kill a shell session, open or close a browser, quit) for the
//! app to perform. Nothing here does I/O or depends on iced, the PTY or the config, so the whole
//! state machine is unit tested and fuzzed with random commands (`tests.rs`).
//!
//! - `ids` defines the typed ids, all drawn from one counter that never reuses a value.
//! - `layout` is the split tree ([`SplitNode`]) and its geometry: pane rectangles, dividers
//!   and directional neighbours.
//! - `model` is [`Model`] itself, the [`Command`] and [`Effect`] enums, the saved-layout specs
//!   and the invariant check ([`Model::check`]).
//!
//! The app crate (`macaroni`) owns the session and browser registries keyed by these ids and
//! renders what the model describes.

mod ids;
mod layout;
mod model;

pub use ids::{BrowserId, GroupId, PaneId, SessionId, TabId, WorkspaceId};
pub use layout::{
    Axis, Direction, Divider, MIN_RATIO, Rect, Side, SplitNode, neighbour, split_rect,
};
pub use model::{
    Agent, AgentState, Command, Effect, Group, LayoutSpec, Model, MoveTo, Notice, Pane, Shift, Tab,
    TabKind, TabSpec, Workspace, WorkspaceSpec,
};

#[cfg(test)]
mod tests;

//! The update loop's handlers, one file per area of the app; `App::handle` dispatches to them.
//!
//! Each file is an `impl App` block. Handlers take what their message carries and return a
//! `Task` (usually none). Anything that changes the model goes through
//! [`App::apply`](crate::App::apply) in [`effects`], which also runs the model's effects, tells
//! control API subscribers and saves the session; handlers never call `Model::apply` directly.

mod actions;

pub(crate) use actions::same_dir;

mod browser;
mod config;
mod dividers;
mod effects;
mod items;
mod menus;
mod rename;
mod search;
mod shortcuts;
mod sidebar;
mod terminals;
mod window;

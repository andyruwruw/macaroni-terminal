//! Typed ids for everything the model addresses.
//!
//! Every id is a newtype over a `u64` drawn from the model's single counter (`Model::next_id`),
//! so ids of different kinds never share a value and none is ever reused. That keeps
//! `workspace/pane/tab` addresses handed out over the control socket valid, and lets the app key
//! its session and browser registries by [`SessionId`] and [`BrowserId`] without collisions.

/// Defines a copyable, ordered id newtype over `u64`, with the given doc attributes.
macro_rules! id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(
            /// The raw value from the model's id counter; unique across all id kinds.
            pub u64,
        );
    };
}

id!(
    /// A workspace: one entry in the sidebar with its own split layout.
    WorkspaceId
);
id!(
    /// A pane: a leaf of a workspace's split tree, holding one or more tabs.
    PaneId
);
id!(
    /// A tab: one terminal or browser inside a pane.
    TabId
);
id!(
    /// A terminal's shell session. The app owns the `Session` itself; the model only refers to
    /// it through a [`TabKind::Terminal`](crate::TabKind::Terminal) tab.
    SessionId
);
id!(
    /// A browser page. The app owns the browser itself; the model only refers to it through a
    /// [`TabKind::Browser`](crate::TabKind::Browser) tab.
    BrowserId
);
id!(
    /// A sidebar group of workspaces.
    GroupId
);

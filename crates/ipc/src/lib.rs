//! macaroni's control protocol: newline-delimited JSON over a per-user local socket (a named
//! pipe on Windows, a Unix socket elsewhere). Shared by the app (server) and `macaroni-cli`.
//!
//! `protocol` holds the message types ([`Request`] with its [`Call`], [`Response`], [`Event`] and
//! the result structs); `transport` finds the endpoint, listens and connects, and frames values
//! as lines. Neither knows anything about the app's model: ids are plain `u64`s.

mod protocol;
mod transport;

pub use protocol::{
    AgentInfo, AgentState, BrowseAction, Call, Direction, Event, Page, PaneInfo, Request, Response,
    Screen, TabInfo, TabKind, Tree, WorkspaceInfo,
};
pub use transport::{
    Listener, SOCKET_ENV, Stream, TAB_ENV, accept, connect, endpoint, listen, read_line, write_line,
};

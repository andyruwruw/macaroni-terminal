# Control API (socket + `macaroni-cli`)

## Mental model

The app listens on a per-user local socket for newline-delimited JSON requests; `macaroni-cli` is a thin client. Every request becomes the same `Command`s the keyboard uses, applied on the UI thread.

```
client ── JSON line ──► socket ──► ipc-read thread ──► Message::Ipc ──► App::call (ipc.rs)
       ◄── JSON line ◄── ipc-write thread ◄── response / events ◄──────┘
```

## Key files

- `crates/ipc/src/protocol.rs`: `Request`, `Response`, `Call` (one variant per method), `Event`, `Tree`, `Screen`, `Page`.
- `crates/ipc/src/transport.rs`: `endpoint`, `listen`, `connect`, access control.
- `crates/app/src/ipc.rs`: the server. `App::call` dispatches to one helper per method; `ipc_message` answers; `broadcast` sends events to subscribers.
- `crates/app/src/update/effects.rs`: emits workspace and tab events (`IpcState.session_tabs` / `browser_tabs` remember which tab a closed session or browser was).
- `crates/cli/src/main.rs`: the CLI.

## Endpoint and access

- `MACARONI_SOCKET` if set, else `macaroni-<user>`: a named pipe on Windows, a `.sock` in the runtime or temp dir elsewhere.
- Current user only: a protected owner-only DACL on Windows, mode `0600` on Unix.
- `listen` refuses if another instance answers on the endpoint and replaces a stale socket file. If it fails, the app runs without the API and shows why in the banner.
- Every terminal gets `MACARONI_SOCKET` and `MACARONI_TAB_ID`. Methods take an optional `tab` defaulting to the focused tab; the CLI fills `--tab` from `MACARONI_TAB_ID`, so a command run in a tab acts on that tab.

## Protocol

Request `{"id": 1, "method": "<name>", "params": {...}}`; response `{"id": 1, "result": ...}` or `{"id": 1, "error": "..."}`. Unit methods may omit `params`. Unknown methods fail deserialization. `subscribe` turns the connection into an event stream. Read `Call` and `Event` for the method and event list.

## Invariants and traps

- `split`, `new_tab` and `focus` go through `SelectTab`, so they move the user's focus.
- Commands go through `run`, which keeps the iced `Task` the reducer returns in `IpcState.tasks`; `ipc_message` answers first, then returns them. This is what lets closing the last workspace over the socket quit the app.
- `send_text` and `read_screen` need a running terminal. Terminals start lazily when first shown, so a tab in a never-displayed workspace returns an error.
- `notify` takes the same path as an OSC notification (`App::attention`), including muting and the "user is looking at it" check. Agent gating is in [`claude-code.md`](claude-code.md).
- Browser methods given a terminal tab act on that workspace's browser. `read_page` is answered asynchronously (handled in `ipc_message`, not `call`) once Chromium delivers the text; `browse` and `read_page` need the browser started.

## Adding a method

1. Add a `Call` variant (and any result type) in `protocol.rs`, with a round-trip test.
2. Handle it in `App::call` with a helper that resolves ids and goes through `run`.
3. Add the CLI subcommand in `crates/cli/src/main.rs`.
4. If it changes visible state, `broadcast` an `Event`.

## Testing

`crates/ipc` tests protocol round-trips and a real request/response over a socket, including that a second listener can't take the name. End to end, run the app on a scratch endpoint and drive it with the CLI:

```sh
MACARONI_SOCKET=macaroni-e2e cargo run -p macaroni &
MACARONI_SOCKET=macaroni-e2e cargo run -p macaroni-cli -- events &
MACARONI_SOCKET=macaroni-e2e cargo run -p macaroni-cli -- split right
```

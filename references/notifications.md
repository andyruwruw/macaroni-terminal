# Notifications and attention

## Mental model

A program asks for attention with an escape sequence or `macaroni-cli notify`. The tab stays marked until the user interacts with it; an OS notification fires when the user can't see it.

```
PTY output ──► reader thread: OscSniffer::feed (before the emulator) ──► SessionEvent::Notify
  ──► App::attention (update/terminals.rs)
        muted, or window focused and it's the focused tab? → drop
        OS notification if [notifications] os and (window unfocused or tab not visible)
        broadcast Event::Notification, then Command::Notify ──► Tab.attention = Some(Notice)
typing into or clicking the tab ──► Command::ClearAttention
```

## Detection (`crates/term/src/osc.rs`)

`alacritty_terminal` ignores notification sequences, so `OscSniffer` runs every chunk through its own stateful `vte::Parser` (sequences split across reads still match).

- OSC 9 (message), skipping ConEmu's numeric subcommands (`9;4;…` etc.).
- OSC 777 `notify;title;body`.
- OSC 99 (kitty), basic: `i=`, `d=0` chunking, `p=title|body`.
- BEL only when `[notifications] bell` is on. `bell_sound` is separate (`crates/app/src/bell.rs`).

Text is capped and stripped of control characters; a lone title becomes the body; `;` inside a message is rejoined. Verify passthrough with `cargo run -p macaroni-term --example osc_probe` (the bundled ConPTY passes all three).

## State (`crates/core`)

- `Tab.attention: Option<Notice>`. `Notice.seq` comes from the model's id counter, so the newest in a workspace is `max_by_key(seq)`.
- Queries: `pane_attention`, `workspace_notice`, `tab_visible`, `is_muted`.
- Muted tabs or workspaces drop notices in both `App::attention` and the reducer.

## UI

Attention color is `theme::Chrome.attention`. The pane ring (`view/panes.rs`) wins over the focus outline and sits inside the card's padding so it never resizes the terminal. The tab icon (`view/tabs.rs`) and workspace icon (`view/sidebar.rs`) become a bell; the sidebar detail shows the newest notice.

## Invariants and traps

- Clear only on interaction with the tab (`ViewEvent::Write`, including IME commits and paste, or `ViewEvent::Clicked`). Never on tab or workspace selection: a glance must not dismiss it.
- `App.window_focused` starts `false` and follows window focus events, so a window opened in the background still notifies.
- OS notifications (`crates/app/src/notifications.rs`, `notify-rust`) run on a background thread because delivery can block. On Windows they use PowerShell's AppUserModelID, since a custom identity needs a registered Start-menu shortcut. On macOS they need the bundled `.app` (`cargo xtask bundle`). Clicking one doesn't focus the tab.

## Testing

Unit tests: `crates/core/src/tests.rs`, `osc.rs`. End to end, print a sequence (e.g. `printf '\e]777;notify;T;B\a'`) in an unfocused tab, or use `macaroni-cli notify` (see [`control-api.md`](control-api.md)).

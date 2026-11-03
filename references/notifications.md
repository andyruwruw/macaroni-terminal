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
- A command the shell marked (OSC 133, see [`terminal-and-pty.md`](terminal-and-pty.md)) that ran at least `[notifications] long_command` seconds (default 30, 0 off): `SessionEvent::CommandFinished` becomes an attention request titled "Command finished", with the exit code and run time.
- BEL only when `[notifications] bell` is on. `bell_sound` is separate (`crates/app/src/bell.rs`).

Text is capped and stripped of control characters; a lone title becomes the body; `;` inside a message is rejoined. Verify passthrough with `cargo run -p macaroni-term --example osc_probe` (the bundled ConPTY passes all three).

## State (`crates/core`)

- `Tab.attention: Option<Notice>`. `Notice.seq` comes from the model's id counter, so the newest in a workspace is `max_by_key(seq)`.
- Queries: `pane_attention`, `workspace_notice`, `tab_visible`, `is_muted`.
- Muted tabs or workspaces drop notices in both `App::attention` and the reducer.
- Triggers (`[[trigger]] notify`, see [`config.md`](config.md)) raise notices too: `App::triggered` calls `App::attention` with the trigger's `name` (else the tab's title) and the matching line, at most once per `NOTIFY_COOLDOWN` per trigger and terminal. A trigger's `badge` goes to `Tab.badge`, which clears with the notice; neither is set on the tab being looked at.

## UI

Attention color is `theme::Chrome.attention`. The pane ring (`view/panes.rs`) wins over the focus outline and sits inside the card's padding so it never resizes the terminal. The tab icon (`view/tabs.rs`) and workspace icon (`view/sidebar.rs`) become a bell; the sidebar detail shows the newest notice.

## Invariants and traps

- Clear only on interaction with the tab (`ViewEvent::Write`, including IME commits and paste, or `ViewEvent::Clicked`). Never on tab or workspace selection: a glance must not dismiss it.
- `App.window_focused` starts `false` and follows window focus events, so a window opened in the background still notifies.
- OS notifications (`crates/app/src/notifications.rs`) run on a background thread because delivery can block. Windows: a `tauri-winrt-notification` toast under `APP_ID` (`dev.macaroni.terminal`, also the macOS bundle id). An unpackaged app needs no shortcut for its own identity: `notifications::register` (first thing in `main`) writes `DisplayName` and `IconUri` (the PNG copied to `%LOCALAPPDATA%\macaroni`) under `HKCU\Software\Classes\AppUserModelId\<id>`. macOS: `mac-notification-sys`, needing the bundled `.app` (`cargo xtask bundle`). Elsewhere `notify-rust`.
- Clicking a notification shows its tab: Windows' `on_activated` and macOS's `wait_for_click` call `clicked`, which sends through the `CLICKS` channel (installed by the `notifications::clicks` subscription) as `Message::NotificationClicked`; `App::notification_clicked` selects the tab and restores and focuses the window. On Windows only a click on the toast while it's up counts: one left in the notification center isn't wired to the running app. Check toast delivery with `ToastNotificationManager::History.GetHistory("dev.macaroni.terminal")`; Do Not Disturb hides the banner but still records it.

## Answering permission prompts

`App::asks_permission(tab)`: the tab's agent needs input and its detail mentions permission (Claude Code's Notification hook: "Claude needs your permission to use Bash"), and the tab isn't in `App.answered`. Then `notifications::show` gets `ask`: Allow and Deny toast buttons on Windows (`add_button`, argument = label), an "Answer" dropdown on macOS (`MainButton::DropdownActions`, reported as `ActionButton`). They come back as `Message::PermissionAnswer`; the agent dashboard's card has the same two buttons. `App::answer_permission` types `1` (Allow) or Esc (Deny) only if the tab still asks and one of the last `PERMISSION_ROWS` non-blank screen rows contains "Do you want" (the dialog); otherwise it just shows the tab. It then puts the tab in `App.answered`, which the agent's next hook (`ipc.rs` `agent`) clears, so a second press can't type again even while the old dialog text is still on screen.

## Inbox

`Model::waiting_tabs` (every tab with a notice, oldest `seq` first) feeds the palette's attention inbox (`show_waiting`) and `Command::NextWaiting` (`next_waiting`), which selects the waiting tab after the focused one, wrapping. Neither clears a notice: selection never does.

## Testing

Unit tests: `crates/core/src/tests.rs`, `osc.rs`. End to end, print a sequence (e.g. `printf '\e]777;notify;T;B\a'`) in an unfocused tab, or use `macaroni-cli notify` (see [`control-api.md`](control-api.md)).

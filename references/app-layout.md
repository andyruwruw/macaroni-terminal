# App shell: view, layout, input and settings

## Mental model

- `main.rs`: `App` (the `Model`, `running` and `pending` sessions, config, `chrome`, transient UI state, `wins`), `App::update`, the flat dispatcher `App::handle`, `App::subscription`. It's an iced `daemon`, so it can have any number of windows; it only exits through `App::exit`.
- `message.rs`: `Message`. `state.rs`: transient UI types (`Drag`, `ItemDrag`, `Dragged`, `DropTarget`, `Menu`, `Submenu`, `MenuTarget`, `MenuPage`, `Renaming`, `SidebarHover`).
- `update/`: one handler file per area (`actions`, `browser`, `config`, `dividers`, `effects`, `items`, `menus`, `rename`, `search`, `shortcuts`, `sidebar`, `terminals`, `window`).
- `view.rs` + `view/`: `title_bar`, `sidebar`, `tabs`, `panes`, `menus`, `drag`, `widgets`, `icon_picker`, `folder_dialog`, `settings/`.
- `theme.rs` (`Palette`, `Chrome`), `icons.rs`, `glass.rs`, `backdrop.rs` + `backdrop.wgsl`, `anim.rs`, `keymap.rs`, and small custom widgets: `pointer.rs`, `right_click.rs`, `gap_line.rs`, `side_line.rs`, `hair_line.rs`, `drag_area.rs`, `inert.rs`.

```
Message ──► App::update ──► App::handle ──► update/<area>.rs
                         └─► always after: sync_browsers(), sync_fades()
App::apply(Command) (update/effects.rs) ──► Model::apply ──► Effects
  SpawnSession(id) ──► pending[id] ──► first ViewEvent::Resize ──► starting[id] ──► Session::spawn (off thread) ──► SessionStarted ──► running[id]
TerminalView ──► ViewEvent ──► Message::View(id, ev) ──► App::view_event
```

- Every model change goes through `App::apply`, which also broadcasts workspace changes to IPC, scrolls strips whose open tab changed (`reveal_tab`) and saves the session (throttled; `save_ticks` flushes the last change). Handlers must return its task.
- Sessions start lazily at their real grid size, never a placeholder. `Session::spawn` runs off the UI thread (`App::spawn`, `vscode::off_thread`), since creating a pseudoconsole and process takes tens of milliseconds; `App::session_started` types the startup command, agent resume and anything typed meanwhile (`App.typeahead`, kept for `pending` and `starting` terminals) and moves it to `running`. A tab closed while its shell starts leaves no `starting` entry, so the arriving session is dropped, which kills it.
- Hidden tabs' terminals start only while fewer than `MAX_BOOTING` are on their way to a first prompt (`App.booting`, cleared by the first OSC 7, an exit, or `BOOT_TIMEOUT`); see `App::start_hidden`. Many Git Bashes starting together slowed each from about a second to several, the shown ones included. A hidden tab shown before its turn starts at once.
- View modules import shared enums (`Dragged`, `MenuPage`, `ColorTarget`, ...) through the crate root; those `use` lines in `main.rs` must stay even though `main.rs` doesn't use them.

## Windows (`update/window.rs`)

- `App.wins` maps each iced `window::Id` to a `Win`: the model's `WindowId` it shows, its size, position, maximized flag, native handle (the parent of its pages) and scale. A window is added when it's asked to open (`open_window`), before it has.
- **"The window" depends on where you are.** `App::view(id)` sets `App.viewing`, and `App::win`, `window_size`, `scale`, `maximized`, `shown` (the window's `WindowView`) and `shown_workspace` answer for the window being drawn; `update` clears `viewing`, so in handlers they answer for the focused window. View code reads `shown()` / `shown_workspace()`, never `model.workspaces()` / `active_workspace()`, or every window would draw the focused one's sidebar. Lists that span windows (palette, dashboard, scrollback search, IPC `list`) use `model.all_workspaces()`.
- Menus, dialogs, drags and the palette only draw in the focused window (`drawing_focused`); OS focus moving to another window dismisses them and applies `Command::FocusWindow`. Drag moves carry their window id and only the focused window's count.
- The model's `OpenWindow`, `CloseWindow` and `FocusWindow` effects open, close and raise OS windows (`run_effects`). A window's close button closes it and everything in it (`Command::CloseWindow`), except the last, which saves the session and quits. Startup opens one window per model window where the session left it (the focused one last, so it's in front).
- Session: `Saved.windows` (`SavedWindow`: active workspace index, position, size, maximized) and `focused_window`; workspaces carry their window index; a file from before windows is one window. `session::capture` fills the active workspaces and `App::saved_windows` the rest.
- Dragging tabs and workspaces between windows: see Dragging.
- Shortcuts: `new_window`, `close_window`, `move_to_new_window`; the workspace menu's Move Workspace has To New Window and To Window N, and the tab menu's Move has New window and other windows' workspaces (`window_label`).

## How `App::view` is built (`view.rs`)

One `stack`, in covering order: `window()`, `resize_edges` (custom frame, not maximized), `drop_marker`, `drag_ghost`, `menu_layers`, icon picker, folder dialog, worktree dialog, environment dialog, agent dashboard, command palette, settings dialog. Pane letters are drawn by `App::pane` itself, over each pane's card. Overlays are `pin`ned layers; modals put a full-window click catcher under themselves.

`App::window`: under glass `backdrop::window` is the bottom layer and the window container stays clear. Then a column of the title bar (`view/title_bar.rs`, only with `CUSTOM_FRAME`, i.e. Windows) and a row of the sidebar (`view/sidebar.rs`, omitted at width 0) and the main area, with `sidebar_edge` stacked over the row. The main area is the Git Bash offer plus `workspace_area`: `responsive(|size| self.workspace(size))` padded by `style.panes.margin` right and bottom, with the config error banner floating over it. The title bar gap is padding inside the main area, so a colored sidebar runs up to the title bar.

`App::workspace` (`view/panes.rs`): `ws.shown_panes(pane_area(c, size))` gives each pane's rect (just the maximized pane while one is); each pane is a `pin` in one `stack`, plus divider handles from `ws.shown_dividers(area)`. `App::pane` is the tab strip (`view/tabs.rs`) over a card holding a `TerminalView` or `browser_body`.

## Layout math other code depends on

- **`workspace_origin(c)`**: the pane area's top-left in window coordinates. Divider dragging and drop targeting subtract it from cursor positions. Banners aren't counted.
- **`pane_area(c, size)`**: half a pane gap bigger on every side (minus `pane_overlap`); each pane is pinned `gap / 2` inside its share, so gaps show only between panes and the outer panes sit on the area's edges.
- **`drop_target` has no `responsive` size**: it rebuilds the area as `App::window_size() - workspace_origin - panes.margin`. Keep it in sync with `workspace_area`'s padding.
- **Pane vertical layout**: the card starts `tabs.height + tabs.gap` below the pane top. The strip is `strip_scrollbar_room(c)` taller than the tabs (the scrollbar sits under them), so the column spacing is `tabs.gap - strip_scrollbar_room`. Drop zones and browser placement rely on this.
- **Shared borders (solid only)**: meeting borders become one line.
  - `pane_overlap`: with `panes.gap` 0, each pane is pinned a border wider and taller, over its right and lower neighbours.
  - `tabs_flush`: with `tabs.gap` 0 and a pane border, each tab is a border taller so its bottom border lands on the card's, and `App::pane` stacks the strip over the card (a `tabs.height` spacer above it) instead of a column. `tab_spacing` goes negative by a border with `tabs.spacing` 0.
  - `sidebar_edge`: the sidebar's last pixels, or over the panes' left border when `panes_gap` is 0.
  - `with_title_line` / `TitleLine`: the title bar's own last pixels with a gap, else over the sidebar's first pixels and under the pane area, so tab top borders lie on it.
- **Browser bounds**: a page is a native child window (WebView2, WKWebView or CEF). `BrowserHost` (in `browser_body`) reports its bounds as `Message::BrowserBounds`; `App::sync_browsers` (`update/browser.rs`) moves or hides each page after every message. Nothing iced draws can cover a page, so pages hide under menus (`menu_bounds`), modals and tab drags. A new overlay must be added to `place_browsers`' `covered` check.
- **Padding and borders never depend on focus**: a size change resizes the PTY and the shell reflows. Rings are borders, which iced draws over padding without taking room.

## Chrome: colors and sizes

- `Chrome::new(palette, config)` (`theme.rs`): `Chrome::base(ui)`, then each `macaroni_config::COLORS` name painted through `Chrome::color_mut` from its `[colors.<ui>]` override, else its linked theme color (`ColorSetting.link` via `ThemeColors`). `BORDER_FOLLOWERS` follow `border`. `Chrome.s` is the whole `Style`, read as `c.s.tabs.height`. `Chrome` is `Copy`; view functions take `c: Chrome`.
- `update/config.rs` rebuilds `Chrome` on every load; `Palette::with_pane_background` then makes an opaque solid `card` the terminal background.
- Defaults: colors in `theme.rs` (`Chrome::solid`/`Chrome::glass`), numbers in `crates/config/src/style.rs`. Don't hard-code sizes in views; only behavior thresholds are named constants.
- **Add a color**: a row in `colors!` in `config/src/style.rs`, a `Chrome` field with defaults, an arm in `Chrome::color_mut` (a test checks), then read it in the view. The settings dialog lists it automatically.
- **Add a `[style.*]` number**: a field in its table's `numbers!` block. The dialog lists it automatically; the view must read `c.s.<table>.<field>`.

### Glass

- `glass::frosted` (sidebar items, tabs) and `glass::panel` (pane cards) draw the backdrop shader blurred behind a sheet; `glass::sheet` (popups, settings) has no blur, since iced can't read back other UI.
- Trap: a shader primitive draws after every quad in its layer, so it must sit in its own layer underneath (`stack` base, or a sheet's `push_under`).
- Trap: each primitive takes a uniform slot (`SLOTS`, freed in `Pipeline::trim`); past that, sheets draw without backdrop.
- Trap: don't draw sheets with canvas meshes; iced's wgpu renderer misplaces them when several canvases are on screen.
- Selected workspace items lift by layout, not transform: `glass::lift_padding` splits padding into outer and inner parts that sum to the same slot. `float` would draw over menus and escape the sidebar's clip.
- The window can't be transparent: browser pages force wgpu's GL backend on Windows, whose surfaces are opaque. Terminals skip their default background in glass (`TerminalView::transparent`).

## Sidebar (`view/sidebar.rs`, `update/sidebar.rs`)

- Width is dragged by `HANDLE_WIDTH` at the right edge, between `compact_width` and the window minus `MIN_PANE_ROOM`, folding below `COLLAPSE_BELOW`. `App::sync_sidebar` writes the width in effect into `Chrome.s.window.sidebar_width`, which everything above reads. `App.sidebar` is saved in `session.json`.
- `SidebarSize` (`App::sidebar_size`): Full, Narrow, Tiny. All rows share `workspace_interactions`.
- Rows come from `sidebar_list` (also used by settings previews); item lines are templates filled by `label::fill` from `WorkspaceValues::get`. `{branch}` / `{dirty}` / `{sync}` come from `App.git` (`git.rs`): while a template uses them, `git_ticks` fires every `GIT_TICK` and `App::git_tick` runs `git::look` off the UI thread over each workspace's `workspace_dir` (focused tab's cwd, else its folder, else `default_dir`). The branch is read from `HEAD` (worktree `.git` files followed); dirty and the ahead/behind counts (`Repo::sync`, `↑2↓1`) come from one `git status --porcelain=v2 --branch --untracked-files=no` (`parse_status`). Groups: `group_block` with a `SideLine` at `group_line_x`.
- The list scrolls (`SIDEBAR_SCROLL`) with padding inside the scrollable, so lifted items and the top insertion line aren't clipped.
- Search focus trap: `text_input` reports no focus change, so `presses` sends `SidebarSearchCheck` on every press and `operation::is_focused` answers `SidebarSearchFocused`. Ending a search uses `focusable::unfocus`.

## Tab strip (`view/tabs.rs`)

- `fit_tabs` estimates natural widths; when they don't fit, tabs share the width equally down to `style.tabs.min_width`, then scroll in a `scrollable` with id `tab_strip_id(pane)`. `+` stays outside it. A wrapping `mouse_area` turns unused wheel events into `Message::TabScroll` (`operation::scroll_by`).
- New tab buttons (`App::new_tab_buttons`, `[tab_items] new_buttons`) sit at the strip's right end: terminal, browser, Git, and VS Code (a web VS Code tab once downloaded, else the desktop app when `code` is found at launch). A `Fill` spacer pushes them (and the find bar) right; `fit_tabs` subtracts `new_buttons_width` from the tabs' room.
- The find bar (`App.search`, a `SearchTarget` of a terminal or a page; `update/search.rs`) shows in the strip of the pane whose active tab is the terminal it searches (`search_shown`) and takes the new tab buttons' place there; a page's search is a card in the browser pane instead (see [`browser.md`](browser.md)). `fit_tabs` subtracts its width instead, and its field narrows from `SEARCH_WIDTH` to `MIN_SEARCH_WIDTH` in a narrow pane (`search_field_width`) so the buttons never fall off the strip. Enter steps up the scrollback; in the page card it steps down.
- Hovering a tab shows `App::tab_tip` after `TIP_DELAY`: its icon and whole title, then its agent's `agent_line` and status-line summary (`style.tabs.status_size`). Left off while anything is dragged.
- A tab's badge is `Tab.badge` (set by a trigger, in `Chrome.attention`) when there is one, else `[tab_items] badge` (`tab_badge`).
- Revealing the open tab: `on_scroll` reports `Message::TabViewport` into `App.tab_views` (only while overflowing). `App::reveal_tab` scrolls via `tab_span` and retries on the next report (`App.tab_reveals`), because a new tab widens the strip only on the next layout.

## Focus routing

- A `TerminalView` takes keys and IME only when its pane is focused and nothing else wants typing: the condition in `App::pane` (`renaming`, `search`, `menu`, `settings`, `icon_picker`, `folder_dialog`, `worktree_dialog`, `env_dialog`, `dashboard`, `command_palette`, `pane_letters`, `sidebar_search_focused`). A new text-entry surface must be added there and to `dismiss` in `App::subscription` (Escape is only listened for while something is dismissable).
- Clicking an unfocused view emits `Clicked` (becomes `SelectTab`).
- Shortcuts: `TerminalView` asks `Keymap::lookup` before encoding a key and emits `ViewEvent::Action`. With a browser tab focused, `unhandled_keys` (only `Ignored` events) and the page (`App.shared_keymap`) look up instead. All paths end in `App::perform` (`update/shortcuts.rs`).
- A focused page is another native window, so `clicks` runs while `page_focus` is set and `App::reclaim_focus` takes the keyboard back.
- `App.key_focus` (F6, see [`accessibility.md`](accessibility.md)) takes the keyboard from the terminal for the sidebar's and strips' focus ring; it's in the terminal's condition too, and any mouse press drops it.

## Renaming (`update/rename.rs`)

`App.renaming` covers workspaces, tabs and groups, started by `App::start_rename`. Clicking an item sets `App.item_focus` while the terminal keeps the keyboard, so Enter is caught by `TerminalView::enter_renames` (`ViewEvent::RenameItem`) or `Message::Key` for a browser pane. Typing, a shortcut or a press outside (`Pointer::on_press_outside_maybe`, `ItemBlur`) drops it. The field is `RENAME_INPUT`.

## Dragging

- **Dividers** (`update/dividers.rs`): handles fill the gap at each divider, at least `MIN_HANDLE` wide (solid has no gap, so a gap-wide handle couldn't be grabbed; the wider one reaches over the panes' edges). `drag_events` runs only while `App.drag` is set and turns moves into `SetSplitRatio` (cursor minus `workspace_origin`, relative to `divider.area`; the model clamps).
- **Tabs and workspaces** (`update/items.rs`, `view/drag.rs`): `TabPress`/`WorkspacePress` are `mouse_area`s (a press both selects and arms `App.item_drag`); the drag starts after `TAB_DRAG_THRESHOLD`. `drop_target` picks a strip slot (`hovered_tab` + `hover_zone`), a pane body (outer `SPLIT_ZONE` splits), a workspace edge (`edge_drop`), or a sidebar entry (`sidebar_hover`, opening the workspace after `WORKSPACE_HOVER_OPEN`). Workspaces use `workspace_drop`/`workspace_line`, tried on a cloned model so no-ops show nothing.
- **Between windows.** The OS keeps sending the cursor to the window a drag started in (the focused one), in its coordinates, wherever it goes. `App::over` turns each move into an `Over` on the drag: `Here`, `Window(id, point in that window)` from the windows' screen positions (`Win.position`, asked for when a window opens and kept by `Moved`), or `Outside(screen point)`. Over another window, `drop_target` borrows `App.viewing` to work out the target from that window's geometry (`pane_drop`); nothing there reports hover, so its sidebar means its active workspace and its strips their end. `DropTarget::Edge` carries the workspace so the split lands in that window. That window draws the marker (`drop_marker` only draws in the window the cursor is over), and every window's pages step aside while dragging. Outside every window a tab drops as `MoveTo::NewWindow` and a workspace as `MoveWorkspaceToWindow { window: None }`, with `App.next_window_at` placing the new window under the cursor (`DROP_GRAB`); a workspace over another window moves into its sidebar. Window positions are logical, so monitors with different scales line up only roughly.
- `Pointer::on_zone` reports `Before`/`After` and `GapBefore`/`GapAfter`. Zone messages carry their item because one move can publish the old exit and new entry in either order; during a drag exits are ignored so gaps keep a target.
- Insertion lines are `GapLine`s, which paint outside their own bounds. Trap: don't use `pin` at a negative offset instead; `Pin` passes its bounds as the viewport and children outside are culled.

## Menus and modals (`view/menus.rs`, `update/menus.rs`)

- `RightClick` reports the cursor's window position (`mouse_area` doesn't) and opens `App.menu` with a `MenuTarget`. `MenuTarget::Link` is opened by a terminal instead (Ctrl+right-click on a link, `ViewEvent::LinkMenu`); what was clicked is in `App.clicked`.
- Keyboard: `Menu.cursor` highlights an item; `menu_items` (what the keyboard can reach) and `menu_labels` (for screen readers) read `menu_entries` too. See [`accessibility.md`](accessibility.md).
- `menu_entries(target, page)` is the single source of entries; `MenuEntry::height` sizes them before layout so `menu_bounds` can place the main list and submenu (`Submenu.pinned`) inside the window. Separators are `HairLine`s (whole device pixels; a 1-unit container blurs at fractional scales). Shared builders: `color_entries`, `pin_entry`, `mute_entry`, `move_entries`, `icon_grid`.

## Animations (`anim.rs`)

`App::sync_fades` sets selected/hovered targets after each message; the view reads `Fades::get` and blends with `widgets::item_ease`. `window::frames` runs only while `Fades::animating`. iced's svg tint ignores alpha, so dimmed icons use svg opacity.

## Settings dialog (`view/settings/`)

- `mod.rs` (`Section`, `Settings`, `SettingsMessage`, `Control`, `Setting`, `settings_update`), `list.rs` (`settings_list`), `dialog.rs` (drawing), `preview.rs`, `color_picker.rs`, `sound_menu.rs` (the sound settings' popup, where each sound can be played before it's picked).
- Rows are rebuilt from `App.config` on every draw. A card is a run of adjacent rows with the same (section, group).
- **Pages**: General, Terminal, Notifications, Keyboard Shortcuts, Accessibility, and under the Integrations heading Agents, Browser and VS Code, are written out in `list.rs` (VS Code only where `vscode::available`). `Section::heading` starts each heading's run. The Appearance pages (Theme, Window, Sidebar, Tabs, Panes, Menus & Controls, Effects) come from the `LOOK` table: per page, cards of keys, each a `COLORS` / `THEME_COLORS` name, a `[style.*]` key, or an `item_rows` key (themes, `[workspace_items]`, `[tab_items]`). Each part's colors sit beside its sizes. Rows the interface doesn't draw are dropped (group "Solid theme" colors in glass, glass-only colors and `GLASS_NUMBERS` in solid).
- **Add a setting**: `Setting::new(section, group, label, description, Control::...)` in the page's builder in `list.rs`. A new color or `[style.*]` number needs a place in `LOOK`: `every_color_and_number_has_one_card` fails until it has exactly one. Non-style numbers are `NumberSetting`s in `list.rs` (`number_setting`).
- **The file is the source of truth**: each control writes one key with `macaroni_config::write_setting` (`toml_edit`, keeps comments), and the watcher reloads it. `None` removes the key; `SetMany` writes several.
- Color drags preview without writing (`App::preview_color` edits `App.config.colors` in memory and rebuilds `Chrome`); the release writes.
- Search (`Setting::matches`) looks at the section, card, label, description and config key, and skips `Setting.repeat` rows. A result card's title sends `Reveal`, which opens its page and sends `Jump`. Quick Access scrolls with `CardOffset`, a custom `Operation`.
- **Previews**: one per page (`preview_for(section)`), pinned between the title and the scrolling cards so it stays in view, and outside the scrollable so it never moves a card. Under `PIN_MIN_HEIGHT` it's the first item in the scroll instead. Previews reuse real view code inside `inert::Inert`, which forwards no events or operations, so preview widgets never answer for real ones by id. `outlined` stacks a border layer because a container's border is covered by its content.
- **Keyboard**: `keys.rs` moves a ring over the rows (`Settings.row`) and changes the ringed one; text fields carry `field_id(key)` so Enter can focus them. See [`accessibility.md`](accessibility.md).
- **Dev-only page**: `Section::Quick` exists only under `cfg!(debug_assertions)` (`Section::shown_in_build`), and `settings_list` omits its rows (`QUICK_CARD`).

## Icons and shortcuts

- **Add a UI icon**: download from `lucide-static` into `crates/app/assets/icons/`, thin the stroke to 1.5, add an `Icon` variant and a `handles!` line. Item icons are `Glyph`s from `named_glyph`: a `WORKSPACE_ICONS` name, a name in `assets/icons/library.tsv`, or `file:<name>` in `icons::custom_dir`.
- **Add a shortcut**: an `Action` variant (or `Action::Command`), its name in `keymap::action`, chords in `DEFAULTS_OTHER` and `DEFAULTS_MAC`, handling in `App::perform`. The dialog lists `named_bindings`. Overrides: [`config.md`](config.md).

## Traps

- **A `button` never centers its content**: it lays it out at the top-left of its padding. A row button with a fixed `.height()` and no vertical padding needs its content `.height(Fill)` with `align_y(Center)` (as `menus.rs`, the Git tab's file rows and the agent setup list do), else the text sits on the top edge.
- **Panel scrollbars**: the Git, agent setup and file tabs, the file tree and the agent dashboard scroll with `widgets::thin_scroll` (or `thin_scrollbar` and `thin_scroll_style` for another direction), not iced's 10px default. `.spacing(n)` after it puts the scrollbar beside the rows instead of over their right edge.
- **iced reuses widget state by tree position**, not identity. `TerminalView` keys its last size by session id for this; don't keep per-session state in widget state.
- **Swapping a widget's type at a tree position rebuilds the subtree**, losing scroll and focus. The settings root is always a `stack` with the picker as an optional layer. Prefer optional children or extra layers.
- **No `height(Fill)` inside a scrollable's column**; and a `Fill` element in a row makes the row claim all the height (why group lines are `SideLine`s).
- **Don't clip a taller child to hide its border**: iced lays it out within the clip's limits, so it shrinks and draws the border anyway.
- **`stack` hides the cursor from lower layers only where an upper layer reports an interaction**; modals need a click catcher and `opaque` content.
- **Windows**: `move_loop.rs` subclasses every window. It starts title bar drags and edge resizes itself (`WM_NCLBUTTONDOWN` with the hit code, then a posted `WM_LBUTTONUP` when the loop ends), and only while the primary button is physically down: the press reaches the window late, and a loop started after the release leaves the window stuck to the cursor, while one that never starts leaves winit's own drag flag set so `window::drag` refuses forever after. It also holds back `WM_DPICHANGED` during a drag and replays one on release, keeping the rect Windows gave a snapped or maximized window. State is per window (`HWND`), dropped on `WM_NCDESTROY`. The custom frame has no system resize, so `resize_edges` provides it.

## Worktree dialog (`view/worktree_dialog.rs`)

Opened from a workspace's menu or `new_worktree` (the active workspace). The repository is the workspace's `workspace_dir`. Create runs `git::add_worktree` off the UI thread (a sibling `<repo>-<branch>` folder; `worktree add <path> <branch>` for an existing branch, `-b` otherwise) and answers with `WorktreeMessage::Created`. Success applies `NewWorkspace`, `RenameWorkspace` and `SetWorkspaceCwd` in one update, which works because sessions spawn lazily (after the folder is set). "Start Claude Code" puts `claude
` in `App.startup_input` for the new tab; `spawn` writes a tab's startup input right after the session starts (after `resume_agent`).

## Agent dashboard (`view/dashboard.rs`)

`App.dashboard` (a bool; `agent_dashboard` toggles it, `dismiss` closes it). `dashboard_agents` lists every tab with `Tab.agent`, needs-input first, then working, then idle, sidebar order within. Each card shows the tab, its workspace, `agent_line` (the sidebar's state text and color), `Agent.summary` (the status line) and how long it's been in its state: `App.agent_since` (tab to state and `Instant`) is set in `ipc.rs`'s `agent` handler when a hook reports a different state, and dropped when the agent ends. Restored agents have no time until their next hook. `dashboard_ticks` sends `Message::Frame` every second while it's open so the times count. The cards sit in a wrapping `row` in a `scrollable`.

## Command palette (`view/palette.rs`)

`App.command_palette` (not `App.palette`, the color palette). `Action::CommandPalette` opens it; `App::palette_items` lists keymap actions (`keymap::NAMES`, labeled by `keymap::label` with their chord from `named_bindings`), workspaces, every tab, `[[ssh]]` profiles and `Section::shown()` settings pages. `fuzzy` scores in-order letter matches (runs and word starts count more); the best `MAX_SHOWN` show. Up/Down come from `palette_keys` (it takes the `search` subscription's slot while open), Enter is the field's `on_submit`, Escape and the backdrop go through `dismiss`. `palette_run` closes it before running, so a target can open its own dialog. A new action needs a line in `keymap::NAMES` to be listed (`every_palette_action_exists` checks the names resolve). `Palette.waiting` makes it the attention inbox (`show_waiting`): `waiting_items` lists `Model::waiting_tabs` (oldest notice first) and nothing else. `next_waiting` is the model's `Command::NextWaiting`, cycling the same order without opening anything.

`Palette.mode` is `Commands`, `Waiting`, `Search(Vec<Scrollback>)` or `History { ws, pane }` (the command history, see [`command-history.md`](command-history.md)). Scrollback search (`search_workspace`, `search_all`, `PaletteMessage::OpenSearch`) copies every running terminal's `all_text` once when it opens (`App::scrollbacks`), then `search_items` lists matching lines, smart case like the find bar, newest first per terminal, without fuzzy reordering. Each result's `Target::Found { tab, query, back }` selects the tab and calls `App::find_in_tab`, which opens the find bar with the query and steps `back` matches up from the newest (`back` counts the matches below the line, plus earlier matches on it).

## Pane letters, reopening tabs, workspace environment, scrollback

- `[workspace_items] pane_badge` is filled from the same `WorkspaceValues` as the sidebar (made `pub(super)` for it) and drawn by the workspace layout over each shown pane's top right, below the tab strip, faded (`PANE_BADGE_*` in `view/panes.rs`). It's drawn over the terminal, not behind it: a solid terminal background would hide it.
- `jump_to_pane` sets `App.pane_letters` (only with more than one pane shown); `App::pane` draws `pane_letter(i)` (A, B, ...) over each of `shown_panes(Rect::UNIT)` in that order, and `pane_letter_keys` takes the next key: a letter focuses that pane (`App::pane_letter`), anything else just closes the letters.
- Closed tabs: `App::apply` records the tabs a close command (`CloseTab`, `CloseActiveTab`, `ClosePane`, `CloseWorkspace`, `CloseActiveWorkspace`) removes, as `ClosedTab { saved: session::saved_tab(..), pane, workspace }` in `App.closed_tabs` (newest last, `CLOSED_TABS_KEPT`), unless the command quits. `reopen_tab` (`update/effects.rs`) focuses the old pane (or workspace), opens a `NewTab` whose new session gets the saved tab through `App.restored` (folder, `claude --resume`), `OpenBrowser` at the page, or `OpenGit` / `OpenAgentSetup` / `OpenFile` at the saved folder or file (reading it at once), then reapplies name, icon, color and mute.
- Workspace env and startup: the workspace menu's Environment & Startup… opens `view/env_dialog.rs` (`EnvDialog`, a `NAME=value` per line editor parsed by `parse_env`, and the command). `session_env` puts the workspace's variables before macaroni's own, and the spawn path types the startup command into each new shell after showing saved scrollback and before `claude --resume` (skipped for SSH tabs).
- Scrollback across restarts: on close, `write_scrollback` (`update/window.rs`) saves each running terminal's `all_text` (last `SCROLLBACK_KEPT_BYTES`) to `scrollback/<millis>-<tab>.txt` next to the config, records the file in `App.scrollback_files` for `session::capture`, and deletes files nothing refers to; `[session] keep_scrollback = false` removes the folder. On restore `show_saved_scrollback` prints the text dimmed (control characters stripped) before the shell's first prompt, then deletes the file. `save_scrollback` writes a terminal's text through an rfd save dialog.

## Driving the UI from a script (Windows)

There's no UI test harness. Prefer `macaroni-cli` ([`control-api.md`](control-api.md)); it needs no focus. With `SendInput`, confirm macaroni is the foreground window before every chord: Windows often refuses `SetForegroundWindow` and input lands in whatever is in front. With the Chromium engine its helpers are also `macaroni.exe`, so find the window by title (ending in "- macaroni") across all of them. `PrintWindow` misses browser pages; capture a browser tab from the screen with the window in front.

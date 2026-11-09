# Accessibility

## Mental model

Four parts, all in the app crate:

- **Settings** (`[accessibility]`, `AccessibilitySettings` in `crates/config/src/settings.rs`): `ui_scale`, `reduce_motion` and `high_contrast` (each a `Follow`: `system`, `on`, `off`), `minimum_contrast`, `cursor_thickness`, `announce_output`. Settings > Accessibility lists them (`accessibility_settings` in `view/settings/list.rs`).
- **Vision and motion** (`accessibility.rs`, `system.rs`, `contrast.rs`): `App.access` (`Effective`) is what's on once `system` options are answered from `App.system_prefs`. `apply_config` sets everything from it.
- **Keyboard navigation** (`update/keyboard.rs`, `view/settings/keys.rs`, `App::git_key`): F6 puts a focus ring on the tab strip, then the sidebar; arrows move it; menus, the settings dialog and the Git tab take arrows too.
- **Screen readers** (`accessibility/adapter.rs`, `accessibility/tree.rs`): an AccessKit adapter per OS window, fed a tree of the sidebar, tabs, terminals' text, menus and dialogs, and a live region that reads new output.

```
system thread (POLL) ─► Message::SystemPrefs ─► apply_config ─► Effective ─► theme, Chrome, Metrics, Fades
F6 ─► Action::CycleRegion ─► App.key_focus ─► focus_keys ─► Message::FocusKey ─► App::focus_key
screen reader ─► WM_GETOBJECT / NSAccessibility ─► adapter ─► Event::Activated ─► App.reader
App::update ─► sync_reader ─► build_tree ─► diff ─► window::run(adapter::update)
reader_ticks (READER_TICK) ─► bounds::find_all ─► ReaderBounds ─► terminal text runs, announce()
```

## Vision and motion

- **System settings** (`system::read`): Windows reads `SPI_GETCLIENTAREAANIMATION` (Animation effects off means reduce motion) and `SPI_GETHIGHCONTRAST`; macOS reads `NSWorkspace`'s reduce motion and increase contrast. `system_prefs` (`main.rs`) runs on a thread while any option follows the system and sends a change; `system_prefs_changed` reapplies the config only when `Effective` changes.
- **UI scale** is iced's `scale_factor` for every window. iced's logical pixels then include it; the system's don't. `Win.size`, `Win.position` and cursor positions are iced's; saved window sizes and positions, `window::position` and new windows' `Position::Specific` are the system's, converted with `App::ui_scale` in `update/window.rs`. `App::scale()` (hairlines) and browser placement multiply the display scale by it. iced sends no resize when only its scale changes, so `apply_config` rescales `Win` sizes itself.
- **High contrast**: the terminal theme becomes `HIGH_CONTRAST_THEME` (`macaroni-high-contrast`, built in), `Chrome::high_contrast` raises text to 7:1 (details 4.5:1) against the background, draws every line in the label color and thickens the focus, attention and open tab lines, and minimum contrast is at least 7.
- **Minimum contrast** lives in `Metrics` (with `cursor_thickness`), so a change clears the row cache. `cell_colors` (`terminal/render.rs`) runs every foreground through `contrast::ensure` against what's behind it, with dim text's alpha counted.
- **Reduced motion**: `Fades::instant` makes every fade start at its target, so `window::frames` never runs for them.

## Keyboard navigation

- `App.key_focus: Option<KeyFocus>` (`state.rs`): a sidebar stop (group header, workspace, New workspace) or a strip stop (tab, `+`, new tab button). While it's set, `TerminalView` doesn't take keys (`terminal_view`'s condition), `unhandled_keys` is off, and `focus_keys` sends every key no widget captured as `Message::FocusKey`. Keys the ring doesn't use are looked up in the keymap, so shortcuts still work.
- Regions: F6 / Shift+F6 (`focus_next_region`, `focus_previous_region`) and Tab / Shift+Tab cycle terminal, strip, sidebar (skipped while folded). Stops come from `sidebar_stops` (the sidebar's own order and search filter) and `strip_stops`; `strip_buttons` is the one list of new tab buttons, which the view draws too.
- Keys: arrows move (Left/Right in a strip, Up/Down in the sidebar; Up/Down in a strip go to the pane before or after, Left/Right on a group header fold it), Home/End, Enter activates and hands the keyboard back, Space activates and stays, F2 renames, Delete closes a tab, Shift+F10 or the menu key opens the item's menu under it (`bounds::find` by `KeyFocus::widget_id`), Escape leaves. Any mouse press drops the ring (`SidebarSearchCheck`).
- Rings: `widgets::focus_ring` wraps each stop in a container with its widget id and a ring layer that's always there (clear when unfocused) so focus moving doesn't rebuild the item.
- Menus: `Menu.cursor` (`MenuCursor`) is the highlighted item, drawn like a hovered one. `menu_items` lists what the keyboard can reach (items that act or open a submenu). Right or Enter on a submenu item opens it with its first item highlighted; Left, or Escape (`dismiss` checks first), closes it.
- Settings dialog (`view/settings/keys.rs`): `Settings.row` (a `RowKey`: page, card, label) is ringed; `settings_keys` sends uncaptured keys as `SettingsMessage::Key`. Up/Down move through `keyed_rows` (the page's or search's rows, minus folded cards and shortcuts) and scroll the row into view (`RowSpan`), taking the keyboard out of the search field (Up/Down pass through text fields, Space and Left/Right don't). Space/Enter flips, cycles or presses, or focuses the row's field (`field_id`); Left/Right step numbers and choices; Delete resets; Ctrl+Page Up/Down changes page.
- Git tab (`App::git_key`, from `unhandled_key`): Up/Down select files or commits (Alt for a commit's files), Space checks a file, Delete asks to discard, Ctrl+Enter commits, F5 reloads.

## Screen readers

- **Adapters** (`adapter.rs`) aren't `Send` and must live on the event loop's thread, so `attach`, `update`, `focus` and `detach` always run inside `window::run`, and adapters sit in a thread-local map. `attach` runs in `window_opened`'s `window::run`, `detach` before `window::close`.
  - Windows: a `SetWindowSubclass` procedure (id 2; `move_loop` is 1) hands `WM_GETOBJECT` to `accesskit_windows::Adapter`, follows `WM_SETFOCUS`/`WM_KILLFOCUS`, and drops the state on `WM_NCDESTROY`. AccessKit's own `SubclassingAdapter` isn't used because it panics on a window that's already visible, which an iced window can be by the time the app gets its handle.
  - macOS: `accesskit_macos::SubclassingAdapter` on the window's `NSView`, inside `catch_unwind` (it panics if something already observes the view); window focus goes through `adapter::focus`.
- **Activation**: nothing is built until a screen reader asks. The activation handler returns `None` (AccessKit shows a placeholder) and sends `Event::Activated`; `reader_event` marks the window `fresh`, and the next `sync_reader` sends a whole tree. Actions arrive as `Event::Action` (any thread) and go to `reader_action` by the node's `Target`.
- **The tree** (`tree.rs`, `build_tree` with `viewing` set so `shown()` answers for that window): Window > the sidebar (`Tree` of `TreeItem`s, groups holding their members, then a New workspace button), Panes > each pane (`Group` "Pane A") > `TabList` (tabs, `+`, new tab buttons) and the open tab's content (`Terminal` with a `TextRun` per row and the caret as its text selection, a `Document` for a page, a `Group` for Git), the announcer (`Status`, polite live region), and the open menu, palette or settings dialog in the focused window.
- **Ids**: model items use their model id (unique across kinds); fixed nodes count down from `u64::MAX` (`fixed`); the rest put a `Kind` in the top byte (`derived`).
- **Updates**: `sync_reader` runs after every `App::update`; `next_update` diffs the built nodes against what was sent and sends only changes plus the focus. Focus is the menu's highlighted item, the palette's highlighted match, the ringed settings row, the ringed stop, else the focused pane's content; it must always be a node in the tree.
- **Ticks** (`READER_TICK`, only while a window is read): `reader_tick` asks `bounds::find_all` where every stop and terminal view (`terminal_id`) is; `reader_bounds` stores that, rebuilds each shown terminal's text runs, and runs `announce`.
- **Announcing output** (`announce`, `new_lines`): the focused terminal's visible lines are compared with last tick's, allowing for scrolling and a grown last line; what's new (at most `MAX_ANNOUNCED_LINES`) becomes the announcer's label. The cursor's row is left out for `TYPING_QUIET` after a key is typed (`Reader.typed`, set in `view_event`), since screen readers echo keys. A newly focused terminal isn't read out.

## Files

- `crates/config/src/settings.rs` (`AccessibilitySettings`, `Follow`), `template.toml`, `themes/macaroni-high-contrast.toml`.
- `crates/app/src/accessibility.rs` (`Effective`, `POLL`), `accessibility/{system,contrast,bounds,adapter,tree}.rs`.
- `crates/app/src/update/keyboard.rs`, `state.rs` (`KeyFocus`, `MenuCursor`), `view/widgets.rs` (`focus_ring`, `ring`), `view/settings/keys.rs`.
- `crates/app/src/update/config.rs` (applying it), `update/window.rs` (UI scale conversions, adapters), `theme.rs` (`Chrome::high_contrast`), `terminal/render.rs` (contrast, cursor), `anim.rs` (`Fades::instant`).

## Traps

- **Keep the focus a real node.** AccessKit panics on a focus or child id that isn't in the tree. `build_tree_here` only focuses a stop it built.
- **Never send a node whose parent isn't in the tree or the same update.** Sending only changed nodes works because a parent whose children changed is itself changed.
- **Don't hold the adapter map's borrow while raising events**: raising can call back into the window procedure. `update` and the procedure raise after the borrow ends, and the procedure uses `try_borrow_mut`.
- **A new stop needs a widget id and a ring** (`ringed`), a place in `sidebar_stops` or `strip_stops`, and a node in `build_sidebar` or `build_panes`.
- **Window geometry mixes two scales** under `ui_scale`; see Vision and motion before touching window positions or sizes.
- **Testing**: `cargo test -p macaroni accessibility` (contrast math, `new_lines`, and `every_update_makes_a_valid_tree`, which feeds every update through `accesskit_consumer`'s tree, which panics on an invalid one). A live check needs Narrator (Ctrl+Win+Enter) or NVDA on Windows, VoiceOver (Cmd+F5) on macOS, or Accessibility Insights to inspect the tree.

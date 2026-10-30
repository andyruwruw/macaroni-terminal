# Model and commands

## Mental model

`crates/core` (`macaroni-core`) is the app state as plain data plus one function that changes it. No dependencies: no iced, no PTY, no config.

```
Command ──► Model::apply ──► Vec<Effect>   (the app runs effects in update/effects.rs::run_effects)
Model { workspaces (sidebar order), active, groups, panes, tabs, next_id }
Workspace { id, name, layout: SplitNode, focused: PaneId, cwd, group }
SplitNode = Leaf(PaneId) | Split { axis: Row|Column, ratio, first, second }
Pane { id, tabs: Vec<TabId>, active }
Tab { id, title, name, kind: TabKind, attention, agent, cwd, url, muted, pinned }
```

- **`Model::apply` is the only way to change state.** Keybindings, menus, the sidebar and the socket API all produce `Command`s. Side effects are returned, never performed: `SpawnSession`, `KillSession`, `OpenBrowser`, `CloseBrowser`, `Quit`.
- **Resources are only ids here.** The app owns the `SessionId → Session` and browser registries. Moving or rearranging tabs emits no effects; only creating and closing tabs do.
- **Ids come from one counter and are never reused**, so addresses stay valid for the socket API.

## Files

- `model.rs`: types, queries, `Command`, `Effect`, and `Model::apply`, a one-line-per-command dispatcher that ends with `drop_empty_groups` and a debug-build `check()`.
- `model/`: command bodies by area. `workspaces.rs` (`take_blocks`, `insert_block`, `set_pinned_block`, `remove_workspace`), `groups.rs`, `panes.rs` (splits, focus, `close_pane` and `detach_empty_pane`, both via `remove_leaf`), `tabs.rs` (`move_tab`, `take_tab`, `insert_tab_at`), `events.rs` (session/browser updates, attention, agents), `check.rs` (invariants, one function per area).
- `layout.rs`: tree operations, `Rect::UNIT` geometry, `neighbour`, `neighbour_of`, `dividers`, `MIN_RATIO`.

## Invariants (`Model::check`, after every command in debug builds)

- At least one workspace; `active` exists.
- Each workspace's `focused` pane is a leaf of its layout; every pane is in exactly one layout.
- Every pane has at least one tab and its `active` is one of them; every tab is in exactly one pane.
- Pinned tabs come first in each pane; pinned workspaces come first in the sidebar.
- A group's workspaces are contiguous and share a pinned state; every referenced group exists; no group is empty.
- Every split ratio is strictly between 0 and 1 (`SetSplitRatio` clamps to `MIN_RATIO..1-MIN_RATIO`).

## Behaviour that code relies on

- Closing cascades: last tab closes the pane, last pane closes the workspace. Closing the last workspace leaves the model unchanged and emits `KillSession`/`CloseBrowser` for everything, then `Quit`.
- When a pane closes, focus goes to the nearest leaf of its sibling subtree (`SplitNode::neighbour_of`). `FocusDirection` uses geometry: `layout.panes(Rect::UNIT)` plus `layout::neighbour`.
- `SessionExited` closes the tab and emits `KillSession` so the app drops the dead `Session`. `BrowserClosed` does the same for a page that closed itself.
- `MoveTab { tab, to: MoveTo::... }` re-parents without effects. A pane left empty is detached (`detach_empty_pane`), not killed. Pointless moves are ignored.
- Sidebar order is a list of blocks (a group's workspaces, or one ungrouped workspace; `take_blocks`). `PlaceWorkspace` is what sidebar drags apply; `insert_block` keeps pinned-first; `SetWorkspacePinned` on a grouped workspace pins the whole group (`set_pinned_block`); joining a group (`set_workspace_group`) takes its pinned state.
- `SetTabPinned` is a no-op when the flag is unchanged; session restore relies on this.
- `Tab::label()` is name-else-title; use it wherever a tab is shown. `Model::is_muted(tab)` checks tab and workspace mute; `Notify` on a muted tab does nothing.
- Icon/color names and `Workspace.cwd` are stored uninterpreted; the app maps and validates them.
- `layout.dividers(area)` gives each split's `path` (`Vec<Side>` from the root) for `SetSplitRatio`.
- `Model::spec()` / `Model::restore(spec, active)` convert to and from plain data (`WorkspaceSpec`, `LayoutSpec`, `TabSpec`). Restore uses fresh ids, clamps bad values, and returns tabs in the same fixed order as `spec()`, which the app's `session.rs` uses to pair saved per-tab details.

## Extending

- **Command:** add a `Command` variant, a dispatch line in `Model::apply`, and the body in the right `model/` file. Push effects rather than doing work. Add a unit test in `tests.rs` and, if valid at random, a case in the random test's generator.
- **Effect:** add the variant and handle it in the app's `run_effects` (`crates/app/src/update/effects.rs`).
- **Tab kind:** a `TabKind` variant, a `TabSpec` variant for saved layouts, an open/close `Effect` pair if it owns a resource, and a body in the app's pane view. Code that needs a terminal (`App::focused_session`, the IPC `session` helper, the find bar) matches on the kind and treats others as "not a terminal".
- **Invariant:** add it to the relevant `check_*` in `model/check.rs`.

## Tests

`cargo test -p macaroni-core`. `layout.rs` tests tree ops and directional focus. `tests.rs` has a unit test per command plus `random_commands_keep_the_model_consistent`: seeded runs of random valid commands asserting `check()` and that spawned/killed sessions match the model's tabs. It uses a tiny built-in PRNG (no `proptest`, no shrinking); a failure prints the seed and step.

## Traps

- **Smart App Control** on Windows can block freshly compiled build scripts ("os error 4551"). Turn it off on dev machines.

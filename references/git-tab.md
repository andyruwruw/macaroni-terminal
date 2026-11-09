# Git tab

## Mental model

A Git tab is a repository's Changes and History, in the manner of GitHub Desktop, drawn with macaroni's own widgets (no web view). It's a tab kind like terminals and browsers, so it splits, drags, moves between windows and is saved with the session.

```
model:  TabKind::Git, folder in Tab.cwd        (Command::OpenGit, Command::SetGitFolder)
app:    App.git_tabs: TabId -> GitTab           (view/git_tab.rs: state, GitMessage, update, view)
git:    git/repo.rs: status, diffs, history, Op + perform   (blocking; run via vscode::off_thread)
```

- **The model holds only the kind and the folder.** `OpenGit` and `SetGitFolder` emit no effects; nothing is opened or closed. `TabSpec::Git(folder)` saves it, `SavedTab.git` in `session.json`, and reopening a closed one (Ctrl+Shift+R) works like a browser tab's.
- **Everything shown is the app's**, in `App.git_tabs`, made on first use (`entry().or_default()` in `App::git_update`) and pruned in `App::apply` after commands that close tabs. A tab without state draws "Loading the repository...".
- **One status load at a time per tab** (`GitTab.loading`), and **one operation at a time** (`running`, the rest in `queue`). Each finished operation (`GitMessage::Done`) starts the next, or reloads the status. A status that arrives while an operation runs is dropped: it predates the operation, and would undo the checkbox's optimistic flip.
- **Refreshing**: opening the tab loads it; `Message::GitTick` (every `GIT_TICK`) reloads every Git tab on screen (`App::shown_git_tabs`), so an agent's edits show up on their own. The tick subscription runs while the sidebar shows git placeholders or a Git tab is on screen; it fires once when it starts, which loads restored tabs.
- **Checkbox = staged.** A file's checkbox stages it whole (`git add -A`) or unstages it (`git reset`, or `git rm --cached` before the first commit). Partly staged files show a minus. Commit commits the index, so the count on the button is the staged files.
- **Diffs** in Changes are against `HEAD` (staged and unstaged together, as GitHub Desktop shows them), or the empty tree before the first commit; untracked files use `diff --no-index /dev/null`. In History they're a commit against its first parent. `Diff` keeps at most `MAX_DIFF_LINES`. `DiffKey` drops a late answer for an earlier selection.
- **History** reloads only when `HEAD` or the ahead count changed (`History.head`, `.ahead`), so ticks don't redo `git log`. Commits the upstream lacks show an up arrow.
- **The remote button** (`remote_action`): Publish branch (no upstream: `push -u <remote> HEAD`), Pull when behind, Push when ahead, else Fetch. The remote is the upstream's, else `origin`, else the first.
- **Layout**: the left column (Changes or History list) sits on the sidebar's background (`sidebar_background`), and its edge drags (`GitTab.list_width`, `resizing`): a `RESIZE_HANDLE` `mouse_area` pinned over the line arms it, the columns' `mouse_area` reads moves relative to itself (so the cursor's x is the width), and `drag_events` runs while `App::git_resizing` so a release anywhere ends it (`end_git_resize`). The columns are always wrapped in that `mouse_area`, so starting a drag doesn't rebuild their state. The diff's wrap button (`GitTab.wrap`) wraps long lines and scrolls only down.
- **Menus** (repository and branch) are drawn inside the tab (`stack` + `pin` under the top bar, with a full-size `mouse_area` behind that closes them), not with the app's right-click menus, which open at the cursor. The repository menu lists `git::top` of every tab's and workspace's folder (a file system walk, no `git`).

## Files

- `crates/app/src/view/git_tab.rs`: `GitTab`, `GitMessage`, `App::git_update`, `App::git_body` and its parts, `open_git_tab`, `shown_git_tabs`.
- `crates/app/src/git/repo.rs`: `status` (porcelain v2, `-z`), `change_diff`, `commit_diff`, `history`, `commit_files`, `branches`, `Op` and `perform`, with parsers and a test against a real repository.
- `crates/app/src/git.rs`: the sidebar's git state, and `top`.
- Entry points: the strip's new-tab buttons and new-tab menu ("Git"), the `git_tab` action (Ctrl+Shift+H, Cmd+Shift+H), the command palette. Outside a repository (`App::pane_in_repository`, `git::top` of `pane_dir`) the button is grayed out with a tooltip, the menu item is disabled, and `open_git_tab` does nothing.
- Changing the folder: "Change Folder…" on the tab's menu (also opened by right-clicking anywhere in the tab, `git_body` wraps it in `RightClick`) opens the folder dialog (`view/folder_dialog.rs`, `FolderTarget::Git`), which only takes a folder in a repository. It and the repository menu go through `App::git_folder_changed`, which drops the tab's state, since it was another repository's.

## Traps

- **`git` must never wait for input.** Every command gets `GIT_TERMINAL_PROMPT=0` and no stdin (the commit message goes in on stdin with `-F -`), and runs without a console window (`vscode::hide_window`). Signing in to a remote is the credential helper's job (Git Credential Manager has its own window); without one, push fails and the error banner says why.
- **Paths come through `-z`**, so names with spaces or non-ASCII characters aren't quoted. Renames carry both paths (`Change.from`), and staging or discarding passes both.
- **Commit messages use `--cleanup=whitespace`**, not git's default `strip`, which would drop a description line starting with `#`.
- **A diff's rows can't fill a sideways scrollable**, so `diff_view` sizes them with `responsive`: as wide as the longest line, or the space when that's wider, so the tint runs to the edge.
- **`--no-optional-locks`** on `status`, so the tick never takes `index.lock` from a `git` the user is running.

## Extending

- A new operation: an `Op` variant, its `doing` text, its arm in `perform`, and a button that sends a `GitMessage` which `enqueue`s it.
- Testing: `cargo test -p macaroni git` runs the parsers and `stages_commits_and_discards_in_a_real_repository`, which builds a repository in the temp folder. To look at it, start an isolated instance (`MACARONI_CONFIG_DIR`, `MACARONI_SOCKET`) with `[terminal] default_dir` at a scratch repository and press Ctrl+Shift+H.

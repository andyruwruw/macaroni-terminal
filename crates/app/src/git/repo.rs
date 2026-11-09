//! What a Git tab shows and does, by running `git`: the working tree's changes, a file's diff,
//! the history and a commit's files, and staging, committing, discarding, switching branches,
//! fetching, pulling and pushing.
//!
//! Everything here blocks, so the tab runs it off the UI thread (`crate::vscode::off_thread`).
//! Output is read with `-z` where paths are involved, so names with spaces, quotes or non-ASCII
//! characters come through as they are. Remote commands get `GIT_TERMINAL_PROMPT=0`: with no
//! console to type into, asking for a password would hang, so they fail instead and a credential
//! helper (Git Credential Manager on Windows) is what signs in.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Git's empty tree, the base a repository's first commit (or a repository with none yet) is
/// compared with.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// The most diff lines kept for one file; the rest are counted, not shown.
const MAX_DIFF_LINES: usize = 5000;

/// How many commits the History view lists.
const HISTORY_LENGTH: usize = 300;

/// The repository holding a folder: its branch, how it stands against its upstream, and every
/// changed file.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    /// The repository's top folder.
    pub top: PathBuf,
    /// The checked-out branch; `None` when `HEAD` is detached.
    pub branch: Option<String>,
    /// The commit `HEAD` points at; `None` before the first commit.
    pub head: Option<String>,
    /// The branch's upstream (`origin/main`), if it has one.
    pub upstream: Option<String>,
    /// Commits on the branch its upstream doesn't have.
    pub ahead: u32,
    /// Commits on the upstream the branch doesn't have.
    pub behind: u32,
    /// The remote a branch without an upstream is published to: `origin`, else the first one.
    pub remote: Option<String>,
    /// Changed files, by path.
    pub files: Vec<Change>,
}

/// A changed file in the working tree or the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Its path from the repository's top, with `/` separators.
    pub path: String,
    /// Where a renamed file came from.
    pub from: Option<String>,
    /// What changed, against `HEAD`.
    pub kind: ChangeKind,
    /// How much of the change is staged for the next commit.
    pub staged: Staged,
}

/// What happened to a changed file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// New, staged or not.
    Added,
    /// Changed.
    Modified,
    /// Removed.
    Deleted,
    /// Moved, maybe changed too.
    Renamed,
    /// Both sides of a merge changed it.
    Conflicted,
}

impl ChangeKind {
    /// The letter the file list shows.
    pub fn letter(self) -> &'static str {
        match self {
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Conflicted => "!",
        }
    }
}

/// How much of a file's change is in the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Staged {
    /// All of it.
    All,
    /// Some of it: the working tree has more.
    Some,
    /// None of it.
    None,
}

/// One line of a diff, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// What kind of line it is.
    pub kind: LineKind,
    /// Its number in the old file, for context and removed lines.
    pub old: Option<u32>,
    /// Its number in the new file, for context and added lines.
    pub new: Option<u32>,
    /// Its text, without the leading `+`, `-` or space, with tabs as spaces.
    pub text: String,
}

/// What a diff line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// A hunk's `@@ -a,b +c,d @@` header.
    Hunk,
    /// Unchanged.
    Context,
    /// Only in the new file.
    Added,
    /// Only in the old file.
    Removed,
    /// `\ No newline at end of file`.
    Note,
}

/// A file's diff.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Diff {
    /// Its lines, at most [`MAX_DIFF_LINES`].
    pub lines: Vec<Line>,
    /// Git called it binary: there are no lines.
    pub binary: bool,
    /// How many lines past [`MAX_DIFF_LINES`] were left out.
    pub cut: usize,
}

/// A commit in the History view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Its full hash.
    pub id: String,
    /// Its first parent; `None` for a root commit.
    pub parent: Option<String>,
    /// The author's name.
    pub author: String,
    /// When it was authored, in seconds since 1970.
    pub time: i64,
    /// The message's first line.
    pub summary: String,
    /// The rest of the message.
    pub body: String,
    /// Not on the upstream yet.
    pub unpushed: bool,
}

/// A file a commit changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFile {
    /// Its path after the commit.
    pub path: String,
    /// Where a renamed file came from.
    pub from: Option<String>,
    /// What the commit did to it.
    pub kind: ChangeKind,
}

/// The repository holding `dir` and its changes. Untracked files are listed one by one, as
/// GitHub Desktop does, so a new folder's files can be picked separately.
pub fn status(dir: &Path) -> Result<Status, String> {
    let top = PathBuf::from(git(dir, &["rev-parse", "--show-toplevel"])?.trim());
    let out = git(
        &top,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=all",
            "-z",
        ],
    )?;
    let remotes = git(&top, &["remote"]).unwrap_or_default();
    let remote = remotes
        .lines()
        .find(|r| *r == "origin")
        .or_else(|| remotes.lines().next())
        .map(str::to_owned);
    let mut status = parse_status(&out);
    status.top = top;
    status.remote = status
        .upstream
        .as_deref()
        .and_then(|u| remotes.lines().find(|r| u.starts_with(&format!("{r}/"))))
        .map(str::to_owned)
        .or(remote);
    Ok(status)
}

/// Reads `git status --porcelain=v2 --branch -z`: `# branch.*` headers, then a record per
/// changed file (`1` changed, `2` renamed with its old path in the next field, `u` unmerged,
/// `?` untracked).
fn parse_status(out: &str) -> Status {
    let mut status = Status {
        top: PathBuf::new(),
        branch: None,
        head: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        remote: None,
        files: Vec::new(),
    };
    let mut fields = out.split('\0');
    while let Some(record) = fields.next() {
        if let Some(header) = record.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.oid" if value != "(initial)" => status.head = Some(value.to_owned()),
                "branch.head" if value != "(detached)" => status.branch = Some(value.to_owned()),
                "branch.upstream" => status.upstream = Some(value.to_owned()),
                "branch.ab" => {
                    let mut parts = value.split_whitespace();
                    status.ahead = parts
                        .next()
                        .and_then(|p| p.trim_start_matches('+').parse().ok())
                        .unwrap_or(0);
                    status.behind = parts
                        .next()
                        .and_then(|p| p.trim_start_matches('-').parse().ok())
                        .unwrap_or(0);
                }
                _ => {}
            }
            continue;
        }
        let change = match record.split_at_checked(2) {
            Some(("1 ", rest)) => {
                let parts: Vec<_> = rest.splitn(8, ' ').collect();
                parts.get(7).map(|path| changed(parts[0], path, None))
            }
            Some(("2 ", rest)) => {
                let parts: Vec<_> = rest.splitn(9, ' ').collect();
                let from = fields.next().map(str::to_owned);
                parts.get(8).map(|path| changed(parts[0], path, from))
            }
            Some(("u ", rest)) => rest.splitn(10, ' ').nth(9).map(|path| Change {
                path: path.to_owned(),
                from: None,
                kind: ChangeKind::Conflicted,
                staged: Staged::None,
            }),
            Some(("? ", path)) => Some(Change {
                path: path.to_owned(),
                from: None,
                kind: ChangeKind::Added,
                staged: Staged::None,
            }),
            _ => None,
        };
        status.files.extend(change);
    }
    status.files.sort_by(|a, b| a.path.cmp(&b.path));
    status
}

/// A tracked file's change from its two status letters (index, then working tree; `.` for
/// none).
fn changed(xy: &str, path: &str, from: Option<String>) -> Change {
    let mut letters = xy.chars();
    let index = letters.next().unwrap_or('.');
    let tree = letters.next().unwrap_or('.');
    let kind = if index == 'R' || index == 'C' {
        ChangeKind::Renamed
    } else if index == 'A' {
        ChangeKind::Added
    } else if index == 'D' || tree == 'D' {
        ChangeKind::Deleted
    } else {
        ChangeKind::Modified
    };
    let staged = match (index != '.', tree != '.') {
        (true, false) => Staged::All,
        (true, true) => Staged::Some,
        _ => Staged::None,
    };
    Change {
        path: path.to_owned(),
        from,
        kind,
        staged,
    }
}

/// A working tree file's whole change against `HEAD` (staged or not), as the Changes view shows
/// it. A new file not yet staged is compared with nothing.
pub fn change_diff(top: &Path, change: &Change, head: Option<&str>) -> Result<Diff, String> {
    let untracked = change.kind == ChangeKind::Added && change.staged == Staged::None;
    let out = if untracked {
        // `--no-index` exits 1 when the files differ, which they always do here.
        run(
            top,
            &[
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-index",
                "--",
                "/dev/null",
                &change.path,
            ],
            true,
        )?
    } else {
        let mut args = vec![
            "diff",
            "--no-color",
            "--no-ext-diff",
            "-M",
            head.unwrap_or(EMPTY_TREE),
            "--",
            &change.path,
        ];
        if let Some(from) = &change.from {
            args.push(from);
        }
        git(top, &args)?
    };
    Ok(parse_diff(&out))
}

/// The change a commit made to one file, against its first parent.
pub fn commit_diff(top: &Path, commit: &Commit, file: &CommitFile) -> Result<Diff, String> {
    let parent = commit.parent.as_deref().unwrap_or(EMPTY_TREE);
    let mut args = vec![
        "diff",
        "--no-color",
        "--no-ext-diff",
        "-M",
        parent,
        &commit.id,
        "--",
        &file.path,
    ];
    if let Some(from) = &file.from {
        args.push(from);
    }
    Ok(parse_diff(&git(top, &args)?))
}

/// Reads a unified diff, skipping the file headers before the first hunk.
fn parse_diff(out: &str) -> Diff {
    let mut diff = Diff::default();
    let (mut old, mut new) = (0, 0);
    let mut in_hunk = false;
    for raw in out.lines() {
        if raw.starts_with("Binary files ") || raw == "GIT binary patch" {
            diff.binary = true;
            continue;
        }
        let line = if let Some(header) = raw.strip_prefix("@@") {
            in_hunk = true;
            let numbers = |sign: char| {
                header
                    .split_whitespace()
                    .find_map(|p| p.strip_prefix(sign))
                    .and_then(|p| p.split(',').next()?.parse::<u32>().ok())
                    .unwrap_or(1)
            };
            old = numbers('-');
            new = numbers('+');
            Line {
                kind: LineKind::Hunk,
                old: None,
                new: None,
                text: raw.to_owned(),
            }
        } else if !in_hunk {
            continue;
        } else if let Some(text) = raw.strip_prefix('+') {
            new += 1;
            Line {
                kind: LineKind::Added,
                old: None,
                new: Some(new - 1),
                text: text.to_owned(),
            }
        } else if let Some(text) = raw.strip_prefix('-') {
            old += 1;
            Line {
                kind: LineKind::Removed,
                old: Some(old - 1),
                new: None,
                text: text.to_owned(),
            }
        } else if raw.starts_with('\\') {
            Line {
                kind: LineKind::Note,
                old: None,
                new: None,
                text: raw.trim_start_matches('\\').trim().to_owned(),
            }
        } else if raw.starts_with("diff ") {
            // A second file's header (a rename shown as delete and add): skip to its hunks.
            in_hunk = false;
            continue;
        } else {
            old += 1;
            new += 1;
            Line {
                kind: LineKind::Context,
                old: Some(old - 1),
                new: Some(new - 1),
                text: raw.strip_prefix(' ').unwrap_or(raw).to_owned(),
            }
        };
        if diff.lines.len() < MAX_DIFF_LINES {
            diff.lines.push(Line {
                text: line.text.replace('\t', "    "),
                ..line
            });
        } else {
            diff.cut += 1;
        }
    }
    diff
}

/// The latest commits on `HEAD`, newest first, each marked when its upstream doesn't have it.
/// Empty before the first commit.
pub fn history(top: &Path, status: &Status) -> Result<Vec<Commit>, String> {
    if status.head.is_none() {
        return Ok(Vec::new());
    }
    let out = git(
        top,
        &[
            "log",
            &format!("-n{HISTORY_LENGTH}"),
            "--format=%H%x1f%P%x1f%an%x1f%at%x1f%s%x1f%b%x1e",
        ],
    )?;
    let unpushed: Vec<String> = match &status.upstream {
        Some(upstream) => git(top, &["rev-list", &format!("{upstream}..HEAD")])
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect(),
        None => Vec::new(),
    };
    Ok(parse_log(&out, &unpushed))
}

/// Reads [`history`]'s `git log`: records end in `\x1e`, fields are split by `\x1f`.
fn parse_log(out: &str, unpushed: &[String]) -> Vec<Commit> {
    out.split('\x1e')
        .filter_map(|record| {
            let mut fields = record.trim_start_matches('\n').split('\x1f');
            let id = fields.next().filter(|id| !id.is_empty())?.to_owned();
            let parent = fields
                .next()
                .and_then(|p| p.split_whitespace().next())
                .map(str::to_owned);
            Some(Commit {
                unpushed: unpushed.contains(&id),
                id,
                parent,
                author: fields.next().unwrap_or_default().to_owned(),
                time: fields.next().and_then(|t| t.parse().ok()).unwrap_or(0),
                summary: fields.next().unwrap_or_default().to_owned(),
                body: fields.next().unwrap_or_default().trim().to_owned(),
            })
        })
        .collect()
}

/// The files `commit` changed, against its first parent.
pub fn commit_files(top: &Path, commit: &Commit) -> Result<Vec<CommitFile>, String> {
    let parent = commit.parent.as_deref().unwrap_or(EMPTY_TREE);
    let out = git(
        top,
        &["diff", "--name-status", "-z", "-M", parent, &commit.id],
    )?;
    Ok(parse_name_status(&out))
}

/// Reads `git diff --name-status -z`: a status (`M`, `A`, `D`, `R100`...), then the path, or
/// for a rename or copy the old path and the new one.
fn parse_name_status(out: &str) -> Vec<CommitFile> {
    let mut files = Vec::new();
    let mut fields = out.split('\0').filter(|f| !f.is_empty());
    while let Some(code) = fields.next() {
        let renamed = code.starts_with('R') || code.starts_with('C');
        let (from, path) = if renamed {
            (fields.next().map(str::to_owned), fields.next())
        } else {
            (None, fields.next())
        };
        let Some(path) = path else { break };
        let kind = match code.chars().next() {
            Some('A') => ChangeKind::Added,
            Some('D') => ChangeKind::Deleted,
            Some('R' | 'C') => ChangeKind::Renamed,
            Some('U') => ChangeKind::Conflicted,
            _ => ChangeKind::Modified,
        };
        files.push(CommitFile {
            path: path.to_owned(),
            from,
            kind,
        });
    }
    files
}

/// The local branches, the checked-out one included, by name.
pub fn branches(top: &Path) -> Result<Vec<String>, String> {
    let out = git(top, &["branch", "--format=%(refname:short)"])?;
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Something a Git tab asks `git` to do. Each runs on its own and is followed by a fresh
/// [`status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Stage these files whole (a deletion stages as one).
    Stage(Vec<String>),
    /// Take these files out of the next commit, keeping their changes.
    Unstage(Vec<String>),
    /// Commit what's staged with this message.
    Commit(String),
    /// Throw away a file's changes: back to `HEAD`, or deleted when it's new.
    Discard(Change),
    /// Check out a local branch.
    Switch(String),
    /// Create a branch at `HEAD` and check it out.
    CreateBranch(String),
    /// Fetch every remote, dropping branches deleted there.
    Fetch,
    /// Pull the upstream into the branch.
    Pull,
    /// Push the branch to its upstream.
    Push,
    /// Push a branch with no upstream to this remote and make that its upstream.
    Publish(String),
}

impl Op {
    /// What the tab says while it runs.
    pub fn doing(&self) -> &'static str {
        match self {
            Self::Stage(_) | Self::Unstage(_) => "Updating",
            Self::Commit(_) => "Committing",
            Self::Discard(_) => "Discarding",
            Self::Switch(_) | Self::CreateBranch(_) => "Switching branch",
            Self::Fetch => "Fetching",
            Self::Pull => "Pulling",
            Self::Push | Self::Publish(_) => "Pushing",
        }
    }
}

/// Runs `op` in the repository at `top`; what `git` said went wrong if it failed.
pub fn perform(top: &Path, op: &Op, has_head: bool) -> Result<(), String> {
    let paths = |command: &[&str], paths: &[String]| -> Vec<String> {
        command
            .iter()
            .map(|s| (*s).to_owned())
            .chain(std::iter::once("--".to_owned()))
            .chain(paths.iter().cloned())
            .collect()
    };
    let run_owned = |args: Vec<String>| {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        git(top, &args).map(drop)
    };
    match op {
        Op::Stage(files) => run_owned(paths(&["add", "-A"], files)),
        // Before the first commit there's no `HEAD` to reset the index to.
        Op::Unstage(files) if has_head => run_owned(paths(&["reset", "-q"], files)),
        Op::Unstage(files) => run_owned(paths(&["rm", "--cached", "-r", "-q"], files)),
        Op::Commit(message) => input(
            top,
            &["commit", "-q", "--cleanup=whitespace", "-F", "-"],
            message,
        )
        .map(drop),
        Op::Discard(change) => discard(top, change, has_head),
        Op::Switch(branch) => git(top, &["switch", branch]).map(drop),
        Op::CreateBranch(branch) => git(top, &["switch", "-c", branch.trim()]).map(drop),
        Op::Fetch => git(top, &["fetch", "--all", "--prune"]).map(drop),
        Op::Pull => git(top, &["pull"]).map(drop),
        Op::Push => git(top, &["push"]).map(drop),
        Op::Publish(remote) => git(top, &["push", "-u", remote, "HEAD"]).map(drop),
    }
}

/// Throws away `change`: a file new to the repository is deleted (and dropped from the index),
/// anything else goes back to `HEAD` in the index and the working tree.
fn discard(top: &Path, change: &Change, has_head: bool) -> Result<(), String> {
    if change.kind == ChangeKind::Added || !has_head {
        if change.staged != Staged::None {
            git(top, &["rm", "--cached", "-q", "-f", "--", &change.path])?;
        }
        let path = top.join(&change.path);
        return std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()));
    }
    let mut args = vec![
        "restore",
        "--source=HEAD",
        "--staged",
        "--worktree",
        "--",
        &change.path,
    ];
    if let Some(from) = &change.from {
        args.push(from);
    }
    git(top, &args).map(drop)
}

/// Runs `git -C dir <args>`; its output, or its error.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    run(dir, args, false)
}

/// Runs `git -C dir <args>` without a console window or a terminal prompt; its output, or what
/// it printed to stderr when it failed. `differ_ok` also takes exit code 1, which
/// `diff --no-index` uses for "the files differ".
fn run(dir: &Path, args: &[&str], differ_ok: bool) -> Result<String, String> {
    let out = command(dir, args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git didn't run: {e}"))?;
    if out.status.success() || differ_ok && out.status.code() == Some(1) {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    Err(failure(&out.stderr))
}

/// Runs `git -C dir <args>` with `text` on its stdin.
fn input(dir: &Path, args: &[&str], text: &str) -> Result<String, String> {
    let mut child = command(dir, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git didn't run: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("git didn't take the message: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("git didn't finish: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    Err(failure(&out.stderr))
}

/// `git -C dir <args>`, hidden, never prompting.
fn command(dir: &Path, args: &[&str]) -> Command {
    let mut git = Command::new("git");
    git.arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0");
    crate::vscode::hide_window(&mut git);
    git
}

/// What a failed `git` printed, without progress lines and hints; `git failed` when it said
/// nothing.
fn failure(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("hint:"))
        .collect();
    if lines.is_empty() {
        "git failed".to_owned()
    } else {
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new repository in a scratch folder for one test, with a committer set.
    fn repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("macaroni-git-tab-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]).unwrap();
        git(&dir, &["config", "user.name", "t"]).unwrap();
        git(&dir, &["config", "user.email", "t@t"]).unwrap();
        git(&dir, &["config", "core.autocrlf", "false"]).unwrap();
        dir
    }

    #[test]
    fn reads_status_records() {
        let out = "# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0\
                   # branch.ab +2 -1\0\
                   1 M. N... 100644 100644 100644 a b src/a b.rs\0\
                   1 MM N... 100644 100644 100644 a b two.rs\0\
                   1 .D N... 100644 100644 000000 a b gone.rs\0\
                   2 R. N... 100644 100644 100644 a b R100 new.rs\0old.rs\0\
                   ? fresh.txt\0";
        let status = parse_status(out);
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.head.as_deref(), Some("abc"));
        assert_eq!((status.ahead, status.behind), (2, 1));
        let summary: Vec<_> = status
            .files
            .iter()
            .map(|c| (c.path.as_str(), c.from.as_deref(), c.kind, c.staged))
            .collect();
        assert_eq!(
            summary,
            [
                ("fresh.txt", None, ChangeKind::Added, Staged::None),
                ("gone.rs", None, ChangeKind::Deleted, Staged::None),
                ("new.rs", Some("old.rs"), ChangeKind::Renamed, Staged::All),
                ("src/a b.rs", None, ChangeKind::Modified, Staged::All),
                ("two.rs", None, ChangeKind::Modified, Staged::Some),
            ]
        );
        let fresh = parse_status("# branch.oid (initial)\0# branch.head (detached)\0");
        assert_eq!((fresh.head, fresh.branch), (None, None));
    }

    #[test]
    fn reads_a_diff_with_line_numbers() {
        let out = "diff --git a/x b/x\nindex 1..2 100644\n--- a/x\n+++ b/x\n\
                   @@ -3,3 +3,3 @@ fn x\n a\n-b\n+c\n\td\n\\ No newline at end of file\n";
        let diff = parse_diff(out);
        let lines: Vec<_> = diff
            .lines
            .iter()
            .map(|l| (l.kind, l.old, l.new, l.text.as_str()))
            .collect();
        assert_eq!(
            lines,
            [
                (LineKind::Hunk, None, None, "@@ -3,3 +3,3 @@ fn x"),
                (LineKind::Context, Some(3), Some(3), "a"),
                (LineKind::Removed, Some(4), None, "b"),
                (LineKind::Added, None, Some(4), "c"),
                (LineKind::Context, Some(5), Some(5), "    d"),
                (LineKind::Note, None, None, "No newline at end of file"),
            ]
        );
        assert!(parse_diff("Binary files a/x and b/x differ\n").binary);
    }

    #[test]
    fn reads_name_status_with_renames() {
        let files = parse_name_status("M\0a.rs\0R090\0old.rs\0new.rs\0D\0gone.rs\0");
        assert_eq!(files.len(), 3);
        assert_eq!(files[1].from.as_deref(), Some("old.rs"));
        assert_eq!(files[1].path, "new.rs");
        assert_eq!(files[2].kind, ChangeKind::Deleted);
    }

    #[test]
    fn stages_commits_and_discards_in_a_real_repository() {
        let dir = repo("flow");
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        let status = status(&dir).unwrap();
        assert_eq!(status.head, None);
        let change = &status.files[0];
        assert_eq!(
            (change.kind, change.staged),
            (ChangeKind::Added, Staged::None)
        );
        let diff = change_diff(&status.top, change, None).unwrap();
        assert_eq!(diff.lines.last().unwrap().text, "one");

        perform(&status.top, &Op::Stage(vec!["a.txt".into()]), false).unwrap();
        perform(&status.top, &Op::Unstage(vec!["a.txt".into()]), false).unwrap();
        perform(&status.top, &Op::Stage(vec!["a.txt".into()]), false).unwrap();
        perform(&status.top, &Op::Commit("First\n\nBody".into()), false).unwrap();

        std::fs::write(dir.join("a.txt"), "two\n").unwrap();
        std::fs::write(dir.join("b.txt"), "new\n").unwrap();
        let status = super::status(&dir).unwrap();
        assert!(status.head.is_some());
        assert_eq!(status.files.len(), 2);
        let commits = history(&status.top, &status).unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(
            (commits[0].summary.as_str(), commits[0].body.as_str()),
            ("First", "Body")
        );
        let files = commit_files(&status.top, &commits[0]).unwrap();
        assert_eq!(files[0].kind, ChangeKind::Added);
        let diff = commit_diff(&status.top, &commits[0], &files[0]).unwrap();
        assert!(
            diff.lines
                .iter()
                .any(|l| l.kind == LineKind::Added && l.text == "one")
        );

        for change in &status.files {
            perform(&status.top, &Op::Discard(change.clone()), true).unwrap();
        }
        assert!(super::status(&dir).unwrap().files.is_empty());
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "one\n");

        perform(&status.top, &Op::CreateBranch("feature".into()), true).unwrap();
        assert_eq!(branches(&status.top).unwrap(), ["feature", "main"]);
        assert!(perform(&status.top, &Op::Switch("nope".into()), true).is_err());
    }
}

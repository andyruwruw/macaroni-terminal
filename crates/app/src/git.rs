//! Git state for the sidebar: each workspace's branch (`{branch}`), whether its working tree
//! has uncommitted changes (`{dirty}`) and how far it is ahead of and behind its upstream
//! (`{sync}`), for the folder its focused tab is in.
//!
//! The branch is read straight from the repository's `HEAD` file, so it costs a few small reads
//! and needs no `git`; that also covers worktrees, whose `.git` is a file pointing at their own
//! `HEAD`. The rest takes one `git status`, so [`look`] runs off the UI thread, every
//! [`crate::GIT_TICK`] while a `[workspace_items]` template shows either placeholder.
//!
//! [`repo`] is what Git tabs show and do.
//!
//! [`add_worktree`] makes a worktree for a new workspace (the worktree dialog); it runs `git`
//! and blocks, so it's run off the UI thread too.

pub mod repo;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What the sidebar shows for one folder in a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    /// The checked-out branch, or the commit's first 7 hex digits when `HEAD` is detached.
    pub branch: String,
    /// Tracked files have uncommitted changes; false when `git` couldn't tell (not installed,
    /// say).
    pub dirty: bool,
    /// Commits on the branch its upstream doesn't have; 0 without an upstream.
    pub ahead: u32,
    /// Commits on the upstream the branch doesn't have; 0 without an upstream.
    pub behind: u32,
}

impl Repo {
    /// `{sync}`: `↑2↓1`, either part left out when it's 0, empty when both are.
    pub fn sync(&self) -> String {
        let mut out = String::new();
        if self.ahead > 0 {
            out.push_str(&format!("\u{2191}{}", self.ahead));
        }
        if self.behind > 0 {
            out.push_str(&format!("\u{2193}{}", self.behind));
        }
        out
    }
}

/// The repository state for each folder in `dirs`; folders outside a repository are left out.
/// Blocking: runs `git status` once per repository folder.
pub fn look(dirs: Vec<String>) -> HashMap<String, Repo> {
    dirs.into_iter()
        .filter_map(|dir| {
            let branch = branch(Path::new(&dir))?;
            let (dirty, ahead, behind) = status(Path::new(&dir));
            Some((
                dir,
                Repo {
                    branch,
                    dirty,
                    ahead,
                    behind,
                },
            ))
        })
        .collect()
}

/// The branch checked out in the repository holding `dir`, from its `HEAD`; `None` outside a
/// repository.
pub fn branch(dir: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git_dir(dir)?.join("HEAD")).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref:") {
        Some(reference) => {
            let reference = reference.trim();
            Some(
                reference
                    .strip_prefix("refs/heads/")
                    .unwrap_or(reference)
                    .to_owned(),
            )
        }
        None => Some(head.chars().take(7).collect()).filter(|s: &String| !s.is_empty()),
    }
}

/// The top folder of the repository holding `dir`: the nearest one with a `.git` (a folder, or
/// a worktree's file). Reads the file system only, so it's cheap enough for menus.
pub fn top(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|folder| folder.join(".git").exists())
        .map(Path::to_path_buf)
}

/// The git directory of the repository holding `dir`: the nearest `.git` folder, or for a
/// worktree or submodule the folder its `.git` file names (`gitdir: <path>`, relative to it).
fn git_dir(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().find_map(|folder| {
        let dot_git = folder.join(".git");
        if dot_git.is_dir() {
            return Some(dot_git);
        }
        let text = std::fs::read_to_string(&dot_git).ok()?;
        let target = text.trim().strip_prefix("gitdir:")?.trim();
        Some(folder.join(target))
    })
}

/// Whether tracked files in the repository holding `dir` have uncommitted changes, and how
/// many commits it's ahead and behind its upstream; all false and 0 when `git` can't tell.
/// Untracked files don't count: listing them is the slow part of `git status` in big trees.
fn status(dir: &Path) -> (bool, u32, u32) {
    let mut git = Command::new("git");
    git.arg("-C")
        .arg(dir)
        .args([
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=no",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    crate::vscode::hide_window(&mut git);
    match git.output() {
        Ok(out) if out.status.success() => parse_status(&String::from_utf8_lossy(&out.stdout)),
        _ => (false, 0, 0),
    }
}

/// Reads `git status --porcelain=v2 --branch`: header lines start with `#` (`# branch.ab +2 -1`
/// when there's an upstream), and every other line is a changed file.
fn parse_status(out: &str) -> (bool, u32, u32) {
    let mut ahead_behind = (0, 0);
    let mut dirty = false;
    for line in out.lines() {
        if let Some(counts) = line.strip_prefix("# branch.ab ") {
            let mut parts = counts.split_whitespace();
            let mut count = |sign: char| {
                parts
                    .next()
                    .and_then(|p| p.strip_prefix(sign)?.parse().ok())
                    .unwrap_or(0)
            };
            ahead_behind = (count('+'), count('-'));
        } else if !line.starts_with('#') && !line.trim().is_empty() {
            dirty = true;
        }
    }
    (dirty, ahead_behind.0, ahead_behind.1)
}

/// Adds a worktree for `branch` beside the repository holding `dir`: a sibling folder named
/// `<repository>-<branch>` (slashes in the branch become dashes). Checks out the branch if it
/// exists, else creates it from the current `HEAD`. Returns the new folder, or what `git` said
/// went wrong. Blocking.
pub fn add_worktree(dir: &Path, branch: &str) -> Result<PathBuf, String> {
    let branch = branch.trim();
    if branch.is_empty() {
        return Err("Name the branch.".to_owned());
    }
    let top = PathBuf::from(git(dir, &["rev-parse", "--show-toplevel"])?.trim());
    let name = top
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_owned());
    let folder = format!("{name}-{}", branch.replace(['/', '\\', ' '], "-"));
    let path = top
        .parent()
        .ok_or("The repository has no parent folder to put the worktree in.")?
        .join(folder);
    if path.exists() {
        return Err(format!("{} already exists.", path.display()));
    }
    let target = path.to_string_lossy().into_owned();
    let exists = git(
        &top,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok();
    if exists {
        git(&top, &["worktree", "add", &target, branch])?;
    } else {
        git(&top, &["worktree", "add", "-b", branch, &target])?;
    }
    Ok(path)
}

/// Runs `git -C dir <args>` without a console window; its output, or its error message (`git`'s
/// last stderr line) when it fails.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut git = Command::new("git");
    git.arg("-C").arg(dir).args(args).stdin(Stdio::null());
    crate::vscode::hide_window(&mut git);
    let out = git.output().map_err(|e| format!("git didn't run: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(err
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("git failed")
        .trim()
        .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch folder for one test, emptied first.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("macaroni-git-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_status_with_ahead_and_behind() {
        let out = "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n\
                   # branch.ab +2 -1\n1 .M N... 100644 100644 100644 a b src/x.rs\n";
        assert_eq!(parse_status(out), (true, 2, 1));
        assert_eq!(
            parse_status("# branch.oid abc\n# branch.head main\n"),
            (false, 0, 0)
        );
        let repo = Repo {
            branch: "main".into(),
            dirty: false,
            ahead: 2,
            behind: 0,
        };
        assert_eq!(repo.sync(), "\u{2191}2");
    }

    #[test]
    fn reads_the_branch_from_head_in_any_subfolder() {
        let repo = scratch("branch");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("src/deep")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        assert_eq!(branch(&repo.join("src/deep")).as_deref(), Some("feature/x"));

        std::fs::write(repo.join(".git/HEAD"), "0123456789abcdef\n").unwrap();
        assert_eq!(branch(&repo).as_deref(), Some("0123456"));
    }

    #[test]
    fn adds_a_worktree_beside_the_repository() {
        let root = scratch("add");
        let repo = root.join("proj");
        std::fs::create_dir_all(&repo).unwrap();
        let run = |args: &[&str]| git(&repo, args).unwrap();
        run(&["init", "-q", "-b", "main"]);
        run(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "x",
        ]);

        let path = add_worktree(&repo, "feature/x").unwrap();
        assert_eq!(path.file_name().unwrap(), "proj-feature-x");
        assert_eq!(branch(&path).as_deref(), Some("feature/x"));
        // The same branch again: its folder exists now.
        assert!(add_worktree(&repo, "feature/x").is_err());
        assert!(add_worktree(&repo, " ").is_err());
    }

    #[test]
    fn follows_a_worktrees_gitdir_file() {
        let root = scratch("worktree");
        let real = root.join("main/.git/worktrees/wt");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("HEAD"), "ref: refs/heads/wt-branch\n").unwrap();
        let worktree = root.join("wt");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(worktree.join(".git"), "gitdir: ../main/.git/worktrees/wt\n").unwrap();
        assert_eq!(branch(&worktree).as_deref(), Some("wt-branch"));
    }
}

//! The folder tree beside a file tab: the file's repository (or its folder) as folders that
//! open and close, the file marked. Clicking a file shows it in the tab; right-clicking one
//! opens the menu of places to open it (as Ctrl+right-click does in a terminal).
//!
//! Whether trees show is one switch for every file tab (`App.file_tree`, saved with the
//! session); each tab keeps its own [`Tree`]. Folders are read off the UI thread
//! ([`Request::run`]): opening the tree or the tab, or going to another file, lists the root
//! and opens the folders down to the file; the file tick lists again only the open folders
//! whose modified time changed, which adding, removing or renaming an entry updates.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use iced::widget::{Space, button, column, container, row};
use iced::{Element, Fill, Padding, Point, Task, alignment};
use macaroni_core::TabId;

use super::file_tab::{FileMessage, kind_icon};
use super::git_tab::{centered, tooltip_button};
use super::widgets::{flat_button, one_line, thin_scroll};
use crate::file_view::Kind;
use crate::icons::{Icon, icon};
use crate::message::{Clicked, FileLink};
use crate::state::MenuTarget;
use crate::{App, Message};

/// The tree's width.
const TREE_WIDTH: f32 = 240.0;

/// How far each level is indented.
const INDENT: f32 = 12.0;

/// The size of names.
const TEXT_SIZE: f32 = 12.5;

/// The size of the header and notes.
const SMALL_SIZE: f32 = 11.0;

/// The most entries listed for one folder; the rest are counted.
const MAX_ENTRIES: usize = 2000;

/// Names never listed.
const SKIPPED: &[&str] = &[".git"];

/// One file tab's tree.
#[derive(Debug, Default)]
pub struct Tree {
    /// The folder at the top, once found.
    root: Option<PathBuf>,
    /// The open folders.
    expanded: HashSet<PathBuf>,
    /// What each folder held when last listed.
    listed: HashMap<PathBuf, Listing>,
    /// A scan from the file tick or a read is running.
    loading: bool,
}

/// What a folder held.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    /// The folder's modified time when listed, compared on the tick.
    modified: Option<SystemTime>,
    /// Folders first, then files, each in natural order.
    entries: Vec<Entry>,
    /// Entries past [`MAX_ENTRIES`] left out.
    more: usize,
}

/// A folder's entry.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Its path.
    path: PathBuf,
    /// It's a folder (or a link to one).
    folder: bool,
}

/// What a scan found.
#[derive(Debug, Clone)]
pub struct Scan {
    /// The root the scan was asked about (`None`: find one), so a scan for a root the tree
    /// has since left is dropped.
    asked: Option<PathBuf>,
    /// The root.
    root: PathBuf,
    /// Folders to open so the file shows.
    reveal: Vec<PathBuf>,
    /// The folders listed.
    listings: Vec<(PathBuf, Listing)>,
}

/// What the tree sends, in `FileMessage::Tree`.
#[derive(Debug, Clone)]
pub enum TreeMessage {
    /// Show or hide trees in every file tab.
    Toggle,
    /// Open or close a folder.
    Fold(PathBuf),
    /// Make the root's parent the root.
    Up,
    /// A scan finished.
    Scanned(Box<Scan>),
    /// A file was right-clicked at this window position.
    Menu(PathBuf, Point),
}

/// What to scan, built on the UI thread and run off it.
struct Request {
    /// The tab's file.
    file: PathBuf,
    /// The root, or `None` to find it from the file.
    root: Option<PathBuf>,
    /// Folders to list besides the root.
    folders: Vec<PathBuf>,
    /// Modified times of folders already listed: one that still matches isn't listed again.
    known: HashMap<PathBuf, Option<SystemTime>>,
    /// Open the folders down to the file.
    reveal: bool,
}

impl Request {
    /// Finds the root if needed and lists what changed. Blocking.
    fn run(self) -> Scan {
        let root = self.root.clone().unwrap_or_else(|| root_of(&self.file));
        let reveal: Vec<PathBuf> = if self.reveal {
            self.file
                .ancestors()
                .skip(1)
                .take_while(|dir| dir.starts_with(&root))
                .map(Path::to_path_buf)
                .collect()
        } else {
            Vec::new()
        };
        let mut folders = self.folders;
        folders.push(root.clone());
        folders.extend(reveal.iter().cloned());
        folders.sort();
        folders.dedup();
        let listings = folders
            .into_iter()
            .filter_map(|dir| {
                let modified = std::fs::metadata(&dir).and_then(|m| m.modified()).ok();
                // A folder without a time is listed every time rather than never again.
                let same = modified.is_some() && self.known.get(&dir) == Some(&modified);
                (!same).then(|| {
                    let listing = list(&dir, modified);
                    (dir, listing)
                })
            })
            .collect();
        Scan {
            asked: self.root,
            root,
            reveal,
            listings,
        }
    }
}

/// The top of `file`'s tree: the nearest folder above it holding `.git` (a folder, or a file
/// in a worktree), else its own folder.
fn root_of(file: &Path) -> PathBuf {
    let folder = file.parent().unwrap_or(file);
    folder
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .unwrap_or(folder)
        .to_path_buf()
}

/// What `dir` holds, folders first, without [`SKIPPED`] names.
fn list(dir: &Path, modified: Option<SystemTime>) -> Listing {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Listing {
            modified,
            ..Listing::default()
        };
    };
    let mut entries: Vec<Entry> = read
        .flatten()
        .filter(|e| !SKIPPED.contains(&&*e.file_name().to_string_lossy()))
        .map(|e| {
            let path = e.path();
            Entry {
                folder: path.is_dir(),
                path,
            }
        })
        .collect();
    entries.sort_by_cached_key(|e| {
        let name = e.path.file_name().unwrap_or_default().to_string_lossy();
        (!e.folder, crate::file_view::natural_key(&name))
    });
    let more = entries.len().saturating_sub(MAX_ENTRIES);
    entries.truncate(MAX_ENTRIES);
    Listing {
        modified,
        entries,
        more,
    }
}

/// The last part of `path`, or all of it (a drive).
fn name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.to_string_lossy(), |n| n.to_string_lossy())
        .into_owned()
}

impl Tree {
    /// Forgets everything when `file` is outside the root, so the next scan finds its own.
    pub(super) fn follow(&mut self, file: &Path) {
        if self
            .root
            .as_ref()
            .is_some_and(|root| !file.starts_with(root))
        {
            *self = Self::default();
        }
    }
}

impl App {
    /// Applies a tree's message for file tab `tab`, showing `path`.
    pub(super) fn tree_update(
        &mut self,
        tab: TabId,
        path: &Path,
        message: TreeMessage,
    ) -> Task<Message> {
        let tree = &mut self.file_tabs.entry(tab).or_default().tree;
        match message {
            TreeMessage::Toggle => {
                self.file_tree = !self.file_tree;
                self.session_dirty = true;
                if self.file_tree {
                    return self.tree_scan(tab, path, true);
                }
                Task::none()
            }
            TreeMessage::Fold(dir) => {
                if tree.expanded.remove(&dir) {
                    return Task::none();
                }
                tree.expanded.insert(dir.clone());
                let request = Request {
                    file: path.to_path_buf(),
                    root: tree.root.clone(),
                    folders: vec![dir],
                    known: HashMap::new(),
                    reveal: false,
                };
                scanning(tab, request)
            }
            TreeMessage::Up => {
                let Some(root) = tree.root.take() else {
                    return Task::none();
                };
                let Some(parent) = root.parent().map(Path::to_path_buf) else {
                    tree.root = Some(root);
                    return Task::none();
                };
                tree.expanded.insert(root);
                tree.root = Some(parent);
                let request = Request {
                    file: path.to_path_buf(),
                    root: tree.root.clone(),
                    folders: Vec::new(),
                    known: HashMap::new(),
                    reveal: false,
                };
                scanning(tab, request)
            }
            TreeMessage::Scanned(scan) => {
                tree.loading = false;
                if scan.asked != tree.root {
                    return Task::none();
                }
                tree.root = Some(scan.root);
                tree.expanded.extend(scan.reveal);
                tree.listed.extend(scan.listings);
                Task::none()
            }
            TreeMessage::Menu(file, at) => {
                self.clicked = Some(Clicked::File(FileLink {
                    path: file,
                    line: None,
                    column: None,
                }));
                self.menu_open(MenuTarget::Link, at)
            }
        }
    }

    /// Lists file tab `tab`'s tree again when trees show: everything, opening the folders
    /// down to `path` (`force`: opening, reload, another file), or only the open folders that
    /// changed (the tick).
    pub(super) fn tree_scan(&mut self, tab: TabId, path: &Path, force: bool) -> Task<Message> {
        if !self.file_tree {
            return Task::none();
        }
        let tree = &mut self.file_tabs.entry(tab).or_default().tree;
        if tree.loading && !force {
            return Task::none();
        }
        tree.loading = true;
        let mut folders: Vec<PathBuf> = tree.expanded.iter().cloned().collect();
        folders.extend(tree.root.clone());
        let known = if force {
            HashMap::new()
        } else {
            folders
                .iter()
                .filter_map(|dir| Some((dir.clone(), tree.listed.get(dir)?.modified)))
                .collect()
        };
        let request = Request {
            file: path.to_path_buf(),
            root: tree.root.clone(),
            folders,
            known,
            reveal: force || tree.root.is_none(),
        };
        scanning(tab, request)
    }

    /// File tab `tab`'s tree, showing `path` as the current file.
    pub(super) fn file_tree_view<'a>(&self, tab: TabId, path: &Path) -> Element<'a, Message> {
        let c = self.chrome;
        let tree = self.file_tabs.get(&tab).map(|s| &s.tree);
        let content: Element<'a, Message> = match tree.and_then(|t| Some((t, t.root.as_ref()?))) {
            None => centered(c, "Reading the folder..."),
            Some((tree, root)) => {
                let mut rows = Vec::new();
                self.tree_rows(&mut rows, tab, path, tree, root, 0);
                let up = root
                    .parent()
                    .map(|_| Message::File(tab, FileMessage::Tree(TreeMessage::Up)));
                let header = row![
                    container(one_line(name(root), SMALL_SIZE, c.muted))
                        .width(Fill)
                        .clip(true),
                    tooltip_button(c, Icon::Up, "Show the folder above", up),
                ]
                .spacing(4)
                .padding([2, 4])
                .align_y(alignment::Vertical::Center);
                column![
                    header,
                    thin_scroll(c, column(rows).padding([0, 4])).height(Fill)
                ]
                .into()
            }
        };
        container(content)
            .width(TREE_WIDTH)
            .height(Fill)
            .padding([4, 0])
            .into()
    }

    /// Pushes a row for each entry of `dir` onto `rows`, and the rows of each open folder
    /// after its own, indented by `depth`.
    fn tree_rows<'a>(
        &self,
        rows: &mut Vec<Element<'a, Message>>,
        tab: TabId,
        current: &Path,
        tree: &Tree,
        dir: &Path,
        depth: usize,
    ) {
        let c = self.chrome;
        let indent = Padding::ZERO.left(depth as f32 * INDENT);
        let Some(listing) = tree.listed.get(dir) else {
            rows.push(
                container(one_line("...", SMALL_SIZE, c.faint))
                    .padding(indent.left(depth as f32 * INDENT + 28.0))
                    .into(),
            );
            return;
        };
        for entry in &listing.entries {
            let open = entry.folder && tree.expanded.contains(&entry.path);
            let lead: Element<'a, Message> = if entry.folder {
                icon(
                    if open {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    },
                    12.0,
                    c.muted,
                )
                .into()
            } else {
                Space::new().width(12).into()
            };
            let glyph = if entry.folder {
                Icon::Folder
            } else {
                kind_icon(Kind::of(&entry.path))
            };
            let selected = entry.path == current;
            let message = if entry.folder {
                FileMessage::Tree(TreeMessage::Fold(entry.path.clone()))
            } else {
                FileMessage::Go(entry.path.clone())
            };
            let line = button(
                row![
                    lead,
                    icon(glyph, 13.0, c.label),
                    one_line(name(&entry.path), TEXT_SIZE, c.text),
                ]
                .spacing(6)
                .padding(indent)
                .align_y(alignment::Vertical::Center),
            )
            .on_press(Message::File(tab, message))
            .width(Fill)
            .padding([3, 6])
            .style(move |_, s| flat_button(c, s, selected.then_some(c.raised)));
            rows.push(if entry.folder {
                line.into()
            } else {
                let file = entry.path.clone();
                crate::right_click::RightClick::new(line, move |at| {
                    Message::File(tab, FileMessage::Tree(TreeMessage::Menu(file.clone(), at)))
                })
                .into()
            });
            if open {
                self.tree_rows(rows, tab, current, tree, &entry.path, depth + 1);
            }
        }
        if listing.more > 0 {
            rows.push(
                container(one_line(
                    format!("{} more", listing.more),
                    SMALL_SIZE,
                    c.faint,
                ))
                .padding(indent.left(depth as f32 * INDENT + 28.0))
                .into(),
            );
        }
    }
}

/// Runs `request` off the UI thread, reporting to file tab `tab`.
fn scanning(tab: TabId, request: Request) -> Task<Message> {
    Task::perform(
        crate::vscode::off_thread(move || request.run()),
        move |scan| Message::File(tab, FileMessage::Tree(TreeMessage::Scanned(Box::new(scan)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_find_the_repository_and_open_the_way_to_the_file() {
        let base = std::env::temp_dir().join(format!("macaroni-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let deep = base.join("repo").join("src").join("view");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(base.join("repo").join(".git")).unwrap();
        std::fs::create_dir_all(base.join("repo").join("docs")).unwrap();
        std::fs::write(base.join("repo").join("README.md"), "").unwrap();
        let file = deep.join("tab.rs");
        std::fs::write(&file, "").unwrap();
        let root = base.join("repo");
        assert_eq!(root_of(&file), root);
        assert_eq!(root_of(&base.join("loose.txt")), base);

        let scan = Request {
            file: file.clone(),
            root: None,
            folders: Vec::new(),
            known: HashMap::new(),
            reveal: true,
        }
        .run();
        assert_eq!(scan.root, root);
        assert_eq!(scan.reveal, [deep.clone(), root.join("src"), root.clone()]);
        let top = &scan.listings.iter().find(|(d, _)| *d == root).unwrap().1;
        let names: Vec<String> = top.entries.iter().map(|e| name(&e.path)).collect();
        assert_eq!(names, ["docs", "src", "README.md"]);

        // Unchanged folders aren't listed again.
        let known = scan
            .listings
            .iter()
            .map(|(d, l)| (d.clone(), l.modified))
            .collect();
        let again = Request {
            file,
            root: Some(root),
            folders: vec![deep],
            known,
            reveal: false,
        }
        .run();
        assert!(again.listings.is_empty());
        std::fs::remove_dir_all(&base).unwrap();
    }
}

//! Viewer extensions: opt-in downloads that let file tabs draw what plain Markdown can't
//! (Mermaid, math, Graphviz, PlantUML and D2).
//!
//! Each [`Extension`] is one npm package, pinned to a version and its SHA-256, fetched from the
//! npm registry with the system's `curl` when the user asks (Settings > Files), checked,
//! and unpacked (only the files the page loads, and the license) with the system's `tar` into
//! its folder under [`dir`]. A `package.txt` naming the tarball marks it installed, so a later
//! macaroni that pins another version offers the download again.
//!
//! The scripts run in the file tab's web page ([`crate::preview`]), which loads each one only
//! when the document has something for it.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::vscode::{off_thread, run};

/// The file in an extension's folder naming the tarball it came from.
const PACKAGE_FILE: &str = "package.txt";

/// One downloadable viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Extension {
    /// Mermaid diagrams (`mermaid.js`).
    Mermaid,
    /// TeX math (`KaTeX`).
    Math,
    /// Graphviz's DOT language (`viz.js`, Graphviz compiled to WebAssembly).
    Graphviz,
    /// PlantUML (the official engine compiled to JavaScript).
    PlantUml,
    /// D2 diagrams (the official `d2.js`, D2 compiled to WebAssembly).
    D2,
}

/// An npm package pinned for download.
struct Package {
    /// The tarball on the npm registry.
    url: &'static str,
    /// The tarball's SHA-256, in hex.
    sha256: &'static str,
    /// Its download size, for the settings row.
    megabytes: u32,
    /// What's kept from it, as paths inside the tarball (whose top folder `package/` is dropped
    /// when it's unpacked).
    files: &'static [&'static str],
}

impl Extension {
    /// Every extension, in the order settings lists them.
    pub const ALL: [Self; 5] = [
        Self::Mermaid,
        Self::Math,
        Self::Graphviz,
        Self::PlantUml,
        Self::D2,
    ];

    /// The extension's folder name, and its name in the page's script.
    pub fn id(self) -> &'static str {
        match self {
            Self::Mermaid => "mermaid",
            Self::Math => "math",
            Self::Graphviz => "graphviz",
            Self::PlantUml => "plantuml",
            Self::D2 => "d2",
        }
    }

    /// The extension's name in settings and notes.
    pub fn name(self) -> &'static str {
        match self {
            Self::Mermaid => "Mermaid",
            Self::Math => "Math",
            Self::Graphviz => "Graphviz",
            Self::PlantUml => "PlantUML",
            Self::D2 => "D2",
        }
    }

    /// What it draws, for its settings row.
    pub fn about(self) -> &'static str {
        match self {
            Self::Mermaid => {
                "Flowcharts, sequence, class, state and other diagrams in ```mermaid blocks and \
                 .mmd files, drawn by mermaid.js"
            }
            Self::Math => "TeX math between $ and $$ and in ```math blocks, typeset by KaTeX",
            Self::Graphviz => {
                "Graphs in ```dot blocks and .dot and .gv files, laid out by Graphviz (viz.js)"
            }
            Self::PlantUml => {
                "UML and other diagrams in ```plantuml blocks and .puml files, drawn by \
                 PlantUML's own engine"
            }
            Self::D2 => "Diagrams in ```d2 blocks and .d2 files, drawn by D2's own engine",
        }
    }

    /// The pinned package.
    fn package(self) -> Package {
        match self {
            Self::Mermaid => Package {
                url: "https://registry.npmjs.org/mermaid/-/mermaid-12.3.0.tgz",
                sha256: "be0684e8d20bfc874df12c0f04927a5c482f57f2200eec54cca52a4583fd4040",
                megabytes: 26,
                files: &["package/dist/mermaid.min.js", "package/LICENSE"],
            },
            Self::Math => Package {
                url: "https://registry.npmjs.org/katex/-/katex-0.19.1.tgz",
                sha256: "24da5a599eb2fcede2e270b99a6ff48bca816beecc95feeb84e8cccbd1bb34fd",
                megabytes: 2,
                files: &[
                    "package/dist/katex.min.js",
                    "package/dist/katex.min.css",
                    "package/dist/fonts",
                    "package/LICENSE",
                ],
            },
            Self::Graphviz => Package {
                url: "https://registry.npmjs.org/@viz-js/viz/-/viz-3.31.0.tgz",
                sha256: "7dc0189874849cca2fed7cbe88bf334cd0e6e012db16657794a812ad6f2b8c70",
                megabytes: 2,
                files: &["package/dist/viz-global.js"],
            },
            Self::PlantUml => Package {
                url: "https://registry.npmjs.org/@plantuml/core/-/core-1.2026.9.tgz",
                sha256: "ea9e28ffb7aeb3eb20691cb5803b55a67ac0be869eb7dcdd9349b2ec2bd71f70",
                megabytes: 3,
                files: &[
                    "package/plantuml.js",
                    "package/viz-global.js",
                    "package/themes.js",
                    "package/LICENSE",
                ],
            },
            Self::D2 => Package {
                url: "https://registry.npmjs.org/@d2lang/d2/-/d2-0.1.34.tgz",
                sha256: "3256bd0f8865e749175c85d013b461c9569fcfd798ff1f9ca1a4f89d29feb647",
                megabytes: 32,
                files: &[
                    "package/dist/browser/index.js",
                    "package/LICENSE.txt",
                    "package/THIRD_PARTY_NOTICES.txt",
                ],
            },
        }
    }

    /// The download's size in megabytes, for its settings row.
    pub fn megabytes(self) -> u32 {
        self.package().megabytes
    }

    /// The extension that draws a fenced code block in `language` (its info string's first
    /// word, any case), if any. Math isn't here: its blocks are typeset whether or not it's
    /// installed, and only look like code without it.
    pub fn for_language(language: &str) -> Option<Self> {
        match language.to_ascii_lowercase().as_str() {
            "mermaid" => Some(Self::Mermaid),
            "dot" | "graphviz" | "gv" => Some(Self::Graphviz),
            "plantuml" | "puml" => Some(Self::PlantUml),
            "d2" => Some(Self::D2),
            _ => None,
        }
    }

    /// The extension that draws a whole file with extension `extension` (any case), if any.
    pub fn for_file(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "mmd" | "mermaid" => Some(Self::Mermaid),
            "dot" | "gv" => Some(Self::Graphviz),
            "puml" | "plantuml" | "pu" => Some(Self::PlantUml),
            "d2" => Some(Self::D2),
            _ => None,
        }
    }

    /// Where it's unpacked.
    pub fn folder(self) -> PathBuf {
        dir().join(self.id())
    }

    /// Whether this build's version of it is there.
    pub fn installed(self) -> bool {
        std::fs::read_to_string(self.folder().join(PACKAGE_FILE))
            .is_ok_and(|url| url.trim() == self.package().url)
    }
}

/// The extensions' folder: `viewers` beside the browser profile (`%LOCALAPPDATA%\macaroni\viewers`,
/// or `<MACARONI_CONFIG_DIR>/viewers`).
pub fn dir() -> PathBuf {
    crate::browser::local_data("viewers")
}

/// A program that ships with the OS: Windows' own `curl` and `tar` in System32 (Git's GNU
/// `tar` on `PATH` reads `C:` as a remote host), else the one on `PATH`.
fn tool(name: &str) -> PathBuf {
    if cfg!(windows) {
        crate::vscode::system_tool(&format!("{name}.exe"))
    } else {
        PathBuf::from(name)
    }
}

/// Downloads, checks and unpacks `extension` into its folder, replacing any earlier copy only
/// once the new one is complete. Blocking: run it off the UI thread ([`off_thread`]).
pub fn install(extension: Extension) -> Result<(), String> {
    let package = extension.package();
    let dir = dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let download = dir.join(format!("{}.tgz", extension.id()));
    let partial = dir.join(format!("{}.partial", extension.id()));
    let _ = std::fs::remove_dir_all(&partial);
    let result = fetch(&package, &download, &partial).and_then(|()| {
        remove(extension)?;
        std::fs::rename(&partial, extension.folder())
            .map_err(|e| format!("{}: {e}", extension.folder().display()))
    });
    let _ = std::fs::remove_file(&download);
    let _ = std::fs::remove_dir_all(&partial);
    result
}

/// The download, check and unpacking for [`install`], into `folder`.
fn fetch(package: &Package, download: &Path, folder: &Path) -> Result<(), String> {
    let mut curl = Command::new(tool("curl"));
    curl.args(["-fsSL", "--retry", "2", "-o"])
        .arg(download)
        .arg(package.url);
    run(curl, "Download")?;
    let actual = sha256(download)?;
    if !actual.eq_ignore_ascii_case(package.sha256) {
        return Err(format!(
            "the download is damaged (SHA-256 {actual}, expected {})",
            package.sha256
        ));
    }
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    // Both bsdtar (Windows, macOS) and GNU tar take a folder's name to mean everything in it,
    // and fail when a name isn't in the archive, which catches a package laid out differently.
    let mut tar = Command::new(tool("tar"));
    tar.arg("-xzf")
        .arg(download)
        .arg("-C")
        .arg(folder)
        .args(["--strip-components", "1"])
        .args(package.files);
    run(tar, "Unpacking")?;
    std::fs::write(folder.join(PACKAGE_FILE), package.url)
        .map_err(|e| format!("{}: {e}", folder.display()))
}

/// `path`'s SHA-256 in lowercase hex.
fn sha256(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("Checking the download: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("Checking the download: {e}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Deletes `extension`'s folder. Pages load its files over HTTP, so nothing holds them open.
/// Blocking.
pub fn remove(extension: Extension) -> Result<(), String> {
    let folder = extension.folder();
    match std::fs::remove_dir_all(&folder) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("{}: {e}", folder.display()))
        }
        _ => Ok(()),
    }
}

/// The app's state for the downloads, shown in settings.
#[derive(Debug, Default)]
pub struct Viewers {
    /// The extensions this build's version of is there, checked at launch and after each
    /// download or removal.
    pub installed: BTreeSet<Extension>,
    /// Those being downloaded or removed.
    pub busy: BTreeSet<Extension>,
    /// What each one's last download or removal reported.
    pub errors: BTreeMap<Extension, String>,
}

impl Viewers {
    /// The state at launch.
    pub fn new() -> Self {
        Self {
            installed: Extension::ALL
                .into_iter()
                .filter(|e| e.installed())
                .collect(),
            ..Self::default()
        }
    }

    /// Downloads (`download`) or removes `extension`, off the UI thread; the result says whether
    /// it's installed now, or why it failed.
    pub fn work(
        extension: Extension,
        download: bool,
    ) -> impl Future<Output = Result<bool, String>> {
        off_thread(move || {
            if download {
                install(extension).map(|()| true)
            } else {
                remove(extension).map(|()| false)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_and_files_find_their_extension() {
        assert_eq!(Extension::for_language("Mermaid"), Some(Extension::Mermaid));
        assert_eq!(Extension::for_language("dot"), Some(Extension::Graphviz));
        assert_eq!(Extension::for_language("puml"), Some(Extension::PlantUml));
        assert_eq!(Extension::for_language("d2"), Some(Extension::D2));
        assert_eq!(Extension::for_language("rust"), None);
        assert_eq!(Extension::for_language("math"), None);
        assert_eq!(Extension::for_file("MMD"), Some(Extension::Mermaid));
        assert_eq!(Extension::for_file("gv"), Some(Extension::Graphviz));
        assert_eq!(Extension::for_file("md"), None);
    }

    #[test]
    fn every_package_is_pinned_from_npm() {
        for extension in Extension::ALL {
            let package = extension.package();
            assert!(package.url.starts_with("https://registry.npmjs.org/"));
            assert_eq!(package.sha256.len(), 64);
            assert!(package.files.iter().all(|f| f.starts_with("package/")));
        }
    }

    #[test]
    fn sha256_matches_a_known_digest() {
        let file = std::env::temp_dir().join(format!("macaroni-sha-{}", std::process::id()));
        std::fs::write(&file, b"abc").unwrap();
        let digest = sha256(&file).unwrap();
        std::fs::remove_file(&file).unwrap();
        assert_eq!(
            digest,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}

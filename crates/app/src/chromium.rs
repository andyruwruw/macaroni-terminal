//! Chromium for browser tabs: an opt-in download of the exact CEF runtime this build links
//! against, so the app itself ships without it.
//!
//! Nothing is downloaded until the user asks (Settings > Integrations > Browser, which calls
//! [`install`]): CEF's "minimal" distribution ([`macaroni_browser::CHROMIUM_ARCHIVE`], about
//! 175 MB) comes from Spotify's CEF CDN with the system's `curl`, is checked against the SHA-1
//! the build recorded, and its runtime (`Release` and `Resources`, flattened) is unpacked into
//! [`dir`] with the system's `tar`. A [`macaroni_browser::CHROMIUM_VERSION_FILE`] names the
//! archive, so a later macaroni built against another CEF asks for a new download instead of
//! loading a mismatched one.
//!
//! [`Chromium`] is the app's side: whether it's there, and the download in progress. Windows
//! only, so [`available`] is false elsewhere.

use std::path::PathBuf;
use std::process::Command;

use crate::vscode::{off_thread, run, system_tool};

/// Where CEF's builds are published.
const CDN: &str = "https://cef-builds.spotifycdn.com";

/// Whether Chromium can be downloaded on this platform (and by this build).
pub fn available() -> bool {
    cfg!(windows) && !archive().0.is_empty()
}

/// The archive this build needs and its SHA-1; empty off Windows.
fn archive() -> (&'static str, &'static str) {
    #[cfg(windows)]
    return (
        macaroni_browser::CHROMIUM_ARCHIVE,
        macaroni_browser::CHROMIUM_SHA1,
    );
    #[cfg(not(windows))]
    ("", "")
}

/// The runtime's folder: `chromium` beside the browser profile (`%LOCALAPPDATA%\macaroni\chromium`,
/// or `<MACARONI_CONFIG_DIR>/chromium`).
pub fn dir() -> PathBuf {
    crate::browser::local_data("chromium")
}

/// Whether a runtime Chromium can start from is there: this build's download, or the files a
/// development build has next to the executable.
pub fn installed() -> bool {
    macaroni_browser::chromium_available(&dir())
}

/// Downloads, checks and unpacks the runtime into [`dir`], replacing any earlier copy.
/// Blocking: run it off the UI thread ([`off_thread`]).
pub fn install() -> Result<(), String> {
    let (name, sha1) = archive();
    if name.is_empty() {
        return Err("Chromium isn't available on this platform".to_owned());
    }
    remove()?;
    let dir = dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let download = dir.join("download.tar.bz2");
    let result = fetch(name, sha1, &download, &dir);
    let _ = std::fs::remove_file(&download);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    result
}

/// The download, check and unpacking for [`install`].
fn fetch(
    name: &str,
    sha1: &str,
    download: &std::path::Path,
    dir: &std::path::Path,
) -> Result<(), String> {
    // `+` in the build's name would be read as a space in a URL path.
    let url = format!("{CDN}/{}", name.replace('+', "%2B"));
    let mut curl = Command::new(system_tool("curl.exe"));
    curl.args(["-fsSL", "--retry", "2", "-o"])
        .arg(download)
        .arg(&url);
    run(curl, "Download")?;
    let actual = file_sha1(download)?;
    if !actual.eq_ignore_ascii_case(sha1) {
        return Err(format!(
            "the download is damaged (SHA-1 {actual}, expected {sha1})"
        ));
    }
    // The archive holds one folder (named like the file) with `Release` (libraries) and
    // `Resources` (paks, locales); both are flattened into `dir`, as Chromium expects. The
    // import library and the sandbox bootstrap aren't needed to run it.
    let root = name.trim_end_matches(".tar.bz2");
    let mut tar = Command::new(system_tool("tar.exe"));
    tar.arg("-xjf")
        .arg(download)
        .arg("-C")
        .arg(dir)
        .args([
            "--strip-components",
            "2",
            "--exclude",
            "*.lib",
            "--exclude",
            "bootstrap*.exe",
        ])
        .arg(format!("{root}/Release"))
        .arg(format!("{root}/Resources"));
    run(tar, "Unpacking")?;
    std::fs::write(dir.join(macaroni_browser::CHROMIUM_VERSION_FILE), name)
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    if installed() {
        Ok(())
    } else {
        Err("the download didn't contain Chromium".to_owned())
    }
}

/// `path`'s SHA-1 in hex, from Windows' `certutil`, whose second output line is the hash.
fn file_sha1(path: &std::path::Path) -> Result<String, String> {
    let mut certutil = Command::new(system_tool("certutil.exe"));
    certutil.arg("-hashfile").arg(path).arg("SHA1");
    crate::vscode::hide_window(&mut certutil);
    let out = certutil
        .output()
        .map_err(|e| format!("Checking the download: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .nth(1)
        .map(|line| line.split_whitespace().collect::<String>())
        .filter(|hash| out.status.success() && hash.len() == 40)
        .ok_or_else(|| "Checking the download failed".to_owned())
}

/// Deletes [`dir`]. Fails while this process has Chromium loaded: its files stay in use until
/// macaroni quits. Blocking.
pub fn remove() -> Result<(), String> {
    if macaroni_browser::chromium_loaded() {
        return Err(
            "Chromium is in use. Set the engine to System and restart macaroni first.".to_owned(),
        );
    }
    match std::fs::remove_dir_all(dir()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("{}: {e}", dir().display()))
        }
        _ => Ok(()),
    }
}

/// The app's state for the download: shown in the settings row.
#[derive(Debug, Default)]
pub struct Chromium {
    /// Whether a runtime is there, checked at launch and after downloading or removing it.
    pub installed: bool,
    /// Set while it's being downloaded or removed.
    pub busy: bool,
    /// What the last download or removal reported.
    pub error: Option<String>,
}

impl Chromium {
    /// The state at launch.
    pub fn new() -> Self {
        Self {
            installed: installed(),
            ..Self::default()
        }
    }

    /// Downloads (`download`) or removes the runtime, off the UI thread; the result says whether
    /// it's installed now, or why it failed.
    pub fn work(download: bool) -> impl Future<Output = Result<bool, String>> {
        off_thread(move || {
            if download {
                install().map(|()| true)
            } else {
                remove().map(|()| false)
            }
        })
    }
}

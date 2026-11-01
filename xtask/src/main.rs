//! Project automation: `cargo xtask <task>`.
//!
//! - `bundle`: release build packaged for the current OS. macOS gets `macaroni.app` (macOS only
//!   delivers notifications for bundled apps); Windows and Linux get a folder with the executable,
//!   the bundled ConPTY on Windows, and third-party licenses. `macaroni-cli` ships alongside.
//!   Chromium's runtime isn't bundled: the app downloads it when asked.
//! - `dev`: debug build, rebuilt and restarted on every save (see `dev.rs`).
//! - `install`: `bundle`, then install it as the everyday macaroni and start it (see
//!   `install.rs`).

/// `cargo xtask dev`: the rebuild-and-restart loop.
mod dev;
/// `cargo xtask install`: install the bundle and start it.
mod install;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// The macOS bundle identifier. macOS ties notification permission to it, so it must stay
/// stable across releases.
const BUNDLE_ID: &str = "dev.macaroni.terminal";

/// Runs the task named by the first argument; prints usage and fails for anything else.
fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let result = match args.next().as_deref() {
        Some("bundle") => bundle().map(|path| println!("bundled: {}", path.display())),
        Some("dev") => dev::run(&root()),
        Some("install") => install::run(&root(), args),
        _ => {
            eprintln!("usage: cargo xtask bundle | dev | install [--no-build | --uninstall]");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xtask: {err}");
            ExitCode::FAILURE
        }
    }
}

/// The workspace root (the parent of this crate's manifest directory).
///
/// # Panics
///
/// Only if `xtask` were moved to the filesystem root, which would leave it without a parent.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace root")
        .to_path_buf()
}

/// `cargo`, with Visual Studio's Ninja on `PATH` when none is there: CEF's C++ wrapper is built
/// with CMake and Ninja, and Visual Studio ships both but only puts them on `PATH` in its own
/// developer shells.
fn cargo() -> Command {
    let mut command = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    if cfg!(windows)
        && !on_path("ninja.exe")
        && let Some(ninja) = visual_studio_ninja()
    {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let paths = std::iter::once(ninja).chain(std::env::split_paths(&path));
        if let Ok(path) = std::env::join_paths(paths) {
            command.env("PATH", path);
        }
    }
    command
}

/// Whether `program` (with its extension) is a file in some `PATH` directory.
fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// The directory holding the newest Visual Studio's bundled `ninja.exe`, found with
/// `vswhere`; `None` without Visual Studio or without its CMake component. Runs `vswhere.exe`.
fn visual_studio_ninja() -> Option<PathBuf> {
    let vswhere = r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe";
    let output = Command::new(vswhere)
        .args(["-latest", "-products", "*", "-property", "installationPath"])
        .output()
        .ok()?;
    let install = String::from_utf8(output.stdout).ok()?;
    let ninja =
        Path::new(install.trim()).join(r"Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja");
    ninja.join("ninja.exe").is_file().then_some(ninja)
}

/// `cargo xtask bundle`: builds `macaroni` and `macaroni-cli` in release, then lays out the
/// package and returns its path: `target/release/bundle/macaroni.app` on macOS, else
/// `target/release/dist/macaroni`. The output directory is deleted and recreated each time.
fn bundle() -> Result<PathBuf, String> {
    let root = root();
    let status = cargo()
        .current_dir(&root)
        .args([
            "build",
            "--release",
            "--package",
            "macaroni",
            "--package",
            "macaroni-cli",
        ])
        .status()
        .map_err(|e| format!("running cargo: {e}"))?;
    if !status.success() {
        return Err("release build failed".into());
    }

    let release = root.join("target").join("release");
    let out = bundle_path(&root);
    let (exe, cli) = if cfg!(windows) {
        ("macaroni.exe", "macaroni-cli.exe")
    } else {
        ("macaroni", "macaroni-cli")
    };

    if cfg!(target_os = "macos") {
        let contents = out.join("Contents");
        recreate(&out)?;
        fs::create_dir_all(contents.join("MacOS")).map_err(io)?;
        fs::create_dir_all(contents.join("Resources")).map_err(io)?;
        copy(&release.join(exe), &contents.join("MacOS").join(exe))?;
        copy(&release.join(cli), &contents.join("MacOS").join(cli))?;
        fs::write(contents.join("Info.plist"), info_plist()).map_err(io)?;
        copy_licenses(&root, &contents.join("Resources"))?;
    } else {
        recreate(&out)?;
        copy(&release.join(exe), &out.join(exe))?;
        copy(&release.join(cli), &out.join(cli))?;
        if cfg!(windows) {
            // build.rs already placed these next to the release executable.
            for file in ["conpty.dll", "OpenConsole.exe"] {
                copy(&release.join(file), &out.join(file))?;
            }
        }
        copy_licenses(&root, &out)?;
    }
    Ok(out)
}

/// Where [`bundle`] puts the package: `target/release/bundle/macaroni.app` on macOS, else
/// `target/release/dist/macaroni`.
fn bundle_path(root: &Path) -> PathBuf {
    let release = root.join("target").join("release");
    if cfg!(target_os = "macos") {
        release.join("bundle").join("macaroni.app")
    } else {
        release.join("dist").join("macaroni")
    }
}

/// The app bundle's `Info.plist`, versioned from this crate's version (the workspace version).
fn info_plist() -> String {
    // macOS wants plain dotted numbers here, so a pre-release (`-alpha.1`) is left off.
    let version = env!("CARGO_PKG_VERSION")
        .split('-')
        .next()
        .unwrap_or(env!("CARGO_PKG_VERSION"));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>macaroni</string>
    <key>CFBundleDisplayName</key>
    <string>macaroni</string>
    <key>CFBundleIdentifier</key>
    <string>{BUNDLE_ID}</string>
    <key>CFBundleExecutable</key>
    <string>macaroni</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
"#
    )
}

/// Copies macaroni's license and every bundled third-party license (fonts, icons, and on
/// Windows ConPTY and Chromium) into `<dest>/licenses`.
fn copy_licenses(root: &Path, dest: &Path) -> Result<(), String> {
    let licenses = dest.join("licenses");
    fs::create_dir_all(&licenses).map_err(io)?;
    copy(&root.join("LICENSE"), &licenses.join("macaroni-LICENSE"))?;
    let fonts = root.join("crates/app/assets/fonts");
    for name in ["JetBrainsMono-OFL.txt", "SymbolsNerdFont-LICENSE"] {
        copy(&fonts.join(name), &licenses.join(name))?;
    }
    copy(
        &root.join("crates/app/assets/icons/LICENSE"),
        &licenses.join("Lucide-LICENSE"),
    )?;
    if cfg!(windows) {
        copy(
            &root.join("vendor/conpty/README.md"),
            &licenses.join("ConPTY-README.md"),
        )?;
        copy(
            &cef_dir(root)?.join("CREDITS.html"),
            &licenses.join("Chromium-CREDITS.html"),
        )?;
    }
    Ok(())
}

/// The CEF distribution the build used: `<CEF_PATH>/<version>/cef_windows_<arch>`, found by its
/// `CREDITS.html`. `CEF_PATH` is `target/cef` unless overridden (CI keeps it outside `target/`).
fn cef_dir(root: &Path) -> Result<PathBuf, String> {
    let base = std::env::var_os("CEF_PATH")
        .map_or_else(|| root.join("target").join("cef"), |path| root.join(path));
    fs::read_dir(&base)
        .map_err(|e| format!("reading {}: {e}", base.display()))?
        .filter_map(|e| e.ok())
        .flat_map(|version| fs::read_dir(version.path()).into_iter().flatten())
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.join("CREDITS.html").is_file())
        .ok_or_else(|| format!("no CEF distribution under {}", base.display()))
}

/// Deletes `dir` if it exists and creates it empty, so a bundle never keeps stale files.
fn recreate(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        fs::remove_dir_all(dir).map_err(|e| format!("clearing {}: {e}", dir.display()))?;
    }
    fs::create_dir_all(dir).map_err(io)
}

/// Copies one file, with both paths in the error.
fn copy(from: &Path, to: &Path) -> Result<(), String> {
    fs::copy(from, to)
        .map(drop)
        .map_err(|e| format!("copying {} to {}: {e}", from.display(), to.display()))
}

/// An I/O error as this tool's error type, for `map_err`.
fn io(e: std::io::Error) -> String {
    e.to_string()
}

//! Copies the CEF runtime (libcef.dll, resources, locales) next to the executables, where
//! Chromium looks for it. Files already there with the same size are skipped.
//!
//! `cef-dll-sys` downloads the CEF distribution (into `CEF_PATH`, set to `target/cef` by
//! `.cargo/config.toml`) and exports its directory as `DEP_CEF_DLL_WRAPPER_CEF_DIR`, which only
//! reaches this script because `cef-dll-sys` is a direct dependency. Off Windows it isn't, so the
//! script does nothing there. `cargo xtask bundle` copies the same files into the release.

use std::path::{Path, PathBuf};

/// Finds the CEF runtime and the profile directory, then copies. A failed copy is a warning, not
/// an error, so a locked `libcef.dll` (a running macaroni) doesn't break the build.
///
/// # Panics
///
/// If Cargo didn't set `OUT_DIR`, which it always does for build scripts.
fn main() {
    let Ok(cef_dir) = std::env::var("DEP_CEF_DLL_WRAPPER_CEF_DIR") else {
        return;
    };
    let cef_dir = PathBuf::from(cef_dir);
    // OUT_DIR is target/<profile>/build/<crate>-<hash>/out; the executables live in
    // target/<profile>.
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let Some(profile_dir) = out_dir.ancestors().nth(3) else {
        return;
    };
    println!("cargo:rerun-if-changed={}", cef_dir.display());
    if let Err(err) = copy_runtime(&cef_dir, profile_dir) {
        println!("cargo:warning=could not copy the CEF runtime next to the executable: {err}");
    }
}

/// Copies the runtime files in `from` (and its `locales` directory) into `to`.
///
/// Only runtime files are copied: libraries, `.pak` resources, ICU data and snapshot blobs.
/// Headers, import libraries and the distribution's `archive.json` stay behind.
fn copy_runtime(from: &Path, to: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name == "locales" {
                std::fs::create_dir_all(to.join("locales"))?;
                copy_runtime(&path, &to.join("locales"))?;
            }
            continue;
        }
        let runtime = [".dll", ".pak", ".dat", ".bin", ".json", ".so", ".dylib"]
            .iter()
            .any(|ext| name.ends_with(ext))
            && name != "archive.json";
        if !runtime {
            continue;
        }
        let dest = to.join(&*name);
        // Size is a cheap stand-in for "already copied": copying ~300 MB on every build would
        // be slow, and a different CEF version changes the sizes in practice.
        let same = std::fs::metadata(&dest)
            .is_ok_and(|d| d.len() == entry.metadata().map_or(0, |m| m.len()));
        if !same {
            std::fs::copy(&path, &dest)?;
        }
    }
    Ok(())
}

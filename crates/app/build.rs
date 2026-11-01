//! Build script for the app: embeds `macaroni.exe.manifest` and the app icon (`macaroni.rc`,
//! `assets/macaroni.ico`, what Explorer and pinned shortcuts show) in the Windows executable,
//! and copies the bundled ConPTY (`vendor/conpty/<arch>/conpty.dll`, `OpenConsole.exe`) next to
//! the built executables: `portable-pty` loads `conpty.dll` by bare name, so the app's directory
//! wins over the inbox one. Does nothing off Windows.
//!
//! The manifest must be embedded, not side by side: without its `supportedOS` entry for Windows
//! 10, Windows reports version 8 and Chromium's GPU process dies at start.
//!
//! `libcef.dll` is delay-loaded: Chromium isn't shipped with the app but downloaded on request,
//! so the executable must start without it. `macaroni-browser` loads it from the download
//! before the first CEF call (`delayimp.lib` comes with `cef-dll-sys`).

use std::path::{Path, PathBuf};

/// Emits the manifest link arguments and copies the ConPTY files. A failed copy is a cargo
/// warning, not an error, so the build still succeeds (sessions then use the inbox ConPTY).
///
/// # Panics
///
/// If cargo didn't set `OUT_DIR`.
fn main() {
    let vendor = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/conpty");
    println!("cargo:rerun-if-changed={}", vendor.display());

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let rc = Path::new(env!("CARGO_MANIFEST_DIR")).join("macaroni.rc");
    println!("cargo:rerun-if-changed=assets/macaroni.ico");
    // Only the GUI binary gets the icon; macaroni-cli is another crate.
    let _ = embed_resource::compile_for(&rc, ["macaroni"], embed_resource::NONE)
        .manifest_optional()
        .map_err(|e| println!("cargo:warning=app icon not embedded: {e}"));
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("macaroni.exe.manifest");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg-bin=macaroni=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bin=macaroni=/DELAYLOAD:libcef.dll");
        println!(
            "cargo:rustc-link-arg-bin=macaroni=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("aarch64") => "arm64",
        _ => return,
    };

    // OUT_DIR is target/<profile>/build/<crate>-<hash>/out; the executables live in target/<profile>.
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let Some(profile_dir) = out_dir.ancestors().nth(3) else {
        return;
    };
    for file in ["conpty.dll", "OpenConsole.exe"] {
        let dest = profile_dir.join(file);
        println!("cargo:rerun-if-changed={}", dest.display());
        if let Err(err) = std::fs::copy(vendor.join(arch).join(file), &dest) {
            println!("cargo:warning=could not copy {file} next to the executable: {err}");
        }
    }
}

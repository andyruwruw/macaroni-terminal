//! CMake and Ninja for building CEF's C++ wrapper, downloaded when the machine has neither on
//! `PATH` (nor, on Windows, in Visual Studio), so building needs no extra setup.
//!
//! Each is the official portable release (Kitware's CMake, ninja-build's Ninja), pinned to a
//! version and a SHA-256, downloaded with the system's `curl`, checked, and unpacked into
//! `target/tools/<name>`, where later builds find it. Nothing is installed system-wide.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

/// One downloadable tool.
struct Tool {
    /// The folder under `target/tools` it's unpacked into, named with its version.
    name: &'static str,
    /// The release archive's URL.
    url: &'static str,
    /// The archive's SHA-256, as lowercase hex.
    sha256: &'static str,
    /// The folder inside the unpacked archive that holds the program.
    bin: &'static str,
}

/// This machine's download of `program` (`cmake` or `ninja`), if there's one for it. Windows on
/// Arm runs the x64 builds.
fn tool(program: &str) -> Option<Tool> {
    let tool = match (program, std::env::consts::OS, std::env::consts::ARCH) {
        ("cmake", "windows", _) => Tool {
            name: "cmake-3.31.6",
            url: "https://github.com/Kitware/CMake/releases/download/v3.31.6/cmake-3.31.6-windows-x86_64.zip",
            sha256: "d163cd3ab4959b0a53fa8988f2ddbd2e6c501658201e6a154386bad9dbe4f836",
            bin: "cmake-3.31.6-windows-x86_64/bin",
        },
        ("cmake", "macos", _) => Tool {
            name: "cmake-3.31.6",
            url: "https://github.com/Kitware/CMake/releases/download/v3.31.6/cmake-3.31.6-macos-universal.tar.gz",
            sha256: "330b9514f5112e5ed4fb08b8b05803b776fd9b539a6ae12927d14dcc0ee2ba8d",
            bin: "cmake-3.31.6-macos-universal/CMake.app/Contents/bin",
        },
        ("cmake", "linux", "x86_64") => Tool {
            name: "cmake-3.31.6",
            url: "https://github.com/Kitware/CMake/releases/download/v3.31.6/cmake-3.31.6-linux-x86_64.tar.gz",
            sha256: "5a1133ff103c71eb5120e2cc3de922733e7d8a26a98ae716397e8676adb367bf",
            bin: "cmake-3.31.6-linux-x86_64/bin",
        },
        ("ninja", "windows", _) => Tool {
            name: "ninja-1.12.1",
            url: "https://github.com/ninja-build/ninja/releases/download/v1.12.1/ninja-win.zip",
            sha256: "f550fec705b6d6ff58f2db3c374c2277a37691678d6aba463adcbb129108467a",
            bin: "",
        },
        ("ninja", "macos", _) => Tool {
            name: "ninja-1.12.1",
            url: "https://github.com/ninja-build/ninja/releases/download/v1.12.1/ninja-mac.zip",
            sha256: "89a287444b5b3e98f88a945afa50ce937b8ffd1dcc59c555ad9b1baf855298c9",
            bin: "",
        },
        ("ninja", "linux", "x86_64") => Tool {
            name: "ninja-1.12.1",
            url: "https://github.com/ninja-build/ninja/releases/download/v1.12.1/ninja-linux.zip",
            sha256: "6f98805688d19672bd699fbbfa2c2cf0fc054ac3df1f0e6a47664d963d530255",
            bin: "",
        },
        _ => return None,
    };
    Some(tool)
}

/// The folder holding `program` (`cmake` or `ninja`) under `root`'s `target/tools`,
/// downloading and unpacking it first if it isn't there yet.
///
/// # Errors
///
/// When there's no download for this machine, or downloading, checking or unpacking fails.
pub fn fetch(root: &Path, program: &str) -> Result<PathBuf, String> {
    let tool = tool(program).ok_or_else(|| format!("no {program} download for this system"))?;
    let dir = root.join("target").join("tools").join(tool.name);
    let bin = dir.join(tool.bin);
    if crate::find(std::slice::from_ref(&bin), program).is_some() {
        return Ok(bin);
    }
    println!(
        "xtask: downloading {} (not found on this machine)",
        tool.name
    );
    // A half-unpacked folder from an interrupted run starts over.
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let archive = dir.join(tool.url.rsplit('/').next().unwrap_or("download"));
    let downloaded = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--output",
        ])
        .arg(&archive)
        .arg(tool.url)
        .status()
        .map_err(|e| format!("running curl: {e}"))?;
    if !downloaded.success() {
        return Err(format!("downloading {} failed", tool.url));
    }
    let bytes = fs::read(&archive).map_err(|e| format!("reading {}: {e}", archive.display()))?;
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if digest != tool.sha256 {
        let _ = fs::remove_dir_all(&dir);
        return Err(format!(
            "{} didn't match its pinned checksum, so it wasn't used",
            tool.url
        ));
    }
    let unpacked = unpack(&archive, &dir)?;
    let _ = fs::remove_file(&archive);
    if !unpacked || crate::find(std::slice::from_ref(&bin), program).is_none() {
        let _ = fs::remove_dir_all(&dir);
        return Err(format!("unpacking {} failed", archive.display()));
    }
    // Zip archives don't keep the executable bit on macOS and Linux.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(bin.join(program), fs::Permissions::from_mode(0o755));
    }
    Ok(bin)
}

/// Unpacks `archive` (a `.zip` or `.tar.gz`) into `dir`; whether it worked. Windows' own `tar`
/// (bsdtar) and macOS's read both kinds; Git Bash's GNU `tar` can come first on `PATH` and
/// can't read a zip, so Windows' is named in full. Linux uses `unzip` for a zip.
fn unpack(archive: &Path, dir: &Path) -> Result<bool, String> {
    let zip = archive.extension().is_some_and(|e| e == "zip");
    let mut command = if cfg!(windows) {
        let system = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        Command::new(Path::new(&system).join(r"System32\tar.exe"))
    } else if zip && cfg!(target_os = "linux") {
        Command::new("unzip")
    } else {
        Command::new("tar")
    };
    if zip && cfg!(target_os = "linux") {
        command.arg("-q").arg(archive).arg("-d").arg(dir);
    } else {
        command.arg("-xf").arg(archive).arg("-C").arg(dir);
    }
    command
        .status()
        .map(|s| s.success())
        .map_err(|e| format!("unpacking {}: {e}", archive.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Downloads both tools for this machine into a scratch root and runs them. Ignored by
    /// default since it downloads about 50 MB: `cargo test -p xtask -- --ignored`.
    #[test]
    #[ignore]
    fn both_tools_download_check_and_run() {
        let root = std::env::temp_dir().join("macaroni-xtask-tools-test");
        let _ = fs::remove_dir_all(&root);
        for program in ["cmake", "ninja"] {
            let bin = fetch(&root, program).unwrap();
            let ran = Command::new(bin.join(program))
                .arg("--version")
                .status()
                .unwrap();
            assert!(ran.success(), "{program} --version");
            // A second fetch finds it without downloading.
            assert_eq!(fetch(&root, program).unwrap(), bin);
        }
        let _ = fs::remove_dir_all(&root);
    }
}

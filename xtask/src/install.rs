//! `cargo xtask install`: build the release bundle, install it as the everyday macaroni, and
//! start it. `./install` (`install.cmd` on Windows) runs it.
//!
//! macOS installs `macaroni.app` to `/Applications`; Windows installs the bundle folder to
//! `%LOCALAPPDATA%\Programs\macaroni` with a Start menu shortcut. Settings and data live
//! elsewhere and are never touched. The running app is closed with a normal close request, so
//! it saves its session and the new build restores it.
//!
//! Run from a terminal inside macaroni, closing the app would kill that terminal and this task
//! with it, so after the build the rest runs detached: in a console of its own on Windows, in
//! the background with a log on macOS.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a close request gets before the installed app is killed.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(10);

/// What `cargo xtask install` was asked to do.
#[derive(Default)]
struct Options {
    /// Install the last bundle instead of building one (`--no-build`).
    no_build: bool,
    /// Remove the installed app instead (`--uninstall`).
    uninstall: bool,
    /// Already running detached from a terminal inside macaroni (`--detached`, internal).
    detached: bool,
}

/// Parses `args` (after `install`) and installs, or uninstalls. Closes and starts the installed
/// app, writes to its install folder, and on Windows its Start menu shortcut.
pub fn run(root: &Path, args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut options = Options::default();
    for arg in args {
        match arg.as_str() {
            "--no-build" => options.no_build = true,
            "--uninstall" => options.uninstall = true,
            "--detached" => options.detached = true,
            _ => {
                return Err(format!(
                    "unknown option {arg}; usage: install [--no-build | --uninstall]"
                ));
            }
        }
    }
    if options.detached {
        return detached(&options, root);
    }
    if !options.no_build && !options.uninstall {
        crate::bundle()?;
    }
    if std::env::var_os("MACARONI_SOCKET").is_some() {
        return hand_off(&options);
    }
    step(&options, root)
}

/// Installs or uninstalls, now that nothing is left to build.
fn step(options: &Options, root: &Path) -> Result<(), String> {
    let dest = dest()?;
    close(&dest)?;
    if options.uninstall {
        return uninstall(&dest);
    }
    let bundle = crate::bundle_path(root);
    if !bundle.exists() {
        return Err(format!(
            "no bundle at {}; run without --no-build",
            bundle.display()
        ));
    }
    println!("Installing to {}", dest.display());
    copy_bundle(&bundle, &dest)?;
    println!("Starting macaroni");
    launch(&dest)
}

/// Runs [`step`] in the detached process. On Windows its console closes when it ends, so it
/// waits for Enter whenever there's something left to read.
fn detached(options: &Options, root: &Path) -> Result<(), String> {
    let result = step(options, root);
    if cfg!(windows) && (result.is_err() || options.uninstall) {
        if let Err(err) = &result {
            eprintln!("xtask: {err}");
        }
        println!("Press Enter to close");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
    result
}

/// Starts this task again, detached from the terminal, to finish without the build.
fn hand_off(options: &Options) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("finding xtask: {e}"))?;
    let mut command = Command::new(exe);
    command.args(["install", "--no-build", "--detached"]);
    if options.uninstall {
        command.arg("--uninstall");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        /// `CREATE_NEW_CONSOLE`: not attached to the terminal's console, so closing it doesn't
        /// end this process.
        const CREATE_NEW_CONSOLE: u32 = 0x10;
        command.creation_flags(CREATE_NEW_CONSOLE);
        println!("Running inside macaroni: continuing in a separate window.");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A process group of its own isn't sent the terminal's hangup when macaroni quits.
        command.process_group(0);
        let log = std::env::temp_dir().join("macaroni-install.log");
        let file =
            std::fs::File::create(&log).map_err(|e| format!("creating {}: {e}", log.display()))?;
        let err = file
            .try_clone()
            .map_err(|e| format!("opening {}: {e}", log.display()))?;
        command.stdin(Stdio::null()).stdout(file).stderr(err);
        println!(
            "Running inside macaroni: continuing in the background (log: {}).",
            log.display()
        );
    }
    command
        .spawn()
        .map(drop)
        .map_err(|e| format!("starting the detached install: {e}"))
}

/// Where the app is installed: the `.app` on macOS, the folder on Windows.
fn dest() -> Result<PathBuf, String> {
    if cfg!(target_os = "macos") {
        Ok("/Applications/macaroni.app".into())
    } else if cfg!(windows) {
        let local = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA isn't set")?;
        Ok(Path::new(&local).join("Programs").join("macaroni"))
    } else {
        Err("install supports macOS and Windows".into())
    }
}

/// The Start menu shortcut on Windows.
fn shortcut() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(Path::new(&appdata).join(r"Microsoft\Windows\Start Menu\Programs\macaroni.lnk"))
}

/// The processes running from the install, by id: the app and, on Windows, Chromium's helpers.
fn running(dest: &Path) -> Vec<String> {
    let output = if cfg!(windows) {
        let script = format!(
            "Get-Process | Where-Object {{ $_.Path -and $_.Path.StartsWith('{}', 'OrdinalIgnoreCase') }} | ForEach-Object Id",
            dest.display()
        );
        Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
    } else {
        Command::new("pgrep")
            .args(["-f", &format!("^{}/", dest.display())])
            .output()
    };
    output
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Closes the installed app as its close button would, waits for every process from the
/// install to exit (Chromium's helpers follow a moment after the window), and kills what's
/// left after [`CLOSE_TIMEOUT`], so no file is locked or replaced while in use.
fn close(dest: &Path) -> Result<(), String> {
    let pids = running(dest);
    if pids.is_empty() {
        return Ok(());
    }
    println!("Closing macaroni...");
    if cfg!(windows) {
        // Without `/F`, taskkill posts WM_CLOSE, so the app saves its session. The windowless
        // helpers refuse it, which is fine: they exit with the app.
        for pid in &pids {
            let _ = Command::new("taskkill").args(["/PID", pid]).output();
        }
    } else {
        let _ = Command::new("kill").arg("-TERM").args(&pids).output();
    }
    let deadline = Instant::now() + CLOSE_TIMEOUT;
    while Instant::now() < deadline {
        if running(dest).is_empty() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    for pid in running(dest) {
        let _ = if cfg!(windows) {
            Command::new("taskkill").args(["/F", "/PID", &pid]).output()
        } else {
            Command::new("kill").args(["-KILL", &pid]).output()
        };
    }
    while !running(dest).is_empty() {
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

/// Replaces the install with the bundle, leaving nothing from the previous version.
fn copy_bundle(bundle: &Path, dest: &Path) -> Result<(), String> {
    if cfg!(windows) {
        let status = Command::new("robocopy")
            .arg(bundle)
            .arg(dest)
            .args(["/MIR", "/NFL", "/NDL", "/NJH", "/NJS", "/NP"])
            .stdout(Stdio::null())
            .status()
            .map_err(|e| format!("running robocopy: {e}"))?;
        // robocopy's exit codes below 8 all mean success (they say what it copied).
        if status.code().is_none_or(|code| code >= 8) {
            return Err(format!("robocopy failed ({status})"));
        }
        create_shortcut(dest)
    } else {
        if dest.exists() {
            std::fs::remove_dir_all(dest)
                .map_err(|e| format!("removing {}: {e}", dest.display()))?;
        }
        // ditto keeps the bundle's symlinks, permissions and signature intact.
        let status = Command::new("ditto")
            .arg(bundle)
            .arg(dest)
            .status()
            .map_err(|e| format!("running ditto: {e}"))?;
        if !status.success() {
            return Err(format!("ditto failed ({status})"));
        }
        Ok(())
    }
}

/// Points the Start menu shortcut at the installed executable.
fn create_shortcut(dest: &Path) -> Result<(), String> {
    let link = shortcut().ok_or("APPDATA isn't set")?;
    let exe = dest.join("macaroni.exe");
    let script = format!(
        "$l = (New-Object -ComObject WScript.Shell).CreateShortcut('{link}'); \
         $l.TargetPath = '{exe}'; $l.WorkingDirectory = '{dir}'; $l.IconLocation = '{exe},0'; $l.Save()",
        link = link.display(),
        exe = exe.display(),
        dir = dest.display(),
    );
    let status = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .status()
        .map_err(|e| format!("running powershell: {e}"))?;
    if !status.success() {
        return Err("creating the Start menu shortcut failed".into());
    }
    Ok(())
}

/// Starts the installed app through the shell (Explorer, Launch Services) rather than as a
/// child, so it gets a clean environment instead of this terminal's (`MACARONI_*` or
/// `NO_COLOR` would leak into every tab) and outlives this task.
fn launch(dest: &Path) -> Result<(), String> {
    let mut command = if cfg!(windows) {
        let mut c = Command::new("explorer.exe");
        c.arg(dest.join("macaroni.exe"));
        c
    } else {
        let mut c = Command::new("open");
        c.arg(dest);
        c
    };
    // explorer.exe exits with 1 even when it started the app, so only failing to run counts.
    command
        .status()
        .map(drop)
        .map_err(|e| format!("starting macaroni: {e}"))
}

/// Removes the install and, on Windows, its shortcut.
fn uninstall(dest: &Path) -> Result<(), String> {
    if dest.exists() {
        std::fs::remove_dir_all(dest).map_err(|e| format!("removing {}: {e}", dest.display()))?;
    }
    if let Some(link) = shortcut().filter(|l| cfg!(windows) && l.exists()) {
        std::fs::remove_file(&link).map_err(|e| format!("removing {}: {e}", link.display()))?;
    }
    println!("Removed {}. Settings and data are kept.", dest.display());
    Ok(())
}

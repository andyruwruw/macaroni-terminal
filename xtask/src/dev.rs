//! `cargo xtask dev`: rebuild on every save and restart macaroni with the new build.
//!
//! The app runs from a copy in `target/dev/run`, so the next build can replace the executable
//! while the old one is still open; it's only closed once the new build succeeded. Closing is a
//! normal close request, so the session is saved and the new instance restores it. The dev
//! instance has its own config directory and socket, so it runs beside an everyday macaroni.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};

/// Saves usually arrive as several events; wait for them to settle before building.
const SETTLE: Duration = Duration::from_millis(300);
/// How long a close request gets before the old instance is killed.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(8);
/// Files whose changes trigger a rebuild: sources, manifests, the lockfile and the assets
/// compiled into the app.
const WATCHED_EXTENSIONS: [&str; 6] = ["rs", "toml", "lock", "svg", "ttf", "manifest"];

/// The dev loop: build, restart the app on success, wait for a change, repeat. Only returns on
/// an error setting up the file watcher.
///
/// Watches `crates/`, `Cargo.toml` and `Cargo.lock` under `root`. Honors `CARGO_TARGET_DIR`.
/// Spawns and stops `macaroni` processes, and copies build output into `target/dev/run`.
pub fn run(root: &Path) -> Result<(), String> {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(|dir| root.join(dir))
        .unwrap_or_else(|| root.join("target"));
    let build_dir = target.join("debug");
    let dev = target.join("dev");
    let run_dir = dev.join("run");

    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if let Ok(event) = event
            && event.paths.iter().any(|p| watched(p))
        {
            let _ = tx.send(());
        }
    })
    .map_err(|e| format!("watching for changes: {e}"))?;
    for path in ["crates", "Cargo.toml", "Cargo.lock"] {
        watcher
            .watch(&root.join(path), RecursiveMode::Recursive)
            .map_err(|e| format!("watching {path}: {e}"))?;
    }

    let mut app: Option<Child> = None;
    loop {
        // Build while the old instance keeps running from its copy; only a good build replaces
        // it, so a compile error never leaves you without a running app.
        if build(root) {
            stop(&mut app);
            match sync(&build_dir, &run_dir).and_then(|()| start(root, &dev, &run_dir)) {
                Ok(child) => {
                    println!("dev: macaroni started; save a file to rebuild and restart it");
                    app = Some(child);
                }
                Err(err) => eprintln!("dev: {err}"),
            }
        } else if app.is_some() {
            eprintln!("dev: build failed; the running macaroni is the previous build");
        }
        wait_for_change(&rx, &mut app);
    }
}

/// Whether a changed path should trigger a rebuild (by [`WATCHED_EXTENSIONS`]).
fn watched(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| WATCHED_EXTENSIONS.contains(&e))
}

/// Debug-builds `macaroni` and `macaroni-cli`, output going to the terminal; `true` on
/// success.
fn build(root: &Path) -> bool {
    crate::cargo()
        .current_dir(root)
        .args([
            "build",
            "--package",
            "macaroni",
            "--package",
            "macaroni-cli",
        ])
        .status()
        .is_ok_and(|s| s.success())
}

/// Blocks until a watched file changes and the burst of events settles. Meanwhile, notes when
/// the app exits by itself: it starts again with the next build.
fn wait_for_change(rx: &Receiver<()>, app: &mut Option<Child>) {
    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(()) => break,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if let Some(child) = app
            && let Ok(Some(status)) = child.try_wait()
        {
            println!("dev: macaroni exited ({status}); it starts again after the next change");
            *app = None;
        }
    }
    while rx.recv_timeout(SETTLE).is_ok() {}
}

/// Copies the build into the run directory: whatever changed, which after the first time is
/// usually just the executables.
fn sync(build_dir: &Path, run_dir: &Path) -> Result<(), String> {
    copy_changed(build_dir, run_dir, |name| {
        let suffix = std::env::consts::EXE_SUFFIX;
        name == format!("macaroni{suffix}")
            || name == format!("macaroni-cli{suffix}")
            || name == "macaroni.pdb"
            || name == "OpenConsole.exe"
            || ([".dll", ".pak", ".dat", ".bin", ".json"]
                .iter()
                .any(|ext| name.ends_with(ext))
                && name != "archive.json")
    })?;
    let locales = build_dir.join("locales");
    if locales.is_dir() {
        copy_changed(&locales, &run_dir.join("locales"), |_| true)?;
    }
    Ok(())
}

/// Copies the files directly in `from` whose names pass `wanted` into `to`, skipping any whose
/// size and modification time already match. Where the platform's copy doesn't keep
/// modification times, unchanged files are simply copied again. Creates `to` if needed.
fn copy_changed(from: &Path, to: &Path, wanted: impl Fn(&str) -> bool) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| format!("creating {}: {e}", to.display()))?;
    let entries = fs::read_dir(from).map_err(|e| format!("reading {}: {e}", from.display()))?;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !path.is_file() || !wanted(&name) {
            continue;
        }
        let dest = to.join(&name);
        let same = |a: &Path, b: &Path| -> Option<bool> {
            let (a, b) = (fs::metadata(a).ok()?, fs::metadata(b).ok()?);
            Some(a.len() == b.len() && a.modified().ok()? == b.modified().ok()?)
        };
        if same(&path, &dest) != Some(true) {
            fs::copy(&path, &dest)
                .map_err(|e| format!("copying {} to {}: {e}", path.display(), dest.display()))?;
        }
    }
    Ok(())
}

/// Starts the copied app from `run_dir` with the repo root as its working directory. Unless
/// already set, `MACARONI_CONFIG_DIR` points at `<dev>/config` and `MACARONI_SOCKET` at a dev
/// endpoint, so it never touches an everyday instance's config, session or socket.
fn start(root: &Path, dev: &Path, run_dir: &Path) -> Result<Child, String> {
    let exe = run_dir.join(format!("macaroni{}", std::env::consts::EXE_SUFFIX));
    let mut command = Command::new(&exe);
    command.current_dir(root);
    if std::env::var_os("MACARONI_CONFIG_DIR").is_none() {
        command.env("MACARONI_CONFIG_DIR", dev.join("config"));
    }
    if std::env::var_os("MACARONI_SOCKET").is_none() {
        let socket: PathBuf = if cfg!(windows) {
            "macaroni-dev".into()
        } else {
            dev.join("macaroni.sock")
        };
        command.env("MACARONI_SOCKET", socket);
    }
    command
        .spawn()
        .map_err(|e| format!("starting {}: {e}", exe.display()))
}

/// Asks the running instance to close, as clicking × would, so it saves its session; kills it
/// if it hasn't exited in time.
fn stop(app: &mut Option<Child>) {
    let Some(mut child) = app.take() else {
        return;
    };
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    let pid = child.id().to_string();
    // Without `/F`, taskkill posts WM_CLOSE to the window, the same close request as clicking
    // the close button, so the app saves its session before exiting.
    let _ = if cfg!(windows) {
        Command::new("taskkill").args(["/PID", &pid]).output()
    } else {
        Command::new("kill").args(["-TERM", &pid]).output()
    };
    let deadline = Instant::now() + CLOSE_TIMEOUT;
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("dev: macaroni didn't close in time; killing it");
    let _ = child.kill();
    let _ = child.wait();
}

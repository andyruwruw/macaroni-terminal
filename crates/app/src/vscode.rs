//! VS Code in browser tabs: an opt-in download of Microsoft's standalone VS Code CLI, whose
//! `code serve-web` hosts VS Code for the web on this machine, shown in an ordinary browser tab.
//!
//! Nothing is downloaded until the user asks (Settings > Integrations > VS Code, which calls
//! [`install`]): the CLI (about 10 MB) goes to [`dir`], fetched with the system's `curl` and
//! unpacked with its `tar`. The CLI then downloads the VS Code server itself, into the same
//! folder, the first time a tab loads.
//!
//! [`WebVsCode`] is the app's side: whether it's installed, the running [`Server`], and the tabs
//! waiting for it to listen. The server binds 127.0.0.1 only and needs a connection token,
//! which, like its port, is kept in [`dir`] so the saved addresses of restored tabs stay valid
//! across restarts. It's stopped (with its child processes) when the app quits or it's removed.
//! Browser tabs are Windows only, so [`available`] is false elsewhere.

use std::fs::File;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use iced::futures::channel::oneshot;
use macaroni_core::PaneId;

/// Where the CLI for this platform downloads from; `None` where browser tabs don't exist.
fn download_url() -> Option<&'static str> {
    match (cfg!(windows), std::env::consts::ARCH) {
        (true, "x86_64") => {
            Some("https://update.code.visualstudio.com/latest/cli-win32-x64/stable")
        }
        (true, "aarch64") => {
            Some("https://update.code.visualstudio.com/latest/cli-win32-arm64/stable")
        }
        _ => None,
    }
}

/// The license `serve-web` asks to accept before it downloads the server; the settings row
/// links it, and downloading counts as accepting it.
pub const LICENSE_URL: &str = "https://aka.ms/vscode-server-license";

/// How long a starting server has to begin listening before its tabs give up.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// How often a starting server's port is tried.
const POLL_EVERY: Duration = Duration::from_millis(200);

/// Whether VS Code tabs can exist on this platform.
pub fn available() -> bool {
    download_url().is_some()
}

/// The folder for the CLI, the server it downloads and its data: `vscode` beside the browser
/// profile (`%LOCALAPPDATA%\macaroni\vscode`, or `<MACARONI_CONFIG_DIR>/vscode`).
pub fn dir() -> PathBuf {
    crate::browser::local_data("vscode")
}

/// The downloaded CLI.
fn cli() -> PathBuf {
    dir()
        .join("cli")
        .join(if cfg!(windows) { "code.exe" } else { "code" })
}

/// Whether the CLI has been downloaded.
pub fn installed() -> bool {
    cli().is_file()
}

/// `program` from `System32`: Windows' own `curl` and `tar` (bsdtar, which unpacks zips), not
/// Git for Windows' GNU tar that may come first on `PATH`.
fn system_tool(program: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("System32").join(program)
}

/// Runs `command` without a console window, turning a failure into a message naming `what`.
fn run(mut command: Command, what: &str) -> Result<(), String> {
    hide_window(&mut command);
    let out = command
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{what}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    Err(format!("{what} failed: {detail}"))
}

/// Downloads and unpacks the CLI into [`dir`], replacing any earlier copy. Blocking: run it off
/// the UI thread ([`off_thread`]).
pub fn install() -> Result<(), String> {
    let url = download_url().ok_or("VS Code tabs aren't supported on this platform")?;
    let target = dir().join("cli");
    let _ = std::fs::remove_dir_all(&target);
    std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    let zip = dir().join("cli.zip");
    let mut curl = Command::new(system_tool("curl.exe"));
    curl.args(["-fsSL", "--retry", "2", "-o"])
        .arg(&zip)
        .arg(url);
    run(curl, "Download")?;
    let mut tar = Command::new(system_tool("tar.exe"));
    tar.arg("-xf").arg(&zip).arg("-C").arg(&target);
    let unpacked = run(tar, "Unpacking");
    let _ = std::fs::remove_file(&zip);
    unpacked?;
    if installed() {
        Ok(())
    } else {
        Err("the download didn't contain the VS Code CLI".to_owned())
    }
}

/// Deletes [`dir`]: the CLI, the server and their data. Blocking.
pub fn uninstall() -> Result<(), String> {
    match std::fs::remove_dir_all(dir()) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(format!("{}: {e}", dir().display())),
        _ => Ok(()),
    }
}

/// Runs `work` on its own thread, as a future for [`iced::Task::future`]; the executor has no
/// blocking pool.
pub async fn off_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    let (send, receive) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = send.send(work());
    });
    receive.await.expect("the worker thread always sends")
}

/// A running `code serve-web`. Dropping it stops the CLI and the server it started.
pub struct Server {
    /// The CLI process.
    child: Child,
    /// The port it listens on, on 127.0.0.1.
    pub port: u16,
    /// The connection token every address carries.
    token: String,
}

impl Server {
    /// Starts the CLI on the saved port and token (new ones when there are none, or the port is
    /// taken), hidden, logging to `server.log` in [`dir`]. Returns before it listens.
    fn start() -> io::Result<Self> {
        let dir = dir();
        let (saved_port, token) = saved().unwrap_or_else(|| (0, new_token()));
        let port = if port_free(saved_port) {
            saved_port
        } else {
            free_port()?
        };
        std::fs::write(dir.join("server"), format!("{port} {token}"))?;
        let log = File::create(dir.join("server.log"))?;
        let mut command = Command::new(cli());
        command
            .arg("serve-web")
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            .args([
                "--connection-token",
                &token,
                "--accept-server-license-terms",
            ])
            .arg("--server-data-dir")
            .arg(dir.join("server"))
            // Keeps the CLI's own state (and the server builds it downloads) in `dir` too.
            .env("VSCODE_CLI_DATA_DIR", dir.join("cli-data"))
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        hide_window(&mut command);
        Ok(Self {
            child: command.spawn()?,
            port,
            token,
        })
    }

    /// The address that opens `folder` (else an empty window).
    pub fn url(&self, folder: Option<&Path>) -> String {
        let mut url = format!("http://127.0.0.1:{}/?tkn={}", self.port, self.token);
        if let Some(folder) = folder {
            url.push_str("&folder=");
            url.push_str(&folder_query(folder));
        }
        url
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // The CLI runs the server as a child, which killing the CLI alone would leave running
        // (and holding the port).
        if cfg!(windows) {
            let mut kill = Command::new(system_tool("taskkill.exe"));
            kill.args(["/T", "/F", "/PID", &self.child.id().to_string()]);
            let _ = run(kill, "Stopping VS Code");
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The saved port and token, if both are there.
fn saved() -> Option<(u16, String)> {
    let text = std::fs::read_to_string(dir().join("server")).ok()?;
    let (port, token) = text.trim().split_once(' ')?;
    Some((port.parse().ok()?, token.to_owned()))
}

/// The saved port, for recognizing a restored tab's address before the server starts.
fn saved_port() -> Option<u16> {
    saved().map(|(port, _)| port)
}

/// 128 random bits as hex. `RandomState` is seeded from the OS, so no random crate is needed.
fn new_token() -> String {
    let half = || {
        std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish()
    };
    format!("{:016x}{:016x}", half(), half())
}

/// Whether `port` (non-zero) can be listened on at 127.0.0.1.
fn port_free(port: u16) -> bool {
    port != 0 && TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok()
}

/// A port nothing listens on, from the OS.
fn free_port() -> io::Result<u16> {
    Ok(TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?
        .local_addr()?
        .port())
}

/// Tries `port` until something listens or [`START_TIMEOUT`] passes. Blocking.
pub fn wait_listening(port: u16) -> bool {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let deadline = Instant::now() + START_TIMEOUT;
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(&address, POLL_EVERY).is_ok() {
            return true;
        }
        std::thread::sleep(POLL_EVERY);
    }
    false
}

/// `folder` as VS Code's `folder` query wants it: a URI path (`/C:/Users/me/project` on
/// Windows), percent-encoded.
fn folder_query(folder: &Path) -> String {
    let mut path = folder.to_string_lossy().replace('\\', "/");
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    let mut out = String::new();
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Keeps a child process from opening a console window on Windows.
fn hide_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        /// `CREATE_NO_WINDOW`.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// The app's VS Code state.
#[derive(Default)]
pub struct WebVsCode {
    /// Whether the CLI is downloaded, checked at launch and after installing or removing it.
    pub installed: bool,
    /// Set while it's being downloaded or removed.
    pub busy: bool,
    /// What the last download, removal or start reported, shown in the settings row.
    pub error: Option<String>,
    /// The server, once started.
    pub server: Option<Server>,
    /// Whether the server listens yet.
    pub ready: bool,
    /// Tabs asked for before it listened: the pane, and the folder to open.
    pub waiting: Vec<(PaneId, Option<PathBuf>)>,
}

impl std::fmt::Debug for WebVsCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebVsCode")
            .field("installed", &self.installed)
            .field("ready", &self.ready)
            .finish_non_exhaustive()
    }
}

impl WebVsCode {
    /// The state at launch: installed if the CLI is there. Nothing starts yet.
    pub fn new() -> Self {
        Self {
            installed: available() && installed(),
            ..Self::default()
        }
    }

    /// Starts the server if it isn't running. Returns its port while it's starting (so the
    /// caller can wait for it), `None` once it listens or when it couldn't start (see
    /// [`WebVsCode::error`]).
    pub fn ensure_started(&mut self) -> Option<u16> {
        if let Some(server) = &self.server {
            return (!self.ready).then_some(server.port);
        }
        match Server::start() {
            Ok(server) => {
                let port = server.port;
                self.server = Some(server);
                self.ready = false;
                Some(port)
            }
            Err(e) => {
                self.error = Some(format!("VS Code didn't start: {e}"));
                None
            }
        }
    }

    /// Stops the server.
    pub fn stop(&mut self) {
        self.server = None;
        self.ready = false;
        self.waiting.clear();
    }

    /// Whether `url` is a VS Code tab's address: on the running server's port, or before it
    /// runs, the saved one.
    pub fn owns(&self, url: &str) -> bool {
        let port = match &self.server {
            Some(server) => Some(server.port),
            None if self.installed => saved_port(),
            None => None,
        };
        port.is_some_and(|port| url.starts_with(&format!("http://127.0.0.1:{port}/")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_become_uri_paths() {
        assert_eq!(
            folder_query(Path::new(r"C:\Users\me\my project")),
            "/C:/Users/me/my%20project"
        );
        assert_eq!(folder_query(Path::new("/home/me/a&b")), "/home/me/a%26b");
    }

    #[test]
    fn tokens_are_long_and_differ() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}

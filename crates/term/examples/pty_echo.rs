//! Manual probe: spawn a shell through portable-pty, run `echo hi`, and confirm the output
//! comes back. On Windows, also report which ConPTY host was used (OpenConsole.exe means the
//! sideloaded conpty.dll next to this executable was picked up).
//!
//! It answers ConPTY's `ESC[6n` cursor query by hand (there's no emulator here), types a command
//! whose output differs from its echo, and waits up to 10 seconds for that output. Exits with 1 if
//! it never arrives.
//!
//! Usage: cargo run -p macaroni-term --example pty_echo [-- <shell>]

use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

/// DSR 6: "where is the cursor?". ConPTY sends it at startup and waits for the answer.
const CURSOR_POSITION_REQUEST: &[u8] = b"\x1b[6n";

/// Runs the probe against the shell named on the command line, or [`default_shell`].
///
/// # Panics
///
/// If the PTY can't be opened, the shell can't be started, or writing to it fails.
fn main() {
    let shell = std::env::args().nth(1).unwrap_or_else(default_shell);
    println!("shell: {shell}");

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut child = pair
        .slave
        .spawn_command(CommandBuilder::new(&shell))
        .expect("spawn shell");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("reader");
    let mut writer = pair.master.take_writer().expect("writer");

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    #[cfg(windows)]
    report_conpty_host();

    let (command, expected) = probe_for(&shell);
    write!(writer, "{command}\r").expect("write");
    writer.flush().expect("flush");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut output = Vec::new();
    let mut seen = false;
    while Instant::now() < deadline {
        let Ok(chunk) = rx.recv_timeout(Duration::from_millis(100)) else {
            continue;
        };
        if chunk
            .windows(CURSOR_POSITION_REQUEST.len())
            .any(|w| w == CURSOR_POSITION_REQUEST)
        {
            writer.write_all(b"\x1b[1;1R").expect("answer DSR");
            writer.flush().expect("flush");
        }
        output.extend_from_slice(&chunk);
        if String::from_utf8_lossy(&output).contains(expected) {
            seen = true;
            break;
        }
    }

    child.kill().ok();

    let text = String::from_utf8_lossy(&output);
    println!("--- raw output ({} bytes) ---\n{text:?}", output.len());
    if seen {
        println!("PASS: output came back through the PTY");
    } else {
        println!("FAIL: {expected:?} not seen within 10s");
        std::process::exit(1);
    }
}

/// The command to type into `shell` and the text its output must contain.
///
/// The command computes its output so the typed input echo can't satisfy the check.
fn probe_for(shell: &str) -> (&'static str, &'static str) {
    let name = shell.to_ascii_lowercase();
    if name.contains("cmd") {
        ("echo macaroni-%OS%", "macaroni-Windows_NT")
    } else if name.contains("powershell") || name.contains("pwsh") {
        ("echo macaroni-$(40+2)", "macaroni-42")
    } else {
        ("echo macaroni-$((40+2))", "macaroni-42")
    }
}

/// The shell to probe when none is given: `cmd.exe` on Windows, else `$SHELL` or `/bin/sh`.
fn default_shell() -> String {
    if cfg!(windows) {
        "cmd.exe".into()
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    }
}

/// Prints this process's children and whether one is `OpenConsole.exe`, which means the bundled
/// `conpty.dll` was loaded instead of the inbox one (whose host is `conhost.exe`).
///
/// Waits half a second first so ConPTY has started its host, then asks PowerShell for the
/// process list.
///
/// # Panics
///
/// If PowerShell can't be started.
#[cfg(windows)]
fn report_conpty_host() {
    std::thread::sleep(Duration::from_millis(500));
    let query = format!(
        "Get-CimInstance Win32_Process -Filter 'ParentProcessId={}' | Select-Object -ExpandProperty Name",
        std::process::id()
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &query])
        .output()
        .expect("query child processes");
    let names = String::from_utf8_lossy(&out.stdout);
    let names: Vec<&str> = names
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    println!("child processes: {names:?}");
    if names
        .iter()
        .any(|n| n.eq_ignore_ascii_case("OpenConsole.exe"))
    {
        println!("conpty: sideloaded (OpenConsole.exe)");
    } else {
        println!("conpty: inbox (no OpenConsole.exe child)");
    }
}

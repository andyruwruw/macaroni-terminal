//! Manual probe: do notification escape sequences (OSC 9, OSC 777, OSC 99) survive the PTY?
//! ConPTY re-renders output instead of passing it through, and may drop sequences it doesn't know.
//!
//! Runs a one-shot script (PowerShell on Windows, `sh` elsewhere) that prints each sequence plus an
//! OSC 0 title as a control, then a marker line. It reads the raw PTY output until the marker or
//! 10 seconds, and prints whether each sequence came through unchanged, followed by the raw bytes.
//! The result backs the claim in `references/notifications.md` that both the inbox and the bundled
//! ConPTY pass all three through.
//!
//! Usage: cargo run -p macaroni-term --example osc_probe

use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

/// Runs the probe and prints the verdict for each sequence.
///
/// # Panics
///
/// If the PTY can't be opened or the script can't be started.
fn main() {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let script = "$e=[char]27; $b=[char]7; \
        [Console]::Write(\"$e]9;osc9-message$b\"); \
        [Console]::Write(\"$e]777;notify;osc777-title;osc777-body$b\"); \
        [Console]::Write(\"$e]99;;osc99-title$e\\\"); \
        [Console]::Write(\"$e]0;title-check$b\"); \
        Write-Host done-marker";
    let mut command = if cfg!(windows) {
        let mut c = CommandBuilder::new("powershell.exe");
        c.args(["-NoLogo", "-NoProfile", "-Command", script]);
        c
    } else {
        let mut c = CommandBuilder::new("/bin/sh");
        c.args([
            "-c",
            "printf '\\033]9;osc9-message\\007\\033]777;notify;osc777-title;osc777-body\\007\\033]99;;osc99-title\\033\\\\\\033]0;title-check\\007done-marker\\n'",
        ]);
        c
    };
    command.env("TERM", "xterm-256color");
    let _child = pair.slave.spawn_command(command).expect("spawn");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("reader");
    let mut writer = pair.master.take_writer().expect("writer");
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    // There is no emulator here, so answer ConPTY's startup cursor query by hand; the inbox
    // ConPTY prints nothing until it gets a reply.
    let mut output = Vec::new();
    while Instant::now() < deadline {
        let Ok(chunk) = rx.recv_timeout(Duration::from_millis(100)) else {
            continue;
        };
        if chunk.windows(4).any(|w| w == b"\x1b[6n") {
            writer.write_all(b"\x1b[1;1R").ok();
        }
        output.extend_from_slice(&chunk);
        if String::from_utf8_lossy(&output).contains("done-marker") {
            // Give anything ConPTY flushes after the marker a moment to arrive.
            std::thread::sleep(Duration::from_millis(300));
            while let Ok(chunk) = rx.try_recv() {
                output.extend_from_slice(&chunk);
            }
            break;
        }
    }

    let text = String::from_utf8_lossy(&output);
    for (name, needle) in [
        ("OSC 9", "\x1b]9;osc9-message"),
        ("OSC 777", "\x1b]777;notify;osc777-title;osc777-body"),
        ("OSC 99", "\x1b]99;;osc99-title"),
        ("OSC 0 (control)", "title-check"),
    ] {
        let seen = text.contains(needle);
        println!(
            "{name:16} {}",
            if seen { "passed through" } else { "DROPPED" }
        );
    }
    println!("--- raw ---\n{text:?}");
}

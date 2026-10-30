//! How fast the emulator parses output and how long a snapshot takes under the lock.
//!
//! Measures [`AlacrittyEmulator::feed`] on plain and color-heavy output, with and without the
//! [`OscSniffer`] in front of it (as the session reader thread runs them), in the same 64 KiB
//! chunks the reader reads. Each iteration builds a fresh emulator, so construction is included.
//! Also times one [`Emulator::snapshot`] of a full 200x50 screen, the cost paid under the session
//! lock on every redraw.
//!
//! Usage: cargo bench -p macaroni-term

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use macaroni_term::{AlacrittyEmulator, Emulator, GridSize, OscSniffer};

/// A large pane: 200x50 cells.
const SIZE: GridSize = GridSize {
    cols: 200,
    rows: 50,
    cell_width: 8,
    cell_height: 18,
};

/// 10,000 lines of plain ASCII, about 1 MiB: the fast path (`cat` of a log).
fn plain_output() -> Vec<u8> {
    let line = "0123456789abcdefghijklmnopqrstuvwxyz".repeat(3);
    (0..10_000)
        .flat_map(|i| format!("{i:06} {line}\r\n").into_bytes())
        .collect()
}

/// 10,000 lines mixing 16-color, truecolor and 256-color SGR sequences, like a compiler or
/// test runner's output.
fn colored_output() -> Vec<u8> {
    (0..10_000)
        .flat_map(|i| {
            format!(
                "\x1b[1;3{}m{i:06}\x1b[0m \x1b[38;2;{};{};200mtrue color\x1b[0m \x1b[48;5;{}m bg \x1b[0m ok\r\n",
                i % 8,
                i % 256,
                (i * 7) % 256,
                i % 256
            )
            .into_bytes()
        })
        .collect()
}

/// An emulator of [`SIZE`] with 10,000 lines of scrollback and no event sink.
fn emulator() -> AlacrittyEmulator {
    AlacrittyEmulator::new(SIZE, 10_000, |_| {})
}

/// Registers the `feed` group and the snapshot benchmark.
fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("feed");
    for (name, bytes) in [("plain", plain_output()), ("colored", colored_output())] {
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_function(name, |b| {
            b.iter(|| {
                let mut emu = emulator();
                for chunk in bytes.chunks(64 * 1024) {
                    emu.feed(black_box(chunk));
                }
            });
        });
        group.bench_function(format!("{name} + notification sniffer"), |b| {
            b.iter(|| {
                let mut emu = emulator();
                let mut sniffer = OscSniffer::default();
                for chunk in bytes.chunks(64 * 1024) {
                    black_box(sniffer.feed(chunk));
                    emu.feed(black_box(chunk));
                }
            });
        });
    }
    group.finish();

    let mut emu = emulator();
    emu.feed(&colored_output());
    c.bench_function("snapshot 200x50", |b| b.iter(|| black_box(emu.snapshot())));
}

criterion_group!(benches, bench);
criterion_main!(benches);

//! Opt-in timing of terminal draws: run with `MACARONI_FRAME_STATS=1` (stderr must be attached:
//! run from a shell, or use a debug build) and the average and maximum time to lay out and
//! record one terminal is printed every 240 draws. It measures our CPU work per terminal per
//! frame, not GPU time. [`super::TerminalView`]'s `draw` brackets [`GridRenderer::draw`] with
//! [`start`] and [`finish`].
//!
//! [`GridRenderer::draw`]: super::render::GridRenderer::draw

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How many draws each report averages over.
const REPORT_EVERY: u32 = 240;

/// Whether `MACARONI_FRAME_STATS` is set, read once.
fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("MACARONI_FRAME_STATS").is_some())
}

/// The start of a draw, or `None` when timing is off.
pub fn start() -> Option<Instant> {
    enabled().then(Instant::now)
}

/// Adds the draw that began at `started` to the totals, and prints and resets them every
/// [`REPORT_EVERY`] draws. Does nothing for `None`.
pub fn finish(started: Option<Instant>) {
    static TOTALS: Mutex<(u32, Duration, Duration)> =
        Mutex::new((0, Duration::ZERO, Duration::ZERO));
    let Some(started) = started else {
        return;
    };
    let elapsed = started.elapsed();
    let Ok(mut totals) = TOTALS.lock() else {
        return;
    };
    totals.0 += 1;
    totals.1 += elapsed;
    totals.2 = totals.2.max(elapsed);
    if totals.0 == REPORT_EVERY {
        eprintln!(
            "terminal draw: avg {:?}, max {:?} over {} draws",
            totals.1 / totals.0,
            totals.2,
            totals.0
        );
        *totals = (0, Duration::ZERO, Duration::ZERO);
    }
}

//! macaroni's own sounds, for agents asking for attention (`[notifications] sound`) and the
//! bell (`bell_sound`): short recordings ([`RECORDED`], in `assets/sounds` by kind: success,
//! notification, error), and tones built from a few decaying sine partials.
//!
//! The recordings are 16-bit mono WAV, trimmed of silence and scaled to peak at [`PEAK`] like
//! the tones. [`file()`] writes a sound's WAV to `local_data("sounds")` the first time it's
//! played in a run (and again whenever this build's differs), since both players take a file:
//! `PlaySoundW` on Windows and `afplay` on macOS (see [`crate::bell`]).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

/// Samples per second.
const RATE: u32 = 44_100;

/// The loudest a sound gets, as a share of full scale: notification sounds should be gentle.
const PEAK: f32 = 0.45;

/// The built-in sounds: the config value, and the name the settings show. The recordings come
/// first, by kind, then the tones.
pub const BUILT_IN: &[(&str, &str)] = &[
    ("arcade", "Arcade"),
    ("chimes", "Chimes"),
    ("chord", "Chord"),
    ("mail", "Mail"),
    ("perky", "Perky"),
    ("ping", "Ping"),
    ("success", "Success"),
    ("bubbly", "Bubbly"),
    ("cola", "Cola"),
    ("ding", "Ding"),
    ("drip", "Drip"),
    ("flick", "Flick"),
    ("plastic", "Plastic"),
    ("pop", "Pop"),
    ("shutter", "Shutter"),
    ("signal", "Signal"),
    ("step", "Step"),
    ("switch", "Switch"),
    ("villager", "Villager"),
    ("403", "403"),
    ("bummer", "Bummer"),
    ("buzzer", "Buzzer"),
    ("no-thanks", "No thanks"),
    ("thonk", "Thonk"),
    ("warning", "Warning"),
    ("chime", "Chime"),
    ("marimba", "Marimba"),
    ("bell", "Bell"),
    ("rise", "Rise"),
    ("blip", "Blip"),
    ("bubble", "Bubble"),
    ("hum", "Hum"),
];

/// The kinds [`BUILT_IN`] falls into, in its order: each kind's heading and how many sounds
/// in a row it covers.
pub const KINDS: &[(&str, usize)] = &[
    ("Success", 7),
    ("Notification", 12),
    ("Error", 6),
    ("Tones", 7),
];

/// The recorded sounds' WAV files, by config value.
const RECORDED: &[(&str, &[u8])] = &[
    (
        "arcade",
        include_bytes!("../assets/sounds/success/arcade.wav"),
    ),
    (
        "chimes",
        include_bytes!("../assets/sounds/success/chimes.wav"),
    ),
    (
        "chord",
        include_bytes!("../assets/sounds/success/chord.wav"),
    ),
    ("mail", include_bytes!("../assets/sounds/success/mail.wav")),
    (
        "perky",
        include_bytes!("../assets/sounds/success/perky.wav"),
    ),
    ("ping", include_bytes!("../assets/sounds/success/ping.wav")),
    (
        "success",
        include_bytes!("../assets/sounds/success/success.wav"),
    ),
    (
        "bubbly",
        include_bytes!("../assets/sounds/notification/bubbly.wav"),
    ),
    (
        "cola",
        include_bytes!("../assets/sounds/notification/cola.wav"),
    ),
    (
        "ding",
        include_bytes!("../assets/sounds/notification/ding.wav"),
    ),
    (
        "drip",
        include_bytes!("../assets/sounds/notification/drip.wav"),
    ),
    (
        "flick",
        include_bytes!("../assets/sounds/notification/flick.wav"),
    ),
    (
        "plastic",
        include_bytes!("../assets/sounds/notification/plastic.wav"),
    ),
    (
        "pop",
        include_bytes!("../assets/sounds/notification/pop.wav"),
    ),
    (
        "shutter",
        include_bytes!("../assets/sounds/notification/shutter.wav"),
    ),
    (
        "signal",
        include_bytes!("../assets/sounds/notification/signal.wav"),
    ),
    (
        "step",
        include_bytes!("../assets/sounds/notification/step.wav"),
    ),
    (
        "switch",
        include_bytes!("../assets/sounds/notification/switch.wav"),
    ),
    (
        "villager",
        include_bytes!("../assets/sounds/notification/villager.wav"),
    ),
    ("403", include_bytes!("../assets/sounds/error/403.wav")),
    (
        "bummer",
        include_bytes!("../assets/sounds/error/bummer.wav"),
    ),
    (
        "buzzer",
        include_bytes!("../assets/sounds/error/buzzer.wav"),
    ),
    (
        "no-thanks",
        include_bytes!("../assets/sounds/error/no-thanks.wav"),
    ),
    ("thonk", include_bytes!("../assets/sounds/error/thonk.wav")),
    (
        "warning",
        include_bytes!("../assets/sounds/error/warning.wav"),
    ),
];

/// Sounds written to disk in this run.
static WRITTEN: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);

/// One partial of a note: a sine at `ratio` times the note's pitch, `level` loud, dying
/// `decay` times faster than the note.
#[derive(Clone, Copy)]
struct Partial {
    /// Its pitch over the note's.
    ratio: f32,
    /// Its loudness against the first partial.
    level: f32,
    /// How much faster than the note it dies away.
    decay: f32,
}

/// A note in a sound.
#[derive(Clone, Copy)]
struct Note {
    /// When it starts, in seconds.
    at: f32,
    /// Its pitch in Hz at the start.
    pitch: f32,
    /// Its pitch at the end of [`Note::glide`], for blips that slide.
    to: f32,
    /// Seconds the pitch takes to slide from `pitch` to `to`.
    glide: f32,
    /// Seconds it takes to fade in (no click at the start).
    attack: f32,
    /// Seconds it takes to fall to about a third (an exponential decay's time constant).
    tau: f32,
    /// Its loudness against the other notes.
    level: f32,
    /// What it's made of.
    partials: &'static [Partial],
}

/// A pure tone.
const SINE: &[Partial] = &[Partial {
    ratio: 1.0,
    level: 1.0,
    decay: 1.0,
}];

/// A soft chime: the octave and twelfth quieter and shorter.
const CHIME: &[Partial] = &[
    Partial {
        ratio: 1.0,
        level: 1.0,
        decay: 1.0,
    },
    Partial {
        ratio: 2.0,
        level: 0.22,
        decay: 1.8,
    },
    Partial {
        ratio: 3.0,
        level: 0.07,
        decay: 2.5,
    },
];

/// A struck bar: a strong fourth partial that dies quickly, as on a marimba.
const BAR: &[Partial] = &[
    Partial {
        ratio: 1.0,
        level: 1.0,
        decay: 1.0,
    },
    Partial {
        ratio: 4.0,
        level: 0.3,
        decay: 4.0,
    },
];

/// A small bell: partials that aren't whole multiples.
const BELL: &[Partial] = &[
    Partial {
        ratio: 1.0,
        level: 1.0,
        decay: 1.0,
    },
    Partial {
        ratio: 2.76,
        level: 0.35,
        decay: 1.6,
    },
    Partial {
        ratio: 5.40,
        level: 0.15,
        decay: 2.4,
    },
    Partial {
        ratio: 8.93,
        level: 0.06,
        decay: 3.2,
    },
];

/// A note that doesn't slide.
const fn note(at: f32, pitch: f32, tau: f32, level: f32, partials: &'static [Partial]) -> Note {
    Note {
        at,
        pitch,
        to: pitch,
        glide: 0.0,
        attack: 0.004,
        tau,
        level,
        partials,
    }
}

/// A sine blip sliding from `pitch` to `to` over `glide` seconds.
const fn blip(at: f32, pitch: f32, to: f32, glide: f32, tau: f32) -> Note {
    Note {
        at,
        pitch,
        to,
        glide,
        attack: 0.003,
        tau,
        level: 1.0,
        partials: SINE,
    }
}

/// The notes of built-in sound `name`, and how long it lasts in seconds.
fn notes(name: &str) -> Option<(Vec<Note>, f32)> {
    Some(match name {
        // E5 then B5.
        "chime" => (
            vec![
                note(0.0, 659.25, 0.32, 0.9, CHIME),
                note(0.12, 987.77, 0.42, 1.0, CHIME),
            ],
            1.4,
        ),
        // C5 then G5, short and woody.
        "marimba" => (
            vec![
                note(0.0, 523.25, 0.14, 1.0, BAR),
                note(0.1, 783.99, 0.16, 0.9, BAR),
            ],
            0.7,
        ),
        // One A5.
        "bell" => (vec![note(0.0, 880.0, 0.55, 1.0, BELL)], 2.0),
        // C5, E5, G5, C6.
        "rise" => (
            vec![
                note(0.0, 523.25, 0.22, 0.8, CHIME),
                note(0.07, 659.25, 0.22, 0.8, CHIME),
                note(0.14, 783.99, 0.22, 0.85, CHIME),
                note(0.21, 1046.5, 0.35, 1.0, CHIME),
            ],
            1.2,
        ),
        "blip" => (vec![blip(0.0, 950.0, 320.0, 0.05, 0.035)], 0.2),
        "bubble" => (
            vec![
                blip(0.0, 380.0, 900.0, 0.07, 0.045),
                blip(0.11, 520.0, 1250.0, 0.07, 0.05),
            ],
            0.35,
        ),
        // A low, slow A4 with a soft start.
        "hum" => (
            vec![Note {
                attack: 0.04,
                ..note(0.0, 440.0, 0.45, 1.0, CHIME)
            }],
            1.6,
        ),
        _ => return None,
    })
}

/// The samples of built-in sound `name`, scaled to [`PEAK`].
fn samples(name: &str) -> Option<Vec<f32>> {
    let (notes, length) = notes(name)?;
    let rate = RATE as f32;
    let mut out = vec![0.0f32; (length * rate) as usize];
    for n in &notes {
        let start = (n.at * rate) as usize;
        // Phase is accumulated per partial so a sliding pitch stays smooth.
        let mut phases = vec![0.0f32; n.partials.len()];
        for (i, sample) in out.iter_mut().enumerate().skip(start) {
            let t = (i - start) as f32 / rate;
            let slide = if n.glide > 0.0 {
                (t / n.glide).min(1.0)
            } else {
                1.0
            };
            // Exponential between the two pitches, which sounds even.
            let pitch = n.pitch * (n.to / n.pitch).powf(slide);
            let attack = (t / n.attack).min(1.0);
            let mut value = 0.0;
            for (p, phase) in n.partials.iter().zip(&mut phases) {
                *phase += std::f32::consts::TAU * pitch * p.ratio / rate;
                *phase %= std::f32::consts::TAU;
                value += p.level * phase.sin() * (-t * p.decay / n.tau).exp();
            }
            *sample += n.level * attack * value;
        }
    }
    let loudest = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if loudest > 0.0 {
        for s in &mut out {
            *s *= PEAK / loudest;
        }
    }
    // The last 10 ms fade out, so a sound cut off at its length doesn't click.
    let fade = (rate * 0.01) as usize;
    let len = out.len();
    for (k, s) in out.iter_mut().skip(len.saturating_sub(fade)).enumerate() {
        *s *= 1.0 - k as f32 / fade as f32;
    }
    Some(out)
}

/// `samples` as a 16-bit mono PCM WAV file.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    // PCM, one channel.
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    // Two bytes a frame, 16 bits a sample.
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        let s = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Built-in sound `name` as a WAV file's bytes: a recording's own, or a tone made now.
fn bytes(name: &str) -> Option<Vec<u8>> {
    match RECORDED.iter().find(|(n, _)| *n == name) {
        Some((_, wav)) => Some(wav.to_vec()),
        None => Some(wav(&samples(name)?)),
    }
}

/// The WAV file of built-in sound `name`, written if this run hasn't yet. `None` for a name
/// that isn't built in, or when the file can't be written.
pub fn file(name: &str) -> Option<PathBuf> {
    let &(name, _) = BUILT_IN.iter().find(|(n, _)| *n == name)?;
    let path = crate::browser::local_data("sounds").join(format!("{name}.wav"));
    let mut written = WRITTEN.lock().unwrap_or_else(|e| e.into_inner());
    let written = written.get_or_insert_with(HashSet::new);
    if !written.contains(name) {
        let bytes = bytes(name)?;
        if std::fs::read(&path).ok().as_deref() != Some(&bytes[..]) {
            std::fs::create_dir_all(path.parent()?).ok()?;
            std::fs::write(&path, &bytes).ok()?;
        }
        written.insert(name);
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tone_is_made_and_gentle() {
        let tones = BUILT_IN
            .iter()
            .filter(|(n, _)| !RECORDED.iter().any(|(r, _)| r == n));
        for (name, _) in tones {
            let samples = samples(name).unwrap_or_else(|| panic!("{name} isn't made"));
            let loudest = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!((loudest - PEAK).abs() < 0.01, "{name} peaks at {loudest}");
            assert!(
                samples.last().is_some_and(|s| s.abs() < 1e-3),
                "{name} clicks"
            );
            assert!(samples.len() < RATE as usize * 3, "{name} is too long");
        }
        assert!(samples("nope").is_none());
    }

    #[test]
    fn recordings_are_short_gentle_mono_wavs() {
        for (name, wav) in RECORDED {
            assert_eq!(&wav[..4], b"RIFF", "{name}");
            let field = |at: usize| u16::from_le_bytes(wav[at..at + 2].try_into().unwrap());
            assert_eq!(
                (field(20), field(22), field(34)),
                (1, 1, 16),
                "{name} is 16-bit mono PCM"
            );
            let rate = u32::from_le_bytes(wav[24..28].try_into().unwrap());
            assert!(wav.len() - 44 < rate as usize * 2 * 3, "{name} is too long");
            let loudest = wav[44..]
                .chunks(2)
                .map(|s| i16::from_le_bytes([s[0], s[1]]).unsigned_abs())
                .max()
                .unwrap_or(0);
            let loudest = f32::from(loudest) / f32::from(i16::MAX);
            assert!((loudest - PEAK).abs() < 0.01, "{name} peaks at {loudest}");
        }
    }

    #[test]
    fn the_kinds_cover_every_built_in_sound() {
        let covered: usize = KINDS.iter().map(|(_, n)| n).sum();
        assert_eq!(covered, BUILT_IN.len());
        let tones = KINDS.last().unwrap().1;
        assert!(
            BUILT_IN[BUILT_IN.len() - tones..]
                .iter()
                .all(|(name, _)| !RECORDED.iter().any(|(r, _)| r == name))
        );
    }

    #[test]
    fn every_built_in_sound_has_one_name_and_a_wav() {
        let mut seen = HashSet::new();
        for (name, _) in BUILT_IN {
            assert!(seen.insert(name), "{name} is listed twice");
            assert!(bytes(name).is_some(), "{name} has no sound");
        }
        assert_eq!(
            RECORDED.len() + 7,
            BUILT_IN.len(),
            "every recording and tone is listed"
        );
    }

    #[test]
    fn wav_headers_describe_the_data() {
        let bytes = wav(&[0.0, 0.5, -0.5]);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 36 + 6);
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), RATE);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 6);
        assert_eq!(bytes.len(), 44 + 6);
    }
}

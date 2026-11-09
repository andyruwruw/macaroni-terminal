//! WCAG contrast: relative luminance, the contrast ratio between two colors, and moving a text
//! color just far enough toward white or black to reach a minimum ratio against its
//! background (`[accessibility] minimum_contrast`).

use iced::Color;

/// How many halvings the search for the least change takes; past this the step is under a
/// thousandth of the way, finer than 8-bit color.
const STEPS: usize = 10;

/// The WCAG relative luminance of an opaque color, 0 (black) to 1 (white).
pub fn luminance(c: Color) -> f32 {
    /// One sRGB channel made linear.
    fn linear(v: f32) -> f32 {
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
}

/// The WCAG contrast ratio between two opaque colors, 1 to 21.
pub fn ratio(a: Color, b: Color) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// `fg` (drawn with its alpha over `bg`) with at least `min` contrast against `bg`: unchanged
/// when it has it, else opaque and moved toward white or black, whichever can reach more, by as
/// little as reaches `min` (all the way when even that can't).
pub fn ensure(fg: Color, bg: Color, min: f32) -> Color {
    let bg = Color { a: 1.0, ..bg };
    let shown = mix(bg, Color { a: 1.0, ..fg }, fg.a);
    if min <= 1.0 || ratio(shown, bg) >= min {
        return fg;
    }
    let target = if ratio(Color::WHITE, bg) >= ratio(Color::BLACK, bg) {
        Color::WHITE
    } else {
        Color::BLACK
    };
    if ratio(target, bg) <= min {
        return target;
    }
    // Contrast grows steadily on the way to the target, so halve toward the least mix that
    // reaches `min`.
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..STEPS {
        let mid = (low + high) / 2.0;
        if ratio(mix(shown, target, mid), bg) >= min {
            high = mid;
        } else {
            low = mid;
        }
    }
    mix(shown, target, high)
}

/// `a` moved `t` of the way to `b`, opaque.
fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgb(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratios_match_wcag() {
        assert!((ratio(Color::BLACK, Color::WHITE) - 21.0).abs() < 0.01);
        assert!((ratio(Color::WHITE, Color::WHITE) - 1.0).abs() < 0.01);
        // #777777 on white is the classic just-under-AA grey.
        let grey = Color::from_rgb8(0x77, 0x77, 0x77);
        assert!((ratio(grey, Color::WHITE) - 4.48).abs() < 0.02);
    }

    #[test]
    fn low_contrast_text_is_moved_just_enough() {
        let bg = Color::from_rgb8(0x12, 0x13, 0x14);
        let fg = Color::from_rgb8(0x3b, 0x42, 0x52);
        assert!(ratio(fg, bg) < 4.5);
        let fixed = ensure(fg, bg, 4.5);
        let r = ratio(fixed, bg);
        assert!((4.5..4.7).contains(&r), "ratio {r}");
        // Lighter, since the background is dark.
        assert!(luminance(fixed) > luminance(fg));
    }

    #[test]
    fn enough_contrast_is_left_alone() {
        let fg = Color::from_rgba(0.9, 0.9, 0.9, 1.0);
        assert_eq!(ensure(fg, Color::BLACK, 4.5), fg);
        assert_eq!(ensure(Color::BLACK, Color::BLACK, 1.0), Color::BLACK);
    }

    #[test]
    fn dim_text_counts_its_alpha() {
        // Half-transparent white over black shows as mid grey, which falls short of 21.
        let fg = Color::from_rgba(1.0, 1.0, 1.0, 0.5);
        let fixed = ensure(fg, Color::BLACK, 21.0);
        assert_eq!(fixed, Color::WHITE);
    }
}

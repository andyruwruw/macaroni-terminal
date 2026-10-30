//! The glass theme's surfaces: a translucent fill lit from the bottom left (darkest at the top
//! right), and a thin rim that catches the light along the top and bottom edges. The fill is the
//! sheet's background; the rim is a shader (`backdrop::rim`; iced borders take one color), laid
//! over the content so opaque content, like a menu's background, can't hide it.
//!
//! Frosted sheets (`frosted`, `panel`) sit on the window's backdrop, blurred (`backdrop`).
//! Popups and what's on them use plain `sheet`: the backdrop isn't what's behind them.
//!
//! Every function here returns its content unchanged when [`Chrome::glass`] is unset (the solid
//! look), so the view can call them unconditionally. The view uses [`frosted`] for sidebar items,
//! tabs and the title bar, [`panel`] for pane cards and [`sheet`] for menus and the settings
//! dialog.
//!
//! Don't draw any of this with canvas meshes: iced's wgpu renderer misplaces meshes when several
//! canvases are on screen (fills under content vanished or stacked up; rims drew stray segments
//! in the gaps between tabs). Shader primitives land where they're laid out.

use iced::gradient::Linear;
use iced::widget::{Stack, container};
use iced::{Background, Border, Color, Degrees, Element, Padding, Shadow, Size, Vector};

use crate::backdrop;
use crate::theme::Chrome;

/// Up to this diagonal (about a tab's), a `panel`'s fill is at full strength.
const SMALL_DIAGONAL: f32 = 320.0;

/// The glass look's colors, at full strength.
#[derive(Debug, Clone, Copy)]
pub struct Glass {
    /// Fill at the bottom-left corner.
    pub light: Color,
    /// Fill at the top-right corner.
    pub dark: Color,
    /// The rim all round.
    pub rim: Color,
    /// The rim's alpha where it catches the light.
    pub shine: f32,
}

/// `content` on a glass sheet, if the theme is glass; otherwise `content` as it is. `strength`
/// scales the fill and rim (1 for an active item, less for quieter ones); the shine falls off
/// much faster than the rest, so quiet items barely catch the light.
pub fn sheet<'a, M: 'a>(
    c: Chrome,
    content: impl Into<Element<'a, M>>,
    radius: f32,
    strength: f32,
) -> Element<'a, M> {
    sheet_with(c, content, radius, strength, strength, None)
}

/// A `sheet` that blurs the window's backdrop behind it. `lift`, 0 to 1, turns it into thicker,
/// clearer glass that refracts the backdrop at its edges (for a selected item; see also
/// `lift_padding` and `lift_shadow`).
pub fn frosted<'a, M: 'a>(
    c: Chrome,
    content: impl Into<Element<'a, M>>,
    radius: f32,
    strength: f32,
    lift: f32,
) -> Element<'a, M> {
    sheet_with(c, content, radius, strength, strength, Some(lift))
}

/// Splits a lifting item's `padding` into an outer one, around its sheet, and an inner one, in
/// it. Together they always add up to `padding`, give or take the rise, so the item's slot
/// keeps its size: at rest the sheet sits a little inside its slot, and as it lifts it grows to
/// fill it and its content rises. Solid has no lift: everything stays inside.
pub fn lift_padding(c: Chrome, padding: Padding, lift: f32) -> (Padding, Padding) {
    if c.glass.is_none() {
        return (Padding::ZERO, padding);
    }
    // How far a fully lifted sheet grows past its slot on each side, and rises (no more than
    // it grows, so it stays in its slot).
    let grow = c.s.effects.lift;
    let rise = grow;
    let rest = 1.0 - lift;
    let outer = Padding {
        top: (grow + rise) * rest,
        bottom: (grow - rise) * rest,
        left: grow * rest,
        right: grow * rest,
    };
    let inner = Padding {
        top: (padding.top - grow - rise + grow * lift).max(0.0),
        bottom: (padding.bottom - grow + rise + grow * lift).max(0.0),
        left: (padding.left - grow + grow * lift).max(0.0),
        right: (padding.right - grow + grow * lift).max(0.0),
    };
    (outer, inner)
}

/// `shadow` for a lifting item: faint and tight at rest, a little deeper and softer lifted.
pub fn lift_shadow(c: Chrome, shadow: Shadow, lift: f32) -> Shadow {
    if c.glass.is_none() {
        return shadow;
    }
    let rest = 1.0 - lift;
    Shadow {
        color: shadow.color.scale_alpha(0.5 * rest + 1.3 * lift),
        offset: Vector::new(
            shadow.offset.x,
            shadow.offset.y * 0.5 * rest + (shadow.offset.y + 1.0) * lift,
        ),
        blur_radius: shadow.blur_radius * 0.6 * rest + (shadow.blur_radius + 3.0) * lift,
    }
}

/// A frosted sheet for a surface about `size` big, like a pane card: full-strength rim, and a
/// fill whose light side dims as the surface grows, so a large panel doesn't read as a milky
/// slab.
pub fn panel<'a, M: 'a>(
    c: Chrome,
    content: impl Into<Element<'a, M>>,
    radius: f32,
    size: Size,
) -> Element<'a, M> {
    quiet_panel(c, content, radius, size, 1.0)
}

/// [`panel`] with its rim at `strength` (its edge a little fainter, its shine by the cube), for
/// a small control that should sit back rather than catch the light like a pane card.
pub fn quiet_panel<'a, M: 'a>(
    c: Chrome,
    content: impl Into<Element<'a, M>>,
    radius: f32,
    size: Size,
    strength: f32,
) -> Element<'a, M> {
    let fill = (SMALL_DIAGONAL / size.width.hypot(size.height))
        .sqrt()
        .clamp(0.3, 1.0);
    sheet_with(c, content, radius, fill, strength, Some(0.0))
}

/// The sheet: `content` on the gradient fill (`fill_strength` scales its alpha), the rim over
/// it (`strength` scales its edge a little and its shine by the cube), and with `frost`, the
/// blurred backdrop under it. A lifted sheet's backdrop sheds some blur
/// (`style.effects.lift_clear`) and refracts (`style.effects.refraction`).
fn sheet_with<'a, M: 'a>(
    c: Chrome,
    content: impl Into<Element<'a, M>>,
    radius: f32,
    fill_strength: f32,
    strength: f32,
    // Frosted sheets' lift; plain sheets don't blur.
    frost: Option<f32>,
) -> Element<'a, M> {
    let content = content.into();
    let Some(glass) = c.glass else {
        return content;
    };
    let fill = Linear::new(Degrees(45.0))
        .add_stop(0.0, glass.light.scale_alpha(fill_strength))
        .add_stop(1.0, glass.dark.scale_alpha(fill_strength));
    let content = container(content).style(move |_| container::Style {
        background: Some(Background::Gradient(fill.into())),
        border: Border {
            radius: radius.into(),
            ..Border::default()
        },
        ..container::Style::default()
    });
    // The rim is the stack's top layer, so opaque content can't hide it; the backdrop goes
    // under with `push_under`, in a layer below the content (see `backdrop`'s trap).
    let sheet = Stack::with_children([content.into()]).push(backdrop::rim(
        radius,
        glass.rim.scale_alpha(0.5 + 0.5 * strength),
        glass.shine * strength.powi(3),
    ));
    match frost {
        Some(lift) => sheet
            .push_under(backdrop::behind(
                radius,
                c.s.effects.blur * (1.0 - c.s.effects.lift_clear * lift),
                lift * c.s.effects.refraction,
                c.backdrop,
            ))
            .into(),
        None => sheet.into(),
    }
}

// The glass theme's window background, the part of it behind one sheet blurred, or a sheet's
// rim (`mode` 1). `backdrop.rs` builds the pipeline and fills the two uniforms below.
//
// Each primitive draws one full-screen triangle; iced clips the pass to the widget's bounds, and
// the fragment stage works in window coordinates (`frag.xy`), so the backdrop behind a sheet
// lines up with the window's backdrop around it. Corners come from the rounded rectangle's
// signed distance (`sdf`), as alpha.
//
// The background is a diagonal gradient with soft color glows and a fine grain. Each glow is a
// Gaussian, and a Gaussian blurred by a Gaussian is another one (variances add, peak scales by
// the ratio), so a sheet draws exactly what a blur of the background behind it would show. The
// grain averages out under any real blur, so sheets leave it off.

// One primitive's parameters (binding 0, one slot per primitive, bound by dynamic offset). The
// field order and padding match the `[f32; FLOATS]` array `backdrop.rs` writes in `prepare`.
struct Sheet {
    // The widget's bounds (x, y, width, height), logical pixels, in the window.
    rect: vec4<f32>,
    // The rim all round (sRGB, straight alpha).
    color: vec4<f32>,
    // The window's logical size; the scene is laid out relative to it.
    window: vec2<f32>,
    // Physical pixels per logical pixel.
    scale: f32,
    // Corner radius, logical pixels.
    radius: f32,
    // The blur's standard deviation, logical pixels (0 for the window's own backdrop).
    blur: f32,
    // 1 when the target is sRGB, so colors written here (sRGB) must be linearized.
    linear_out: f32,
    // 0 draws the backdrop, 1 the rim.
    mode: f32,
    // How far the highlights peak from the middle of their edge, as a fraction of its length.
    shift: f32,
    // The highlights' alpha at their peak (`color` is the rest of the rim).
    shine: f32,
    // 0 to 1: how much the sheet acts as a thick lens (see `lens`).
    refract: f32,
}

// The rim's thickness, logical pixels.
const RIM_WIDTH: f32 = 0.75;
// How far in from the edge a refracting sheet bends the backdrop, logical pixels.
const LENS_ZONE: f32 = 14.0;
// How far the backdrop is pulled in from beyond the edge, at the edge, logical pixels.
const LENS_SHIFT: f32 = 28.0;
// How much farther red bends than blue, as a fraction of the shift.
const DISPERSION: f32 = 0.35;

// The gradient's two ends, then the glows (alpha is each one's strength), in sRGB. Binding 1,
// shared by every primitive: `Chrome.backdrop`, written once per frame by the first backdrop
// primitive's `prepare`.
struct Scene {
    colors: array<vec4<f32>, 7>,
}

@group(0) @binding(0) var<uniform> sheet: Sheet;
@group(0) @binding(1) var<uniform> palette: Scene;

// Vertex stage: indices 0, 1, 2 become (-1,-1), (3,-1), (-1,3) in clip space, one triangle
// covering the whole target, with no vertex buffer.
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

// A Gaussian glow of standard deviation `sigma` at `at`, as seen through this sheet's blur: the
// blurred Gaussian's variance is the sum of both, and its peak drops by the ratio of variances
// (in 2D the peak scales with 1/variance), so the glow's total light is unchanged.
fn glow(p: vec2<f32>, at: vec2<f32>, sigma: f32, color: vec3<f32>, amount: f32) -> vec3<f32> {
    let s2 = sigma * sigma;
    let total = s2 + sheet.blur * sheet.blur;
    let d = p - at;
    return color * amount * (s2 / total) * exp(-dot(d, d) / (2.0 * total));
}

// A cheap per-pixel pseudo-random value in 0..1, for the grain.
fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

// The sRGB transfer function's inverse, for sRGB render targets (which re-encode on write).
fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

// A highlight's brightness along an edge: 0 at its ends (0 and 1), 1 at `at`.
fn profile(u: f32, at: f32) -> f32 {
    if u < at {
        return smoothstep(0.0, at, u);
    }
    return 1.0 - smoothstep(at, 1.0, u);
}

// Signed distance from the sheet's rounded rectangle, logical pixels (negative inside).
fn sdf(p: vec2<f32>) -> f32 {
    let half = sheet.rect.zw * 0.5;
    let q = abs(p - sheet.rect.xy - half) - half + sheet.radius;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - sheet.radius;
}

// The backdrop at window point `p`: the gradient runs from the top left to the bottom right, and
// the glows' positions and sizes are fractions of the window, so the scene scales with it.
fn scene(p: vec2<f32>) -> vec3<f32> {
    let size = sheet.window;
    let diag = length(size);

    let t = clamp(dot(p / size, vec2<f32>(0.5, 0.5)), 0.0, 1.0);
    let k = palette.colors;
    var c = mix(k[0].rgb, k[1].rgb, t);
    c += glow(p, size * vec2<f32>(0.08, 0.12), diag * 0.24, k[2].rgb, k[2].a);
    c += glow(p, size * vec2<f32>(0.92, 0.22), diag * 0.17, k[3].rgb, k[3].a);
    c += glow(p, size * vec2<f32>(0.70, 0.98), diag * 0.20, k[4].rgb, k[4].a);
    c += glow(p, size * vec2<f32>(0.34, 0.72), diag * 0.06, k[5].rgb, k[5].a);
    c += glow(p, size * vec2<f32>(0.56, 0.34), diag * 0.045, k[6].rgb, k[6].a);
    return c;
}

// The outward normal of the sheet's outline nearest `p`: the SDF's gradient by central
// differences half a pixel apart.
fn normal(p: vec2<f32>) -> vec2<f32> {
    let e = vec2<f32>(0.5, 0.0);
    let g = vec2<f32>(sdf(p + e.xy) - sdf(p - e.xy), sdf(p + e.yx) - sdf(p - e.yx));
    return g / max(length(g), 1e-4);
}

// The backdrop seen through a thick sheet: toward the edges it bends, pulling in what lies
// beyond the edge and splitting it slightly by color, and the edges catch light at the top left
// and fall into shade at the bottom right.
fn lens(p: vec2<f32>) -> vec3<f32> {
    let depth = max(-sdf(p), 0.0);
    let n = normal(p);
    let edge = 1.0 - smoothstep(0.0, LENS_ZONE, depth);
    let bend = edge * edge * sheet.refract * LENS_SHIFT;
    let r = scene(p + n * bend * (1.0 + DISPERSION)).r;
    let g = scene(p + n * bend).g;
    let b = scene(p + n * bend * (1.0 - DISPERSION)).b;
    var c = vec3<f32>(r, g, b);
    let lip = 1.0 - smoothstep(0.0, 5.0, depth);
    let facing = dot(n, normalize(vec2<f32>(-0.6, -0.8)));
    c += vec3<f32>(0.11) * lip * lip * max(facing, 0.0) * sheet.refract;
    c *= 1.0 - 0.18 * edge * max(-facing, 0.0) * sheet.refract;
    return c * (1.0 + 0.10 * sheet.refract) + 0.008 * sheet.refract;
}

// The rim: a `RIM_WIDTH` band just inside the outline in `color`, plus the highlights. The band is
// the difference of two antialiased coverages (outline, and outline moved in by the width). The
// highlight profile runs along the straight part of the edge only (`u`), so it doesn't move with
// the sheet's size; the top one peaks left of center and the dimmer bottom one right of it.
fn rim(p: vec2<f32>) -> vec4<f32> {
    let dist = sdf(p);
    let band = clamp(0.5 - dist * sheet.scale, 0.0, 1.0)
        - clamp(0.5 - (dist + RIM_WIDTH) * sheet.scale, 0.0, 1.0);
    let local = p - sheet.rect.xy;
    let straight = max(sheet.rect.z - 2.0 * sheet.radius, 1.0);
    let u = clamp((local.x - sheet.radius) / straight, 0.0, 1.0);
    var light: f32;
    if local.y < sheet.rect.w * 0.5 {
        light = profile(u, 0.5 - sheet.shift);
    } else {
        light = 0.85 * profile(u, 0.5 + sheet.shift);
    }
    var c = sheet.color.rgb;
    if sheet.linear_out > 0.5 {
        c = to_linear(c);
    }
    return vec4<f32>(c, band * min(sheet.color.a + sheet.shine * light, 1.0));
}

// Fragment stage: the rim in rim mode, else the backdrop (through the lens when refracting), with
// grain only when unblurred, clipped to the rounded rectangle by antialiased alpha.
@fragment
fn fs(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let p = frag.xy / sheet.scale;
    if sheet.mode > 0.5 {
        return rim(p);
    }
    var c: vec3<f32>;
    if sheet.refract > 0.0 {
        c = lens(p);
    } else {
        c = scene(p);
    }
    if sheet.blur == 0.0 {
        c += (hash(floor(frag.xy)) - 0.5) * 0.025;
    }

    let alpha = clamp(0.5 - sdf(p) * sheet.scale, 0.0, 1.0);

    if sheet.linear_out > 0.5 {
        c = to_linear(c);
    }
    return vec4<f32>(c, alpha);
}

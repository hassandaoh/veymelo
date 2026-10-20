//! Noise & Grain, part one: made noise (Fractal Noise, Turbulent Noise), flow from noise (Curl
//! Noise), and noise sprinkled on the layer (Noise, Noise Alpha, Noise HLS, Noise HLS Auto).
//!
//! Fractal and Turbulent Noise are one engine: layers of value noise (a random number on each
//! whole point of a grid, in x, y and the evolution, and something smooth between them), each
//! layer finer, fainter, turned and shifted as the sub settings say, shaped by the fractal type,
//! then contrast, brightness and overflow. Sizes are in layer units, so a preview shows the same
//! noise as the full picture.

use veymelo_color::BlendMode;
use veymelo_geom::vec2;
use veymelo_raster::{Px, Sampling};

use crate::defs::{angle, choice, effect, flag, num, pct, point, px, ranged};
use crate::util::{hsl, map_px, premul, rgb, unpremul};
use crate::{Buf, Ctx, Spec};

const CATEGORY: &str = "noise";

/// The grid's number at a whole point, from 0 to 1.
#[inline]
fn lattice(x: i64, y: i64, z: i64, seed: u64) -> f32 {
    let h = crate::noise::hash((x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ (z as u64).wrapping_mul(0x1656_67B1_9E37_79F9) ^ seed);
    (h >> 40) as f32 / (1u64 << 24) as f32
}

const FRACTAL_TYPES: &[&str] = &["basic", "turbulentSmooth", "turbulentBasic", "turbulentSharp", "dynamic", "dynamicTwist", "dynamicProgressive", "max", "strings"];
const NOISE_TYPES: &[&str] = &["block", "linear", "softLinear", "spline"];
const OVERFLOWS: &[&str] = &["clip", "softClamp", "wrapBack", "allowHdrResults"];
const BLENDS: &[&str] = &[
    "none",
    "normal",
    "add",
    "multiply",
    "screen",
    "overlay",
    "softLight",
    "hardLight",
    "colorDodge",
    "colorBurn",
    "darken",
    "lighten",
    "difference",
    "exclusion",
    "hue",
    "saturation",
    "color",
    "luminosity",
    "stencilAlpha",
    "stencilLuma",
    "silhouetteAlpha",
    "silhouetteLuma",
    "alphaAdd",
    "luminescentPremul",
];
const BLEND_OF: [BlendMode; 24] = [
    BlendMode::Normal,
    BlendMode::Normal,
    BlendMode::Add,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::SoftLight,
    BlendMode::HardLight,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
    BlendMode::StencilAlpha,
    BlendMode::StencilLuma,
    BlendMode::SilhouetteAlpha,
    BlendMode::SilhouetteLuma,
    BlendMode::AlphaAdd,
    BlendMode::LuminescentPremul,
];

/// Soft linear (a quadratic B-spline) keeps 0.55 of the grid's spread over a pixel in each
/// direction; this brings it up to that of a quintic fade (181/231 in each), so the noise has the
/// contrast one expects.
const SOFT_GAIN: f32 = (181.0 / 231.0) / 0.55;

#[inline]
fn catmull(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    0.5 * (2.0 * p1 + (p2 - p0) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t * t * t)
}

/// Value noise at (x, y, z), about 0 to 1, smoothed between whole points as `kind` says in x and y
/// (block, linear, soft linear, spline), always smoothly in z. With a `period` it repeats every
/// that many along z.
pub fn value(x: f32, y: f32, z: f32, seed: u64, kind: u32, period: Option<i64>) -> f32 {
    // soft linear's three points are round the nearest whole point, the others start at the cell's
    let h = if kind == 2 { 0.5 } else { 0.0 };
    let (x0, y0, z0) = ((x + h).floor(), (y + h).floor(), z.floor());
    let (tx, ty, tz) = (x + h - x0, y + h - y0, z - z0);
    let fz = tz * tz * tz * (tz * (tz * 6.0 - 15.0) + 10.0);
    let (ix, iy, iz) = (x0 as i64, y0 as i64, z0 as i64);
    let wrap = |k: i64| match period {
        Some(p) if p > 0 => k.rem_euclid(p),
        _ => k,
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let slice = |dz: i64| -> f32 {
        let kz = wrap(iz + dz);
        let c = |dx: i64, dy: i64| lattice(ix + dx, iy + dy, kz, seed);
        match kind {
            0 => c(0, 0),
            1 => lerp(lerp(c(0, 0), c(1, 0), tx), lerp(c(0, 1), c(1, 1), tx), ty),
            2 => {
                let w = |t: f32| {
                    let s = 1.0 - t;
                    [0.5 * s * s, 0.5 + t * s, 0.5 * t * t]
                };
                let (wx, wy) = (w(tx), w(ty));
                let row = |dy: i64| wx[0] * c(-1, dy) + wx[1] * c(0, dy) + wx[2] * c(1, dy);
                wy[0] * row(-1) + wy[1] * row(0) + wy[2] * row(1)
            }
            _ => {
                let row = |dy: i64| catmull(c(-1, dy), c(0, dy), c(1, dy), c(2, dy), tx);
                catmull(row(-1), row(0), row(1), row(2), ty)
            }
        }
    };
    let n = lerp(slice(0), slice(1), fz);
    if kind == 2 { 0.5 + (n - 0.5) * SOFT_GAIN } else { n }
}

/// A layer of noise shaped by the fractal type: folded (turbulent), kept to thin lines (strings).
#[inline]
fn shape(kind: u32, n: f32) -> f32 {
    let s = n * 2.0 - 1.0;
    match kind {
        1 => 1.0 - s * s,
        2 => 1.0 - s.abs(),
        3 => (1.0 - s.abs()).powi(2),
        8 => 1.0 - (s.abs() / 0.12).clamp(0.0, 1.0).powi(2),
        _ => n,
    }
}

/// A value past 0 to 1 brought back as the overflow says.
pub fn overflow(v: f32, how: u32) -> f32 {
    match how {
        // as it is in the middle, easing toward black and white without reaching them
        1 if v > 0.8 => 0.8 + 0.2 * ((v - 0.8) / 0.2).tanh(),
        1 if v < 0.2 => 0.2 - 0.2 * ((0.2 - v) / 0.2).tanh(),
        1 => v,
        2 => {
            let t = v.rem_euclid(2.0);
            if t > 1.0 { 2.0 - t } else { t }
        }
        3 => v,
        _ => v.clamp(0.0, 1.0),
    }
}

/// Fractal or Turbulent Noise's settings, worked out once (sizes in buffer pixels).
pub struct Fractal {
    kind: u32,
    noise: u32,
    contrast: f32,
    brightness: f32,
    overflow: u32,
    invert: bool,
    turn: (f32, f32),
    size: [f32; 2],
    origin: [f32; 2],
    centre: [f32; 2],
    perspective: bool,
    layers: f32,
    influence: f32,
    sub_scaling: f32,
    sub_turn: f32,
    sub_offset: [f32; 2],
    centre_subscale: bool,
    evolution: f32,
    period: Option<i64>,
    turbulence: f32,
    seed: u64,
}

impl Fractal {
    pub fn of(ctx: &Ctx, b: &Buf, turbulent: bool) -> Fractal {
        let uniform = ctx.flag("uniformScaling");
        let s = ctx.num("scale", 100.0);
        let (sw, sh) = if uniform { (s, s) } else { (ctx.num("scaleWidth", 100.0), ctx.num("scaleHeight", 100.0)) };
        let o = b.pixel(ctx.point("offsetTurbulence"));
        let c = b.pixel(ctx.bounds.center());
        let so = ctx.point("subOffset");
        Fractal {
            kind: ctx.choice("fractalType"),
            noise: ctx.choice("noiseType"),
            contrast: (ctx.num("contrast", 100.0) / 100.0) as f32,
            brightness: (ctx.num("brightness", 0.0) / 100.0) as f32,
            overflow: ctx.choice("overflow"),
            invert: ctx.flag("invert"),
            turn: (ctx.num("rotation", 0.0).to_radians() as f32).sin_cos(),
            size: [(sw * b.scale).max(1e-3) as f32, (sh * b.scale).max(1e-3) as f32],
            origin: [o.x as f32, o.y as f32],
            centre: [c.x as f32, c.y as f32],
            perspective: ctx.flag("perspectiveOffset"),
            layers: ctx.num("complexity", 6.0).clamp(1.0, 20.0) as f32,
            influence: (ctx.num("subInfluence", 70.0) / 100.0).clamp(0.0, 1.0) as f32,
            sub_scaling: (ctx.num("subScaling", 56.0) / 100.0).clamp(0.1, 1.0) as f32,
            sub_turn: if turbulent { 0.0 } else { ctx.num("subRotation", 0.0).to_radians() as f32 },
            sub_offset: if turbulent { [0.0; 2] } else { [(so.x * b.scale) as f32, (so.y * b.scale) as f32] },
            centre_subscale: !turbulent && ctx.flag("centerSubscale"),
            evolution: (ctx.num("evolution", 0.0) / 360.0) as f32,
            period: (!turbulent && ctx.flag("cycleEvolution")).then(|| ctx.num("cycle", 1.0).round().max(1.0) as i64),
            turbulence: if turbulent { ctx.num("turbulenceFactor", 0.5).max(0.0) as f32 } else { 0.0 },
            seed: crate::noise::mix(&[if turbulent { 0x7a3d } else { 0x51ed }, ctx.num("randomSeed", 0.0).max(0.0) as u64]),
        }
    }

    /// One layer at grid point (u, v): finer layers of turbulent noise evolve faster.
    #[inline]
    fn layer(&self, u: f32, v: f32, o: f32, z: f32, seed: u64) -> f32 {
        let e = if self.period.is_some() { self.evolution } else { self.evolution * (1.0 + self.turbulence * o) };
        value(u, v, e + z, seed, self.noise, self.period)
    }

    /// The layers summed at buffer pixel (x, y), before contrast.
    pub fn raw(&self, x: f32, y: f32) -> f32 {
        let n = self.layers.ceil() as usize;
        let part = self.layers - self.layers.floor();
        let (sr, cr) = self.turn;
        let (mut sum, mut total, mut strength, mut fine, mut best, mut depth) = (0f32, 0f32, 1f32, 1f32, 0f32, 1f32);
        for o in 0..n {
            let w = if o + 1 == n && part > 0.0 { part } else { 1.0 };
            let of = o as f32;
            // the offset moves the finer (farther) layers less with perspective, more with turbulence
            let k = if self.perspective { depth } else { 1.0 } * (1.0 + self.turbulence * of);
            let mut org = self.origin;
            if k != 1.0 {
                org = [self.centre[0] + (self.origin[0] - self.centre[0]) * k, self.centre[1] + (self.origin[1] - self.centre[1]) * k];
            }
            if !self.centre_subscale {
                org = [org[0] + self.sub_offset[0] * of, org[1] + self.sub_offset[1] * of];
            }
            let (dx, dy) = (x - org[0], y - org[1]);
            let (mut u, mut v) = ((dx * cr + dy * sr) / self.size[0], (-dx * sr + dy * cr) / self.size[1]);
            if self.sub_turn != 0.0 {
                let (s, c) = (self.sub_turn * of).sin_cos();
                (u, v) = (u * c + v * s, -u * s + v * c);
            }
            let (mut u, mut v) = (u * fine, v * fine);
            let z = of * 7.31;
            let seed = crate::noise::mix(&[self.seed, o as u64]);
            if (4..=6).contains(&self.kind) {
                // dynamic: each layer pushed about by a coarser field of its own (twist turns it,
                // progressive pushes the finer layers more)
                let k = if self.kind == 6 { 0.5 + of * 0.35 } else { 1.0 };
                let a = self.layer(u * 0.5, v * 0.5, of, z + 3.1, seed ^ 0x55) - 0.5;
                let c = self.layer(u * 0.5, v * 0.5, of, z + 5.7, seed ^ 0xaa) - 0.5;
                if self.kind == 5 {
                    let (s, co) = (a * 3.0).sin_cos();
                    (u, v) = (u * co - v * s, u * s + v * co);
                } else {
                    u += a * 1.5 * k;
                    v += c * 1.5 * k;
                }
            }
            let s = shape(self.kind, self.layer(u, v, of, z, seed));
            if self.kind == 7 {
                best = best.max(s * strength);
            }
            sum += s * strength * w;
            total += strength * w;
            strength *= self.influence;
            fine /= self.sub_scaling;
            depth *= self.sub_scaling;
        }
        if self.kind == 7 { best } else { sum / total.max(1e-6) }
    }

    /// The grey at buffer pixel (x, y): contrast, brightness, overflow and invert done.
    pub fn grey(&self, x: f32, y: f32) -> f32 {
        let v = overflow((self.raw(x, y) - 0.5) * self.contrast + 0.5 + self.brightness, self.overflow);
        if self.invert { 1.0 - v } else { v }
    }
}

macro_rules! noise_effect {
    ($def:ident, $id:literal, $name:literal, $doc:literal, $kind:expr, $over:expr, [$($sub:expr),*], [$($evo:expr),*]) => {
        effect!(
            $def,
            $id,
            $name,
            $doc,
            [
                choice("fractalType", "Fractal Type", FRACTAL_TYPES, $kind),
                choice("noiseType", "Noise Type", NOISE_TYPES, 2),
                flag("invert", "Invert", false),
                ranged("contrast", "Contrast", 100.0, 0.0, 10_000.0),
                ranged("brightness", "Brightness", 0.0, -10_000.0, 10_000.0),
                choice("overflow", "Overflow", OVERFLOWS, $over),
                angle("rotation", "Rotation", 0.0),
                flag("uniformScaling", "Uniform Scaling", true),
                ranged("scale", "Scale", 100.0, 1.0, 10_000.0),
                ranged("scaleWidth", "Scale Width", 100.0, 1.0, 10_000.0),
                ranged("scaleHeight", "Scale Height", 100.0, 1.0, 10_000.0),
                point("offsetTurbulence", "Offset Turbulence"),
                flag("perspectiveOffset", "Perspective Offset", false).doc("the finer layers move less, as if farther off"),
                ranged("complexity", "Complexity", 6.0, 1.0, 20.0).doc("how many layers"),
                pct("subInfluence", "Sub Influence (%)", 70.0, 0.0, 100.0).doc("each layer this strong next to the one before"),
                pct("subScaling", "Sub Scaling", 56.0, 10.0, 100.0).doc("each layer this big next to the one before"),
                $($sub,)*
                angle("evolution", "Evolution", 0.0).doc("turn it to animate: each turn is a new pattern, smoothly"),
                $($evo,)*
                pct("opacity", "Opacity", 100.0, 0.0, 100.0),
                choice("blendingMode", "Blending Mode", BLENDS, 1).doc("none: the noise alone"),
            ]
        );
    };
}

noise_effect!(
    FRACTAL_DEF,
    "fractalNoise",
    "Fractal Noise",
    "noise of many layers, like clouds, smoke, water or marble",
    0,
    3,
    [angle("subRotation", "Sub Rotation", 0.0), point("subOffset", "Sub Offset").doc("each layer shifted this much more (layer units)"), flag("centerSubscale", "Center Subscale", false)],
    [
        flag("cycleEvolution", "Cycle Evolution", false),
        num("cycle", "Cycle (in Revolutions)", 1.0).range(1.0, 1000.0).doc("the pattern comes back after this many turns"),
        num("randomSeed", "Random Seed", 0.0).range(0.0, 1_000_000.0)
    ]
);
pub const FRACTAL: Spec = Spec::new(&FRACTAL_DEF, CATEGORY, fractal_noise).points(&[("offsetTurbulence", [0.5, 0.5])]);

noise_effect!(
    TURBULENT_DEF,
    "turbulentNoise",
    "Turbulent Noise",
    "fractal noise whose finer layers swirl and drift faster, like flames or rushing smoke",
    1,
    0,
    [],
    [ranged("turbulenceFactor", "Turbulence Factor", 0.5, 0.0, 10.0), num("randomSeed", "Random Seed", 0.0).range(0.0, 1_000_000.0)]
);
pub const TURBULENT: Spec = Spec::new(&TURBULENT_DEF, CATEGORY, turbulent_noise).points(&[("offsetTurbulence", [0.5, 0.5])]);

/// Grey `v` put on the layer's pixel by the mode at `opacity` (none: the noise alone), kept
/// inside the layer's alpha.
fn put(p: Px, v: f32, opacity: f32, mode: u32) -> Px {
    let (c, a) = unpremul(p);
    if a <= 0.0 {
        return p;
    }
    let g = [v * opacity, v * opacity, v * opacity, opacity];
    let out = if mode == 0 { g } else { veymelo_color::blend(BLEND_OF[(mode as usize).min(23)], [c[0], c[1], c[2], 1.0], g, 0.5) };
    out.map(|k| k * a)
}

fn noise_on(ctx: &Ctx, mut b: Buf, turbulent: bool) -> Buf {
    let f = Fractal::of(ctx, &b, turbulent);
    let opacity = (ctx.num("opacity", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
    let mode = ctx.choice("blendingMode");
    map_px(&mut b.img, |x, y, p| put(p, f.grey(x as f32 + 0.5, y as f32 + 0.5), opacity, mode));
    b
}

fn fractal_noise(ctx: &Ctx, b: Buf) -> Buf {
    noise_on(ctx, b, false)
}

fn turbulent_noise(ctx: &Ctx, b: Buf) -> Buf {
    noise_on(ctx, b, true)
}

effect!(
    CURL_DEF,
    "curlNoise",
    "Curl Noise",
    "the picture carried along a flow that swirls without bunching up (the curl of a noise field)",
    [
        choice("view", "View", &["finalOutput", "flowField"], 0),
        ranged("displacementAmount", "Displacement Amount", 20.0, -1000.0, 1000.0),
        ranged("scale", "Scale", 100.0, 1.0, 10_000.0),
        angle("rotation", "Rotation", 0.0),
        point("offsetTurbulence", "Offset Turbulence"),
        ranged("complexity", "Complexity", 3.0, 1.0, 20.0),
        angle("evolution", "Evolution", 0.0),
        ranged("flowSteps", "Flow Steps", 4.0, 1.0, 16.0),
        choice("edgeBehavior", "Edge Behavior", &["clamp", "wrap"], 0),
        num("randomSeed", "Random Seed", 0.0).range(0.0, 1_000_000.0),
    ]
);
pub const CURL: Spec = Spec::new(&CURL_DEF, CATEGORY, curl).points(&[("offsetTurbulence", [0.5, 0.5])]);

fn curl(ctx: &Ctx, mut b: Buf) -> Buf {
    let amount = ctx.num("displacementAmount", 20.0) * b.scale;
    let view = ctx.choice("view");
    if amount.abs() < 1e-6 && view == 0 {
        return b;
    }
    let size = ctx.num("scale", 100.0).max(1.0) * b.scale;
    let octaves = ctx.num("complexity", 3.0);
    let (sr, cr) = ctx.num("rotation", 0.0).to_radians().sin_cos();
    let o = b.pixel(ctx.point("offsetTurbulence"));
    let z = ctx.num("evolution", 0.0) / 360.0 * 4.0;
    let seed = crate::noise::mix(&[0x51_7cc1, ctx.num("randomSeed", 0.0).max(0.0) as u64]);
    let steps = ctx.num("flowSteps", 4.0).round().clamp(1.0, 16.0) as usize;
    let wrap = ctx.choice("edgeBehavior") == 1;
    let (w, h) = (b.img.width, b.img.height);
    // the noise field over the pixels, then the flow round its contours (its curl)
    let field: Vec<f64> = (0..w * h)
        .map(|k| {
            let (dx, dy) = ((k % w) as f64 + 0.5 - o.x, (k / w) as f64 + 0.5 - o.y);
            crate::noise::fractal((dx * cr + dy * sr) / size, (-dx * sr + dy * cr) / size, z, seed, octaves, 0.5, None)
        })
        .collect();
    let at = |x: i64, y: i64| field[(y.clamp(0, h as i64 - 1) as usize) * w + x.clamp(0, w as i64 - 1) as usize];
    let mut flow = veymelo_raster::Image::new(w, h);
    for (k, f) in flow.px.iter_mut().enumerate() {
        let (x, y) = ((k % w) as i64, (k / w) as i64);
        let (gx, gy) = ((at(x + 1, y) - at(x - 1, y)) * 0.5, (at(x, y + 1) - at(x, y - 1)) * 0.5);
        // about a pixel a unit of the amount
        *f = [(gy * size * 0.5) as f32, (-gx * size * 0.5) as f32, 0.0, 0.0];
    }
    if view == 1 {
        map_px(&mut b.img, |x, y, _| {
            let v = flow.px[y * w + x];
            [(0.5 + v[0] * 0.5).clamp(0.0, 1.0), (0.5 + v[1] * 0.5).clamp(0.0, 1.0), 0.5, 1.0]
        });
        return b;
    }
    let src = b.img.clone();
    let dt = amount / steps as f64;
    let (wf, hf) = (w as f64, h as f64);
    map_px(&mut b.img, |x, y, _| {
        // followed back along the flow
        let (mut sx, mut sy) = (x as f64 + 0.5, y as f64 + 0.5);
        for _ in 0..steps {
            let v = flow.sample(Sampling::Bilinear, sx.clamp(0.5, wf - 0.5), sy.clamp(0.5, hf - 0.5));
            sx -= v[0] as f64 * dt;
            sy -= v[1] as f64 * dt;
        }
        if wrap {
            (sx, sy) = (sx.rem_euclid(wf), sy.rem_euclid(hf));
        }
        src.sample(Sampling::Bilinear, sx.clamp(0.5, wf - 0.5), sy.clamp(0.5, hf - 0.5))
    });
    b
}

/// A number from 0 to 1 for a pixel of the layer (its units) and a seed.
#[inline]
fn speck(b: &Buf, x: usize, y: usize, seed: u64) -> f32 {
    let (lx, ly) = ((b.origin.x + (x as f64 + 0.5) / b.scale).floor() as i64 as u64, (b.origin.y + (y as f64 + 0.5) / b.scale).floor() as i64 as u64);
    crate::noise::unit(crate::noise::mix(&[seed, lx, ly])) as f32
}

effect!(
    NOISE_DEF,
    "noise",
    "Noise",
    "random specks over the picture, new every frame",
    [pct("amountOfNoise", "Amount of Noise", 0.0, 0.0, 100.0), flag("useColorNoise", "Use Color Noise", true), flag("clipResultValues", "Clip Result Values", true).doc("off: past black or white wraps round"),]
);
pub const NOISE: Spec = Spec::new(&NOISE_DEF, CATEGORY, noise);

fn noise(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = (ctx.num("amountOfNoise", 0.0) / 100.0) as f32;
    if k <= 0.0 {
        return b;
    }
    let colour = ctx.flag("useColorNoise");
    let clip = ctx.flag("clipResultValues");
    let seed = crate::noise::mix(&[ctx.seed, ctx.time.seconds().to_bits()]);
    let geo = Buf { img: veymelo_raster::Image::new(0, 0), origin: b.origin, scale: b.scale };
    map_px(&mut b.img, |x, y, p| {
        let (c, a) = unpremul(p);
        if a <= 0.0 {
            return p;
        }
        let n = |i: u64| (speck(&geo, x, y, seed ^ i.wrapping_mul(0x9E37)) - 0.5) * k;
        let d = if colour { [n(0), n(1), n(2)] } else { [n(0); 3] };
        let o = [0, 1, 2].map(|i| if clip { (c[i] + d[i]).clamp(0.0, 1.0) } else { (c[i] + d[i]).rem_euclid(1.0) });
        premul(o, a)
    });
    b
}

/// A number from 0 to 1 for a pixel that changes smoothly with `phase` (each whole step a new
/// pattern), the patterns coming round again every `cycle` steps (0: never).
fn drifting(b: &Buf, x: usize, y: usize, seed: u64, phase: f32, cycle: i64) -> f32 {
    let k = phase.floor();
    let t = phase - k;
    let k = k as i64;
    let wrap = |k: i64| (if cycle > 0 { k.rem_euclid(cycle) } else { k }) as u64;
    let a = speck(b, x, y, crate::noise::mix(&[seed, wrap(k)]));
    let c = speck(b, x, y, crate::noise::mix(&[seed, wrap(k + 1)]));
    a + (c - a) * t * t * (3.0 - 2.0 * t)
}

effect!(
    NOISE_ALPHA_DEF,
    "noiseAlpha",
    "Noise Alpha",
    "random specks in the layer's alpha",
    [
        choice("noise", "Noise", &["uniformRandom", "squaredRandom", "uniformAnimation", "squaredAnimation"], 0),
        pct("amount", "Amount", 0.0, 0.0, 100.0),
        choice("originalAlpha", "Original Alpha", &["add", "clamp", "scale", "edges"], 1),
        choice("overflow", "Overflow", &["clip", "wrapBack", "wrap"], 0),
        num("randomSeed", "Random Seed", 0.0).range(0.0, 1_000_000.0),
        angle("noisePhase", "Noise Phase", 0.0),
        flag("cycleNoise", "Cycle Noise", false),
        num("cycle", "Cycle", 1.0).range(1.0, 100.0),
    ]
);
pub const NOISE_ALPHA: Spec = Spec::new(&NOISE_ALPHA_DEF, CATEGORY, noise_alpha);

fn noise_alpha(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = (ctx.num("amount", 0.0) / 100.0) as f32;
    if k <= 0.0 {
        return b;
    }
    let kind = ctx.choice("noise");
    let how = ctx.choice("originalAlpha");
    let over = ctx.choice("overflow");
    let seed = crate::noise::mix(&[0x2f1, ctx.num("randomSeed", 0.0).max(0.0) as u64]);
    // the animation kinds drift a whole pattern a second
    let phase = (ctx.num("noisePhase", 0.0) / 360.0) as f32 + if kind >= 2 { ctx.time.seconds() as f32 } else { 0.0 };
    let phase = if kind >= 2 { phase } else { phase.floor() };
    let cycle = if ctx.flag("cycleNoise") { ctx.num("cycle", 1.0).round().max(1.0) as i64 } else { 0 };
    let geo = Buf { img: veymelo_raster::Image::new(0, 0), origin: b.origin, scale: b.scale };
    map_px(&mut b.img, |x, y, p| {
        let mut n = drifting(&geo, x, y, seed, phase, cycle);
        if kind % 2 == 1 {
            n *= n;
        }
        let (c, a) = unpremul(p);
        let na = match how {
            0 => a + n * k,
            1 => {
                if a > 0.0 {
                    a + n * k
                } else {
                    0.0
                }
            }
            2 => a * (1.0 + (2.0 * n - 1.0) * k),
            _ => a + (2.0 * n - 1.0) * k * 4.0 * a * (1.0 - a),
        };
        let na = match over {
            1 => {
                let t = na.rem_euclid(2.0);
                if t > 1.0 { 2.0 - t } else { t }
            }
            2 if !(0.0..=1.0).contains(&na) => na.rem_euclid(1.0),
            _ => na,
        }
        .clamp(0.0, 1.0);
        premul(c, na)
    });
    b
}

const HLS_KINDS: &[&str] = &["uniform", "squared", "grain"];

effect!(
    NOISE_HLS_DEF,
    "noiseHls",
    "Noise HLS",
    "random changes to each pixel's hue, lightness and saturation",
    [
        choice("noise", "Noise", HLS_KINDS, 0),
        pct("hue", "Hue", 0.0, 0.0, 100.0),
        pct("lightness", "Lightness", 0.0, 0.0, 100.0),
        pct("saturation", "Saturation", 0.0, 0.0, 100.0),
        px("grainSize", "Grain Size", 1.0, 100.0),
        angle("noisePhase", "Noise Phase", 0.0),
    ]
);
pub const NOISE_HLS: Spec = Spec::new(&NOISE_HLS_DEF, CATEGORY, noise_hls);

effect!(
    NOISE_HLS_AUTO_DEF,
    "noiseHlsAuto",
    "Noise HLS Auto",
    "random changes to each pixel's hue, lightness and saturation, moving by themselves",
    [
        choice("noise", "Noise", HLS_KINDS, 0),
        pct("hue", "Hue", 0.0, 0.0, 100.0),
        pct("lightness", "Lightness", 0.0, 0.0, 100.0),
        pct("saturation", "Saturation", 0.0, 0.0, 100.0),
        px("grainSize", "Grain Size", 1.0, 100.0),
        ranged("noiseAnimationSpeed", "Noise Animation Speed", 10.0, 0.0, 1000.0).doc("new patterns a second"),
    ]
);
pub const NOISE_HLS_AUTO: Spec = Spec::new(&NOISE_HLS_AUTO_DEF, CATEGORY, noise_hls_auto);

fn hls_noise(ctx: &Ctx, mut b: Buf, phase: f32) -> Buf {
    let (dh, dl, ds) = ((ctx.num("hue", 0.0) / 100.0) as f32, (ctx.num("lightness", 0.0) / 100.0) as f32, (ctx.num("saturation", 0.0) / 100.0) as f32);
    if dh == 0.0 && dl == 0.0 && ds == 0.0 {
        return b;
    }
    let kind = ctx.choice("noise");
    let grain = ctx.num("grainSize", 1.0).max(0.1) as f32;
    let seed = crate::noise::mix(&[ctx.seed, 0x51f3]);
    let geo = Buf { img: veymelo_raster::Image::new(0, 0), origin: b.origin, scale: b.scale };
    map_px(&mut b.img, |x, y, p| {
        let (c, a) = unpremul(p);
        if a <= 0.0 {
            return p;
        }
        let n = |k: u64| -> f32 {
            let s = crate::noise::mix(&[seed, k]);
            match kind {
                // soft grains of their own size
                2 => {
                    let l = vec2(geo.origin.x + (x as f64 + 0.5) / geo.scale, geo.origin.y + (y as f64 + 0.5) / geo.scale);
                    ((value(l.x as f32 / grain, l.y as f32 / grain, phase, s, 2, None) - 0.5) * 3.0).clamp(-1.0, 1.0)
                }
                1 => {
                    let v = drifting(&geo, x, y, s, phase, 0) * 2.0 - 1.0;
                    v * v.abs()
                }
                _ => drifting(&geo, x, y, s, phase, 0) * 2.0 - 1.0,
            }
        };
        let (mut h, mut s, mut l) = hsl(c);
        if dh != 0.0 {
            h = (h + n(0) * dh * 0.5).rem_euclid(1.0);
        }
        if dl != 0.0 {
            l = (l + n(1) * dl).clamp(0.0, 1.0);
        }
        if ds != 0.0 {
            s = (s + n(2) * ds).clamp(0.0, 1.0);
        }
        premul(rgb(h, s, l), a)
    });
    b
}

fn noise_hls(ctx: &Ctx, b: Buf) -> Buf {
    let phase = (ctx.num("noisePhase", 0.0) / 360.0) as f32;
    hls_noise(ctx, b, phase)
}

fn noise_hls_auto(ctx: &Ctx, b: Buf) -> Buf {
    let phase = (ctx.time.seconds() * ctx.num("noiseAnimationSpeed", 10.0)) as f32;
    hls_noise(ctx, b, phase)
}

/// Every effect here.
pub const ALL: [Spec; 7] = [FRACTAL, TURBULENT, CURL, NOISE, NOISE_ALPHA, NOISE_HLS, NOISE_HLS_AUTO];

#[cfg(test)]
mod tests;

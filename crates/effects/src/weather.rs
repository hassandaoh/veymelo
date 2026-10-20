//! Things falling, rising and spreading over the layer: rain in streaks with the scene seen
//! through the drops (Rainfall), drifting snow (Snowfall), bubbles carrying the picture
//! (Bubbles), raindrops rippling a water surface (Drizzle) and the layer flying past as stars
//! (Star Burst).
//!
//! Each is worked out from the time alone (no frame depends on the one before), the drops,
//! flakes and bubbles at depths that make the near ones bigger, quicker and brighter.

use std::f32::consts::{PI, TAU};

use veymelo_raster::{Image, Px, Sampling};

use crate::defs::{angle, choice, color, effect, flag, pct, point, px, ranged};
use crate::sprites::{Shape, Sprite, draw, rand, rand2};
use crate::util::{map_px, unpremul};
use crate::{Buf, Ctx, Spec};

const CATEGORY: &str = "simulation";

/// How near a thing at depth `z` (0 near to 1 far) looks, from 1 down, the more so the deeper
/// the scene.
fn nearness(z: f32, depth: f32) -> f32 {
    1.0 / (1.0 + z * depth / 2500.0)
}

/// `v` brought into `lo`..`lo + span` (what goes out one side comes back the other).
fn wrap(v: f32, lo: f32, span: f32) -> f32 {
    if span <= 0.0 { v } else { lo + (v - lo).rem_euclid(span) }
}

/// The layer's colour (as it is) round buffer pixel (x, y): the mean of five samples `spread`
/// pixels apart, and how much of the layer is there.
fn background(img: &Image, x: f32, y: f32, spread: f32) -> ([f32; 3], f32) {
    let s = spread.max(0.0);
    let (mut c, mut a) = ([0f32; 3], 0f32);
    for (dx, dy) in [(0.0, 0.0), (-s, 0.0), (s, 0.0), (0.0, -s), (0.0, s)] {
        let p = img.sample(Sampling::Bilinear, (x + dx) as f64, (y + dy) as f64);
        (0..3).for_each(|k| c[k] += p[k]);
        a += p[3];
    }
    if a > 1e-6 { (c.map(|v| v / a), a / 5.0) } else { ([0.0; 3], 0.0) }
}

/// The drawn things met with the layer: laid over it, or the lighter of the two (`lighten`), or
/// alone.
fn meet(mut b: Buf, marks: Image, lighten: bool, with_layer: bool) -> Buf {
    if !with_layer {
        b.img = marks;
        return b;
    }
    for (d, m) in b.img.px.iter_mut().zip(&marks.px) {
        *d = if lighten {
            // the lighter of the two colours where both are, each as it is elsewhere
            let (ma, da) = (m[3], d[3]);
            let (mc, dc) = (unpremul(*m).0, unpremul(*d).0);
            let c = [0, 1, 2].map(|k| m[k] * (1.0 - da) + d[k] * (1.0 - ma) + ma * da * mc[k].max(dc[k]));
            [c[0], c[1], c[2], ma + da - ma * da]
        } else {
            let k = 1.0 - m[3];
            [m[0] + d[0] * k, m[1] + d[1] * k, m[2] + d[2] * k, m[3] + d[3] * k]
        };
    }
    b
}

effect!(
    RAINFALL_DEF,
    "rainfall",
    "Rainfall",
    "rain falling in streaks, near drops bigger and quicker, the scene seen through them",
    [
        ranged("drops", "Drops", 5000.0, 0.0, 100_000.0),
        ranged("size", "Size", 1.0, 0.0, 50.0),
        ranged("sceneDepth", "Scene Depth", 5000.0, 0.0, 20_000.0).doc("how far back the rain goes: the deeper, the more the far drops shrink and slow"),
        ranged("speed", "Speed", 4000.0, 0.0, 20_000.0),
        ranged("wind", "Wind", 0.0, -10_000.0, 10_000.0),
        pct("windVariation", "Variation % (Wind)", 0.0, 0.0, 100.0),
        ranged("spread", "Spread", 0.0, 0.0, 1000.0).doc("drops blown every which way"),
        color("color", "Color", 0.8, 0.8, 0.8),
        pct("opacity", "Opacity", 25.0, 0.0, 100.0),
        pct("backgroundInfluence", "Background Reflection Influence", 0.0, 0.0, 100.0).doc("how much the drops take the colour of what is behind them"),
        px("backgroundSpread", "Background Reflection Spread", 20.0, 500.0),
        choice("transferMode", "Transfer Mode", &["composite", "lighten"], 1),
        flag("compositeWithOriginal", "Composite With Original", true),
        choice("appearance", "Appearance", &["refracting", "softSolid"], 0).doc("refracting: the scene seen through each drop, brightened"),
        point("offset", "Offset"),
        pct("groundLevel", "Ground Level %", 100.0, 0.0, 100.0).doc("how far down the drops fall before they are gone"),
        ranged("randomSeed", "Random Seed", 0.0, 0.0, 10_000.0),
    ]
);
pub const RAINFALL: Spec = Spec::new(&RAINFALL_DEF, CATEGORY, rainfall).points(&[("offset", [0.5, 0.5])]);

fn rainfall(ctx: &Ctx, b: Buf) -> Buf {
    let n = ctx.num("drops", 5000.0).clamp(0.0, 100_000.0) as u64;
    let size = ctx.num("size", 1.0).max(0.0) as f32;
    let depth = ctx.num("sceneDepth", 5000.0).max(0.0) as f32;
    let speed = ctx.num("speed", 4000.0) as f32;
    let wind = ctx.num("wind", 0.0) as f32;
    let wind_var = (ctx.num("windVariation", 0.0) / 100.0) as f32;
    let spread = ctx.num("spread", 0.0) as f32;
    let col = ctx.color("color");
    let opacity = (ctx.num("opacity", 25.0) / 100.0) as f32;
    let influence = (ctx.num("backgroundInfluence", 0.0) / 100.0) as f32;
    let bg_spread = ctx.num("backgroundSpread", 20.0) as f32;
    let refract = ctx.choice("appearance") == 0;
    let ground = (ctx.num("groundLevel", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
    let seed = crate::noise::mix(&[ctx.seed, ctx.num("randomSeed", 0.0) as u64]);
    let r = ctx.bounds;
    let (lw, lh) = (r.width() as f32, r.height() as f32);
    let off = ctx.point("offset");
    let (ox, oy) = (off.x as f32 - r.center().x as f32, off.y as f32 - r.center().y as f32);
    let (t, s) = (ctx.time.seconds() as f32, b.scale as f32);
    let floor = lh * ground;
    let sprites: Vec<Sprite> = (0..n)
        .map(|i| {
            let k = nearness(rand(i, 1, seed), depth);
            // a second's fall is a few layer heights for the nearest drops
            let vy = speed / 4000.0 * lh * 2.2 * k;
            let vx = wind / 1000.0 * lh * (1.0 + wind_var * rand2(i, 2, seed)) * k + spread * rand2(i, 3, seed) * k;
            // the streak a drop draws while the shutter is open (1/48 s)
            let (ly, lx) = ((vy / 48.0).max(1.0), vx / 48.0);
            let m = ly + lx.abs() + 4.0;
            let y = wrap(rand(i, 4, seed) * (floor + 2.0 * m) + vy * t + oy, -m, floor + m);
            let x = wrap(rand(i, 5, seed) * (lw + 2.0 * m) + vx * t + ox, -m, lw + 2.0 * m);
            let p = b.pixel(veymelo_geom::vec2(r.x0 + x as f64, r.y0 + y as f64));
            let (px, py) = (p.x as f32, p.y as f32);
            let mut c = [col.r as f32, col.g as f32, col.b as f32];
            if refract {
                let (bg, a) = background(&b.img, px + 2.0 * s, py, 0.0);
                if a > 0.0 {
                    c = [0, 1, 2].map(|j| c[j] * (bg[j] * 0.6 + 0.5));
                }
            }
            if influence > 0.0 {
                let (bg, a) = background(&b.img, px, py, bg_spread * s);
                c = [0, 1, 2].map(|j| c[j] + (bg[j] - c[j]) * influence * a);
            }
            let alpha = opacity * (0.35 + 0.65 * k);
            Sprite::new(px, py, (size * 0.6 * k * s).max(0.25), [c[0], c[1], c[2], alpha], Shape::Streak { dx: lx * s, dy: ly * s })
        })
        .collect();
    let mut marks = Image::new(b.img.width, b.img.height);
    draw(&mut marks, &sprites, false);
    let (lighten, with) = (ctx.choice("transferMode") == 1, ctx.flag("compositeWithOriginal"));
    meet(b, marks, lighten, with)
}

effect!(
    SNOWFALL_DEF,
    "snowfall",
    "Snowfall",
    "snow drifting down and swaying, near flakes bigger and quicker, lit by what is behind",
    [
        ranged("flakes", "Flakes", 10_000.0, 0.0, 200_000.0),
        ranged("size", "Size", 3.0, 0.0, 50.0),
        pct("sizeVariation", "Variation % (Size)", 50.0, 0.0, 100.0),
        ranged("sceneDepth", "Scene Depth", 5000.0, 0.0, 20_000.0),
        ranged("speed", "Speed", 100.0, 0.0, 5000.0),
        pct("speedVariation", "Variation % (Speed)", 50.0, 0.0, 100.0),
        ranged("wind", "Wind", 25.0, -5000.0, 5000.0),
        pct("windVariation", "Variation % (Wind)", 50.0, 0.0, 100.0),
        ranged("spread", "Spread", 0.0, 0.0, 1000.0),
        ranged("wiggleAmount", "Wiggle Amount", 2.0, 0.0, 100.0),
        pct("wiggleAmountVariation", "Variation % (Amount)", 50.0, 0.0, 100.0),
        ranged("wiggleFrequency", "Wiggle Frequency", 1.0, 0.0, 20.0),
        pct("wiggleFrequencyVariation", "Variation % (Frequency)", 50.0, 0.0, 100.0),
        color("color", "Color", 1.0, 1.0, 1.0),
        pct("opacity", "Opacity", 100.0, 0.0, 100.0),
        pct("backgroundInfluence", "Background Illumination Influence", 0.0, 0.0, 100.0).doc("how much the flakes are lit by the colour behind them"),
        px("backgroundSpread", "Background Illumination Spread", 20.0, 500.0),
        choice("transferMode", "Transfer Mode", &["composite", "lighten"], 0),
        flag("compositeWithOriginal", "Composite With Original", true),
        point("offset", "Offset"),
        pct("groundLevel", "Ground Level %", 100.0, 0.0, 100.0),
        ranged("randomSeed", "Random Seed", 0.0, 0.0, 10_000.0),
    ]
);
pub const SNOWFALL: Spec = Spec::new(&SNOWFALL_DEF, CATEGORY, snowfall).points(&[("offset", [0.5, 0.5])]);

fn snowfall(ctx: &Ctx, b: Buf) -> Buf {
    let n = ctx.num("flakes", 10_000.0).clamp(0.0, 200_000.0) as u64;
    let f = |id: &str, d: f64| ctx.num(id, d) as f32;
    let var = |id: &str, d: f64| (ctx.num(id, d) / 100.0) as f32;
    let (size, size_var, depth) = (f("size", 3.0).max(0.0), var("sizeVariation", 50.0), f("sceneDepth", 5000.0).max(0.0));
    let (speed, speed_var, wind, wind_var, spread) = (f("speed", 100.0), var("speedVariation", 50.0), f("wind", 25.0), var("windVariation", 50.0), f("spread", 0.0));
    let (wig, wig_var, freq, freq_var) = (f("wiggleAmount", 2.0), var("wiggleAmountVariation", 50.0), f("wiggleFrequency", 1.0), var("wiggleFrequencyVariation", 50.0));
    let col = ctx.color("color");
    let (opacity, influence, bg_spread) = (var("opacity", 100.0), var("backgroundInfluence", 0.0), f("backgroundSpread", 20.0));
    let ground = var("groundLevel", 100.0).clamp(0.0, 1.0);
    let seed = crate::noise::mix(&[ctx.seed, ctx.num("randomSeed", 0.0) as u64, 7]);
    let r = ctx.bounds;
    let (lw, lh) = (r.width() as f32, r.height() as f32);
    let off = ctx.point("offset");
    let (ox, oy) = (off.x as f32 - r.center().x as f32, off.y as f32 - r.center().y as f32);
    let (t, s) = (ctx.time.seconds() as f32, b.scale as f32);
    let floor = lh * ground;
    let sprites: Vec<Sprite> = (0..n)
        .map(|i| {
            let k = nearness(rand(i, 1, seed), depth);
            let radius = (size * 0.5 * k * (1.0 + size_var * rand2(i, 2, seed))).max(0.05);
            let vy = speed * (lh / 600.0) * k * (1.0 + speed_var * rand2(i, 3, seed));
            let vx = wind * (lh / 1200.0) * k * (1.0 + wind_var * rand2(i, 4, seed)) + spread * rand2(i, 5, seed) * k;
            let amp = wig * 2.0 * k * (1.0 + wig_var * rand2(i, 6, seed));
            let fr = freq * (1.0 + freq_var * rand2(i, 7, seed));
            let m = radius + amp.abs() + 2.0;
            let y = wrap(rand(i, 8, seed) * (floor + 2.0 * m) + vy * t + oy, -m, floor + m);
            let x = wrap(rand(i, 9, seed) * (lw + 2.0 * m) + vx * t + ox, -m, lw + 2.0 * m) + amp * (fr * t * TAU + rand(i, 10, seed) * TAU).sin();
            let p = b.pixel(veymelo_geom::vec2(r.x0 + x as f64, r.y0 + y as f64));
            let mut c = [col.r as f32, col.g as f32, col.b as f32];
            if influence > 0.0 {
                let (bg, a) = background(&b.img, p.x as f32, p.y as f32, bg_spread * s);
                c = [0, 1, 2].map(|j| c[j] * (1.0 + (bg[j] * 1.5 - 1.0) * influence * a));
            }
            Sprite::new(p.x as f32, p.y as f32, radius * s, [c[0], c[1], c[2], opacity * (0.4 + 0.6 * k)], Shape::Soft)
        })
        .collect();
    let mut marks = Image::new(b.img.width, b.img.height);
    draw(&mut marks, &sprites, false);
    let (lighten, with) = (ctx.choice("transferMode") == 1, ctx.flag("compositeWithOriginal"));
    meet(b, marks, lighten, with)
}

effect!(
    BUBBLES_DEF,
    "bubbles",
    "Bubbles",
    "bubbles rising and wobbling, each carrying the picture seen through or reflected in it",
    [
        ranged("bubbleAmount", "Bubble Amount", 100.0, 0.0, 10_000.0),
        ranged("bubbleSpeed", "Bubble Speed", 0.5, -10.0, 10.0).doc("below 0 they sink"),
        ranged("wobbleAmplitude", "Wobble Amplitude", 5.0, 0.0, 500.0),
        ranged("wobbleFrequency", "Wobble Frequency", 1.0, 0.0, 50.0),
        ranged("bubbleSize", "Bubble Size", 1.0, 0.0, 20.0),
        choice("reflectionType", "Reflection Type", &["liquid", "metal"], 0).doc("liquid: the picture seen through it, turned over; metal: reflected wide"),
        choice("shadingType", "Shading Type", &["none", "lighten", "darken", "fadeInwards", "fadeOutwards"], 0),
    ]
);
pub const BUBBLES: Spec = Spec::new(&BUBBLES_DEF, CATEGORY, bubbles);

fn bubbles(ctx: &Ctx, mut b: Buf) -> Buf {
    let n = ctx.num("bubbleAmount", 100.0).clamp(0.0, 10_000.0) as u64;
    let speed = ctx.num("bubbleSpeed", 0.5) as f32;
    let (amp, freq) = (ctx.num("wobbleAmplitude", 5.0) as f32, ctx.num("wobbleFrequency", 1.0) as f32);
    let size = ctx.num("bubbleSize", 1.0).max(0.0) as f32;
    let metal = ctx.choice("reflectionType") == 1;
    let shading = ctx.choice("shadingType");
    let seed = crate::noise::mix(&[ctx.seed, 11]);
    let r = ctx.bounds;
    let (lw, lh) = (r.width() as f32, r.height() as f32);
    let (t, s) = (ctx.time.seconds() as f32, b.scale as f32);
    let base = lw.min(lh) * 0.035 * size;
    // each bubble: where it is, how big, and where in the picture it carries from
    let list: Vec<[f32; 5]> = (0..n)
        .map(|i| {
            let rad = base * (0.5 + rand(i, 1, seed));
            let travel = lh + 2.0 * rad;
            let v = speed * lh * 0.3 * (0.6 + 0.8 * rand(i, 2, seed));
            let y = lh + rad - (rand(i, 3, seed) * travel + v * t).rem_euclid(travel);
            let x0 = rand(i, 4, seed) * lw;
            let x = x0 + amp * (rad / base.max(1e-3)) * (t * freq * TAU + rand(i, 5, seed) * TAU).sin();
            let (p, q) = (b.pixel(veymelo_geom::vec2(r.x0 + x as f64, r.y0 + y as f64)), b.pixel(veymelo_geom::vec2(r.x0 + x0 as f64, r.y0 + y as f64)));
            [p.x as f32, p.y as f32, rad * s, q.x as f32, q.y as f32]
        })
        .collect();
    let src = b.img.clone();
    let mut out = Image::new(src.width, src.height);
    let (w, h) = (out.width, out.height);
    map_px(&mut out, |x, y, _| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut acc: Px = [0.0; 4];
        for q in &list {
            let rr = q[2].max(0.5);
            if (fx - q[0]).abs() > rr + 1.0 || (fy - q[1]).abs() > rr + 1.0 {
                continue;
            }
            let (dx, dy) = ((fx - q[0]) / rr, (fy - q[1]) / rr);
            let d2 = dx * dx + dy * dy;
            let cover = ((1.0 - d2.sqrt()) * rr + 0.5).clamp(0.0, 1.0);
            if cover <= 0.0 {
                continue;
            }
            // liquid turns the picture over and magnifies; metal mirrors it wide
            let k = if metal { 1.6 } else { -0.7 };
            let nz = (1.0 - d2).max(0.0).sqrt();
            let p = src.sample(Sampling::Bilinear, (q[3] + dx * rr * k) as f64, (q[4] + dy * rr * k) as f64);
            let (c, pa) = unpremul(p);
            let mut c = if pa <= 0.0 { [0.6, 0.7, 0.8] } else { c };
            let mut a = cover;
            match shading {
                1 => c = c.map(|v| v * 0.7 + 0.3 * (1.0 - nz)),
                2 => c = c.map(|v| v * (0.5 + 0.5 * nz)),
                3 => a *= nz.sqrt(),
                4 => a *= 1.0 - nz * 0.8,
                _ => {}
            }
            let shine = (1.0 - ((dx + 0.35).powi(2) + (dy + 0.35).powi(2)) / 0.05).max(0.0) * 0.8;
            let c = c.map(|v| v + shine);
            let m = [c[0] * a, c[1] * a, c[2] * a, a];
            let under = 1.0 - m[3];
            acc = [0, 1, 2, 3].map(|j| m[j] + acc[j] * under);
        }
        acc
    });
    let _ = (w, h);
    b.img = out;
    b
}

effect!(
    DRIZZLE_DEF,
    "drizzle",
    "Drizzle",
    "raindrops landing on the layer as on water: rings spreading and fading, bending and lighting the picture",
    [
        ranged("dripRate", "Drip Rate", 10.0, 0.0, 200.0).doc("drops a second"),
        ranged("longevity", "Longevity (sec)", 1.0, 0.0, 30.0),
        angle("rippling", "Rippling", 360.0).doc("how many waves a ring has"),
        ranged("displacement", "Displacement", 5.0, 0.0, 100.0),
        ranged("rippleHeight", "Ripple Height", 30.0, 0.0, 200.0),
        ranged("spreading", "Spreading", 100.0, 0.0, 1000.0),
        pct("lightIntensity", "Light Intensity", 100.0, 0.0, 400.0),
        color("lightColor", "Light Color", 1.0, 1.0, 1.0),
        pct("lightHeight", "Light Height", 25.0, -100.0, 100.0),
        angle("lightDirection", "Light Direction", -45.0),
        pct("ambient", "Ambient", 0.0, -200.0, 200.0),
        pct("diffuse", "Diffuse", 0.0, 0.0, 200.0),
        pct("specular", "Specular", 20.0, 0.0, 200.0),
        ranged("roughness", "Roughness", 0.05, 0.001, 1.0),
    ]
);
pub const DRIZZLE: Spec = Spec::new(&DRIZZLE_DEF, CATEGORY, drizzle);

/// The drops on the water at time `t` (layer units): where, how far their ring has spread,
/// how high it stands.
fn drops(ctx: &Ctx, t: f64) -> Vec<[f32; 4]> {
    let rate = ctx.num("dripRate", 10.0).max(0.0);
    let life = ctx.num("longevity", 1.0).max(0.01);
    if rate <= 0.0 || t < 0.0 {
        return Vec::new();
    }
    let spreading = ctx.num("spreading", 100.0) as f32;
    let height = (ctx.num("rippleHeight", 30.0) / 100.0) as f32;
    let seed = crate::noise::mix(&[ctx.seed, 13]);
    let r = ctx.bounds;
    let (lw, lh) = (r.width() as f32, r.height() as f32);
    // drop k lands at k / rate, somewhere of its own; it is there while younger than its life
    let (k0, k1) = ((((t - life) * rate).floor().max(0.0)) as u64, (t * rate).floor() as u64);
    (k0..=k1)
        .filter_map(|k| {
            let age = (t - k as f64 / rate) as f32;
            if age < 0.0 || age as f64 >= life {
                return None;
            }
            let radius = spreading * age * lh / 400.0;
            // the wave dies as it ages and spreads its energy round a growing ring
            let fade = 1.0 - age / life as f32;
            let amp = height * fade * fade * (0.6 + 0.4 * rand(k, 3, seed)) / (1.0 + radius / (0.1 * lh)).sqrt();
            Some([r.x0 as f32 + rand(k, 1, seed) * lw, r.y0 as f32 + rand(k, 2, seed) * lh, radius, amp])
        })
        .collect()
}

fn drizzle(ctx: &Ctx, mut b: Buf) -> Buf {
    let list = drops(ctx, ctx.time.seconds());
    if list.is_empty() {
        return b;
    }
    let s = b.scale as f32;
    let lh = ctx.bounds.height() as f32;
    let rippling = (ctx.num("rippling", 360.0) / 360.0).clamp(0.05, 8.0) as f32;
    let wave = (lh * 0.04 / rippling.clamp(0.25, 4.0)).max(1.0 / s);
    let width = wave * (1.0 + rippling * 2.0);
    // the drops in the picture's pixels
    let list: Vec<[f32; 4]> = list
        .iter()
        .map(|d| {
            let p = b.pixel(veymelo_geom::vec2(d[0] as f64, d[1] as f64));
            [p.x as f32, p.y as f32, d[2] * s, d[3]]
        })
        .collect();
    let (wave, width) = (wave * s, width * s);
    let height = |x: f32, y: f32| -> f32 {
        list.iter()
            .map(|&[cx, cy, r, a]| {
                let u = ((x - cx).hypot(y - cy) - r) / width;
                if u.abs() >= 1.0 {
                    return 0.0;
                }
                a * (0.5 + 0.5 * (u * PI).cos()) * (u * width / wave * TAU).cos()
            })
            .sum()
    };
    let disp = ctx.num("displacement", 5.0) as f32;
    let li = (ctx.num("lightIntensity", 100.0) / 100.0) as f32;
    let lc = ctx.color("lightColor");
    let lift = (ctx.num("lightHeight", 25.0) / 100.0) as f32;
    let dir = (ctx.num("lightDirection", -45.0) as f32).to_radians();
    let (ambient, diffuse, specular) = ((ctx.num("ambient", 0.0) / 100.0) as f32, (ctx.num("diffuse", 0.0) / 100.0) as f32, (ctx.num("specular", 20.0) / 100.0) as f32);
    let rough = ctx.num("roughness", 0.05).max(0.001) as f32;
    let norm = |v: [f32; 3]| {
        let m = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
        v.map(|c| c / m)
    };
    let light = norm([dir.cos(), dir.sin(), lift.max(0.05)]);
    let half = norm([light[0], light[1], light[2] + 1.0]);
    let src = b.img.clone();
    let (ww, hh) = (src.width as f64, src.height as f64);
    map_px(&mut b.img, |x, y, p| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let (gx, gy) = ((height(fx + 1.0, fy) - height(fx - 1.0, fy)) * 0.5, (height(fx, fy + 1.0) - height(fx, fy - 1.0)) * 0.5);
        if gx == 0.0 && gy == 0.0 {
            return p;
        }
        let q = src.sample(Sampling::Bilinear, ((fx - gx * disp * s * 8.0) as f64).clamp(0.5, ww - 0.5), ((fy - gy * disp * s * 8.0) as f64).clamp(0.5, hh - 0.5));
        let n = norm([-gx * 12.0, -gy * 12.0, 1.0]);
        let lit = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).max(0.0);
        // the shine a tilted ripple catches beyond what the flat water does (flat stays as it was)
        let gloss = |nh: f32| nh.max(0.0).powf(1.0 / rough);
        let shine = (gloss(n[0] * half[0] + n[1] * half[1] + n[2] * half[2]) - gloss(half[2])).max(0.0) * specular * li;
        // light against how the flat water is lit: the still parts as they were
        let shade = 1.0 + (ambient + diffuse * (lit - light[2])) * li;
        let a = q[3];
        [q[0] * shade + shine * lc.r as f32 * a, q[1] * shade + shine * lc.g as f32 * a, q[2] * shade + shine * lc.b as f32 * a, a]
    });
    b
}

effect!(
    STAR_BURST_DEF,
    "starBurst",
    "Star Burst",
    "the layer broken into points flying past toward the viewer, as through a field of stars",
    [
        ranged("scatter", "Scatter", 100.0, 0.0, 1000.0),
        ranged("speed", "Speed", 1.0, -10.0, 10.0),
        angle("phase", "Phase", 0.0),
        ranged("gridSpacing", "Grid Spacing", 2.0, 1.0, 100.0),
        ranged("size", "Size", 100.0, 0.0, 1000.0),
        pct("blendWithOriginal", "Blend w. Original", 0.0, 0.0, 100.0),
    ]
);
pub const STAR_BURST: Spec = Spec::new(&STAR_BURST_DEF, CATEGORY, star_burst);

fn star_burst(ctx: &Ctx, mut b: Buf) -> Buf {
    let scatter = ctx.num("scatter", 100.0) as f32;
    let speed = ctx.num("speed", 1.0) as f32;
    let phase = (ctx.num("phase", 0.0) / 360.0) as f32;
    let spacing = ctx.num("gridSpacing", 2.0).max(1.0) as f32;
    let size = (ctx.num("size", 100.0) / 100.0) as f32;
    let blend = (ctx.num("blendWithOriginal", 0.0) / 100.0) as f32;
    let seed = crate::noise::mix(&[ctx.seed, 17]);
    let r = ctx.bounds;
    let (lw, lh) = (r.width() as f32, r.height() as f32);
    let (t, s) = (ctx.time.seconds() as f32, b.scale as f32);
    let (nx, ny) = ((lw / spacing).ceil().max(1.0) as u64, (lh / spacing).ceil().max(1.0) as u64);
    let (cx, cy) = (lw * 0.5, lh * 0.5);
    let mut stars: Vec<(f32, Sprite)> = (0..(nx * ny).min(4_000_000))
        .filter_map(|i| {
            let (gx, gy) = ((i % nx) as f32 * spacing + spacing * 0.5, (i / nx) as f32 * spacing + spacing * 0.5);
            let at = b.pixel(veymelo_geom::vec2(r.x0 + gx as f64, r.y0 + gy as f64));
            let (c, a) = unpremul(b.img.sample(Sampling::Bilinear, at.x, at.y));
            if a <= 0.0 {
                return None;
            }
            // each point coming nearer round a loop of depth; nearer, it spreads from the middle
            let z = (rand(i, 1, seed) + t * speed * 0.25 + phase).rem_euclid(1.0);
            let near = 1.0 / (1.0 - 0.9 * z);
            let (jx, jy) = (rand2(i, 2, seed) * scatter * 0.005 * lw, rand2(i, 3, seed) * scatter * 0.005 * lh);
            let p = b.pixel(veymelo_geom::vec2(r.x0 + (cx + (gx - cx + jx) * near) as f64, r.y0 + (cy + (gy - cy + jy) * near) as f64));
            let fade = (z * 8.0).min(1.0) * ((1.0 - z) * 6.0).min(1.0);
            Some((z, Sprite::new(p.x as f32, p.y as f32, spacing * 0.5 * size * near * s, [c[0], c[1], c[2], a * fade], Shape::Disc)))
        })
        .collect();
    stars.sort_by(|a, b| a.0.total_cmp(&b.0));
    let sprites: Vec<Sprite> = stars.into_iter().map(|(_, s)| s).collect();
    let mut marks = Image::new(b.img.width, b.img.height);
    draw(&mut marks, &sprites, false);
    if blend > 0.0 {
        for (m, o) in marks.px.iter_mut().zip(&b.img.px) {
            *m = [0, 1, 2, 3].map(|j| m[j] + (o[j] - m[j]) * blend);
        }
    }
    b.img = marks;
    b
}

/// Every effect here.
pub const ALL: [Spec; 5] = [RAINFALL, SNOWFALL, BUBBLES, DRIZZLE, STAR_BURST];

#[cfg(test)]
mod tests;

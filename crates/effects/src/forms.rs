//! Generated forms: waves rippling out from a point, a polygon, a picture's outline or a mask
//! (Radio Waves), lightning bolts (Advanced Lightning), and the Mandelbrot and Julia sets
//! (Fractal).

use std::f64::consts::PI;

use veymelo_raster::filter::{self, Edge};
use veymelo_raster::{Image, Px, Sampling};
use veymelo_time::Tick;

use crate::color::rgb3;
use crate::defs::{angle, choice, color, effect, flag, layer, num, pct, point, px, ranged};
use crate::util::{hsl, luma, map_px, rgb, unpremul};
use crate::{Buf, Ctx, Spec, Take};

const CATEGORY: &str = "generate";

fn over(dst: Px, src: Px) -> Px {
    let k = 1.0 - src[3];
    [src[0] + dst[0] * k, src[1] + dst[1] * k, src[2] + dst[2] * k, src[3] + dst[3] * k]
}

/// Random numbers in turn from a seed, the same every time.
struct Draws(u64, u64);

impl Draws {
    fn new(seed: u64) -> Draws {
        Draws(seed, 0)
    }

    /// From 0 to 1.
    fn next(&mut self) -> f64 {
        self.1 += 1;
        crate::noise::unit(crate::noise::mix(&[self.0, self.1]))
    }

    /// From -1 to 1.
    fn signed(&mut self) -> f64 {
        self.next() * 2.0 - 1.0
    }
}

/// How far each pixel is from the edge of the part that is on: below zero inside.
fn signed_distance(on: &[bool], w: usize, h: usize) -> Vec<f32> {
    let off: Vec<bool> = on.iter().map(|v| !v).collect();
    let to_on = crate::puppet::mesh::distance_sq(w, h, on);
    let to_off = crate::puppet::mesh::distance_sq(w, h, &off);
    on.iter().enumerate().map(|(i, o)| if *o { -(to_off[i].sqrt() as f32) + 0.5 } else { to_on[i].sqrt() as f32 - 0.5 }).collect()
}

const PROFILES: &[&str] = &["square", "taperFront", "taperBack", "taperBoth", "sine", "sawtoothOut", "sawtoothIn"];
const CHANNELS: &[&str] = &["red", "green", "blue", "alpha", "luminance", "hue", "lightness", "saturation"];

effect!(
    RADIO_WAVES_DEF,
    "radioWaves",
    "Radio Waves",
    "waves rippling out from a point: circles, polygons, stars, or the outline of a picture or a mask",
    [
        point("producerPoint", "Producer Point"),
        choice("parametersAreSetAt", "Parameters Are Set At", &["birth", "eachFrame"], 1).doc("birth: each wave keeps what it was born with"),
        ranged("renderQuality", "Render Quality", 4.0, 1.0, 10.0),
        choice("waveType", "Wave Type", &["polygon", "imageContours", "mask"], 0),
        ranged("sides", "Sides", 64.0, 3.0, 64.0).doc("64 is a circle"),
        flag("star", "Star", false),
        ranged("starDepth", "Star Depth", -0.3, -1.0, 1.0),
        layer("sourceLayer", "Source Layer").doc("for image contours"),
        point("sourceCenter", "Source Center"),
        choice("valueChannel", "Value Channel", CHANNELS, 4),
        flag("invertInput", "Invert Input", false),
        ranged("valueThreshold", "Value Threshold", 128.0, 0.0, 255.0),
        px("preBlur", "Pre-Blur", 1.0, 100.0),
        px("tolerance", "Tolerance", 1.0, 100.0),
        ranged("contour", "Contour", 0.0, 0.0, 100.0).doc("0: every part; n: the n-th biggest"),
        num("mask", "Mask", 1.0).range(1.0, 1000.0).doc("which mask (from 1), for wave type mask"),
        ranged("frequency", "Frequency", 1.0, 0.01, 100.0).doc("waves a second"),
        px("expansion", "Expansion", 100.0, 10_000.0).doc("how fast they grow, pixels a second"),
        angle("orientation", "Orientation", 0.0),
        angle("direction", "Direction", 90.0),
        px("velocity", "Velocity", 0.0, 10_000.0).doc("how fast they move toward the direction"),
        ranged("spin", "Spin", 0.0, -3600.0, 3600.0).doc("degrees a second"),
        ranged("lifespan", "Lifespan (sec)", 2.0, 0.01, 100.0),
        flag("reflection", "Reflection", false).doc("bounce off the layer's edges"),
        choice("profile", "Profile", PROFILES, 0),
        color("color", "Color", 1.0, 1.0, 1.0),
        pct("opacity", "Opacity", 100.0, 0.0, 100.0),
        ranged("fadeInTime", "Fade-in Time", 0.0, 0.0, 100.0),
        ranged("fadeOutTime", "Fade-out Time", 0.0, 0.0, 100.0),
        px("startWidth", "Start Width", 5.0, 1000.0),
        px("endWidth", "End Width", 5.0, 1000.0),
    ]
);
pub const RADIO_WAVES: Spec = Spec::new(&RADIO_WAVES_DEF, CATEGORY, radio_waves).points(&[("producerPoint", [0.5, 0.5]), ("sourceCenter", [0.5, 0.5])]);

/// How strong a wave's stroke is across it, from -1 (inside edge) to 1 (outside edge).
fn profile(kind: u32, u: f64) -> f64 {
    let u = u.clamp(-1.0, 1.0);
    match kind {
        1 => (1.0 - u) * 0.5,
        2 => (1.0 + u) * 0.5,
        3 => 1.0 - u.abs(),
        4 => (u * PI * 0.5).cos(),
        5 if u < 0.0 => 1.0 + u,
        6 if u > 0.0 => 1.0 - u,
        _ => 1.0,
    }
}

/// A coordinate kept within lo and hi by bouncing off them.
fn bounce(v: f64, lo: f64, hi: f64) -> f64 {
    let span = hi - lo;
    if span <= 0.0 {
        return lo;
    }
    let t = (v - lo).rem_euclid(2.0 * span);
    lo + if t > span { 2.0 * span - t } else { t }
}

/// The distance from `p` (from the wave's middle) to the outline, along the line from the
/// middle, as seen square on to the side it crosses.
fn outline_distance(pts: &[(f64, f64)], dx: f64, dy: f64) -> f64 {
    let r = dx.hypot(dy);
    if r < 1e-9 {
        return pts.iter().map(|p| p.0.hypot(p.1)).fold(f64::INFINITY, f64::min);
    }
    let dir = (dx / r, dy / r);
    let n = pts.len();
    let mut best = f64::INFINITY;
    for k in 0..n {
        let (p0, p1) = (pts[k], pts[(k + 1) % n]);
        let e = (p1.0 - p0.0, p1.1 - p0.1);
        let den = dir.0 * e.1 - dir.1 * e.0;
        if den.abs() < 1e-12 {
            continue;
        }
        let t = (p0.0 * e.1 - p0.1 * e.0) / den;
        let el2 = (e.0 * e.0 + e.1 * e.1).max(1e-12);
        let s = ((dir.0 * t - p0.0) * e.0 + (dir.1 * t - p0.1) * e.1) / el2;
        if t <= 0.0 || !(-1e-9..=1.0 + 1e-9).contains(&s) {
            continue;
        }
        best = best.min((r - t).abs() * den.abs() / el2.sqrt());
    }
    best
}

/// One wave now.
struct Wave {
    centre: (f64, f64),
    radius: f64,
    half: f64,
    turn: f64,
    fade: f32,
    color: [f32; 3],
    opacity: f32,
    outline: Option<Vec<(f64, f64)>>,
}

fn radio_waves(ctx: &Ctx, mut b: Buf) -> Buf {
    let sc = b.scale;
    let (w, h) = (b.img.width, b.img.height);
    let wave_type = ctx.choice("waveType");
    // the shape waves ripple from, as distances from its edge, and the point of it on the producer point
    let contour: Option<(Vec<f32>, (f64, f64))> = match wave_type {
        1 => ctx.layer_over("sourceLayer", &b, Take::All, false).and_then(|img| {
            let ch = ctx.choice("valueChannel");
            let mut plane = Image::new(w, h);
            for (o, p) in plane.px.iter_mut().zip(&img.px) {
                let (c, a) = unpremul(*p);
                let (hh, s, l) = hsl(c);
                *o = [match ch {
                    0 => c[0] * a,
                    1 => c[1] * a,
                    2 => c[2] * a,
                    3 => a,
                    5 => hh,
                    6 => l * a,
                    7 => s,
                    _ => luma(c) * a,
                }; 4];
            }
            let blur = ctx.num("preBlur", 1.0).max(0.0) * sc;
            if blur > 0.0 {
                plane = filter::gaussian(&plane, blur * 0.5, blur * 0.5, Edge::Repeat);
            }
            let level = ctx.num("valueThreshold", 128.0) as f32 / 255.0;
            let invert = ctx.flag("invertInput");
            let mut on: Vec<bool> = plane.px.iter().map(|p| (p[0] >= level) != invert).collect();
            let pick = ctx.num("contour", 0.0).round() as usize;
            if pick > 0 {
                keep_part(&mut on, w, h, pick);
            }
            if !on.iter().any(|v| *v) {
                return None;
            }
            let mut sdf = signed_distance(&on, w, h);
            let tol = ctx.num("tolerance", 1.0).max(0.0) * sc;
            if tol > 0.0 {
                let mut img = Image::new(w, h);
                for (p, v) in img.px.iter_mut().zip(&sdf) {
                    *p = [*v; 4];
                }
                sdf = filter::gaussian(&img, tol * 0.5, tol * 0.5, Edge::Repeat).px.iter().map(|p| p[0]).collect();
            }
            let a = b.pixel(ctx.point("sourceCenter"));
            Some((sdf, (a.x, a.y)))
        }),
        2 => ctx.mask(ctx.num("mask", 0.0).round() as usize).filter(|m| m.points.len() >= 3).and_then(|m| {
            let pts: Vec<[f64; 2]> = m.points.iter().map(|q| {
                let p = b.pixel(veymelo_geom::vec2(q[0], q[1]));
                [p.x, p.y]
            }).collect();
            let on: Vec<bool> = (0..w * h).map(|i| crate::draw::inside(&pts, [(i % w) as f64 + 0.5, (i / w) as f64 + 0.5]) != m.inverted).collect();
            if !on.iter().any(|v| *v) {
                return None;
            }
            let a = b.pixel(ctx.point("producerPoint"));
            Some((signed_distance(&on, w, h), (a.x, a.y)))
        }),
        _ => None,
    };
    if wave_type != 0 && contour.is_none() {
        return b;
    }
    let t = ctx.time.seconds().max(0.0);
    let freq = ctx.num("frequency", 1.0).max(0.01);
    let life_now = ctx.num("lifespan", 2.0).max(0.01);
    let at_birth = ctx.choice("parametersAreSetAt") == 0;
    // the edge softened over half a pixel, and more the lower the quality asked (1.5 px at 1)
    let soft = 0.5 + 1.5 / ctx.num("renderQuality", 4.0).clamp(1.0, 10.0);
    let kind = ctx.choice("profile");
    let reflection = ctx.flag("reflection");
    let r = ctx.bounds;
    let (lo, hi) = (b.pixel(veymelo_geom::vec2(r.x0, r.y0)), b.pixel(veymelo_geom::vec2(r.x1, r.y1)));
    let k_hi = (t * freq).floor() as i64;
    let k_lo = (((t - life_now) * freq).floor() as i64 - 1).max(0).max(k_hi - 256);
    let waves: Vec<Wave> = (k_lo..=k_hi)
        .filter_map(|k| {
            let born = k as f64 / freq;
            let age = t - born;
            let when = if at_birth { Tick::from_seconds(born) } else { ctx.time };
            let n = |id: &str, d: f64| ctx.value_at(id, when).map_or(d, |v| v.as_f64());
            let life = n("lifespan", 2.0).max(0.01);
            if !(0.0..life).contains(&age) {
                return None;
            }
            let pp = ctx.value_at("producerPoint", when).map_or([0.0; 2], |v| v.as_vec2());
            let c = b.pixel(veymelo_geom::vec2(pp[0], pp[1]));
            let (fin, fout) = (n("fadeInTime", 0.0).max(0.0), n("fadeOutTime", 0.0).max(0.0));
            let fade = (if fin > 0.0 { (age / fin).min(1.0) } else { 1.0 }) * (if fout > 0.0 { ((life - age) / fout).min(1.0) } else { 1.0 });
            let dir = n("direction", 90.0).to_radians();
            let speed = n("velocity", 0.0) * sc;
            let mut centre = (c.x + dir.sin() * speed * age, c.y - dir.cos() * speed * age);
            if reflection {
                centre = (bounce(centre.0, lo.x, hi.x), bounce(centre.1, lo.y, hi.y));
            }
            let (sw, ew) = (n("startWidth", 5.0) * sc, n("endWidth", 5.0) * sc);
            let radius = n("expansion", 100.0) * sc * age;
            let turn = n("orientation", 0.0).to_radians() + n("spin", 0.0).to_radians() * age;
            let outline = if wave_type == 0 {
                let sides = n("sides", 64.0).round().clamp(3.0, 64.0);
                let star = ctx.value_at("star", when).is_some_and(|v| v.as_bool());
                let depth = n("starDepth", -0.3).clamp(-1.0, 1.0);
                if sides >= 64.0 && !star {
                    None
                } else {
                    let s = sides as usize;
                    let unit: Vec<(f64, f64)> = if star {
                        (0..2 * s).map(|i| (PI * i as f64 / s as f64 - PI / 2.0, if i % 2 == 0 { 1.0 } else { (1.0 + depth).max(0.0) })).collect()
                    } else {
                        (0..s).map(|i| (2.0 * PI * i as f64 / s as f64 - PI / 2.0, 1.0)).collect()
                    };
                    Some(unit.iter().map(|&(a, k)| ((a + turn).cos() * radius * k, (a + turn).sin() * radius * k)).collect())
                }
            } else {
                None
            };
            let col = ctx.value_at("color", when).map_or([1.0; 3], |v| rgb3(v.as_color()));
            Some(Wave { centre, radius, half: ((sw + (ew - sw) * age / life) * 0.5).max(0.25), turn, fade: fade as f32, color: col, opacity: n("opacity", 100.0) as f32 / 100.0, outline })
        })
        .collect();
    map_px(&mut b.img, |x, y, p| {
        let mut out = p;
        // older waves first, so newer ones are on top
        for wv in &waves {
            let (dx, dy) = (x as f64 + 0.5 - wv.centre.0, y as f64 + 0.5 - wv.centre.1);
            let d = match (&contour, &wv.outline) {
                (Some((sdf, anchor)), _) => {
                    // the outline set with its anchor on the wave's middle, turned
                    let (s, c) = (-wv.turn).sin_cos();
                    let (rx, ry) = (dx * c - dy * s, dx * s + dy * c);
                    between(sdf, w, h, anchor.0 + rx, anchor.1 + ry) - wv.radius
                }
                (None, Some(o)) => {
                    let d = outline_distance(o, dx, dy);
                    let poly: Vec<[f64; 2]> = o.iter().map(|q| [q.0, q.1]).collect();
                    if crate::draw::inside(&poly, [dx, dy]) { -d } else { d }
                }
                (None, None) => dx.hypot(dy) - wv.radius,
            };
            let cover = ((wv.half - d.abs()) / soft + 0.5).clamp(0.0, 1.0);
            if cover <= 0.0 {
                continue;
            }
            let a = (cover * profile(kind, d / wv.half)) as f32 * wv.fade * wv.opacity;
            out = over(out, [wv.color[0] * a, wv.color[1] * a, wv.color[2] * a, a]);
        }
        out
    });
    b
}

/// A plane's value at a point between its pixels' middles (the nearest edge one past them).
fn between(plane: &[f32], w: usize, h: usize, x: f64, y: f64) -> f64 {
    let (fx, fy) = ((x - 0.5).clamp(0.0, (w - 1) as f64), (y - 0.5).clamp(0.0, (h - 1) as f64));
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
    let at = |x: usize, y: usize| plane[y * w + x] as f64;
    let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
    let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
    top + (bottom - top) * ty
}

/// Only the `pick`-th biggest joined part kept.
fn keep_part(on: &mut [bool], w: usize, h: usize, pick: usize) {
    let mut label = vec![0u32; w * h];
    let mut sizes = vec![0usize];
    for start in 0..w * h {
        if !on[start] || label[start] != 0 {
            continue;
        }
        let id = sizes.len() as u32;
        let (mut stack, mut n) = (vec![start], 0);
        label[start] = id;
        while let Some(i) = stack.pop() {
            n += 1;
            let (x, y) = (i % w, i / w);
            for (ok, j) in [(x > 0, i.wrapping_sub(1)), (x + 1 < w, i + 1), (y > 0, i.wrapping_sub(w)), (y + 1 < h, i + w)] {
                if ok && on[j] && label[j] == 0 {
                    label[j] = id;
                    stack.push(j);
                }
            }
        }
        sizes.push(n);
    }
    let mut order: Vec<usize> = (1..sizes.len()).collect();
    order.sort_by(|a, c| sizes[*c].cmp(&sizes[*a]).then(a.cmp(c)));
    let keep = order.get(pick - 1).map_or(u32::MAX, |k| *k as u32);
    for (v, l) in on.iter_mut().zip(&label) {
        *v = *v && *l == keep;
    }
}

const LIGHTNING_TYPES: &[&str] = &["direction", "strike", "breaking", "bouncy", "omni", "anywhere", "vertical", "twoWayStrike"];

effect!(
    LIGHTNING_DEF,
    "advancedLightning",
    "Advanced Lightning",
    "lightning from an origin, branching and glowing; change the conductivity state to make it flicker",
    [
        choice("lightningType", "Lightning Type", LIGHTNING_TYPES, 1),
        point("origin", "Origin"),
        point("direction", "Direction").doc("where it heads (or strikes)"),
        ranged("conductivityState", "Conductivity State", 10.0, 0.0, 100_000.0).doc("each whole number is another bolt"),
        px("coreRadius", "Core Radius", 3.0, 100.0),
        pct("coreOpacity", "Core Opacity", 75.0, 0.0, 100.0),
        color("coreColor", "Core Color", 1.0, 1.0, 1.0),
        px("glowRadius", "Glow Radius", 50.0, 1000.0),
        pct("glowOpacity", "Glow Opacity", 50.0, 0.0, 100.0),
        color("glowColor", "Glow Color", 0.25, 0.35, 1.0),
        ranged("turbulence", "Turbulence", 1.0, 0.0, 10.0),
        pct("forking", "Forking", 25.0, 0.0, 100.0),
        ranged("decay", "Decay", 0.3, 0.0, 1.0).doc("how much fainter each branch is"),
        flag("decayMainCore", "Decay Main Core", false),
        ranged("alphaObstacle", "Alpha Obstacle", 0.0, -10.0, 10.0).doc("above 0 it avoids the layer's opaque parts, below 0 it is drawn to them"),
        flag("compositeOnOriginal", "Composite On Original", true),
        ranged("complexity", "Complexity", 6.0, 1.0, 12.0),
        px("minForkDistance", "Min Fork Distance", 0.0, 1000.0),
        pct("terminationThreshold", "Termination Threshold", 5.0, 0.0, 100.0),
        flag("mainCoreCollisionOnly", "Main Core Collision Only", false),
        choice("fractalType", "Fractal Type", &["linear", "semiLinear", "spline"], 0),
        pct("coreDrain", "Core Drain", 0.0, 0.0, 100.0),
        pct("forkStrength", "Fork Strength", 50.0, 0.0, 100.0),
        pct("forkVariation", "Fork Variation", 0.0, 0.0, 100.0),
    ]
);
pub const LIGHTNING: Spec = Spec::new(&LIGHTNING_DEF, CATEGORY, lightning).points(&[("origin", [0.25, 0.2]), ("direction", [0.75, 0.8])]);

/// A piece of a bolt: from, to (buffer pixels), how strong.
type Piece = ((f64, f64), (f64, f64), f32);

/// How bolts grow.
struct Growth<'a> {
    turbulence: f64,
    fork: f64,
    decay: f32,
    alpha: Option<&'a Image>,
    obstacle: f64,
    main_only: bool,
    min_fork: f64,
    stop: f32,
    shape: f64,
    fork_strength: f32,
    fork_variation: f32,
}

impl Growth<'_> {
    fn alpha_at(&self, p: (f64, f64)) -> f32 {
        self.alpha.map_or(0.0, |a| a.sample(Sampling::Bilinear, p.0, p.1)[3])
    }
}

/// A bolt from `a` to `z`: its middle pushed aside again and again (`depth` times), forking
/// here and there.
#[allow(clippy::too_many_arguments)]
fn grow(g: &Growth, a: (f64, f64), z: (f64, f64), depth: u32, strength: f32, main: bool, draws: &mut Draws, out: &mut Vec<Piece>, mains: &mut Vec<bool>) {
    if out.len() > 20_000 {
        return;
    }
    let (dx, dy) = (z.0 - a.0, z.1 - a.1);
    let len = dx.hypot(dy);
    if depth == 0 || len < 1.0 {
        out.push((a, z, strength));
        mains.push(main);
        return;
    }
    let (nx, ny) = (-dy / len.max(1e-9), dx / len.max(1e-9));
    // pushed aside by up to 0.22 of its length: its halves turn by up to about 24° (atan 0.44),
    // the sharpest turns photographed bolts make between their steps
    let mut off = draws.signed() * len * 0.22 * g.turbulence * g.shape;
    // the alpha obstacle: of the two sides, the one with less (or more) of the layer
    if g.obstacle != 0.0 && g.alpha.is_some() && (main || !g.main_only) {
        let mid = ((a.0 + z.0) * 0.5, (a.1 + z.1) * 0.5);
        let (a1, a2) = (g.alpha_at((mid.0 + nx * off, mid.1 + ny * off)), g.alpha_at((mid.0 - nx * off, mid.1 - ny * off)));
        let worse = if g.obstacle > 0.0 { a1 > a2 } else { a1 < a2 };
        if worse && draws.next() < g.obstacle.abs() / 10.0 {
            off = -off;
        }
    }
    let m = ((a.0 + z.0) * 0.5 + nx * off, (a.1 + z.1) * 0.5 + ny * off);
    if g.obstacle > 0.0 && !main && g.alpha_at(m) > 0.5 && draws.next() < g.obstacle / 10.0 {
        return;
    }
    grow(g, a, m, depth - 1, strength, main, draws, out, mains);
    grow(g, m, z, depth - 1, strength, main, draws, out, mains);
    // branches only from the longer pieces (from the shortest they would be stubs), heading
    // off to one side about as far as the piece is long
    if depth >= 3 && draws.next() < g.fork && strength > g.stop && len >= g.min_fork {
        let (s, c) = (draws.signed() * 0.7).sin_cos();
        let reach = (0.5 + 0.6 * draws.next()) / len;
        let (fx, fy) = ((dx * c - dy * s) * reach * len, (dx * s + dy * c) * reach * len);
        let vary = if g.fork_variation > 0.0 { 1.0 - g.fork_variation * draws.next() as f32 } else { 1.0 };
        let fs = strength * (1.0 - g.decay) * g.fork_strength * 2.0 * vary;
        grow(g, m, (m.0 + fx, m.1 + fy), depth - 1, fs.min(1.0), false, draws, out, mains);
    }
}

fn lightning(ctx: &Ctx, mut b: Buf) -> Buf {
    let sc = b.scale;
    let o = b.pixel(ctx.point("origin"));
    let d = b.pixel(ctx.point("direction"));
    let (o, d) = ((o.x, o.y), (d.x, d.y));
    let fractal = ctx.choice("fractalType");
    let depth = ctx.num("complexity", 6.0).round().clamp(1.0, 12.0) as u32 + u32::from(fractal == 2);
    let mut draws = Draws::new(crate::noise::mix(&[ctx.num("conductivityState", 10.0).floor() as i64 as u64, ctx.seed]));
    let obstacle = ctx.num("alphaObstacle", 0.0).clamp(-10.0, 10.0);
    let alpha = (obstacle != 0.0).then(|| b.img.clone());
    let fork = (ctx.num("forking", 25.0) / 100.0).clamp(0.0, 1.0);
    let turbulence = ctx.num("turbulence", 1.0).max(0.0);
    let g = Growth {
        turbulence,
        fork,
        decay: ctx.num("decay", 0.3).clamp(0.0, 1.0) as f32,
        alpha: alpha.as_ref(),
        obstacle,
        main_only: ctx.flag("mainCoreCollisionOnly"),
        min_fork: ctx.num("minForkDistance", 0.0).max(0.0) * sc,
        stop: (ctx.num("terminationThreshold", 5.0) / 100.0).clamp(0.0, 1.0) as f32,
        shape: [1.0, 0.7, 0.45][(fractal as usize).min(2)],
        fork_strength: (ctx.num("forkStrength", 50.0) / 100.0).clamp(0.0, 1.0) as f32,
        fork_variation: (ctx.num("forkVariation", 0.0) / 100.0).clamp(0.0, 1.0) as f32,
    };
    let (w, h) = (b.img.width, b.img.height);
    let (mut pieces, mut mains) = (Vec::new(), Vec::new());
    let mut go = |a, z, depth, g: &Growth, draws: &mut Draws| grow(g, a, z, depth, 1.0, true, draws, &mut pieces, &mut mains);
    match ctx.choice("lightningType") {
        0 => go(o, (o.0 + (d.0 - o.0) * 3.0, o.1 + (d.1 - o.1) * 3.0), depth, &g, &mut draws),
        2 => {
            let apart = (d.0 - o.0).hypot(d.1 - o.1);
            let f = (fork * (0.5 + apart / (w.max(h) as f64).max(1.0))).min(1.0);
            go(o, d, depth, &Growth { fork: f, ..g }, &mut draws);
        }
        3 => go(o, d, depth, &Growth { turbulence: turbulence * 1.6, ..g }, &mut draws),
        4 | 5 => {
            let reach = (d.0 - o.0).hypot(d.1 - o.1).max(w.min(h) as f64 * 0.25);
            let anywhere = ctx.choice("lightningType") == 5;
            for _ in 0..4 {
                let a = draws.next() * 2.0 * PI;
                let l = if anywhere { reach * (0.3 + 0.7 * draws.next()) } else { reach };
                go(o, (o.0 + a.cos() * l, o.1 + a.sin() * l), depth, &g, &mut draws);
            }
        }
        6 => go(o, (o.0, h as f64), depth, &g, &mut draws),
        7 => {
            let m = ((o.0 + d.0) * 0.5, (o.1 + d.1) * 0.5);
            go(o, m, depth.saturating_sub(1).max(1), &g, &mut draws);
            go(d, m, depth.saturating_sub(1).max(1), &g, &mut draws);
        }
        _ => go(o, d, depth, &g, &mut draws),
    }
    let dist = |p: (f64, f64)| (p.0 - o.0).hypot(p.1 - o.1);
    let far = pieces.iter().map(|(_, z, _)| dist(*z)).fold(1e-9, f64::max);
    if ctx.flag("decayMainCore") {
        let decay = g.decay;
        for (a, _, s) in &mut pieces {
            *s *= (1.0 - decay * (dist(*a) / far) as f32).max(0.0);
        }
    }
    let drain = (ctx.num("coreDrain", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
    if drain > 0.0 {
        for ((a, _, s), m) in pieces.iter_mut().zip(&mains) {
            if *m {
                *s *= (1.0 - drain * (dist(*a) / far) as f32).max(0.0);
            }
        }
    }
    let core_r = (ctx.num("coreRadius", 3.0) * sc).max(0.3);
    // weaker branches thinner, each line solid in its middle and soft at its sides
    let segs: Vec<crate::draw::Seg> = pieces.iter().map(|(a, z, s)| crate::draw::Seg { a: [a.0, a.1], b: [z.0, z.1], r: core_r * (0.35 + 0.65 * s.min(1.0) as f64), v: *s }).collect();
    let core = crate::draw::segments(w, h, &segs, 0.6);
    let glow_r = ctx.num("glowRadius", 50.0) * sc;
    let mut plane = Image::new(w, h);
    for (p, v) in plane.px.iter_mut().zip(&core) {
        *p = [*v; 4];
    }
    // a wide glow, and a close one hugging the core
    let wide = if glow_r > 0.5 { Some(filter::gaussian(&plane, glow_r / 3.0, glow_r / 3.0, Edge::Clear)) } else { None };
    let close = (glow_r > 0.5).then(|| filter::gaussian(&plane, core_r * 1.5, core_r * 1.5, Edge::Clear));
    let glow: Vec<f32> = (0..w * h).map(|i| (wide.as_ref().map_or(0.0, |g| g.px[i][0] * 3.0) + close.as_ref().map_or(0.0, |g| g.px[i][0] * 0.6)).min(1.0)).collect();
    let (core_op, core_c) = (ctx.num("coreOpacity", 75.0) as f32 / 100.0, rgb3(ctx.color("coreColor")));
    let (glow_op, glow_c) = (ctx.num("glowOpacity", 50.0) as f32 / 100.0, rgb3(ctx.color("glowColor")));
    let composite = ctx.flag("compositeOnOriginal");
    map_px(&mut b.img, |x, y, p| {
        let i = y * w + x;
        let (cv, gv) = (core[i] * core_op, glow[i] * glow_op);
        let l = [0, 1, 2].map(|k| core_c[k] * cv + glow_c[k] * gv);
        let la = l.iter().fold(cv.max(gv), |m, v| m.max(*v)).min(1.0);
        if composite { [p[0] + l[0], p[1] + l[1], p[2] + l[2], (p[3] + la * (1.0 - p[3])).clamp(0.0, 1.0)] } else { [l[0], l[1], l[2], la] }
    });
    b
}

const SETS: &[&str] = &["mandelbrot", "mandelbrotInverse", "mandelbrotOverJulia", "mandelbrotInverseOverJulia", "julia", "juliaInverse"];

effect!(
    FRACTAL_DEF,
    "fractal",
    "Fractal",
    "the Mandelbrot or Julia set, zoomed and coloured",
    [
        choice("setChoice", "Set Choice", SETS, 0),
        choice("equation", "Equation", &["zSquaredPlusC", "zCubedPlusC", "zToTheFourthPlusC"], 0),
        ranged("mandelbrotX", "Mandelbrot X (Real)", -0.74, -2.0, 2.0),
        ranged("mandelbrotY", "Mandelbrot Y (Imaginary)", 0.15, -2.0, 2.0),
        ranged("mandelbrotMagnification", "Mandelbrot Magnification", 0.0, -5.0, 30.0).doc("each 1 zooms in twice as far"),
        ranged("mandelbrotEscapeLimit", "Mandelbrot Escape Limit", 4.0, 1.0, 1000.0),
        ranged("juliaX", "Julia X (Real)", 0.0, -2.0, 2.0),
        ranged("juliaY", "Julia Y (Imaginary)", 0.0, -2.0, 2.0),
        ranged("juliaMagnification", "Julia Magnification", 0.0, -5.0, 30.0),
        ranged("juliaEscapeLimit", "Julia Escape Limit", 4.0, 1.0, 1000.0),
        ranged("postInversionX", "Post-Inversion X Offset", 0.0, -2.0, 2.0),
        ranged("postInversionY", "Post-Inversion Y Offset", 0.0, -2.0, 2.0),
        flag("overlay", "Overlay", false).doc("drawn over the layer, the set's inside see-through"),
        flag("transparency", "Transparency", false),
        choice("palette", "Palette", &["lightnessGradient", "hueWheel", "blackAndWhite", "solidColor"], 0),
        angle("hue", "Hue", 0.0),
        ranged("cycleSteps", "Cycle Steps", 32.0, 1.0, 1000.0),
        angle("cycleOffset", "Cycle Offset", 0.0).doc("turns the colours round (animate it)"),
        flag("edgeHighlight", "Edge Highlight", false),
        choice("oversampleMethod", "Oversample Method", &["edgeDetect", "bruteForce"], 0),
        ranged("oversampleFactor", "Oversample Factor", 2.0, 1.0, 9.0),
    ]
);
pub const FRACTAL: Spec = Spec::new(&FRACTAL_DEF, CATEGORY, fractal);

/// How many steps a point takes to escape (smooth), or None if it never does (it is in the set).
fn escape(z0: (f64, f64), c: (f64, f64), power: u32, limit: f64, most: u32) -> Option<f64> {
    let (mut x, mut y) = z0;
    let lim2 = limit * limit;
    for i in 0..most {
        let (nx, ny) = match power {
            3 => (x * x * x - 3.0 * x * y * y, 3.0 * x * x * y - y * y * y),
            4 => {
                let (a, b) = (x * x - y * y, 2.0 * x * y);
                (a * a - b * b, 2.0 * a * b)
            }
            _ => (x * x - y * y, 2.0 * x * y),
        };
        (x, y) = (nx + c.0, ny + c.1);
        let m2 = x * x + y * y;
        if m2 > lim2 {
            let nu = ((m2.ln() * 0.5) / limit.max(1.0001).ln()).max(1e-9).ln() / (power as f64).ln();
            return Some((i as f64 + 1.0 - nu).max(0.0));
        }
    }
    None
}

fn fractal(ctx: &Ctx, mut b: Buf) -> Buf {
    let which = ctx.choice("setChoice");
    let power = 2 + ctx.choice("equation").min(2);
    let n = |k: &str, d: f64| ctx.num(k, d);
    let (mc, mmag, mlim) = ((n("mandelbrotX", -0.74), n("mandelbrotY", 0.15)), n("mandelbrotMagnification", 0.0), n("mandelbrotEscapeLimit", 4.0).max(1.01));
    let (jc, jmag, jlim) = ((n("juliaX", 0.0), n("juliaY", 0.0)), n("juliaMagnification", 0.0), n("juliaEscapeLimit", 4.0).max(1.01));
    let post = (n("postInversionX", 0.0), n("postInversionY", 0.0));
    let (overlay, transparency) = (ctx.flag("overlay"), ctx.flag("transparency"));
    let palette = ctx.choice("palette");
    let hue = n("hue", 0.0) as f32 / 360.0;
    let steps = n("cycleSteps", 32.0).max(1.0);
    let cycle_off = n("cycleOffset", 0.0) / 360.0;
    let edge = ctx.flag("edgeHighlight");
    let every = ctx.choice("oversampleMethod") == 1;
    let factor = n("oversampleFactor", 2.0).round().clamp(1.0, 9.0) as usize;
    let (julia, inverse, over_julia) = match which {
        1 => (false, true, false),
        2 => (false, false, true),
        3 => (false, true, true),
        4 => (true, false, false),
        5 => (true, true, false),
        _ => (false, false, false),
    };
    let r = ctx.bounds;
    let (lw, lh) = (r.width().max(1.0), r.height().max(1.0));
    let (ox, oy, s) = (b.origin.x, b.origin.y, b.scale);
    let most = |mag: f64| (96.0 + 48.0 * mag.max(0.0)).clamp(64.0, 3000.0) as u32;
    // a point of the layer (its units) in the set's plane, and how fast it escapes
    let value = |lx: f64, ly: f64| -> Option<f64> {
        let (centre, mag, lim) = if julia { (jc, jmag, jlim) } else { (mc, mmag, mlim) };
        let span = 3.5 / 2f64.powf(mag);
        let (mut u, mut v) = ((lx - r.x0 - lw * 0.5) / lw * span + centre.0, (ly - r.y0 - lh * 0.5) / lw * span + centre.1);
        if inverse {
            let d = (u * u + v * v).max(1e-12);
            (u, v) = (u / d + post.0, -v / d + post.1);
        }
        if julia { escape((u, v), mc, power, lim, most(mag)) } else { escape(if over_julia { jc } else { (0.0, 0.0) }, (u, v), power, lim, most(mag)) }
    };
    let shade = |k: Option<f64>| -> Px {
        if palette == 3 {
            let c = rgb(hue, 1.0, 0.5);
            return if k.is_none() != transparency { [c[0], c[1], c[2], 1.0] } else { [0.0; 4] };
        }
        let Some(k) = k else { return if transparency || overlay { [0.0; 4] } else { [0.0, 0.0, 0.0, 1.0] } };
        let u = k / steps + cycle_off;
        let band = u.rem_euclid(1.0) as f32;
        let mut c = match palette {
            1 => rgb((hue + band).rem_euclid(1.0), 1.0, 0.5),
            2 => [if (u.floor() as i64).rem_euclid(2) == 0 { 0.0 } else { 1.0 }; 3],
            _ => rgb((hue + u.floor() as f32 * 0.125).rem_euclid(1.0), 0.85, band),
        };
        if edge && k.fract() < 0.12 {
            c = c.map(|v| v * 0.4 + 0.6);
        }
        [c[0], c[1], c[2], 1.0]
    };
    let (w, h) = (b.img.width, b.img.height);
    let mut plane = Image::new(w, h);
    map_px(&mut plane, |x, y, _| [value(ox + (x as f64 + 0.5) / s, oy + (y as f64 + 0.5) / s).map_or(-1.0, |k| k as f32); 4]);
    let first: Vec<Option<f64>> = plane.px.iter().map(|p| (p[0] >= 0.0).then_some(p[0] as f64)).collect();
    let src = overlay.then(|| b.img.clone());
    map_px(&mut b.img, |x, y, _| {
        let here = first[y * w + x];
        // more samples where neighbours differ (the set's edges), or everywhere
        let more = factor > 1
            && (every || [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)].iter().any(|(dx, dy)| {
                let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    return false;
                }
                match (here, first[ny as usize * w + nx as usize]) {
                    (Some(a), Some(c)) => (a - c).abs() > steps * 0.25,
                    (None, None) => false,
                    _ => true,
                }
            }));
        let c = if more {
            let mut acc = [0f32; 4];
            for sy in 0..factor {
                for sx in 0..factor {
                    let q = shade(value(ox + (x as f64 + (sx as f64 + 0.5) / factor as f64) / s, oy + (y as f64 + (sy as f64 + 0.5) / factor as f64) / s));
                    for k in 0..3 {
                        acc[k] += q[k] * q[3];
                    }
                    acc[3] += q[3];
                }
            }
            acc.map(|v| v / (factor * factor) as f32)
        } else {
            let q = shade(here);
            [q[0] * q[3], q[1] * q[3], q[2] * q[3], q[3]]
        };
        match &src {
            Some(o) => over(o.px[y * w + x], c),
            None => c,
        }
    });
    b
}

/// Every effect here.
pub const ALL: [Spec; 3] = [RADIO_WAVES, LIGHTNING, FRACTAL];

#[cfg(test)]
mod tests;

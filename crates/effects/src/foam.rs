//! Foam: bubbles blown from a producer, growing to their size, drifting with the wind and the
//! turbulence, running down a flow map's slopes, pushing apart and sticking together in clumps,
//! wobbling, and popping (pushing their neighbours away) when their time is up; drawn as
//! outlines (a draft) or as soap bubbles, froth, gum, dishwater, milk or a layer, mirroring a
//! map of their surroundings.
//!
//! Its settings keep the units foam is usually set in: speeds and sizes in hundredths of the
//! layer's width, times in thirtieths of a second. The simulation itself runs in steps of time
//! (so a frame is the same at any frame rate).

use std::f32::consts::TAU;

use veymelo_raster::{Image, Sampling};
use veymelo_time::Tick;

use crate::defs::{angle, choice, effect, flag, layer, point, ranged};
use crate::sprites::{rand, rand2};
use crate::stepper::{RATE, Stepped, key_of, step_at};
use crate::util::{luma, unpremul};
use crate::{Buf, Ctx, Spec, Take};

const CATEGORY: &str = "simulation";

/// The foam settings' frame: their speeds are a frame's movement, their times in frames.
const FRAME: f32 = 30.0;

/// No more bubbles than this at once.
const MOST: usize = 4000;

pub const TEXTURES: &[&str] = &["defaultBubble", "spit", "bubblegum", "dishwater", "milky", "userDefined"];

effect!(
    FOAM_DEF,
    "foam",
    "Foam",
    "bubbles blown from a producer, drifting with the wind and turbulence, down a flow map's slopes, pushing apart and clumping, wobbling and popping; drawn as outlines or as bubbles",
    [
        choice("view", "View", &["draft", "draftFlowMap", "rendered"], 0),
        point("producerPoint", "Producer Point"),
        ranged("producerXSize", "Producer X Size", 0.05, 0.0, 2.0).doc("of the layer's width"),
        ranged("producerYSize", "Producer Y Size", 0.05, 0.0, 2.0).doc("of the layer's height"),
        angle("producerOrientation", "Producer Orientation", 0.0),
        flag("zoomProducerPoint", "Zoom Producer Point", false).doc("zoom about the producer, not the middle"),
        ranged("productionRate", "Production Rate", 1.0, 0.0, 50.0).doc("bubbles a thirtieth of a second"),
        ranged("size", "Size", 0.5, 0.0, 10.0),
        ranged("sizeVariance", "Size Variance", 0.5, 0.0, 1.0),
        ranged("lifespan", "Lifespan", 300.0, 1.0, 30_000.0).doc("thirtieths of a second"),
        ranged("growthSpeed", "Bubble Growth Speed", 0.1, 0.0001, 10.0).doc("of its size each thirtieth of a second"),
        ranged("strength", "Strength", 10.0, 0.0, 100.0).doc("weaker bubbles may pop sooner"),
        ranged("initialSpeed", "Initial Speed", 0.0, -100.0, 100.0),
        angle("initialDirection", "Initial Direction", 0.0),
        ranged("windSpeed", "Wind Speed", 0.5, -100.0, 100.0),
        angle("windDirection", "Wind Direction", 90.0),
        ranged("turbulence", "Turbulence", 0.5, 0.0, 10.0),
        ranged("wobbleAmount", "Wobble Amount", 0.05, 0.0, 10.0),
        ranged("repulsion", "Repulsion", 1.0, 0.0, 10.0),
        ranged("popVelocity", "Pop Velocity", 1.0, -100.0, 100.0),
        ranged("viscosity", "Viscosity", 0.1, 0.0, 1.0),
        ranged("stickiness", "Stickiness", 0.75, 0.0, 1.0),
        ranged("zoom", "Zoom", 1.0, 0.01, 50.0),
        ranged("universeSize", "Universe Size", 1.0, 0.01, 10.0).doc("of the layer: bubbles past it are gone"),
        choice("blendMode", "Blend Mode", &["transparent", "solidOldOnTop", "solidNewOnTop"], 0),
        choice("bubbleTexture", "Bubble Texture", TEXTURES, 0),
        layer("bubbleTextureLayer", "Bubble Texture Layer").doc("for the user-defined texture"),
        choice("bubbleOrientation", "Bubble Orientation", &["fixed", "physicalOrientation", "bubbleVelocity"], 0),
        layer("environmentMap", "Environment Map"),
        ranged("reflectionStrength", "Reflection Strength", 0.0, 0.0, 1.0),
        ranged("reflectionConvergence", "Reflection Convergence", 0.0, 0.0, 1.0),
        layer("flowMap", "Flow Map").doc("its brightness a height: the bubbles run down it"),
        ranged("flowMapSteepness", "Flow Map Steepness", 0.05, -100.0, 100.0),
        choice("flowMapFits", "Flow Map Fits", &["background", "universe"], 0),
        choice("simulationQuality", "Simulation Quality", &["normal", "high", "intense"], 0),
        ranged("randomSeed", "Random Seed", 1.0, 0.0, 10_000.0),
    ]
);
pub const FOAM: Spec = Spec::new(&FOAM_DEF, CATEGORY, foam).points(&[("producerPoint", [0.5, 0.5])]);

#[derive(Clone, Debug)]
pub struct Bubble {
    p: [f32; 2],
    v: [f32; 2],
    /// Seconds since it was blown.
    age: f32,
    /// Its radius grown (layer pixels).
    size: f32,
    /// How far it has rolled (radians).
    spin: f32,
    id: u64,
    /// The age it pops at.
    lasts: f32,
}

impl Bubble {
    fn radius(&self, growth: f32) -> f32 {
        self.size * (self.age * growth * FRAME).min(1.0)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Froth {
    bubbles: Vec<Bubble>,
    /// Bubbles due but not yet blown (a part of one).
    owed: f32,
    next: u64,
}

static KEPT: Stepped<Froth> = Stepped::new();

/// Another layer stretched over a rectangle of this one.
struct Over {
    other: Buf,
    rect: veymelo_geom::Rect,
    /// This layer's rectangle it covers, in its pixels.
    over: [f32; 4],
}

impl Over {
    /// Its colour (straight) and alpha at this layer's point (x, y).
    fn at(&self, x: f32, y: f32) -> ([f32; 3], f32) {
        let [x0, y0, x1, y1] = self.over;
        let (fx, fy) = (((x - x0) / (x1 - x0).max(1e-6)) as f64, ((y - y0) / (y1 - y0).max(1e-6)) as f64);
        self.fraction(fx, fy)
    }

    /// Its colour and alpha at a fraction across and down it (its edge pixels past its edge).
    fn fraction(&self, fx: f64, fy: f64) -> ([f32; 3], f32) {
        let q = self.other.pixel(veymelo_geom::vec2(self.rect.x0 + fx.clamp(0.0, 1.0) * self.rect.width(), self.rect.y0 + fy.clamp(0.0, 1.0) * self.rect.height()));
        let (w, h) = (self.other.img.width as f64, self.other.img.height as f64);
        unpremul(self.other.img.sample(Sampling::Bilinear, q.x.clamp(0.5, (w - 0.5).max(0.5)), q.y.clamp(0.5, (h - 0.5).max(0.5))))
    }

    /// Its brightness at this layer's point, as a height.
    fn height(&self, x: f32, y: f32) -> f32 {
        let (c, a) = self.at(x, y);
        luma(c) * a
    }
}

/// Layer `path` as it is at the layer's start (`start`) or now, over `over`.
fn other(ctx: &Ctx, path: &str, start: bool, over: [f32; 4]) -> Option<Over> {
    let id = ctx.value_in(ctx.effect, path).and_then(|v| v.as_layer())?;
    let host = ctx.host?;
    let (other, rect) = if start { host.layer_at(id, -ctx.time, Take::All)? } else { host.layer(id, Take::All)? };
    Some(Over { other, rect, over })
}

fn foam(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = ctx.bounds;
    let (lw, lh) = (r.width() as f32, r.height() as f32);
    let unit = lw * 0.01;
    let mid = [r.center().x as f32, r.center().y as f32];
    let seed = crate::noise::mix(&[ctx.seed, ctx.num("randomSeed", 1.0) as u64, 7]);
    let universe = ctx.num("universeSize", 1.0).max(0.01) as f32;
    let world = [mid[0] - lw * universe * 0.5, mid[1] - lh * universe * 0.5, mid[0] + lw * universe * 0.5, mid[1] + lh * universe * 0.5];
    // the flow map: its brightness a height the bubbles run down
    let fits = if ctx.choice("flowMapFits") == 1 { world } else { [r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32] };
    let flow = other(ctx, "flowMap", true, fits);
    let substeps = [1, 2, 4][ctx.choice("simulationQuality").min(2) as usize];
    let growth = ctx.num("growthSpeed", 0.1).max(0.0001) as f32;
    let mut key = vec![key_of(ctx, 7), (lw as f64).to_bits(), (lh as f64).to_bits(), r.x0.to_bits(), r.y0.to_bits()];
    if let Some(m) = &flow {
        key.extend(m.other.img.px.iter().step_by(7).map(|p| (luma([p[0], p[1], p[2]]) as f64).to_bits()));
    }
    let froth = KEPT.run(crate::noise::mix(&key), step_at(ctx.time.seconds()), Froth::default, |f, k| {
        let when = k as f64 / RATE;
        let tt = Tick::from_seconds(when);
        let num = |id: &str, d: f64| ctx.value_at(id, tt).map_or(d, |v| v.as_f64()) as f32;
        let dt = (1.0 / RATE) as f32;
        // a speed in the settings' units, in layer pixels a second
        let speed = unit * FRAME;
        // blown
        f.owed += num("productionRate", 1.0).max(0.0) * FRAME * dt;
        let at = ctx.value_at("producerPoint", tt).map_or([mid[0] as f64, mid[1] as f64], |v| v.as_vec2());
        let (half_w, half_h) = (num("producerXSize", 0.05) * lw * 0.5, num("producerYSize", 0.05) * lh * 0.5);
        let (so, co) = num("producerOrientation", 0.0).to_radians().sin_cos();
        let (s0, d0) = (num("initialSpeed", 0.0) * speed, num("initialDirection", 0.0).to_radians());
        let (size, variance) = (num("size", 0.5).max(0.0), num("sizeVariance", 0.5).clamp(0.0, 1.0));
        let lifespan = num("lifespan", 300.0).max(1.0) / FRAME;
        // weak bubbles may pop any time up to their lifespan; strong ones live it out
        let frail = 10.0 / (10.0 + num("strength", 10.0).max(0.0));
        while f.owed >= 1.0 {
            f.owed -= 1.0;
            let id = f.next;
            f.next += 1;
            if f.bubbles.len() >= MOST {
                continue;
            }
            let (u, v) = (rand2(id, 1, seed) * half_w, rand2(id, 2, seed) * half_h);
            f.bubbles.push(Bubble {
                p: [at[0] as f32 + u * co - v * so, at[1] as f32 + u * so + v * co],
                v: [d0.sin() * s0, -d0.cos() * s0],
                age: 0.0,
                size: (size * (1.0 + variance * rand2(id, 3, seed)) * 3.0 * unit).max(0.01),
                spin: rand(id, 4, seed) * TAU,
                id,
                lasts: lifespan * (1.0 - frail * rand(id, 5, seed)),
            });
        }
        let (wind, wd) = (num("windSpeed", 0.5) * speed, num("windDirection", 90.0).to_radians());
        let wind = [wd.sin() * wind, -wd.cos() * wind];
        let turbulence = num("turbulence", 0.5) * speed * 3.0;
        let viscosity = num("viscosity", 0.1).clamp(0.0, 1.0);
        let (repulsion, sticky) = (num("repulsion", 1.0).max(0.0), num("stickiness", 0.75).clamp(0.0, 1.0));
        let steep = num("flowMapSteepness", 0.05);
        let h = dt / substeps as f32;
        for sub in 0..substeps {
            let time = when as f32 + sub as f32 * h;
            let (catch, slow) = (1.0 - (-3.0 * h).exp(), (-viscosity * 6.0 * h).exp());
            for q in f.bubbles.iter_mut() {
                // carried toward the wind, slowed by the thick air, stirred
                let n = |k: u64| crate::noise::smooth((q.p[0] / (lw * 0.15)) as f64, (q.p[1] / (lw * 0.15)) as f64, (time * 0.5) as f64, seed ^ k, None) as f32;
                for (v, w) in q.v.iter_mut().zip(wind) {
                    *v = (*v + (w - *v) * catch) * slow;
                }
                q.v[0] += n(11) * turbulence * h;
                q.v[1] += n(12) * turbulence * h;
                // down the flow map, from light (high) to dark
                if let Some(m) = &flow
                    && steep != 0.0
                {
                    let e = unit.max(1.0);
                    let gx = (m.height(q.p[0] + e, q.p[1]) - m.height(q.p[0] - e, q.p[1])) / (2.0 * e);
                    let gy = (m.height(q.p[0], q.p[1] + e) - m.height(q.p[0], q.p[1] - e)) / (2.0 * e);
                    q.v[0] -= gx * steep * lw * 0.5 * FRAME * FRAME * h;
                    q.v[1] -= gy * steep * lw * 0.5 * FRAME * FRAME * h;
                }
            }
            meet(&mut f.bubbles, growth, repulsion, sticky, h);
            for q in f.bubbles.iter_mut() {
                q.p[0] += q.v[0] * h;
                q.p[1] += q.v[1] * h;
                // rolled along by its sideways movement
                q.spin += q.v[0] / q.radius(growth).max(0.5) * h;
            }
        }
        for q in f.bubbles.iter_mut() {
            q.age += dt;
        }
        // popped: pushing their neighbours away
        let pop = num("popVelocity", 1.0) * speed;
        let popped: Vec<([f32; 2], f32)> = f.bubbles.iter().filter(|q| q.age >= q.lasts).map(|q| (q.p, q.radius(growth))).collect();
        if pop != 0.0 {
            for (p, rad) in &popped {
                let reach = (rad * 3.0).max(unit);
                for q in f.bubbles.iter_mut() {
                    let (dx, dy) = (q.p[0] - p[0], q.p[1] - p[1]);
                    let d = (dx * dx + dy * dy).sqrt();
                    if d > 1e-4 && d < reach {
                        let k = pop * (1.0 - d / reach) / d;
                        q.v[0] += dx * k;
                        q.v[1] += dy * k;
                    }
                }
            }
        }
        f.bubbles.retain(|q| q.age < q.lasts && q.p[0] > world[0] && q.p[0] < world[2] && q.p[1] > world[1] && q.p[1] < world[3]);
    });
    draw(ctx, &mut b, &froth.bubbles, growth, flow.as_ref());
    b
}

/// Bubbles that overlap pushed apart (`repulsion`), those touching sharing their movement
/// (`sticky`), so foam holds together in clumps.
fn meet(bubbles: &mut [Bubble], growth: f32, repulsion: f32, sticky: f32, h: f32) {
    if bubbles.len() < 2 || (repulsion <= 0.0 && sticky <= 0.0) {
        return;
    }
    let radii: Vec<f32> = bubbles.iter().map(|q| q.radius(growth)).collect();
    let cell = radii.iter().fold(0f32, |a, &r| a.max(r)).max(1.0) * 2.2;
    let mut grid: std::collections::HashMap<(i32, i32), Vec<usize>> = std::collections::HashMap::new();
    for (i, q) in bubbles.iter().enumerate() {
        grid.entry(((q.p[0] / cell).floor() as i32, (q.p[1] / cell).floor() as i32)).or_default().push(i);
    }
    let (mut moved, mut dv) = (vec![[0f32; 2]; bubbles.len()], vec![[0f32; 2]; bubbles.len()]);
    let push = 1.0 - (-repulsion * FRAME * h).exp();
    let share = 0.5 * sticky * (1.0 - (-10.0 * h).exp());
    for (i, q) in bubbles.iter().enumerate() {
        let (gx, gy) = ((q.p[0] / cell).floor() as i32, (q.p[1] / cell).floor() as i32);
        for oy in -1..=1 {
            for ox in -1..=1 {
                let Some(list) = grid.get(&(gx + ox, gy + oy)) else { continue };
                for &j in list.iter().filter(|&&j| j > i) {
                    let o = &bubbles[j];
                    let (mut dx, mut dy) = (o.p[0] - q.p[0], o.p[1] - q.p[1]);
                    if dx * dx + dy * dy < 1e-8 {
                        // on top of each other: parted some way of their own
                        let a = rand(q.id ^ o.id.rotate_left(17), 9, 0) * TAU;
                        (dx, dy) = (a.cos() * 1e-3, a.sin() * 1e-3);
                    }
                    let d = (dx * dx + dy * dy).sqrt();
                    let reach = radii[i] + radii[j];
                    // touching (or all but): they move together
                    if d < reach * 1.1 {
                        let s = [(o.v[0] - q.v[0]) * share, (o.v[1] - q.v[1]) * share];
                        dv[i] = [dv[i][0] + s[0], dv[i][1] + s[1]];
                        dv[j] = [dv[j][0] - s[0], dv[j][1] - s[1]];
                    }
                    let overlap = reach - d;
                    if overlap > 0.0 {
                        let k = overlap * 0.5 * push / d;
                        moved[i][0] -= dx * k;
                        moved[i][1] -= dy * k;
                        moved[j][0] += dx * k;
                        moved[j][1] += dy * k;
                    }
                }
            }
        }
    }
    for (q, (m, v)) in bubbles.iter_mut().zip(moved.iter().zip(&dv)) {
        q.p[0] += m[0];
        q.p[1] += m[1];
        q.v[0] += v[0];
        q.v[1] += v[1];
    }
}

/// How a texture looks at (u, v) on a bubble (-1 to 1 across, turned with it), `d` from its
/// middle: straight colour and alpha. `film` is the bubble's own film thickness.
fn texture(kind: u32, u: f32, v: f32, d: f32, film: f32) -> ([f32; 3], f32) {
    let nz = (1.0 - d * d).max(0.0).sqrt();
    let rim = (1.0 - nz).powi(3);
    // lit from the upper left
    let lit = (-0.45 * u - 0.5 * v + 0.74 * nz).clamp(0.0, 1.0);
    let glint = (-((u + 0.38).powi(2) + (v + 0.38).powi(2)) / 0.012).exp() + 0.5 * (-((u - 0.35).powi(2) + (v - 0.42).powi(2)) / 0.004).exp();
    // a white glint laid over the colour
    let shine = |c: [f32; 3], a: f32, k: f32| {
        let g = (glint * k).min(1.0);
        let all = g + a * (1.0 - g);
        (c.map(|x| (g + x * a * (1.0 - g)) / all.max(1e-6)), all)
    };
    match kind {
        // froth: whitish, cloudier
        1 => shine([0.95, 0.95, 0.92], 0.22 + 0.55 * rim, 0.5),
        // gum: pink, solid, lit
        2 => shine([1.0 * (0.55 + 0.45 * lit), 0.55 * (0.55 + 0.45 * lit), 0.75 * (0.55 + 0.45 * lit)], 0.92, 0.9),
        // dishwater: murky, thin
        3 => shine([0.74, 0.8, 0.66], 0.12 + 0.6 * rim, 0.45),
        // milk: a white ball
        4 => shine([0.35 + 0.65 * lit; 3], 1.0, 0.7),
        // a soap film: clear in the middle, coloured at its rim by the light meeting itself
        // across the film (thinner at the top, where it drains)
        _ => {
            let t = film + 0.35 * v + 0.25 * nz;
            let c = [0.0, 0.33, 0.67].map(|o| 0.62 + 0.38 * (TAU * (t * 1.6 + o)).cos());
            shine(c, 0.04 + 0.75 * rim, 0.95)
        }
    }
}

/// The bubbles drawn: outlines (a draft, over the flow map if asked) or textured, mirroring
/// the environment map.
fn draw(ctx: &Ctx, b: &mut Buf, bubbles: &[Bubble], growth: f32, flow: Option<&Over>) {
    let r = ctx.bounds;
    let s = b.scale as f32;
    let (w, h) = (b.img.width, b.img.height);
    let mut out = Image::new(w, h);
    let view = ctx.choice("view");
    if view == 1
        && let Some(m) = flow
    {
        // the flow map at half strength under the outlines
        for y in 0..h {
            for x in 0..w {
                let (px, py) = (b.origin.x as f32 + (x as f32 + 0.5) / s, b.origin.y as f32 + (y as f32 + 0.5) / s);
                let v = m.height(px, py) * 0.5;
                out.px[y * w + x] = [v, v, v, 0.5];
            }
        }
    }
    let zoom = ctx.num("zoom", 1.0).max(0.01) as f32;
    let at = ctx.point("producerPoint");
    let about = if ctx.flag("zoomProducerPoint") { [at.x as f32, at.y as f32] } else { [r.center().x as f32, r.center().y as f32] };
    let seed = crate::noise::mix(&[ctx.seed, ctx.num("randomSeed", 1.0) as u64, 7]);
    let wobble = ctx.num("wobbleAmount", 0.05).max(0.0) as f32;
    let orient = ctx.choice("bubbleOrientation");
    let blend = ctx.choice("blendMode");
    let kind = ctx.choice("bubbleTexture");
    let picture = (view == 2 && kind == 5).then(|| other(ctx, "bubbleTextureLayer", false, [0.0; 4])).flatten();
    if view == 2 && kind == 5 && picture.is_none() {
        ctx.note("the user-defined texture needs a Bubble Texture Layer: drawn as plain bubbles".into());
    }
    let reflect = ctx.num("reflectionStrength", 0.0).clamp(0.0, 1.0) as f32;
    let env = (view == 2 && reflect > 0.0).then(|| ctx.layer_over("environmentMap", b, Take::All, true)).flatten();
    let converge = ctx.num("reflectionConvergence", 0.0).clamp(0.0, 1.0) as f32;
    let mut order: Vec<&Bubble> = bubbles.iter().collect();
    match blend {
        // the oldest on top: drawn last
        1 => order.sort_by(|a, c| a.id.cmp(&c.id).reverse()),
        _ => order.sort_by_key(|q| q.id),
    }
    for q in order {
        let rad = q.radius(growth) * zoom * s;
        if rad < 0.05 {
            continue;
        }
        let p = b.pixel(veymelo_geom::vec2((about[0] + (q.p[0] - about[0]) * zoom) as f64, (about[1] + (q.p[1] - about[1]) * zoom) as f64));
        let (x, y) = (p.x as f32, p.y as f32);
        // fading out over its last tenth of a second
        let fade = ((q.lasts - q.age) * 10.0).clamp(0.0, 1.0);
        let turn = match orient {
            1 => q.spin,
            2 => q.v[1].atan2(q.v[0]),
            _ => 0.0,
        };
        // wobbling: squashed one way and stretched the other, back and forth
        let wob = wobble * 0.15 * (q.age * 10.5 + rand(q.id, 6, seed) * TAU).sin();
        let (rx, ry) = (rad * (1.0 + wob), rad * (1.0 - wob));
        let film = rand(q.id, 7, seed);
        let (sn, cs) = turn.sin_cos();
        let reach = rx.max(ry) + 1.0;
        let (x0, x1) = ((x - reach).floor().max(0.0) as usize, ((x + reach).ceil().max(0.0) as usize).min(w));
        let (y0, y1) = ((y - reach).floor().max(0.0) as usize, ((y + reach).ceil().max(0.0) as usize).min(h));
        for py in y0..y1 {
            for px in x0..x1 {
                let (dx, dy) = (px as f32 + 0.5 - x, py as f32 + 0.5 - y);
                // into the bubble's own turned frame
                let (u, v) = ((dx * cs + dy * sn) / rx, (-dx * sn + dy * cs) / ry);
                let d = (u * u + v * v).sqrt();
                let cover = ((1.0 - d) * rx.min(ry) + 0.5).clamp(0.0, 1.0) * fade;
                if cover <= 0.0 {
                    continue;
                }
                let (u, v, d) = (u.clamp(-1.0, 1.0), v.clamp(-1.0, 1.0), d.min(1.0));
                let (mut c, mut a) = if view != 2 {
                    // an outline
                    let ring = (1.0 - ((1.0 - d) * rx.min(ry) - 0.6).abs()).clamp(0.0, 1.0);
                    ([0.55, 0.75, 1.0], ring)
                } else if let Some(t) = &picture {
                    t.fraction(u as f64 * 0.5 + 0.5, v as f64 * 0.5 + 0.5)
                } else {
                    texture(kind, u, v, d, film)
                };
                if let Some(e) = &env {
                    // the surroundings mirrored off the ball, more at its rim
                    let nz = (1.0 - d * d).max(0.0).sqrt();
                    let far = (1.0 - converge) * (w.max(h) as f32) * 0.5;
                    let (ex, ey) = (x + 2.0 * nz * u * far, y + 2.0 * nz * v * far);
                    let (ec, ea) = unpremul(e.sample(Sampling::Bilinear, ex as f64, ey as f64));
                    let k = reflect * (0.25 + 0.75 * (1.0 - nz).powi(2)) * ea;
                    c = [0, 1, 2].map(|i| (c[i] * a + ec[i] * k) / (a + k * (1.0 - a)).max(1e-6));
                    a += k * (1.0 - a);
                }
                if view == 2 && blend != 0 {
                    // solid: the bubble over black, hiding what is under it
                    c = c.map(|v| v * a);
                    a = 1.0;
                }
                let k = a * cover;
                let o = &mut out.px[py * w + px];
                *o = [c[0] * k + o[0] * (1.0 - k), c[1] * k + o[1] * (1.0 - k), c[2] * k + o[2] * (1.0 - k), k + o[3] * (1.0 - k)];
            }
        }
    }
    if view != 2 {
        // the producer: a small cross
        let p = b.pixel(at);
        let arm = 6.0f32;
        for i in -6i32..=6 {
            for (px, py) in [(p.x as f32 + i as f32 * arm / 6.0, p.y as f32), (p.x as f32, p.y as f32 + i as f32 * arm / 6.0)] {
                let (px, py) = (px.floor() as i64, py.floor() as i64);
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                    out.px[py as usize * w + px as usize] = [1.0, 0.25, 0.2, 1.0];
                }
            }
        }
    }
    b.img = out;
}

#[cfg(test)]
mod tests;

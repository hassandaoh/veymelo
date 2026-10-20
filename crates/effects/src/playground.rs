//! Particle Playground: particles fired from a cannon, sent out from a grid each frame, or burst
//! from another layer; pulled by gravity, pushing each other apart, kept in by a mask's walls;
//! their colour, size, speed, spin and more set from map layers (for good, or for a step at a
//! time); bursting into smaller ones; drawn as dots, as letters, or as a copy of another layer.
//! Each force and mapper acts on the particles its Affects settings pick: by where they came
//! from, a selection map, their letters and their age.
//!
//! Simulated from the layer's start in steps of a sixtieth of a second; the map layers are read
//! at the frame being drawn.

use std::f32::consts::TAU;

use veymelo_geom::{Rect, vec2};
use veymelo_keyframe::{Kind, Value};
use veymelo_path::{Path, Rule};
use veymelo_project::PropDef;
use veymelo_raster::{Image, Px, Sampling};
use veymelo_time::Tick;

use crate::defs::{angle, choice, color, flag, layer, point, ranged};
use crate::sprites::{Shape, Sprite, draw, rand, rand2};
use crate::stepper::{RATE, Stepped, key_of, step_at};
use crate::util::{luma, unpremul};
use crate::{Buf, Ctx, Spec, Take};

/// Where a particle came from.
const CANNON: u8 = 0;
const GRID: u8 = 1;
const EXPLODED_LAYER: u8 = 2;
const EXPLODED_PARTICLE: u8 = 3;

/// A particle (layer units, degrees, seconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub p: [f32; 2],
    pub v: [f32; 2],
    /// Not premultiplied, and its opacity.
    pub color: [f32; 4],
    pub radius: f32,
    pub mass: f32,
    pub friction: f32,
    pub static_friction: f32,
    pub force: [f32; 2],
    pub angle: f32,
    pub spin: f32,
    pub torque: f32,
    pub scale: [f32; 2],
    pub scale_speed: f32,
    pub age: f32,
    pub lifespan: f32,
    /// The letter it shows (0: a dot), and how big.
    pub letter: u32,
    pub font_size: f32,
    /// How far the layer map's time is moved for it (seconds).
    pub time_offset: f32,
    pub from: u8,
    pub id: u32,
}

impl Particle {
    fn new(p: [f32; 2], v: [f32; 2], color: [f32; 4], radius: f32, from: u8, id: u32) -> Particle {
        Particle {
            p,
            v,
            color,
            radius,
            mass: 1.0,
            friction: 0.0,
            static_friction: 0.0,
            force: [0.0; 2],
            angle: 0.0,
            spin: 0.0,
            torque: 0.0,
            scale: [1.0; 2],
            scale_speed: 0.0,
            age: 0.0,
            lifespan: f32::INFINITY,
            letter: 0,
            font_size: 0.0,
            time_offset: 0.0,
            from,
            id,
        }
    }
}

/// The particles alive, the births owed, and the frames the grid has fired.
#[derive(Clone, Debug, Default)]
pub struct State {
    pub parts: Vec<Particle>,
    due: f64,
    next: u32,
    grid_frames: i64,
    exploded: bool,
}

/// What a property mapper's channel can set, in order.
pub const PROPERTIES: &[&str] = &[
    "none",
    "red",
    "green",
    "blue",
    "kineticFriction",
    "staticFriction",
    "angle",
    "angularVelocity",
    "torque",
    "scale",
    "xScale",
    "yScale",
    "x",
    "y",
    "gradientVelocity",
    "xSpeed",
    "ySpeed",
    "gradientForce",
    "xForce",
    "yForce",
    "opacity",
    "mass",
    "lifespan",
    "character",
    "fontSize",
    "timeOffset",
    "scaleSpeed",
];

const OPERATORS: &[&str] = &["set", "add", "difference", "subtract", "multiply", "min", "max"];
const FROM: &[&str] = &["all", "cannon", "grid", "layerExploder", "particleExploder", "gridAndLayerExploder", "cannonAndGrid", "cannonAndParticleExploder"];

fn get(q: &Particle, what: u32) -> f32 {
    match what {
        1 => q.color[0],
        2 => q.color[1],
        3 => q.color[2],
        4 => q.friction,
        5 => q.static_friction,
        6 => q.angle,
        7 => q.spin,
        8 => q.torque,
        9 => (q.scale[0] + q.scale[1]) * 0.5,
        10 => q.scale[0],
        11 => q.scale[1],
        12 => q.p[0],
        13 => q.p[1],
        15 => q.v[0],
        16 => q.v[1],
        18 => q.force[0],
        19 => q.force[1],
        20 => q.color[3],
        21 => q.mass,
        22 => q.lifespan,
        23 => q.letter as f32,
        24 => q.font_size,
        25 => q.time_offset,
        26 => q.scale_speed,
        _ => 0.0,
    }
}

fn set(q: &mut Particle, what: u32, v: f32) {
    match what {
        1 => q.color[0] = v,
        2 => q.color[1] = v,
        3 => q.color[2] = v,
        4 => q.friction = v.clamp(0.0, 10.0),
        5 => q.static_friction = v.max(0.0),
        6 => q.angle = v,
        7 => q.spin = v,
        8 => q.torque = v,
        9 => q.scale = [v.max(0.0); 2],
        10 => q.scale[0] = v.max(0.0),
        11 => q.scale[1] = v.max(0.0),
        12 => q.p[0] = v,
        13 => q.p[1] = v,
        15 => q.v[0] = v,
        16 => q.v[1] = v,
        18 => q.force[0] = v,
        19 => q.force[1] = v,
        20 => q.color[3] = v.clamp(0.0, 1.0),
        21 => q.mass = v.max(0.01),
        22 => q.lifespan = v.max(0.0),
        23 => q.letter = v.round().clamp(0.0, 0x10_ffff as f32) as u32,
        24 => q.font_size = v.max(0.0),
        25 => q.time_offset = v,
        26 => q.scale_speed = v,
        _ => {}
    }
}

/// A map layer over this layer (stretched to it), read in layer units.
struct Map {
    img: Image,
    origin: [f32; 2],
    scale: [f32; 2],
}

impl Map {
    /// Not premultiplied, and its alpha.
    fn at(&self, x: f32, y: f32) -> Px {
        let (c, a) = unpremul(self.img.sample(Sampling::Bilinear, ((x - self.origin[0]) * self.scale[0]) as f64, ((y - self.origin[1]) * self.scale[1]) as f64));
        [c[0], c[1], c[2], a]
    }
}

/// The map layer a layer setting names, stretched over this layer.
fn map_of(ctx: &Ctx, id: &str) -> Option<Map> {
    let lid = ctx.value_in(ctx.effect, id).and_then(|v| v.as_layer())?;
    let (other, rect) = ctx.host?.layer(lid, Take::All)?;
    let r = ctx.bounds;
    // a layer point p is the other layer's point rect.x0 + (p - r.x0) · rect.w / r.w
    let (kx, ky) = ((rect.width() / r.width().max(1e-9)) as f32, (rect.height() / r.height().max(1e-9)) as f32);
    let at0 = other.pixel(vec2(rect.x0, rect.y0));
    Some(Map {
        img: other.img,
        origin: [r.x0 as f32 - at0.x as f32 / (kx * other.scale as f32), r.y0 as f32 - at0.y as f32 / (ky * other.scale as f32)],
        scale: [kx * other.scale as f32, ky * other.scale as f32],
    })
}

/// Which particles something acts on, as a weight from 0 to 1.
#[derive(Default)]
struct Affects {
    from: u32,
    selection: Option<Map>,
    letters: Vec<char>,
    /// Above 0: older than this; below 0: younger than its size (seconds).
    age: f32,
    feather: f32,
}

impl Affects {
    fn of(ctx: &Ctx, group: &str) -> Affects {
        let g = |s: &str| format!("{group}Affects{s}");
        Affects {
            from: ctx.choice(&g("ParticlesFrom")),
            selection: map_of(ctx, &g("SelectionMap")),
            letters: ctx.value_in(ctx.effect, &g("Characters")).map(|v| v.as_text().chars().collect()).unwrap_or_default(),
            age: ctx.num(&g("OlderYoungerThan"), 0.0) as f32,
            feather: ctx.num(&g("AgeFeather"), 0.0).max(0.0) as f32,
        }
    }

    fn weight(&self, q: &Particle) -> f32 {
        let from = match self.from {
            1 => q.from == CANNON,
            2 => q.from == GRID,
            3 => q.from == EXPLODED_LAYER,
            4 => q.from == EXPLODED_PARTICLE,
            5 => q.from == GRID || q.from == EXPLODED_LAYER,
            6 => q.from == CANNON || q.from == GRID,
            7 => q.from == CANNON || q.from == EXPLODED_PARTICLE,
            _ => true,
        };
        if !from || (!self.letters.is_empty() && !char::from_u32(q.letter).is_some_and(|c| self.letters.contains(&c))) {
            return 0.0;
        }
        let mut w = 1.0;
        if self.age != 0.0 {
            // how far past the age line, softened over the feather
            let d = if self.age > 0.0 { q.age - self.age } else { -self.age - q.age };
            w *= if self.feather > 0.0 { (d / self.feather + 0.5).clamp(0.0, 1.0) } else if d >= 0.0 { 1.0 } else { 0.0 };
        }
        if let Some(m) = &self.selection {
            let c = m.at(q.p[0], q.p[1]);
            w *= luma([c[0], c[1], c[2]]).clamp(0.0, 1.0) * c[3];
        }
        w
    }
}

/// A property mapper: each channel of its map sets a property (from min to max).
struct Mapper {
    map: Map,
    /// Red, green, blue: what, how (the ephemeral mapper's operator), min, max.
    channels: [(u32, u32, f32, f32); 3],
    affects: Affects,
}

impl Mapper {
    fn of(ctx: &Ctx, group: &str, operators: bool) -> Option<Mapper> {
        let map = map_of(ctx, &format!("{group}UseLayerAsMap"))?;
        let channels = ["Red", "Green", "Blue"].map(|c| {
            (
                ctx.choice(&format!("{group}Map{c}To")),
                if operators { ctx.choice(&format!("{group}{c}Operator")) } else { 0 },
                ctx.num(&format!("{group}{c}Min"), 0.0) as f32,
                ctx.num(&format!("{group}{c}Max"), 1.0) as f32,
            )
        });
        Some(Mapper { map, channels, affects: Affects::of(ctx, group) })
    }

    fn apply(&self, q: &mut Particle) {
        let w = self.affects.weight(q);
        if w <= 0.0 {
            return;
        }
        let px = self.map.at(q.p[0], q.p[1]);
        for (ch, &(what, op, lo, hi)) in self.channels.iter().enumerate() {
            if what == 0 {
                continue;
            }
            if what == 14 || what == 17 {
                // along the slope of the channel
                let e = 1.0;
                let gx = (self.map.at(q.p[0] + e, q.p[1])[ch] - self.map.at(q.p[0] - e, q.p[1])[ch]) * 0.5 * (hi - lo) * w;
                let gy = (self.map.at(q.p[0], q.p[1] + e)[ch] - self.map.at(q.p[0], q.p[1] - e)[ch]) * 0.5 * (hi - lo) * w;
                if what == 14 {
                    q.v = [q.v[0] + gx, q.v[1] + gy];
                } else {
                    q.force = [q.force[0] + gx, q.force[1] + gy];
                }
                continue;
            }
            let v = lo + (hi - lo) * px[ch];
            let was = get(q, what);
            let new = match op {
                1 => was + v,
                2 => (was - v).abs(),
                3 => was - v,
                4 => was * v,
                5 => was.min(v),
                6 => was.max(v),
                _ => v,
            };
            set(q, what, if w >= 1.0 || !was.is_finite() { new } else { was + (new - was) * w });
        }
    }
}

/// Is (x, y) inside the polygon?
fn inside(poly: &[[f32; 2]], x: f32, y: f32) -> bool {
    let mut odd = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + n - 1) % n]);
        if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
            odd = !odd;
        }
    }
    odd
}

/// The edge of `poly` the segment from `p` to `q` crosses first, as its unit normal.
fn crossed(poly: &[[f32; 2]], p: [f32; 2], q: [f32; 2]) -> Option<[f32; 2]> {
    let n = poly.len();
    let (mut best, mut normal) = (f32::MAX, None);
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        let (rx, ry, sx, sy) = (q[0] - p[0], q[1] - p[1], b[0] - a[0], b[1] - a[1]);
        let den = rx * sy - ry * sx;
        if den.abs() < 1e-12 {
            continue;
        }
        let (ax, ay) = (a[0] - p[0], a[1] - p[1]);
        let (t, u) = ((ax * sy - ay * sx) / den, (ax * ry - ay * rx) / den);
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) && t < best {
            let l = sx.hypot(sy).max(1e-9);
            (best, normal) = (t, Some([-sy / l, sx / l]));
        }
    }
    normal
}

/// The settings that may change as the particles fly, read at each step's time.
struct Knobs {
    cannon: Option<Cannon>,
    grid: Option<Grid>,
    /// The particle exploder's new size and scatter.
    burst: (f32, f32),
    /// Gravity's pull and how much it varies.
    gravity: ([f32; 2], f32),
    /// Repel's strength and reach.
    repel: (f32, f32),
}

/// What is read once for the frame drawn: the maps and burst layer, the walls, and which
/// particles each force acts on.
struct World {
    exploded: Vec<Particle>,
    bursts: Affects,
    falls: Affects,
    repels: Affects,
    wall: Option<(Vec<[f32; 2]>, Affects)>,
    lasting: Option<Mapper>,
    passing: Option<Mapper>,
    seed: u64,
    /// Where particles may go (past it they are dropped).
    room: Rect,
    frame: f64,
}

struct Cannon {
    at: [f32; 2],
    radius: f32,
    rate: f64,
    direction: f32,
    spread: f32,
    speed: f32,
    speed_spread: f32,
    color: [f32; 4],
    size: f32,
    text: Vec<char>,
    font_size: f32,
}

struct Grid {
    at: [f32; 2],
    w: f32,
    h: f32,
    across: u32,
    down: u32,
    color: [f32; 4],
    size: f32,
    text: Vec<char>,
    font_size: f32,
}

fn rgba(c: veymelo_color::Rgba) -> [f32; 4] {
    [c.r as f32, c.g as f32, c.b as f32, c.a as f32]
}

/// A text setting's letters (no control characters).
fn letters(ctx: &Ctx, id: &str) -> Vec<char> {
    ctx.value_in(ctx.effect, id).map(|v| v.as_text().chars().filter(|c| !c.is_control()).collect()).unwrap_or_default()
}

impl Knobs {
    fn read(ctx: &Ctx, t: f64) -> Knobs {
        let tt = Tick::from_seconds(t);
        let num = |id: &str, d: f64| ctx.value_at(id, tt).map_or(d, |v| v.as_f64()) as f32;
        let pt = |id: &str| ctx.value_at(id, tt).map_or([0.0; 2], |v| v.as_vec2()).map(|v| v as f32);
        let font_size = num("fontSize", 18.0).max(1.0);
        let rate = num("cannonParticlesPerSecond", 60.0).max(0.0) as f64;
        let cannon = (rate > 0.0).then(|| Cannon {
            at: pt("cannonPosition"),
            radius: num("cannonBarrelRadius", 0.0),
            rate,
            direction: num("cannonDirection", 0.0),
            spread: num("cannonDirectionRandomSpread", 20.0),
            speed: num("cannonVelocity", 130.0),
            speed_spread: num("cannonVelocityRandomSpread", 20.0),
            color: rgba(ctx.color("cannonColor")),
            size: num("cannonParticleRadius", 2.0).max(0.0),
            text: letters(ctx, "cannonText"),
            font_size,
        });
        let (across, down) = (num("gridParticlesAcross", 0.0).max(0.0) as u32, num("gridParticlesDown", 0.0).max(0.0) as u32);
        let grid = (across > 0 && down > 0).then(|| Grid {
            at: pt("gridPosition"),
            w: num("gridWidth", 100.0),
            h: num("gridHeight", 100.0),
            across,
            down,
            color: rgba(ctx.color("gridColor")),
            size: num("gridParticleRadius", 2.0).max(0.0),
            text: letters(ctx, "gridText"),
            font_size,
        });
        let g = num("gravityForce", 108.0);
        let gd = num("gravityDirection", 180.0).to_radians();
        Knobs {
            cannon,
            grid,
            burst: (num("particleExploderRadiusOfNewParticles", 0.0).max(0.0), num("particleExploderVelocityDispersion", 20.0)),
            gravity: ([gd.sin() * g, -gd.cos() * g], num("gravityForceRandomSpread", 0.0).max(0.0) / 100.0),
            repel: (num("repelForce", 0.0), num("repelForceRadius", 0.0).max(0.0)),
        }
    }
}

impl World {
    fn read(ctx: &Ctx) -> World {
        let r = ctx.bounds;
        let wall = ctx
            .mask(ctx.num("wallBoundary", 0.0).round().max(0.0) as usize)
            .filter(|m| m.closed && m.points.len() >= 3)
            .map(|m| (m.points.iter().map(|p| [p[0] as f32, p[1] as f32]).collect(), Affects::of(ctx, "wall")));
        World {
            exploded: explode(ctx),
            bursts: Affects::of(ctx, "particleExploder"),
            falls: Affects::of(ctx, "gravity"),
            repels: Affects::of(ctx, "repel"),
            wall,
            lasting: Mapper::of(ctx, "persistentPropertyMapper", false),
            passing: Mapper::of(ctx, "ephemeralPropertyMapper", true),
            seed: crate::noise::mix(&[ctx.seed, ctx.num("randomSeed", 0.0) as u64, 29]),
            room: Rect::new(r.x0 - r.width(), r.y0 - r.height(), r.x1 + r.width(), r.y1 + r.height()),
            frame: ctx.frame.seconds().max(1e-4),
        }
    }

    fn fire(&self, c: &Cannon, id: u32) -> Particle {
        let (i, s) = (id as u64, self.seed);
        // 0° up, clockwise
        let a = (c.direction + rand2(i, 1, s) * c.spread * 0.5).to_radians();
        let speed = (c.speed + rand2(i, 3, s) * c.speed_spread).max(0.0);
        // a barrel above 0 is a square round the cannon, below 0 a disc
        let off = if c.radius >= 0.0 {
            [rand2(i, 2, s) * c.radius, rand2(i, 4, s) * c.radius]
        } else {
            let (b, rr) = (rand(i, 5, s) * TAU, rand(i, 6, s).sqrt() * -c.radius);
            [b.cos() * rr, b.sin() * rr]
        };
        let mut q = Particle::new([c.at[0] + off[0], c.at[1] + off[1]], [a.sin() * speed, -a.cos() * speed], c.color, c.size, CANNON, id);
        if !c.text.is_empty() {
            q.letter = c.text[id as usize % c.text.len()] as u32;
            q.font_size = c.font_size;
        }
        q
    }

    /// The particle as this step's passing mapper leaves it.
    fn passing(&self, q: &Particle) -> Particle {
        let mut e = *q;
        if let Some(m) = &self.passing {
            m.apply(&mut e);
        }
        e
    }

    fn step(&self, k: &Knobs, st: &mut State, n: u64) {
        let dt = (1.0 / RATE) as f32;
        let seed = self.seed;
        // the layer burst into particles at the start
        if !st.exploded {
            st.exploded = true;
            for q in &self.exploded {
                st.parts.push(Particle { id: st.next, ..*q });
                st.next = st.next.wrapping_add(1);
            }
        }
        // the grid's set, each frame
        if let Some(g) = &k.grid {
            let frame = ((n as f64 / RATE) / self.frame + 1e-6).floor() as i64;
            if frame >= st.grid_frames {
                st.grid_frames = frame + 1;
                let mut i = 0usize;
                for row in 0..g.down {
                    for col in 0..g.across {
                        if st.parts.len() >= 80_000 {
                            break;
                        }
                        let fx = if g.across > 1 { col as f32 / (g.across - 1) as f32 - 0.5 } else { 0.0 };
                        let fy = if g.down > 1 { row as f32 / (g.down - 1) as f32 - 0.5 } else { 0.0 };
                        let mut q = Particle::new([g.at[0] + fx * g.w, g.at[1] + fy * g.h], [0.0; 2], g.color, g.size, GRID, st.next);
                        st.next = st.next.wrapping_add(1);
                        if !g.text.is_empty() {
                            q.letter = g.text[i % g.text.len()] as u32;
                            q.font_size = g.font_size;
                        }
                        i += 1;
                        st.parts.push(q);
                    }
                }
            }
        }
        if let Some(m) = &self.lasting {
            st.parts.iter_mut().for_each(|q| m.apply(q));
        }
        let now: Vec<Particle> = st.parts.iter().map(|q| self.passing(q)).collect();
        // pushing apart: neighbours found through cells the reach wide
        let mut push = vec![[0f32; 2]; now.len()];
        let (strength, reach) = k.repel;
        if strength != 0.0 && reach > 0.0 && now.len() > 1 {
            let mut cells: std::collections::HashMap<(i32, i32), Vec<usize>> = std::collections::HashMap::new();
            for (i, q) in now.iter().enumerate() {
                cells.entry(((q.p[0] / reach).floor() as i32, (q.p[1] / reach).floor() as i32)).or_default().push(i);
            }
            for (i, f) in push.iter_mut().enumerate() {
                let q = now[i];
                let w = self.repels.weight(&q);
                if w <= 0.0 {
                    continue;
                }
                let (cx, cy) = ((q.p[0] / reach).floor() as i32, (q.p[1] / reach).floor() as i32);
                for gy in cy - 1..=cy + 1 {
                    for gx in cx - 1..=cx + 1 {
                        for &j in cells.get(&(gx, gy)).map(Vec::as_slice).unwrap_or(&[]) {
                            let o = now[j];
                            let (dx, dy) = (q.p[0] - o.p[0], q.p[1] - o.p[1]);
                            let d = dx.hypot(dy);
                            if j != i && d < reach && d > 1e-4 {
                                let s = w * strength * 100.0 * (1.0 - d / reach) / d;
                                (f[0], f[1]) = (f[0] + dx * s, f[1] + dy * s);
                            }
                        }
                    }
                }
            }
        }
        let (pull, spread) = k.gravity;
        for ((q, e), f) in st.parts.iter_mut().zip(&now).zip(&push) {
            let gw = self.falls.weight(e) * if spread > 0.0 { 1.0 + rand2(q.id as u64, 7, seed) * spread } else { 1.0 };
            let (mut ax, mut ay) = (pull[0] * gw + (e.force[0] + f[0]) / e.mass, pull[1] * gw + (e.force[1] + f[1]) / e.mass);
            // a particle at rest stays till the push is stronger than its static friction
            if e.v[0].abs() < 1e-3 && e.v[1].abs() < 1e-3 && ax.hypot(ay) <= e.static_friction {
                (ax, ay) = (0.0, 0.0);
            }
            let keep = (-e.friction * 6.0 * dt).exp();
            let nv = [(e.v[0] + ax * dt) * keep, (e.v[1] + ay * dt) * keep];
            // only what the step did stays (the passing mapper's own change goes)
            q.v = [q.v[0] + nv[0] - e.v[0], q.v[1] + nv[1] - e.v[1]];
            q.spin += e.torque * dt;
            q.angle += e.spin * dt;
            q.scale = [(q.scale[0] + e.scale_speed * dt).max(0.0), (q.scale[1] + e.scale_speed * dt).max(0.0)];
            q.age += dt;
            let np = [q.p[0] + nv[0] * dt, q.p[1] + nv[1] * dt];
            if let Some((poly, who)) = &self.wall
                && who.weight(e) > 0.5
                && inside(poly, q.p[0], q.p[1]) != inside(poly, np[0], np[1])
            {
                // off the wall it met: its speed into the wall turned back
                if let Some(nm) = crossed(poly, q.p, np) {
                    let d = q.v[0] * nm[0] + q.v[1] * nm[1];
                    q.v = [(q.v[0] - 2.0 * d * nm[0]) * 0.9, (q.v[1] - 2.0 * d * nm[1]) * 0.9];
                } else {
                    q.v = [-q.v[0] * 0.9, -q.v[1] * 0.9];
                }
                continue;
            }
            q.p = np;
        }
        let room = self.room;
        let lives: Vec<f32> = st.parts.iter().map(|q| self.passing(q).lifespan).collect();
        let mut i = 0;
        st.parts.retain(|q| {
            i += 1;
            room.contains(vec2(q.p[0] as f64, q.p[1] as f64)) && q.age <= lives[i - 1]
        });
        // bursting: a picked particle bigger than the new ones becomes several of them
        let (small, scatter) = k.burst;
        if small > 0.0 {
            let mut out = Vec::with_capacity(st.parts.len());
            for q in std::mem::take(&mut st.parts) {
                if q.from == EXPLODED_PARTICLE || q.letter != 0 || q.radius <= small * 1.01 || self.bursts.weight(&q) <= 0.5 || out.len() > 60_000 {
                    out.push(q);
                    continue;
                }
                let count = ((q.radius / small).powi(2).round() as u32).clamp(2, 16);
                for j in 0..count {
                    let id = st.next;
                    st.next = st.next.wrapping_add(1);
                    let a = (j as f32 + rand(id as u64, 31, seed)) / count as f32 * TAU;
                    let (sa, ca) = a.sin_cos();
                    let (rr, sp) = (q.radius - small, scatter * (0.5 + rand(id as u64, 32, seed)));
                    out.push(Particle { p: [q.p[0] + ca * rr, q.p[1] + sa * rr], v: [q.v[0] + ca * sp, q.v[1] + sa * sp], radius: small, from: EXPLODED_PARTICLE, id, ..q });
                }
            }
            st.parts = out;
        }
        if let Some(c) = &k.cannon {
            st.due += c.rate / RATE;
            let births = st.due.floor() as u32;
            st.due -= births as f64;
            for _ in 0..births {
                if st.parts.len() >= 80_000 {
                    break;
                }
                let id = st.next;
                st.next = st.next.wrapping_add(1);
                let mut q = self.fire(c, id);
                let early = rand(id as u64, 9, seed) * dt;
                q.p = [q.p[0] + q.v[0] * early, q.p[1] + q.v[1] * early];
                st.parts.push(q);
            }
        }
    }
}

/// The burst layer as particles: one every `size` pixels of it where it shows, its colour,
/// flying apart from its middle.
fn explode(ctx: &Ctx) -> Vec<Particle> {
    let Some(lid) = ctx.value_in(ctx.effect, "layerExploderExplodeLayer").and_then(|v| v.as_layer()) else { return Vec::new() };
    let Some((other, rect)) = ctx.host.and_then(|h| h.layer(lid, Take::All)) else { return Vec::new() };
    let size = ctx.num("layerExploderRadiusOfNewParticles", 2.0).max(0.5) as f32;
    let spread = ctx.num("layerExploderVelocityDispersion", 20.0) as f32;
    let seed = crate::noise::mix(&[ctx.seed, 31]);
    let r = ctx.bounds;
    // the other layer laid over this one, middle on middle
    let shift = [(r.center().x - rect.center().x) as f32, (r.center().y - rect.center().y) as f32];
    let mid = [r.center().x as f32, r.center().y as f32];
    let step = 2.0 * size;
    let (nx, ny) = ((rect.width() as f32 / step).ceil() as usize, (rect.height() as f32 / step).ceil() as usize);
    let mut out = Vec::new();
    for j in 0..ny.min(400) {
        for i in 0..nx.min(400) {
            let (x, y) = (rect.x0 as f32 + (i as f32 + 0.5) * step, rect.y0 as f32 + (j as f32 + 0.5) * step);
            let at = other.pixel(vec2(x as f64, y as f64));
            let (c, a) = unpremul(other.img.sample(Sampling::Bilinear, at.x, at.y));
            if a <= 0.05 {
                continue;
            }
            let p = [x + shift[0], y + shift[1]];
            let k = (j * nx + i) as u64;
            let (dx, dy) = (p[0] - mid[0], p[1] - mid[1]);
            let l = dx.hypot(dy).max(1e-3);
            let v = [dx / l * spread * rand(k, 1, seed) + rand2(k, 2, seed) * spread * 0.3, dy / l * spread * rand(k, 1, seed) + rand2(k, 3, seed) * spread * 0.3];
            out.push(Particle::new(p, v, [c[0], c[1], c[2], a], size, EXPLODED_LAYER, 0));
        }
    }
    out
}

macro_rules! playground_def {
    (affects: [$($a:literal),*], mappers: [$($m:literal),*]) => {
        pub static PLAYGROUND_DEF: veymelo_project::GroupDef = veymelo_project::GroupDef {
            id: "particlePlayground",
            name: "Particle Playground",
            doc: "particles from a cannon, a grid or a burst layer, under gravity, pushing apart, walled in, set by map layers; drawn as dots, letters or copies of a layer",
            body: veymelo_project::Body::Fixed(&[
                veymelo_project::DefNode::Prop(PropDef::new("cannonText", "Cannon Text", Kind::Text, Value::Text(String::new())).doc("letters the cannon fires in turn (empty: dots)")),
                veymelo_project::DefNode::Prop(PropDef::new("gridText", "Grid Text", Kind::Text, Value::Text(String::new())).doc("letters set on the grid in reading order")),
                veymelo_project::DefNode::Prop(PropDef::new("font", "Font", Kind::Text, Value::Text(String::new())).doc("for the letters; empty: the system's")),
                veymelo_project::DefNode::Prop(ranged("fontSize", "Font Size", 18.0, 1.0, 1000.0)),
                veymelo_project::DefNode::Prop(flag("autoOrient", "Auto-Orient Rotation", false).doc("letters and copies turned the way they fly")),
                veymelo_project::DefNode::Prop(point("cannonPosition", "Cannon Position")),
                veymelo_project::DefNode::Prop(ranged("cannonBarrelRadius", "Barrel Radius", 0.0, -1000.0, 1000.0).doc("above 0 a square, below 0 a disc")),
                veymelo_project::DefNode::Prop(ranged("cannonParticlesPerSecond", "Particles Per Second", 60.0, 0.0, 500.0)),
                veymelo_project::DefNode::Prop(angle("cannonDirection", "Direction", 0.0)),
                veymelo_project::DefNode::Prop(ranged("cannonDirectionRandomSpread", "Direction Random Spread", 20.0, 0.0, 360.0)),
                veymelo_project::DefNode::Prop(ranged("cannonVelocity", "Velocity", 130.0, 0.0, 1000.0)),
                veymelo_project::DefNode::Prop(ranged("cannonVelocityRandomSpread", "Velocity Random Spread", 20.0, 0.0, 500.0)),
                veymelo_project::DefNode::Prop(color("cannonColor", "Cannon Color", 1.0, 0.0, 0.0)),
                veymelo_project::DefNode::Prop(ranged("cannonParticleRadius", "Cannon Particle Radius", 2.0, 0.0, 50.0)),
                veymelo_project::DefNode::Prop(point("gridPosition", "Grid Position")),
                veymelo_project::DefNode::Prop(ranged("gridWidth", "Grid Width", 100.0, 0.0, 10_000.0)),
                veymelo_project::DefNode::Prop(ranged("gridHeight", "Grid Height", 100.0, 0.0, 10_000.0)),
                veymelo_project::DefNode::Prop(ranged("gridParticlesAcross", "Particles Across", 0.0, 0.0, 200.0)),
                veymelo_project::DefNode::Prop(ranged("gridParticlesDown", "Particles Down", 0.0, 0.0, 200.0)),
                veymelo_project::DefNode::Prop(color("gridColor", "Grid Color", 1.0, 1.0, 1.0)),
                veymelo_project::DefNode::Prop(ranged("gridParticleRadius", "Grid Particle Radius", 2.0, 0.0, 50.0)),
                veymelo_project::DefNode::Prop(layer("layerExploderExplodeLayer", "Explode Layer")),
                veymelo_project::DefNode::Prop(ranged("layerExploderRadiusOfNewParticles", "Layer Exploder: Radius of New Particles", 2.0, 0.5, 50.0)),
                veymelo_project::DefNode::Prop(ranged("layerExploderVelocityDispersion", "Layer Exploder: Velocity Dispersion", 20.0, 0.0, 500.0)),
                veymelo_project::DefNode::Prop(ranged("particleExploderRadiusOfNewParticles", "Particle Exploder: Radius of New Particles", 0.0, 0.0, 50.0).doc("0: no bursting")),
                veymelo_project::DefNode::Prop(ranged("particleExploderVelocityDispersion", "Particle Exploder: Velocity Dispersion", 20.0, 0.0, 500.0)),
                veymelo_project::DefNode::Prop(layer("layerMapUseLayer", "Layer Map: Use Layer").doc("each particle drawn as a copy of this layer")),
                veymelo_project::DefNode::Prop(choice("layerMapTimeOffsetType", "Time Offset Type", &["relative", "absolute", "relativeRandom", "absoluteRandom"], 0)),
                veymelo_project::DefNode::Prop(ranged("layerMapTimeOffset", "Time Offset", 0.0, -1000.0, 1000.0)),
                veymelo_project::DefNode::Prop(ranged("gravityForce", "Gravity Force", 108.0, -1000.0, 1000.0)),
                veymelo_project::DefNode::Prop(ranged("gravityForceRandomSpread", "Gravity Force Random Spread", 0.0, 0.0, 100.0)),
                veymelo_project::DefNode::Prop(angle("gravityDirection", "Gravity Direction", 180.0)),
                veymelo_project::DefNode::Prop(ranged("repelForce", "Repel Force", 0.0, -100.0, 100.0)),
                veymelo_project::DefNode::Prop(ranged("repelForceRadius", "Repel Force Radius", 0.0, 0.0, 1000.0)),
                veymelo_project::DefNode::Prop(ranged("wallBoundary", "Wall Boundary", 0.0, 0.0, 100.0).doc("the number of the layer's mask particles stay inside (0: none)")),
                $(
                    veymelo_project::DefNode::Prop(layer(concat!($m, "UseLayerAsMap"), "Use Layer As Map")),
                    veymelo_project::DefNode::Prop(choice(concat!($m, "MapRedTo"), "Map Red To", PROPERTIES, 0)),
                    veymelo_project::DefNode::Prop(choice(concat!($m, "RedOperator"), "Red Operator", OPERATORS, 0).doc("(the ephemeral mapper's) how the map's value meets the property")),
                    veymelo_project::DefNode::Prop(ranged(concat!($m, "RedMin"), "Red Min", 0.0, -10_000.0, 10_000.0)),
                    veymelo_project::DefNode::Prop(ranged(concat!($m, "RedMax"), "Red Max", 1.0, -10_000.0, 10_000.0)),
                    veymelo_project::DefNode::Prop(choice(concat!($m, "MapGreenTo"), "Map Green To", PROPERTIES, 0)),
                    veymelo_project::DefNode::Prop(choice(concat!($m, "GreenOperator"), "Green Operator", OPERATORS, 0)),
                    veymelo_project::DefNode::Prop(ranged(concat!($m, "GreenMin"), "Green Min", 0.0, -10_000.0, 10_000.0)),
                    veymelo_project::DefNode::Prop(ranged(concat!($m, "GreenMax"), "Green Max", 1.0, -10_000.0, 10_000.0)),
                    veymelo_project::DefNode::Prop(choice(concat!($m, "MapBlueTo"), "Map Blue To", PROPERTIES, 0)),
                    veymelo_project::DefNode::Prop(choice(concat!($m, "BlueOperator"), "Blue Operator", OPERATORS, 0)),
                    veymelo_project::DefNode::Prop(ranged(concat!($m, "BlueMin"), "Blue Min", 0.0, -10_000.0, 10_000.0)),
                    veymelo_project::DefNode::Prop(ranged(concat!($m, "BlueMax"), "Blue Max", 1.0, -10_000.0, 10_000.0)),
                )*
                $(
                    veymelo_project::DefNode::Prop(choice(concat!($a, "AffectsParticlesFrom"), concat!($a, ": Particles From"), FROM, 0)),
                    veymelo_project::DefNode::Prop(layer(concat!($a, "AffectsSelectionMap"), concat!($a, ": Selection Map"))),
                    veymelo_project::DefNode::Prop(PropDef::new(concat!($a, "AffectsCharacters"), concat!($a, ": Characters"), Kind::Text, Value::Text(String::new()))),
                    veymelo_project::DefNode::Prop(ranged(concat!($a, "AffectsOlderYoungerThan"), concat!($a, ": Older/Younger Than"), 0.0, -1000.0, 1000.0)),
                    veymelo_project::DefNode::Prop(ranged(concat!($a, "AffectsAgeFeather"), concat!($a, ": Age Feather"), 0.0, 0.0, 1000.0)),
                )*
                veymelo_project::DefNode::Prop(ranged("randomSeed", "Random Seed", 0.0, 0.0, 10_000.0)),
            ]),
        };
    };
}

playground_def!(affects: ["particleExploder", "layerMap", "gravity", "repel", "wall", "persistentPropertyMapper", "ephemeralPropertyMapper"], mappers: ["persistentPropertyMapper", "ephemeralPropertyMapper"]);

pub const PLAYGROUND: Spec = Spec::new(&PLAYGROUND_DEF, "simulation", playground).points(&[("cannonPosition", [0.5, 0.9]), ("gridPosition", [0.5, 0.5])]);

static KEPT: Stepped<State> = Stepped::new();

/// A number standing for a picture's pixels (every seventh read).
fn picture_key(img: &Image) -> u64 {
    let mut h = crate::noise::mix(&[img.width as u64, img.height as u64]);
    for p in img.px.iter().step_by(7) {
        for c in p {
            h = crate::noise::hash(h ^ c.to_bits() as u64);
        }
    }
    h
}

fn playground(ctx: &Ctx, mut b: Buf) -> Buf {
    let t = ctx.time.seconds();
    let world = World::read(ctx);
    // what the simulation depends on: its settings, and the pixels of the layers it reads
    let mut parts = vec![key_of(ctx, 4)];
    for m in [&world.lasting, &world.passing].into_iter().flatten() {
        parts.push(picture_key(&m.map.img));
    }
    for a in [&world.bursts, &world.falls, &world.repels] {
        if let Some(m) = &a.selection {
            parts.push(picture_key(&m.img));
        }
    }
    parts.extend(world.exploded.iter().step_by(7).map(|q| (q.color[0].to_bits() as u64) ^ ((q.p[0].to_bits() as u64) << 32)));
    let key = crate::noise::mix(&parts);
    let state = KEPT.run(key, step_at(t), State::default, |s, n| world.step(&Knobs::read(ctx, n as f64 / RATE), s, n));
    let extra = (t - step_at(t) as f64 / RATE).max(0.0) as f32;
    let shown: Vec<Particle> = state
        .parts
        .iter()
        .map(|q| {
            let mut e = world.passing(q);
            e.p = [e.p[0] + e.v[0] * extra, e.p[1] + e.v[1] * extra];
            e
        })
        .collect();
    b.img = render(ctx, &b, &shown);
    b
}

/// Letters' outlines at 100 units to an em, by letter.
type Glyphs = std::collections::HashMap<u32, Path>;

/// The particles drawn on a clear picture of `b`'s size: as copies of the layer map's layer
/// where it acts, as their letters, else as dots.
fn render(ctx: &Ctx, b: &Buf, shown: &[Particle]) -> Image {
    let (w, h) = (b.img.width, b.img.height);
    let s = b.scale as f32;
    let mut out = Image::new(w, h);
    let orient = ctx.flag("autoOrient");
    let turn_of = |q: &Particle| if orient && q.v[0].hypot(q.v[1]) > 1e-3 { q.v[1].atan2(q.v[0]).to_degrees() + 90.0 } else { q.angle };
    // the layer map: its layer at each particle's moment
    let map_id = ctx.value_in(ctx.effect, "layerMapUseLayer").and_then(|v| v.as_layer());
    let map_who = Affects::of(ctx, "layerMap");
    let (kind, base) = (ctx.choice("layerMapTimeOffsetType"), ctx.num("layerMapTimeOffset", 0.0) as f32);
    let fd = ctx.frame.seconds().max(1e-4);
    let now = ctx.time.seconds();
    let mut frames: Vec<(i64, Option<(Buf, Rect)>)> = Vec::new();
    // letters
    let face = crate::words::font_for(&ctx.value_in(ctx.effect, "font").map(|v| v.as_text().to_string()).unwrap_or_default(), 400.0);
    let mut glyphs: Glyphs = Glyphs::new();
    let mut dots: Vec<Sprite> = Vec::new();
    // drawn in turn: the dots gathered and drawn before each copy or letter, so the order holds
    let flush = |dots: &mut Vec<Sprite>, out: &mut Image| {
        draw(out, dots, false);
        dots.clear();
    };
    for q in shown {
        if q.color[3] <= 0.0 {
            continue;
        }
        let at = b.pixel(vec2(q.p[0] as f64, q.p[1] as f64));
        let (x, y) = (at.x as f32, at.y as f32);
        if let Some(id) = map_id.filter(|_| map_who.weight(q) > 0.5) {
            let rnd = rand(q.id as u64, 41, 7);
            let off = q.time_offset + base * if kind >= 2 { rnd } else { 1.0 };
            let when = if kind == 1 || kind == 3 { off as f64 } else { now + off as f64 };
            let frame = (when / fd).floor() as i64;
            let found = match frames.iter().position(|(f, _)| *f == frame) {
                Some(i) => i,
                None if frames.len() < 32 => {
                    let got = ctx.host.and_then(|hst| hst.layer_at(id, Tick::from_seconds(frame as f64 * fd - now), Take::All));
                    frames.push((frame, got));
                    frames.len() - 1
                }
                None => 0,
            };
            if let Some((img, rect)) = frames.get(found).and_then(|(_, g)| g.as_ref()) {
                flush(&mut dots, &mut out);
                stamp(&mut out, img, rect, [x, y], turn_of(q), [q.scale[0] * s, q.scale[1] * s], q.color[3]);
            }
            continue;
        }
        if q.letter != 0
            && let Some(face) = &face
        {
            let outline = glyphs.entry(q.letter).or_insert_with(|| char::from_u32(q.letter).map_or_else(Path::default, |c| crate::words::set_line(&c.to_string(), face, 100.0, 0.0, false, 1, vec2(0.0, 0.0)).0));
            flush(&mut dots, &mut out);
            letter(&mut out, outline, x, y, turn_of(q), [q.scale[0] * q.font_size / 100.0 * s, q.scale[1] * q.font_size / 100.0 * s], q.color);
            continue;
        }
        let r = q.radius * (q.scale[0] + q.scale[1]) * 0.5 * s;
        dots.push(Sprite { x, y, r, color: q.color, shape: Shape::Disc, turn: 0.0 });
    }
    flush(&mut dots, &mut out);
    out
}

/// Another layer's picture laid at pixel (x, y), its middle there, turned and scaled (pixels
/// to its unit).
fn stamp(out: &mut Image, src: &Buf, rect: &Rect, [x, y]: [f32; 2], turn: f32, scale: [f32; 2], opacity: f32) {
    let (sn, cs) = turn.to_radians().sin_cos();
    let (hw, hh) = (rect.width() as f32 * 0.5 * scale[0], rect.height() as f32 * 0.5 * scale[1]);
    let reach = hw.hypot(hh) + 1.0;
    let (x0, x1) = ((x - reach).floor().max(0.0) as usize, ((x + reach).ceil().max(0.0) as usize).min(out.width));
    let (y0, y1) = ((y - reach).floor().max(0.0) as usize, ((y + reach).ceil().max(0.0) as usize).min(out.height));
    let mid = rect.center();
    for py in y0..y1 {
        for px in x0..x1 {
            let (dx, dy) = (px as f32 + 0.5 - x, py as f32 + 0.5 - y);
            // back to the layer's own units
            let (u, v) = ((dx * cs + dy * sn) / scale[0].max(1e-6), (-dx * sn + dy * cs) / scale[1].max(1e-6));
            let p = src.pixel(vec2(mid.x + u as f64, mid.y + v as f64));
            let c = src.img.sample(Sampling::Bilinear, p.x, p.y).map(|v| v * opacity);
            if c[3] > 0.0 {
                let d = &mut out.px[py * out.width + px];
                let k = 1.0 - c[3];
                *d = [c[0] + d[0] * k, c[1] + d[1] * k, c[2] + d[2] * k, c[3] + d[3] * k];
            }
        }
    }
}

/// A letter's outline (100 units to an em, centred) filled at pixel (x, y), turned and scaled.
fn letter(out: &mut Image, outline: &Path, x: f32, y: f32, turn: f32, scale: [f32; 2], color: [f32; 4]) {
    let (sn, cs) = turn.to_radians().sin_cos();
    let mut p = Path::default();
    for c in outline.contours() {
        for (k, q) in c.iter().enumerate() {
            let (u, v) = (q.0 as f32 * scale[0], q.1 as f32 * scale[1]);
            let (px, py) = ((x + u * cs - v * sn) as f64, (y + u * sn + v * cs) as f64);
            if k == 0 {
                p.move_to(px, py);
            } else {
                p.line_to(px, py);
            }
        }
        p.close();
    }
    let Some(m) = veymelo_path::fill(&p, Rule::NonZero, out.width, out.height) else { return };
    for py in m.top..(m.top + m.height).min(out.height) {
        for px in m.left..(m.left + m.width).min(out.width) {
            let k = m.at(px, py) * color[3];
            if k > 0.0 {
                let d = &mut out.px[py * out.width + px];
                let (src, under) = ([color[0] * k, color[1] * k, color[2] * k, k], 1.0 - k);
                *d = [src[0] + d[0] * under, src[1] + d[1] * under, src[2] + d[2] * under, src[3] + d[3] * under];
            }
        }
    }
}

#[cfg(test)]
mod tests;

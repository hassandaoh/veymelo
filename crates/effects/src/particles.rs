//! Particles: a producer giving birth to particles that fly by one of eleven motions under
//! gravity and resistance (bouncing on or sticking to a floor), growing or shrinking, changing
//! colour and fading as they age — seen in 3D through a camera (Particle World) or flat on the
//! layer (Particle Systems); and particles as drops of liquid metal running together (Mercury).
//!
//! The particles are simulated from the layer's start in steps of a sixtieth of a second
//! ([`crate::stepper`]), each step reading the settings at its own time (so they may be
//! animated); between steps the particles are carried on by their speed.

use std::f32::consts::TAU;

use veymelo_raster::{Image, Px, Sampling};
use veymelo_time::Tick;

use crate::defs::{angle, choice, color, effect, layer, pct, point, ranged};
use crate::sprites::{Shape, Sprite, draw_with, rand, rand2};
use crate::stepper::{RATE, Stepped, key_of, step_at};
use crate::util::{map_px, unpremul};
use crate::{Buf, Ctx, Spec, Take};

const CATEGORY: &str = "simulation";

/// A particle (in world units).
#[derive(Clone, Debug)]
pub struct Particle {
    pub p: [f32; 3],
    pub v: [f32; 3],
    pub age: f32,
    pub life: f32,
    /// Its own random number, 0 to 1 (size, colour and spin vary by it).
    pub rnd: f32,
    pub id: u32,
}

/// Particles alive, and the births owed.
#[derive(Clone, Debug, Default)]
pub struct Swarm {
    pub parts: Vec<Particle>,
    due: f64,
    next: u32,
}

/// The ways particles move.
pub const MOTIONS: &[&str] = &["explosive", "directionAxis", "coneAxis", "viscouse", "twirly", "twirl", "vortex", "fire", "jetSideways", "fractalOmni", "fractalUni"];

/// How particles are born and move at one moment (world units, y down).
#[derive(Clone, Copy, Debug)]
pub struct Physics {
    /// Births a second.
    pub rate: f64,
    pub life: f32,
    pub at: [f32; 3],
    pub radius: [f32; 3],
    pub motion: u32,
    pub speed: f32,
    pub gravity: [f32; 3],
    pub resistance: f32,
    pub axis: [f32; 3],
    pub extra: f32,
    /// A cone's opening (degrees).
    pub cone: f32,
    /// Where the floor is (down), what it does (1: bounce, 2: stick) and how much a bounce
    /// keeps.
    pub floor: Option<(f32, u32, f32)>,
    pub seed: u64,
    /// On the layer's plane (no depth).
    pub flat: bool,
    pub most: usize,
}

fn unit3(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-9 { v.map(|c| c / l) } else { [0.0, -1.0, 0.0] }
}

/// A direction of its own for particle `i` (round on the plane when `flat`, over the sphere
/// otherwise).
fn heading(i: u64, k: u64, seed: u64, flat: bool) -> [f32; 3] {
    if flat {
        let a = rand(i, k, seed) * TAU;
        return [a.cos(), a.sin(), 0.0];
    }
    let z = rand2(i, k, seed);
    let a = rand(i, k + 1, seed) * TAU;
    let r = (1.0 - z * z).max(0.0).sqrt();
    [r * a.cos(), r * a.sin(), z]
}

/// Smooth noise from about -0.5 to 0.5.
fn wobble(x: f32, y: f32, z: f32, seed: u64) -> f32 {
    crate::noise::smooth(x as f64, y as f64, z as f64, seed, None) as f32 * 0.5
}

impl Physics {
    fn born(&self, id: u32) -> Particle {
        let (i, seed) = (id as u64, self.seed);
        // anywhere in the producer's ball, evenly
        let u = heading(i, 1, seed, self.flat);
        let rr = rand(i, 3, seed).powf(if self.flat { 0.5 } else { 1.0 / 3.0 });
        let mut p = [0, 1, 2].map(|k| self.at[k] + u[k] * self.radius[k] * rr);
        let ax = unit3(self.axis);
        let j = heading(i, 5, seed, self.flat);
        let dir = match self.motion {
            1 | 10 => unit3([ax[0] + j[0] * 0.08, ax[1] + j[1] * 0.08, ax[2] + j[2] * 0.08]),
            2 => {
                let k = (self.cone.to_radians() * 0.5).tan().abs().min(20.0) * rand(i, 8, seed);
                unit3([ax[0] + j[0] * k, ax[1] + j[1] * k, ax[2] + j[2] * k])
            }
            5 | 6 => unit3([j[0], j[1] * 0.2, if self.flat { 0.0 } else { j[2] }]),
            7 => unit3([j[0] * 0.25, -1.0, j[2] * 0.25]),
            8 => unit3([1.0, j[1] * 0.05, j[2] * 0.05]),
            _ => j,
        };
        let sp = self.speed * (0.5 + 0.5 * rand(i, 9, seed));
        let mut v = dir.map(|d| d * sp);
        if self.flat {
            (p[2], v[2]) = (0.0, 0.0);
        }
        Particle { p, v, age: 0.0, life: self.life * (0.8 + 0.4 * rand(i, 10, seed)), rnd: rand(i, 11, seed), id }
    }

    /// One step `n` of the swarm.
    pub fn step(&self, s: &mut Swarm, n: u64) {
        let dt = (1.0 / RATE) as f32;
        let t = n as f32 * dt;
        let drag = self.resistance + if self.motion == 3 { 3.0 * self.extra.max(0.1) } else { 0.0 };
        let keep = (-drag * dt).exp();
        let (seed, c) = (self.seed, self.at);
        for q in &mut s.parts {
            let mut a = self.gravity;
            let (dx, dy, dz) = (q.p[0] - c[0], q.p[1] - c[1], q.p[2] - c[2]);
            match self.motion {
                // out and round the up axis (on the layer: round the producer)
                4 => {
                    let k = 4.0 * self.extra;
                    if self.flat {
                        (a[0], a[1]) = (a[0] - dy * k, a[1] + dx * k);
                    } else {
                        (a[0], a[2]) = (a[0] - dz * k, a[2] + dx * k);
                    }
                }
                // round the producer, held near it
                5 => {
                    let k = 5.0 * self.extra;
                    if self.flat {
                        (a[0], a[1]) = (a[0] - dy * k - dx * 3.0, a[1] + dx * k - dy * 3.0);
                    } else {
                        (a[0], a[2]) = (a[0] - dz * k - dx * 3.0, a[2] + dx * k - dz * 3.0);
                    }
                }
                // a whirlpool: round and in, on the layer's plane
                6 => {
                    let k = 6.0 * self.extra;
                    (a[0], a[1]) = (a[0] - dy * k - dx * 2.0, a[1] + dx * k - dy * 2.0);
                }
                // flames: rising against gravity, flickering
                7 => {
                    a[0] += wobble(q.p[0] * 8.0, q.p[1] * 8.0, t * 3.0, seed) * 4.0 * self.extra;
                    a[1] -= self.gravity[1] * 1.5 + 0.3 * self.extra;
                }
                // tossed by a turbulent field
                9 | 10 => {
                    let f = 6.0 * self.extra;
                    a[0] += wobble(q.p[0] * 5.0, q.p[1] * 5.0, q.p[2] * 5.0 + t, seed) * f;
                    a[1] += wobble(q.p[0] * 5.0 + 17.0, q.p[1] * 5.0, q.p[2] * 5.0 + t, seed) * f;
                    if !self.flat {
                        a[2] += wobble(q.p[0] * 5.0, q.p[1] * 5.0 + 31.0, q.p[2] * 5.0 + t, seed) * f;
                    }
                }
                _ => {}
            }
            for ((v, p), a) in q.v.iter_mut().zip(q.p.iter_mut()).zip(a) {
                *v = (*v + a * dt) * keep;
                *p += *v * dt;
            }
            if self.flat {
                (q.p[2], q.v[2]) = (0.0, 0.0);
            }
            if let Some((y, action, bounce)) = self.floor
                && q.p[1] > y
            {
                match action {
                    1 => {
                        q.p[1] = y - (q.p[1] - y) * bounce;
                        q.v[1] = -q.v[1] * bounce;
                        q.v[0] *= 0.95;
                        q.v[2] *= 0.95;
                    }
                    2 => {
                        q.p[1] = y;
                        q.v = [0.0; 3];
                    }
                    _ => {}
                }
            }
            q.age += dt;
        }
        s.parts.retain(|q| q.age < q.life);
        s.due += self.rate / RATE;
        let births = s.due.floor().max(0.0) as u32;
        s.due -= births as f64;
        for _ in 0..births {
            if s.parts.len() >= self.most {
                break;
            }
            let id = s.next;
            s.next = s.next.wrapping_add(1);
            // born part of the way through the step, so a stream does not come in clumps
            let mut q = self.born(id);
            let early = rand(id as u64, 12, seed) * dt;
            for k in 0..3 {
                q.p[k] += q.v[k] * early;
            }
            q.age = early;
            s.parts.push(q);
        }
    }
}

/// The swarm at layer time `t`, its particles carried on to the moment between steps.
fn swarm_at(kept: &Stepped<Swarm>, key: u64, t: f64, physics: impl Fn(f64) -> Physics) -> Vec<Particle> {
    let n = step_at(t);
    let s = kept.run(key, n, Swarm::default, |s, k| physics(k as f64 / RATE).step(s, k));
    let extra = (t - n as f64 / RATE).max(0.0) as f32;
    s.parts
        .iter()
        .map(|q| {
            let mut q = q.clone();
            for k in 0..3 {
                q.p[k] += q.v[k] * extra;
            }
            q.age += extra;
            q
        })
        .filter(|q| q.age < q.life)
        .collect()
}

/// A particle's colour and size at its age: from its birth's to its death's, faded as the
/// opacity map says (0: in and out, 1: out, 2: in, 3: constant).
fn look_of(q: &Particle, birth: [f32; 3], death: [f32; 3], sizes: (f32, f32, f32), most: f32, fade: u32) -> ([f32; 4], f32) {
    let u = (q.age / q.life.max(1e-4)).clamp(0.0, 1.0);
    let size = (sizes.0 + (sizes.1 - sizes.0) * u) * (1.0 + sizes.2 * (q.rnd * 2.0 - 1.0));
    let (din, dout) = ((u * 20.0).min(1.0), ((1.0 - u) * 5.0).min(1.0));
    let k = match fade {
        1 => 1.0 - u,
        2 => u.min(1.0),
        3 => 1.0,
        _ => din * dout,
    };
    let c = [0, 1, 2].map(|i| birth[i] + (death[i] - birth[i]) * u);
    ([c[0], c[1], c[2], most * k], size.max(0.0))
}

/// The particle kinds.
pub const KINDS: &[&str] = &["line", "shadedSphere", "fadedSphere", "bubble", "star", "motionPolygon", "triPolygon", "square", "texturedSquare", "texturedDisc"];

/// A particle drawn as `kind` at pixel (x, y), `r` across each side, moving (vx, vy) pixels a
/// second.
fn mark(kind: u32, [x, y]: [f32; 2], r: f32, c: [f32; 4], [vx, vy]: [f32; 2], turn: f32) -> Sprite {
    let speed = vx.hypot(vy).max(1e-6);
    let (shape, r) = match kind {
        0 => (Shape::Streak { dx: vx / speed * r * 2.0, dy: vy / speed * r * 2.0 }, (r * 0.12).max(0.5)),
        1 => (Shape::Ball, r),
        2 => (Shape::Soft, r),
        3 => (Shape::Bubble, r),
        4 => (Shape::Glint, r),
        5 => (Shape::Streak { dx: vx * 0.05, dy: vy * 0.05 }, r * 0.5),
        6 => (Shape::Triangle, r),
        8 => (Shape::Picture { index: 0, round: false }, r),
        9 => (Shape::Picture { index: 0, round: true }, r),
        _ => (Shape::Square, r),
    };
    Sprite { x, y, r, color: c, shape, turn }
}

/// The picture a particle texture setting names (its layer at the time), if any.
fn texture(ctx: &Ctx, id: &str) -> Vec<Image> {
    let Some(lid) = ctx.value_in(ctx.effect, id).and_then(|v| v.as_layer()) else { return Vec::new() };
    ctx.host.and_then(|h| h.layer(lid, Take::All)).map(|(b, _)| vec![b.img]).unwrap_or_default()
}

/// Particles drawn, meeting each other as the transfer mode says (0 composite, 1 screen, 2 add,
/// 3 black background); they take the layer's place.
fn drawn(mut b: Buf, sprites: &[Sprite], mode: u32, pictures: &[Image]) -> Buf {
    let mut marks = Image::new(b.img.width, b.img.height);
    match mode {
        1 => {
            // screen: each over the rest as light, never past white
            for s in sprites {
                let mut one = Image::new(marks.width, marks.height);
                draw_with(&mut one, std::slice::from_ref(s), false, pictures);
                for (d, m) in marks.px.iter_mut().zip(&one.px) {
                    *d = [0, 1, 2, 3].map(|k| d[k] + m[k] - d[k] * m[k]);
                }
            }
        }
        2 => draw_with(&mut marks, sprites, true, pictures),
        3 => {
            marks.px.iter_mut().for_each(|p| *p = [0.0, 0.0, 0.0, 1.0]);
            draw_with(&mut marks, sprites, false, pictures);
        }
        _ => draw_with(&mut marks, sprites, false, pictures),
    }
    b.img = marks;
    b
}

static WORLD: Stepped<Swarm> = Stepped::new();

effect!(
    WORLD_DEF,
    "particleWorld",
    "Particle World",
    "particles in 3D, born from a producer and flying by one of eleven motions, seen through a camera",
    [
        ranged("birthRate", "Birth Rate", 0.5, 0.0, 100.0),
        ranged("longevity", "Longevity (sec)", 1.0, 0.0, 100.0),
        ranged("producerX", "Producer Position X", 0.0, -100.0, 100.0).doc("in world units: the layer is 1 wide"),
        ranged("producerY", "Producer Position Y", 0.0, -100.0, 100.0),
        ranged("producerZ", "Producer Position Z", 0.0, -100.0, 100.0),
        ranged("radiusX", "Producer Radius X", 0.025, 0.0, 100.0),
        ranged("radiusY", "Producer Radius Y", 0.025, 0.0, 100.0),
        ranged("radiusZ", "Producer Radius Z", 0.025, 0.0, 100.0),
        choice("animation", "Animation", MOTIONS, 0),
        ranged("velocity", "Velocity", 1.0, -100.0, 100.0),
        ranged("gravity", "Gravity", 0.5, -100.0, 100.0),
        ranged("resistance", "Resistance", 0.0, 0.0, 100.0),
        ranged("extra", "Extra", 0.5, -10.0, 10.0).doc("how strongly the motion acts (swirl, turbulence, flicker)"),
        angle("extraAngle", "Extra Angle", 90.0).doc("a cone's opening"),
        choice("floorAction", "Floor Action", &["none", "bounce", "stick"], 0),
        ranged("floorPosition", "Floor Position", 0.5, -100.0, 100.0),
        pct("bounciness", "Bounciness", 50.0, 0.0, 100.0),
        ranged("axisX", "Direction Axis X", 0.0, -1.0, 1.0),
        ranged("axisY", "Direction Axis Y", -1.0, -1.0, 1.0),
        ranged("axisZ", "Direction Axis Z", 0.0, -1.0, 1.0),
        ranged("gravityX", "Gravity Vector X", 0.0, -1.0, 1.0),
        ranged("gravityY", "Gravity Vector Y", 1.0, -1.0, 1.0),
        ranged("gravityZ", "Gravity Vector Z", 0.0, -1.0, 1.0),
        choice("particleType", "Particle Type", KINDS, 0),
        layer("texture", "Texture Layer").doc("for the textured kinds"),
        ranged("rotationSpeed", "Rotation Speed", 0.0, -3600.0, 3600.0).doc("degrees a second"),
        ranged("birthSize", "Birth Size", 0.02, 0.0, 100.0),
        ranged("deathSize", "Death Size", 0.04, 0.0, 100.0),
        pct("sizeVariation", "Size Variation", 0.0, 0.0, 100.0),
        pct("maxOpacity", "Max Opacity", 75.0, 0.0, 100.0),
        choice("opacityMap", "Opacity Map", &["fadeInOut", "fadeOut", "fadeIn", "constant"], 0),
        color("birthColor", "Birth Color", 1.0, 1.0, 0.0),
        color("deathColor", "Death Color", 0.6, 0.0, 0.0),
        choice("transferMode", "Transfer Mode", &["composite", "screen", "add", "blackBackground"], 0),
        ranged("cameraDistance", "Camera Distance", 2.0, 0.01, 100.0),
        angle("cameraRotationX", "Camera Rotation X", 0.0),
        angle("cameraRotationY", "Camera Rotation Y", 0.0),
        angle("cameraRotationZ", "Camera Rotation Z", 0.0),
        ranged("cameraFov", "Camera FOV", 50.0, 1.0, 179.0),
        choice("depthCue", "Depth Cue", &["none", "fade", "fog"], 0),
        ranged("depthCueDistance", "Depth Cue Distance", 3.0, 0.0, 100.0),
        color("fogColor", "Fog Color", 0.0, 0.0, 0.0),
        ranged("randomSeed", "Random Seed", 0.0, 0.0, 10_000.0),
    ]
);
pub const WORLD_SPEC: Spec = Spec::new(&WORLD_DEF, CATEGORY, particle_world);

/// A setting at layer time `t`.
fn at(ctx: &Ctx, id: &str, t: f64, d: f64) -> f64 {
    ctx.value_at(id, Tick::from_seconds(t)).map_or(d, |v| v.as_f64())
}

fn world_physics(ctx: &Ctx, t: f64) -> Physics {
    let f = |id: &str, d: f64| at(ctx, id, t, d) as f32;
    let g = f("gravity", 0.5);
    Physics {
        rate: at(ctx, "birthRate", t, 0.5).max(0.0) * 120.0,
        life: f("longevity", 1.0).max(0.0),
        at: [f("producerX", 0.0), f("producerY", 0.0), f("producerZ", 0.0)],
        radius: [f("radiusX", 0.025).max(0.0), f("radiusY", 0.025).max(0.0), f("radiusZ", 0.025).max(0.0)],
        motion: ctx.choice("animation").min(MOTIONS.len() as u32 - 1),
        speed: f("velocity", 1.0) * 0.5,
        gravity: [f("gravityX", 0.0) * g, f("gravityY", 1.0) * g, f("gravityZ", 0.0) * g],
        resistance: f("resistance", 0.0).max(0.0),
        axis: [f("axisX", 0.0), f("axisY", -1.0), f("axisZ", 0.0)],
        extra: f("extra", 0.5),
        cone: f("extraAngle", 90.0),
        floor: match ctx.choice("floorAction") {
            0 => None,
            a => Some((f("floorPosition", 0.5), a, (f("bounciness", 50.0) / 100.0).clamp(0.0, 1.0))),
        },
        seed: crate::noise::mix(&[ctx.seed, ctx.num("randomSeed", 0.0) as u64, 1]),
        flat: false,
        most: 60_000,
    }
}

/// Turning by angles about x, then y, then z (degrees).
fn turning(ax: f32, ay: f32, az: f32) -> [[f32; 3]; 3] {
    let (sx, cx) = ax.to_radians().sin_cos();
    let (sy, cy) = ay.to_radians().sin_cos();
    let (sz, cz) = az.to_radians().sin_cos();
    let mul = |a: [[f32; 3]; 3], b: [[f32; 3]; 3]| -> [[f32; 3]; 3] { [0, 1, 2].map(|i| [0, 1, 2].map(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum())) };
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    mul(rz, mul(ry, rx))
}

fn particle_world(ctx: &Ctx, b: Buf) -> Buf {
    let t = ctx.time.seconds();
    let key = key_of(ctx, 1);
    let parts = swarm_at(&WORLD, key, t, |t| world_physics(ctx, t));
    let r = ctx.bounds;
    let (lw, s) = (r.width() as f32, b.scale as f32);
    let centre = r.center();
    // the camera: back along -z, the world turned about its origin; keeping the origin's plane
    // at the layer's size as the field of view changes (so it sets the strength of perspective)
    let half = |fov: f32| (fov.clamp(1.0, 179.0).to_radians() * 0.5).tan();
    let zoom = half(50.0) / half(ctx.num("cameraFov", 50.0) as f32);
    let dist = ctx.num("cameraDistance", 2.0).max(0.01) as f32 * zoom;
    let focal = 2.0 * zoom;
    let turn = turning(ctx.num("cameraRotationX", 0.0) as f32, ctx.num("cameraRotationY", 0.0) as f32, ctx.num("cameraRotationZ", 0.0) as f32);
    let rot = |q: [f32; 3]| turn.map(|row| row[0] * q[0] + row[1] * q[1] + row[2] * q[2]);
    let kind = ctx.choice("particleType");
    let (bc, dc) = (ctx.color("birthColor"), ctx.color("deathColor"));
    let (birth, death) = ([bc.r as f32, bc.g as f32, bc.b as f32], [dc.r as f32, dc.g as f32, dc.b as f32]);
    let sizes = (ctx.num("birthSize", 0.02) as f32, ctx.num("deathSize", 0.04) as f32, (ctx.num("sizeVariation", 0.0) / 100.0) as f32);
    let most = (ctx.num("maxOpacity", 75.0) / 100.0) as f32;
    let fade = ctx.choice("opacityMap");
    let spin = (ctx.num("rotationSpeed", 0.0) as f32).to_radians();
    let (cue, cue_d) = (ctx.choice("depthCue"), ctx.num("depthCueDistance", 3.0).max(1e-3) as f32);
    let fog = ctx.color("fogColor");
    let mut seen: Vec<(f32, Sprite)> = parts
        .iter()
        .filter_map(|q| {
            let (p, v) = (rot(q.p), rot(q.v));
            let z = dist + p[2];
            if !z.is_finite() || z <= 0.025 * dist {
                return None;
            }
            let k = focal / z;
            let at = b.pixel(veymelo_geom::vec2(centre.x + (p[0] * lw * k) as f64, centre.y + (p[1] * lw * k) as f64));
            let (mut c, size) = look_of(q, birth, death, sizes, most, fade);
            // depth cue: past its distance, fading away or into fog
            let far = ((z - dist) / cue_d).clamp(0.0, 1.0);
            match cue {
                1 => c[3] *= 1.0 - far,
                2 => (0..3).for_each(|i| c[i] += ([fog.r, fog.g, fog.b][i] as f32 - c[i]) * far),
                _ => {}
            }
            let px = size * 0.5 * lw * k * s;
            let turn = q.rnd * TAU + q.age * spin;
            Some((z, mark(kind, [at.x as f32, at.y as f32], px, c, [v[0] * lw * k * s, v[1] * lw * k * s], turn)))
        })
        .collect();
    // far first
    seen.sort_by(|a, b| b.0.total_cmp(&a.0));
    let sprites: Vec<Sprite> = seen.into_iter().map(|(_, s)| s).collect();
    drawn(b, &sprites, ctx.choice("transferMode"), &texture(ctx, "texture"))
}

static SYSTEMS: Stepped<Swarm> = Stepped::new();

effect!(
    SYSTEMS_DEF,
    "particleSystems",
    "Particle Systems",
    "particles on the layer, born from a producer and flying by one of eleven motions",
    [
        ranged("birthRate", "Birth Rate", 2.0, 0.0, 100.0),
        ranged("longevity", "Longevity (sec)", 1.0, 0.0, 100.0),
        point("position", "Producer Position"),
        ranged("radiusX", "Producer Radius X", 20.0, 0.0, 10_000.0),
        ranged("radiusY", "Producer Radius Y", 20.0, 0.0, 10_000.0),
        choice("animation", "Animation", MOTIONS, 0),
        ranged("velocity", "Velocity", 2.0, -100.0, 100.0),
        ranged("gravity", "Gravity", 1.0, -100.0, 100.0),
        ranged("resistance", "Resistance", 0.0, 0.0, 100.0),
        angle("direction", "Direction", 0.0).doc("for the axis and cone motions: 0 up, clockwise"),
        ranged("extra", "Extra", 1.0, -10.0, 10.0),
        choice("particleType", "Particle Type", KINDS, 0),
        layer("texture", "Texture Layer"),
        ranged("rotationSpeed", "Rotation Speed", 0.0, -3600.0, 3600.0),
        ranged("birthSize", "Birth Size", 0.25, 0.0, 100.0),
        ranged("deathSize", "Death Size", 0.25, 0.0, 100.0),
        pct("sizeVariation", "Size Variation", 0.0, 0.0, 100.0),
        pct("maxOpacity", "Max Opacity", 75.0, 0.0, 100.0),
        choice("opacityMap", "Opacity Map", &["fadeInOut", "fadeOut", "fadeIn", "constant"], 0),
        color("birthColor", "Birth Color", 1.0, 1.0, 0.0),
        color("deathColor", "Death Color", 0.6, 0.0, 0.0),
        choice("transferMode", "Transfer Mode", &["composite", "screen", "add", "blackBackground"], 0),
        ranged("randomSeed", "Random Seed", 0.0, 0.0, 10_000.0),
    ]
);
pub const SYSTEMS_SPEC: Spec = Spec::new(&SYSTEMS_DEF, CATEGORY, particle_systems).points(&[("position", [0.5, 0.5])]);

/// The flat systems' physics, positions in layer heights.
fn flat_physics(ctx: &Ctx, t: f64, rate: f64, speed: f64, seed: u64, most: usize) -> Physics {
    let unit = ctx.bounds.height().max(1.0) as f32;
    let f = |id: &str, d: f64| at(ctx, id, t, d) as f32;
    let pos = ctx.value_at("position", Tick::from_seconds(t)).or_else(|| ctx.value_at("producer", Tick::from_seconds(t))).map_or([0.0; 2], |v| v.as_vec2());
    let dir = f("direction", 0.0).to_radians();
    Physics {
        rate: at(ctx, "birthRate", t, 2.0).max(0.0) * rate,
        life: f("longevity", 1.0).max(0.0),
        at: [pos[0] as f32 / unit, pos[1] as f32 / unit, 0.0],
        radius: [f("radiusX", 20.0).max(0.0) / unit, f("radiusY", 20.0).max(0.0) / unit, 0.0],
        motion: ctx.choice("animation").min(MOTIONS.len() as u32 - 1),
        speed: f("velocity", 2.0) * speed as f32,
        gravity: [0.0, f("gravity", 1.0) * 0.5, 0.0],
        resistance: f("resistance", 0.0).max(0.0),
        axis: [dir.sin(), -dir.cos(), 0.0],
        extra: f("extra", 1.0),
        cone: 45.0 * f("extra", 1.0),
        floor: None,
        seed,
        flat: true,
        most,
    }
}

fn particle_systems(ctx: &Ctx, b: Buf) -> Buf {
    let t = ctx.time.seconds();
    let seed = crate::noise::mix(&[ctx.seed, ctx.num("randomSeed", 0.0) as u64, 2]);
    let parts = swarm_at(&SYSTEMS, key_of(ctx, 2), t, |t| flat_physics(ctx, t, 60.0, 0.25, seed, 60_000));
    let unit = ctx.bounds.height().max(1.0);
    let s = b.scale as f32;
    let kind = ctx.choice("particleType");
    let (bc, dc) = (ctx.color("birthColor"), ctx.color("deathColor"));
    let (birth, death) = ([bc.r as f32, bc.g as f32, bc.b as f32], [dc.r as f32, dc.g as f32, dc.b as f32]);
    let sizes = (ctx.num("birthSize", 0.25) as f32, ctx.num("deathSize", 0.25) as f32, (ctx.num("sizeVariation", 0.0) / 100.0) as f32);
    let most = (ctx.num("maxOpacity", 75.0) / 100.0) as f32;
    let fade = ctx.choice("opacityMap");
    let spin = (ctx.num("rotationSpeed", 0.0) as f32).to_radians();
    let sprites: Vec<Sprite> = parts
        .iter()
        .map(|q| {
            let at = b.pixel(veymelo_geom::vec2(q.p[0] as f64 * unit, q.p[1] as f64 * unit));
            let (c, size) = look_of(q, birth, death, sizes, most, fade);
            let r = size * unit as f32 * 0.02 * s;
            mark(kind, [at.x as f32, at.y as f32], r, c, [q.v[0] * unit as f32 * s, q.v[1] * unit as f32 * s], q.rnd * TAU + q.age * spin)
        })
        .collect();
    drawn(b, &sprites, ctx.choice("transferMode"), &texture(ctx, "texture"))
}

static MERCURY: Stepped<Swarm> = Stepped::new();

effect!(
    MERCURY_DEF,
    "mercury",
    "Mercury",
    "the layer as drops of liquid metal flowing from a producer, running together where they meet, lit and bending the picture",
    [
        ranged("radiusX", "Radius X", 60.0, 0.0, 10_000.0),
        ranged("radiusY", "Radius Y", 60.0, 0.0, 10_000.0),
        point("producer", "Producer"),
        angle("direction", "Direction", 0.0),
        ranged("velocity", "Velocity", 1.0, -100.0, 100.0),
        ranged("birthRate", "Birth Rate", 1.0, 0.0, 100.0),
        ranged("longevity", "Longevity (sec)", 2.0, 0.0, 100.0),
        ranged("gravity", "Gravity", 0.5, -100.0, 100.0),
        ranged("resistance", "Resistance", 0.0, 0.0, 100.0),
        ranged("extra", "Extra", 1.0, -10.0, 10.0),
        choice("animation", "Animation", MOTIONS, 0),
        pct("blobInfluence", "Blob Influence", 100.0, 0.0, 400.0).doc("how far drops reach for each other"),
        ranged("blobBirthSize", "Blob Birth Size", 0.5, 0.0, 10.0),
        ranged("blobDeathSize", "Blob Death Size", 0.5, 0.0, 10.0),
        pct("lightIntensity", "Light Intensity", 100.0, 0.0, 400.0),
        color("lightColor", "Light Color", 1.0, 1.0, 1.0),
        pct("lightHeight", "Light Height", 50.0, -100.0, 100.0),
        angle("lightDirection", "Light Direction", -45.0),
        pct("ambient", "Ambient", 20.0, -200.0, 200.0),
        pct("diffuse", "Diffuse", 75.0, 0.0, 200.0),
        pct("specular", "Specular", 100.0, 0.0, 200.0),
        ranged("roughness", "Roughness", 0.05, 0.001, 1.0),
        pct("metal", "Metal", 100.0, 0.0, 100.0).doc("the shine in the metal's own colour"),
    ]
);
pub const MERCURY_SPEC: Spec = Spec::new(&MERCURY_DEF, CATEGORY, mercury).points(&[("producer", [0.5, 0.5])]);

fn mercury(ctx: &Ctx, mut b: Buf) -> Buf {
    let t = ctx.time.seconds();
    let seed = crate::noise::mix(&[ctx.seed, 3]);
    let parts = swarm_at(&MERCURY, key_of(ctx, 3), t, |t| flat_physics(ctx, t, 10.0, 0.3, seed, 4000));
    let unit = ctx.bounds.height().max(1.0) as f32;
    let s = b.scale as f32;
    let (bs, ds) = (ctx.num("blobBirthSize", 0.5) as f32, ctx.num("blobDeathSize", 0.5) as f32);
    let reach = 2.0 * (1.0 + (ctx.num("blobInfluence", 100.0) / 100.0) as f32);
    // each drop in the picture's pixels: where, how far its pull reaches
    let blobs: Vec<[f32; 3]> = parts
        .iter()
        .filter_map(|q| {
            let u = (q.age / q.life.max(1e-4)).clamp(0.0, 1.0);
            let r = (bs + (ds - bs) * u) * unit * 0.04 * s;
            let at = b.pixel(veymelo_geom::vec2(q.p[0] as f64 * unit as f64, q.p[1] as f64 * unit as f64));
            (r > 0.1).then_some([at.x as f32, at.y as f32, r * reach])
        })
        .collect();
    let (w, h) = (b.img.width, b.img.height);
    // the field of every drop's pull, and its slope: f, d/dx, d/dy
    let mut field = vec![[0f32; 3]; w * h];
    for &[cx, cy, rr] in &blobs {
        let (x0, x1) = (((cx - rr).floor().max(0.0)) as usize, ((cx + rr).ceil().max(0.0) as usize).min(w));
        let (y0, y1) = (((cy - rr).floor().max(0.0)) as usize, ((cy + rr).ceil().max(0.0) as usize).min(h));
        for y in y0..y1 {
            for x in x0..x1 {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let d2 = (dx * dx + dy * dy) / (rr * rr);
                if d2 < 1.0 {
                    let k = 1.0 - d2;
                    let g = -6.0 * k * k / (rr * rr);
                    let f = &mut field[y * w + x];
                    *f = [f[0] + k * k * k, f[1] + g * dx, f[2] + g * dy];
                }
            }
        }
    }
    let f = |id: &str, d: f64| (ctx.num(id, d) / 100.0) as f32;
    let (ambient, diffuse, specular, metal, li) = (f("ambient", 20.0), f("diffuse", 75.0), f("specular", 100.0), f("metal", 100.0), f("lightIntensity", 100.0));
    let rough = ctx.num("roughness", 0.05).max(0.001) as f32;
    let lc = ctx.color("lightColor");
    let lc = [lc.r as f32, lc.g as f32, lc.b as f32];
    let dir = (ctx.num("lightDirection", -45.0) as f32).to_radians();
    let light = unit3([dir.sin(), -dir.cos(), f("lightHeight", 50.0).max(0.05)]);
    let half = unit3([light[0], light[1], light[2] + 1.0]);
    let src = b.img.clone();
    let threshold = 0.35f32;
    map_px(&mut b.img, |x, y, _| {
        let [fv, gx, gy] = field[y * w + x];
        // the surface where the field passes the threshold, its edge smoothed over a pixel-ish
        let cover = ((fv - threshold * 0.8) / (threshold * 0.4)).clamp(0.0, 1.0);
        if cover <= 0.0 {
            return [0.0; 4];
        }
        let n = unit3([gx * 3.0, gy * 3.0, 1.0 / (1.0 + fv)]);
        // the picture bent through the drop
        let p = src.sample(Sampling::Bilinear, (x as f32 + 0.5 - n[0] * 12.0 * s) as f64, (y as f32 + 0.5 - n[1] * 12.0 * s) as f64);
        let (c, pa) = unpremul(p);
        let base = if pa > 0.0 { c } else { [0.7, 0.7, 0.75] };
        let lit = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).max(0.0);
        let shine = (n[0] * half[0] + n[1] * half[1] + n[2] * half[2]).max(0.0).powf(1.0 / rough) * specular * li;
        let shade = ambient + diffuse * lit * li;
        let o: Px = [0, 1, 2, 3].map(|k| if k == 3 { cover } else { (base[k] * shade + shine * (lc[k] * (1.0 - metal) + base[k] * metal)) * cover });
        o
    });
    b
}

/// Every effect here.
pub const ALL: [Spec; 3] = [WORLD_SPEC, SYSTEMS_SPEC, MERCURY_SPEC];

#[cfg(test)]
mod tests;

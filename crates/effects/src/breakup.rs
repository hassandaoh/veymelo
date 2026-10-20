//! The layer broken into bits: a grid of lit balls in its colours, turned and scattered in 3D
//! (Ball Action); triangles or squares bursting from a point, spinning and falling (Pixel
//! Polly); every pixel thrown about and the two halves twisted away (Scatterize).

use std::f64::consts::PI;

use veymelo_raster::{Image, Sampling};

use crate::defs::{angle, choice, effect, flag, pct, point, ranged};
use crate::scene3d::{self, Camera, Piece};
use crate::sprites::{Shape, Sprite, draw, rand, rand2};
use crate::util::{luma, unpremul};
use crate::{Buf, Ctx, Spec};

const CATEGORY: &str = "simulation";

effect!(
    BALL_ACTION_DEF,
    "ballAction",
    "Ball Action",
    "the layer as a grid of lit balls in its colours, turned about an axis, twisted and scattered in 3D",
    [
        ranged("scatter", "Scatter", 0.0, 0.0, 2000.0),
        choice("rotationAxis", "Rotation Axis", &["xAxis", "yAxis", "zAxis", "xyAxis", "xzAxis", "yzAxis", "xyzAxis"], 0),
        angle("rotation", "Rotation", 0.0),
        choice("twistProperty", "Twist Property", &["xAxis", "yAxis", "radius", "brightness"], 0).doc("what makes balls turn more: across, down, out from the middle, or brightness"),
        angle("twistAngle", "Twist Angle", 0.0),
        ranged("gridSpacing", "Grid Spacing", 4.0, 1.0, 200.0),
        pct("ballSize", "Ball Size", 40.0, 0.0, 400.0),
        angle("instabilityState", "Instability State", 0.0).doc("turn it to set the scattered balls wandering"),
    ]
);
pub const BALL_ACTION: Spec = Spec::new(&BALL_ACTION_DEF, CATEGORY, ball_action);

fn ball_action(ctx: &Ctx, mut b: Buf) -> Buf {
    let scatter = ctx.num("scatter", 0.0) as f32;
    let axis = match ctx.choice("rotationAxis") {
        0 => [1.0, 0.0, 0.0],
        1 => [0.0, 1.0, 0.0],
        2 => [0.0, 0.0, 1.0],
        3 => [1.0, 1.0, 0.0],
        4 => [1.0, 0.0, 1.0],
        5 => [0.0, 1.0, 1.0],
        _ => [1.0, 1.0, 1.0],
    };
    let (rotation, twist) = (ctx.num("rotation", 0.0).to_radians(), ctx.num("twistAngle", 0.0).to_radians());
    let twist_by = ctx.choice("twistProperty");
    let spacing = ctx.num("gridSpacing", 4.0).max(1.0) as f32;
    let size = (ctx.num("ballSize", 40.0) / 100.0) as f32;
    let wander = ctx.num("instabilityState", 0.0).to_radians() as f32;
    let seed = crate::noise::mix(&[ctx.seed, 41]);
    let r = ctx.bounds;
    let (lw, lh) = (r.width() as f32, r.height() as f32);
    let s = b.scale as f32;
    let (nx, ny) = ((lw / spacing).ceil().max(1.0) as u64, (lh / spacing).ceil().max(1.0) as u64);
    let d = 2.0 * lw.max(lh) as f64;
    let (cx, cy) = (r.center().x, r.center().y);
    let far = (lw.hypot(lh) * 0.5).max(1.0);
    let src = b.img.clone();
    let mut balls: Vec<(f64, Sprite)> = (0..(nx * ny).min(2_000_000))
        .filter_map(|i| {
            let (gx, gy) = (r.x0 as f32 + (i % nx) as f32 * spacing + spacing * 0.5, r.y0 as f32 + (i / nx) as f32 * spacing + spacing * 0.5);
            let at = b.pixel(veymelo_geom::vec2(gx as f64, gy as f64));
            let (c, a) = unpremul(src.sample(Sampling::Bilinear, at.x, at.y));
            if a <= 0.0 || size <= 0.0 {
                return None;
            }
            let how_far = match twist_by {
                0 => (gx - r.x0 as f32) / lw.max(1.0),
                1 => (gy - r.y0 as f32) / lh.max(1.0),
                2 => (gx - cx as f32).hypot(gy - cy as f32) / far,
                _ => luma(c),
            } as f64;
            // scattered: thrown out a way of its own, the way wandering with the instability
            let way = rand(i, 1, seed) * std::f32::consts::TAU + wander;
            let out = scatter * rand(i, 2, seed);
            let p0 = [(gx - cx as f32 + out * way.cos()) as f64, (gy - cy as f32 + out * way.sin()) as f64, (scatter * rand2(i, 3, seed)) as f64];
            let q = scene3d::apply(&scene3d::about(axis, rotation + twist * how_far), p0);
            if d + q[2] <= d * 0.05 {
                return None;
            }
            let near = d / (d + q[2]);
            let p = b.pixel(veymelo_geom::vec2(cx + q[0] * near, cy + q[1] * near));
            Some((q[2], Sprite::new(p.x as f32, p.y as f32, spacing * 0.5 * size * near as f32 * s, [c[0], c[1], c[2], a], Shape::Ball)))
        })
        .collect();
    balls.sort_by(|a, b| b.0.total_cmp(&a.0));
    let sprites: Vec<Sprite> = balls.into_iter().map(|(_, s)| s).collect();
    let mut out = Image::new(b.img.width, b.img.height);
    draw(&mut out, &sprites, false);
    b.img = out;
    b
}

effect!(
    PIXEL_POLLY_DEF,
    "pixelPolly",
    "Pixel Polly",
    "the layer cut into triangles or squares that burst away from a point, spinning and falling",
    [
        ranged("force", "Force", 100.0, -1000.0, 1000.0),
        ranged("gravity", "Gravity", 1.0, -20.0, 20.0),
        angle("spinning", "Spinning", 0.0),
        point("forceCenter", "Force Center"),
        pct("directionRandomness", "Direction Randomness", 10.0, 0.0, 100.0),
        pct("speedRandomness", "Speed Randomness", 10.0, 0.0, 100.0),
        ranged("gridSpacing", "Grid Spacing", 10.0, 1.0, 500.0),
        choice("object", "Object", &["polygon", "texturedPolygon", "square", "texturedSquare"], 0).doc("plain pieces take the colour at their middle"),
        flag("enableDepthSort", "Enable Depth Sort", true),
        ranged("startTime", "Start Time (sec)", 0.0, -1000.0, 1000.0),
    ]
);
pub const PIXEL_POLLY: Spec = Spec::new(&PIXEL_POLLY_DEF, CATEGORY, pixel_polly).points(&[("forceCenter", [0.5, 0.5])]);

fn pixel_polly(ctx: &Ctx, mut b: Buf) -> Buf {
    let t = ctx.time.seconds() - ctx.num("startTime", 0.0);
    if t <= 0.0 {
        return b;
    }
    let (force, gravity, spin) = (ctx.num("force", 100.0), ctx.num("gravity", 1.0), ctx.num("spinning", 0.0).to_radians());
    let fc = ctx.point("forceCenter");
    let (way_rand, speed_rand) = (ctx.num("directionRandomness", 10.0) / 100.0, ctx.num("speedRandomness", 10.0) / 100.0);
    let g = ctx.num("gridSpacing", 10.0).max(1.0);
    let object = ctx.choice("object");
    let seed = crate::noise::mix(&[ctx.seed, 43]);
    let r = ctx.bounds;
    let (lw, lh) = (r.width(), r.height());
    let s = b.scale;
    let mid = b.pixel(r.center());
    let cam = Camera::plain([mid.x, mid.y], 2.0 * lw.max(lh) * s);
    let (nx, ny) = ((lw / g).ceil().max(1.0) as u64, (lh / g).ceil().max(1.0) as u64);
    let (triangles, textured) = (object <= 1, object == 1 || object == 3);
    let most = lw.max(lh);
    let per = if triangles { 2 } else { 1 };
    let src = b.img.clone();
    let mut placed: Vec<_> = (0..(nx * ny * per).min(1_000_000))
        .filter_map(|i| {
            let cell = i / per;
            let (x0, y0) = (r.x0 + (cell % nx) as f64 * g, r.y0 + (cell / nx) as f64 * g);
            let poly: Vec<[f64; 2]> = if !triangles {
                vec![[x0, y0], [x0 + g, y0], [x0 + g, y0 + g], [x0, y0 + g]]
            } else if i % 2 == 0 {
                vec![[x0, y0], [x0 + g, y0], [x0, y0 + g]]
            } else {
                vec![[x0 + g, y0], [x0 + g, y0 + g], [x0, y0 + g]]
            };
            let c = [poly.iter().map(|v| v[0]).sum::<f64>() / poly.len() as f64, poly.iter().map(|v| v[1]).sum::<f64>() / poly.len() as f64];
            let (dx, dy) = (c[0] - fc.x, c[1] - fc.y);
            let dist = dx.hypot(dy).max(1e-6);
            // away from the force's middle, the nearer the quicker
            let way = dy.atan2(dx) + way_rand * PI * rand2(i, 1, seed) as f64;
            let speed = force / 100.0 * 0.6 * most / (1.0 + dist / (0.5 * most)) * (1.0 + speed_rand * rand2(i, 2, seed) as f64);
            let toward = -speed * 0.5 * rand(i, 3, seed) as f64;
            let at = [c[0] + way.cos() * speed * t, c[1] + way.sin() * speed * t + 0.25 * gravity * lh * t * t, toward * t];
            let turn = scene3d::about([rand2(i, 4, seed) as f64, rand2(i, 5, seed) as f64, rand2(i, 6, seed) as f64], t * (PI * rand(i, 7, seed) as f64 + spin));
            let outline: Vec<[f32; 2]> = poly.iter().map(|v| {
                let p = b.pixel(veymelo_geom::vec2(v[0], v[1]));
                [p.x as f32, p.y as f32]
            }).collect();
            let (cp, ap) = (b.pixel(veymelo_geom::vec2(c[0], c[1])), b.pixel(veymelo_geom::vec2(at[0], at[1])));
            let rest = [cp.x as f32, cp.y as f32];
            let (flat, alpha) = if textured {
                (None, 1.0)
            } else {
                let (cc, a) = unpremul(src.sample(Sampling::Bilinear, rest[0] as f64, rest[1] as f64));
                if a <= 0.0 {
                    return None;
                }
                (Some(cc), a)
            };
            let mut p = (Piece { outline, rest, turn, at: [ap.x, ap.y, at[2] * s], alpha, flat, mirror: 0 }).place(&cam)?;
            // lit by how squarely it faces the viewer
            let k = 0.35 + 0.65 * turn[2][2].abs() as f32;
            p.tint = [k; 3];
            Some(p)
        })
        .collect();
    if !ctx.flag("enableDepthSort") {
        // in the grid's order (draw sorts far first; same depth keeps the order)
        placed.iter_mut().for_each(|p| p.depth = 0.0);
    }
    let mut out = Image::new(b.img.width, b.img.height);
    scene3d::draw(&mut out, &mut placed, &src, None);
    b.img = out;
    b
}

effect!(
    SCATTERIZE_DEF,
    "scatterize",
    "Scatterize",
    "every pixel of the layer thrown about, the left and right halves twisted away in 3D",
    [
        ranged("scatter", "Scatter", 0.0, 0.0, 1000.0),
        angle("rightTwist", "Right Twist", 0.0),
        angle("leftTwist", "Left Twist", 0.0),
        choice("transferMode", "Transfer Mode", &["composite", "add"], 0),
    ]
);
pub const SCATTERIZE: Spec = Spec::new(&SCATTERIZE_DEF, CATEGORY, scatterize);

fn scatterize(ctx: &Ctx, mut b: Buf) -> Buf {
    let scatter = ctx.num("scatter", 0.0) as f32;
    let (right, left) = (ctx.num("rightTwist", 0.0).to_radians(), ctx.num("leftTwist", 0.0).to_radians());
    if scatter == 0.0 && right == 0.0 && left == 0.0 {
        return b;
    }
    let seed = crate::noise::mix(&[ctx.seed, 47]);
    let s = b.scale;
    let (w, h) = (b.img.width, b.img.height);
    let mid = b.pixel(ctx.bounds.center());
    let half = (ctx.bounds.width() * 0.5 * s).max(1.0);
    let d = 2.0 * ctx.bounds.width().max(ctx.bounds.height()) * s;
    let src = b.img.clone();
    let sprites: Vec<Sprite> = (0..w * h)
        .filter_map(|i| {
            let p = src.px[i];
            if p[3] <= 0.0 {
                return None;
            }
            let (x, y) = ((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
            // twisted about the line across the middle, the more the farther to each side
            let k = ((x - mid.x) / half).abs().min(1.0);
            let a = if x < mid.x { left } else { right } * k;
            let (sa, ca) = a.sin_cos();
            let (yy, zz) = ((y - mid.y) * ca, (y - mid.y) * sa);
            let near = d / (d + zz).max(d * 0.05);
            let (jx, jy) = (rand2(i as u64, 1, seed) * scatter * s as f32, rand2(i as u64, 2, seed) * scatter * s as f32);
            let (c, al) = unpremul(p);
            Some(Sprite::new((mid.x + (x - mid.x) * near) as f32 + jx, (mid.y + yy * near) as f32 + jy, 0.6 * near as f32, [c[0], c[1], c[2], al], Shape::Disc))
        })
        .collect();
    let mut out = Image::new(w, h);
    draw(&mut out, &sprites, ctx.choice("transferMode") == 1);
    b.img = out;
    b
}

/// Every effect here.
pub const ALL: [Spec; 3] = [BALL_ACTION, PIXEL_POLLY, SCATTERIZE];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{instance, n, run, run_when};
    use veymelo_keyframe::Value;
    use veymelo_time::Tick;

    fn picture() -> Image {
        let mut img = Image::new(60, 40);
        for (i, p) in img.px.iter_mut().enumerate() {
            let x = (i % 60) as f32 / 60.0;
            *p = [x, 0.5, 1.0 - x, 1.0];
        }
        img
    }

    fn at_time(spec: &Spec, values: &[(&str, Value)], t: f64) -> Buf {
        let mut g = instance(spec, values);
        spec.start(&mut g, veymelo_geom::Rect::from_size(60.0, 40.0));
        for (path, v) in values {
            g.prop_mut(path).unwrap().set_value(v.clone()).unwrap();
        }
        run_when(spec, &g, picture(), Tick::from_seconds(t), &[], None)
    }

    fn unlike(a: &Image, b: &Image) -> f32 {
        a.px.iter().zip(&b.px).map(|(p, q)| (0..4).map(|k| (p[k] - q[k]).abs()).fold(0f32, f32::max)).fold(0f32, f32::max)
    }

    #[test]
    fn balls() {
        let still = run(&BALL_ACTION, &[("gridSpacing", n(10.0)), ("ballSize", n(80.0))], picture());
        assert!(still.img.px.iter().any(|p| p[3] > 0.9) && still.img.px.iter().any(|p| p[3] == 0.0), "balls with gaps between");
        let turned = run(&BALL_ACTION, &[("gridSpacing", n(10.0)), ("ballSize", n(80.0)), ("rotation", n(60.0))], picture());
        assert!(unlike(&turned.img, &still.img) > 0.2);
        let scattered = run(&BALL_ACTION, &[("scatter", n(30.0))], picture());
        assert_eq!(scattered.img.px, run(&BALL_ACTION, &[("scatter", n(30.0))], picture()).img.px);
        assert!(run(&BALL_ACTION, &[("ballSize", n(0.0))], picture()).img.px.iter().all(|p| p[3] == 0.0));
    }

    #[test]
    fn polygons_burst() {
        assert_eq!(at_time(&PIXEL_POLLY, &[], 0.0).img.px, picture().px, "before the start: the layer");
        let later = at_time(&PIXEL_POLLY, &[("object", Value::Choice(1))], 0.5);
        assert!(unlike(&later.img, &picture()) > 0.3 && later.img.px.iter().any(|p| p[3] == 0.0), "flown apart");
        let soon = at_time(&PIXEL_POLLY, &[("object", Value::Choice(1)), ("force", n(1.0)), ("gravity", n(0.0))], 0.01);
        assert!(soon.img.px[20 * 60 + 30][3] > 0.5, "barely moved at first");
    }

    #[test]
    fn pixels_scatter() {
        assert_eq!(run(&SCATTERIZE, &[], picture()).img.px, picture().px, "nothing asked: the layer");
        let a = run(&SCATTERIZE, &[("scatter", n(5.0))], picture());
        assert!(unlike(&a.img, &picture()) > 0.1);
        let twisted = run(&SCATTERIZE, &[("rightTwist", n(80.0))], picture());
        let right_edge: f32 = (0..40).map(|y| twisted.img.px[y * 60 + 58][3]).sum();
        let left_edge: f32 = (0..40).map(|y| twisted.img.px[y * 60 + 1][3]).sum();
        assert!(right_edge < left_edge * 0.5, "the right side turned away: {right_edge} {left_edge}");
    }

    #[test]
    fn every_one_is_registered() {
        for spec in ALL {
            assert!(crate::find(spec.def.id).is_some_and(|s| s.category == "simulation"), "{}", spec.def.id);
        }
    }
}

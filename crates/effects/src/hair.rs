//! Hair grown from the layer's solid parts: each strand rising from its root, drooping with its
//! weight, made longer or leaning by a map layer, coloured by its root or its own colour and lit
//! along its length as hair is (bright across the light, shining where the light glances).

use veymelo_raster::{Image, Sampling};

use crate::defs::{angle, color, effect, flag, layer, pct, px, ranged};
use crate::sprites::{Shape, Sprite, draw, rand, rand2};
use crate::util::{luma, unpremul};
use crate::{Buf, Ctx, Spec, Take};

effect!(
    HAIR_DEF,
    "hair",
    "Hair",
    "hair grown from the layer's solid parts: strands drooping with their weight, steered by a map, lit as hair",
    [
        px("length", "Length", 30.0, 200.0),
        ranged("thickness", "Thickness", 1.0, 0.05, 20.0),
        ranged("weight", "Weight", 0.2, -10.0, 10.0).doc("how much a strand droops (below 0 it lifts)"),
        flag("constantMass", "Constant Mass", false).doc("on: long and short strands droop alike"),
        ranged("density", "Density", 100.0, 0.0, 1000.0).doc("strands for every thousand square pixels"),
        pct("mapStrength", "Hairfall Map Strength", 0.0, 0.0, 100.0),
        layer("mapLayer", "Hairfall Map Layer").doc("its brightness makes strands longer, its slopes lean them"),
        px("mapSoftness", "Hairfall Map Softness", 0.0, 100.0),
        pct("addNoise", "Add Noise", 0.0, 0.0, 100.0),
        color("hairColor", "Color", 0.35, 0.25, 0.15),
        pct("brightness", "Brightness", 100.0, 0.0, 400.0),
        pct("opacity", "Opacity", 100.0, 0.0, 100.0),
        pct("colorInheritance", "Color Inheritance", 0.0, 0.0, 100.0).doc("how much a strand takes its root's colour"),
        pct("lightIntensity", "Light Intensity", 100.0, 0.0, 400.0),
        angle("lightDirection", "Light Direction", -45.0),
        pct("ambient", "Ambient", 40.0, 0.0, 100.0),
        pct("diffuse", "Diffuse", 60.0, 0.0, 100.0),
        pct("specular", "Specular", 30.0, 0.0, 100.0),
        pct("roughness", "Roughness", 20.0, 1.0, 100.0),
        ranged("randomSeed", "Random Seed", 0.0, 0.0, 10_000.0),
    ]
);
pub const HAIR: Spec = Spec::new(&HAIR_DEF, "simulation", hair);

fn hair(ctx: &Ctx, mut b: Buf) -> Buf {
    let length = ctx.num("length", 30.0).max(0.0) as f32;
    let density = ctx.num("density", 100.0).max(0.0) as f32;
    if length <= 0.0 || density <= 0.0 {
        return b;
    }
    let f = |id: &str, d: f64| ctx.num(id, d) as f32;
    let pc = |id: &str, d: f64| (ctx.num(id, d) / 100.0) as f32;
    let (thick, weight, constant) = (f("thickness", 1.0).max(0.05), f("weight", 0.2), ctx.flag("constantMass"));
    let (strength, softness, noise) = (pc("mapStrength", 0.0), f("mapSoftness", 0.0).max(0.0), pc("addNoise", 0.0));
    let hc = ctx.color("hairColor");
    let own = [hc.r as f32, hc.g as f32, hc.b as f32];
    let (bright, opacity, inherit) = (pc("brightness", 100.0), pc("opacity", 100.0), pc("colorInheritance", 0.0));
    let (li, ambient, diffuse, specular, rough) = (pc("lightIntensity", 100.0), pc("ambient", 40.0), pc("diffuse", 60.0), pc("specular", 30.0), pc("roughness", 20.0).max(0.01));
    let ld = f("lightDirection", -45.0).to_radians();
    let light = [ld.sin(), -ld.cos()];
    let seed = crate::noise::mix(&[ctx.seed, ctx.num("randomSeed", 0.0) as u64, 23]);
    let s = b.scale as f32;
    let (w, h) = (b.img.width, b.img.height);
    // the map, laid over this layer
    let map = (strength > 0.0).then(|| ctx.layer_over("mapLayer", &b, Take::All, true)).flatten();
    let map_at = |x: f32, y: f32| -> f32 {
        let Some(m) = &map else { return 0.5 };
        let r = (softness * s).max(1.0);
        let sample = |dx: f32, dy: f32| {
            let (c, a) = unpremul(m.sample(Sampling::Bilinear, (x + dx) as f64, (y + dy) as f64));
            luma(c) * a
        };
        (sample(0.0, 0.0) * 2.0 + sample(-r, 0.0) + sample(r, 0.0) + sample(0.0, -r) + sample(0.0, r)) / 6.0
    };
    // roots: places over the layer, as many as the density asks, kept where it is solid
    let area = (w * h) as f32 / (s * s);
    let count = (area / 1000.0 * density).min(400_000.0) as u64;
    let src = b.img.clone();
    let mut sprites = Vec::new();
    for i in 0..count {
        let (rx, ry) = (rand(i, 1, seed) * w as f32, rand(i, 2, seed) * h as f32);
        let root = src.sample(Sampling::Nearest, rx as f64, ry as f64);
        if root[3] < 0.5 {
            continue;
        }
        let (rc, _) = unpremul(root);
        // how long, and which way it starts: up, leaning by chance, by noise and by the map's slopes
        let mut len = length * s * (0.8 + 0.4 * rand(i, 3, seed));
        let mut lean = rand2(i, 4, seed) * 0.35 + noise * rand2(i, 5, seed) * 1.2;
        if map.is_some() {
            let m = map_at(rx, ry);
            len *= 1.0 - strength + strength * 2.0 * m;
            let e = (softness * s).max(2.0);
            let (gx, gy) = (map_at(rx + e, ry) - map_at(rx - e, ry), map_at(rx, ry + e) - map_at(rx, ry - e));
            lean += strength * (gx * 4.0 - gy * 0.5);
        }
        if len < 0.5 {
            continue;
        }
        let dir = [lean.sin(), -lean.cos()];
        // the droop: as heavy as long (or the same for all), pulling the tip down
        let droop = weight * if constant { length * s } else { len } * 0.9;
        let ctrl = [rx + dir[0] * len * 0.5, ry + dir[1] * len * 0.5];
        let tip = [rx + dir[0] * len, ry + dir[1] * len + droop];
        let base = [0, 1, 2].map(|k| (own[k] + (rc[k] - own[k]) * inherit) * bright);
        let segments = ((len / 3.0).ceil() as usize).clamp(2, 24);
        let at = |t: f32| {
            let u = 1.0 - t;
            [u * u * rx + 2.0 * u * t * ctrl[0] + t * t * tip[0], u * u * ry + 2.0 * u * t * ctrl[1] + t * t * tip[1]]
        };
        for k in 0..segments {
            let (t0, t1) = (k as f32 / segments as f32, (k + 1) as f32 / segments as f32);
            let (p0, p1) = (at(t0), at(t1));
            let (dx, dy) = (p1[0] - p0[0], p1[1] - p0[1]);
            let l = dx.hypot(dy).max(1e-6);
            let tan = [dx / l, dy / l];
            // hair's light: brightest across the light, a shine where it glances along
            let tl = tan[0] * light[0] + tan[1] * light[1];
            let across = (1.0 - tl * tl).max(0.0).sqrt();
            let half = {
                let hv = [light[0], light[1] - 1.0];
                let m = hv[0].hypot(hv[1]).max(1e-6);
                [hv[0] / m, hv[1] / m]
            };
            let th = tan[0] * half[0] + tan[1] * half[1];
            let shine = (1.0 - th * th).max(0.0).sqrt().powf(1.0 / rough) * specular * li;
            let shade = ambient + diffuse * across * li;
            let c = base.map(|v| v * shade + shine);
            // thinner toward the tip
            let r = thick * s * 0.5 * (1.0 - 0.6 * t0);
            sprites.push(Sprite::new(p1[0], p1[1], r.max(0.2), [c[0], c[1], c[2], opacity], Shape::Line { dx, dy }));
        }
    }
    let mut strands = Image::new(w, h);
    draw(&mut strands, &sprites, false);
    for (d, m) in b.img.px.iter_mut().zip(&strands.px) {
        let k = 1.0 - m[3];
        *d = [m[0] + d[0] * k, m[1] + d[1] * k, m[2] + d[2] * k, m[3] + d[3] * k];
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{n, run};
    use veymelo_keyframe::Value;

    /// A solid disc in the middle of a clear 60 × 60.
    fn disc() -> Image {
        let mut img = Image::new(60, 60);
        for y in 0..60 {
            for x in 0..60 {
                if ((x as f32 - 30.0).powi(2) + (y as f32 - 34.0).powi(2)) < 225.0 {
                    img.px[y * 60 + x] = [0.8, 0.6, 0.4, 1.0];
                }
            }
        }
        img
    }

    #[test]
    fn grows() {
        let out = run(&HAIR, &[("length", n(12.0)), ("density", n(300.0))], disc());
        let above = (0..60).map(|x| out.img.px[16 * 60 + x][3]).fold(0f32, f32::max);
        assert!(above > 0.3, "strands reach above the disc: {above}");
        assert_eq!(run(&HAIR, &[("length", n(12.0)), ("density", n(300.0))], disc()).img.px, out.img.px, "the same each time");
        assert_eq!(run(&HAIR, &[("length", n(0.0))], disc()).img.px, disc().px, "no length: as it was");
        // heavy hair droops: less of it high up
        let heavy = run(&HAIR, &[("length", n(12.0)), ("density", n(300.0)), ("weight", n(1.5))], disc());
        let high = |img: &Image| (0..20 * 60).map(|i| img.px[i][3]).sum::<f32>();
        assert!(high(&heavy.img) < high(&out.img));
        let own = run(&HAIR, &[("length", n(12.0)), ("density", n(300.0)), ("colorInheritance", n(100.0)), ("hairColor", Value::Color(veymelo_color::rgba(0.0, 0.0, 1.0, 1.0)))], disc());
        let blue = run(&HAIR, &[("length", n(12.0)), ("density", n(300.0)), ("hairColor", Value::Color(veymelo_color::rgba(0.0, 0.0, 1.0, 1.0)))], disc());
        assert!(high(&own.img) > 0.0 && own.img.px[14 * 60 + 30][2] < blue.img.px[14 * 60 + 30][2] + 1e-6, "the root's colour, not blue");
    }
}

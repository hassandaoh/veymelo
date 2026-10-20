use super::*;
use crate::testing::{instance, n, run_when};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;
use veymelo_raster::Image;
use veymelo_time::Tick;

fn go_at(spec: &Spec, values: &[(&str, Value)], img: Image, t: f64) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(img.width as f64, img.height as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::from_seconds(t), &[], None)
}

fn go(spec: &Spec, values: &[(&str, Value)], img: Image) -> Buf {
    go_at(spec, values, img, 0.0)
}

fn grey(w: usize, h: usize, v: f32) -> Image {
    Image::filled(w, h, [v, v, v, 1.0])
}

fn spread(b: &Buf) -> (f32, f32) {
    b.img.px.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p[0]), hi.max(p[0])))
}

#[test]
fn value_noise_kinds() {
    for kind in 0..4 {
        let mut sum = 0.0;
        for i in 0..2000 {
            let v = value(i as f32 * 0.173, i as f32 * 0.071 - 3.0, (i % 13) as f32 * 0.37, 9, kind, None);
            assert!((-0.3..=1.3).contains(&v), "kind {kind}: {v}");
            sum += v;
        }
        assert!((sum / 2000.0 - 0.5).abs() < 0.06, "kind {kind} about a half on the whole: {}", sum / 2000.0);
    }
    // block: the same over a whole cell
    assert_eq!(value(2.1, 3.2, 0.0, 1, 0, None), value(2.9, 3.8, 0.0, 1, 0, None));
    // looping evolution: the same a period on
    assert_eq!(value(0.3, 0.7, 0.4, 1, 2, Some(3)), value(0.3, 0.7, 3.4, 1, 2, Some(3)));
    assert_eq!(overflow(1.3, 0), 1.0);
    assert!((overflow(1.3, 2) - 0.7).abs() < 1e-6);
    assert_eq!(overflow(1.3, 3), 1.3);
    assert!(overflow(1.3, 1) < 1.0 && overflow(1.3, 1) > 0.95 && overflow(-0.3, 1) > 0.0 && overflow(0.5, 1) == 0.5, "softly short of white");
}

#[test]
fn fractal_noise_draws_clouds() {
    let img = grey(64, 64, 0.0);
    let out = go(&FRACTAL, &[], img.clone());
    let (lo, hi) = spread(&out);
    assert!(hi - lo > 0.15 && lo >= 0.0 && hi <= 1.0, "{lo} {hi}");
    assert!(out.img.px.iter().all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 1.0), "grey, opaque");
    assert_eq!(go(&FRACTAL, &[], img.clone()).img.px, out.img.px, "the same every time");
    for (k, v) in [("evolution", n(90.0)), ("randomSeed", n(4.0)), ("rotation", n(30.0)), ("scale", n(40.0)), ("complexity", n(2.0)), ("subRotation", n(45.0))] {
        assert_ne!(go(&FRACTAL, &[(k, v.clone())], img.clone()).img.px, out.img.px, "{k}");
    }
    for kind in 1..FRACTAL_TYPES.len() as u32 {
        assert_ne!(go(&FRACTAL, &[("fractalType", Value::Choice(kind))], img.clone()).img.px, out.img.px, "{}", FRACTAL_TYPES[kind as usize]);
    }
    let strong = go(&FRACTAL, &[("contrast", n(400.0)), ("overflow", Value::Choice(0))], img.clone());
    let (slo, shi) = spread(&strong);
    assert!(shi - slo > hi - lo && slo >= 0.0 && shi <= 1.0);
    let inv = go(&FRACTAL, &[("invert", Value::Bool(true))], img.clone());
    assert!(inv.img.px.iter().zip(&out.img.px).all(|(a, b)| (a[0] + b[0] - 1.0).abs() < 1e-5));
    // cycling: the pattern comes back after the cycle's turns
    let cyc = |e: f64| go(&FRACTAL, &[("cycleEvolution", Value::Bool(true)), ("cycle", n(2.0)), ("evolution", n(e))], img.clone()).img.px;
    assert!(cyc(100.0).iter().zip(&cyc(100.0 + 720.0)).all(|(a, b)| (a[0] - b[0]).abs() < 1e-4));
    // moving the offset slides the noise
    let slid = go(&FRACTAL, &[("offsetTurbulence", Value::Vec2([42.0, 32.0]))], img.clone());
    assert!((slid.img.px[10 * 64 + 20][0] - out.img.px[10 * 64 + 10][0]).abs() < 1e-4);
}

#[test]
fn noise_sits_inside_the_layer() {
    let mut img = Image::new(32, 32);
    for p in &mut img.px[..32 * 16] {
        *p = [0.2, 0.4, 0.6, 1.0];
    }
    let out = go(&FRACTAL, &[], img.clone());
    assert!(out.img.px[32 * 20..].iter().all(|p| p[3] == 0.0), "nothing where the layer is not");
    let faint = go(&FRACTAL, &[("opacity", n(0.0))], img.clone());
    assert!(faint.img.px.iter().zip(&img.px).all(|(a, b)| (0..4).all(|k| (a[k] - b[k]).abs() < 1e-6)), "no opacity: the layer as it was");
    let alone = go(&FRACTAL, &[("blendingMode", Value::Choice(0))], img.clone());
    assert!(alone.img.px[..32 * 16].iter().all(|p| p[0] == p[1]));
    let mult = go(&FRACTAL, &[("blendingMode", Value::Choice(3))], img.clone());
    assert!(mult.img.px[..32 * 16].iter().all(|p| p[2] <= 0.6 + 1e-5 && p[0] <= 0.2 + 1e-5), "multiply darkens");
}

#[test]
fn turbulent_noise_swirls_faster() {
    let img = grey(48, 48, 0.0);
    let a = go(&TURBULENT, &[], img.clone());
    let b = go(&TURBULENT, &[("evolution", n(30.0))], img.clone());
    let c = go(&TURBULENT, &[("evolution", n(30.0)), ("turbulenceFactor", n(0.0))], img.clone());
    let diff = |x: &Buf, y: &Buf| x.img.px.iter().zip(&y.img.px).map(|(p, q)| (p[0] - q[0]).abs()).sum::<f32>();
    assert!(diff(&a, &b) > diff(&a, &c), "more turbulence, more change for the same evolution");
}

#[test]
fn curl_noise_carries_the_picture() {
    let mut img = Image::new(64, 64);
    for y in 0..64 {
        for x in 0..64 {
            let v = if (x / 8 + y / 8) % 2 == 0 { 1.0 } else { 0.0 };
            img.px[y * 64 + x] = [v, v, v, 1.0];
        }
    }
    let out = go(&CURL, &[], img.clone());
    assert_ne!(out.img.px, img.px);
    assert!(out.img.px.iter().all(|p| p[3] > 0.99), "nothing torn open");
    assert_eq!(go(&CURL, &[("displacementAmount", n(0.0))], img.clone()).img.px, img.px);
    let field = go(&CURL, &[("view", Value::Choice(1))], img.clone());
    assert!(field.img.px.iter().all(|p| p[2] == 0.5));
}

#[test]
fn specks() {
    let img = grey(32, 32, 0.5);
    assert_eq!(go(&NOISE, &[], img.clone()).img.px, img.px);
    let out = go(&NOISE, &[("amountOfNoise", n(50.0))], img.clone());
    assert!(out.img.px.iter().any(|p| p[0] != p[1]), "coloured");
    assert!(out.img.px.iter().all(|p| (p[0] - 0.5).abs() <= 0.25 + 1e-6));
    let mono = go(&NOISE, &[("amountOfNoise", n(50.0)), ("useColorNoise", Value::Bool(false))], img.clone());
    assert!(mono.img.px.iter().all(|p| p[0] == p[1] && p[1] == p[2]));
    assert_ne!(go_at(&NOISE, &[("amountOfNoise", n(50.0))], img.clone(), 1.0 / 30.0).img.px, out.img.px, "new every frame");
    let a = go(&NOISE_ALPHA, &[("amount", n(80.0)), ("originalAlpha", Value::Choice(2))], img.clone());
    assert!(a.img.px.iter().any(|p| p[3] < 0.9) && a.img.px.iter().all(|p| p[3] <= 1.0));
    let still = go_at(&NOISE_ALPHA, &[("amount", n(80.0)), ("originalAlpha", Value::Choice(2))], img.clone(), 0.5);
    assert_eq!(still.img.px, a.img.px, "random kinds stay still");
    let moving = go_at(&NOISE_ALPHA, &[("amount", n(80.0)), ("originalAlpha", Value::Choice(2)), ("noise", Value::Choice(2))], img.clone(), 0.5);
    assert_ne!(moving.img.px, a.img.px);
    let colour = Image::filled(32, 32, [0.8, 0.2, 0.2, 1.0]);
    assert_eq!(go(&NOISE_HLS, &[], colour.clone()).img.px, colour.px);
    let h = go(&NOISE_HLS, &[("hue", n(50.0))], colour.clone());
    assert!(h.img.px.iter().any(|p| p[1] > 0.25 || p[2] > 0.25), "hues moved");
    let auto0 = go_at(&NOISE_HLS_AUTO, &[("lightness", n(30.0))], colour.clone(), 0.0);
    let auto1 = go_at(&NOISE_HLS_AUTO, &[("lightness", n(30.0))], colour.clone(), 0.05);
    assert_ne!(auto0.img.px, auto1.img.px, "it moves by itself");
    let g = go(&NOISE_HLS, &[("lightness", n(30.0)), ("noise", Value::Choice(2)), ("grainSize", n(4.0))], colour.clone());
    assert_ne!(g.img.px, colour.px);
}

#[test]
fn every_one_is_registered() {
    for spec in ALL {
        assert!(crate::find(spec.def.id).is_some_and(|s| s.category == "noise"), "{}", spec.def.id);
    }
}

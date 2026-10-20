use super::*;
use crate::testing::{instance, n, run_when};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;
use veymelo_time::Tick;

fn go(spec: &Spec, values: &[(&str, Value)], img: Image, t: f64) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(img.width as f64, img.height as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::from_seconds(t), &[], None)
}

fn grey(w: usize, h: usize) -> Image {
    Image::filled(w, h, [0.2, 0.2, 0.2, 1.0])
}

/// Grey from dark to light across.
fn ramp(w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let v = 0.1 + 0.8 * x as f32 / w as f32;
            img.px[y * w + x] = [v, v * 0.8, v * 0.6, 1.0];
        }
    }
    img
}

fn moved(a: &Image, b: &Image) -> f32 {
    a.px.iter().zip(&b.px).map(|(p, q)| (0..4).map(|k| (p[k] - q[k]).abs()).sum::<f32>()).sum::<f32>() / a.px.len() as f32
}

/// The middle of what differs from `bg`, down.
fn middle_y(img: &Image, bg: &Image) -> f32 {
    let (mut s, mut k) = (0f32, 0f32);
    for (i, (p, q)) in img.px.iter().zip(&bg.px).enumerate() {
        let d: f32 = (0..4).map(|c| (p[c] - q[c]).abs()).sum();
        s += d * (i / img.width) as f32;
        k += d;
    }
    s / k.max(1e-9)
}

#[test]
fn rain_falls() {
    let v = [("drops", n(400.0)), ("opacity", n(80.0))];
    let a = go(&RAINFALL, &v, grey(60, 60), 0.5);
    assert_eq!(a.img.px, go(&RAINFALL, &v, grey(60, 60), 0.5).img.px, "the same each time");
    assert!(moved(&a.img, &grey(60, 60)) > 0.005, "drops drawn");
    assert!(moved(&a.img, &go(&RAINFALL, &v, grey(60, 60), 0.52).img) > 0.002, "and moving");
    assert_eq!(go(&RAINFALL, &[("drops", n(0.0))], grey(60, 60), 0.5).img.px, grey(60, 60).px, "none: as it was");
    let alone = go(&RAINFALL, &[("drops", n(400.0)), ("compositeWithOriginal", Value::Bool(false))], grey(60, 60), 0.5);
    assert!(alone.img.px.iter().any(|p| p[3] == 0.0), "alone: clear between the drops");
    // the ground: nothing falls below it
    let low = go(&RAINFALL, &[("drops", n(2000.0)), ("groundLevel", n(50.0)), ("compositeWithOriginal", Value::Bool(false))], grey(60, 60), 0.3);
    let below: f32 = low.img.px[40 * 60..].iter().map(|p| p[3]).sum();
    assert!(below < 0.5, "{below}");
}

#[test]
fn snow_drifts() {
    let v = [("flakes", n(300.0)), ("size", n(4.0))];
    let a = go(&SNOWFALL, &v, grey(60, 60), 1.0);
    assert_eq!(a.img.px, go(&SNOWFALL, &v, grey(60, 60), 1.0).img.px);
    assert!(moved(&a.img, &grey(60, 60)) > 0.01);
    assert!(moved(&a.img, &go(&SNOWFALL, &v, grey(60, 60), 1.3).img) > 0.005);
    let lit = go(&SNOWFALL, &[("flakes", n(300.0)), ("size", n(4.0)), ("backgroundInfluence", n(100.0)), ("color", Value::Color(veymelo_color::rgba(0.5, 0.5, 0.5, 1.0)))], ramp(60, 60), 1.0);
    let plain = go(&SNOWFALL, &[("flakes", n(300.0)), ("size", n(4.0)), ("color", Value::Color(veymelo_color::rgba(0.5, 0.5, 0.5, 1.0)))], ramp(60, 60), 1.0);
    assert!(moved(&lit.img, &plain.img) > 0.001, "lit by what is behind");
}

#[test]
fn bubbles_rise() {
    let one = [("bubbleAmount", n(1.0)), ("bubbleSize", n(3.0)), ("wobbleAmplitude", n(0.0))];
    let blank = Image::new(40, 40);
    let y0 = middle_y(&go(&BUBBLES, &one, ramp(40, 40), 0.0).img, &blank);
    let y1 = middle_y(&go(&BUBBLES, &one, ramp(40, 40), 0.1).img, &blank);
    assert!(y1 < y0, "{y0} then {y1}");
    let a = go(&BUBBLES, &[("bubbleAmount", n(60.0))], ramp(40, 40), 0.3);
    assert_eq!(a.img.px, go(&BUBBLES, &[("bubbleAmount", n(60.0))], ramp(40, 40), 0.3).img.px);
    assert!(a.img.px.iter().any(|p| p[3] > 0.5) && a.img.px.iter().any(|p| p[3] == 0.0), "bubbles on clear");
    assert!(go(&BUBBLES, &[("bubbleAmount", n(0.0))], ramp(40, 40), 0.3).img.px.iter().all(|p| p[3] == 0.0));
}

#[test]
fn drizzle_ripples() {
    let before = go(&DRIZZLE, &[], ramp(48, 48), -0.5);
    assert_eq!(before.img.px, ramp(48, 48).px, "no drops yet");
    let a = go(&DRIZZLE, &[], ramp(48, 48), 0.6);
    assert_eq!(a.img.px, go(&DRIZZLE, &[], ramp(48, 48), 0.6).img.px);
    assert!(moved(&a.img, &ramp(48, 48)) > 0.001, "rings bend the picture");
    let lit = go(&DRIZZLE, &[("specular", n(150.0)), ("roughness", n(0.2))], ramp(48, 48), 0.6);
    assert!(moved(&lit.img, &a.img) > 0.0005, "and catch the light");
    assert!(moved(&a.img, &go(&DRIZZLE, &[], ramp(48, 48), 0.7).img) > 0.0005, "spreading");
}

#[test]
fn stars_fly_past() {
    let a = go(&STAR_BURST, &[("gridSpacing", n(4.0))], ramp(40, 40), 0.0);
    assert_eq!(a.img.px, go(&STAR_BURST, &[("gridSpacing", n(4.0))], ramp(40, 40), 0.0).img.px);
    assert!(a.img.px.iter().any(|p| p[3] > 0.5));
    let later = go(&STAR_BURST, &[("gridSpacing", n(4.0))], ramp(40, 40), 0.4);
    assert!(moved(&a.img, &later.img) > 0.01, "moving");
    let half = go(&STAR_BURST, &[("gridSpacing", n(4.0)), ("blendWithOriginal", n(100.0))], ramp(40, 40), 0.4);
    assert!(moved(&half.img, &ramp(40, 40)) < 1e-5, "blended all the way: the layer");
    let clear = Image::new(40, 40);
    assert!(go(&STAR_BURST, &[], clear.clone(), 0.2).img.px.iter().all(|p| p[3] == 0.0), "no stars from nothing");
}

#[test]
fn every_one_is_registered() {
    for spec in ALL {
        assert!(crate::find(spec.def.id).is_some_and(|s| s.category == "simulation"), "{}", spec.def.id);
    }
}

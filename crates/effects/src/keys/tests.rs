use super::*;
use crate::testing::{buf, instance, n, run_when, square};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;
use veymelo_time::Tick;

fn go_with(spec: &Spec, values: &[(&str, Value)], img: Image, masks: &[crate::MaskShape], host: Option<&dyn crate::Host>) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(img.width as f64, img.height as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::ZERO, masks, host)
}

fn go(spec: &Spec, values: &[(&str, Value)], img: Image) -> Buf {
    go_with(spec, values, img, &[], None)
}

/// A blue screen 40 × 30 with a red square on it.
fn blue_plate() -> Image {
    let mut img = Image::filled(40, 30, [0.1, 0.1, 0.9, 1.0]);
    for y in 10..20 {
        for x in 10..20 {
            img.px[y * 40 + x] = [0.8, 0.2, 0.1, 1.0];
        }
    }
    img
}

fn alpha(b: &Buf, x: usize, y: usize) -> f32 {
    b.img.px[y * b.img.width + x][3]
}

#[test]
fn colour_keys() {
    let plate = blue_plate();
    let blue = Value::Color(veymelo_color::rgba(0.1, 0.1, 0.9, 1.0));
    for how in 0..3 {
        let out = go(&LINEAR, &[("keyColor", blue.clone()), ("matchColors", Value::Choice(how))], plate.clone());
        assert!(alpha(&out, 2, 2) < 0.01 && alpha(&out, 15, 15) > 0.99, "match {how}");
    }
    let kept = go(&LINEAR, &[("keyColor", blue.clone()), ("keyOperation", Value::Choice(1))], plate.clone());
    assert!(alpha(&kept, 2, 2) > 0.99 && alpha(&kept, 15, 15) < 0.01, "keep colours turns it round");
    let matte = go(&LINEAR, &[("keyColor", blue.clone()), ("view", Value::Choice(2))], plate.clone());
    assert!(matte.img.px.iter().all(|p| p[3] == 1.0));
    let diff = go(&DIFFERENCE_KEY, &[("keyColor", blue.clone())], plate.clone());
    assert!(alpha(&diff, 2, 2) < 0.05 && alpha(&diff, 15, 15) > 0.95, "{} {}", alpha(&diff, 2, 2), alpha(&diff, 15, 15));
    let quad = go(&DIFFERENCE_KEY, &[("keyColor", blue.clone()), ("view", Value::Choice(8))], plate.clone());
    assert!(quad.img.px[2 * 40 + 2][3] == 1.0 && quad.img.px[28 * 40 + 38] == [0.0; 4], "four views at once");
    let ck = go(&COLOR_KEY, &[("keyColor", blue.clone()), ("colorTolerance", n(20.0))], plate.clone());
    assert!(alpha(&ck, 2, 2) == 0.0 && alpha(&ck, 15, 15) == 1.0);
    let spill = go(&SPILL, &[("colorToSuppress", blue.clone())], Image::filled(4, 4, [0.3, 0.4, 0.8, 1.0]));
    assert!((spill.img.px[0][2] - 0.4).abs() < 1e-6, "blue down to the green");
    let better = go(&SPILL, &[("colorToSuppress", blue), ("colorAccuracy", Value::Choice(1))], Image::filled(4, 4, [0.3, 0.4, 0.8, 1.0]));
    assert!(better.img.px[0][2] < 0.8 && better.img.px[0][0] > 0.3);
}

#[test]
fn ranges_and_levels() {
    let mut ramp = Image::new(32, 4);
    for (i, p) in ramp.px.iter_mut().enumerate() {
        let v = (i % 32) as f32 / 31.0;
        *p = [v, v, v, 1.0];
    }
    let ex = go(&EXTRACT, &[("blackPoint", n(100.0)), ("whitePoint", n(200.0))], ramp.clone());
    assert_eq!(alpha(&ex, 2, 0), 0.0);
    assert_eq!(alpha(&ex, 20, 0), 1.0);
    assert_eq!(alpha(&ex, 30, 0), 0.0);
    let soft = go(&EXTRACT, &[("blackPoint", n(100.0)), ("blackSoftness", n(100.0))], ramp.clone());
    assert!(alpha(&soft, 8, 0) > 0.0 && alpha(&soft, 8, 0) < 1.0);
    let lk = go(&LUMA_KEY, &[("threshold", n(128.0))], ramp.clone());
    assert_eq!(alpha(&lk, 2, 0), 0.0, "darker keyed out");
    assert_eq!(alpha(&lk, 30, 0), 1.0);
    // the default range (Lab: a up to 100, b from 128 up) keys out yellow-greens
    let cr = go(&COLOR_RANGE, &[], Image::filled(4, 4, [0.3, 0.9, 0.1, 1.0]));
    assert_eq!(cr.img.px[0][3], 0.0);
    let kept = go(&COLOR_RANGE, &[], Image::filled(4, 4, [0.1, 0.1, 0.9, 1.0]));
    assert_eq!(kept.img.px[0][3], 1.0);
    let un = go(&UNMULT, &[], Image::filled(4, 4, [0.5, 0.25, 0.0, 1.0]));
    assert!((un.img.px[0][3] - 0.5).abs() < 1e-6 && (un.img.px[0][0] / un.img.px[0][3] - 1.0).abs() < 1e-5, "light on black: alpha the brightest channel, colour full");
}

/// An empty plate the same as the blue screen without the square.
struct Plate;

impl crate::Host for Plate {
    fn layer(&self, _: u64, _: Take) -> Option<(Buf, Rect)> {
        Some((buf(Image::filled(40, 30, [0.1, 0.1, 0.9, 1.0])), Rect::from_size(40.0, 30.0)))
    }
}

#[test]
fn difference_from_a_plate() {
    let out = go_with(&DIFFERENCE_MATTE, &[("differenceLayer", Value::Layer(Some(3)))], blue_plate(), &[], Some(&Plate));
    assert_eq!(alpha(&out, 2, 2), 0.0);
    assert_eq!(alpha(&out, 15, 15), 1.0);
    assert_eq!(go(&DIFFERENCE_MATTE, &[], blue_plate()).img.px.iter().filter(|p| p[3] > 0.0).count(), 0, "against itself: nothing differs");
}

#[test]
fn inner_outer() {
    let plate = blue_plate();
    assert_eq!(go(&INNER_OUTER, &[], plate.clone()).img.px, plate.px, "no masks, no change");
    let masks = [square(12.0, 12.0, 6.0), square(6.0, 6.0, 18.0)];
    let out = go_with(&INNER_OUTER, &[("foregroundInside", n(1.0)), ("backgroundOutside", n(2.0))], plate.clone(), &masks, None);
    assert!(alpha(&out, 15, 15) > 0.99, "inside solid");
    assert!(alpha(&out, 2, 2) < 0.01, "outside clear");
    assert!(alpha(&out, 8, 15) < 0.2 && alpha(&out, 11, 15) > 0.8, "between, by colour: {} {}", alpha(&out, 8, 15), alpha(&out, 11, 15));
    let cleaned = go_with(&INNER_OUTER, &[("foregroundInside", n(1.0)), ("backgroundOutside", n(2.0)), ("cleanupForeground1", n(3.0)), ("cleanupForeground1BrushPressure", n(100.0))], plate.clone(), &[masks[0].clone(), masks[1].clone(), square(7.0, 7.0, 2.0)], None);
    assert!(alpha(&cleaned, 7, 8) > alpha(&out, 7, 8) + 0.3, "the clean-up brush paints it solid");
}

#[test]
fn spill_cleaning_and_wires() {
    let green = Value::Color(veymelo_color::rgba(0.0, 1.0, 0.0, 1.0));
    let img = Image::filled(4, 4, [0.4, 0.8, 0.3, 1.0]);
    let out = go(&ADVANCED_SPILL, &[], img.clone());
    assert!(out.img.px[0][1] < 0.8 && out.img.px[0][0] == 0.4, "green taken down: {:?}", out.img.px[0]);
    let ultra = go(&ADVANCED_SPILL, &[("method", Value::Choice(1)), ("keyColor", green), ("lumaCorrection", n(100.0))], img.clone());
    assert!(ultra.img.px[0][0] > 0.4, "brightness given back");
    let mut edge = Image::filled(10, 10, [1.0, 0.0, 0.0, 1.0]);
    edge.px[55] = [0.0, 0.5 * 0.5, 0.0, 0.5];
    let clean = go(&KEY_CLEANER, &[], edge.clone());
    let p = clean.img.px[55];
    assert!(p[0] / p[3] > 0.3, "the edge takes the colour round it: {p:?}");
    // a dark wire across a light picture painted out
    let mut wire = Image::filled(30, 30, [0.8, 0.8, 0.8, 1.0]);
    for i in 0..30 {
        for d in 0..2 {
            let x = (i + d).min(29);
            wire.px[i * 30 + x] = [0.0, 0.0, 0.0, 1.0];
        }
    }
    let gone = go(&WIRE, &[("pointA", Value::Vec2([0.0, 0.0])), ("pointB", Value::Vec2([30.0, 30.0])), ("thickness", n(6.0))], wire.clone());
    assert!(gone.img.px[15 * 30 + 15][0] > 0.6, "{:?}", gone.img.px[15 * 30 + 15]);
    for style in [0, 3] {
        let g = go(&WIRE, &[("pointA", Value::Vec2([0.0, 0.0])), ("pointB", Value::Vec2([30.0, 30.0])), ("thickness", n(6.0)), ("removalStyle", Value::Choice(style))], wire.clone());
        assert!(g.img.px[15 * 30 + 15][0] > 0.4, "style {style}");
    }
}

#[test]
fn every_one_is_registered() {
    for spec in ALL.iter().chain(&crate::screen::ALL) {
        assert!(crate::find(spec.def.id).is_some(), "{}", spec.def.id);
    }
}

use super::*;
use crate::testing::{n, run};

fn px(spec: &Spec, values: &[(&str, Value)], c: [f32; 4]) -> [f32; 4] {
    run(spec, values, Image::filled(2, 2, c)).img.px[0]
}

fn grey(v: f32) -> [f32; 4] {
    [v, v, v, 1.0]
}

fn near(a: [f32; 4], b: [f32; 4], tol: f32) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

fn text(s: &str) -> Value {
    Value::Text(s.into())
}

/// A ramp across: dark red to light green.
fn ramp() -> Image {
    let mut img = Image::new(16, 4);
    for y in 0..4 {
        for x in 0..16 {
            let v = 0.2 + 0.5 * x as f32 / 15.0;
            img.px[y * 16 + x] = [v, 1.0 - v, 0.3, 1.0];
        }
    }
    img
}

#[test]
fn as_they_start_they_change_nothing() {
    let c = [0.3, 0.5, 0.7, 1.0];
    for spec in [&CURVES, &LEVELS_INDIVIDUAL, &SELECTIVE_COLOR, &COLOR_BALANCE_HLS, &COLOR_OFFSET, &KERNEL] {
        assert!(near(px(spec, &[], c), c, 1e-5), "{}", spec.def.id);
    }
}

#[test]
fn curves() {
    let lift = [("rgb", text("0,0 0.5,0.75 1,1"))];
    assert!((px(&CURVES, &lift, grey(0.5))[0] - 0.75).abs() < 1e-4);
    assert!(px(&CURVES, &lift, grey(0.25))[0] > 0.25 && px(&CURVES, &lift, grey(1.0))[0] <= 1.0001);
    let red = px(&CURVES, &[("red", text("0,0 1,0.5"))], grey(1.0));
    assert!(near(red, [0.5, 1.0, 1.0, 1.0], 1e-4));
    let c = Curve::parse("0,0 0.3,0.6 0.6,0.4 1,1").unwrap();
    // never overshoots between its points
    for i in 0..=100 {
        let x = i as f32 / 100.0;
        let y = c.at(x);
        assert!((0.0..=1.0).contains(&y), "{x}: {y}");
    }
    assert!(Curve::parse("0,0 1,1").is_none(), "the identity");
}

#[test]
fn levels_with_every_control() {
    assert!(near(px(&LEVELS_INDIVIDUAL, &[("inputWhite", n(0.5))], grey(0.25)), grey(0.5), 1e-6));
    assert!(near(px(&LEVELS_INDIVIDUAL, &[("greenOutputWhite", n(0.5))], grey(1.0)), [1.0, 0.5, 1.0, 1.0], 1e-6));
}

#[test]
fn automatic_corrections_stretch_the_tones() {
    let stretched = run(&AUTO_CONTRAST, &[("blackClip", n(0.0)), ("whiteClip", n(0.0))], ramp());
    // all channels together: the darkest of them (red, left) becomes black, the lightest (green, left) white
    let (lo, hi) = (stretched.img.px[0], stretched.img.px[15]);
    assert!(lo[0] < 0.01 && lo[1] > 0.99, "{lo:?}");
    assert!((hi[0] - 0.8333).abs() < 0.01, "the same stretch for every channel: {hi:?}");
    let levels = run(&AUTO_LEVELS, &[("blackClip", n(0.0)), ("whiteClip", n(0.0))], ramp());
    assert!(levels.img.px[0][0] < 0.05 && levels.img.px[15][1] < 0.05, "each channel its own way");
    let even = run(&EQUALIZE, &[], ramp());
    assert!(even.img.px[15][0] > 0.95);
    let kept = run(&AUTO_LEVELS, &[("blendWithOriginal", n(100.0))], ramp());
    assert!(kept.img.px.iter().zip(&ramp().px).all(|(a, b)| near(*a, *b, 1e-6)), "fully blended back, it is as it was");
}

#[test]
fn shadows_are_brought_out() {
    let mut img = Image::filled(40, 20, [0.1, 0.1, 0.1, 1.0]);
    for y in 0..20 {
        for x in 20..40 {
            img.px[y * 40 + x] = [0.9, 0.9, 0.9, 1.0];
        }
    }
    let out = run(&SHADOW_HIGHLIGHT, &[("autoAmounts", Value::Bool(false)), ("shadowAmount", n(80.0)), ("blackClip", n(0.0)), ("whiteClip", n(0.0))], img);
    assert!(out.img.px[10 * 40 + 2][0] > 0.15, "the dark side is lighter: {:?}", out.img.px[10 * 40 + 2]);
    assert!((out.img.px[10 * 40 + 38][0] - 0.9).abs() < 0.05, "the light side much as it was");
}

#[test]
fn selective_inks() {
    let less_red = px(&SELECTIVE_COLOR, &[("redsCyan", n(100.0)), ("method", Value::Choice(1))], [1.0, 0.0, 0.0, 1.0]);
    assert!(less_red[0] < 0.1, "more cyan in the reds is less red: {less_red:?}");
    assert!(near(px(&SELECTIVE_COLOR, &[("redsCyan", n(100.0))], [0.0, 0.0, 1.0, 1.0]), [0.0, 0.0, 1.0, 1.0], 1e-6), "blue is not a red");
    let darker = px(&SELECTIVE_COLOR, &[("neutralsBlack", n(50.0)), ("method", Value::Choice(1))], grey(0.5));
    assert!(darker[0] < 0.5);
}

#[test]
fn hls_link_and_broadcast() {
    assert!(near(px(&COLOR_BALANCE_HLS, &[("hue", n(120.0))], [1.0, 0.0, 0.0, 1.0]), [0.0, 1.0, 0.0, 1.0], 1e-5));
    // a picture's own average, multiplied over it
    let mut img = Image::new(2, 1);
    img.px = vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]];
    let linked = run(&COLOR_LINK, &[], img);
    assert!(near(linked.img.px[0], [0.5, 0.0, 0.5, 1.0], 1e-5) && near(linked.img.px[1], [0.5, 0.0, 0.5, 1.0], 1e-5));
    let safe = px(&BROADCAST, &[], [1.0, 1.0, 0.0, 1.0]);
    assert!(safe[0] < 1.0, "a hot yellow is brought down: {safe:?}");
    assert!(near(px(&BROADCAST, &[], grey(0.5)), grey(0.5), 1e-6), "a grey is safe");
}

#[test]
fn toner_offset_kernel() {
    assert!(near(px(&TONER, &[], grey(0.5)), [0.55, 0.43, 0.28, 1.0], 1e-5), "middle grey takes the midtones");
    let turned = px(&COLOR_OFFSET, &[("redPhase", n(180.0))], [0.25, 0.0, 0.0, 1.0]);
    assert!((turned[0] - 0.75).abs() < 1e-6);
    let wrapped = px(&COLOR_OFFSET, &[("redPhase", n(180.0))], [0.75, 0.0, 0.0, 1.0]);
    assert!((wrapped[0] - 0.25).abs() < 1e-5);
    let doubled = px(&KERNEL, &[("center", n(2.0))], grey(0.25));
    assert!(near(doubled, grey(0.5), 1e-6));
}

#[test]
fn change_limit_neutralize_map() {
    let shifted = px(&CHANGE_COLOR, &[("hueTransform", n(120.0)), ("colorToChange", Value::Color(veymelo_color::Rgba { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }))], [1.0, 0.0, 0.0, 1.0]);
    assert!(near(shifted, [0.0, 1.0, 0.0, 1.0], 1e-5), "{shifted:?}");
    assert!(near(px(&CHANGE_COLOR, &[("hueTransform", n(120.0))], [0.0, 0.0, 1.0, 1.0]), [0.0, 0.0, 1.0, 1.0], 1e-6), "blue is not near the colour");
    let limited = px(&VIDEO_LIMITER, &[("clipLevel", Value::Choice(0))], grey(1.0));
    assert!(limited[0] <= 0.9 + 1e-6 && limited[0] > 0.85, "{limited:?}");
    assert!(near(px(&VIDEO_LIMITER, &[], grey(0.5)), grey(0.5), 1e-6));
    let cast = [("midtonesUnbalance", Value::Color(veymelo_color::Rgba { r: 0.6, g: 0.5, b: 0.4, a: 1.0 }))];
    let neutral = px(&NEUTRALIZER, &cast, [0.6, 0.5, 0.4, 1.0]);
    assert!((neutral[0] - neutral[2]).abs() < 0.03, "the cast is gone: {neutral:?}");
    let inverted: String = (0..256).map(|i| (255 - i).to_string()).collect::<Vec<_>>().join(",");
    assert!(near(px(&ARBITRARY_MAP, &[("map", text(&inverted))], grey(0.25)), grey(0.75), 1e-3));
}

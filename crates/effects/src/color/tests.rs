use super::*;
use crate::testing::{n, run};
use veymelo_color::Rgba;
use veymelo_raster::Image;

/// One pixel of colour `c` through `spec` with `values`.
fn px(spec: &Spec, values: &[(&str, Value)], c: [f32; 4]) -> [f32; 4] {
    run(spec, values, Image::filled(2, 2, c)).img.px[0]
}

fn grey(v: f32) -> [f32; 4] {
    [v, v, v, 1.0]
}

fn near(a: [f32; 4], b: [f32; 4], tol: f32) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

fn colour(r: f64, g: f64, b: f64) -> Value {
    Value::Color(Rgba { r, g, b, a: 1.0 })
}

#[test]
fn as_they_start_they_change_nothing() {
    let c = [0.3, 0.5, 0.7, 1.0];
    // (Tint as it starts makes the picture grey, as it should)
    for spec in [&BRIGHTNESS_CONTRAST, &HUE_SATURATION, &LEVELS, &EXPOSURE, &LEAVE_COLOR, &VIBRANCE, &COLOR_BALANCE, &CHANNEL_MIXER, &GAMMA_PEDESTAL_GAIN] {
        let out = px(spec, &[], c);
        assert!(near(out, c, 1e-5), "{}: {out:?}", spec.def.id);
    }
    // see-through pixels keep their alpha
    let half = [0.15, 0.25, 0.35, 0.5];
    assert!((px(&HUE_SATURATION, &[("masterHue", n(120.0))], half)[3] - 0.5).abs() < 1e-6);
}

#[test]
fn tints() {
    let red_white = [("mapBlackTo", colour(1.0, 0.0, 0.0)), ("mapWhiteTo", colour(1.0, 1.0, 1.0))];
    assert!(near(px(&TINT, &red_white, grey(0.0)), [1.0, 0.0, 0.0, 1.0], 1e-5));
    assert!(near(px(&TINT, &red_white, grey(1.0)), grey(1.0), 1e-5));
    assert!(near(px(&TRITONE, &[], grey(0.5)), [0.5, 0.4, 0.3, 1.0], 1e-5), "middle grey becomes the midtone colour");
    let filtered = px(&PHOTO_FILTER, &[("density", n(100.0)), ("preserveLuminosity", Value::Bool(false)), ("filter", Value::Choice(15))], grey(0.8));
    assert!(near(filtered, [0.8, 0.0, 0.0, 1.0], 1e-5), "deep red lets only red through: {filtered:?}");
    let kept = px(&PHOTO_FILTER, &[("density", n(100.0)), ("filter", Value::Choice(3))], grey(0.5));
    assert!((luma([kept[0], kept[1], kept[2]]) - 0.5).abs() < 1e-4 && kept[2] > kept[0], "cooler, as light");
}

#[test]
fn brightness_and_contrast() {
    let bc = |values: &[(&str, Value)], v: f32| px(&BRIGHTNESS_CONTRAST, values, grey(v))[0];
    let up = [("brightness", n(50.0)), ("contrast", n(40.0))];
    assert_eq!(bc(&up, 0.0), 0.0);
    assert!((bc(&up, 1.0) - 1.0).abs() < 1e-6, "black and white stay");
    assert!(bc(&up, 0.4) > 0.4);
    // against a compositing tool's ramps (8-bit), within 10
    for (b, c, input, want) in [(0.0, 100.0, 64u8, 40i32), (0.0, 100.0, 192, 217), (0.0, 50.0, 64, 53), (100.0, 0.0, 128, 209), (50.0, 0.0, 128, 171), (106.0, -18.0, 128, 214)] {
        let got = (bc(&[("brightness", n(b)), ("contrast", n(c))], input as f32 / 255.0) * 255.0).round() as i32;
        assert!((got - want).abs() <= 10, "B{b} C{c} {input}: {got} vs {want}");
    }
    let legacy = [("brightness", n(50.0)), ("useLegacy", Value::Bool(true))];
    assert!((bc(&legacy, 0.2) - 0.7).abs() < 1e-5);
}

#[test]
fn hues_turn_and_ranges_pick() {
    let turned = px(&HUE_SATURATION, &[("masterHue", n(120.0))], [1.0, 0.0, 0.0, 1.0]);
    assert!(near(turned, [0.0, 1.0, 0.0, 1.0], 1e-5), "red turned a third is green: {turned:?}");
    let grey_out = px(&HUE_SATURATION, &[("masterSaturation", n(-100.0))], [1.0, 0.0, 0.0, 1.0]);
    assert!(near(grey_out, grey(0.5), 1e-5));
    // only the reds range: red changes, blue does not
    let reds = [("redHue", n(120.0))];
    assert!(px(&HUE_SATURATION, &reds, [1.0, 0.0, 0.0, 1.0])[1] > 0.9);
    assert!(near(px(&HUE_SATURATION, &reds, [0.0, 0.0, 1.0, 1.0]), [0.0, 0.0, 1.0, 1.0], 1e-6));
    let colorized = px(&HUE_SATURATION, &[("colorize", Value::Bool(true)), ("colorizeHue", n(240.0)), ("colorizeSaturation", n(100.0))], grey(0.5));
    assert!(near(colorized, [0.0, 0.0, 1.0, 1.0], 1e-5));
}

#[test]
fn levels() {
    let lv = |values: &[(&str, Value)], c: [f32; 4]| px(&LEVELS, values, c);
    assert!(near(lv(&[("inputWhite", n(0.5))], grey(0.25)), grey(0.5), 1e-6));
    assert!(near(lv(&[("inputWhite", n(0.5))], grey(0.75)), grey(1.0), 1e-6), "clipped at white in an 8-bit project");
    assert!(near(lv(&[("gamma", n(2.0))], grey(0.25)), grey(0.5), 1e-6));
    assert!(near(lv(&[("outputBlack", n(0.2)), ("outputWhite", n(0.8))], grey(0.5)), grey(0.5), 1e-6));
    let red_only = lv(&[("redOutputWhite", n(0.5))], grey(1.0));
    assert!(near(red_only, [0.5, 1.0, 1.0, 1.0], 1e-6));
    let alpha = lv(&[("alphaOutputWhite", n(0.5))], [1.0, 1.0, 1.0, 1.0]);
    assert!((alpha[3] - 0.5).abs() < 1e-6 && (alpha[0] - 0.5).abs() < 1e-6, "premultiplied by the new alpha");
}

#[test]
fn exposure_in_stops() {
    let ex = px(&EXPOSURE, &[("exposure", n(1.0))], grey(0.5));
    let want = veymelo_color::linear_to_srgb(veymelo_color::srgb_to_linear(0.5) * 2.0) as f32;
    assert!((ex[0] - want).abs() < 1e-5, "one stop doubles the light: {} vs {want}", ex[0]);
    let raw = px(&EXPOSURE, &[("exposure", n(1.0)), ("bypassLinearLightConversion", Value::Bool(true))], grey(0.25));
    assert!((raw[0] - 0.5).abs() < 1e-5);
    let red = px(&EXPOSURE, &[("channels", Value::Choice(1)), ("redExposure", n(1.0))], grey(0.25));
    assert!(red[0] > 0.3 && (red[1] - 0.25).abs() < 1e-6);
}

#[test]
fn black_and_white() {
    let bw = |c: [f32; 3]| px(&BLACK_WHITE, &[], [c[0], c[1], c[2], 1.0]);
    for (c, g) in [([1.0, 0.0, 0.0], 0.4), ([1.0, 1.0, 0.0], 0.6), ([0.0, 1.0, 0.0], 0.4), ([0.0, 0.0, 1.0], 0.2), ([1.0, 0.0, 1.0], 0.8)] {
        assert!(near(bw(c), grey(g), 1e-5), "{c:?}: {:?}", bw(c));
    }
    assert!(near(bw([0.3, 0.3, 0.3]), grey(0.3), 1e-6), "a grey keeps its value");
}

#[test]
fn picking_colours() {
    let blue = px(&CHANGE_TO_COLOR, &[], [1.0, 0.0, 0.0, 1.0]);
    assert!(blue[2] > 0.9 && blue[0] < 0.1, "red becomes blue: {blue:?}");
    assert!(near(px(&CHANGE_TO_COLOR, &[], [0.0, 1.0, 0.0, 1.0]), [0.0, 1.0, 0.0, 1.0], 1e-6), "green is left");
    let matte = px(&CHANGE_TO_COLOR, &[("viewCorrectionMatte", Value::Bool(true))], [1.0, 0.0, 0.0, 1.0]);
    assert!(near(matte, grey(1.0), 1e-6));
    let leave = [("amountToDecolor", n(100.0))];
    assert!(near(px(&LEAVE_COLOR, &leave, [1.0, 0.0, 0.0, 1.0]), [1.0, 0.0, 0.0, 1.0], 1e-6), "red is left");
    let gone = px(&LEAVE_COLOR, &leave, [0.0, 0.0, 1.0, 1.0]);
    assert!(near(gone, grey(0.0722), 1e-5), "blue goes grey: {gone:?}");
}

#[test]
fn mixing_and_balancing() {
    let swapped = px(&CHANNEL_MIXER, &[("redRed", n(0.0)), ("redBlue", n(100.0))], [0.1, 0.2, 0.9, 1.0]);
    assert!(near(swapped, [0.9, 0.2, 0.9, 1.0], 1e-6));
    let warm = px(&COLOR_BALANCE, &[("midtoneRedBalance", n(40.0))], grey(0.5));
    assert!(warm[0] > 0.6 && (warm[1] - 0.5).abs() < 1e-6);
    let kept = px(&COLOR_BALANCE, &[("midtoneRedBalance", n(40.0)), ("preserveLuminosity", Value::Bool(true))], grey(0.5));
    assert!((luma([kept[0], kept[1], kept[2]]) - 0.5).abs() < 1e-5);
    let vivid = px(&VIBRANCE, &[("vibrance", n(100.0))], [0.6, 0.5, 0.5, 1.0]);
    assert!(vivid[0] - vivid[1] > 0.1, "a dull colour is saturated: {vivid:?}");
    let gain = px(&GAMMA_PEDESTAL_GAIN, &[("redGain", n(0.5))], grey(1.0));
    assert!(near(gain, [0.5, 1.0, 1.0, 1.0], 1e-6));
}

#[test]
fn colorama_cycles() {
    // by default brightness picks round the hue cycle: black is red, a third of the way is green
    assert!(near(px(&COLORAMA, &[], grey(0.0)), [1.0, 0.0, 0.0, 1.0], 1e-5));
    let third = px(&COLORAMA, &[], grey(1.0 / 3.0));
    assert!(near(third, [0.0, 1.0, 0.0, 1.0], 1e-3), "{third:?}");
    // from red, two colours of one brightness differ
    let by_red = [("getPhaseFrom", Value::Choice(1))];
    assert!(!near(px(&COLORAMA, &by_red, [0.6, 0.2, 0.2, 1.0]), px(&COLORAMA, &by_red, [0.2, 0.2, 0.6, 1.0]), 1e-3));
    // not smooth: near tones land on one colour
    let held = [("interpolatePalette", Value::Bool(false))];
    assert_eq!(px(&COLORAMA, &held, grey(0.20)), px(&COLORAMA, &held, grey(0.22)));
    // a palette of its own
    let mut g = Gradient { colors: vec![(0.0, [0.0, 0.0, 0.0]), (0.5, [1.0, 1.0, 1.0])], opacities: vec![(0.0, 1.0)] };
    g.opacities.push((1.0, 1.0));
    let custom = [("usePresetPalette", Value::Choice(8)), ("outputCycle", Value::Gradient(g))];
    assert!(near(px(&COLORAMA, &custom, grey(0.25)), grey(0.5), 1e-4));
    assert!(near(px(&COLORAMA, &custom, grey(0.75)), grey(0.5), 1e-4), "it goes round back to black");
}

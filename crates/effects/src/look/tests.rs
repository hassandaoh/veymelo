use super::*;
use crate::testing::{n, run};

fn px(spec: &Spec, values: &[(&str, Value)], c: [f32; 4]) -> [f32; 4] {
    run(spec, values, Image::filled(2, 2, c)).img.px[0]
}

fn near(a: [f32; 4], b: [f32; 4], tol: f32) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

fn text(s: &str) -> Value {
    Value::Text(s.into())
}

/// A 2 × 2 × 2 table that turns colours to their opposites.
const INVERT: &str = "TITLE \"invert\"\nLUT_3D_SIZE 2\n1 1 1\n0 1 1\n1 0 1\n0 0 1\n1 1 0\n0 1 0\n1 0 0\n0 0 0\n";

#[test]
fn a_grade_as_it_starts_changes_nothing() {
    let c = [0.3, 0.5, 0.7, 1.0];
    assert!(near(px(&COLOR_GRADE, &[], c), c, 1e-5), "{:?}", px(&COLOR_GRADE, &[], c));
}

#[test]
fn the_basics() {
    let up = px(&COLOR_GRADE, &[("exposure", n(1.0))], [0.25, 0.25, 0.25, 1.0]);
    assert!(near(up, [0.5, 0.5, 0.5, 1.0], 1e-5));
    let grey = px(&COLOR_GRADE, &[("saturation", n(0.0))], [1.0, 0.0, 0.0, 1.0]);
    assert!((grey[0] - grey[1]).abs() < 1e-5);
    let warm = px(&COLOR_GRADE, &[("temperature", n(100.0))], [0.5, 0.5, 0.5, 1.0]);
    assert!(warm[0] > warm[2]);
    let mono = px(&COLOR_GRADE, &[("look", Value::Choice(7))], [1.0, 0.0, 0.0, 1.0]);
    assert!((mono[0] - mono[2]).abs() < 1e-5, "the monochrome look is grey");
    let curved = px(&COLOR_GRADE, &[("rgbCurve", text("0,0 0.5,0.75 1,1"))], [0.5, 0.5, 0.5, 1.0]);
    assert!((curved[0] - 0.75).abs() < 1e-3);
    let turned = px(&COLOR_GRADE, &[("hueVsHue", text("0,0.75 0.5,0.75"))], [1.0, 0.0, 0.0, 1.0]);
    assert!(turned[1] > 0.5 || turned[2] > 0.5, "red's hue was turned: {turned:?}");
}

#[test]
fn a_keyed_secondary_and_a_vignette() {
    let key = [("hueCenter", n(0.0)), ("hueRange", n(20.0)), ("showMask", Value::Bool(true)), ("maskStyle", Value::Choice(2))];
    assert!(near(px(&COLOR_GRADE, &key, [1.0, 0.0, 0.0, 1.0]), [1.0, 1.0, 1.0, 1.0], 1e-5), "red is in the key");
    assert!(near(px(&COLOR_GRADE, &key, [0.0, 0.0, 1.0, 1.0]), [0.0, 0.0, 0.0, 1.0], 1e-5), "blue is not");
    let dark = run(&COLOR_GRADE, &[("vignetteAmount", n(-5.0))], Image::filled(40, 40, [0.8, 0.8, 0.8, 1.0]));
    assert!(dark.img.px[0][0] < 0.5 && (dark.img.px[20 * 40 + 20][0] - 0.8).abs() < 1e-4, "darker in the corners only");
}

#[test]
fn tables() {
    let t = lut::parse(INVERT).unwrap();
    for between in [Between::Nearest, Between::Trilinear, Between::Tetrahedral] {
        let c = t.apply([0.0, 1.0, 0.0], between);
        assert!(c.iter().zip([1.0, 0.0, 1.0]).all(|(a, b)| (a - b).abs() < 1e-6), "{between:?}: {c:?}");
    }
    let mid = t.apply([0.25, 0.5, 0.75], Between::Tetrahedral);
    assert!((mid[0] - 0.75).abs() < 1e-5 && (mid[2] - 0.25).abs() < 1e-5);
    assert!(lut::parse("LUT_3D_SIZE 2\n0 0 0\n").unwrap_err().contains("has 1 rows"));
    let applied = px(&APPLY_LUT, &[("lut", text(INVERT))], [0.2, 0.4, 0.6, 1.0]);
    assert!(near(applied, [0.8, 0.6, 0.4, 1.0], 1e-5));
    let missing = run(&APPLY_LUT, &[("lut", text("/no/such/file.cube"))], Image::filled(1, 1, [0.5; 4]));
    assert_eq!(missing.img.px[0], [0.5; 4], "a file that cannot be read changes nothing");
}

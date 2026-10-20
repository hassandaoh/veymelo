use super::*;
use crate::testing::{instance, n, run_when, square};
use veymelo_keyframe::Value;
use veymelo_time::Tick;

/// A green screen 40 × 30 with a red square on it and a strip of half red, half green.
pub fn plate() -> Image {
    let mut img = Image::new(40, 30);
    for y in 0..30 {
        for x in 0..40 {
            img.px[y * 40 + x] = if (10..20).contains(&x) && (10..20).contains(&y) {
                [0.8, 0.2, 0.1, 1.0]
            } else if (25..30).contains(&x) {
                [0.45, 0.55, 0.1, 1.0]
            } else {
                [0.1, 0.9, 0.1, 1.0]
            };
        }
    }
    img
}

fn go(values: &[(&str, Value)], img: Image, masks: &[crate::MaskShape]) -> Buf {
    let mut g = instance(&SCREEN_KEYER, &[]);
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(&SCREEN_KEYER, &g, img, Tick::ZERO, masks, None)
}

fn screen() -> (&'static str, Value) {
    ("screenColour", Value::Color(veymelo_color::rgba(0.1, 0.9, 0.1, 1.0)))
}

#[test]
fn keys_the_screen_out() {
    let out = go(&[screen()], plate(), &[]);
    let at = |x: usize, y: usize| out.img.px[y * 40 + x];
    assert!(at(2, 2)[3] < 0.02, "the screen gone: {:?}", at(2, 2));
    assert!(at(15, 15)[3] > 0.98, "the subject kept");
    let mix = at(27, 5);
    assert!(mix[3] > 0.2 && mix[3] < 0.8, "the half-and-half strip part see-through: {mix:?}");
    assert!(mix[1] / mix[3] < 0.6, "and no longer green");
    assert_eq!(go(&[screen()], plate(), &[]).img.px, out.img.px, "the same every time");
    assert_eq!(go(&[screen(), ("view", Value::Choice(0))], plate(), &[]).img.px, plate().px, "the source view");
}

#[test]
fn the_matte_is_cleaned() {
    let clipped = go(&[screen(), ("clipWhite", n(40.0))], plate(), &[]);
    assert!(clipped.img.px[5 * 40 + 27][3] > 0.95, "clip white makes the strip solid");
    let m = go(&[screen(), ("view", Value::Choice(4))], plate(), &[]);
    assert!(m.img.px.iter().all(|p| p[3] == 1.0 && p[0] == p[1]), "the screen matte in grey");
    let status = go(&[screen(), ("view", Value::Choice(8))], plate(), &[]);
    assert!(status.img.px.iter().all(|p| p[0] == 0.0 || p[0] == 0.5 || p[0] == 1.0));
    let mut specked = plate();
    specked.px[3 * 40 + 3] = [0.8, 0.2, 0.1, 1.0];
    let raw = go(&[screen()], specked.clone(), &[]);
    let clean = go(&[screen(), ("screenDespotWhite", n(1.0))], specked, &[]);
    assert!(raw.img.px[3 * 40 + 3][3] > 0.9 && clean.img.px[3 * 40 + 3][3] < 0.1, "a speck in the screen cleared");
    let grown = go(&[screen(), ("screenShrinkGrow", n(2.0))], plate(), &[]);
    assert!(grown.img.px[15 * 40 + 8][3] > 0.9, "grown past the square's edge");
}

#[test]
fn masks_inside_and_out() {
    let inside = go(&[screen(), ("insideMask", n(1.0))], plate(), &[square(0.0, 0.0, 6.0)]);
    assert!(inside.img.px[2 * 40 + 2][3] > 0.98, "solid inside the inside mask");
    let outside = go(&[screen(), ("outsideMask", n(1.0))], plate(), &[square(12.0, 12.0, 4.0)]);
    assert!(outside.img.px[14 * 40 + 14][3] < 0.02, "gone inside the outside mask");
    assert!(outside.img.px[11 * 40 + 11][3] > 0.98);
}

#[test]
fn colours_are_corrected() {
    let plain = go(&[screen()], plate(), &[]);
    let warm = go(&[screen(), ("enableColourCorrection", Value::Bool(true)), ("brightness", n(-50.0))], plate(), &[]);
    let (p, q) = (plain.img.px[15 * 40 + 15], warm.img.px[15 * 40 + 15]);
    assert!(q[0] < p[0], "darker: {p:?} {q:?}");
    let s = suppress([0.2, 0.9, 0.1], 2, 0.5, 1.0);
    assert!((s[1] - 0.15).abs() < 1e-6, "green down to the mean of red and blue");
    let y = suppress([0.9, 0.8, 0.1], 6, 0.5, 1.0);
    assert!((y[0] - 0.2).abs() < 1e-6 && (y[1] - 0.1).abs() < 1e-6, "yellow's excess over blue taken off");
    let cropped = go(&[screen(), ("cropLeft", n(25.0))], plate(), &[]);
    assert_eq!(cropped.img.px[15 * 40 + 5], [0.0; 4]);
    assert!(cropped.img.px[15 * 40 + 15][3] > 0.98);
}

use super::*;
use crate::testing::{at, n, run, run_masked, square};
use veymelo_keyframe::Value;

fn clear(w: usize, h: usize) -> Image {
    Image::new(w, h)
}

#[test]
fn fill_inside_masks_or_the_shape() {
    let mask = [square(8.0, 8.0, 16.0)];
    let filled = run_masked(&FILL, &[("fillMask", n(1.0))], Image::filled(32, 32, [0.0, 0.0, 1.0, 1.0]), &mask);
    assert_eq!(at(&filled, 16.0, 16.0), [1.0, 0.0, 0.0, 1.0], "red inside the mask");
    assert_eq!(at(&filled, 2.0, 2.0), [0.0, 0.0, 1.0, 1.0], "blue outside");
    let plain = run(&FILL, &[], Image::filled(4, 4, [0.0, 0.0, 0.5, 0.5]));
    assert_eq!(plain.img.px[0], [0.5, 0.0, 0.0, 0.5], "without a mask the layer's shape, its alpha kept");
}

#[test]
fn stroke_draws_along_a_mask() {
    let mask = [square(8.0, 8.0, 16.0)];
    let out = run_masked(&STROKE, &[("brushSize", n(4.0)), ("paintStyle", Value::Choice(1))], clear(32, 32), &mask);
    assert!(at(&out, 8.0, 16.0)[3] > 0.9, "on the outline");
    assert!(at(&out, 16.0, 16.0)[3] < 0.01, "not inside");
    let half = run_masked(&STROKE, &[("brushSize", n(4.0)), ("end", n(25.0)), ("paintStyle", Value::Choice(1))], clear(32, 32), &mask);
    assert!(at(&half, 16.0, 8.0)[3] > 0.9 && at(&half, 16.0, 24.0)[3] < 0.01, "a quarter of the way round");
}

#[test]
fn scribbles_fill_a_mask() {
    let mask = [square(4.0, 4.0, 40.0)];
    let out = run_masked(&SCRIBBLE, &[("spacing", n(4.0)), ("strokeWidth", n(2.0))], clear(48, 48), &mask);
    let inside = (6..42).flat_map(|y| (6..42).map(move |x| (x, y))).filter(|&(x, y)| out.img.px[y * 48 + x][3] > 0.5).count();
    let outside = (0..48).flat_map(|y| (0..48).map(move |x| (x, y))).filter(|&(x, y)| !(2..46).contains(&x) || !(2..46).contains(&y)).filter(|&(x, y)| out.img.px[y * 48 + x][3] > 0.5).count();
    assert!(inside > 300, "lines across the inside: {inside}");
    assert!(outside < 10, "little outside: {outside}");
    let none = run_masked(&SCRIBBLE, &[], Image::filled(4, 4, [1.0; 4]), &[]);
    assert_eq!(none.img.px[0], [0.0; 4], "without a mask, on transparent, nothing");
}

#[test]
fn vegas_lights_the_edges() {
    let mut img = clear(48, 48);
    for y in 12..36 {
        for x in 12..36 {
            img.px[y * 48 + x] = [1.0; 4];
        }
    }
    let out = run(&VEGAS, &[("channel", Value::Choice(4)), ("blendMode", Value::Choice(0)), ("length", n(1.0)), ("segmentDistribution", Value::Choice(1)), ("width", n(3.0))], img);
    let lit = |x: usize, y: usize| out.img.px[y * 48 + x][3];
    assert!((10..14).any(|x| lit(x, 24) > 0.3), "lit along the edge");
    assert!(lit(24, 24) < 0.01, "not in the middle");
}

#[test]
fn write_on_paints_where_the_brush_is() {
    let out = run(&WRITE_ON, &[("brushPosition", Value::Vec2([10.0, 10.0])), ("brushSize", n(6.0)), ("paintStyle", Value::Choice(1))], clear(32, 32));
    assert!(at(&out, 10.0, 10.0)[3] > 0.9 && at(&out, 25.0, 25.0)[3] < 0.01);
}

#[test]
fn bucket_and_eyedropper() {
    let mut img = Image::filled(20, 20, [1.0, 1.0, 1.0, 1.0]);
    for y in 0..20 {
        img.px[y * 20 + 10] = [0.0, 0.0, 0.0, 1.0];
    }
    let out = run(&PAINT_BUCKET, &[("fillPoint", Value::Vec2([3.0, 3.0]))], img.clone());
    assert_eq!(at(&out, 3.0, 3.0), [1.0, 0.0, 0.0, 1.0], "the left is filled");
    assert_eq!(at(&out, 15.0, 3.0), [1.0, 1.0, 1.0, 1.0], "the black line stops it");
    let spread = run(&PAINT_BUCKET, &[("fillPoint", Value::Vec2([3.0, 3.0])), ("stroke", Value::Choice(2)), ("spreadRadius", n(2.0))], img.clone());
    assert!(at(&spread, 10.5, 3.0)[1] < 0.01, "spread over the line");
    let mut half = Image::filled(10, 10, [0.0, 1.0, 0.0, 1.0]);
    half.px[0] = [1.0, 0.0, 0.0, 1.0];
    let picked = run(&EYEDROPPER, &[("samplePoint", Value::Vec2([0.5, 0.5]))], half);
    assert!(picked.img.px.iter().all(|p| *p == [1.0, 0.0, 0.0, 1.0]), "the colour at the point everywhere");
}

#[test]
fn glue_and_threads() {
    let glue = run(&GLUE_GUN, &[("brushPosition", Value::Vec2([16.0, 16.0])), ("timeSpan", n(0.0)), ("style", Value::Choice(1))], Image::filled(32, 32, [0.5, 0.5, 0.5, 1.0]));
    assert!(at(&glue, 16.0, 16.0)[3] > 0.5 && at(&glue, 1.0, 1.0)[3] == 0.0);
    let woven = run(&THREADS, &[("width", n(8.0)), ("height", n(8.0)), ("center", Value::Vec2([0.0, 0.0]))], Image::filled(32, 32, [1.0, 0.0, 0.0, 1.0]));
    let gaps = woven.img.px.iter().filter(|p| p[3] == 0.0).count();
    assert!(gaps > 0 && gaps < 32 * 32 / 2, "threads with gaps between: {gaps}");
}

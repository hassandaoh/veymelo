use super::*;
use veymelo_keyframe::Value;
use crate::testing::{at, buf, halves, n, run, run_on};
use crate::Host;
use veymelo_geom::{Rect, vec2};

fn dot(w: usize, h: usize, x: usize, y: usize) -> Image {
    let mut img = Image::new(w, h);
    img.px[y * w + x] = [1.0; 4];
    img
}

fn mass(b: &Buf) -> f32 {
    b.img.px.iter().map(|p| p[3]).sum::<f32>() / (b.scale * b.scale) as f32
}

#[test]
fn nothing_to_blur_changes_nothing() {
    let img = halves(16, 8);
    for spec in [&GAUSSIAN, &FAST_BOX, &DIRECTIONAL, &SHARPEN, &CHANNEL, &FLICKER] {
        let out = run(spec, &[], img.clone());
        assert_eq!((out.img.px.clone(), out.origin), (img.px.clone(), vec2(0.0, 0.0)), "{}", spec.def.id);
    }
}

#[test]
fn gaussian_spreads_past_the_edges_or_repeats_them() {
    let out = run(&GAUSSIAN, &[("blurriness", n(8.0))], dot(9, 9, 4, 4));
    assert!(out.img.width > 9, "room was made");
    assert!((mass(&out) - 1.0).abs() < 1e-3, "{}", mass(&out));
    assert!(at(&out, -3.5, 4.5)[3] > 0.0, "spread past the layer's edge");
    let flat = run(&GAUSSIAN, &[("blurriness", n(8.0)), ("repeatEdge", Value::Bool(true))], Image::filled(10, 6, [0.4, 0.3, 0.2, 1.0]));
    assert_eq!(flat.img.width, 10);
    assert!(flat.img.px.iter().all(|p| (p[0] - 0.4).abs() < 1e-5 && (p[3] - 1.0).abs() < 1e-5));
    // across only
    let across = run(&GAUSSIAN, &[("blurriness", n(6.0)), ("dimensions", Value::Choice(1))], dot(21, 21, 10, 10));
    assert!(at(&across, 13.5, 10.5)[3] > 0.0 && at(&across, 10.5, 13.5)[3] == 0.0);
}

#[test]
fn drawn_small_it_blurs_the_same() {
    let full = run(&GAUSSIAN, &[("blurriness", n(10.0))], halves(40, 20));
    let mut small = Image::new(20, 10);
    for y in 0..10 {
        for x in 0..20 {
            let v = if x < 10 { 0.1 } else { 0.9 };
            small.px[y * 20 + x] = [v, v, v, 1.0];
        }
    }
    let half = run_on(&GAUSSIAN, &[("blurriness", n(10.0))], Buf { img: small, origin: vec2(0.0, 0.0), scale: 0.5 }, Rect::from_size(40.0, 20.0), None);
    for x in [14.0, 18.0, 22.0, 26.0] {
        let (a, b) = (at(&full, x, 10.0)[0], at(&half, x, 10.0)[0]);
        assert!((a - b).abs() < 0.06, "at {x}: {a} vs {b}");
    }
}

#[test]
fn boxes_directions_and_centres() {
    let boxed = run(&FAST_BOX, &[("radius", n(3.0)), ("iterations", n(1.0))], dot(15, 15, 7, 7));
    assert!((mass(&boxed) - 1.0).abs() < 1e-3);
    assert!((at(&boxed, 9.5, 9.5)[3] - at(&boxed, 7.5, 7.5)[3]).abs() < 1e-6, "one box is flat");
    let sideways = run(&DIRECTIONAL, &[("direction", n(90.0)), ("length", n(10.0))], dot(21, 21, 10, 10));
    assert!(at(&sideways, 13.5, 10.5)[3] > 0.0 && at(&sideways, 10.5, 13.5)[3] == 0.0, "90° is across");
    let upright = run(&DIRECTIONAL, &[("length", n(10.0))], dot(21, 21, 10, 10));
    assert!(at(&upright, 10.5, 13.5)[3] > 0.0 && at(&upright, 13.5, 10.5)[3] == 0.0, "0° is up and down");
    // the radial blur's centre starts in the middle of the layer, which stays still
    let g = crate::testing::instance(&RADIAL, &[]);
    let mut started = g.clone();
    RADIAL.start(&mut started, Rect::from_size(30.0, 20.0));
    assert_eq!(started.prop("center").unwrap().value, Value::Vec2([15.0, 10.0]));
    let mut ring = Image::new(31, 31);
    ring.px[15 * 31 + 15] = [1.0; 4];
    ring.px[15 * 31 + 25] = [1.0; 4];
    let spun = run(&RADIAL, &[("amount", n(60.0)), ("center", Value::Vec2([15.5, 15.5]))], ring);
    assert!((at(&spun, 15.5, 15.5)[3] - 1.0).abs() < 1e-5, "the centre does not move");
    assert!(at(&spun, 25.5, 15.5)[3] < 0.5 && at(&spun, 24.5, 19.5)[3] > 0.0, "the rim is carried round");
}

#[test]
fn sharpening() {
    let img = halves(20, 6);
    let sharp = run(&SHARPEN, &[("amount", n(50.0))], img.clone());
    assert!(at(&sharp, 9.5, 3.0)[0] < 0.1 && at(&sharp, 10.5, 3.0)[0] > 0.9, "the edge is pushed apart");
    let quiet = run(&UNSHARP, &[("amount", n(200.0)), ("threshold", n(255.0))], img);
    assert!((at(&quiet, 9.5, 3.0)[0] - 0.1).abs() < 1e-6, "below the threshold nothing changes");
}

#[test]
fn channels_apart() {
    let out = run(&CHANNEL, &[("redBlurriness", n(6.0)), ("repeatEdge", Value::Bool(true))], halves(32, 16));
    let (l, r) = (at(&out, 15.5, 8.0), at(&out, 16.5, 8.0));
    assert!(r[0] - l[0] < 0.6, "red is blurred");
    assert!((l[1] - 0.1).abs() < 1e-5 && (r[1] - 0.9).abs() < 1e-5, "green is not");
    let alpha = run(&CHANNEL, &[("alphaBlurriness", n(4.0))], dot(9, 9, 4, 4));
    assert!(at(&alpha, 4.5, 4.5)[3] < 1.0 && (at(&alpha, 4.5, 4.5)[0] - at(&alpha, 4.5, 4.5)[3]).abs() < 1e-5, "the colour stays white");
}

/// Gives a map: dark on the left half, white on the right.
struct Map;

impl Host for Map {
    fn layer(&self, _: u64, _: crate::Take) -> Option<(Buf, Rect)> {
        let mut m = Image::new(20, 20);
        for y in 0..20 {
            for x in 10..20 {
                m.px[y * 20 + x] = [1.0; 4];
            }
        }
        Some((buf(m), Rect::from_size(20.0, 20.0)))
    }
}

#[test]
fn a_map_says_where_to_blur() {
    let mut img = Image::filled(20, 20, [0.0, 0.0, 0.0, 1.0]);
    for y in 0..20 {
        img.px[y * 20 + 4] = [1.0; 4];
        img.px[y * 20 + 15] = [1.0; 4];
    }
    let values = [("blurLayer", Value::Layer(Some(9))), ("maximumBlur", n(6.0))];
    let out = run_on(&COMPOUND, &values, buf(img.clone()), Rect::from_size(20.0, 20.0), Some(&Map));
    assert!((at(&out, 4.5, 10.5)[0] - 1.0).abs() < 1e-4, "dark keeps the line sharp");
    assert!(at(&out, 15.5, 10.5)[0] < 0.9, "white blurs it");
    let lens = run_on(&LENS, &[("blurRadius", n(6.0)), ("repeatEdge", Value::Bool(true)), ("blurMapLayer", Value::Layer(Some(9)))], buf(img), Rect::from_size(20.0, 20.0), Some(&Map));
    assert!((at(&lens, 4.5, 10.5)[0] - 1.0).abs() < 1e-4 && at(&lens, 15.5, 10.5)[0] < 0.9);
}

#[test]
fn edges_kept() {
    let img = halves(32, 16);
    let bi = run(&BILATERAL, &[("radius", n(4.0))], img.clone());
    let g = filter::gaussian(&img, 2.0, 2.0, Edge::Repeat);
    let jump = |l: [f32; 4], r: [f32; 4]| r[0] - l[0];
    assert!(jump(at(&bi, 15.5, 8.0), at(&bi, 16.5, 8.0)) > jump(g.px[8 * 32 + 15], g.px[8 * 32 + 16]) + 0.3);
    let grey = run(&BILATERAL, &[("radius", n(2.0)), ("colorize", Value::Bool(false))], Image::filled(4, 4, [1.0, 0.0, 0.0, 1.0]));
    assert!((grey.img.px[5][0] - grey.img.px[5][1]).abs() < 1e-6, "grey");
    let lines = run(&SMART, &[("radius", n(2.0)), ("mode", Value::Choice(1))], img);
    assert_eq!(at(&lines, 16.0, 8.0)[0], 1.0, "the edge is white");
    assert_eq!(at(&lines, 4.5, 8.0)[0], 0.0, "flat parts are black");
}

#[test]
fn crossed_streaked_and_along_the_picture() {
    let crossed = run(&CROSS, &[("radiusX", n(3.0)), ("radiusY", n(0.0))], dot(15, 15, 7, 7));
    assert!(at(&crossed, 9.5, 7.5)[3] > 0.0 && at(&crossed, 7.5, 9.5)[3] == 0.0);
    let streaked = run(&RADIAL_FAST, &[("center", Value::Vec2([0.5, 0.5]))], dot(21, 21, 15, 15));
    assert!(at(&streaked, 17.5, 17.5)[3] > 0.0, "streaks lead out from the centre");
    let flat = Image::filled(12, 12, [0.3, 0.5, 0.7, 1.0]);
    let still = run(&VECTOR, &[("amount", n(20.0))], flat.clone());
    assert_eq!(still.img.px, flat.px, "nothing to follow on a flat picture");
}

#[test]
fn a_lens() {
    let flat = run(&LENS, &[("repeatEdge", Value::Bool(true))], Image::filled(24, 20, [0.4, 0.3, 0.2, 1.0]));
    assert!(flat.img.px.iter().all(|p| (p[0] - 0.4).abs() < 1e-4 && (p[3] - 1.0).abs() < 1e-4));
    let disc = run(&LENS, &[("blurRadius", n(5.0))], dot(9, 9, 4, 4));
    assert!((mass(&disc) - 1.0).abs() < 1e-3 && disc.img.width > 9);
    let wide = run(&LENS, &[("blurRadius", n(5.0)), ("irisAspectRatio", n(2.0))], dot(41, 41, 20, 20));
    assert!(at(&wide, 28.5, 20.5)[3] > 0.0 && at(&wide, 20.5, 12.5)[3] == 0.0, "twice as wide as tall");
    let mut dark = Image::filled(41, 41, [0.0, 0.0, 0.0, 1.0]);
    dark.px[20 * 41 + 20] = [1.0; 4];
    let v = [("blurRadius", n(10.0)), ("irisShape", Value::Choice(8)), ("repeatEdge", Value::Bool(true))];
    let even = run(&LENS, &v, dark.clone());
    let ring = run(&LENS, &[v.as_slice(), &[("diffractionFringe", n(300.0))]].concat(), dark);
    assert!((at(&even, 20.5, 20.5)[0] - at(&even, 29.5, 20.5)[0]).abs() < 1e-5, "an even disc");
    assert!(at(&ring, 29.5, 20.5)[0] > at(&ring, 20.5, 20.5)[0] * 2.0, "a bright rim");
}

#[test]
fn flicker_softens_up_and_down() {
    let mut img = Image::filled(8, 8, [0.0, 0.0, 0.0, 1.0]);
    for x in 0..8 {
        img.px[4 * 8 + x] = [1.0; 4];
    }
    let out = run(&FLICKER, &[("softness", n(1.0))], img);
    assert!(at(&out, 3.5, 4.5)[0] < 0.9 && at(&out, 3.5, 3.5)[0] > 0.05);
    assert!((at(&out, 0.5, 4.5)[0] - at(&out, 7.5, 4.5)[0]).abs() < 1e-6);
}

fn checker(w: usize, h: usize, cell: usize) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            img.px[y * w + x] = if (x / cell + y / cell).is_multiple_of(2) { [1.0; 4] } else { [0.0, 0.0, 0.0, 1.0] };
        }
    }
    img
}

#[test]
fn zoomed_and_turned_five_ways() {
    let img = checker(64, 64, 4);
    let differs = |a: &Image, x: usize, y: usize| (0..3).map(|c| (a.px[y * 64 + x][c] - img.px[y * 64 + x][c]).abs()).sum::<f32>();
    let mut outs = Vec::new();
    for kind in 0..5 {
        let out = run(&ZOOM, &[("type", Value::Choice(kind)), ("amount", n(60.0)), ("center", Value::Vec2([32.0, 32.0]))], img.clone()).img;
        // the middle stays sharp; out at the corners it is smeared
        assert!(differs(&out, 34, 34) < 0.05, "type {kind}: near the centre");
        let smeared: f32 = (2..10).map(|k| differs(&out, k, k + 1)).sum();
        assert!(smeared > 0.5, "type {kind}: the corner {smeared}");
        outs.push(out);
    }
    for i in 0..5 {
        for j in i + 1..5 {
            assert_ne!(outs[i].px, outs[j].px, "types {i} and {j} differ");
        }
    }
    assert_eq!(run(&ZOOM, &[("amount", n(0.0))], img.clone()).img.px, img.px);
}

/// Gives the frames either side: sharp; the frame at 0 is blurred.
struct Steady;

impl Host for Steady {
    fn layer(&self, _: u64, _: crate::Take) -> Option<(Buf, Rect)> {
        None
    }

    fn this_at(&self, t: veymelo_time::Tick, _: usize) -> Option<Buf> {
        (t != veymelo_time::Tick::ZERO).then(|| buf(checker(64, 64, 8)))
    }
}

#[test]
fn a_shaken_frame_takes_its_sharper_neighbours() {
    let sharp = checker(64, 64, 8);
    let blurry = veymelo_raster::filter::gaussian(&sharp, 2.5, 2.5, Edge::Repeat);
    let host = Steady;
    let g = crate::testing::instance(&DEBLUR, &[]);
    let error = |a: &Image| a.px.iter().zip(&sharp.px).map(|(p, q)| (p[0] - q[0]).abs()).sum::<f32>();
    let out = crate::testing::run_when(&DEBLUR, &g, blurry.clone(), veymelo_time::Tick::ZERO, &[], Some(&host));
    assert!(error(&out.img) < error(&blurry) * 0.2, "sharper: {} from {}", error(&out.img), error(&blurry));
    // already sharp: nothing sharper to take; no neighbours: nothing to take
    let same = crate::testing::run_when(&DEBLUR, &g, sharp.clone(), veymelo_time::Tick::ZERO, &[], Some(&host));
    assert_eq!(same.img.px, sharp.px);
    assert_eq!(run(&DEBLUR, &[], blurry.clone()).img.px, blurry.px);
    // shown in red where frames stand in
    let g = crate::testing::instance(&DEBLUR, &[("showBlurryFrames", Value::Bool(true))]);
    let shown = crate::testing::run_when(&DEBLUR, &g, blurry.clone(), veymelo_time::Tick::ZERO, &[], Some(&host));
    let red = |img: &Image| img.px.iter().map(|p| p[0] - p[1]).sum::<f32>();
    assert!(red(&shown.img) > red(&out.img) + 10.0);
}

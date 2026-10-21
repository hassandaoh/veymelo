use super::*;
use crate::testing::{instance, n, run_when};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;
use veymelo_time::Tick;

const W: usize = 64;
const H: usize = 32;

fn go(spec: &Spec, values: &[(&str, Value)], img: Image) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(img.width as f64, img.height as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::ZERO, &[], None)
}

/// A smooth world: colour by direction (so any point of it is known).
fn world(w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let d = dir(x as f64 + 0.5, y as f64 + 0.5, w as f64, h as f64);
            img.px[y * w + x] = [(0.5 + 0.5 * d[0]) as f32, (0.5 + 0.5 * d[1]) as f32, (0.5 + 0.5 * d[2]) as f32, 1.0];
        }
    }
    img
}

fn unlike(a: &Image, b: &Image) -> f32 {
    a.px.iter().zip(&b.px).map(|(p, q)| (0..4).map(|k| (p[k] - q[k]).abs()).fold(0f32, f32::max)).fold(0f32, f32::max)
}

fn close(p: Px, q: Px, by: f32) -> bool {
    (0..4).all(|k| (p[k] - q[k]).abs() <= by)
}

#[test]
fn directions_and_turns() {
    for (x, y) in [(3.5, 7.5), (40.25, 20.0), (63.0, 1.0)] {
        let (sx, sy) = spot(dir(x, y, 64.0, 32.0), 64.0, 32.0);
        assert!((sx - x).abs() < 1e-9 && (sy - y).abs() < 1e-9);
    }
    // the middle is the front; a pan turns the view right, a tilt up
    assert!(apart(dir(32.0, 16.0, 64.0, 32.0), [0.0, 0.0, 1.0]) < 1e-12);
    assert!(apart(turn(&rotation(0.0, 90.0, 0.0), [0.0, 0.0, 1.0]), [1.0, 0.0, 0.0]) < 1e-12);
    assert!(turn(&rotation(30.0, 0.0, 0.0), [0.0, 0.0, 1.0])[1] > 0.49);
    // a roll turns the view clockwise: what was up is seen to the left of up... the view's up
    // shows the scene's up-and-right
    assert!(turn(&rotation(0.0, 0.0, 90.0), [0.0, 1.0, 0.0])[0] > 0.99);
    let m = rotation(20.0, 70.0, -35.0);
    let v = unit([0.3, -0.5, 0.81]);
    assert!(apart(turn(&turned_back(&m), turn(&m, v)), v) < 1e-7);
}

#[test]
fn a_round_box_keeps_the_sum_and_goes_round() {
    let mut line: Vec<Px> = (0..10).map(|i| [if i == 0 { 1.0 } else { 0.0 }, 0.0, 0.0, 1.0]).collect();
    round_box(&mut line, 3.0);
    let sum: f32 = line.iter().map(|p| p[0]).sum();
    assert!((sum - 1.0).abs() < 1e-5);
    assert!((line[1][0] - line[9][0]).abs() < 1e-6 && line[9][0] > 0.3, "round past the end: {line:?}");
    let mut wide = line.clone();
    round_box(&mut wide, 50.0);
    assert!(wide.iter().all(|p| (p[0] - 0.1).abs() < 1e-5), "wider than the line: its mean");
}

#[test]
fn the_sphere_blur_has_no_seam_and_goes_over_the_poles() {
    let even = Image::filled(W, H, [0.3, 0.5, 0.7, 1.0]);
    assert!(unlike(&sphere_blur(&even, 4.0), &even) < 1e-5, "an even sphere stays even");
    // a line down the seam: it spreads both ways alike
    let mut seam = Image::new(W, H);
    for y in 0..H {
        seam.px[y * W] = [1.0; 4];
    }
    let s = sphere_blur(&seam, 2.0);
    for y in 4..H - 4 {
        assert!((s.px[y * W + 1][0] - s.px[y * W + W - 1][0]).abs() < 1e-4, "row {y}");
        assert!(s.px[y * W + W - 2][0] > 0.05);
    }
    // a spot by the north pole comes over it, down the other side
    let mut pole = Image::new(W, H);
    pole.px[5] = [1.0; 4];
    let p = sphere_blur(&pole, 2.0);
    assert!(p.px[5 + W / 2][0] > 0.002, "over the pole: {}", p.px[5 + W / 2][0]);
    // nearer a pole a pixel is narrower on the sphere, so the blur reaches more of them
    let mut dots = Image::new(W, H);
    for y in [4, 16] {
        dots.px[y * W + 32] = [1.0; 4];
    }
    let d = sphere_blur(&dots, 1.5);
    let reach = |y: usize| (0..W).filter(|&x| d.px[y * W + x][0] > 1e-3).count();
    assert!(reach(4) as f32 > reach(16) as f32 * 1.6, "{} {}", reach(4), reach(16));
}

#[test]
fn blur_sharpen_and_glow_change_what_they_should() {
    let w = world(W, H);
    assert!(unlike(&go(&BLUR, &[("blurriness", n(0.0))], w.clone()).img, &w) < 1e-6);
    let mut spot = Image::filled(W, H, [0.1, 0.1, 0.1, 1.0]);
    spot.px[16 * W] = [1.0, 1.0, 1.0, 1.0];
    let blurred = go(&BLUR, &[("blurriness", n(6.0))], spot.clone());
    let (left, right) = (blurred.img.px[16 * W + W - 1][0], blurred.img.px[16 * W + 1][0]);
    assert!(left > 0.105 && (left - right).abs() < 1e-4, "across the seam as on this side: {left} {right}");
    // sharpening a soft edge makes it steeper; none leaves it
    let soft = sphere_blur(&{
        let mut i = Image::filled(W, H, [0.2, 0.2, 0.2, 1.0]);
        for p in i.px.iter_mut().enumerate().filter(|(k, _)| (k % W) >= 32).map(|(_, p)| p) {
            *p = [0.8, 0.8, 0.8, 1.0];
        }
        i
    }, 1.5);
    assert!(unlike(&go(&SHARPEN, &[("sharpenAmount", n(0.0))], soft.clone()).img, &soft) < 1e-6);
    let sharp = go(&SHARPEN, &[("sharpenAmount", n(100.0))], soft.clone());
    let step = |i: &Image| i.px[16 * W + 32][0] - i.px[16 * W + 31][0];
    assert!(step(&sharp.img) > step(&soft) * 1.2);
    // a bright spot glows round itself, past the seam; the dim parts do not
    let glow_out = go(&GLOW, &[("glowRadius", n(8.0))], spot);
    let (left, right) = (glow_out.img.px[16 * W + W - 2][0], glow_out.img.px[16 * W + 2][0]);
    assert!(left > 0.103 && (left - right).abs() < 1e-4, "round the spot, past the seam as on this side: {left} {right}");
    assert!((glow_out.img.px[4 * W + 30][0] - 0.1).abs() < 0.01);
}

#[test]
fn de_noise_evens_noise_without_a_seam() {
    let mut noisy = Image::new(W, H);
    for (i, p) in noisy.px.iter_mut().enumerate() {
        let v = 0.5 + (crate::sprites::rand(i as u64, 1, 3) - 0.5) * 0.3;
        *p = [v, v, v, 1.0];
    }
    let spread = |i: &Image| {
        let m = i.px.iter().map(|p| p[0]).sum::<f32>() / i.px.len() as f32;
        i.px.iter().map(|p| (p[0] - m).powi(2)).sum::<f32>() / i.px.len() as f32
    };
    let clean = go(&DENOISE, &[("noiseLevel", n(60.0))], noisy.clone());
    assert!(spread(&clean.img) < spread(&noisy) * 0.5, "{} {}", spread(&clean.img), spread(&noisy));
    // specks: one white pixel in grey is gone
    let mut speck = Image::filled(W, H, [0.5, 0.5, 0.5, 1.0]);
    speck.px[10 * W + 20] = [1.0; 4];
    let out = go(&DENOISE, &[("noiseType", Value::Choice(1))], speck);
    assert!((out.img.px[10 * W + 20][0] - 0.5).abs() < 0.01);
}

#[test]
fn turning_the_sphere() {
    let w = world(W, H);
    let turned = go(&ROTATE, &[("pan", n(90.0))], w.clone());
    // the middle now shows what was to the right (a quarter turn round)
    assert!(close(turned.img.px[16 * W + 32], w.px[16 * W + 48], 0.02), "{:?} {:?}", turned.img.px[16 * W + 32], w.px[16 * W + 48]);
    // turned back the other way with the same settings: as it was
    let back = go(&ROTATE, &[("pan", n(90.0)), ("tilt", n(25.0)), ("invertRotation", Value::Bool(true))], go(&ROTATE, &[("pan", n(90.0)), ("tilt", n(25.0))], w.clone()).img);
    assert!(unlike(&back.img, &w) < 0.04, "{}", unlike(&back.img, &w));
}

#[test]
fn converting_there_and_back() {
    let w = world(W * 2, H * 2);
    for kind in [1u32, 2, 3, 6, 7] {
        let cube = go(&CONVERTER, &[("targetProjection", Value::Choice(kind))], w.clone());
        let back = go(&CONVERTER, &[("sourceProjection", Value::Choice(kind)), ("targetProjection", Value::Choice(0))], cube.img);
        assert!(unlike(&back.img, &w) < 0.06, "{kind}: {}", unlike(&back.img, &w));
    }
    // a dome holds the upper half; a sphere map the front
    let dome = go(&CONVERTER, &[("targetProjection", Value::Choice(8))], w.clone());
    assert!(dome.img.px[32 * 128 + 64][1] > 0.95, "looking up in its middle");
    let front = go(&CONVERTER, &[("targetProjection", Value::Choice(4)), ("targetHorizontalFov", n(180.0))], w.clone());
    assert!(front.img.px[32 * 128 + 64][2] > 0.95 && front.img.px[0][3] == 0.0, "the front in its middle, nothing past its circle");
}

#[test]
fn flat_views_out_and_in() {
    let w = world(W, H);
    let ahead = go(&TO_PLANE, &[], w.clone());
    assert!(ahead.img.px[16 * W + 32][2] > 0.95, "straight ahead");
    let right = go(&TO_PLANE, &[("pan", n(90.0))], w.clone());
    assert!(right.img.px[16 * W + 32][0] > 0.95, "to the right");
    // the edge of a 90° view is 45° off
    let edge = ahead.img.px[16 * W][0];
    assert!((edge - (0.5 - 0.5 * FRAC_PI_4.sin()) as f32).abs() < 0.03, "{edge}");
    // a flat picture laid on the sphere: in front, nothing behind
    let card = Image::filled(W, H, [0.9, 0.2, 0.1, 1.0]);
    let laid = go(&TO_SPHERE, &[("scale", n(60.0))], card.clone());
    assert!(close(laid.img.px[16 * W + 32], [0.9, 0.2, 0.1, 1.0], 1e-3));
    assert_eq!(laid.img.px[16 * W][3], 0.0, "behind");
    let moved = go(&TO_SPHERE, &[("scale", n(60.0)), ("pan", n(-90.0))], card.clone());
    assert!(moved.img.px[16 * W + 16][3] > 0.99 && moved.img.px[16 * W + 32][3] == 0.0, "to the left");
    let soft = go(&TO_SPHERE, &[("scale", n(60.0)), ("feather", n(50.0))], card);
    assert!(soft.img.px[16 * W + 32][3] > 0.99 && soft.img.px[16 * W + 38][3] < 0.9, "softened toward its edges");
}

#[test]
fn colours_split_away_from_a_point() {
    let w = world(W, H);
    assert!(unlike(&go(&CHROMATIC, &[("aberrationRed", n(0.0)), ("aberrationBlue", n(0.0))], w.clone()).img, &w) < 1e-6);
    let split = go(&CHROMATIC, &[("aberrationRed", n(100.0)), ("aberrationBlue", n(-100.0))], w.clone());
    // at the point nothing moves; far from it red and blue part
    assert!(close(split.img.px[16 * W + 32], w.px[16 * W + 32], 0.01));
    let (far, was) = (split.img.px[16 * W + 4], w.px[16 * W + 4]);
    assert!((far[0] - was[0]).abs() > 0.01 && (far[1] - was[1]).abs() < 1e-3);
}

#[test]
fn gradients_round_the_sphere() {
    let out = go(&GRADIENTS, &[], Image::filled(W, H, [0.0, 0.0, 0.0, 1.0]));
    // on a point, its colour
    assert!(close(out.img.px[16 * W + 16], [1.0, 0.0, 0.0, 1.0], 0.05), "{:?}", out.img.px[16 * W + 16]);
    // no seam: the two sides meet
    for y in 0..H {
        assert!(close(out.img.px[y * W], out.img.px[y * W + W - 1], 0.08), "row {y}");
    }
    // at half opacity over the layer, half of each
    let half = go(&GRADIENTS, &[("opacity", n(50.0)), ("blendingMode", Value::Choice(1))], Image::filled(W, H, [0.0, 0.0, 1.0, 1.0]));
    let p = half.img.px[16 * W + 16];
    assert!((p[0] - 0.5).abs() < 0.05 && (p[2] - 0.5).abs() < 0.05, "{p:?}");
}

#[test]
fn fractal_noise_has_no_seam_or_pinch() {
    let out = go(&FRACTAL, &[], Image::filled(W * 2, H * 2, [0.0, 0.0, 0.0, 1.0]));
    let (w, h) = (W * 2, H * 2);
    let v = |x: usize, y: usize| out.img.px[y * w + x][0];
    let values: Vec<f32> = out.img.px.iter().map(|p| p[0]).collect();
    let (lo, hi) = values.iter().fold((1f32, 0f32), |(a, b), &v| (a.min(v), b.max(v)));
    assert!(hi - lo > 0.3, "noise: {lo} {hi}");
    // across the seam it goes on smoothly, like any neighbours
    let seam = (0..h).map(|y| (v(0, y) - v(w - 1, y)).abs()).fold(0f32, f32::max);
    let inside = (0..h).map(|y| (v(40, y) - v(41, y)).abs()).fold(0f32, f32::max);
    assert!(seam < inside * 2.0 + 0.02, "{seam} {inside}");
    // the top row is all one point of the sphere, nearly
    let top: Vec<f32> = (0..w).map(|x| v(x, 0)).collect();
    let (tl, th) = top.iter().fold((1f32, 0f32), |(a, b), &v| (a.min(v), b.max(v)));
    assert!(th - tl < 0.15, "{tl} {th}");
    // evolution changes it in place, smoothly
    let later = go(&FRACTAL, &[("evolution", n(60.0))], Image::filled(w, h, [0.0, 0.0, 0.0, 1.0]));
    let d = unlike(&later.img, &out.img);
    assert!(d > 0.005 && d < 0.3, "{d}");
}

#[test]
fn glitches_and_where_they_land() {
    let w = world(W, H);
    assert!(unlike(&go(&GLITCH, &[("masterAmplitude", n(0.0))], w.clone()).img, &w) < 1e-6);
    let broken = go(&GLITCH, &[("scanlines", n(0.0))], w.clone());
    assert!(unlike(&broken.img, &w) > 0.05);
    // round a point behind the camera only: the front is left alone
    let behind = go(&GLITCH, &[("targetPan", n(180.0)), ("targetRadius", n(40.0))], w.clone());
    for y in 0..H {
        assert!(close(behind.img.px[y * W + 32], w.px[y * W + 32], 1e-6));
    }
}

#[test]
fn each_eye_on_its_own() {
    // over/under: a white top eye over a black bottom one
    let mut img = Image::new(W, H * 2);
    for (i, p) in img.px.iter_mut().enumerate() {
        *p = if i / W < H { [1.0; 4] } else { [0.0, 0.0, 0.0, 1.0] };
    }
    let out = go(&BLUR, &[("blurriness", n(10.0)), ("frameLayout", Value::Choice(1))], img.clone());
    assert!(out.img.px[(H - 1) * W + 5][0] > 0.999 && out.img.px[H * W + 5][0] < 1e-3, "nothing crosses between the eyes");
    let mixed = go(&BLUR, &[("blurriness", n(10.0))], img);
    assert!(mixed.img.px[(H - 1) * W + 5][0] < 0.9, "as one frame it would");
}

#[test]
fn every_one_is_registered() {
    for id in ["vrBlur", "vrChromaticAberrations", "vrColorGradients", "vrConverter", "vrDeNoise", "vrDigitalGlitch", "vrFractalNoise", "vrGlow", "vrPlaneToSphere", "vrRotateSphere", "vrSharpen", "vrSphereToPlane"] {
        assert!(crate::find(id).is_some_and(|s| s.category == "immersive"), "{id}");
    }
}


/// The GPU makes VR Fractal Noise as the processor does: every fractal type, mono and both stereo
/// layouts, many layers, an evolution, a turned view, and blended on (its places on the sphere
/// held as the processor's doubles, its sums in floats).
#[test]
fn the_gpu_makes_vr_fractal_noise_as_the_processor_does() {
    if !veymelo_raster::gpu::available() {
        return;
    }
    let mut img = Image::new(512, 256);
    for (i, p) in img.px.iter_mut().enumerate() {
        let a = if i % 13 == 0 { 0.5 } else { 1.0 };
        *p = [(i % 512) as f32 / 512.0 * a, 0.3 * a, (i / 512) as f32 / 256.0 * a, a];
    }
    let c = |k: u32| Value::Choice(k);
    let mut cases: Vec<Vec<(&str, Value)>> = (0..4).map(|k| vec![("fractalType", c(k))]).collect();
    cases.push(vec![("frameLayout", c(1)), ("complexity", n(9.5)), ("evolution", n(250.0)), ("tilt", n(30.0)), ("pan", n(-40.0)), ("roll", n(15.0))]);
    cases.push(vec![("frameLayout", c(2)), ("contrast", n(300.0)), ("brightness", n(-20.0)), ("invert", Value::Bool(true)), ("scale", n(40.0))]);
    cases.push(vec![("blendingMode", c(3)), ("opacity", n(60.0)), ("subScaling", n(70.0)), ("subInfluence", n(80.0))]);
    for values in cases {
        let before = veymelo_raster::gpu::ran();
        let gpu = veymelo_raster::gpu::with_mode(false, || go(&FRACTAL, &values, img.clone()));
        assert!(veymelo_raster::gpu::ran() > before, "drawn by the GPU");
        let cpu = veymelo_raster::gpu::on_processor(|| go(&FRACTAL, &values, img.clone()));
        let most = gpu.img.px.iter().zip(&cpu.img.px).flat_map(|(p, q)| (0..4).map(move |k| (p[k] - q[k]).abs())).fold(0f32, f32::max);
        assert!(most <= 1e-5, "{values:?}: {most}");
    }
}

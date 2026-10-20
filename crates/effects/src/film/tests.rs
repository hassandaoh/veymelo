use super::*;
use crate::testing::{buf, instance, n, run_when};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;
use veymelo_time::Tick;

fn go_with(spec: &Spec, values: &[(&str, Value)], img: Image, t: f64, host: Option<&dyn crate::Host>) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(img.width as f64, img.height as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::from_seconds(t), &[], host)
}

fn go(spec: &Spec, values: &[(&str, Value)], img: Image) -> Buf {
    go_with(spec, values, img, 0.0, None)
}

fn grey(w: usize, h: usize, v: f32) -> Image {
    Image::filled(w, h, [v, v, v, 1.0])
}

/// Grey with grain of spread about `amp` in it.
fn grainy(w: usize, h: usize, amp: f32, seed: u64) -> Image {
    let mut img = grey(w, h, 0.5);
    for (i, p) in img.px.iter_mut().enumerate() {
        let r = |k: u64| (crate::noise::unit(crate::noise::mix(&[seed, i as u64, k])) as f32 - 0.5) * 2.0 * amp * 1.73;
        *p = [0.5 + r(0), 0.5 + r(1), 0.5 + r(2), 1.0];
    }
    img
}

fn spread(img: &Image) -> f32 {
    let n = img.px.len() as f32;
    let mean = img.px.iter().map(|p| p[1]).sum::<f32>() / n;
    (img.px.iter().map(|p| (p[1] - mean).powi(2)).sum::<f32>() / n).sqrt()
}

#[test]
fn grain_is_added() {
    let img = grey(64, 64, 0.5);
    let out = go(&ADD_GRAIN, &[], img.clone());
    assert!(spread(&out.img) > 0.01, "{}", spread(&out.img));
    assert_eq!(go(&ADD_GRAIN, &[], img.clone()).img.px, out.img.px, "the same each time");
    assert_ne!(go_with(&ADD_GRAIN, &[], img.clone(), 0.5, None).img.px, out.img.px, "it moves");
    let mono = go(&ADD_GRAIN, &[("monochromatic", Value::Bool(true))], img.clone());
    assert!(mono.img.px.iter().all(|p| (p[0] - p[1]).abs() < 1e-6 && (p[1] - p[2]).abs() < 1e-6));
    let strong = go(&ADD_GRAIN, &[("intensity", n(3.0))], img.clone());
    assert!(spread(&strong.img) > spread(&out.img) * 2.0);
    let big = go(&ADD_GRAIN, &[("size", n(4.0))], img.clone());
    // bigger grain: neighbours more alike
    let rough = |b: &Buf| b.img.px.windows(2).map(|w| (w[0][1] - w[1][1]).abs()).sum::<f32>() / spread(&b.img);
    assert!(rough(&big) < rough(&out));
    let matte = go(&ADD_GRAIN, &[("viewingMode", Value::Choice(1))], img.clone());
    assert!(matte.img.px.iter().all(|p| p[3] == 1.0 && p[0] == p[1]));
    let shown = go(&ADD_GRAIN, &[("viewingMode", Value::Choice(0)), ("previewWidth", n(20.0)), ("previewHeight", n(20.0)), ("showBox", Value::Bool(false))], img.clone());
    assert_eq!(shown.img.px[2 * 64 + 2], [0.5, 0.5, 0.5, 1.0], "outside the preview box, as it was");
    assert_ne!(shown.img.px[32 * 64 + 32], [0.5, 0.5, 0.5, 1.0]);
    for preset in 1..PRESETS.len() as u32 {
        assert_ne!(go(&ADD_GRAIN, &[("preset", Value::Choice(preset))], img.clone()).img.px, out.img.px);
    }
}

#[test]
fn grain_is_measured() {
    let flat = graininess(&grey(40, 40, 0.5), None);
    assert!(flat.spread.iter().all(|s| *s < 1e-6));
    let g = graininess(&grainy(40, 40, 0.05, 1), None);
    assert!(g.spread.iter().all(|s| *s > 0.03 && *s < 0.07), "{:?}", g.spread);
    // the flattest boxes are found where there is no detail
    let mut img = grainy(60, 30, 0.002, 2);
    for (i, p) in img.px.iter_mut().enumerate() {
        if i % 60 < 30 {
            let v = ((i * 7919) % 97) as f32 / 97.0;
            *p = [v, v, v, 1.0];
        }
    }
    let boxes = flat_boxes(&img, 3, 9);
    assert_eq!(boxes.len(), 3);
    assert!(boxes.iter().all(|b| b.0 >= 30), "{boxes:?}");
}

/// A grainy source layer 64 × 64.
struct Grainy;

impl crate::Host for Grainy {
    fn layer(&self, _: u64, _: Take) -> Option<(Buf, Rect)> {
        Some((buf(grainy(64, 64, 0.06, 7)), Rect::from_size(64.0, 64.0)))
    }
}

#[test]
fn grain_is_matched() {
    let img = grey(64, 64, 0.5);
    assert_eq!(go(&MATCH_GRAIN, &[], img.clone()).img.px, img.px, "no source, no grain");
    let out = go_with(&MATCH_GRAIN, &[("noiseSourceLayer", Value::Layer(Some(1)))], img.clone(), 0.0, Some(&Grainy));
    let s = spread(&out.img);
    assert!(s > 0.02 && s < 0.12, "about as grainy as the source: {s}");
    let samples = go_with(&MATCH_GRAIN, &[("noiseSourceLayer", Value::Layer(Some(1))), ("viewingMode", Value::Choice(1))], img.clone(), 0.0, Some(&Grainy));
    assert!(samples.img.px.contains(&[1.0, 1.0, 0.0, 1.0]), "the sample boxes drawn");
}

#[test]
fn grain_is_removed() {
    let img = grainy(64, 64, 0.04, 3);
    let out = go(&REMOVE_GRAIN, &[("noiseReduction", n(3.0)), ("passes", n(2.0))], img.clone());
    assert!(spread(&out.img) < spread(&img) * 0.6, "{} {}", spread(&out.img), spread(&img));
    let some_back = go(&REMOVE_GRAIN, &[("noiseReduction", n(3.0)), ("passes", n(2.0)), ("texture", n(0.5))], img.clone());
    assert!(spread(&some_back.img) > spread(&out.img));
    // an edge stays sharp
    let mut edge = grey(40, 20, 0.1);
    for (i, p) in edge.px.iter_mut().enumerate() {
        if i % 40 >= 20 {
            *p = [0.9, 0.9, 0.9, 1.0];
        }
    }
    let kept = go(&REMOVE_GRAIN, &[("noiseReduction", n(3.0))], edge.clone());
    assert!(kept.img.px[10 * 40 + 18][0] < 0.25 && kept.img.px[10 * 40 + 21][0] > 0.75);
    let boxes = go(&REMOVE_GRAIN, &[("viewingMode", Value::Choice(1))], img.clone());
    assert!(boxes.img.px.contains(&[1.0, 1.0, 0.0, 1.0]));
    assert_eq!(go(&REMOVE_GRAIN, &[("noiseReduction", n(0.0))], img.clone()).img.px, img.px);
}

#[test]
fn medians() {
    let mut img = grey(16, 16, 0.5);
    for (x, y) in [(3, 3), (8, 2), (12, 12), (5, 10)] {
        img.px[y * 16 + x] = [1.0; 4];
    }
    let out = go(&MEDIAN, &[("radius", n(1.0))], img.clone());
    assert!(out.img.px.iter().all(|p| (p[0] - 0.5).abs() < 2.0 / 511.0), "specks gone");
    // the same as working it out the slow way
    let mut pic = Image::new(9, 7);
    for (i, p) in pic.px.iter_mut().enumerate() {
        let v = ((i * 37) % 23) as f32 / 22.0;
        *p = [v, 1.0 - v, (v * 3.0).fract(), 1.0];
    }
    let fast = median_of(&pic, 2);
    for y in 0..7i64 {
        for x in 0..9i64 {
            for c in 0..3 {
                let mut vals: Vec<f32> = Vec::new();
                for dy in -2..=2 {
                    for dx in -2..=2 {
                        let (xx, yy) = ((x + dx).clamp(0, 8) as usize, (y + dy).clamp(0, 6) as usize);
                        vals.push((pic.px[yy * 9 + xx][c] * 511.0).round() / 511.0);
                    }
                }
                vals.sort_by(f32::total_cmp);
                assert!((fast.px[y as usize * 9 + x as usize][c] - vals[12]).abs() < 1e-6, "({x}, {y}) channel {c}");
            }
        }
    }
    let legacy = go(&MEDIAN_LEGACY, &[], img.clone());
    assert!(legacy.img.px.iter().all(|p| (p[0] - 0.5).abs() < 2.0 / 511.0));
    // dust and scratches leaves what differs little
    let mut soft = grey(16, 16, 0.5);
    soft.px[5 * 16 + 5] = [0.52, 0.52, 0.52, 1.0];
    soft.px[9 * 16 + 9] = [1.0; 4];
    let d = go(&DUST, &[("threshold", n(20.0))], soft.clone());
    assert_eq!(d.img.px[5 * 16 + 5], [0.52, 0.52, 0.52, 1.0], "a small change stays");
    assert!((d.img.px[9 * 16 + 9][0] - 0.5).abs() < 0.01, "a speck goes");
}

#[test]
fn every_one_is_registered() {
    for spec in ALL {
        assert!(crate::find(spec.def.id).is_some_and(|s| s.category == "noise"), "{}", spec.def.id);
    }
}

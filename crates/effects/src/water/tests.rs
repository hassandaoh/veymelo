use super::*;
use crate::testing::{instance, n, run_when};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;

const W: usize = 64;
const H: usize = 48;

/// Stripes of colour across, darker down.
fn picture() -> Image {
    let mut img = Image::new(W, H);
    for (i, p) in img.px.iter_mut().enumerate() {
        let (x, y) = (i % W, i / W);
        let band = if (x / 8) % 2 == 0 { 0.8 } else { 0.3 };
        *p = [band, 0.5 * (1.0 - y as f32 / H as f32), 1.0 - band, 1.0];
    }
    img
}

/// Other layers, by id, the same at every time.
struct Layers(Vec<(u64, Image)>);

impl crate::Host for Layers {
    fn layer(&self, id: u64, _: Take) -> Option<(Buf, Rect)> {
        let (_, img) = self.0.iter().find(|(k, _)| *k == id)?;
        Some((crate::testing::buf(img.clone()), Rect::from_size(img.width as f64, img.height as f64)))
    }

    fn layer_at(&self, id: u64, _: Tick, take: Take) -> Option<(Buf, Rect)> {
        self.layer(id, take)
    }
}

fn go(spec: &Spec, values: &[(&str, Value)], img: Image, t: f64, host: Option<&dyn crate::Host>) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(img.width as f64, img.height as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::from_seconds(t), &[], host)
}

fn unlike(a: &Image, b: &Image) -> f32 {
    a.px.iter().zip(&b.px).map(|(p, q)| (0..4).map(|k| (p[k] - q[k]).abs()).fold(0f32, f32::max)).fold(0f32, f32::max)
}

/// Ripples running down: brightness rising and falling every 16 pixels across.
fn ripples() -> Image {
    let mut img = Image::new(W, H);
    for (i, p) in img.px.iter_mut().enumerate() {
        let v = 0.5 + 0.5 * (TAU * (i % W) as f32 / 16.0).sin();
        *p = [v, v, v, 1.0];
    }
    img
}

/// Lit evenly, with no shine and no surface of its own.
const PLAIN: [(&str, f64); 4] = [("ambientLight", 1.0), ("diffuseReflection", 0.0), ("specularReflection", 0.0), ("surfaceOpacity", 0.0)];

fn plain(more: &[(&'static str, Value)]) -> Vec<(&'static str, Value)> {
    PLAIN.iter().map(|(k, v)| (*k, n(*v))).chain(more.iter().cloned()).collect()
}

#[test]
fn still_clear_water_shows_the_bottom() {
    let out = go(&CAUSTICS, &plain(&[]), picture(), 0.0, None);
    assert!(unlike(&out.img, &picture()) < 1e-4, "{}", unlike(&out.img, &picture()));
    // the surface's own colour over it, as opaque as asked
    let half = go(&CAUSTICS, &plain(&[("surfaceOpacity", n(0.5)), ("surfaceColor", Value::Color(veymelo_color::Rgba::BLACK))]), picture(), 0.0, None);
    let (p, q) = (half.img.px[5 * W + 3], picture().px[5 * W + 3]);
    assert!((0..3).all(|k| (p[k] - q[k] * 0.5).abs() < 1e-4), "{p:?} {q:?}");
}

#[test]
fn ripples_bend_the_bottom_and_gather_the_light() {
    let host = Layers(vec![(2, ripples())]);
    let rippled = [("waterSurface", Value::Layer(Some(2))), ("smoothing", n(0.0)), ("waveHeight", n(0.3)), ("waterDepth", n(0.15)), ("refractiveIndex", n(1.33))];
    let bent = go(&CAUSTICS, &plain(&rippled), picture(), 0.0, Some(&host));
    assert!(unlike(&bent.img, &picture()) > 0.3, "bent");
    // across the ripples only: down each column the bottom stays where it was
    assert!((0..W).all(|x| (bent.img.px[10 * W + x][2] - bent.img.px[30 * W + x][2]).abs() < 1e-4));
    // on an even bottom the gathered light makes bright lines and dark gaps, but no more light
    // in all (away from the edges, where the light from beyond them is missing)
    let grey = Image::filled(W, H, [0.4, 0.4, 0.4, 1.0]);
    let even = go(&CAUSTICS, &plain(&rippled), grey.clone(), 0.0, Some(&host));
    let lines = go(&CAUSTICS, &plain(&[rippled.as_slice(), &[("causticsStrength", n(1.0))]].concat()), grey, 0.0, Some(&host));
    let row = |b: &Buf| (16..48).map(|x| b.img.px[20 * W + x][0]).collect::<Vec<_>>();
    let (e, l) = (row(&even), row(&lines));
    assert!(e.iter().all(|v| (v - 0.4).abs() < 1e-4), "no caustics: even");
    let (lo, hi) = l.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    assert!(lo < 0.3 && hi > 0.6, "{lo} {hi}");
    // brightest under the crests (at 4, 20, 36 … across), which gather it
    let top = 16 + (0..l.len()).max_by(|&a, &b| l[a].total_cmp(&l[b])).unwrap();
    assert!((top as i64 - 19).abs() <= 2 || (top as i64 - 35).abs() <= 2, "{top}");
}

#[test]
fn the_sky_mirrors_on_the_surface() {
    let host = Layers(vec![(4, Image::filled(W, H, [1.0, 0.0, 0.0, 1.0]))]);
    let out = go(&CAUSTICS, &plain(&[("sky", Value::Layer(Some(4))), ("skyIntensity", n(1.0))]), picture(), 0.0, Some(&host));
    assert!(out.img.px.iter().all(|p| p[0] > 0.999 && p[1] < 1e-3 && p[2] < 1e-3), "a mirror of the sky");
    let some = go(&CAUSTICS, &plain(&[("sky", Value::Layer(Some(4))), ("skyIntensity", n(0.5))]), picture(), 0.0, Some(&host));
    let (p, q) = (some.img.px[3 * W + 2], picture().px[3 * W + 2]);
    assert!((p[0] - (q[0] * 0.5 + 0.5)).abs() < 1e-3 && (p[2] - q[2] * 0.5).abs() < 1e-3, "{p:?}");
}

#[test]
fn a_smaller_bottom_tiles() {
    let out = go(&CAUSTICS, &plain(&[("bottomScaling", n(0.5)), ("bottomRepeatMode", Value::Choice(1))]), picture(), 0.0, None);
    // seen at half size, so it comes round again every half width
    for x in [4, 12, 20] {
        let (p, q) = (out.img.px[20 * W + x], out.img.px[20 * W + x + 32]);
        assert!((0..3).all(|k| (p[k] - q[k]).abs() < 0.02), "{x}: {p:?} {q:?}");
    }
    let once = go(&CAUSTICS, &plain(&[("bottomScaling", n(0.5)), ("bottomRepeatMode", Value::Choice(0))]), picture(), 0.0, None);
    assert!(once.img.px[2][3] < 0.01, "past it, clear");
}

/// Wave World's height map at time `t` with these settings over the defaults.
fn heights(values: &[(&str, Value)], t: f64, host: Option<&dyn crate::Host>) -> Buf {
    let mut all = vec![("view", Value::Choice(1)), ("gridResolution", n(32.0))];
    all.extend(values.iter().cloned());
    go(&WAVE_WORLD, &all, Image::new(W, H), t, host)
}

#[test]
fn still_water_is_level() {
    let out = heights(&[("producer1Amplitude", n(0.0))], 1.0, None);
    assert!(out.img.px.iter().all(|p| (p[0] - 0.5).abs() < 1e-6 && p[3] == 1.0));
}

#[test]
fn waves_spread_from_a_ring() {
    let out = heights(&[("waveSpeed", n(0.5))], 0.5, None);
    // half a layer width a second: half a second on, a quarter width (16 pixels) out
    let ring = |lo: f64, hi: f64| {
        out.img.px.iter().enumerate().filter(|(i, _)| {
            let d = (((i % W) as f64 + 0.5 - 32.0).powi(2) + ((i / W) as f64 + 0.5 - 24.0).powi(2)).sqrt();
            d >= lo && d < hi
        }).map(|(_, p)| (p[0] - 0.5).abs()).fold(0f32, f32::max)
    };
    assert!(ring(0.0, 14.0) > 0.05, "the waves near: {}", ring(0.0, 14.0));
    assert!(ring(26.0, 100.0) < 0.01, "not yet far: {}", ring(26.0, 100.0));
    // a later frame has gone on (and is not the same)
    assert!(unlike(&out.img, &heights(&[("waveSpeed", n(0.5))], 0.75, None).img) > 0.01);
}

#[test]
fn open_edges_let_the_waves_out() {
    // near the edges, long after the waves first reached them
    let near = |b: &Buf| {
        let picked: Vec<f32> = b.img.px.iter().enumerate().filter(|(i, _)| {
            let (x, y) = (i % W, i / W);
            !(4..W - 4).contains(&x) || !(4..H - 4).contains(&y)
        }).map(|(_, p)| (p[0] - 0.5).abs()).collect();
        picked.iter().sum::<f32>() / picked.len() as f32
    };
    let settings = |edges: u32| vec![("waveSpeed", n(0.5)), ("damping", n(0.0)), ("reflectEdges", Value::Choice(edges)), ("contrast", n(0.2))];
    let (open, walled) = (near(&heights(&settings(0), 4.0, None)), near(&heights(&settings(5), 4.0, None)));
    assert!(walled > open * 1.3, "walls keep them in: {walled} vs {open}");
}

#[test]
fn dry_ground_holds_no_water() {
    // white on the right: the ground comes up to the water's surface there
    let mut ground = Image::filled(W, H, [0.0, 0.0, 0.0, 1.0]);
    for (i, p) in ground.px.iter_mut().enumerate() {
        if i % W >= 44 {
            *p = [1.0; 4];
        }
    }
    let host = Layers(vec![(3, ground)]);
    let out = heights(&[("ground", Value::Layer(Some(3))), ("renderDryAreasAs", Value::Choice(1)), ("waveSpeed", n(0.5))], 2.0, Some(&host));
    for y in 0..H {
        assert!(out.img.px[y * W + 60][3] == 0.0 && out.img.px[y * W + 20][3] == 1.0, "{y}: {:?} {:?}", out.img.px[y * W + 60], out.img.px[y * W + 20]);
    }
    assert!(out.img.px.iter().all(|p| p.iter().all(|v| v.is_finite())));
    // or solid: level, as still water
    let solid = heights(&[("ground", Value::Layer(Some(3)))], 2.0, Some(&host));
    assert_eq!(solid.img.px[10 * W + 60], [0.5, 0.5, 0.5, 1.0]);
}

#[test]
fn shallow_water_is_slower() {
    // grey ground: everywhere wet, but shallow
    let host = Layers(vec![(3, Image::filled(W, H, [0.8, 0.8, 0.8, 1.0]))]);
    let reach = |b: &Buf| (32..W).rev().find(|&x| (b.img.px[24 * W + x][0] - 0.5).abs() > 0.01).unwrap_or(0);
    let deep = heights(&[("waveSpeed", n(0.5)), ("ground", Value::Layer(Some(3)))], 0.6, Some(&host));
    let shallow = heights(&[("waveSpeed", n(0.5)), ("ground", Value::Layer(Some(3))), ("waveStrength", n(1.0))], 0.6, Some(&host));
    assert!(reach(&shallow) + 3 < reach(&deep), "{} {}", reach(&shallow), reach(&deep));
}

#[test]
fn the_wireframe_draws_the_grid() {
    let out = go(&WAVE_WORLD, &[("gridResolution", n(16.0))], Image::new(W, H), 1.0, None);
    assert!(out.img.px.iter().all(|p| p[3] == 1.0), "over black");
    let lit = out.img.px.iter().filter(|p| p[1] > 0.2).count();
    assert!(lit > W * H / 20 && lit < W * H * 3 / 4, "{lit}");
}

#[test]
fn every_one_is_registered() {
    for id in ["caustics", "waveWorld"] {
        assert!(crate::find(id).is_some_and(|s| s.category == "simulation"), "{id}");
    }
}

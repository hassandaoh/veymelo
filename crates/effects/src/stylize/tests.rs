use super::*;
use crate::testing::{at as px_at, buf, instance, n, run_when};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;
use veymelo_time::Tick;

fn picture(w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            img.px[y * w + x] = [x as f32 / w as f32, y as f32 / h as f32, ((x * 7 + y * 3) % 11) as f32 / 11.0, 1.0];
        }
    }
    img
}

fn get(img: &Image, x: usize, y: usize) -> Px {
    img.px[y * img.width + x]
}

fn go(spec: &Spec, values: &[(&str, Value)], img: Image) -> Buf {
    go_at(spec, values, img, 0.0, None)
}

fn go_at(spec: &Spec, values: &[(&str, Value)], img: Image, t: f64, host: Option<&dyn crate::Host>) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(img.width as f64, img.height as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::from_seconds(t), &[], host)
}

/// A white square 10 across in the middle of a black 40 × 40 layer.
fn white_square() -> Image {
    let mut img = Image::filled(40, 40, [0.0, 0.0, 0.0, 1.0]);
    for y in 15..25 {
        for x in 15..25 {
            img.px[y * 40 + x] = [1.0; 4];
        }
    }
    img
}

#[test]
fn glow_spills_light() {
    let img = white_square();
    let out = go(&GLOW, &[], img.clone());
    assert!(out.img.width > 40, "room to spill into");
    assert!(px_at(&out, 13.0, 20.0)[0] > 0.1, "light beside the square");
    assert!((px_at(&out, 20.0, 20.0)[0] - 1.0).abs() < 0.3 || px_at(&out, 20.0, 20.0)[0] >= 1.0, "the square still bright");
    assert!(px_at(&out, 2.0, 2.0)[0] < 0.02, "far off, dark");
    let behind = go(&GLOW, &[("compositeOriginal", Value::Choice(1))], img.clone());
    assert!((px_at(&behind, 13.0, 20.0)[0] - 0.0).abs() < 1e-4, "behind an opaque layer it does not show");
    let alone = go(&GLOW, &[("compositeOriginal", Value::Choice(2))], img.clone());
    assert!(px_at(&alone, 2.0, 2.0)[3] < 0.02, "the glow alone");
    let ab = go(&GLOW, &[("glowColors", Value::Choice(1)), ("colorA", Value::Color(veymelo_color::rgba(1.0, 0.0, 0.0, 1.0))), ("colorB", Value::Color(veymelo_color::rgba(1.0, 0.0, 0.0, 1.0)))], img.clone());
    let g = px_at(&ab, 13.0, 20.0);
    assert!(g[0] > 0.05 && g[1] < 0.01, "a red glow: {g:?}");
    let mapped = go(&GLOW, &[("glowColors", Value::Choice(2)), ("colorLooping", Value::Choice(0)), ("arbitraryMap", Value::Text("0,0 1,1 | 0,0 1,0 | 0,0 1,0".into()))], img.clone());
    let g = px_at(&mapped, 13.0, 20.0);
    assert!(g[0] > 0.02 && g[1] < 0.001, "red only from the map: {g:?}");
    assert!((a_to_b(0.25, 0, 1.0, 0.0, 0.5) - 0.25).abs() < 1e-5 && (a_to_b(0.25, 1, 1.0, 0.0, 0.5) - 0.75).abs() < 1e-5);
    assert!((a_to_b(0.5, 2, 1.0, 0.0, 0.5) - 1.0).abs() < 1e-5, "a triangle peaks half way");
    let across = go(&GLOW, &[("glowDimensions", Value::Choice(1))], img.clone());
    assert!(px_at(&across, 13.0, 20.0)[0] > 0.1 && px_at(&across, 20.0, 13.0)[0] < 0.01, "only sideways");
}

#[test]
fn blocks_and_steps() {
    let img = picture(40, 40);
    let out = go(&MOSAIC, &[("horizontalBlocks", n(4.0)), ("verticalBlocks", n(2.0))], img.clone());
    assert_eq!(get(&out.img, 0, 0), get(&out.img, 9, 19), "one block");
    assert_ne!(get(&out.img, 9, 0), get(&out.img, 10, 0));
    assert!((get(&out.img, 0, 0)[0] - 0.1125).abs() < 1e-3, "the mean: {:?}", get(&out.img, 0, 0));
    let sharp = go(&MOSAIC, &[("horizontalBlocks", n(4.0)), ("verticalBlocks", n(2.0)), ("sharpColors", Value::Bool(true))], img.clone());
    assert_eq!(get(&sharp.img, 0, 0), get(&img, 5, 10), "the middle's colour");
    let post = go(&POSTERIZE, &[("level", n(2.0))], img.clone());
    assert!(post.img.px.iter().all(|p| p[..3].iter().all(|v| *v == 0.0 || *v == 1.0)));
    let th = go(&THRESHOLD, &[], img.clone());
    assert!(th.img.px.iter().all(|p| p[0] == p[1] && p[1] == p[2] && (p[0] == 0.0 || p[0] == 1.0)));
}

#[test]
fn edges_and_relief() {
    let img = white_square();
    let e = go(&FIND_EDGES, &[], img.clone());
    assert!(get(&e.img, 15, 20)[0] < 0.5 && get(&e.img, 5, 5)[0] > 0.99 && get(&e.img, 20, 20)[0] > 0.99, "dark edges on white");
    let inv = go(&FIND_EDGES, &[("invert", Value::Bool(true))], img.clone());
    assert!(get(&inv.img, 15, 20)[0] > 0.5 && get(&inv.img, 5, 5)[0] < 0.01);
    let em = go(&EMBOSS, &[], img.clone());
    assert!((get(&em.img, 5, 5)[0] - 0.5).abs() < 1e-4, "flat parts grey");
    // lit from the top right (45°): the square's right side bright, its left dark
    assert!(get(&em.img, 24, 20)[0] > 0.7 && get(&em.img, 15, 20)[0] < 0.3, "{:?} {:?}", get(&em.img, 24, 20), get(&em.img, 15, 20));
    let red = Image::filled(20, 20, [1.0, 0.0, 0.0, 1.0]);
    let ce = go(&COLOR_EMBOSS, &[], red.clone());
    assert_eq!(get(&ce.img, 10, 10), [1.0, 0.0, 0.0, 1.0], "flat colour stays");
}

#[test]
fn cartoon_flattens_and_inks() {
    let img = picture(48, 48);
    let out = go(&CARTOON, &[], img.clone());
    assert_ne!(out.img.px, img.px);
    let fill = go(&CARTOON, &[("render", Value::Choice(0)), ("shadingSteps", n(2.0)), ("shadingSmoothness", n(0.0))], img.clone());
    // two steps of brightness: few different brightnesses left
    let mut levels: Vec<i32> = fill.img.px.iter().map(|p| (luma(unpremul(*p).0) * 20.0).round() as i32).collect();
    levels.sort();
    levels.dedup();
    assert!(levels.len() <= 4, "{levels:?}");
    let sq = go(&CARTOON, &[("render", Value::Choice(1))], white_square());
    assert!(get(&sq.img, 15, 20)[0] < 0.5 && get(&sq.img, 5, 5)[0] > 0.9, "inked at the edge");
    assert_eq!(smooth_keeping_edges(&Image::filled(10, 10, [0.3, 0.2, 0.1, 1.0]), 3, 0.01).px[55], [0.3, 0.2, 0.1, 1.0].map(|v: f32| v), "flat stays flat");
}

#[test]
fn rough_edges() {
    let mut img = Image::new(60, 60);
    for y in 10..50 {
        for x in 10..50 {
            img.px[y * 60 + x] = [0.5, 0.5, 0.5, 1.0];
        }
    }
    let out = go(&ROUGHEN, &[], img.clone());
    assert_eq!(get(&out.img, 30, 30)[3], 1.0, "the middle stays");
    let partial = out.img.px.iter().zip(&img.px).filter(|(a, b)| b[3] > 0.0 && a[3] < 0.99).count();
    assert!(partial > 20, "the edge worn: {partial}");
    let other = go(&ROUGHEN, &[("randomSeed", n(5.0))], img.clone());
    assert_ne!(other.img.px, out.img.px);
    let rusty = go(&ROUGHEN, &[("edgeType", Value::Choice(1))], img.clone());
    assert!(rusty.img.px.iter().any(|p| p[3] > 0.5 && p[0] / p[3] > 0.52 && p[2] / p[3] < 0.48), "coloured at the edge");
}

#[test]
fn scatter_and_strobe() {
    let img = picture(40, 40);
    assert_eq!(go(&SCATTER, &[], img.clone()).img.px, img.px);
    let s = go(&SCATTER, &[("scatterAmount", n(3.0))], img.clone());
    assert_ne!(s.img.px, img.px);
    let across = go(&SCATTER, &[("scatterAmount", n(3.0)), ("grain", Value::Choice(1))], img.clone());
    assert!(across.img.px.iter().enumerate().all(|(i, p)| p[3] == 0.0 || (p[1] - img.px[i][1]).abs() < 1e-6), "only sideways: rows keep their green");
    assert_eq!(go_at(&SCATTER, &[("scatterAmount", n(3.0))], img.clone(), 1.0, None).img.px, s.img.px, "the same each frame");
    assert_ne!(go_at(&SCATTER, &[("scatterAmount", n(3.0)), ("scatterRandomness", Value::Bool(true))], img.clone(), 1.0, None).img.px, s.img.px);
    assert!(flashing(0.02, 0.05, 1.0, 0.0, 1) && !flashing(0.5, 0.05, 1.0, 0.0, 1) && flashing(1.03, 0.05, 1.0, 0.0, 1));
    let lit = go_at(&STROBE, &[], img.clone(), 0.01, None);
    assert!(lit.img.px.iter().all(|p| *p == [1.0; 4]), "white while it flashes");
    assert_eq!(go_at(&STROBE, &[], img.clone(), 0.5, None).img.px, img.px);
    let gone = go_at(&STROBE, &[("strobe", Value::Choice(1))], img.clone(), 0.01, None);
    assert!(gone.img.px.iter().all(|p| p[3] == 0.0));
    let always = (0..20).filter(|k| flashing(*k as f64 + 0.5, 0.05, 1.0, 1.0, 3)).count();
    assert_eq!(always, 20, "certain chance: every period");
}

/// A texture: a bright stripe down the middle of a 40 × 40 layer.
struct Stripe;

impl crate::Host for Stripe {
    fn layer(&self, _: u64, _: Take) -> Option<(Buf, Rect)> {
        let mut m = Image::filled(40, 40, [0.0, 0.0, 0.0, 1.0]);
        for y in 0..40 {
            for x in 18..22 {
                m.px[y * 40 + x] = [1.0; 4];
            }
        }
        Some((buf(m), Rect::from_size(40.0, 40.0)))
    }
}

#[test]
fn texturize_presses_a_texture_in() {
    let img = Image::filled(40, 40, [0.5, 0.5, 0.5, 1.0]);
    assert_eq!(go(&TEXTURIZE, &[], img.clone()).img.px, img.px, "no texture, no change");
    let out = go_at(&TEXTURIZE, &[("textureLayer", Value::Layer(Some(2)))], img.clone(), 0.0, Some(&Stripe));
    // lit from the top left (135°): one side of the stripe lighter, the other darker
    let (l, r) = (get(&out.img, 17, 20)[0], get(&out.img, 22, 20)[0]);
    assert!((l - 0.5).abs() > 0.05 && (r - 0.5).abs() > 0.05 && (l - 0.5) * (r - 0.5) < 0.0, "{l} {r}");
    assert_eq!(get(&out.img, 5, 20), [0.5, 0.5, 0.5, 1.0]);
}

#[test]
fn every_one_is_registered() {
    for spec in ALL {
        assert!(crate::find(spec.def.id).is_some_and(|s| s.category == "stylize"), "{}", spec.def.id);
    }
}

/// The GPU wears the edges as the processor does: every edge type, no border and a wide one,
/// stretched, evolved and cycled.
#[test]
fn the_gpu_roughens_edges_as_the_processor_does() {
    if !veymelo_raster::gpu::available() {
        return;
    }
    let mut img = Image::new(320, 240);
    for (i, p) in img.px.iter_mut().enumerate() {
        let (x, y) = ((i % 320) as f32 - 160.0, (i / 320) as f32 - 120.0);
        let a = if x * x / 1.3 + y * y < 90.0 * 90.0 { 1.0 } else { 0.0 };
        *p = [0.8 * a, 0.5 * a, 0.2 * a, a];
    }
    let c = |k: u32| Value::Choice(k);
    let mut cases: Vec<Vec<(&str, Value)>> = (0..8).map(|k| vec![("edgeType", c(k)), ("border", n(14.0))]).collect();
    cases.push(vec![("border", n(0.0)), ("complexity", n(5.5)), ("stretchWidthOrHeight", n(2.5)), ("evolution", n(300.0))]);
    cases.push(vec![("border", n(60.0)), ("cycleEvolution", Value::Bool(true)), ("cycle", n(2.0)), ("evolution", n(500.0)), ("stretchWidthOrHeight", n(-3.0)), ("edgeSharpness", n(4.0))]);
    for values in cases {
        let before = veymelo_raster::gpu::ran();
        let gpu = veymelo_raster::gpu::with_mode(false, || go(&ROUGHEN, &values, img.clone()));
        assert!(veymelo_raster::gpu::ran() > before, "drawn by the GPU");
        let cpu = veymelo_raster::gpu::on_processor(|| go(&ROUGHEN, &values, img.clone()));
        let most = gpu.img.px.iter().zip(&cpu.img.px).flat_map(|(p, q)| (0..4).map(move |k| (p[k] - q[k]).abs())).fold(0f32, f32::max);
        assert!(most <= 1e-5, "{values:?}: {most}");
    }
}

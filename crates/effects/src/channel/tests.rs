use super::*;
use crate::testing::{buf, instance, n, run_when};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;
use veymelo_raster::Image;
use veymelo_time::Tick;

fn picture(w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            img.px[y * w + x] = [x as f32 / w as f32, y as f32 / h as f32, 0.3, 1.0];
        }
    }
    img
}

fn get(img: &Image, x: usize, y: usize) -> Px {
    img.px[y * img.width + x]
}

fn near(a: Px, b: Px, tol: f32) -> bool {
    (0..4).all(|k| (a[k] - b[k]).abs() <= tol)
}

fn go_with(spec: &Spec, values: &[(&str, Value)], img: Image, host: Option<&dyn crate::Host>) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(img.width as f64, img.height as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::ZERO, &[], host)
}

fn go(spec: &Spec, values: &[(&str, Value)], img: Image) -> Buf {
    go_with(spec, values, img, None)
}

/// Another layer 16 × 8: white on its left half, black on its right; and the layer before its
/// effects is plain red.
struct Other;

impl crate::Host for Other {
    fn layer(&self, _: u64, _: Take) -> Option<(Buf, Rect)> {
        let mut m = Image::filled(16, 8, [0.0, 0.0, 0.0, 1.0]);
        for y in 0..8 {
            for x in 0..8 {
                m.px[y * 16 + x] = [1.0; 4];
            }
        }
        Some((buf(m), Rect::from_size(16.0, 8.0)))
    }

    fn this_at(&self, _: Tick, _: usize) -> Option<Buf> {
        Some(buf(Image::filled(16, 8, [1.0, 0.0, 0.0, 1.0])))
    }
}

#[test]
fn invert_channels() {
    let img = picture(16, 8);
    let rgb = go(&INVERT, &[], img.clone());
    assert!(near(get(&rgb.img, 4, 2), [0.75, 0.75, 0.7, 1.0], 1e-6));
    let red = go(&INVERT, &[("channel", Value::Choice(1))], img.clone());
    assert!(near(get(&red.img, 4, 2), [0.75, 0.25, 0.3, 1.0], 1e-6));
    let yiq = go(&INVERT, &[("channel", Value::Choice(9))], Image::filled(4, 4, [0.2, 0.2, 0.2, 1.0]));
    assert!(near(yiq.img.px[0], [0.8, 0.8, 0.8, 1.0], 1e-3), "grey's brightness inverted: {:?}", yiq.img.px[0]);
    let mut half = Image::filled(4, 4, [0.25, 0.25, 0.25, 0.25]);
    half.px[0] = [0.0; 4];
    let alpha = go(&INVERT, &[("channel", Value::Choice(12))], half.clone());
    assert!((alpha.img.px[5][3] - 0.75).abs() < 1e-6 && (alpha.img.px[5][0] / alpha.img.px[5][3] - 1.0).abs() < 1e-6, "colour kept");
    let kept = go(&INVERT, &[("blendWithOriginal", n(100.0))], img.clone());
    assert!(kept.img.px.iter().zip(&img.px).all(|(a, b)| near(*a, *b, 1e-6)));
}

#[test]
fn channels_moved_about() {
    let img = picture(16, 8);
    assert_eq!(go(&SHIFT_CHANNELS, &[], img.clone()).img.px, img.px);
    let swapped = go(&SHIFT_CHANNELS, &[("takeRedFrom", Value::Choice(2)), ("takeGreenFrom", Value::Choice(1))], img.clone());
    let (a, b) = (get(&swapped.img, 4, 2), get(&img, 4, 2));
    assert!(near(a, [b[1], b[0], b[2], 1.0], 1e-6));
    let full = go(&SHIFT_CHANNELS, &[("takeBlueFrom", Value::Choice(8)), ("takeAlphaFrom", Value::Choice(9))], img.clone());
    assert!(near(get(&full.img, 4, 2), [0.125, 0.125, 0.5, 0.5], 1e-6), "{:?}", get(&full.img, 4, 2));
    // set from another layer: alpha from its luminance (white left, black right)
    let set = go_with(&SET_CHANNELS, &[("sourceLayer4", Value::Layer(Some(5))), ("setAlphaToSource4s", Value::Choice(4))], img.clone(), Some(&Other));
    assert_eq!(get(&set.img, 2, 2)[3], 1.0);
    assert_eq!(get(&set.img, 13, 2)[3], 0.0);
    let comb = go(&COMBINER, &[], img.clone());
    let l = hsl([get(&img, 4, 2)[0], get(&img, 4, 2)[1], 0.3]).2;
    assert!(near(get(&comb.img, 4, 2), [l, l, l, 1.0], 1e-5), "lightness as grey");
    let back = go(&COMBINER, &[("from", Value::Choice(1))], go(&COMBINER, &[("from", Value::Choice(0)), ("to", Value::Choice(7))], img.clone()).img);
    assert!(back.img.px.iter().zip(&img.px).all(|(a, b)| near(*a, *b, 1e-3)), "to HLS and back");
    let yuv = go(&COMBINER, &[("from", Value::Choice(3))], go(&COMBINER, &[("from", Value::Choice(2))], img.clone()).img);
    assert!(yuv.img.px.iter().zip(&img.px).all(|(a, b)| near(*a, *b, 1e-3)), "to YUV and back");
}

#[test]
fn minimax_grows_and_shrinks() {
    let mut img = Image::filled(20, 20, [0.0, 0.0, 0.0, 1.0]);
    img.px[10 * 20 + 10] = [1.0; 4];
    let grown = go(&MINIMAX, &[("radius", n(2.0))], img.clone());
    assert_eq!(get(&grown.img, 12, 12), [1.0; 4], "a white dot grows into a square");
    assert_eq!(get(&grown.img, 13, 10)[0], 0.0);
    let gone = go(&MINIMAX, &[("radius", n(1.0)), ("operation", Value::Choice(0))], img.clone());
    assert!(gone.img.px.iter().all(|p| p[0] == 0.0), "a dot shrinks away");
    let across = go(&MINIMAX, &[("radius", n(2.0)), ("direction", Value::Choice(1))], img.clone());
    assert_eq!(get(&across.img, 12, 10)[0], 1.0);
    assert_eq!(get(&across.img, 10, 12)[0], 0.0);
    // the alpha shrinks from the edges, unless they are held
    let solid = Image::filled(10, 10, [1.0; 4]);
    let eaten = go(&MINIMAX, &[("radius", n(2.0)), ("operation", Value::Choice(0)), ("channel", Value::Choice(1))], solid.clone());
    assert_eq!(get(&eaten.img, 0, 5)[3], 0.0);
    let held = go(&MINIMAX, &[("radius", n(2.0)), ("operation", Value::Choice(0)), ("channel", Value::Choice(1)), ("dontShrinkEdges", Value::Bool(true))], solid);
    assert_eq!(get(&held.img, 0, 5)[3], 1.0);
    assert_eq!(line_extreme(&[3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0], 1, true, None), vec![3.0, 4.0, 4.0, 5.0, 9.0, 9.0, 9.0]);
}

#[test]
fn arithmetic_and_solids() {
    let img = Image::filled(4, 4, [0.5, 0.25, 0.75, 1.0]);
    let add = go(&ARITHMETIC, &[("redValue", n(51.0))], img.clone());
    assert!((add.img.px[0][0] - 0.7).abs() < 1e-5);
    let and = go(&ARITHMETIC, &[("operator", Value::Choice(0)), ("redValue", n(255.0)), ("greenValue", n(0.0)), ("blueValue", n(255.0))], img.clone());
    assert!((and.img.px[0][0] - 128.0 / 255.0).abs() < 1e-5 && and.img.px[0][1] == 0.0);
    let slice = go(&ARITHMETIC, &[("operator", Value::Choice(10)), ("redValue", n(100.0)), ("greenValue", n(100.0)), ("blueValue", n(100.0))], img.clone());
    assert_eq!(slice.img.px[0], [1.0, 0.0, 1.0, 1.0]);
    let mut half = Image::filled(4, 4, [0.0; 4]);
    half.px[0] = [1.0, 0.0, 0.0, 1.0];
    let solid = go(&SOLID, &[("color", Value::Color(veymelo_color::rgba(0.0, 0.0, 1.0, 1.0)))], half.clone());
    assert_eq!(solid.img.px[0], [1.0, 0.0, 0.0, 1.0], "the layer over the colour");
    assert_eq!(solid.img.px[5], [0.0, 0.0, 1.0, 1.0], "the colour where the layer is not");
    // matted on black, then unmatted: the colour comes back
    let edge = Image::filled(2, 2, [0.4 * 0.5, 0.2 * 0.5, 0.6 * 0.5, 0.5]);
    let mut matted = edge.clone();
    for p in &mut matted.px {
        let bg = [1.0, 1.0, 1.0];
        for c in 0..3 {
            p[c] = (p[c] / p[3] * p[3] + bg[c] * (1.0 - p[3])) * p[3];
        }
    }
    let un = go(&UNMATTE, &[("backgroundColor", Value::Color(veymelo_color::rgba(1.0, 1.0, 1.0, 1.0)))], matted);
    assert!(near(un.img.px[0], edge.px[0], 1e-5), "{:?} {:?}", un.img.px[0], edge.px[0]);
}

#[test]
fn mattes_and_blends() {
    let img = picture(16, 8);
    let m = go_with(&SET_MATTE, &[("takeMatteFromLayer", Value::Layer(Some(5))), ("useForMatte", Value::Choice(4))], img.clone(), Some(&Other));
    assert_eq!(get(&m.img, 2, 2)[3], 1.0);
    assert_eq!(get(&m.img, 13, 2)[3], 0.0);
    let flipped = go_with(&SET_MATTE, &[("takeMatteFromLayer", Value::Layer(Some(5))), ("useForMatte", Value::Choice(4)), ("invertMatte", Value::Bool(true))], img.clone(), Some(&Other));
    assert_eq!(get(&flipped.img, 13, 2)[3], 1.0);
    assert_eq!(go(&SET_MATTE, &[], img.clone()).img.px, img.px, "its own alpha: as it was");
    let cross = go_with(&BLEND, &[("blendWithLayer", Value::Layer(Some(5)))], img.clone(), Some(&Other));
    assert_eq!(get(&cross.img, 2, 2), [1.0; 4]);
    let half = go_with(&BLEND, &[("blendWithLayer", Value::Layer(Some(5))), ("blendWithOriginal", n(50.0))], img.clone(), Some(&Other));
    let o = get(&img, 13, 2);
    assert!(near(get(&half.img, 13, 2), [o[0] * 0.5, o[1] * 0.5, o[2] * 0.5, 1.0], 1e-6), "half way to black");
    let dark = go_with(&BLEND, &[("blendWithLayer", Value::Layer(Some(5))), ("mode", Value::Choice(3))], img.clone(), Some(&Other));
    assert_eq!(get(&dark.img, 13, 2)[..3], [0.0, 0.0, 0.0]);
    assert_eq!(get(&dark.img, 2, 2), get(&img, 2, 2));
    let calc = go_with(&CALCULATIONS, &[("secondLayer", Value::Layer(Some(5))), ("blendingMode", Value::Choice(2))], img.clone(), Some(&Other));
    assert_eq!(get(&calc.img, 13, 2)[..3], [0.0, 0.0, 0.0], "multiplied by black");
    let grey = go(&CALCULATIONS, &[("inputChannel", Value::Choice(2))], img.clone());
    let r = get(&img, 4, 2)[0];
    assert_eq!(get(&grey.img, 4, 2), [r, r, r, 1.0]);
    let comp = go_with(&COMPOUND, &[("secondSourceLayer", Value::Layer(Some(5))), ("operator", Value::Choice(1))], img.clone(), Some(&Other));
    assert_eq!(get(&comp.img, 2, 2)[..3], [1.0, 1.0, 1.0], "added to white: clipped");
    assert!(near(get(&comp.img, 13, 2), get(&img, 13, 2), 1e-6), "added to black: as it was");
    let scaled = go_with(&COMPOUND, &[("secondSourceLayer", Value::Layer(Some(5))), ("operator", Value::Choice(1)), ("overflowBehavior", Value::Choice(2))], img.clone(), Some(&Other));
    assert!((get(&scaled.img, 2, 2)[1] - (get(&img, 2, 2)[1] + 1.0) * 0.5).abs() < 1e-6);
}

#[test]
fn composite_brings_back_the_original() {
    let img = Image::filled(16, 8, [0.0, 0.0, 1.0, 0.5]);
    let front = go_with(&COMPOSITE, &[], img.clone(), Some(&Other));
    assert_eq!(front.img.px[0], [1.0, 0.0, 0.0, 1.0], "the original (red, solid) in front");
    let back = go_with(&COMPOSITE, &[("compositeOriginal", Value::Choice(1))], img.clone(), Some(&Other));
    assert!(near(back.img.px[0], [0.5, 0.0, 1.0, 1.0], 1e-6), "behind: {:?}", back.img.px[0]);
    let rgb_only = go_with(&COMPOSITE, &[("rgbOnly", Value::Bool(true))], img.clone(), Some(&Other));
    assert_eq!(rgb_only.img.px[0][3], 0.5);
    assert_eq!(go(&COMPOSITE, &[], img.clone()).img.px, img.px, "nothing to bring back without a renderer");
}

#[test]
fn every_one_is_registered() {
    for spec in ALL {
        assert!(crate::find(spec.def.id).is_some_and(|s| s.category == "channel"), "{}", spec.def.id);
    }
}

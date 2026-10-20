use super::*;
use crate::testing::{instance, n, run_when};
use veymelo_geom::Rect;
use veymelo_time::Tick;

const W: usize = 80;
const H: usize = 50;

fn go_at(spec: &Spec, values: &[(&str, Value)], img: Image, t: f64) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(img.width as f64, img.height as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::from_seconds(t), &[], None)
}

fn go(spec: &Spec, values: &[(&str, Value)], img: Image) -> Buf {
    go_at(spec, values, img, 0.0)
}

fn ramp() -> Image {
    let mut img = Image::new(W, H);
    for (i, p) in img.px.iter_mut().enumerate() {
        *p = [(i % W) as f32 / W as f32, (i / W) as f32 / H as f32, 0.5, 1.0];
    }
    img
}

fn clear() -> Image {
    Image::new(W, H)
}

fn t(s: &str) -> Value {
    Value::Text(s.into())
}

/// The box round what is drawn (alpha above a little): x0, y0, x1, y1.
fn extent(img: &Image) -> Option<[usize; 4]> {
    let mut e: Option<[usize; 4]> = None;
    for (i, p) in img.px.iter().enumerate() {
        if p[3] > 0.2 {
            let (x, y) = (i % img.width, i / img.width);
            e = Some(match e {
                None => [x, y, x, y],
                Some([a, b, c, d]) => [a.min(x), b.min(y), c.max(x), d.max(y)],
            });
        }
    }
    e
}

#[test]
fn the_old_gaussian() {
    let mut dot = Image::new(21, 21);
    dot.px[10 * 21 + 10] = [1.0; 4];
    assert_eq!(go(&GAUSSIAN, &[], dot.clone()).img.px, dot.px);
    let out = go(&GAUSSIAN, &[("blurriness", n(4.0))], dot.clone());
    assert!(out.img.width > 21, "room made to spread into");
    let total: f32 = out.img.px.iter().map(|p| p[3]).sum();
    assert!((total - 1.0).abs() < 1e-3, "{total}");
    let across = go(&GAUSSIAN, &[("blurriness", n(4.0)), ("blurDimensions", Value::Choice(1))], dot);
    let mid = across.img.height / 2;
    let column: f32 = (0..across.img.height).filter(|&y| y != mid).map(|y| across.img.px[y * across.img.width + across.img.width / 2][3]).sum();
    assert!(column < 1e-6, "across only");
}

#[test]
fn basic_3d_turns_the_layer() {
    assert_eq!(go(&BASIC_3D, &[], ramp()).img.px, ramp().px, "unturned: as it was");
    let swivelled = go(&BASIC_3D, &[("swivel", n(60.0))], ramp());
    assert_eq!(swivelled.img.px[25 * W][3], 0.0, "narrower: its sides gone");
    assert!(swivelled.img.px[25 * W + 40][3] > 0.99);
    let far = go(&BASIC_3D, &[("distanceToImage", n(30.0))], ramp());
    assert_eq!(far.img.px[0][3], 0.0, "farther: smaller");
    // turned toward the light (up and to the left, in front) it glints in a spot
    let toward = [("swivel", n(13.0)), ("tilt", n(-15.0))];
    let flat = go(&BASIC_3D, &toward, ramp());
    let shiny = go(&BASIC_3D, &[&toward[..], &[("specularHighlight", Value::Bool(true))]].concat(), ramp());
    let lit = shiny.img.px.iter().zip(&flat.img.px).filter(|(a, b)| a[1] - b[1] > 0.05).count();
    assert!(lit > 10 && lit < W * H / 2, "a glint: {lit}");
    let wire = go(&BASIC_3D, &[("drawPreviewWireframe", Value::Bool(true))], ramp());
    assert_eq!(wire.img.px[25 * W], [1.0; 4], "its edge drawn");
    // turned right round it shows its back, the other way about
    let back = go(&BASIC_3D, &[("swivel", n(180.0))], ramp());
    assert!(back.img.px[25 * W + 2][0] > 0.9, "{:?}", back.img.px[25 * W + 2]);
}

#[test]
fn basic_text_draws_lines() {
    let one = go(&BASIC_TEXT, &[("text", t("Hi")), ("compositeOnOriginal", Value::Bool(false))], clear());
    let [x0, y0, x1, y1] = extent(&one.img).expect("drawn");
    assert!(x0 < 40 && x1 > 40 && y0 < 25 && y1 > 25, "about its position: {:?}", (x0, y0, x1, y1));
    let red = one.img.px.iter().find(|p| p[3] > 0.99).unwrap();
    assert!(red[0] > 0.99 && red[1] < 0.01, "in the fill colour");
    let two = go(&BASIC_TEXT, &[("text", t("Hi\nHi")), ("compositeOnOriginal", Value::Bool(false)), ("size", n(20.0))], clear());
    let small = go(&BASIC_TEXT, &[("text", t("Hi")), ("compositeOnOriginal", Value::Bool(false)), ("size", n(20.0))], clear());
    let (a, b) = (extent(&two.img).unwrap(), extent(&small.img).unwrap());
    assert!(a[3] - a[1] > (b[3] - b[1]) * 2, "two lines stacked");
    let left = go(&BASIC_TEXT, &[("text", t("Hi")), ("alignment", Value::Choice(0)), ("compositeOnOriginal", Value::Bool(false))], clear());
    assert!(extent(&left.img).unwrap()[0] >= 39, "starting at its position");
    let over = go(&BASIC_TEXT, &[("text", t("Hi"))], ramp());
    assert_eq!(over.img.px[0], ramp().px[0], "over the layer");
}

#[test]
fn path_text_follows_its_path() {
    let base = [("text", t("ABCDEFG")), ("size", n(10.0)), ("compositeOnOriginal", Value::Bool(false)), ("shapeType", Value::Choice(3)), ("vertex1", Value::Vec2([5.0, 40.0])), ("vertex2", Value::Vec2([75.0, 40.0]))];
    let along = go(&PATH_TEXT, &base, clear());
    let [x0, y0, x1, y1] = extent(&along.img).expect("drawn");
    assert!(x0 >= 4 && x1 < 50 && y1 <= 41 && y0 >= 30, "on the line from its start: {:?}", (x0, y0, x1, y1));
    let right = go(&PATH_TEXT, &[&base[..], &[("alignment", Value::Choice(1))]].concat(), clear());
    assert!(extent(&right.img).unwrap()[2] >= 70, "to its end");
    let reversed = go(&PATH_TEXT, &[&base[..], &[("reversePath", Value::Bool(true))]].concat(), clear());
    let [_, ry0, rx1, _] = extent(&reversed.img).unwrap();
    assert!(rx1 >= 70 && ry0 >= 39, "from the other end, upside down below it: {:?}", extent(&reversed.img));
    // none shown, then half fading in
    let none = go(&PATH_TEXT, &[&base[..], &[("visibleCharacters", n(0.0))]].concat(), clear());
    assert!(extent(&none.img).is_none());
    let some = go(&PATH_TEXT, &[&base[..], &[("visibleCharacters", n(3.0))]].concat(), clear());
    assert!(extent(&some.img).unwrap()[2] < x1, "fewer letters");
    // round a circle
    let round = go(&PATH_TEXT, &[("text", t("ROUND AND ROUND")), ("size", n(8.0)), ("compositeOnOriginal", Value::Bool(false)), ("shapeType", Value::Choice(1)), ("vertex1", Value::Vec2([40.0, 25.0])), ("tangent1", Value::Vec2([60.0, 25.0]))], clear());
    let [cx0, cy0, cx1, cy1] = extent(&round.img).unwrap();
    assert!(cx1 > 55 && cy1 > 35 && cx0 > 8 && cx0 < 25 && cy0 < 15, "{:?}", (cx0, cy0, cx1, cy1));
    // jitter: new every frame
    let jittery = [&base[..], &[("rotationJitterMax", n(40.0))]].concat();
    let (f0, f1) = (go_at(&PATH_TEXT, &jittery, clear(), 0.0), go_at(&PATH_TEXT, &jittery, clear(), 1.0 / 30.0));
    assert_ne!(f0.img.px, f1.img.px);
    assert_eq!(f0.img.px, go_at(&PATH_TEXT, &jittery, clear(), 0.01).img.px, "the same within a frame");
}

#[test]
fn the_old_lightning() {
    let bolt = go(&LIGHTNING, &[], Image::new(W, H));
    let [x0, _, x1, _] = extent(&bolt.img).expect("drawn");
    assert!(x0 <= 9 && x1 >= 70, "from start to end: {x0} {x1}");
    let core = bolt.img.px.iter().filter(|p| p[3] > 0.9 && p[0] > 0.9 && p[1] > 0.9).count();
    assert!(core > 20, "a white core: {core}");
    // it crackles as time goes on, smoothly; rerun makes it new every frame
    let later = go_at(&LIGHTNING, &[], Image::new(W, H), 0.04);
    assert_ne!(later.img.px, bolt.img.px);
    let again = [("rerunAtEachFrame", Value::Bool(true))];
    assert_eq!(go_at(&LIGHTNING, &again, Image::new(W, H), 0.0).img.px, go_at(&LIGHTNING, &again, Image::new(W, H), 0.02).img.px, "within a frame the same");
    assert_ne!(go_at(&LIGHTNING, &again, Image::new(W, H), 0.0).img.px, go_at(&LIGHTNING, &again, Image::new(W, H), 0.04).img.px);
    // added onto the layer, it only brightens
    let grey = Image::filled(W, H, [0.4, 0.4, 0.4, 1.0]);
    let added = go(&LIGHTNING, &[("blendingMode", Value::Choice(1))], grey.clone());
    assert!(added.img.px.iter().zip(&grey.px).all(|(a, b)| a[0] >= b[0] - 1e-6));
}

#[test]
fn every_one_is_registered() {
    for id in ["basic3d", "basicText", "gaussianBlurLegacy", "lightning", "pathText", "colorKey", "lumaKey", "spillSuppressor", "reduceInterlaceFlicker"] {
        assert!(crate::find(id).is_some_and(|s| s.category == "obsolete"), "{id}");
    }
}


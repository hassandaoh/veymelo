use super::*;
use crate::testing::{instance, n, run_when};
use veymelo_geom::Rect;
use veymelo_time::Tick;

const W: usize = 40;
const H: usize = 20;

/// A render's passes: object 1 on the left at depth 100, object 2 on the right at depth 500, the
/// top right corner nothing; column 20 half each (Cryptomatte "left" and "right").
struct Render(Arc<Passes>);

impl crate::Host for Render {
    fn layer(&self, _: u64, _: Take) -> Option<(Buf, Rect)> {
        None
    }

    fn passes(&self) -> Option<Arc<Passes>> {
        Some(self.0.clone())
    }
}

fn render() -> Render {
    let mut p = Passes::new(W, H, 1.0);
    let n = W * H;
    let (mut z, mut id, mut nx, mut cov) = (vec![0f32; n], vec![0f32; n], vec![0f32; n], vec![1f32; n]);
    let (left, right) = (crypto_id("left"), crypto_id("right"));
    let (mut c0, mut a0, mut c1, mut a1) = (vec![0f32; n], vec![0f32; n], vec![0f32; n], vec![0f32; n]);
    for y in 0..H {
        for x in 0..W {
            let i = y * W + x;
            if x >= 30 && y < 6 {
                z[i] = FAR;
            } else if x < 20 {
                (z[i], id[i], nx[i], c0[i], a0[i]) = (100.0, 1.0, -1.0, f32::from_bits(left), 1.0);
            } else {
                (z[i], id[i], nx[i], c0[i], a0[i]) = (500.0, 2.0, 1.0, f32::from_bits(right), 1.0);
                if x == 20 {
                    // the edge: half of each
                    (a0[i], c1[i], a1[i], cov[i]) = (0.5, f32::from_bits(left), 0.5, 0.5);
                }
            }
        }
    }
    p.channels = vec![
        ("Z".into(), z),
        ("ObjectID".into(), id),
        ("N.X".into(), nx.clone()),
        ("N.Y".into(), vec![0.0; n]),
        ("N.Z".into(), vec![0.0; n]),
        ("Coverage".into(), cov),
        ("CryptoObject00.R".into(), c0),
        ("CryptoObject00.G".into(), a0),
        ("CryptoObject00.B".into(), c1),
        ("CryptoObject00.A".into(), a1),
    ];
    p.manifests = vec![("CryptoObject".into(), vec![("left".into(), left), ("right".into(), right)])];
    Render(Arc::new(p))
}

/// Stripes down the layer, every other column light.
fn stripes() -> Image {
    let mut img = Image::new(W, H);
    for (i, p) in img.px.iter_mut().enumerate() {
        let v = if (i % W).is_multiple_of(2) { 0.9 } else { 0.1 };
        *p = [v, v * 0.5, 0.3, 1.0];
    }
    img
}

fn go(spec: &Spec, values: &[(&str, Value)], img: Image, host: Option<&dyn crate::Host>) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(W as f64, H as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::ZERO, &[], host)
}

fn grey() -> Image {
    Image::filled(W, H, [0.5, 0.5, 0.5, 1.0])
}

fn at(b: &Buf, x: usize, y: usize) -> Px {
    b.img.px[y * W + x]
}

#[test]
fn without_passes_the_layer_is_left() {
    for spec in ALL {
        let out = go(&spec, &[], stripes(), None);
        assert_eq!(out.img.px, stripes().px, "{}", spec.def.id);
    }
}

#[test]
fn a_pass_shown_as_a_picture() {
    let r = render();
    let depth = go(&EXTRACT, &[("whitePoint", n(1000.0))], grey(), Some(&r));
    assert!((at(&depth, 5, 10)[0] - 0.1).abs() < 1e-6 && (at(&depth, 25, 10)[0] - 0.5).abs() < 1e-6);
    assert_eq!(at(&depth, 35, 2), [1.0; 4], "nothing there: as far as can be, held at white");
    let inverted = go(&EXTRACT, &[("whitePoint", n(1000.0)), ("invertDepthMap", Value::Bool(true))], grey(), Some(&r));
    assert!((at(&inverted, 5, 10)[0] - 0.9).abs() < 1e-6);
    let ids = go(&EXTRACT, &[("channel", Value::Choice(1)), ("whitePoint", n(2.0))], grey(), Some(&r));
    assert!((at(&ids, 5, 10)[0] - 0.5).abs() < 1e-6 && (at(&ids, 25, 10)[0] - 1.0).abs() < 1e-6);
    let normals = go(&EXTRACT, &[("channel", Value::Choice(3)), ("whitePoint", n(1.0))], grey(), Some(&r));
    assert!(at(&normals, 5, 10)[0] < 1e-6 && (at(&normals, 25, 10)[0] - 1.0).abs() < 1e-6 && (at(&normals, 25, 10)[1] - 0.5).abs() < 1e-6);
    // smooth edges: four points a pixel
    let smooth = go(&EXTRACT, &[("whitePoint", n(1000.0)), ("antialias", Value::Bool(true))], grey(), Some(&r));
    assert!((at(&smooth, 5, 10)[0] - 0.1).abs() < 1e-6);
    // a missing pass leaves the layer
    let none = go(&EXTRACT, &[("channel", Value::Choice(2))], grey(), Some(&r));
    assert_eq!(none.img.px, grey().px);
}

#[test]
fn channels_put_anywhere() {
    let r = render();
    assert_eq!(go(&EXTRACTOR, &[], stripes(), Some(&r)).img.px, stripes().px, "each channel its own");
    let out = go(&EXTRACTOR, &[("red", Value::Text("Z".into())), ("green", Value::Text("ObjectID".into())), ("whitePoint", n(1000.0))], stripes(), Some(&r));
    let p = at(&out, 5, 10);
    assert!((p[0] - 0.1).abs() < 1e-6 && (p[1] - 0.001).abs() < 1e-6 && (p[2] - 0.0003).abs() < 1e-6, "{p:?}");
    let clipped = go(&EXTRACTOR, &[("red", Value::Text("Z".into())), ("whitePoint", n(10.0)), ("clip", Value::Bool(true))], stripes(), Some(&r));
    assert_eq!(at(&clipped, 5, 10)[0], 1.0);
}

#[test]
fn cut_by_depth() {
    let r = render();
    let far = go(&DEPTH_MATTE, &[("depth", n(300.0))], grey(), Some(&r));
    assert_eq!(at(&far, 5, 10)[3], 0.0);
    assert_eq!(at(&far, 25, 10)[3], 1.0);
    let near = go(&DEPTH_MATTE, &[("depth", n(300.0)), ("invert", Value::Bool(true))], grey(), Some(&r));
    assert_eq!(at(&near, 5, 10)[3], 1.0);
    assert_eq!(at(&near, 25, 10)[3], 0.0);
    let soft = go(&DEPTH_MATTE, &[("depth", n(300.0)), ("feather", n(800.0))], grey(), Some(&r));
    assert!((at(&soft, 5, 10)[3] - 0.25).abs() < 1e-6 && (at(&soft, 25, 10)[3] - 0.75).abs() < 1e-6);
}

/// How much the colour changes from column to column over columns `xs` of row 12.
fn ripple(b: &Buf, xs: std::ops::Range<usize>) -> f32 {
    xs.map(|x| (at(b, x + 1, 12)[0] - at(b, x, 12)[0]).abs()).sum()
}

#[test]
fn depth_of_field() {
    let r = render();
    // focused on the left: it stays sharp, the right (far behind) blurs
    let left = go(&FOCUS, &[("focalPlane", n(100.0)), ("maximumRadius", n(6.0))], stripes(), Some(&r));
    assert!((ripple(&left, 2..16) - ripple(&go(&FOCUS, &[], stripes(), Some(&r)), 2..16)).abs() < 1e-4, "in focus: sharp");
    assert!(ripple(&left, 24..38) < ripple(&stripes_buf(), 24..38) * 0.3, "out of focus: smooth");
    assert!(left.img.px.iter().all(|p| (p[3] - 1.0).abs() < 1e-3), "no holes");
    // focused on the right: the left is in front and blurs over the right's near edge, but the
    // right away from it stays sharp
    let right = go(&FOCUS, &[("focalPlane", n(500.0)), ("maximumRadius", n(6.0)), ("focalPlaneThickness", n(10.0))], stripes(), Some(&r));
    assert!(ripple(&right, 2..14) < ripple(&stripes_buf(), 2..14) * 0.5, "the near side blurred");
    assert!((ripple(&right, 30..38) - ripple(&stripes_buf(), 30..38)).abs() < 1e-3, "the far side sharp");
    assert!(right.img.px.iter().all(|p| (p[3] - 1.0).abs() < 1e-3), "no holes behind the near edge");
}

fn stripes_buf() -> Buf {
    crate::testing::buf(stripes())
}

#[test]
fn fog_by_depth() {
    let r = render();
    let fogged = go(&FOG, &[("fogEndDepth", n(1000.0)), ("scatteringDensity", n(0.0)), ("fogColor", Value::Color(veymelo_color::Rgba::WHITE))], Image::filled(W, H, [0.0, 0.0, 0.0, 1.0]), Some(&r));
    assert!((at(&fogged, 5, 10)[0] - 0.1).abs() < 1e-5 && (at(&fogged, 25, 10)[0] - 0.5).abs() < 1e-5);
    assert!((at(&fogged, 35, 2)[0] - 1.0).abs() < 1e-5, "the background all fog");
    let clear = go(&FOG, &[("fogEndDepth", n(1000.0)), ("foggyBackground", Value::Bool(false))], Image::filled(W, H, [0.0, 0.0, 0.0, 1.0]), Some(&r));
    assert_eq!(at(&clear, 35, 2)[0], 0.0);
}

#[test]
fn cut_by_number() {
    let r = render();
    let two = go(&ID_MATTE, &[("idSelection", n(2.0))], grey(), Some(&r));
    assert_eq!(at(&two, 5, 10)[3], 0.0);
    assert_eq!(at(&two, 25, 10)[3], 1.0);
    let covered = go(&ID_MATTE, &[("idSelection", n(2.0)), ("useCoverage", Value::Bool(true))], grey(), Some(&r));
    assert!((at(&covered, 20, 10)[3] - 0.5).abs() < 1e-6, "the edge as much as it covers");
    let inverted = go(&ID_MATTE, &[("idSelection", n(2.0)), ("invert", Value::Bool(true))], grey(), Some(&r));
    assert_eq!(at(&inverted, 5, 10)[3], 1.0);
}

#[test]
fn numbers_shown() {
    let r = render();
    let colours = go(&ID_VIEWER, &[], grey(), Some(&r));
    let (a, b) = (at(&colours, 5, 10), at(&colours, 25, 10));
    assert!(a != b && a[3] == 1.0 && at(&colours, 35, 2) == [0.0, 0.0, 0.0, 1.0], "each its own colour, nothing black");
    let picked = go(&ID_VIEWER, &[("id", n(1.0))], grey(), Some(&r));
    assert_eq!(at(&picked, 5, 10), [1.0; 4]);
    let matte = go(&ID_VIEWER, &[("id", n(2.0)), ("display", Value::Choice(1))], grey(), Some(&r));
    assert_eq!((at(&matte, 5, 10)[0], at(&matte, 25, 10)[0]), (0.0, 1.0));
    let raw = go(&ID_VIEWER, &[("display", Value::Choice(3))], grey(), Some(&r));
    assert_eq!((at(&raw, 5, 10)[0], at(&raw, 25, 10)[0]), (0.5, 1.0));
}

#[test]
fn cryptomatte_by_name() {
    let r = render();
    let only = |sel: &str| go(&CRYPTO, &[("selection", Value::Text(sel.into())), ("matteOnly", Value::Bool(true))], grey(), Some(&r));
    let right = only("right");
    assert_eq!((at(&right, 5, 10)[0], at(&right, 25, 10)[0]), (0.0, 1.0));
    assert!((at(&right, 20, 10)[0] - 0.5).abs() < 1e-6, "the edge, as the render saw it");
    let both = only("left, right");
    assert_eq!(at(&both, 20, 10)[0], 1.0);
    assert_eq!(at(&only("r*").clone(), 25, 10)[0], 1.0, "a pattern");
    assert_eq!(at(&only(&format!("<{:08x}>", crypto_id("left"))), 5, 10)[0], 1.0, "an ID");
    let keyed = go(&CRYPTO, &[("selection", Value::Text("left".into())), ("display", Value::Choice(1))], grey(), Some(&r));
    assert_eq!(at(&keyed, 25, 10)[3], 0.0);
    assert_eq!(at(&keyed, 5, 10), [0.5, 0.5, 0.5, 1.0]);
    assert_eq!(names_in(r#" "Green, Glossy", a\,b ,c "#), vec!["Green, Glossy", "a,b", "c"]);
}

#[test]
fn every_one_is_registered() {
    for id in ["channelExtract3d", "cryptomatte", "depthMatte", "depthOfField", "channelExtractor", "fog3d", "idMatte", "idViewer"] {
        assert!(crate::find(id).is_some_and(|s| s.category == "3dChannel"), "{id}");
    }
}


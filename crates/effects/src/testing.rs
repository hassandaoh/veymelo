//! For effects' tests: an instance with some settings, run on a picture.

use veymelo_geom::{Rect, vec2};
use veymelo_keyframe::Value;
use veymelo_project::Group;
use veymelo_raster::Image;
use veymelo_time::Tick;

use crate::{Buf, Ctx, Host, Spec};

/// An instance of `spec` with `values` set (by path).
pub fn instance(spec: &Spec, values: &[(&str, Value)]) -> Group {
    let mut g = Group::instance(spec.def, 1, spec.def.name);
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap_or_else(|e| panic!("{path}: {e}"));
    }
    g
}

/// The picture at one pixel to a unit from the layer's origin.
pub fn buf(img: Image) -> Buf {
    Buf { img, origin: vec2(0.0, 0.0), scale: 1.0 }
}

/// `spec` with `values` run on `b`, the layer's rectangle being `bounds`.
pub fn run_on(spec: &Spec, values: &[(&str, Value)], b: Buf, bounds: Rect, host: Option<&dyn Host>) -> Buf {
    let g = instance(spec, values);
    let read = |p: &veymelo_project::Property| p.value_at(Tick::ZERO);
    let mut ctx = Ctx::new(&g, &read, Tick::ZERO, bounds, 7);
    ctx.host = host;
    (spec.run)(&ctx, b)
}

/// `spec` with `values` run on a picture that is the whole layer.
pub fn run(spec: &Spec, values: &[(&str, Value)], img: Image) -> Buf {
    let bounds = Rect::from_size(img.width as f64, img.height as f64);
    run_on(spec, values, buf(img), bounds, None)
}

/// The pixel at a point of the layer.
pub fn at(b: &Buf, x: f64, y: f64) -> [f32; 4] {
    let p = b.pixel(vec2(x, y));
    b.img.get(p.x.floor() as i64, p.y.floor() as i64)
}

pub fn n(v: f64) -> Value {
    Value::Number(v)
}

/// A picture `w` × `h`: dark grey on the left half, light grey on the right.
pub fn halves(w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let v = if x < w / 2 { 0.1 } else { 0.9 };
            img.px[y * w + x] = [v, v, v, 1.0];
        }
    }
    img
}

/// `spec` with `values` run on a picture that is the whole layer, which has `masks`.
pub fn run_masked(spec: &Spec, values: &[(&str, Value)], img: Image, masks: &[crate::MaskShape]) -> Buf {
    let g = instance(spec, values);
    let read = |p: &veymelo_project::Property| p.value_at(Tick::ZERO);
    let bounds = Rect::from_size(img.width as f64, img.height as f64);
    let mut ctx = Ctx::new(&g, &read, Tick::ZERO, bounds, 7);
    ctx.masks = masks;
    (spec.run)(&ctx, buf(img))
}

/// A closed square mask from (x, y), `s` on a side.
pub fn square(x: f64, y: f64, s: f64) -> crate::MaskShape {
    crate::MaskShape { name: "Mask 1".into(), points: vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]], closed: true, inverted: false }
}

/// `spec` set up as `g` run at layer time `t` on a picture that is the whole layer (its
/// settings read at any time from their keys), with `masks` and `host`.
pub fn run_when(spec: &Spec, g: &Group, img: Image, t: Tick, masks: &[crate::MaskShape], host: Option<&dyn Host>) -> Buf {
    let read = |p: &veymelo_project::Property| p.value_at(t);
    let read_at = |p: &veymelo_project::Property, t: Tick| p.value_at(t);
    let bounds = Rect::from_size(img.width as f64, img.height as f64);
    let mut ctx = Ctx::new(g, &read, t, bounds, 7);
    ctx.read_at = Some(&read_at);
    ctx.masks = masks;
    ctx.host = host;
    (spec.run)(&ctx, buf(img))
}

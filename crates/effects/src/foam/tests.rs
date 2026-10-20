use super::*;
use crate::testing::{instance, n, run_when};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;

const W: usize = 64;
const H: usize = 48;

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

/// Still air: no wind, no stirring, no wobble.
fn still(more: &[(&'static str, Value)]) -> Vec<(&'static str, Value)> {
    let mut v = vec![("windSpeed", n(0.0)), ("turbulence", n(0.0)), ("wobbleAmount", n(0.0))];
    v.extend(more.iter().cloned());
    v
}

fn go(values: &[(&str, Value)], t: f64, host: Option<&dyn crate::Host>) -> Buf {
    let mut g = instance(&FOAM, &[]);
    FOAM.start(&mut g, Rect::from_size(W as f64, H as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(&FOAM, &g, Image::filled(W, H, [0.3, 0.6, 0.9, 1.0]), Tick::from_seconds(t), &[], host)
}

/// Where the drawing is, on average (alpha weighted), and how much of it there is.
fn middle(b: &Buf) -> ([f32; 2], f32) {
    let (mut sx, mut sy, mut sa) = (0f32, 0f32, 0f32);
    for (i, p) in b.img.px.iter().enumerate() {
        sx += (i % W) as f32 * p[3];
        sy += (i / W) as f32 * p[3];
        sa += p[3];
    }
    ([sx / sa.max(1e-6), sy / sa.max(1e-6)], sa)
}

const RENDERED: (&str, Value) = ("view", Value::Choice(2));
const MILK: (&str, Value) = ("bubbleTexture", Value::Choice(4));

#[test]
fn nothing_before_the_first_bubble() {
    let out = go(&[RENDERED], 0.0, None);
    assert!(out.img.px.iter().all(|p| p[3] == 0.0), "the layer is replaced by the bubbles");
    // a draft shows where they will come from
    let draft = go(&[], 0.0, None);
    assert!(draft.img.px[24 * W + 32][0] > 0.9 && draft.img.px[24 * W + 32][3] == 1.0, "the producer");
}

#[test]
fn the_wind_carries_them() {
    let right = go(&still(&[RENDERED, MILK, ("windSpeed", n(1.0)), ("windDirection", n(90.0))]), 1.5, None);
    let down = go(&still(&[RENDERED, MILK, ("windSpeed", n(1.0)), ("windDirection", n(180.0))]), 1.5, None);
    let calm = go(&still(&[RENDERED, MILK]), 1.5, None);
    let ((r, _), (d, _), (c, amount)) = (middle(&right), middle(&down), middle(&calm));
    assert!(amount > 10.0, "bubbles: {amount}");
    assert!(r[0] > c[0] + 5.0 && (r[1] - c[1]).abs() < 3.0, "{r:?} {c:?}");
    assert!(d[1] > c[1] + 5.0 && (d[0] - c[0]).abs() < 3.0, "{d:?} {c:?}");
}

#[test]
fn they_pop_when_their_time_is_up() {
    let wind = [RENDERED, MILK, ("windSpeed", n(1.0)), ("strength", n(100.0))];
    // in half a second they go 0.15 widths (10 pixels), then pop
    let short = go(&still(&[wind.as_slice(), &[("lifespan", n(15.0))]].concat()), 3.0, None);
    let long = go(&still(&wind), 3.0, None);
    let far = |b: &Buf| b.img.px.iter().enumerate().filter(|(i, p)| i % W > 50 && p[3] > 0.1).count();
    assert_eq!(far(&short), 0);
    assert!(far(&long) > 20, "{}", far(&long));
    // and popping pushes the others away
    let pushed = go(&still(&[RENDERED, MILK, ("lifespan", n(15.0)), ("strength", n(0.0)), ("popVelocity", n(20.0))]), 2.0, None);
    let kept = go(&still(&[RENDERED, MILK, ("lifespan", n(15.0)), ("strength", n(0.0)), ("popVelocity", n(0.0))]), 2.0, None);
    let spread = |b: &Buf| b.img.px.iter().filter(|p| p[3] > 0.1).count();
    assert!(spread(&pushed) > spread(&kept), "{} {}", spread(&pushed), spread(&kept));
}

#[test]
fn bubbles_push_apart() {
    let crowd = [RENDERED, MILK, ("productionRate", n(4.0)), ("producerXSize", n(0.0)), ("producerYSize", n(0.0)), ("size", n(2.0)), ("sizeVariance", n(0.0))];
    let piled = go(&still(&[crowd.as_slice(), &[("repulsion", n(0.0))]].concat()), 1.0, None);
    let apart = go(&still(&[crowd.as_slice(), &[("repulsion", n(3.0))]].concat()), 1.0, None);
    let spread = |b: &Buf| b.img.px.iter().filter(|p| p[3] > 0.5).count();
    assert!(spread(&apart) > spread(&piled) * 4, "{} {}", spread(&apart), spread(&piled));
}

/// One big bubble, grown at once, blown at 0.83 seconds.
fn one(more: &[(&'static str, Value)]) -> Vec<(&'static str, Value)> {
    still(&[&[("productionRate", n(0.04)), ("producerXSize", n(0.0)), ("producerYSize", n(0.0)), ("size", n(5.0)), ("sizeVariance", n(0.0)), ("growthSpeed", n(10.0))][..], more].concat())
}

#[test]
fn a_draft_outlines_them() {
    let out = go(&one(&[]), 1.0, None);
    // 15 hundredths of the width across: 9.6 pixels
    let row: Vec<f32> = (0..W).map(|x| out.img.px[24 * W + x][3]).collect();
    assert!(row[32 + 8] > 0.4 && row[32 - 9] > 0.4, "{row:?}");
    // empty inside (off the producer's cross) and out
    assert!(out.img.px[20 * W + 37][3] < 0.05 && row[32 + 13] < 0.05, "{row:?}");
}

#[test]
fn rendered_bubbles_take_their_look() {
    let milk = go(&one(&[RENDERED, MILK]), 1.0, None);
    let p = milk.img.px[24 * W + 30];
    assert!(p[3] > 0.99 && p[0] > 0.6, "a white ball: {p:?}");
    let soap = go(&one(&[RENDERED]), 1.0, None);
    let (mid, rim) = (soap.img.px[24 * W + 34], soap.img.px[24 * W + 32 + 9]);
    assert!(mid[3] < 0.3 && rim[3] > mid[3] + 0.2, "clear in the middle, seen at its rim: {mid:?} {rim:?}");
    // solid: what is under it hidden
    let solid = go(&one(&[RENDERED, ("blendMode", Value::Choice(1))]), 1.0, None);
    assert!(solid.img.px[24 * W + 34][3] > 0.99);
    // a layer of one's own on each bubble
    let host = Layers(vec![(5, Image::filled(8, 8, [1.0, 0.0, 0.0, 1.0]))]);
    let own = go(&one(&[RENDERED, ("bubbleTexture", Value::Choice(5)), ("bubbleTextureLayer", Value::Layer(Some(5)))]), 1.0, Some(&host));
    let p = own.img.px[24 * W + 33];
    assert!(p[0] > 0.99 && p[1] < 0.01 && p[3] > 0.99, "{p:?}");
}

#[test]
fn zoom_makes_them_bigger() {
    let near = go(&one(&[RENDERED, MILK, ("zoom", n(2.0))]), 1.0, None);
    let row: Vec<f32> = (0..W).map(|x| near.img.px[24 * W + x][3]).collect();
    assert!(row[32 + 17] > 0.9 && row[32 - 17] > 0.9, "twice as wide: {row:?}");
}

#[test]
fn they_run_down_the_flow_map() {
    // light on the left, dark on the right: downhill to the right
    let mut slope = Image::new(W, H);
    for (i, p) in slope.px.iter_mut().enumerate() {
        let v = 1.0 - (i % W) as f32 / W as f32;
        *p = [v, v, v, 1.0];
    }
    let host = Layers(vec![(3, slope)]);
    let flowing = go(&still(&[RENDERED, MILK, ("flowMap", Value::Layer(Some(3))), ("flowMapSteepness", n(0.2))]), 1.5, Some(&host));
    let calm = go(&still(&[RENDERED, MILK]), 1.5, None);
    assert!(middle(&flowing).0[0] > middle(&calm).0[0] + 4.0, "{:?} {:?}", middle(&flowing), middle(&calm));
}

#[test]
fn every_one_is_registered() {
    assert!(crate::find("foam").is_some_and(|s| s.category == "simulation"));
}

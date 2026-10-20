//! The layer as a grid of cards in 3D: moved, turned and scaled by the colours of one or two
//! gradient layers (Card Dance), or flipping over one after another to show another layer on
//! their backs (Card Wipe).

use veymelo_raster::Image;

use crate::defs::{choice, flag, layer, pct, ranged};
use crate::scene3d::{self, Camera, Light, Piece, scene_effect};
use crate::sprites::{rand, rand2};
use crate::util::{luma, unpremul};
use crate::{Buf, Ctx, Spec, Take};

/// Where a card value can come from.
const SOURCES: &[&str] = &["none", "intensity1", "red1", "green1", "blue1", "alpha1", "intensity2", "red2", "green2", "blue2", "alpha2"];

/// A gradient source's value (0 to 1) from the two gradient pixels (not premultiplied).
fn source(which: u32, g1: [f32; 4], g2: [f32; 4]) -> f32 {
    if which == 0 {
        return 0.0;
    }
    let g = if which >= 6 { g2 } else { g1 };
    match (which - 1) % 5 {
        0 => luma([g[0], g[1], g[2]]),
        1 => g[0],
        2 => g[1],
        3 => g[2],
        _ => g[3],
    }
}

/// A gradient layer setting's picture over this layer (stretched), or the layer itself.
fn gradient(ctx: &Ctx, b: &Buf, id: &str) -> Image {
    ctx.layer_over(id, b, Take::All, true).unwrap_or_else(|| b.img.clone())
}

/// A pixel as it is (not premultiplied) and its alpha.
fn straight(img: &Image, x: f64, y: f64) -> [f32; 4] {
    let (c, a) = unpremul(img.sample(veymelo_raster::Sampling::Bilinear, x, y));
    [c[0], c[1], c[2], a]
}

/// The grid: rows and columns (the columns following the rows when asked).
fn grid(ctx: &Ctx) -> (u32, u32) {
    let rows = ctx.num("rows", 10.0).clamp(1.0, 1000.0) as u32;
    let cols = if ctx.choice("rowsColumns") == 1 { rows } else { ctx.num("columns", 10.0).clamp(1.0, 1000.0) as u32 };
    (rows, cols)
}

/// Card `i` of the grid: its outline in the picture's pixels and its middle.
fn card(b: &Buf, r: veymelo_geom::Rect, rows: u32, cols: u32, i: u32) -> (Vec<[f32; 2]>, [f64; 2], [f64; 2]) {
    let (cw, ch) = (r.width() / cols as f64, r.height() / rows as f64);
    let (x0, y0) = (r.x0 + (i % cols) as f64 * cw, r.y0 + (i / cols) as f64 * ch);
    let a = b.pixel(veymelo_geom::vec2(x0, y0));
    let z = b.pixel(veymelo_geom::vec2(x0 + cw, y0 + ch));
    let outline = vec![[a.x as f32, a.y as f32], [z.x as f32, a.y as f32], [z.x as f32, z.y as f32], [a.x as f32, z.y as f32]];
    (outline, [(a.x + z.x) * 0.5, (a.y + z.y) * 0.5], [z.x - a.x, z.y - a.y])
}

/// The card value channels: name stem and what it starts as.
const CHANNELS: [(&str, f64); 8] = [("xPosition", 0.0), ("yPosition", 0.0), ("zPosition", 0.0), ("xRotation", 0.0), ("yRotation", 0.0), ("zRotation", 0.0), ("xScale", 1.0), ("yScale", 1.0)];

scene_effect!(
    DANCE_DEF,
    "cardDance",
    "Card Dance",
    "the layer cut into cards that move, turn and grow in 3D by the colours of gradient layers",
    z: 2.0, light: 1.0, ambient: 0.25, diffuse: 0.75,
    [
        choice("rowsColumns", "Rows & Columns", &["independent", "columnsFollowsRows"], 0),
        ranged("rows", "Rows", 10.0, 1.0, 1000.0),
        ranged("columns", "Columns", 10.0, 1.0, 1000.0),
        layer("backLayer", "Back Layer").doc("on the cards' backs; none: the layer"),
        layer("gradientLayer1", "Gradient Layer 1").doc("none: the layer"),
        layer("gradientLayer2", "Gradient Layer 2").doc("none: the layer"),
        choice("xPositionSource", "X Position Source", SOURCES, 0),
        ranged("xPositionMultiplier", "X Position Multiplier", 0.0, -100.0, 100.0),
        ranged("xPositionOffset", "X Position Offset", 0.0, -100.0, 100.0),
        choice("yPositionSource", "Y Position Source", SOURCES, 0),
        ranged("yPositionMultiplier", "Y Position Multiplier", 0.0, -100.0, 100.0),
        ranged("yPositionOffset", "Y Position Offset", 0.0, -100.0, 100.0),
        choice("zPositionSource", "Z Position Source", SOURCES, 0),
        ranged("zPositionMultiplier", "Z Position Multiplier", 0.0, -100.0, 100.0),
        ranged("zPositionOffset", "Z Position Offset", 0.0, -100.0, 100.0),
        choice("xRotationSource", "X Rotation Source", SOURCES, 0),
        ranged("xRotationMultiplier", "X Rotation Multiplier", 0.0, -3600.0, 3600.0),
        ranged("xRotationOffset", "X Rotation Offset", 0.0, -3600.0, 3600.0),
        choice("yRotationSource", "Y Rotation Source", SOURCES, 0),
        ranged("yRotationMultiplier", "Y Rotation Multiplier", 0.0, -3600.0, 3600.0),
        ranged("yRotationOffset", "Y Rotation Offset", 0.0, -3600.0, 3600.0),
        choice("zRotationSource", "Z Rotation Source", SOURCES, 0),
        ranged("zRotationMultiplier", "Z Rotation Multiplier", 0.0, -3600.0, 3600.0),
        ranged("zRotationOffset", "Z Rotation Offset", 0.0, -3600.0, 3600.0),
        choice("xScaleSource", "X Scale Source", SOURCES, 0),
        ranged("xScaleMultiplier", "X Scale Multiplier", 0.0, -100.0, 100.0),
        ranged("xScaleOffset", "X Scale Offset", 1.0, -100.0, 100.0),
        choice("yScaleSource", "Y Scale Source", SOURCES, 0),
        ranged("yScaleMultiplier", "Y Scale Multiplier", 0.0, -100.0, 100.0),
        ranged("yScaleOffset", "Y Scale Offset", 1.0, -100.0, 100.0),
    ]
);
pub const DANCE: Spec = Spec::new(&DANCE_DEF, "simulation", card_dance).points(&scene3d::POINTS);

fn card_dance(ctx: &Ctx, mut b: Buf) -> Buf {
    let (rows, cols) = grid(ctx);
    let (g1, g2) = (gradient(ctx, &b, "gradientLayer1"), gradient(ctx, &b, "gradientLayer2"));
    let back = ctx.layer_over("backLayer", &b, Take::All, true);
    let r = ctx.bounds;
    let cam = Camera::of(ctx, &b);
    let (eye, light) = (cam.eye(), Light::of(ctx, &b));
    let chans: Vec<(u32, f64, f64)> = CHANNELS.iter().map(|(stem, d)| (ctx.choice(&format!("{stem}Source")), ctx.num(&format!("{stem}Multiplier"), 0.0), ctx.num(&format!("{stem}Offset"), *d))).collect();
    let mut placed: Vec<_> = (0..rows * cols)
        .filter_map(|i| {
            let (outline, mid, size) = card(&b, r, rows, cols, i);
            let (p1, p2) = (straight(&g1, mid[0], mid[1]), straight(&g2, mid[0], mid[1]));
            let val = |k: usize| {
                let (src, m, o) = chans[k];
                o + m * source(src, p1, p2) as f64
            };
            let (sx, sy) = (val(6), val(7));
            let t = scene3d::turning(val(3).to_radians(), val(4).to_radians(), val(5).to_radians(), 0);
            let turn = [0, 1, 2].map(|k| [t[k][0] * sx, t[k][1] * sy, t[k][2]]);
            // positions in cards: one is a card's width (or height) along
            let at = [mid[0] + val(0) * size[0], mid[1] + val(1) * size[1], val(2) * size[0].max(size[1])];
            let piece = Piece { outline, rest: [mid[0] as f32, mid[1] as f32], turn, at, alpha: 1.0, flat: None, mirror: 0 };
            Some(piece.place(&cam)?.lit(&light, eye))
        })
        .collect();
    let mut out = Image::new(b.img.width, b.img.height);
    scene3d::draw(&mut out, &mut placed, &b.img, back.as_ref());
    b.img = out;
    b
}

const ORDERS: &[&str] = &["leftToRight", "rightToLeft", "topToBottom", "bottomToTop", "topLeftToBottomRight", "topRightToBottomLeft", "bottomLeftToTopRight", "bottomRightToTopLeft", "gradient"];

scene_effect!(
    WIPE_DEF,
    "cardWipe",
    "Card Wipe",
    "the layer as cards flipping over one after another, another layer on their backs",
    z: 2.0, light: 1.0, ambient: 0.25, diffuse: 0.75,
    [
        pct("transitionCompletion", "Transition Completion", 0.0, 0.0, 100.0),
        pct("transitionWidth", "Transition Width", 50.0, 0.0, 100.0).doc("how many cards are turning at once"),
        layer("backLayer", "Back Layer").doc("on the cards' backs; none: the layer"),
        choice("rowsColumns", "Rows & Columns", &["independent", "columnsFollowsRows"], 0),
        ranged("rows", "Rows", 1.0, 1.0, 1000.0),
        ranged("columns", "Columns", 10.0, 1.0, 1000.0),
        ranged("cardScale", "Card Scale", 1.0, -10.0, 10.0),
        choice("flipAxis", "Flip Axis", &["x", "y", "random"], 0),
        choice("flipDirection", "Flip Direction", &["positive", "negative", "random"], 0),
        choice("flipOrder", "Flip Order", ORDERS, 0),
        layer("gradientLayer", "Gradient Layer").doc("for the gradient order: dark cards first; none: the layer"),
        ranged("timingRandomness", "Timing Randomness", 0.0, 0.0, 1.0),
        ranged("randomSeed", "Random Seed", 1.0, 0.0, 10_000.0),
        flag("positionJitter", "Position Jitter", false),
        ranged("xJitterAmount", "X Jitter Amount", 0.0, 0.0, 100.0),
        ranged("xJitterSpeed", "X Jitter Speed", 1.0, 0.0, 100.0),
        ranged("yJitterAmount", "Y Jitter Amount", 0.0, 0.0, 100.0),
        ranged("yJitterSpeed", "Y Jitter Speed", 1.0, 0.0, 100.0),
        ranged("zJitterAmount", "Z Jitter Amount", 0.0, 0.0, 100.0),
        ranged("zJitterSpeed", "Z Jitter Speed", 1.0, 0.0, 100.0),
        flag("rotationJitter", "Rotation Jitter", false),
        ranged("xRotJitterAmount", "X Rot Jitter Amount", 0.0, 0.0, 360.0),
        ranged("xRotJitterSpeed", "X Rot Jitter Speed", 1.0, 0.0, 100.0),
        ranged("yRotJitterAmount", "Y Rot Jitter Amount", 0.0, 0.0, 360.0),
        ranged("yRotJitterSpeed", "Y Rot Jitter Speed", 1.0, 0.0, 100.0),
        ranged("zRotJitterAmount", "Z Rot Jitter Amount", 0.0, 0.0, 360.0),
        ranged("zRotJitterSpeed", "Z Rot Jitter Speed", 1.0, 0.0, 100.0),
    ]
);
pub const WIPE: Spec = Spec::new(&WIPE_DEF, "transition", card_wipe).points(&scene3d::POINTS);

/// A wandering from -1 to 1 for card `i`'s channel `k` at time `t` (`speed` wanders a second).
fn jitter(i: u64, k: u64, seed: u64, t: f64, speed: f64) -> f64 {
    let x = t * speed + rand(i, k, seed) as f64 * 100.0;
    let (n, f) = (x.floor(), x - x.floor());
    let s = f * f * (3.0 - 2.0 * f);
    let at = |m: f64| rand2(i, k ^ (m as i64 as u64).wrapping_mul(0x9E37), seed) as f64;
    at(n) + (at(n + 1.0) - at(n)) * s
}

fn card_wipe(ctx: &Ctx, mut b: Buf) -> Buf {
    let done = (ctx.num("transitionCompletion", 0.0) / 100.0).clamp(0.0, 1.0);
    let width = (ctx.num("transitionWidth", 50.0) / 100.0).clamp(0.001, 1.0);
    let (rows, cols) = grid(ctx);
    let back = ctx.layer_over("backLayer", &b, Take::All, true);
    let order = ctx.choice("flipOrder");
    let g = (order == 8).then(|| gradient(ctx, &b, "gradientLayer"));
    let randomness = ctx.num("timingRandomness", 0.0).clamp(0.0, 1.0);
    let seed = crate::noise::mix(&[ctx.seed, ctx.num("randomSeed", 1.0) as u64]);
    let scale = ctx.num("cardScale", 1.0);
    let (axis_kind, dir_kind) = (ctx.choice("flipAxis"), ctx.choice("flipDirection"));
    let t = ctx.time.seconds();
    let jit = |id: &str, d: f64| ctx.num(id, d);
    let r = ctx.bounds;
    let cam = Camera::of(ctx, &b);
    let (eye, light) = (cam.eye(), Light::of(ctx, &b));
    let mut placed: Vec<_> = (0..rows * cols)
        .filter_map(|i| {
            let (outline, mid, size) = card(&b, r, rows, cols, i);
            let (u, v) = (((i % cols) as f64 + 0.5) / cols as f64, ((i / cols) as f64 + 0.5) / rows as f64);
            // where in the order the card is, 0 first to 1 last
            let place = match order {
                1 => 1.0 - u,
                2 => v,
                3 => 1.0 - v,
                4 => (u + v) * 0.5,
                5 => (1.0 - u + v) * 0.5,
                6 => (u + 1.0 - v) * 0.5,
                7 => (2.0 - u - v) * 0.5,
                8 => g.as_ref().map_or(0.0, |g| {
                    let p = straight(g, mid[0], mid[1]);
                    luma([p[0], p[1], p[2]]) as f64
                }),
                _ => u,
            };
            let place = place * (1.0 - randomness) + rand(i as u64, 1, seed) as f64 * randomness;
            // how far it has turned: from its turn's start through the width
            let turned = ((done * (1.0 + width) - place) / width).clamp(0.0, 1.0);
            let smooth = turned * turned * (3.0 - 2.0 * turned);
            let about_y = match axis_kind {
                1 => true,
                2 => rand(i as u64, 2, seed) < 0.5,
                _ => false,
            };
            let sign = match dir_kind {
                1 => -1.0,
                2 => if rand(i as u64, 3, seed) < 0.5 { 1.0 } else { -1.0 },
                _ => 1.0,
            };
            let angle = sign * smooth * std::f64::consts::PI;
            let mut turn = scene3d::about(if about_y { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] }, angle);
            let mut at = [mid[0], mid[1], 0.0];
            // jitter, through the turn only
            let live = (turned * std::f64::consts::PI).sin();
            if ctx.flag("positionJitter") {
                let k = size[0].max(size[1]) * live;
                at[0] += jitter(i as u64, 10, seed, t, jit("xJitterSpeed", 1.0)) * jit("xJitterAmount", 0.0) * k;
                at[1] += jitter(i as u64, 11, seed, t, jit("yJitterSpeed", 1.0)) * jit("yJitterAmount", 0.0) * k;
                at[2] += jitter(i as u64, 12, seed, t, jit("zJitterSpeed", 1.0)) * jit("zJitterAmount", 0.0) * k;
            }
            if ctx.flag("rotationJitter") {
                let a = |id: &str, sp: &str, k: u64| (jitter(i as u64, k, seed, t, jit(sp, 1.0)) * jit(id, 0.0) * live).to_radians();
                let wobble = scene3d::turning(a("xRotJitterAmount", "xRotJitterSpeed", 13), a("yRotJitterAmount", "yRotJitterSpeed", 14), a("zRotJitterAmount", "zRotJitterSpeed", 15), 0);
                turn = scene3d::mul(&wobble, &turn);
            }
            let turn = [0, 1, 2].map(|k| [turn[k][0] * scale, turn[k][1] * scale, turn[k][2]]);
            let piece = Piece { outline, rest: [mid[0] as f32, mid[1] as f32], turn, at, alpha: 1.0, flat: None, mirror: if about_y { 2 } else { 1 } };
            Some(piece.place(&cam)?.lit(&light, eye))
        })
        .collect();
    let mut out = Image::new(b.img.width, b.img.height);
    scene3d::draw(&mut out, &mut placed, &b.img, back.as_ref());
    b.img = out;
    b
}

#[cfg(test)]
mod tests;

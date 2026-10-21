//! 3D Channel: effects that read the render passes that came with what the layer shows
//! (depth, object and material numbers, normals, texture coordinates, coverage, Cryptomatte;
//! see [`veymelo_raster::passes`]): a pass shown as a picture (3D Channel Extract), any channels
//! put into red, green, blue and alpha (Channel Extractor), the layer cut by depth (Depth Matte),
//! by an object's or material's number (ID Matte) or by names (Cryptomatte), blurred by its
//! distance from a focal plane (Depth of Field), fogged by depth (Fog 3D), and the numbers shown
//! (ID Viewer).
//!
//! Without passes they leave the layer as it is, and say so. Passes are read at the nearest of
//! their pixels (numbers must not be mixed), with four points a pixel where an edge would
//! otherwise be jagged.

use std::sync::Arc;

use veymelo_keyframe::{Kind, Value};
use veymelo_project::PropDef;
use veymelo_raster::filter::{self, Edge};
use veymelo_raster::passes::{FAR, Passes, crypto_id};
use veymelo_raster::{Image, Px};

use crate::defs::{choice, color, effect, flag, layer, ranged};
use crate::util::{luma, premul, rgb, unpremul};
use crate::{Buf, Ctx, Spec, Take};

const CATEGORY: &str = "3dChannel";

/// The passes, or a note that there are none.
fn passes_of(ctx: &Ctx) -> Option<Arc<Passes>> {
    let p = ctx.host.and_then(|h| h.passes());
    if p.is_none() {
        ctx.note("what the layer shows has no render passes (depth, object numbers…): left as it is".into());
    }
    p
}

/// A pass that is missing, noted.
fn missing<T>(ctx: &Ctx, what: &str) -> Option<T> {
    ctx.note(format!("the render passes have no {what}: left as it is"));
    None
}

/// For each of the buffer's pixels, the passes' pixel under point `at` (0 to 1 across it).
fn under(b: &Buf, p: &Passes, at: [f64; 2]) -> Vec<Option<usize>> {
    let w = b.img.width;
    (0..b.img.px.len()).map(|i| p.index_at(b.origin.x + ((i % w) as f64 + at[0]) / b.scale, b.origin.y + ((i / w) as f64 + at[1]) / b.scale)).collect()
}

/// Four points in a pixel, for edges that would be jagged at one.
const FOUR: [[f64; 2]; 4] = [[0.25, 0.25], [0.75, 0.25], [0.25, 0.75], [0.75, 0.75]];

/// `f` of the passes' pixel under each buffer pixel (none: off the passes), at its middle or as
/// the mean of four points in it.
fn each(b: &Buf, p: &Passes, four: bool, f: impl Fn(Option<usize>) -> Px) -> Vec<Px> {
    let n = b.img.px.len();
    if !four {
        return under(b, p, [0.5, 0.5]).into_iter().map(&f).collect();
    }
    let maps: Vec<Vec<Option<usize>>> = FOUR.iter().map(|at| under(b, p, *at)).collect();
    (0..n)
        .map(|i| {
            let mut s = [0f32; 4];
            for m in &maps {
                let v = f(m[i]);
                (0..4).for_each(|k| s[k] += v[k] * 0.25);
            }
            s
        })
        .collect()
}

/// A colour of its own for an ID (black for 0).
pub fn id_color(id: f32) -> [f32; 3] {
    if id == 0.0 {
        return [0.0; 3];
    }
    let h = crate::sprites::rand(id.to_bits() as u64, 0x1d, 0x5eed);
    rgb(h, 0.75, 0.5)
}

// ---------------------------------------------------------------- 3D Channel Extract

pub const EXTRACTED: &[&str] = &["zDepth", "objectId", "textureUv", "surfaceNormals", "coverage", "backgroundRgb", "unclampedRgb", "materialId"];

effect!(EXTRACT_DEF, "channelExtract3d", "3D Channel Extract", "a render pass shown as a picture: depth, object or material numbers, texture coordinates, normals, coverage, the background or the colour beyond white", [
    choice("channel", "3D Channel", EXTRACTED, 0),
    ranged("blackPoint", "Black Point", 0.0, -1.0e6, 1.0e6).doc("the value shown black"),
    ranged("whitePoint", "White Point", 10_000.0, -1.0e6, 1.0e6).doc("the value shown white"),
    flag("clampOutput", "Clamp Output", true),
    flag("invertDepthMap", "Invert Depth Map", false),
    flag("antialias", "Anti-alias", false).doc("four points a pixel, for smooth edges"),
]);
pub const EXTRACT: Spec = Spec::new(&EXTRACT_DEF, CATEGORY, extract);

fn extract(ctx: &Ctx, mut b: Buf) -> Buf {
    let Some(p) = passes_of(ctx) else { return b };
    let (black, white) = (ctx.num("blackPoint", 0.0) as f32, ctx.num("whitePoint", 10_000.0) as f32);
    let (clamp, invert) = (ctx.flag("clampOutput"), ctx.flag("invertDepthMap"));
    // from black to white (either way round)
    let level = |v: f32| {
        let t = if white == black { if v >= white { 1.0 } else { 0.0 } } else { (v - black) / (white - black) };
        let t = if clamp { t.clamp(0.0, 1.0) } else { t };
        if invert { 1.0 - t } else { t }
    };
    let three = |names: [&[&str]; 3]| -> Option<[&[f32]; 3]> { Some([p.find(names[0])?, p.find(names[1])?, p.find(names[2])?]) };
    fn one(c: Option<&[f32]>) -> Option<[&[f32]; 3]> {
        c.map(|c| [c, c, c])
    }
    let (planes, mapped, what): (Option<[&[f32]; 3]>, bool, &str) = match ctx.choice("channel") {
        0 => (one(p.depth()), true, "depth"),
        1 => (one(p.object_id()), true, "object numbers"),
        7 => (one(p.material_id()), true, "material numbers"),
        4 => (one(p.coverage()), true, "coverage"),
        2 => (p.find(&["UV.U", "U", "uv.x"]).zip(p.find(&["UV.V", "V", "uv.y"])).map(|(u, v)| [u, v, &[][..]]), true, "texture coordinates"),
        3 => (three([&["N.X", "Normal.X", "normals.x"], &["N.Y", "Normal.Y", "normals.y"], &["N.Z", "Normal.Z", "normals.z"]]), true, "normals"),
        5 => (three([&["BG.R", "Background.R"], &["BG.G", "Background.G"], &["BG.B", "Background.B"]]), true, "background colour"),
        _ => (three([&["R", "red"], &["G", "green"], &["B", "blue"]]), false, "colour"),
    };
    let Some(planes) = planes else { return missing(ctx, what).unwrap_or(b) };
    let normals = ctx.choice("channel") == 3;
    let px = each(&b, &p, ctx.flag("antialias"), |i| {
        let Some(i) = i else { return [0.0; 4] };
        let c: [f32; 3] = std::array::from_fn(|k| {
            let Some(&v) = planes[k].get(i) else { return 0.0 };
            // normals from -1..1 to 0..1 first
            let v = if normals { v * 0.5 + 0.5 } else { v };
            if mapped { level(v) } else { v }
        });
        [c[0], c[1], c[2], 1.0]
    });
    b.img.px = px.into();
    b
}

// ---------------------------------------------------------------- Channel Extractor

const fn text(id: &'static str, name: &'static str) -> PropDef {
    PropDef::new(id, name, Kind::Text, Value::Text(String::new()))
}

effect!(EXTRACTOR_DEF, "channelExtractor", "Channel Extractor", "any of the passes' channels (by name) or the layer's own put into red, green, blue and alpha, from a black to a white point", [
    text("red", "Red").doc("a channel's name, or R, G, B or A for the layer's own (empty: R)"),
    text("green", "Green").doc("empty: G"),
    text("blue", "Blue").doc("empty: B"),
    text("alpha", "Alpha").doc("empty: A"),
    ranged("blackPoint", "Black Point", 0.0, -1.0e6, 1.0e6),
    ranged("whitePoint", "White Point", 1.0, -1.0e6, 1.0e6),
    flag("unMult", "UnMult", false).doc("the colour divided by the alpha (for channels kept premultiplied)"),
    flag("clip", "Clip", false).doc("the colour kept within 0 to 1"),
]);
pub const EXTRACTOR: Spec = Spec::new(&EXTRACTOR_DEF, CATEGORY, extractor);

fn extractor(ctx: &Ctx, mut b: Buf) -> Buf {
    let p = ctx.host.and_then(|h| h.passes());
    let names: [String; 4] = ["red", "green", "blue", "alpha"].map(|k| ctx.value_in(ctx.effect, k).map(|v| v.as_text().trim().to_string()).unwrap_or_default());
    let names: [String; 4] = std::array::from_fn(|k| if names[k].is_empty() { ["R", "G", "B", "A"][k].to_string() } else { names[k].clone() });
    let own = |n: &str| match n.to_ascii_uppercase().as_str() {
        "R" | "RED" => Some(0),
        "G" | "GREEN" => Some(1),
        "B" | "BLUE" => Some(2),
        "A" | "ALPHA" => Some(3),
        _ => None,
    };
    // a channel of the passes by its name, unless it is the layer's own
    let from: Vec<Option<&[f32]>> = names.iter().map(|n| if own(n).is_some() && p.as_ref().is_none_or(|p| p.get(n).is_none()) { None } else { p.as_ref().and_then(|p| p.get(n)) }).collect();
    for (n, f) in names.iter().zip(&from) {
        if f.is_none() && own(n).is_none() {
            ctx.note(format!("there is no channel \"{n}\": black (or opaque, for alpha)"));
        }
    }
    let (black, white) = (ctx.num("blackPoint", 0.0) as f32, ctx.num("whitePoint", 1.0) as f32);
    let (unmult, clip) = (ctx.flag("unMult"), ctx.flag("clip"));
    if from.iter().all(Option::is_none) && black == 0.0 && white == 1.0 && !unmult && names.iter().enumerate().all(|(k, n)| own(n) == Some(k)) {
        return b;
    }
    let level = |v: f32| {
        let t = if white == black { if v >= white { 1.0 } else { 0.0 } } else { (v - black) / (white - black) };
        if clip { t.clamp(0.0, 1.0) } else { t }
    };
    let at = p.as_ref().map(|p| under(&b, p, [0.5, 0.5]));
    let px: Vec<Px> = b.img.px.iter().enumerate().map(|(i, src)| {
        let (c, a) = unpremul(*src);
        let straight = [c[0], c[1], c[2], a];
        let o: [f32; 4] = std::array::from_fn(|k| {
            let v = match (from[k], at.as_ref().and_then(|m| m[i])) {
                (Some(ch), Some(j)) => ch[j],
                (Some(_), None) => 0.0,
                (None, _) => own(&names[k]).map_or(if k == 3 { 1.0 } else { 0.0 }, |c| straight[c]),
            };
            if k < 3 { level(v) } else { v }
        });
        let alpha = o[3].clamp(0.0, 1.0);
        let c = if unmult && alpha > 1e-6 { [o[0] / alpha, o[1] / alpha, o[2] / alpha] } else { [o[0], o[1], o[2]] };
        premul(c, alpha)
    }).collect();
    b.img.px = px.into();
    b
}

// ---------------------------------------------------------------- Depth Matte

effect!(DEPTH_MATTE_DEF, "depthMatte", "Depth Matte", "the layer kept where it is at least so deep (or, inverted, nearer)", [
    ranged("depth", "Depth", 0.0, -1.0e6, 1.0e6),
    ranged("feather", "Feather", 0.0, 0.0, 1.0e6).doc("how deep the change from kept to gone is"),
    flag("invert", "Invert", false),
]);
pub const DEPTH_MATTE: Spec = Spec::new(&DEPTH_MATTE_DEF, CATEGORY, depth_matte);

fn depth_matte(ctx: &Ctx, mut b: Buf) -> Buf {
    let Some(p) = passes_of(ctx) else { return b };
    let Some(z) = p.depth() else { return missing(ctx, "depth").unwrap_or(b) };
    let (depth, feather, invert) = (ctx.num("depth", 0.0) as f32, ctx.num("feather", 0.0).max(0.0) as f32, ctx.flag("invert"));
    let keep = |i: Option<usize>| -> Px {
        let d = i.map_or(FAR, |i| z[i]);
        let k = if feather > 0.0 { ((d - depth) / feather + 0.5).clamp(0.0, 1.0) } else if d >= depth { 1.0 } else { 0.0 };
        let k = if invert { 1.0 - k } else { k };
        [k; 4]
    };
    let k = each(&b, &p, true, keep);
    for (px, k) in b.img.px.iter_mut().zip(&k) {
        *px = px.map(|v| v * k[0]);
    }
    b
}

// ---------------------------------------------------------------- Depth of Field

effect!(FOCUS_DEF, "depthOfField", "Depth of Field", "the layer blurred by how far each part is from the focal plane: what is in front blurs over what is behind, what is sharp stays sharp", [
    ranged("focalPlane", "Focal Plane", 0.0, -1.0e6, 1.0e6).doc("the depth in focus"),
    ranged("maximumRadius", "Maximum Radius", 0.0, 0.0, 500.0).doc("the blur of the farthest from focus (pixels)"),
    ranged("focalPlaneThickness", "Focal Plane Thickness", 0.0, 0.0, 1.0e6).doc("how deep the sharp part is"),
    ranged("focalBias", "Focal Bias", 50.0, 0.0, 100.0).doc("above 50 the blur comes on sooner away from focus"),
]);
pub const FOCUS: Spec = Spec::new(&FOCUS_DEF, CATEGORY, depth_of_field);

/// How blurred (pixels) something at depth `z` is: none within the focal plane's thickness,
/// more the farther off it (a focal distance off at the most), as the bias bends it.
pub fn blur_at(z: f32, focal: f32, thickness: f32, most: f32, bias: f32) -> f32 {
    let off = ((z - focal).abs() - thickness * 0.5).max(0.0);
    let n = (off / focal.abs().max(1.0)).min(1.0);
    most * n.powf(2f32.powf((50.0 - bias) / 25.0))
}

/// The most blur (pixels) between one slice of depth and the next.
const STEP: f32 = 1.5;

fn depth_of_field(ctx: &Ctx, mut b: Buf) -> Buf {
    let Some(p) = passes_of(ctx) else { return b };
    let Some(z) = p.depth() else { return missing(ctx, "depth").unwrap_or(b) };
    let most = (ctx.num("maximumRadius", 0.0).max(0.0) * b.scale) as f32;
    if most <= 0.0 {
        return b;
    }
    let (focal, thickness, bias) = (ctx.num("focalPlane", 0.0) as f32, ctx.num("focalPlaneThickness", 0.0).max(0.0) as f32, ctx.num("focalBias", 50.0) as f32);
    let (w, h) = (b.img.width, b.img.height);
    let steps = ((most / STEP).ceil() as usize).clamp(1, 32);
    // the slices from the farthest to the nearest: behind the focal plane from the most blurred
    // to the sharp, then in front of it from the sharp to the most blurred; each pixel in one
    let slice: Vec<usize> = under(&b, &p, [0.5, 0.5])
        .into_iter()
        .map(|i| {
            let d = i.map_or(FAR, |i| z[i]);
            let k = (blur_at(d, focal, thickness, most, bias) / most * steps as f32).round() as usize;
            if d < focal { steps + 1 + k } else { steps - k }
        })
        .collect();
    let radius = |j: usize| most * (if j <= steps { steps - j } else { j - steps - 1 }) as f32 / steps as f32;
    let mut used = vec![false; 2 * steps + 2];
    slice.iter().for_each(|&j| used[j] = true);
    let mut out = Image::new(w, h);
    for j in 0..2 * steps + 2 {
        if !used[j] {
            continue;
        }
        let mut part = Image::new(w, h);
        for (i, px) in b.img.px.iter().enumerate() {
            if slice[i] == j {
                part.px[i] = *px;
            }
        }
        let sigma = (radius(j) * 0.5) as f64;
        // under what is nearer, the slice goes on as it is round there (from its colours near
        // by), as far as its own blur or a nearer one's reaches, so a blurred edge (its own or
        // one over it) shows the slice, not a hole
        let nearer = (j + 1..2 * steps + 2).filter(|&k| used[k]).map(|k| (radius(k) * 0.5) as f64).fold(-1.0, f64::max);
        let reach = sigma.max(nearer) * 2.0;
        if nearer >= 0.0 && reach > 0.3 {
            let reach = reach.max(2.0);
            let round = filter::gaussian(&part, reach, reach, Edge::Clear);
            for (i, px) in part.px.iter_mut().enumerate() {
                let (c, a) = unpremul(round.px[i]);
                if slice[i] > j && round.px[i][3] > 1e-3 && a > 0.0 {
                    *px = premul(c, b.img.px[i][3]);
                }
            }
        }
        let part = if sigma > 0.15 { filter::gaussian(&part, sigma, sigma, Edge::Repeat) } else { part };
        for (o, q) in out.px.iter_mut().zip(&part.px) {
            let k = 1.0 - q[3];
            *o = [q[0] + o[0] * k, q[1] + o[1] * k, q[2] + o[2] * k, q[3] + o[3] * k];
        }
    }
    b.img = out;
    b
}

// ---------------------------------------------------------------- Fog 3D

effect!(FOG_DEF, "fog3d", "Fog 3D", "fog thickening with depth, the background in it or not, its thickness shaped by a gradient layer", [
    color("fogColor", "Fog Color", 1.0, 1.0, 1.0),
    ranged("fogStartDepth", "Fog Start Depth", 0.0, -1.0e6, 1.0e6),
    ranged("fogEndDepth", "Fog End Depth", 3000.0, -1.0e6, 1.0e6),
    ranged("fogOpacity", "Fog Opacity", 100.0, 0.0, 100.0),
    ranged("scatteringDensity", "Scattering Density", 50.0, 0.0, 100.0).doc("higher: thick sooner"),
    flag("foggyBackground", "Foggy Background", true).doc("where nothing was rendered, all fog"),
    layer("gradientLayer", "Gradient Layer").doc("its brightness thins (dark) or keeps (light) the fog"),
    ranged("layerContribution", "Layer Contribution", 0.0, 0.0, 100.0),
]);
pub const FOG: Spec = Spec::new(&FOG_DEF, CATEGORY, fog);

fn fog(ctx: &Ctx, mut b: Buf) -> Buf {
    let Some(p) = passes_of(ctx) else { return b };
    let Some(z) = p.depth() else { return missing(ctx, "depth").unwrap_or(b) };
    let c = ctx.color("fogColor");
    let tint = [c.r as f32, c.g as f32, c.b as f32];
    let (start, end) = (ctx.num("fogStartDepth", 0.0) as f32, ctx.num("fogEndDepth", 3000.0) as f32);
    let opacity = (ctx.num("fogOpacity", 100.0) / 100.0) as f32;
    let density = (ctx.num("scatteringDensity", 50.0) / 100.0) as f32;
    let foggy = ctx.flag("foggyBackground");
    let part = (ctx.num("layerContribution", 0.0) / 100.0) as f32;
    let shape = (part > 0.0).then(|| ctx.layer_over("gradientLayer", &b, Take::All, true)).flatten();
    let at = under(&b, &p, [0.5, 0.5]);
    for (i, px) in b.img.px.iter_mut().enumerate() {
        let d = at[i].map_or(FAR, |j| z[j]);
        let nothing = d >= FAR * 0.5;
        let mut f = if nothing {
            if foggy { 1.0 } else { 0.0 }
        } else {
            let t = ((d - start) / (end - start).abs().max(1e-6)).clamp(0.0, 1.0);
            1.0 - (1.0 - t).powf(1.0 + density * 3.0)
        };
        if let Some(g) = &shape {
            let (gc, ga) = unpremul(g.px[i]);
            f *= 1.0 - part + part * luma(gc) * ga;
        }
        let f = (f * opacity).clamp(0.0, 1.0);
        let (c, a) = unpremul(*px);
        let a = if nothing && foggy { a + (1.0 - a) * f } else { a };
        *px = premul([0, 1, 2].map(|k| c[k] + (tint[k] - c[k]) * f), a);
    }
    b
}

// ---------------------------------------------------------------- ID Matte and ID Viewer

const ID_KINDS: &[&str] = &["objectId", "materialId"];

fn ids_of<'p>(ctx: &Ctx, p: &'p Passes, which: u32) -> Option<&'p [f32]> {
    let ids = if which == 1 { p.material_id() } else { p.object_id() };
    if ids.is_none() {
        return missing(ctx, if which == 1 { "material numbers" } else { "object numbers" });
    }
    ids
}

effect!(ID_MATTE_DEF, "idMatte", "ID Matte", "the layer kept where one object (or material) is, by its number", [
    choice("auxChannel", "Aux. Channel", ID_KINDS, 0),
    ranged("idSelection", "ID Selection", 0.0, 0.0, 65_535.0),
    ranged("feather", "Feather", 0.0, 0.0, 100.0),
    flag("invert", "Invert", false),
    flag("useCoverage", "Use Coverage", false).doc("the coverage pass for the edges"),
]);
pub const ID_MATTE: Spec = Spec::new(&ID_MATTE_DEF, CATEGORY, id_matte);

fn id_matte(ctx: &Ctx, mut b: Buf) -> Buf {
    let Some(p) = passes_of(ctx) else { return b };
    let Some(ids) = ids_of(ctx, &p, ctx.choice("auxChannel")) else { return b };
    let wanted = ctx.num("idSelection", 0.0).round() as f32;
    let cover = if ctx.flag("useCoverage") { p.coverage() } else { None };
    // with coverage the edges are known; without, four points a pixel smooth them
    let m = each(&b, &p, cover.is_none(), |i| {
        let k = i.map_or(0.0, |i| if ids[i].round() == wanted { cover.map_or(1.0, |c| c[i].clamp(0.0, 1.0)) } else { 0.0 });
        [k; 4]
    });
    let mut matte = Image { width: b.img.width, height: b.img.height, px: m.into() };
    let feather = ctx.num("feather", 0.0).max(0.0) * b.scale;
    if feather > 0.0 {
        matte = filter::gaussian(&matte, feather * 0.5, feather * 0.5, Edge::Repeat);
    }
    let invert = ctx.flag("invert");
    for (px, k) in b.img.px.iter_mut().zip(&matte.px) {
        let k = if invert { 1.0 - k[0] } else { k[0] };
        *px = px.map(|v| v * k);
    }
    b
}

effect!(ID_VIEWER_DEF, "idViewer", "ID Viewer", "the object (or material) numbers shown: each in a colour of its own, one picked out as a matte, or the numbers as greys", [
    choice("channelType", "Channel Type", ID_KINDS, 0),
    choice("display", "Display", &["colors", "lumaMatte", "alphaMatte", "raw"], 0),
    ranged("id", "ID", 0.0, 0.0, 65_535.0),
]);
pub const ID_VIEWER: Spec = Spec::new(&ID_VIEWER_DEF, CATEGORY, id_viewer);

fn id_viewer(ctx: &Ctx, mut b: Buf) -> Buf {
    let Some(p) = passes_of(ctx) else { return b };
    let Some(ids) = ids_of(ctx, &p, ctx.choice("channelType")) else { return b };
    let wanted = ctx.num("id", 0.0).round() as f32;
    let display = ctx.choice("display");
    let most = ids.iter().copied().fold(1f32, f32::max);
    let at = under(&b, &p, [0.5, 0.5]);
    for (i, px) in b.img.px.iter_mut().enumerate() {
        let Some(j) = at[i] else {
            if display != 2 {
                *px = [0.0; 4];
            }
            continue;
        };
        let id = ids[j].round();
        let hit = id == wanted;
        *px = match display {
            // every number its own colour, the picked one white
            0 => {
                let c = if hit && wanted != 0.0 { [1.0; 3] } else { id_color(id) };
                [c[0], c[1], c[2], 1.0]
            }
            1 => {
                let v = if hit { 1.0 } else { 0.0 };
                [v, v, v, 1.0]
            }
            // the layer where it is, nothing elsewhere
            2 => if hit { *px } else { [0.0; 4] },
            _ => {
                let v = id / most;
                [v, v, v, 1.0]
            }
        };
    }
    b
}

// ---------------------------------------------------------------- Cryptomatte

pub const CRYPTO_KINDS: &[&str] = &["cryptoObject", "cryptoMaterial", "cryptoAsset"];

effect!(CRYPTO_DEF, "cryptomatte", "Cryptomatte", "the layer kept where the named objects, materials or assets are (from Cryptomatte passes), their edges as the render saw them", [
    choice("layer", "Layer", CRYPTO_KINDS, 0),
    text("selection", "Selection").doc("names, commas between (quote a name with a comma in it), * for any letters, or <hex> IDs"),
    choice("display", "Display", &["colors", "matte", "preview"], 2),
    flag("matteOnly", "Matte Only", false),
]);
pub const CRYPTO: Spec = Spec::new(&CRYPTO_DEF, CATEGORY, cryptomatte);

/// Whether `name` fits `pattern`, where `*` stands for any letters.
fn fits(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == name,
        Some((head, rest)) => {
            let Some(tail) = name.strip_prefix(head) else { return false };
            rest.is_empty() || (0..=tail.len()).filter(|&k| tail.is_char_boundary(k)).any(|k| fits(rest, &tail[k..]))
        }
    }
}

/// A selection's names: split at commas, but not within quotes ("Green, Glossy"), a backslash
/// taking the next letter as it is; trimmed, the quotes dropped.
fn names_in(selection: &str) -> Vec<String> {
    let (mut out, mut now, mut quote) = (Vec::new(), String::new(), None::<char>);
    let mut chars = selection.chars();
    while let Some(c) = chars.next() {
        match (c, quote) {
            ('\\', _) => now.extend(chars.next()),
            (c, Some(q)) if c == q => quote = None,
            ('"' | '\'', None) if now.trim().is_empty() => {
                now.clear();
                quote = Some(c);
            }
            (',', None) => out.push(std::mem::take(&mut now).trim().to_string()),
            (c, _) => now.push(c),
        }
    }
    out.push(now.trim().to_string());
    out.retain(|n| !n.is_empty());
    out
}

/// The IDs a selection stands for: names (through the manifest, or hashed), patterns (through
/// the manifest) and `<hex>` IDs.
pub fn selected_ids(selection: &str, manifest: Option<&[(String, u32)]>) -> Vec<u32> {
    let mut out = Vec::new();
    for n in names_in(selection) {
        if n.contains('*') {
            out.extend(manifest.into_iter().flatten().filter(|(m, _)| fits(&n, m)).map(|(_, id)| *id));
        } else if let Some(id) = n.strip_prefix('<').and_then(|r| r.strip_suffix('>')).and_then(|h| u32::from_str_radix(h, 16).ok()) {
            out.push(id);
        } else {
            out.push(manifest.and_then(|m| m.iter().find(|(m, _)| *m == n).map(|(_, id)| *id)).unwrap_or_else(|| crypto_id(&n)));
        }
    }
    out
}

fn cryptomatte(ctx: &Ctx, mut b: Buf) -> Buf {
    let Some(p) = passes_of(ctx) else { return b };
    let kinds = p.crypto_kinds();
    if kinds.is_empty() {
        return missing(ctx, "Cryptomatte passes").unwrap_or(b);
    }
    let want = ["CryptoObject", "CryptoMaterial", "CryptoAsset"][(ctx.choice("layer") as usize).min(2)];
    let kind = kinds.iter().find(|k| k.as_str() == want || k.ends_with(want)).unwrap_or(&kinds[0]).clone();
    let manifest = p.manifests.iter().find(|(k, _)| *k == kind).map(|(_, m)| m.as_slice());
    let text = ctx.value_in(ctx.effect, "selection").map(|v| v.as_text().to_string()).unwrap_or_default();
    let ids = selected_ids(&text, manifest);
    let ranks = p.crypto_ranks(&kind);
    // how much of the pixel the selection covers: its IDs' coverages added up
    let matte = |i: usize| ranks.iter().filter(|(id, c)| c[i] > 0.0 && ids.contains(&id[i].to_bits())).map(|(_, c)| c[i]).sum::<f32>().clamp(0.0, 1.0);
    // the IDs' colours mixed by how much each covers
    let colours = |i: usize| {
        let mut s = [0f32; 3];
        for (id, c) in &ranks {
            let k = id_color(id[i]);
            (0..3).for_each(|j| s[j] += k[j] * c[i]);
        }
        s
    };
    let (display, only) = (ctx.choice("display"), ctx.flag("matteOnly"));
    let at = under(&b, &p, [0.5, 0.5]);
    for (i, px) in b.img.px.iter_mut().enumerate() {
        let m = at[i].map_or(0.0, matte);
        *px = if only {
            [m; 4]
        } else {
            match display {
                0 => {
                    let c = at[i].map_or([0.0; 3], colours).map(|v| v * (1.0 - 0.5 * m) + 0.5 * m);
                    [c[0], c[1], c[2], 1.0]
                }
                1 => px.map(|v| v * m),
                // the layer, the selection lit up
                _ => [px[0] * (1.0 - 0.5 * m) + 0.5 * m * px[3], px[1] * (1.0 - 0.5 * m) + 0.5 * m * px[3], px[2] * (1.0 - 0.5 * m), px[3]],
            }
        };
    }
    b
}

pub const ALL: [Spec; 8] = [EXTRACT, CRYPTO, DEPTH_MATTE, FOCUS, EXTRACTOR, FOG, ID_MATTE, ID_VIEWER];

#[cfg(test)]
mod tests;

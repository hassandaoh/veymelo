//! A Photoshop document brought in as a composition of its layers: each pixel layer footage of
//! just that layer (the document's size, or its own placed where it is), each group a composition
//! of what is in it (a pass-through group collapsed, so its blend modes reach below it), each
//! layer's opacity, fill opacity, blend mode, visibility and clipping kept.
//!
//! What Photoshop keeps as settings comes as settings: a vector mask as masks (its shapes joined
//! as they are: combined, subtracted, intersected, excluded); layer effects as layer styles,
//! sharing the document's light; a fill layer as a solid (a gradient fill: a solid with a
//! gradient overlay); an adjustment layer as an adjustment layer with the effect that does the
//! same (levels, curves, brightness and contrast, hue and saturation, colour balance, invert,
//! threshold, posterize, exposure, vibrance, black and white, photo filter, channel mixer,
//! selective colour); a type layer as a text layer that can be edited — its runs of fonts, sizes
//! and colours, its paragraphs, point or box text, placed and turned as Photoshop places it — or,
//! if asked, as its pixels; a smart object as footage of the file it holds (written beside the
//! document) or links, pinned by its four corners. A painted mask that cannot go into pixels
//! (on a fill, adjustment or type layer, a smart object or a group) comes as footage of the mask:
//! where a fill or adjustment layer is, or a track matte for the others. What is not brought, or
//! not exactly, is listed.

use std::path::{Path, PathBuf};

use veymelo_json::Value as Json;
use veymelo_keyframe::{Feathers, Gradient, Kind, ShapePath, Value};
use veymelo_project::styles::{self, BLENDING_OPTIONS, LIST, MODES};
use veymelo_project::{AlphaMode, Comp, Footage, FootageKind, Group, ItemKind, MASK, MASK_MODES, Matte, MatteKind, SourceLayer};
use veymelo_psd::adjust::Adjustment;
use veymelo_psd::looks::{Fill, gradient_stops};
use veymelo_psd::typeset::{Paragraph, Style, TypeLayer};
use veymelo_psd::vector::Join;
use veymelo_psd::{Desc, Item, Node, Psd};
use veymelo_time::{FrameRate, Tick};

use crate::{Ctx, Error, Result};

/// How a layered file may come in.
pub(super) const IMPORT_AS: &[&str] = &["footage", "composition", "compositionLayerSizes"];

/// How a Photoshop document's type layers may come in.
pub(super) const TEXT_AS: &[&str] = &["editable", "pixels"];

/// How a Photoshop document comes in: each layer just its own rectangle (or the document's
/// size), type layers as their pixels (or editable), and the folder the files its smart objects
/// hold are written to (default: "<its name> smart objects" beside it).
pub(super) struct How<'a> {
    pub cropped: bool,
    pub text_pixels: bool,
    pub smart_folder: Option<&'a str>,
}

/// A Photoshop blend mode's key as ours (pass through: normal, the group collapsed).
fn blend(key: &[u8; 4]) -> &'static str {
    match key {
        b"diss" => "dissolve",
        b"dark" => "darken",
        b"mul " => "multiply",
        b"idiv" => "colorBurn",
        b"lbrn" => "linearBurn",
        b"dkCl" => "darkerColor",
        b"lite" => "lighten",
        b"scrn" => "screen",
        b"div " => "colorDodge",
        b"lddg" => "linearDodge",
        b"lgCl" => "lighterColor",
        b"over" => "overlay",
        b"sLit" => "softLight",
        b"hLit" => "hardLight",
        b"vLit" => "vividLight",
        b"lLit" => "linearLight",
        b"pLit" => "pinLight",
        b"hMix" => "hardMix",
        b"diff" => "difference",
        b"smud" => "exclusion",
        b"fsub" => "subtract",
        b"fdiv" => "divide",
        b"hue " => "hue",
        b"sat " => "saturation",
        b"colr" => "color",
        b"lum " => "luminosity",
        _ => "normal",
    }
}

/// Photoshop's blend modes in descriptors (`BlnM`) as ours.
const DESC_MODES: [(&str, &str); 27] = [
    ("Nrml", "normal"),
    ("Dslv", "dissolve"),
    ("Drkn", "darken"),
    ("Mltp", "multiply"),
    ("CBrn", "colorBurn"),
    ("linearBurn", "linearBurn"),
    ("darkerColor", "darkerColor"),
    ("Lghn", "lighten"),
    ("Scrn", "screen"),
    ("CDdg", "colorDodge"),
    ("linearDodge", "linearDodge"),
    ("lighterColor", "lighterColor"),
    ("Ovrl", "overlay"),
    ("SftL", "softLight"),
    ("HrdL", "hardLight"),
    ("vividLight", "vividLight"),
    ("linearLight", "linearLight"),
    ("pinLight", "pinLight"),
    ("hardMix", "hardMix"),
    ("Dfrn", "difference"),
    ("Xclu", "exclusion"),
    ("blendSubtraction", "subtract"),
    ("blendDivide", "divide"),
    ("H   ", "hue"),
    ("Strt", "saturation"),
    ("Clr ", "color"),
    ("Lmns", "luminosity"),
];

/// How a Photoshop effect's setting becomes a style's.
#[derive(Clone, Copy)]
enum Conv {
    /// As it is.
    Num,
    /// Pixels, times the effects' scale.
    Px,
    Bool,
    Color,
    Mode,
    /// An enumeration, by Photoshop's value and ours.
    Pick(&'static [(&'static str, &'static str)]),
    /// A gradient (and its smoothness).
    Grad,
    /// An offset in percent of the layer's size, as pixels.
    Offset,
}

const GLOW_TECHNIQUES: &[(&str, &str)] = &[("SfBL", "softer"), ("PrBL", "precise")];
const GLOW_SOURCES: &[(&str, &str)] = &[("SrcC", "center"), ("SrcE", "edge")];
const BEVEL_STYLES: &[(&str, &str)] = &[("OtrB", "outerBevel"), ("InrB", "innerBevel"), ("Embs", "emboss"), ("PlEb", "pillowEmboss"), ("strokeEmboss", "strokeEmboss")];
const BEVEL_TECHNIQUES: &[(&str, &str)] = &[("SfBL", "smooth"), ("PrBL", "chiselHard"), ("Slmt", "chiselSoft")];
const BEVEL_DIRECTIONS: &[(&str, &str)] = &[("In  ", "up"), ("Out ", "down")];
const GRADIENT_STYLES: &[(&str, &str)] = &[("Lnr ", "linear"), ("Rdl ", "radial"), ("Angl", "angle"), ("Rflc", "reflected"), ("Dmnd", "diamond")];
const STROKE_POSITIONS: &[(&str, &str)] = &[("OutF", "outside"), ("InsF", "inside"), ("CtrF", "center")];

/// Each kind of effect's settings beyond its mode, colour and opacity: Photoshop's key, ours,
/// and how.
fn settings(kind: &str) -> &'static [(&'static str, &'static str, Conv)] {
    use Conv::*;
    match kind {
        "dropShadow" => &[("uglg", "useGlobalLight", Bool), ("lagl", "angle", Num), ("Dstn", "distance", Px), ("Ckmt", "spread", Num), ("blur", "size", Px), ("Nose", "noise", Num), ("layerConceals", "knocksOut", Bool)],
        "innerShadow" => &[("uglg", "useGlobalLight", Bool), ("lagl", "angle", Num), ("Dstn", "distance", Px), ("Ckmt", "choke", Num), ("blur", "size", Px), ("Nose", "noise", Num)],
        "outerGlow" => &[("Nose", "noise", Num), ("Grad", "colors", Grad), ("GlwT", "technique", Pick(GLOW_TECHNIQUES)), ("Ckmt", "spread", Num), ("blur", "size", Px), ("Inpr", "range", Num), ("ShdN", "jitter", Num)],
        "innerGlow" => &[("Nose", "noise", Num), ("Grad", "colors", Grad), ("GlwT", "technique", Pick(GLOW_TECHNIQUES)), ("glwS", "source", Pick(GLOW_SOURCES)), ("Ckmt", "choke", Num), ("blur", "size", Px), ("Inpr", "range", Num), ("ShdN", "jitter", Num)],
        "bevelEmboss" => &[
            ("bvlS", "style", Pick(BEVEL_STYLES)),
            ("bvlT", "technique", Pick(BEVEL_TECHNIQUES)),
            ("srgR", "depth", Num),
            ("bvlD", "direction", Pick(BEVEL_DIRECTIONS)),
            ("blur", "size", Px),
            ("Sftn", "soften", Px),
            ("uglg", "useGlobalLight", Bool),
            ("lagl", "angle", Num),
            ("Lald", "altitude", Num),
            ("hglM", "highlightMode", Mode),
            ("hglC", "highlightColor", Color),
            ("hglO", "highlightOpacity", Num),
            ("sdwM", "shadowMode", Mode),
            ("sdwC", "shadowColor", Color),
            ("sdwO", "shadowOpacity", Num),
        ],
        "satin" => &[("lagl", "angle", Num), ("Dstn", "distance", Px), ("blur", "size", Px), ("Invr", "invert", Bool)],
        "gradientOverlay" => &[("Grad", "colors", Grad), ("Angl", "angle", Num), ("Type", "style", Pick(GRADIENT_STYLES)), ("Rvrs", "reverse", Bool), ("Algn", "alignWithLayer", Bool), ("Scl ", "scale", Num), ("Ofst", "offset", Offset)],
        "stroke" => &[("Styl", "position", Pick(STROKE_POSITIONS)), ("Sz  ", "size", Px)],
        _ => &[],
    }
}

/// A gradient descriptor as ours, and its smoothness (percent).
fn gradient_of(g: &Desc) -> (Gradient, f64) {
    let (colors, opacities, smooth) = gradient_stops(g);
    let colors = if colors.len() >= 2 { colors } else { Gradient::default().colors };
    let opacities = if opacities.len() >= 2 { opacities } else { vec![(0.0, 1.0), (1.0, 1.0)] };
    (Gradient { colors, opacities }, (smooth * 100.0).clamp(0.0, 100.0))
}

fn rgb(c: [f64; 3]) -> Value {
    Value::Color(veymelo_color::rgba(c[0], c[1], c[2], 1.0))
}

fn color_json(c: [f64; 3]) -> Json {
    Json::array(c.iter().map(|v| Json::Number(*v)))
}

/// A setting set to `v`, a note where it cannot be.
fn put(g: &mut Group, id: &str, v: Value, notes: &mut Vec<String>, label: &str) {
    match g.prop_mut(id) {
        Ok(p) => {
            if let Err(e) = p.set_value(v) {
                notes.push(format!("{label}: {} not taken ({e})", p.def.name));
            }
        }
        Err(e) => notes.push(format!("{label}: {e}")),
    }
}

/// An adjustment as one of our effects and its settings (none where we have no such effect).
fn adjustment_effect(a: &Adjustment) -> Option<(&'static str, Vec<(String, Json)>)> {
    let n = |k: &str, v: f64| (k.to_string(), Json::Number(v));
    let b = |k: &str, v: bool| (k.to_string(), Json::Bool(v));
    Some(match a {
        Adjustment::Levels(lv) => {
            let mut p = Vec::new();
            for (c, l) in lv.iter().enumerate().take(4) {
                let prefix = ["", "red", "green", "blue"][c];
                let key = |k: &str| if prefix.is_empty() { k[..1].to_lowercase() + &k[1..] } else { format!("{prefix}{k}") };
                p.extend([n(&key("InputBlack"), l[0]), n(&key("InputWhite"), l[1]), n(&key("Gamma"), l[4].clamp(0.1, 10.0)), n(&key("OutputBlack"), l[2]), n(&key("OutputWhite"), l[3])]);
            }
            ("levels", p)
        }
        Adjustment::Curves(cs) => {
            let p = cs
                .iter()
                .filter_map(|(c, pts)| {
                    let name = ["rgb", "red", "green", "blue"].get(*c as usize)?;
                    let mut pts = pts.clone();
                    pts.sort_by(|a, b| a[0].total_cmp(&b[0]));
                    let text: Vec<String> = pts.iter().map(|q| format!("{},{}", (q[0] * 1e4).round() / 1e4, (q[1] * 1e4).round() / 1e4)).collect();
                    Some((name.to_string(), Json::from(text.join(" "))))
                })
                .collect();
            ("curves", p)
        }
        Adjustment::BrightnessContrast { brightness, contrast, legacy } => ("brightnessContrast", vec![n("brightness", brightness.clamp(-150.0, 150.0)), n("contrast", contrast.clamp(-100.0, 100.0)), b("useLegacy", *legacy)]),
        Adjustment::HueSaturation { colorize, master, ranges } => {
            let mut p = vec![n("masterHue", master[0]), n("masterSaturation", master[1].clamp(-100.0, 100.0)), n("masterLightness", master[2].clamp(-100.0, 100.0))];
            for (r, name) in ranges.iter().zip(["red", "yellow", "green", "cyan", "blue", "magenta"]) {
                let (start, mut end) = (r.range[1], r.range[2]);
                if end < start {
                    end += 360.0;
                }
                p.extend([
                    n(&format!("{name}Hue"), r.hsl[0]),
                    n(&format!("{name}Saturation"), r.hsl[1].clamp(-100.0, 100.0)),
                    n(&format!("{name}Lightness"), r.hsl[2].clamp(-100.0, 100.0)),
                    n(&format!("{name}RangeStart"), start),
                    n(&format!("{name}RangeEnd"), end),
                    n(&format!("{name}StartFalloff"), (r.range[1] - r.range[0]).rem_euclid(360.0).min(180.0)),
                    n(&format!("{name}EndFalloff"), (r.range[3] - r.range[2]).rem_euclid(360.0).min(180.0)),
                ]);
            }
            if let Some([h, s, l]) = colorize {
                p.extend([b("colorize", true), n("colorizeHue", h.rem_euclid(360.0)), n("colorizeSaturation", s.clamp(0.0, 100.0)), n("colorizeLightness", l.clamp(-100.0, 100.0))]);
            }
            ("hueSaturation", p)
        }
        Adjustment::ColorBalance { shadows, midtones, highlights, luminosity } => {
            let mut p = Vec::new();
            for (tone, v) in [("shadow", shadows), ("midtone", midtones), ("highlight", highlights)] {
                for (c, x) in ["Red", "Green", "Blue"].iter().zip(v) {
                    p.push(n(&format!("{tone}{c}Balance"), x.clamp(-100.0, 100.0)));
                }
            }
            p.push(b("preserveLuminosity", *luminosity));
            ("colorBalance", p)
        }
        Adjustment::Invert => ("invert", Vec::new()),
        Adjustment::Threshold(l) => ("threshold", vec![n("level", l.clamp(0.0, 255.0))]),
        Adjustment::Posterize(l) => ("posterize", vec![n("level", l.clamp(2.0, 255.0))]),
        Adjustment::Exposure { exposure, offset, gamma } => ("exposure", vec![n("exposure", exposure.clamp(-20.0, 20.0)), n("offset", offset.clamp(-2.0, 2.0)), n("gammaCorrection", gamma.clamp(0.01, 9.99))]),
        Adjustment::Vibrance { vibrance, saturation } => ("vibrance", vec![n("vibrance", vibrance.clamp(-100.0, 100.0)), n("saturation", saturation.clamp(-100.0, 100.0))]),
        Adjustment::BlackWhite { mix, tint } => {
            let mut p: Vec<(String, Json)> = ["reds", "yellows", "greens", "cyans", "blues", "magentas"].iter().zip(mix).map(|(k, v)| n(k, v.clamp(-200.0, 300.0))).collect();
            if let Some(c) = tint {
                p.extend([b("tint", true), ("tintColor".to_string(), color_json(*c))]);
            }
            ("blackWhite", p)
        }
        Adjustment::PhotoFilter { color, density, luminosity } => {
            ("photoFilter", vec![("filter".to_string(), Json::from("custom")), ("color".to_string(), color_json(*color)), n("density", density.clamp(0.0, 100.0)), b("preserveLuminosity", *luminosity)])
        }
        Adjustment::ChannelMixer { monochrome, rows } => {
            let mut p = vec![b("monochrome", *monochrome)];
            for (out, row) in ["red", "green", "blue"].iter().zip(rows) {
                for (inp, v) in ["Red", "Green", "Blue", "Const"].iter().zip(row) {
                    p.push(n(&format!("{out}{inp}"), v.clamp(-200.0, 200.0)));
                }
            }
            ("channelMixer", p)
        }
        Adjustment::SelectiveColor { absolute, ranges } => {
            let mut p = vec![("method".to_string(), Json::from(if *absolute { "absolute" } else { "relative" }))];
            for (r, v) in ["reds", "yellows", "greens", "cyans", "blues", "magentas", "whites", "neutrals", "blacks"].iter().zip(ranges) {
                for (ink, x) in ["Cyan", "Magenta", "Yellow", "Black"].iter().zip(v) {
                    p.push(n(&format!("{r}{ink}"), x.clamp(-100.0, 100.0)));
                }
            }
            ("selectiveColor", p)
        }
        Adjustment::Other(_) => return None,
    })
}

/// What an adjustment block's key is called, for notes.
fn adjustment_name(key: &[u8; 4]) -> &'static str {
    match key {
        b"grdm" => "a gradient map",
        b"clrL" => "a colour lookup",
        _ => "an adjustment",
    }
}

/// What the import knows as it goes.
struct Import<'a> {
    doc: &'a Psd,
    path: &'a str,
    file: String,
    cropped: bool,
    text_pixels: bool,
    /// Where held smart object files go.
    smart_dir: PathBuf,
    /// The folder the layers' footage and the groups' compositions go in.
    folder: u64,
    notes: Vec<String>,
    layers: usize,
}

/// The new layer's id from what `layer.add` gave back.
fn added(made: &Json) -> u64 {
    made.get("layer").and_then(Json::as_u64).expect("added")
}

fn layer_mut<'c>(cx: &'c mut Ctx, cid: u64, lid: u64) -> &'c mut veymelo_project::Layer {
    cx.project.comp_mut(cid).expect("made").layer_mut(lid).expect("added")
}

fn set(layer: &mut veymelo_project::Layer, path: &str, v: Value) -> Result<()> {
    layer.props.prop_mut(path).map_err(Error::refused)?.set_value(v).map_err(Error::refused)?;
    Ok(())
}

/// A layer just added given a Photoshop layer's look: its opacity (times its fill opacity, unless
/// it has styles, where the fill opacity is the styles' own), blend mode, visibility, clipping.
fn look(cx: &mut Ctx, cid: u64, lid: u64, l: &veymelo_psd::Layer, styled: bool) -> Result<()> {
    let layer = layer_mut(cx, cid, lid);
    let fill = if styled { 1.0 } else { l.fill as f64 / 255.0 };
    let opacity = l.opacity as f64 / 255.0 * fill * 100.0;
    set(layer, "transform/opacity", Value::Number((opacity * 10.0).round() / 10.0))?;
    layer.blend = veymelo_color::BlendMode::from_id(blend(&l.blend)).unwrap_or_default();
    layer.switches.video = !l.hidden;
    layer.switches.collapse = &l.blend == b"pass";
    layer.preserve_transparency = l.clipped;
    Ok(())
}

impl Import<'_> {
    fn note(&mut self, i: usize, what: impl std::fmt::Display) {
        self.notes.push(format!("\"{}\": {what}", self.doc.layers[i].name));
    }

    /// Whether layer `i` has a painted mask of its own (not one drawn from its vector mask).
    fn painted(&self, i: usize) -> bool {
        let l = &self.doc.layers[i];
        let vector = l.info.iter().any(|b| &b.key == b"vmsk" || &b.key == b"vsms");
        l.mask.as_ref().is_some_and(|m| !m.disabled && !(m.rendered && vector))
    }

    /// Footage of layer `i` (its pixels, or its mask), in the layers' folder.
    fn footage(&mut self, cx: &mut Ctx, i: usize, mask: bool) -> u64 {
        let l = &self.doc.layers[i];
        let cropped = self.cropped && !mask;
        let (w, h) = match self.doc.pixels(i) {
            Some(p) if cropped => (p.width as u32, p.height as u32),
            _ => (self.doc.width, self.doc.height),
        };
        let mut f = Footage::new(self.path, FootageKind::Image);
        (f.width, f.height, f.alpha) = (w.max(1), h.max(1), AlphaMode::Straight);
        f.layer = Some(SourceLayer { index: i as u32, name: l.name.clone(), cropped, mask });
        let name = if mask { format!("{} Mask/{}", l.name, self.file) } else { format!("{}/{}", l.name, self.file) };
        cx.project.add(&name, Some(self.folder), ItemKind::Footage(f))
    }

    fn add_footage(&mut self, cx: &mut Ctx, cid: u64, item: u64, name: &str) -> Result<u64> {
        let made = super::call(cx, "layer.add", Json::object([("comp", Json::Number(cid as f64)), ("type", Json::from("footage")), ("item", Json::Number(item as f64)), ("name", Json::from(name))]))?;
        Ok(added(&made))
    }

    /// A pixel layer: footage of it, placed where it is. Its id and where its own pixel (0, 0)
    /// is in the document; none for a layer with no pixels.
    fn pixel_layer(&mut self, cx: &mut Ctx, cid: u64, i: usize) -> Result<Option<(u64, [f64; 2])>> {
        let Some(veymelo_psd::Pixels { left, top, width: w, height: h, .. }) = self.doc.pixels(i) else { return Ok(None) };
        let item = self.footage(cx, i, false);
        let name = self.doc.layers[i].name.clone();
        let lid = self.add_footage(cx, cid, item, &name)?;
        if self.cropped {
            let at = Value::Vec3([left as f64 + w as f64 / 2.0, top as f64 + h as f64 / 2.0, 0.0]);
            set(layer_mut(cx, cid, lid), "transform/position", at)?;
            return Ok(Some((lid, [left as f64, top as f64])));
        }
        Ok(Some((lid, [0.0; 2])))
    }

    /// A fill layer: a solid of its colour (a gradient: with a gradient overlay over it); where
    /// it has a painted mask, footage of the mask filled instead.
    fn fill_layer(&mut self, cx: &mut Ctx, cid: u64, i: usize, fill: &Fill) -> Result<Option<u64>> {
        let name = self.doc.layers[i].name.clone();
        let color = match fill {
            Fill::Solid(c) => *c,
            Fill::Gradient(_) => [1.0; 3],
            Fill::Pattern(_) => {
                self.note(i, "a pattern fill: brought in as its pixels");
                return Ok(None);
            }
        };
        let lid = if self.painted(i) {
            let item = self.footage(cx, i, true);
            let lid = self.add_footage(cx, cid, item, &name)?;
            if matches!(fill, Fill::Solid(_)) {
                super::call(cx, "effect.add", Json::object([("comp", Json::Number(cid as f64)), ("layer", Json::Number(lid as f64)), ("type", Json::from("fill")), ("props", Json::object([("color", color_json(color))]))]))?;
            }
            lid
        } else {
            let made = super::call(
                cx,
                "layer.add",
                Json::object([
                    ("comp", Json::Number(cid as f64)),
                    ("type", Json::from("solid")),
                    ("name", Json::from(name.as_str())),
                    ("color", color_json(color)),
                    ("width", Json::Number(self.doc.width.max(1) as f64)),
                    ("height", Json::Number(self.doc.height.max(1) as f64)),
                ]),
            )?;
            added(&made)
        };
        if let Fill::Gradient(g) = fill {
            let size = [self.doc.width as f64, self.doc.height as f64];
            let mut list = vec![("gradientOverlay", g.clone())];
            self.put_styles(cx, cid, lid, i, &mut list, size, true, 100.0);
        }
        Ok(Some(lid))
    }

    /// An adjustment layer with the effect that does the same; where it has a painted mask,
    /// footage of the mask made an adjustment layer (it changes what is under it where the mask
    /// lets through). None where we have no such effect.
    fn adjustment_layer(&mut self, cx: &mut Ctx, cid: u64, i: usize) -> Result<Option<u64>> {
        let Some(adj) = self.doc.adjustment(i) else {
            self.note(i, "an adjustment layer whose settings are not read: left out");
            return Ok(None);
        };
        let Some((effect, props)) = adjustment_effect(&adj) else {
            let Adjustment::Other(key) = adj else { unreachable!("only others have no effect") };
            self.note(i, format!("{}: left out (there is no effect that does it)", adjustment_name(&key)));
            return Ok(None);
        };
        let name = self.doc.layers[i].name.clone();
        let lid = if self.painted(i) {
            let item = self.footage(cx, i, true);
            let lid = self.add_footage(cx, cid, item, &name)?;
            layer_mut(cx, cid, lid).switches.adjustment = true;
            lid
        } else {
            added(&super::call(cx, "layer.add", Json::object([("comp", Json::Number(cid as f64)), ("type", Json::from("adjustment")), ("name", Json::from(name.as_str()))]))?)
        };
        let props = Json::Object(props.into_iter().collect());
        super::call(cx, "effect.add", Json::object([("comp", Json::Number(cid as f64)), ("layer", Json::Number(lid as f64)), ("type", Json::from(effect)), ("props", props)]))?;
        Ok(Some(lid))
    }

    /// A type layer as a text layer: its words, each run's look, each paragraph's setting, point
    /// or box text, placed and turned by its transform.
    fn text_layer(&mut self, cx: &mut Ctx, cid: u64, i: usize, ty: &TypeLayer) -> Result<u64> {
        let name = self.doc.layers[i].name.clone();
        let [a, b, c, d, e, f] = ty.transform;
        // turned and scaled: a scale the same both ways goes into the sizes
        let sx = a.hypot(b);
        let sy = if sx > 0.0 { (a * d - b * c) / sx } else { 0.0 };
        let uniform = sx > 0.0 && (sx - sy).abs() <= 1e-6 * sx;
        let k = if uniform { sx } else { 1.0 };
        let text = ty.text.strip_suffix('\n').unwrap_or(&ty.text).to_string();
        let made = super::call(cx, "layer.add", Json::object([("comp", Json::Number(cid as f64)), ("type", Json::from("text")), ("name", Json::from(name.as_str())), ("text", Json::from(text.as_str()))]))?;
        let lid = added(&made);
        let count = text.chars().count();
        let mut missing: Vec<String> = Vec::new();
        let mut at = 0usize;
        for (n, s) in &ty.runs {
            let end = (at + n).min(count);
            if end > at {
                let mut o = style_json(s, k);
                if let Some(font) = s.font.as_deref().filter(|f| *f != "AdobeInvisFont") {
                    if veymelo_text::resolve(font, 400.0, false).is_none() && !missing.iter().any(|m| m == font) {
                        missing.push(font.to_string());
                    }
                    o.push(("font", Json::from(font)));
                }
                self.text_set(cx, cid, lid, Some([at, end]), o)?;
            }
            at = end;
        }
        let mut at = 0usize;
        for (n, p) in &ty.paragraphs {
            let end = (at + n).min(count);
            if end > at {
                self.text_set(cx, cid, lid, Some([at, end]), paragraph_json(p, k))?;
            }
            at = end;
        }
        let mut whole: Vec<(&str, Json)> = vec![("vertical", Json::Bool(ty.vertical))];
        // where its own (0, 0) is: point text's first baseline, or the box's top left
        let (mut ox, mut oy) = (0.0, 0.0);
        if let Some([l, t, r, btm]) = ty.boxed {
            (ox, oy) = (l, t);
            whole.push(("box", Json::array([Json::Number(((r - l) * k).max(1.0)), Json::Number(((btm - t) * k).max(1.0))])));
        }
        self.text_set(cx, cid, lid, None, whole)?;
        let layer = layer_mut(cx, cid, lid);
        set(layer, "transform/anchor", Value::Vec3([0.0; 3]))?;
        set(layer, "transform/position", Value::Vec3([a * ox + c * oy + e, b * ox + d * oy + f, 0.0]))?;
        let turn = b.atan2(a).to_degrees();
        if turn.abs() > 1e-9 {
            set(layer, "transform/rotation", Value::Number(turn))?;
        }
        if !uniform {
            set(layer, "transform/scale", Value::Vec3([sx * 100.0, sy * 100.0, 100.0]))?;
        }
        for m in missing {
            self.note(i, format!("its font {m} is not on this computer (another stands in until it is)"));
        }
        Ok(lid)
    }

    fn text_set(&mut self, cx: &mut Ctx, cid: u64, lid: u64, range: Option<[usize; 2]>, fields: Vec<(&str, Json)>) -> Result<()> {
        let mut o: Vec<(&str, Json)> = vec![("comp", Json::Number(cid as f64)), ("layer", Json::Number(lid as f64))];
        if let Some([s, e]) = range {
            o.push(("range", Json::array([Json::Number(s as f64), Json::Number(e as f64)])));
        }
        o.extend(fields);
        super::call(cx, "text.set", Json::object(o))?;
        Ok(())
    }

    /// A smart object as footage of the file it holds (written beside the document) or links,
    /// pinned by its corners. None where that file cannot be had.
    fn smart_layer(&mut self, cx: &mut Ctx, cid: u64, i: usize) -> Result<Option<u64>> {
        let Some(p) = self.doc.placed(i) else { return Ok(None) };
        let files = self.doc.linked_files();
        let Some(linked) = files.iter().find(|l| l.id == p.id) else {
            self.note(i, "a smart object whose file is not in the document: brought in as its pixels");
            return Ok(None);
        };
        let path = match self.doc.linked_data(linked) {
            Some(bytes) => match self.write_held(linked, bytes) {
                Ok(path) => path,
                Err(e) => {
                    self.note(i, format!("its file could not be written ({e}): brought in as its pixels"));
                    return Ok(None);
                }
            },
            None => match linked.path.as_deref().filter(|p| Path::new(p).exists()) {
                Some(p) => p.to_string(),
                None => {
                    self.note(i, format!("a smart object linking {}, which is not there: brought in as its pixels", linked.path.as_deref().unwrap_or(&linked.name)));
                    return Ok(None);
                }
            },
        };
        if !veymelo_media::importable(&path) {
            self.note(i, format!("a smart object holding {}, not a kind of file footage is made from: brought in as its pixels", linked.name));
            return Ok(None);
        }
        let mut args = vec![("path", Json::from(path.as_str())), ("folder", Json::Number(self.folder as f64))];
        if let Some(page) = p.page.filter(|_| veymelo_media::has_pages(&path)) {
            args.push(("page", Json::Number(page as f64)));
        }
        let item = match super::call(cx, "footage.import", Json::object(args)) {
            Ok(made) => made.get("item").and_then(Json::as_u64).expect("imported"),
            Err(e) => {
                self.note(i, format!("its file could not be read ({}): brought in as its pixels", e.message));
                return Ok(None);
            }
        };
        let name = self.doc.layers[i].name.clone();
        let lid = self.add_footage(cx, cid, item, &name)?;
        let layer = layer_mut(cx, cid, lid);
        set(layer, "transform/anchor", Value::Vec3([0.0; 3]))?;
        set(layer, "transform/position", Value::Vec3([0.0; 3]))?;
        let point = |q: [f64; 2]| Json::array([Json::Number(q[0]), Json::Number(q[1])]);
        let [ul, ur, lr, ll] = p.corners;
        let pins = Json::object([("upperLeft", point(ul)), ("upperRight", point(ur)), ("lowerLeft", point(ll)), ("lowerRight", point(lr))]);
        super::call(cx, "effect.add", Json::object([("comp", Json::Number(cid as f64)), ("layer", Json::Number(lid as f64)), ("type", Json::from("cornerPin")), ("props", pins)]))?;
        if p.warped {
            self.note(i, "its warp is not followed (it is pinned by its corners)");
        }
        if p.filtered {
            self.note(i, "its smart filters are left out");
        }
        Ok(Some(lid))
    }

    /// A held file written into the smart objects' folder (the same bytes kept once). Its path.
    fn write_held(&self, l: &veymelo_psd::smart::Linked, bytes: &[u8]) -> std::result::Result<String, String> {
        let ext = match &l.kind {
            b"png " => "png",
            b"JPEG" => "jpg",
            b"8BPS" => "psd",
            b"8BPB" => "psb",
            b"PDF " => "pdf",
            b"TIFF" => "tif",
            b"GIFf" => "gif",
            b"SVG " => "svg",
            _ => "",
        };
        let base: String = Path::new(&l.name).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default().chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '\0')).collect();
        let base = if base.is_empty() { l.id.chars().filter(char::is_ascii_alphanumeric).collect() } else { base };
        let base = if Path::new(&base).extension().is_none() && !ext.is_empty() { format!("{base}.{ext}") } else { base };
        std::fs::create_dir_all(&self.smart_dir).map_err(|e| e.to_string())?;
        let (stem, dot) = match base.rsplit_once('.') {
            Some((s, x)) => (s.to_string(), format!(".{x}")),
            None => (base.clone(), String::new()),
        };
        for n in 1..1000 {
            let name = if n == 1 { base.clone() } else { format!("{stem} {n}{dot}") };
            let p = self.smart_dir.join(name);
            match std::fs::read(&p) {
                Ok(have) if have == bytes => return Ok(p.to_string_lossy().into_owned()),
                Ok(_) => continue,
                Err(_) => {
                    std::fs::write(&p, bytes).map_err(|e| e.to_string())?;
                    return Ok(p.to_string_lossy().into_owned());
                }
            }
        }
        Err("too many files of that name".into())
    }

    /// Layer `i`'s vector mask as masks on layer `lid` (whose own pixel (0, 0) is at `origin`
    /// in the document).
    fn vector(&mut self, cx: &mut Ctx, cid: u64, lid: u64, i: usize, origin: [f64; 2]) {
        let Some(v) = self.doc.vector_mask(i) else { return };
        if v.paths.is_empty() {
            return;
        }
        // each subpath's mode as our masks join (top to bottom): a shape merged into the one
        // before makes holes where they overlap
        let mut masks: Vec<(ShapePath, &str, bool)> = Vec::new();
        let mut first = true;
        for p in &v.paths {
            let mode = match (p.join, p.merged) {
                (Join::Add, false) => "add",
                (Join::Subtract, false) => "subtract",
                // a first shape that intersects or excludes starts from all, or from none
                (Join::Intersect, false) if first => "add",
                (Join::Intersect, false) => "intersect",
                (Join::Exclude, false) if first => "add",
                (Join::Exclude, false) => "difference",
                (Join::Add | Join::Exclude, true) => "difference",
                (Join::Subtract, true) => "add",
                (Join::Intersect, true) => "subtract",
            };
            first = false;
            let place = |q: [f64; 2]| [q[0] - origin[0], q[1] - origin[1]];
            let path = ShapePath {
                points: p.knots.iter().map(|k| place(k.at)).collect(),
                in_tangents: p.knots.iter().map(|k| [k.before[0] - k.at[0], k.before[1] - k.at[1]]).collect(),
                out_tangents: p.knots.iter().map(|k| [k.after[0] - k.at[0], k.after[1] - k.at[1]]).collect(),
                closed: true,
                feather: Feathers::NONE,
            };
            masks.push((path, mode, false));
        }
        // inverted: what is left is what was hidden, each join turned about
        if v.inverted {
            let starts_all = masks[0].1 == "subtract";
            for m in &mut masks {
                (m.1, m.2) = match m.1 {
                    "add" => ("subtract", false),
                    "subtract" => ("add", false),
                    "intersect" => ("add", true),
                    other => (other, false),
                };
            }
            if !starts_all && masks[0].1 != "subtract" {
                let (w, h) = (self.doc.width as f64, self.doc.height as f64);
                let all = [[-w, -h], [2.0 * w, -h], [2.0 * w, 2.0 * h], [-w, 2.0 * h]].map(|q| [q[0] - origin[0], q[1] - origin[1]]);
                masks.insert(0, (ShapePath { points: all.to_vec(), in_tangents: vec![[0.0; 2]; 4], out_tangents: vec![[0.0; 2]; 4], closed: true, feather: Feathers::NONE }, "add", false));
            }
        }
        let l = &self.doc.layers[i];
        let (feather, density) = (l.vector_feather * 2.0, l.vector_density);
        // a lessened density: one shape kept by what is outside it, faded
        let single = masks.len() == 1 && masks[0].1 == "add" && !masks[0].2;
        let mut notes = Vec::new();
        if density < 1.0 && !single {
            notes.push(format!("its vector mask's density ({:.0}%) is not followed", density * 100.0));
        }
        if v.disabled {
            notes.push("its vector mask is off (its masks are kept, set to none)".to_string());
        }
        let ids: Vec<u64> = masks.iter().map(|_| cx.project.alloc()).collect();
        let layer = layer_mut(cx, cid, lid);
        let Ok(list) = layer.props.group_mut("masks") else {
            notes.push("its vector mask is left out (the layer takes no masks)".to_string());
            for n in notes {
                self.note(i, n);
            }
            return;
        };
        let count = masks.len();
        for (k, ((path, mode, inverted), id)) in masks.into_iter().zip(ids).enumerate() {
            let name = if count == 1 { "Vector Mask".to_string() } else { format!("Vector Mask {}", k + 1) };
            let mut g = Group::instance(&MASK, id, &name);
            let (mode, inverted, opacity) = if single && density < 1.0 { ("subtract", true, density * 100.0) } else { (mode, inverted, 100.0) };
            let mode = if v.disabled { "none" } else { mode };
            let choice = MASK_MODES.iter().position(|m| *m == mode).unwrap_or(1) as u32;
            for (id, value) in [("path", Value::Path(path)), ("mode", Value::Choice(choice)), ("inverted", Value::Bool(inverted)), ("feather", Value::Vec2([feather; 2])), ("opacity", Value::Number(opacity))] {
                if let Ok(p) = g.prop_mut(id) {
                    p.value = value;
                }
            }
            list.children.push(veymelo_project::Node::Group(g));
        }
        for n in notes {
            self.note(i, n);
        }
    }

    /// Layer `i`'s effects as layer styles on layer `lid` (`size` the layer's, for offsets).
    /// Whether it has any.
    fn styles(&mut self, cx: &mut Ctx, cid: u64, lid: u64, i: usize, size: [f64; 2]) -> bool {
        let Some(fx) = self.doc.effects(i) else { return false };
        let mut list = fx.list.clone();
        self.put_styles(cx, cid, lid, i, &mut list, size, fx.shown, fx.scale)
    }

    /// Effects (kind and settings) as layer styles, with Blending Options holding the
    /// document's light and the layer's fill opacity. Whether any were put.
    #[allow(clippy::too_many_arguments)]
    fn put_styles(&mut self, cx: &mut Ctx, cid: u64, lid: u64, i: usize, list: &mut Vec<(&'static str, Desc)>, size: [f64; 2], shown: bool, scale: f64) -> bool {
        let mut notes = Vec::new();
        let mut made: Vec<Group> = Vec::new();
        for (kind, d) in list.drain(..) {
            let Some(def) = styles::find(kind) else {
                notes.push(format!("its {kind} effect is left out (there is no such style)"));
                continue;
            };
            if made.iter().any(|g| std::ptr::eq(g.def, def)) {
                notes.push(format!("it has more than one {}: the first is kept", def.name));
                continue;
            }
            let label = def.name;
            let mut g = Group::instance(def, cx.project.alloc(), def.name);
            g.enabled = shown && d.flag("enab").unwrap_or(true);
            let common: [(&str, &str, Conv); 3] = [("Md  ", "blendMode", Conv::Mode), ("Clr ", "color", Conv::Color), ("Opct", "opacity", Conv::Num)];
            for &(key, id, conv) in common.iter().chain(settings(kind)) {
                let Some(item) = d.get(key) else { continue };
                // a colour overlay's opacity and a glow's colour where it is a gradient
                if g.prop(id).is_err() {
                    continue;
                }
                let num = d.num(key);
                let value = match conv {
                    Conv::Num => num.map(Value::Number),
                    Conv::Px => num.map(|v| Value::Number(v * scale / 100.0)),
                    Conv::Bool => d.flag(key).map(Value::Bool),
                    Conv::Color => d.color(key).map(rgb),
                    Conv::Mode => {
                        let ours = d.enumeration(key).and_then(|m| DESC_MODES.iter().find(|(p, _)| *p == m)).map(|(_, o)| *o);
                        match ours.and_then(|o| MODES.iter().position(|m| *m == o)) {
                            Some(k) => Some(Value::Choice(k as u32)),
                            None => {
                                notes.push(format!("{label}: its blend mode ({}) is not one a style has; normal is used", ours.unwrap_or("unknown")));
                                Some(Value::Choice(0))
                            }
                        }
                    }
                    Conv::Pick(table) => {
                        let ours = d.enumeration(key).and_then(|m| table.iter().find(|(p, _)| *p == m)).map(|(_, o)| *o);
                        let options = g.prop(id).ok().and_then(|p| if let Kind::Choice(o) = p.def.kind { Some(o) } else { None }).unwrap_or(&[]);
                        ours.and_then(|o| options.iter().position(|x| *x == o)).map(|k| Value::Choice(k as u32))
                    }
                    Conv::Grad => match item {
                        Item::Desc(gd) => {
                            let (grad, smooth) = gradient_of(gd);
                            put(&mut g, "gradientSmoothness", Value::Number(smooth), &mut notes, label);
                            if g.prop("colorType").is_ok() {
                                put(&mut g, "colorType", Value::Choice(1), &mut notes, label);
                            }
                            Some(Value::Gradient(grad))
                        }
                        _ => None,
                    },
                    Conv::Offset => d.desc(key).map(|o| Value::Vec2([o.num("Hrzn").unwrap_or(0.0) / 100.0 * size[0], o.num("Vrtc").unwrap_or(0.0) / 100.0 * size[1]])),
                };
                if let Some(v) = value {
                    put(&mut g, id, v, &mut notes, label);
                }
            }
            if kind == "stroke" && d.enumeration("PntT").is_some_and(|t| t != "SClr") {
                notes.push("its stroke is filled with a gradient or pattern: drawn in one colour".to_string());
            }
            made.push(g);
        }
        let any = !made.is_empty();
        if any {
            let (angle, altitude) = self.doc.global_light();
            let fill = self.doc.layers[i].fill as f64 / 255.0 * 100.0;
            let bo_id = cx.project.alloc();
            let layer = layer_mut(cx, cid, lid);
            match layer.props.group_mut(LIST) {
                Ok(styles_list) => {
                    let mut bo = Group::instance(&BLENDING_OPTIONS, bo_id, BLENDING_OPTIONS.name);
                    put(&mut bo, "globalLightAngle", Value::Number(angle), &mut notes, "Blending Options");
                    put(&mut bo, "globalLightAltitude", Value::Number(altitude.clamp(0.0, 90.0)), &mut notes, "Blending Options");
                    put(&mut bo, "advancedBlending/fillOpacity", Value::Number((fill * 10.0).round() / 10.0), &mut notes, "Blending Options");
                    styles_list.children.retain(|n| n.as_group().is_none_or(|g| g.def.id != styles::BLENDING && made.iter().all(|m| m.def.id != g.def.id)));
                    styles_list.children.insert(0, veymelo_project::Node::Group(bo));
                    made.sort_by_key(|g| styles::rank(g.def.id));
                    for g in made {
                        let at = styles_list.children.iter().position(|n| n.as_group().is_some_and(|o| styles::rank(o.def.id) > styles::rank(g.def.id))).unwrap_or(styles_list.children.len());
                        styles_list.children.insert(at, veymelo_project::Node::Group(g));
                    }
                }
                Err(_) => notes.push("its effects are left out (the layer takes no styles)".to_string()),
            }
        }
        for n in notes {
            self.note(i, n);
        }
        any
    }

    /// A track matte of layer `i`'s painted mask for layer `lid` (just above it, not seen itself).
    fn matte(&mut self, cx: &mut Ctx, cid: u64, lid: u64, i: usize) -> Result<()> {
        let item = self.footage(cx, i, true);
        let name = format!("{} Mask", self.doc.layers[i].name);
        let mid = self.add_footage(cx, cid, item, &name)?;
        layer_mut(cx, cid, mid).switches.video = false;
        layer_mut(cx, cid, lid).matte = Some(Matte { layer: mid, kind: MatteKind::Alpha });
        Ok(())
    }

    /// A composition of `nodes` (top first), called `name`, in `folder`. Its id.
    fn comp(&mut self, cx: &mut Ctx, nodes: &[Node], name: &str, folder: Option<u64>) -> Result<u64> {
        let comp = Comp::new(self.doc.width.max(1), self.doc.height.max(1), FrameRate::FPS_30, Tick::from_seconds(10.0));
        let cid = cx.project.add(name, folder, ItemKind::Comp(std::sync::Arc::new(comp)));
        let doc_size = [self.doc.width as f64, self.doc.height as f64];
        // bottom first, each new layer going on top
        for node in nodes.iter().rev() {
            match node {
                Node::Layer(i) => {
                    let i = *i;
                    let l = &self.doc.layers[i];
                    let fill = self.doc.fill(i);
                    // the layer, where its own pixel (0, 0) is, whether a painted mask still
                    // needs a matte, and its size (for offsets)
                    let mut made: Option<(u64, [f64; 2], bool)> = None;
                    if let Some(f) = &fill {
                        made = self.fill_layer(cx, cid, i, f)?.map(|lid| (lid, [0.0; 2], false));
                    } else if l.adjustment {
                        match self.adjustment_layer(cx, cid, i)? {
                            Some(lid) => made = Some((lid, [0.0; 2], false)),
                            None => continue,
                        }
                    } else if l.text && !self.text_pixels {
                        match self.doc.type_layer(i) {
                            Some(Ok(ty)) => made = Some((self.text_layer(cx, cid, i, &ty)?, [0.0; 2], self.painted(i))),
                            Some(Err(e)) => self.note(i, format!("its text could not be read ({e}): brought in as its pixels")),
                            None => {}
                        }
                    } else if l.smart {
                        made = self.smart_layer(cx, cid, i)?.map(|lid| (lid, [0.0; 2], self.painted(i)));
                    }
                    let made = match made {
                        Some(m) => m,
                        None => match self.pixel_layer(cx, cid, i)? {
                            Some((lid, origin)) => {
                                if l.text && self.text_pixels {
                                    self.note(i, "a type layer: brought in as its pixels");
                                }
                                (lid, origin, false)
                            }
                            None => continue,
                        },
                    };
                    let (lid, origin, needs_matte) = made;
                    let l = &self.doc.layers[i];
                    let [top, left, bottom, right] = l.bounds;
                    let size = if right > left && bottom > top { [(right - left) as f64, (bottom - top) as f64] } else { doc_size };
                    // adjustment layers take no styles; fill layers do
                    let styled = (fill.is_some() || !l.adjustment) && self.styles(cx, cid, lid, i, size);
                    self.vector(cx, cid, lid, i, origin);
                    look(cx, cid, lid, &self.doc.layers[i], styled)?;
                    if needs_matte {
                        self.matte(cx, cid, lid, i)?;
                    }
                    self.layers += 1;
                }
                Node::Group(i, kids) => {
                    let g = &self.doc.layers[*i];
                    let inner = self.comp(cx, kids, &g.name.clone(), Some(self.folder))?;
                    let made = super::call(cx, "layer.add", Json::object([("comp", Json::Number(cid as f64)), ("type", Json::from("comp")), ("item", Json::Number(inner as f64)), ("name", Json::from(self.doc.layers[*i].name.as_str()))]))?;
                    let lid = added(&made);
                    let styled = self.styles(cx, cid, lid, *i, doc_size);
                    self.vector(cx, cid, lid, *i, [0.0; 2]);
                    look(cx, cid, lid, &self.doc.layers[*i], styled)?;
                    if self.painted(*i) {
                        self.matte(cx, cid, lid, *i)?;
                    }
                }
            }
        }
        Ok(cid)
    }
}

/// A run's look as `text.set` takes it (sizes times `k`).
fn style_json(s: &Style, k: f64) -> Vec<(&'static str, Json)> {
    let color = |c: [f64; 4]| Json::array(c.iter().map(|v| Json::Number(*v)));
    let mut o = vec![
        ("size", Json::Number((s.size * k).max(0.1))),
        ("fill", s.fill.map_or(Json::Null, color)),
        ("stroke", s.stroke.map_or(Json::Null, color)),
        ("tracking", Json::Number(s.tracking)),
        ("leading", s.leading.map_or(Json::from("auto"), |l| Json::Number(l * k))),
        ("baselineShift", Json::Number(s.baseline_shift * k)),
        ("caps", Json::from(["normal", "smallCaps", "allCaps"][s.caps.min(2) as usize])),
        ("position", Json::from(["normal", "superscript", "subscript"][s.position.min(2) as usize])),
        ("underline", Json::Bool(s.underline)),
        ("strikethrough", Json::Bool(s.strikethrough)),
        ("hScale", Json::Number(s.h_scale)),
        ("vScale", Json::Number(s.v_scale)),
        ("fauxBold", Json::Bool(s.faux_bold)),
        ("fauxItalic", Json::Bool(s.faux_italic)),
        ("ligatures", Json::Bool(s.ligatures)),
        ("kerning", s.kerning.map_or(Json::from("metrics"), Json::Number)),
    ];
    if s.stroke.is_some() {
        o.push(("strokeWidth", Json::Number(s.stroke_width * k)));
    }
    o
}

/// A paragraph's setting as `text.set` takes it.
fn paragraph_json(p: &Paragraph, k: f64) -> Vec<(&'static str, Json)> {
    let justify = ["left", "right", "center", "justifyLeft", "justifyRight", "justifyCenter", "justifyAll"][p.justification.min(6) as usize];
    vec![
        ("justify", Json::from(justify)),
        ("indentLeft", Json::Number(p.start_indent * k)),
        ("indentRight", Json::Number(p.end_indent * k)),
        ("indentFirst", Json::Number(p.first_indent * k)),
        ("spaceBefore", Json::Number(p.space_before * k)),
        ("spaceAfter", Json::Number(p.space_after * k)),
    ]
}

/// The Photoshop document at `path` as a composition, called `name` (default: the file's), in
/// `folder`.
pub(super) fn import(cx: &mut Ctx, path: &str, how: How, name: Option<&str>, folder: Option<u64>) -> Result<Json> {
    let doc = veymelo_media::layered(path).map_err(Error::not_found)?;
    let file = Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
    let stem = name.map(str::to_string).unwrap_or_else(|| Path::new(&file).file_stem().map_or(file.clone(), |s| s.to_string_lossy().into_owned()));
    let doc_stem = Path::new(&file).file_stem().map_or(file.clone(), |s| s.to_string_lossy().into_owned());
    let smart_dir = match how.smart_folder {
        Some(f) => PathBuf::from(f),
        None => Path::new(path).parent().map(Path::to_path_buf).unwrap_or_default().join(format!("{doc_stem} smart objects")),
    };
    let layers_folder = cx.project.add(&format!("{stem} Layers"), folder, ItemKind::Folder);
    let mut im = Import { doc: &doc, path, file, cropped: how.cropped, text_pixels: how.text_pixels, smart_dir, folder: layers_folder, notes: Vec::new(), layers: 0 };
    let tree = doc.tree();
    let cid = im.comp(cx, &tree, &stem, folder)?;
    cx.label = Some(format!("Import {stem}"));
    Ok(Json::object([
        ("item", Json::Number(cid as f64)),
        ("name", Json::from(stem.as_str())),
        ("layers", Json::Number(im.layers as f64)),
        ("notes", Json::array(im.notes.iter().map(|n| Json::from(n.as_str())))),
    ]))
}

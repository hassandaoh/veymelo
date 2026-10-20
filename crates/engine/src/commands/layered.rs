//! A Photoshop document brought in as a composition of its layers: each pixel layer footage of
//! just that layer (the document's size, or its own placed where it is), each group a composition
//! of what is in it (a pass-through group collapsed, so its blend modes reach below it), each
//! layer's opacity (times its fill), blend mode, visibility and clipping kept. Adjustment and fill
//! layers, which draw from their settings rather than pixels of their own, are left out and
//! listed; type layers and smart objects come in as their pixels.

use veymelo_json::Value as Json;
use veymelo_keyframe::Value;
use veymelo_project::{AlphaMode, Comp, Footage, FootageKind, ItemKind, SourceLayer};
use veymelo_psd::{Node, Psd};
use veymelo_time::{FrameRate, Tick};

use crate::{Ctx, Error, Result};

/// How a layered file may come in.
pub(super) const IMPORT_AS: &[&str] = &["footage", "composition", "compositionLayerSizes"];

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

/// What the import knows as it goes.
struct Import<'a> {
    doc: &'a Psd,
    path: &'a str,
    file: String,
    cropped: bool,
    /// The folder the layers' footage and the groups' compositions go in.
    folder: u64,
    notes: Vec<String>,
    layers: usize,
}

/// A layer just added to composition `cid` given a Photoshop layer's look.
fn look(cx: &mut Ctx, cid: u64, lid: u64, l: &veymelo_psd::Layer) -> Result<()> {
    let comp = cx.project.comp_mut(cid).expect("made");
    let layer = comp.layer_mut(lid).expect("added");
    let opacity = l.opacity as f64 / 255.0 * l.fill as f64 / 255.0 * 100.0;
    layer.props.prop_mut("transform/opacity").map_err(Error::refused)?.set_value(Value::Number((opacity * 10.0).round() / 10.0)).map_err(Error::refused)?;
    layer.blend = veymelo_color::BlendMode::from_id(blend(&l.blend)).unwrap_or_default();
    layer.switches.video = !l.hidden;
    layer.switches.collapse = &l.blend == b"pass";
    layer.preserve_transparency = l.clipped;
    Ok(())
}

impl Import<'_> {
    /// A composition of `nodes` (top first), called `name`, in `folder`. Its id.
    fn comp(&mut self, cx: &mut Ctx, nodes: &[Node], name: &str, folder: Option<u64>) -> Result<u64> {
        let comp = Comp::new(self.doc.width.max(1), self.doc.height.max(1), FrameRate::FPS_30, Tick::from_seconds(10.0));
        let cid = cx.project.add(name, folder, ItemKind::Comp(std::sync::Arc::new(comp)));
        // bottom first, each new layer going on top
        for node in nodes.iter().rev() {
            match node {
                Node::Layer(i) => {
                    let l = &self.doc.layers[*i];
                    // an adjustment or fill layer draws from its settings, not pixels of its own
                    if l.adjustment {
                        self.notes.push(format!("\"{}\" is an adjustment or fill layer: left out", l.name));
                        continue;
                    }
                    let Some(veymelo_psd::Pixels { left, top, width: w, height: h, .. }) = self.doc.pixels(*i) else { continue };
                    let (fw, fh) = if self.cropped { (w as u32, h as u32) } else { (self.doc.width, self.doc.height) };
                    let mut f = Footage::new(self.path, FootageKind::Image);
                    (f.width, f.height, f.alpha) = (fw, fh, AlphaMode::Straight);
                    f.layer = Some(SourceLayer { index: *i as u32, name: l.name.clone(), cropped: self.cropped });
                    let item = cx.project.add(&format!("{}/{}", l.name, self.file), Some(self.folder), ItemKind::Footage(f));
                    let made = super::call(cx, "layer.add", Json::object([("comp", Json::Number(cid as f64)), ("type", Json::from("footage")), ("item", Json::Number(item as f64)), ("name", Json::from(l.name.as_str()))]))?;
                    let lid = made.get("layer").and_then(Json::as_u64).expect("added");
                    if self.cropped {
                        let at = Value::Vec3([left as f64 + w as f64 / 2.0, top as f64 + h as f64 / 2.0, 0.0]);
                        let layer = cx.project.comp_mut(cid).expect("made").layer_mut(lid).expect("added");
                        layer.props.prop_mut("transform/position").map_err(Error::refused)?.set_value(at).map_err(Error::refused)?;
                    }
                    if l.text || l.smart {
                        self.notes.push(format!("\"{}\" is a {} layer: brought in as its pixels", l.name, if l.text { "type" } else { "smart object" }));
                    }
                    look(cx, cid, lid, l)?;
                    self.layers += 1;
                }
                Node::Group(i, kids) => {
                    let g = &self.doc.layers[*i];
                    let inner = self.comp(cx, kids, &g.name, Some(self.folder))?;
                    let made = super::call(cx, "layer.add", Json::object([("comp", Json::Number(cid as f64)), ("type", Json::from("comp")), ("item", Json::Number(inner as f64)), ("name", Json::from(g.name.as_str()))]))?;
                    let lid = made.get("layer").and_then(Json::as_u64).expect("added");
                    look(cx, cid, lid, g)?;
                }
            }
        }
        Ok(cid)
    }
}

/// The Photoshop document at `path` as a composition (its layers each `cropped` to their own
/// rectangle, or the document's size), called `name` (default: the file's), in `folder`.
pub(super) fn import(cx: &mut Ctx, path: &str, cropped: bool, name: Option<&str>, folder: Option<u64>) -> Result<Json> {
    let doc = veymelo_media::layered(path).map_err(Error::not_found)?;
    let file = std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
    let stem = name.map(str::to_string).unwrap_or_else(|| std::path::Path::new(&file).file_stem().map_or(file.clone(), |s| s.to_string_lossy().into_owned()));
    let layers_folder = cx.project.add(&format!("{stem} Layers"), folder, ItemKind::Folder);
    let mut im = Import { doc: &doc, path, file, cropped, folder: layers_folder, notes: Vec::new(), layers: 0 };
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

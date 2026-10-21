//! A composition as a Lottie document.

use std::collections::{HashMap, HashSet};

use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::Value;
use veymelo_project::{AutoOrient, Comp, FootageKind, Group, ItemKind, Layer, MatteKind, Node, Project, Source};
use veymelo_time::Tick;

use crate::values::{Ctx, num, nums, prop_json, same, times};
use crate::{MASK_MODES, base64, fx_out, shapes, text};

/// How a composition goes out.
#[derive(Clone, Debug)]
pub struct Options {
    /// Expressions go along (as `x`; players that run expressions run them).
    pub expressions: bool,
    /// Text as the outlines of its letters: the look exact and no font needed where it plays,
    /// but the words are as they are at the layer's start.
    pub text_as_shapes: bool,
    /// Text kept as text, its letters' outlines carried in the document (`chars`), so players
    /// need no font (each character as its font draws it alone, without the shaping that joins
    /// letters).
    pub embed_glyphs: bool,
    /// Pictures inside the document; else beside it, in `images/`.
    pub embed_images: bool,
    /// Only the work area plays.
    pub work_area: bool,
    /// The composition's background colour as a solid under everything (Lottie has none).
    pub background: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options { expressions: false, text_as_shapes: false, embed_glyphs: false, embed_images: true, work_area: false, background: false }
    }
}

/// A Lottie document and what could not go into it.
#[derive(Clone, Debug)]
pub struct Exported {
    pub json: Json,
    pub warnings: Vec<String>,
    /// Pictures the document names but does not hold: its name for each (in `images/`) and
    /// the footage's file.
    pub images: Vec<(String, String)>,
}

struct Doc<'a> {
    project: &'a Project,
    options: &'a Options,
    read: &'a dyn Fn(&str) -> Option<Vec<u8>>,
    cx: Ctx,
    assets: Vec<Json>,
    /// Asset ids made (or being made).
    made: HashSet<String>,
    fonts: Vec<Json>,
    /// The documents of the text kept as text, for the glyphs the document carries.
    glyph_docs: Vec<veymelo_keyframe::TextDoc>,
    three_d: bool,
    images: Vec<(String, String)>,
}

fn hex(c: veymelo_color::Rgba) -> String {
    let b = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c.r), b(c.g), b(c.b))
}

fn mime_of(path: &str) -> (&'static str, &'static str) {
    let lower = path.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "jpg" | "jpeg" => ("image/jpeg", "jpg"),
        "webp" => ("image/webp", "webp"),
        "gif" => ("image/gif", "gif"),
        "svg" => ("image/svg+xml", "svg"),
        _ => ("image/png", "png"),
    }
}

/// A sound file's media type, by its name.
fn sound_mime(path: &str) -> &'static str {
    match path.to_ascii_lowercase().rsplit('.').next().unwrap_or("") {
        "wav" => "audio/wav",
        "m4a" | "mp4" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" => "audio/ogg",
        "aif" | "aiff" => "audio/aiff",
        "flac" => "audio/flac",
        _ => "audio/mpeg",
    }
}

/// Audio levels in dB as Lottie's: how loud, in % (100 as it is).
fn level_out(v: &Value) -> Value {
    let pct = |db: f64| if db <= -96.0 { 0.0 } else { (100.0 * 10f64.powf(db / 20.0) * 1000.0).round() / 1000.0 };
    match v {
        Value::Vec2([l, r]) => Value::Vec2([pct(*l), pct(*r)]),
        other => other.clone(),
    }
}

fn still(v: Json) -> Json {
    Json::object([("a", Json::from(0.0)), ("k", v)])
}

/// What a matte layer is to the layer it mattes, as Lottie numbers it.
fn matte_code(k: MatteKind) -> f64 {
    match k {
        MatteKind::Alpha => 1.0,
        MatteKind::AlphaInverted => 2.0,
        MatteKind::Luma => 3.0,
        MatteKind::LumaInverted => 4.0,
    }
}

impl Doc<'_> {
    fn put(&mut self, o: &mut Map, key: &str, g: &Group, id: &str, label: &str) {
        if let Ok(p) = g.prop(id) {
            let v = prop_json(&mut self.cx, p, &same, &format!("{label} › {}", p.def.name));
            o.insert(key, v);
        }
    }

    fn transform(&mut self, l: &Layer) -> Json {
        let mut ks = Map::new();
        let tr = l.transform();
        let label = format!("{} › Transform", l.name);
        let scene = matches!(l.source, Source::Camera | Source::Light(_));
        if scene {
            if l.auto_orient == AutoOrient::TowardsPointOfInterest {
                self.put(&mut ks, "a", tr, "pointOfInterest", &label);
            }
        } else {
            self.put(&mut ks, "a", tr, "anchor", &label);
        }
        if l.separate_position {
            let mut p = Map::new();
            p.insert("s", true);
            for (key, id) in [("x", "positionX"), ("y", "positionY"), ("z", "positionZ")] {
                if key != "z" || l.is_3d() {
                    self.put(&mut p, key, tr, id, &label);
                }
            }
            ks.insert("p", Json::Object(p));
        } else {
            self.put(&mut ks, "p", tr, "position", &label);
        }
        self.put(&mut ks, "s", tr, "scale", &label);
        if l.is_3d() {
            for (key, id) in [("or", "orientation"), ("rx", "rotationX"), ("ry", "rotationY"), ("rz", "rotation")] {
                self.put(&mut ks, key, tr, id, &label);
            }
        } else {
            self.put(&mut ks, "r", tr, "rotation", &label);
        }
        self.put(&mut ks, "o", tr, "opacity", &label);
        Json::Object(ks)
    }

    fn masks(&mut self, l: &Layer) -> Vec<Json> {
        let Ok(masks) = l.props.group("masks") else { return Vec::new() };
        let mut out = Vec::new();
        for m in masks.children.iter().filter_map(Node::as_group) {
            let label = format!("{} › {}", l.name, m.name());
            let mode = m.prop("mode").map_or(1, |p| p.value.as_choice()) as usize;
            let mut o = Map::new();
            o.insert("nm", m.name());
            o.insert("mode", if m.enabled { MASK_MODES.get(mode).copied().unwrap_or("a") } else { "n" });
            o.insert("inv", m.prop("inverted").is_ok_and(|p| p.value.as_bool()));
            // RotoBezier: the curves it makes for itself, written out
            let smoothed;
            let m = if m.prop("rotoBezier").is_ok_and(|p| p.value.as_bool()) {
                let mut c = m.clone();
                if let Ok(p) = c.prop_mut("path") {
                    for v in std::iter::once(&mut p.value).chain(p.keys.iter_mut().map(|k| &mut k.value)) {
                        if let Value::Path(sp) = v {
                            *sp = sp.roto_smoothed();
                        }
                    }
                }
                smoothed = c;
                &smoothed
            } else {
                m
            };
            for (key, id) in [("pt", "path"), ("o", "opacity"), ("x", "expansion")] {
                self.put(&mut o, key, m, id, &label);
            }
            let softening_points = m.prop("path").is_ok_and(|p| std::iter::once(&p.value).chain(p.keys.iter().map(|k| &k.value)).any(|v| matches!(v, Value::Path(sp) if !sp.feather.is_empty())));
            if softening_points || m.prop("feather").is_ok_and(|p| p.is_animated() || p.value.as_vec2() != [0.0; 2]) {
                self.cx.warn(format!("{label}: Lottie masks have no feather (a hard edge)"));
            }
            out.push(Json::Object(o));
        }
        out
    }

    fn effects(&mut self, l: &Layer, inds: &HashMap<u64, usize>) -> Vec<Json> {
        let Ok(list) = l.props.group("effects") else { return Vec::new() };
        let mut out = Vec::new();
        for (i, g) in list.children.iter().filter_map(Node::as_group).enumerate() {
            let label = format!("{} › {}", l.name, g.name());
            let Some(fx) = fx_out(g.def.id) else {
                self.cx.warn(format!("{label}: Lottie players do not draw {} (left out)", g.def.name));
                continue;
            };
            let mut params = Vec::new();
            for (k, param) in fx.params.iter().enumerate() {
                let Ok(p) = g.prop(param.id) else { continue };
                let plabel = format!("{label} › {}", p.def.name);
                let v = if param.ty == 10 {
                    let layer = |v: &Value| Value::Number(v.as_layer().and_then(|id| inds.get(&id)).map_or(0.0, |i| *i as f64));
                    prop_json(&mut self.cx, p, &layer, &plabel)
                } else {
                    prop_json(&mut self.cx, p, &times(param.scale), &plabel)
                };
                params.push(Json::object([("ty", Json::from(f64::from(param.ty))), ("nm", Json::from(p.def.name)), ("mn", Json::from(param.id)), ("ix", Json::from((k + 1) as f64)), ("v", v)]));
            }
            let mut o = Map::new();
            o.insert("ty", f64::from(fx.ty));
            o.insert("nm", g.name());
            o.insert("mn", fx.id);
            o.insert("ix", (i + 1) as f64);
            o.insert("en", g.enabled as u8 as f64);
            o.insert("np", params.len() as f64);
            o.insert("ef", Json::Array(params));
            out.push(Json::Object(o));
        }
        out
    }

    /// A sound asset, its file inside the document. Its id.
    fn sound(&mut self, item: u64, l: &Layer) -> Option<String> {
        let id = format!("audio_{item}");
        if self.made.contains(&id) {
            return Some(id);
        }
        let it = self.project.item(item)?;
        let ItemKind::Footage(f) = &it.kind else { return None };
        let Some(bytes) = (self.read)(&f.path) else {
            self.cx.warn(format!("{}: its sound {} could not be read (a hidden null instead)", l.name, f.path));
            return None;
        };
        let mut a = Map::new();
        a.insert("id", id.as_str());
        a.insert("nm", it.name.as_str());
        a.insert("u", "");
        a.insert("p", format!("data:{};base64,{}", sound_mime(&f.path), base64::encode(&bytes)));
        a.insert("e", 1.0);
        self.assets.push(Json::Object(a));
        self.made.insert(id.clone());
        Some(id)
    }

    fn image(&mut self, item: u64, l: &Layer) -> Option<String> {
        let id = format!("image_{item}");
        if self.made.contains(&id) {
            return Some(id);
        }
        let it = self.project.item(item)?;
        let ItemKind::Footage(f) = &it.kind else { return None };
        let (mime, ext) = mime_of(&f.path);
        let mut a = Map::new();
        a.insert("id", id.as_str());
        a.insert("nm", it.name.as_str());
        a.insert("w", f.width as f64);
        a.insert("h", f.height as f64);
        let bytes = if self.options.embed_images { (self.read)(&f.path) } else { None };
        if let Some(bytes) = bytes {
            a.insert("u", "");
            a.insert("p", format!("data:{mime};base64,{}", base64::encode(&bytes)));
            a.insert("e", 1.0);
        } else if self.options.embed_images {
            // named where it is (a player that can reach it there shows it)
            self.cx.warn(format!("{}: its picture {} could not be read, so the document names it instead", l.name, f.path));
            let (dir, file) = match f.path.rfind('/') {
                Some(i) => (&f.path[..=i], &f.path[i + 1..]),
                None => ("", f.path.as_str()),
            };
            a.insert("u", dir);
            a.insert("p", file);
            a.insert("e", 0.0);
        } else {
            let name = format!("img_{item}.{ext}");
            a.insert("u", "images/");
            a.insert("p", name.as_str());
            a.insert("e", 0.0);
            self.images.push((name, f.path.clone()));
        }
        self.assets.push(Json::Object(a));
        self.made.insert(id.clone());
        Some(id)
    }

    fn precomp(&mut self, item: u64) -> Option<String> {
        let id = format!("comp_{item}");
        if self.made.contains(&id) {
            return Some(id);
        }
        let it = self.project.item(item)?;
        let c = it.as_comp()?;
        self.made.insert(id.clone());
        let layers = self.layers(c);
        let a = Json::object([("id", Json::from(id.as_str())), ("nm", Json::from(it.name.as_str())), ("fr", num(c.frame_rate.as_f64())), ("w", Json::from(c.width as f64)), ("h", Json::from(c.height as f64)), ("layers", Json::Array(layers))]);
        self.assets.push(a);
        Some(id)
    }

    fn layers(&mut self, comp: &Comp) -> Vec<Json> {
        let mattes: HashSet<u64> = comp.layers.iter().filter_map(|l| l.matte.map(|m| m.layer)).collect();
        let inds: HashMap<u64, usize> = comp.layers.iter().enumerate().map(|(i, l)| (l.id, i + 1)).collect();
        let mut out = Vec::with_capacity(comp.layers.len());
        for (i, l) in comp.layers.iter().enumerate() {
            out.push(self.layer(comp, i, l, &mattes, &inds));
        }
        out
    }

    fn layer(&mut self, comp: &Comp, idx: usize, l: &Layer, mattes: &HashSet<u64>, inds: &HashMap<u64, usize>) -> Json {
        let name = l.name.as_str();
        let mut o = Map::new();
        o.insert("ind", (idx + 1) as f64);
        o.insert("nm", name);
        let mut hidden = !l.switches.video || l.switches.guide;
        // a null unless it is something Lottie has
        let mut ty = 3.0;
        match &l.source {
            Source::Shape => {
                ty = 4.0;
                let items = l.props.group("contents").map(|c| shapes::items_out(&mut self.cx, c, name)).unwrap_or_default();
                o.insert("shapes", Json::Array(items));
            }
            Source::Text if self.options.text_as_shapes => {
                ty = 4.0;
                o.insert("shapes", Json::Array(text::outlines(&mut self.cx, l)));
            }
            Source::Text => {
                ty = 5.0;
                let t = text::text_out(&mut self.cx, l, &mut self.fonts);
                o.insert("t", t);
                if self.options.embed_glyphs
                    && let Ok(st) = l.props.prop("text/sourceText")
                {
                    self.glyph_docs.extend(std::iter::once(&st.value).chain(st.keys.iter().map(|k| &k.value)).filter_map(Value::doc));
                }
            }
            Source::Solid { .. } if l.switches.adjustment => {
                self.cx.warn(format!("{name}: Lottie has no adjustment layers (a hidden null instead)"));
                hidden = true;
            }
            Source::Solid { color, width, height } => {
                ty = 1.0;
                o.insert("sc", hex(*color));
                o.insert("sw", *width as f64);
                o.insert("sh", *height as f64);
                if color.a < 1.0 {
                    self.cx.warn(format!("{name}: Lottie solids are solid (its colour's alpha is left out)"));
                }
            }
            Source::Comp(item) => match self.precomp(*item) {
                Some(id) => {
                    ty = 0.0;
                    let (w, h) = self.project.comp(*item).map_or((comp.width, comp.height), |c| (c.width, c.height));
                    o.insert("refId", id);
                    o.insert("w", w as f64);
                    o.insert("h", h as f64);
                    if l.time_remap
                        && let Ok(p) = l.props.prop("timeRemap")
                    {
                        let v = prop_json(&mut self.cx, p, &same, &format!("{name} › Time Remap"));
                        o.insert("tm", v);
                    }
                }
                None => hidden = true,
            },
            Source::Model(_) | Source::Primitive(_) => {
                self.cx.warn(format!("{name}: Lottie has no 3D models or solids (a hidden null instead)"));
                hidden = true;
            }
            Source::Footage(item) => {
                let kind = self.project.item(*item).and_then(|i| if let ItemKind::Footage(f) = &i.kind { Some(f.kind) } else { None });
                match kind {
                    Some(FootageKind::Image) => match self.image(*item, l) {
                        Some(id) => {
                            ty = 2.0;
                            o.insert("refId", id);
                        }
                        None => hidden = true,
                    },
                    Some(FootageKind::Audio) => match self.sound(*item, l) {
                        Some(id) => {
                            // a sound layer: its file, and how loud (Lottie's level is a %)
                            ty = 6.0;
                            o.insert("refId", id);
                            if let Ok(p) = l.props.prop("audio/levels") {
                                let lv = prop_json(&mut self.cx, p, &level_out, &format!("{name} › Audio Levels"));
                                o.insert("au", Json::object([("lv", lv)]));
                            }
                        }
                        None => hidden = true,
                    },
                    _ => {
                        self.cx.warn(format!("{name}: Lottie carries no video or picture sequences (a hidden null instead)"));
                        hidden = true;
                    }
                }
            }
            Source::Null => {}
            Source::Camera => {
                ty = 13.0;
                if let Ok(p) = l.props.prop("cameraOptions/zoom") {
                    let v = prop_json(&mut self.cx, p, &same, &format!("{name} › Zoom"));
                    o.insert("pe", v);
                }
                if l.props.prop("cameraOptions/depthOfField").is_ok_and(|p| p.value.as_bool() || p.is_animated()) {
                    self.cx.warn(format!("{name}: Lottie cameras have no depth of field"));
                }
            }
            Source::Light(_) => {
                self.cx.warn(format!("{name}: Lottie has no lights (a hidden null instead, so its children still follow it)"));
                hidden = true;
            }
        }
        o.insert("ty", ty);
        let three = l.is_3d();
        if three {
            self.three_d = true;
        }
        o.insert("ddd", three as u8 as f64);
        if let Some(i) = l.parent.and_then(|p| inds.get(&p)) {
            o.insert("parent", *i as f64);
        }
        o.insert("sr", num(l.stretch / 100.0));
        if l.stretch < 0.0 {
            self.cx.warn(format!("{name}: most Lottie players cannot play a layer backwards"));
        }
        let ks = self.transform(l);
        o.insert("ks", ks);
        o.insert("ao", (l.auto_orient == AutoOrient::AlongPath) as u8 as f64);
        if l.auto_orient == AutoOrient::TowardsCamera {
            self.cx.warn(format!("{name}: Lottie layers do not turn toward the camera"));
        } else if l.auto_orient == AutoOrient::TowardsPointOfInterest && !matches!(l.source, Source::Camera | Source::Light(_)) {
            self.cx.warn(format!("{name}: only Lottie cameras aim at a point"));
        }
        o.insert("ip", num(self.cx.frame(l.in_point)));
        o.insert("op", num(self.cx.frame(l.out_point)));
        o.insert("st", num(self.cx.frame(l.start)));
        let bm = crate::blend_out(l.blend).unwrap_or_else(|| {
            self.cx.warn(format!("{name}: Lottie has no {} blend mode (normal is used)", l.blend.id()));
            0
        });
        o.insert("bm", bm as f64);
        let masks = self.masks(l);
        if !masks.is_empty() {
            o.insert("hasMask", true);
            o.insert("masksProperties", Json::Array(masks));
        }
        let ef = self.effects(l, inds);
        if !ef.is_empty() {
            o.insert("ef", Json::Array(ef));
        }
        let sy = crate::styles::export(&mut self.cx, l);
        if !sy.is_empty() {
            o.insert("sy", Json::Array(sy));
        }
        if let Some(m) = l.matte
            && let Some(&mi) = inds.get(&m.layer)
        {
            o.insert("tt", matte_code(m.kind));
            o.insert("tp", mi as f64);
            if mi != idx {
                self.cx.warn(format!("{name}: its matte is not the layer just above it (players older than the matte's `tp` use the one above)"));
            }
        }
        if mattes.contains(&l.id) {
            o.insert("td", 1.0);
            if l.switches.video {
                self.cx.warn(format!("{name}: Lottie never shows a matte layer itself"));
            }
        } else if hidden {
            o.insert("hd", true);
        }
        if !l.markers.is_empty() {
            self.cx.warn(format!("{name}: Lottie has no layer markers (composition markers go along)"));
        }
        if l.props.group(veymelo_project::styles::LIST).is_ok_and(|g| g.children.iter().filter_map(Node::as_group).any(|s| s.enabled && s.def.id != veymelo_project::styles::BLENDING)) {
            self.cx.warn(format!("{name}: its layer styles go along, but most Lottie players do not draw them"));
        }
        if l.preserve_transparency {
            self.cx.warn(format!("{name}: Lottie cannot draw only where the layers under it are"));
        }
        if l.switches.motion_blur && comp.enable_motion_blur {
            self.cx.warn(format!("{name}: Lottie has no motion blur"));
        }
        Json::Object(o)
    }
}

/// Composition `comp` of `project` as a Lottie document. `read` reads footage files (for
/// pictures that go inside the document).
pub fn export(project: &Project, comp: u64, options: &Options, read: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Exported, String> {
    let item = project.item(comp).ok_or_else(|| format!("there is no item {comp}"))?;
    let c = item.as_comp().ok_or_else(|| format!("{} is not a composition", item.name))?;
    let mut doc = Doc {
        project,
        options,
        read,
        cx: Ctx { rate: c.frame_rate, expressions: options.expressions, warnings: Vec::new() },
        assets: Vec::new(),
        made: HashSet::from([format!("comp_{comp}")]),
        fonts: Vec::new(),
        glyph_docs: Vec::new(),
        three_d: false,
        images: Vec::new(),
    };
    let mut layers = doc.layers(c);
    if options.background {
        let (w, h) = (c.width as f64, c.height as f64);
        let mut ks = Map::new();
        ks.insert("a", still(nums(&[w / 2.0, h / 2.0, 0.0])));
        ks.insert("p", still(nums(&[w / 2.0, h / 2.0, 0.0])));
        ks.insert("s", still(nums(&[100.0, 100.0, 100.0])));
        ks.insert("r", still(num(0.0)));
        ks.insert("o", still(num(100.0)));
        let end = doc.cx.frame(c.duration);
        let bg = Json::object([
            ("ind", Json::from((layers.len() + 1) as f64)),
            ("ty", Json::from(1.0)),
            ("nm", Json::from("Background")),
            ("sc", Json::from(hex(c.background))),
            ("sw", Json::from(w)),
            ("sh", Json::from(h)),
            ("ks", Json::Object(ks)),
            ("ip", Json::from(0.0)),
            ("op", num(end)),
            ("st", Json::from(0.0)),
            ("sr", Json::from(1.0)),
            ("ao", Json::from(0.0)),
            ("bm", Json::from(0.0)),
            ("ddd", Json::from(0.0)),
        ]);
        layers.push(bg);
    }
    if doc.three_d {
        doc.cx.warn("3D layers are drawn flat by most Lottie players (the web player's HTML renderer draws them in depth)");
    }
    if c.pixel_aspect != 1.0 {
        doc.cx.warn("Lottie pixels are square (the composition's pixel aspect is left out)");
    }
    let (ip, op) = if options.work_area { (c.work_area.start, c.work_area.end()) } else { (Tick::ZERO, c.duration) };
    let markers: Vec<Json> = c
        .markers
        .iter()
        .map(|m| Json::object([("tm", num(doc.cx.frame(m.time))), ("cm", Json::from(m.comment.as_str())), ("dr", num(doc.cx.frame(m.duration)))]))
        .collect();
    let mut out = Map::new();
    out.insert("v", "5.12.0");
    out.insert("fr", num(c.frame_rate.as_f64()));
    out.insert("ip", num(doc.cx.frame(ip)));
    out.insert("op", num(doc.cx.frame(op)));
    out.insert("w", c.width as f64);
    out.insert("h", c.height as f64);
    out.insert("nm", item.name.as_str());
    out.insert("ddd", doc.three_d as u8 as f64);
    out.insert("assets", Json::Array(doc.assets));
    out.insert("layers", Json::Array(layers));
    out.insert("markers", Json::Array(markers));
    if !doc.fonts.is_empty() {
        out.insert("fonts", Json::object([("list", Json::Array(doc.fonts))]));
    }
    if !doc.glyph_docs.is_empty() {
        let chars = text::chars_out(&mut doc.cx, &doc.glyph_docs);
        out.insert("chars", Json::Array(chars));
    }
    Ok(Exported { json: Json::Object(out), warnings: doc.cx.warnings, images: doc.images })
}

/// A document packed as a `.lottie` archive: `manifest.json`, `animations/<id>.json` and the
/// pictures it names beside it (`images/<name>`, each name with its bytes).
pub fn dotlottie(e: &Exported, id: &str, images: &[(String, Vec<u8>)]) -> Vec<u8> {
    let id: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    let id = if id.is_empty() { "animation".to_string() } else { id };
    let mut json = e.json.clone();
    // inside the archive, pictures are found from its top
    if let Some(assets) = json.get_mut("assets").and_then(Json::as_array_mut) {
        for a in assets {
            if a.get("u").and_then(Json::as_str) == Some("images/")
                && let Some(o) = a.as_object_mut()
            {
                o.insert("u", "/images/");
            }
        }
    }
    let manifest = Json::object([
        ("version", Json::from("1")),
        ("generator", Json::from("Veymelo")),
        ("animations", Json::Array(vec![Json::object([("id", Json::from(id.as_str())), ("speed", Json::from(1.0)), ("loop", Json::from(true))])])),
    ]);
    let (manifest, doc) = (manifest.to_json(), json.to_json());
    let mut files: Vec<(String, &[u8])> = vec![("manifest.json".into(), manifest.as_bytes()), (format!("animations/{id}.json"), doc.as_bytes())];
    for (name, bytes) in images {
        files.push((format!("images/{name}"), bytes));
    }
    let named: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), *b)).collect();
    veymelo_zip::zip(&named)
}

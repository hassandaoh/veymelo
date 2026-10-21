//! A Lottie document as a new composition (the way back of [`crate::export`]).

use std::collections::HashMap;
use std::path::Path;

use veymelo_color::Rgba;
use veymelo_json::Value as Json;
use veymelo_keyframe::Value;
use veymelo_project::{AutoOrient, Comp, Footage, FootageKind, Group, ItemKind, Layer, MASK, Marker, Matte, MatteKind, Node, Project, Source};
use veymelo_time::{FrameRate, Tick, TimeRange};

use crate::values::{self, Ctx, color_of, flag_of, over, path_value, prop_from, vec2_of, vec3_or};
use crate::{MASK_MODES, base64, fx_in, shapes, text};

/// A picture a Lottie document carries, for the caller to keep in a file.
#[derive(Clone, Debug)]
pub struct Picture {
    /// Its asset id in the document, and its name.
    pub id: String,
    pub name: String,
    pub bytes: Vec<u8>,
    /// What kind of file it is: `png`, `jpg`…
    pub ext: String,
}

/// What an import made.
#[derive(Clone, Debug)]
pub struct Imported {
    /// The new composition (an archive's first animation).
    pub comp: u64,
    /// Every animation's composition, the first first (an archive can hold many).
    pub comps: Vec<u64>,
    /// Every item made (compositions, pictures, their folder).
    pub items: Vec<u64>,
    pub warnings: Vec<String>,
}

fn f(j: &Json, k: &str) -> Option<f64> {
    j.get(k).and_then(Json::as_f64)
}

fn s<'a>(j: &'a Json, k: &str) -> Option<&'a str> {
    j.get(k).and_then(Json::as_str)
}

/// Files by name, with their bytes.
pub type Files = Vec<(String, Vec<u8>)>;

/// The Lottie document in a file (its JSON, or a `.lottie` archive's first animation) and the
/// pictures an archive holds beside it, by their names in it.
pub fn document(bytes: &[u8]) -> Result<(Json, Files), String> {
    let (mut docs, pictures) = documents(bytes)?;
    Ok((docs.remove(0).1, pictures))
}

/// Every animation in a file (a `.json` is one; a `.lottie` archive lists its animations in
/// its manifest, the first first), each with its id in the archive, and the pictures the
/// archive holds.
pub fn documents(bytes: &[u8]) -> Result<(Vec<(String, Json)>, Files), String> {
    if !bytes.starts_with(b"PK") {
        let text = std::str::from_utf8(bytes).map_err(|_| "not a Lottie document (not text)".to_string())?;
        let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
        let json = Json::parse(text).map_err(|e| format!("not a Lottie document: {e}"))?;
        return Ok((vec![(String::new(), json)], Vec::new()));
    }
    let files = veymelo_zip::unzip(bytes)?;
    let is_anim = |n: &str| (n.starts_with("animations/") || n.starts_with("a/")) && n.ends_with(".json");
    let id_of = |n: &str| n.rsplit('/').next().unwrap_or(n).trim_end_matches(".json").to_string();
    let manifest = files.iter().find(|(n, _)| n == "manifest.json").and_then(|(_, b)| std::str::from_utf8(b).ok()).and_then(|t| Json::parse(t).ok());
    let listed: Vec<String> = manifest.as_ref().and_then(|m| m.get("animations")?.as_array().cloned()).unwrap_or_default().iter().filter_map(|a| a.get("id")?.as_str().map(str::to_string)).collect();
    // the manifest's order, then any it does not list
    let mut order: Vec<&(String, Vec<u8>)> = listed.iter().filter_map(|id| files.iter().find(|(n, _)| is_anim(n) && id_of(n) == *id)).collect();
    for f in files.iter().filter(|(n, _)| is_anim(n)) {
        if !order.iter().any(|o| o.0 == f.0) {
            order.push(f);
        }
    }
    if order.is_empty() {
        return Err("a .lottie archive with no animation in it".into());
    }
    let mut docs = Vec::new();
    for (n, bytes) in order {
        let text = std::str::from_utf8(bytes).map_err(|_| format!("{n} is not text"))?;
        let json = Json::parse(text).map_err(|e| format!("{n} is not a Lottie document: {e}"))?;
        docs.push((id_of(n), json));
    }
    let pictures = files.iter().filter(|(n, _)| n.starts_with("images/") || n.starts_with("i/")).cloned().collect();
    Ok((docs, pictures))
}

struct In<'a> {
    cx: Ctx,
    /// Asset ids, and the items they became (with their size).
    assets: HashMap<String, (u64, [u32; 2])>,
    fonts: Vec<(String, String, String)>,
    /// The glyphs the document carries (`chars`).
    chars: Vec<Json>,
    items: Vec<u64>,
    folder: Option<u64>,
    name: String,
    store: &'a mut dyn FnMut(&Picture) -> Option<String>,
}

impl In<'_> {
    fn folder(&mut self, project: &mut Project) -> u64 {
        if let Some(f) = self.folder {
            return f;
        }
        let f = project.add(&format!("{} Assets", self.name), None, ItemKind::Folder);
        self.items.push(f);
        self.folder = Some(f);
        f
    }
}

/// Import a Lottie document (`.json`, or every animation of a `.lottie` archive) into
/// `project` as new compositions, called `name` unless they name themselves. `beside` is the
/// folder the document is in, for pictures kept beside it; `store` keeps the pictures it
/// carries in files and says where.
pub fn import(project: &mut Project, bytes: &[u8], name: &str, beside: Option<&Path>, store: &mut dyn FnMut(&Picture) -> Option<String>) -> Result<Imported, String> {
    let (docs, packed) = documents(bytes)?;
    let many = docs.len() > 1;
    let (mut comps, mut items, mut warnings) = (Vec::new(), Vec::new(), Vec::new());
    for (id, doc) in &docs {
        let fallback = if id.is_empty() { name.to_string() } else { id.clone() };
        let one = import_one(project, doc, &fallback, beside, &packed, store).map_err(|e| if many { format!("{id}: {e}") } else { e })?;
        comps.push(one.comp);
        items.extend(one.items);
        warnings.extend(one.warnings.into_iter().map(|w| if many { format!("{id}: {w}") } else { w }));
    }
    Ok(Imported { comp: comps[0], comps, items, warnings })
}

fn import_one(project: &mut Project, doc: &Json, name: &str, beside: Option<&Path>, packed: &Files, store: &mut dyn FnMut(&Picture) -> Option<String>) -> Result<Imported, String> {
    let doc = doc.clone();
    let (Some(w), Some(h)) = (f(&doc, "w"), f(&doc, "h")) else { return Err("not a Lottie document: it has no size (w, h)".into()) };
    if !(w >= 1.0 && h >= 1.0 && w <= veymelo_project::MAX_SIZE as f64 && h <= veymelo_project::MAX_SIZE as f64) {
        return Err(format!("a Lottie document {w} × {h} pixels is too big or too small"));
    }
    let fps = f(&doc, "fr").filter(|v| *v > 0.0 && *v <= 999.0).unwrap_or(30.0);
    let rate = FrameRate::from_f64(fps).unwrap_or(FrameRate::FPS_30);
    let name = s(&doc, "nm").filter(|n| !n.trim().is_empty()).unwrap_or(name).to_string();
    let ip = f(&doc, "ip").unwrap_or(0.0);
    let op = f(&doc, "op").unwrap_or(ip + fps).max(ip + 1.0);
    let mut im = In { cx: Ctx { rate, expressions: true, warnings: Vec::new() }, assets: HashMap::new(), fonts: Vec::new(), chars: doc.get("chars").and_then(Json::as_array).cloned().unwrap_or_default(), items: Vec::new(), folder: None, name: name.clone(), store };
    let duration = im.cx.tick(op).min(Tick::from_seconds(veymelo_project::MAX_SECONDS));
    if let Some(list) = doc.get("fonts").and_then(|f| f.get("list")).and_then(Json::as_array) {
        for fo in list {
            let key = s(fo, "fName").unwrap_or("").to_string();
            let family = s(fo, "fFamily").unwrap_or(&key).to_string();
            im.fonts.push((key, family, s(fo, "fStyle").unwrap_or("Regular").to_string()));
        }
    }
    let (w, h) = (w.round() as u32, h.round() as u32);
    // the assets: precomps first made empty (so layers can show them), pictures kept
    let assets = doc.get("assets").and_then(Json::as_array).cloned().unwrap_or_default();
    for a in &assets {
        let Some(id) = s(a, "id") else { continue };
        let nm = s(a, "nm").filter(|n| !n.is_empty()).unwrap_or(id).to_string();
        let size = |d: [u32; 2]| [f(a, "w").map_or(d[0], |v| v.round().clamp(1.0, 30_000.0) as u32), f(a, "h").map_or(d[1], |v| v.round().clamp(1.0, 30_000.0) as u32)];
        if a.get("layers").is_some() {
            let folder = im.folder(project);
            let [aw, ah] = size([w, h]);
            let c = Comp::new(aw, ah, rate, duration);
            let item = project.add(&nm, Some(folder), ItemKind::Comp(std::sync::Arc::new(c)));
            im.items.push(item);
            im.assets.insert(id.to_string(), (item, [aw, ah]));
        } else if let Some(p) = s(a, "p").filter(|p| sound(p).is_some()) {
            // a sound: kept as a file, footage of it
            let u = s(a, "u").unwrap_or("");
            let ext = sound(p).unwrap_or("mp3");
            let path = match base64::data_uri(p) {
                Some((_, bytes)) => (im.store)(&Picture { id: id.to_string(), name: nm.clone(), bytes, ext: ext.into() }),
                None => beside.map(|b| b.join(u.trim_start_matches('/')).join(p)).filter(|f| f.is_file()).map(|f| f.to_string_lossy().into_owned()),
            };
            let Some(path) = path else {
                im.cx.warn(format!("the sound {nm} could not be kept (left out)"));
                continue;
            };
            let folder = im.folder(project);
            let mut foot = Footage::new(&path, FootageKind::Audio);
            (foot.has_audio, foot.duration) = (true, duration);
            let item = project.add(&nm, Some(folder), ItemKind::Footage(foot));
            im.items.push(item);
            im.assets.insert(id.to_string(), (item, [0, 0]));
        } else if let Some(p) = s(a, "p") {
            let u = s(a, "u").unwrap_or("");
            let path = if let Some((mime, bytes)) = base64::data_uri(p) {
                let ext = match mime.as_str() {
                    "image/jpeg" | "image/jpg" => "jpg",
                    "image/webp" => "webp",
                    "image/gif" => "gif",
                    "image/svg+xml" => "svg",
                    _ => "png",
                };
                (im.store)(&Picture { id: id.to_string(), name: nm.clone(), bytes, ext: ext.into() })
            } else if let Some((_, bytes)) = packed.iter().find(|(n, _)| n.ends_with(&format!("/{p}")) && (n.starts_with("images/") || n.starts_with("i/"))) {
                let ext = p.rsplit('.').next().unwrap_or("png").to_ascii_lowercase();
                (im.store)(&Picture { id: id.to_string(), name: nm.clone(), bytes: bytes.clone(), ext })
            } else {
                let web = u.starts_with("http://") || u.starts_with("https://") || p.starts_with("http://") || p.starts_with("https://");
                let file = beside.filter(|_| !web).map(|b| b.join(u.trim_start_matches('/')).join(p));
                match file.filter(|f| f.is_file()) {
                    Some(f) => Some(f.to_string_lossy().into_owned()),
                    None => {
                        let why = if web { "is on the web (not fetched)" } else { "is not beside the document" };
                        im.cx.warn(format!("{nm}: the picture {u}{p} {why}, so it is kept as missing footage"));
                        Some(format!("{u}{p}"))
                    }
                }
            };
            let Some(path) = path else {
                im.cx.warn(format!("the picture {nm} could not be kept (left out)"));
                continue;
            };
            let [aw, ah] = size([1, 1]);
            let folder = im.folder(project);
            let mut foot = Footage::new(&path, FootageKind::Image);
            (foot.width, foot.height, foot.frame_rate) = (aw, ah, rate);
            let item = project.add(&nm, Some(folder), ItemKind::Footage(foot));
            im.items.push(item);
            im.assets.insert(id.to_string(), (item, [aw, ah]));
        } else {
            im.cx.warn(format!("the asset {id} is of a kind that is not read"));
        }
    }
    // then what each precomp holds
    for a in &assets {
        let (Some(id), Some(layers)) = (s(a, "id"), a.get("layers").and_then(Json::as_array)) else { continue };
        let Some(&(item, _)) = im.assets.get(id) else { continue };
        let Some(mut c) = project.comp(item).cloned() else { continue };
        let end = layers.iter().filter_map(|l| f(l, "op")).fold(0.0f64, f64::max);
        if end > 0.0 {
            c.duration = im.cx.tick(end).min(Tick::from_seconds(veymelo_project::MAX_SECONDS));
            c.work_area = TimeRange::new(Tick::ZERO, c.duration);
        }
        c.layers = build_layers(&mut im, project, &c, layers);
        if let Some(slot) = project.comp_mut(item) {
            *slot = c;
        }
    }
    let mut comp = Comp::new(w, h, rate, duration);
    if ip > 0.0 {
        comp.work_area = TimeRange::from_bounds(im.cx.tick(ip), duration);
    }
    if let Some(ms) = doc.get("markers").and_then(Json::as_array) {
        for m in ms {
            comp.markers.push(Marker { time: im.cx.tick(f(m, "tm").unwrap_or(0.0)), duration: im.cx.tick(f(m, "dr").unwrap_or(0.0).max(0.0)), comment: s(m, "cm").unwrap_or("").to_string(), ..Marker::default() });
        }
    }
    let layers = doc.get("layers").and_then(Json::as_array).cloned().unwrap_or_default();
    comp.layers = build_layers(&mut im, project, &comp, &layers);
    let id = project.add_comp(&name, comp);
    im.items.push(id);
    Ok(Imported { comp: id, comps: vec![id], items: im.items, warnings: im.cx.warnings })
}

/// A sound asset's kind of file (by its media type or name), if it is a sound.
fn sound(p: &str) -> Option<&'static str> {
    let lower = p.to_ascii_lowercase();
    let kind = match lower.strip_prefix("data:") {
        Some(rest) => rest.split([';', ',']).next().unwrap_or("").to_string(),
        None => lower.rsplit('.').next().unwrap_or("").to_string(),
    };
    Some(match kind.as_str() {
        "audio/mpeg" | "audio/mp3" | "mp3" => "mp3",
        "audio/wav" | "audio/x-wav" | "audio/wave" | "wav" => "wav",
        "audio/mp4" | "audio/x-m4a" | "m4a" => "m4a",
        "audio/aac" | "aac" => "aac",
        "audio/ogg" | "ogg" | "oga" => "ogg",
        "audio/aiff" | "audio/x-aiff" | "aif" | "aiff" => "aif",
        "audio/flac" | "flac" => "flac",
        _ => return None,
    })
}

/// What a Lottie layer shows, and how big it is.
fn source_of(im: &mut In, j: &Json, comp: &Comp) -> Option<(Source, [u32; 2])> {
    let ty = f(j, "ty").unwrap_or(-1.0) as i64;
    let nm = s(j, "nm").unwrap_or("a layer");
    let asset = |im: &In| s(j, "refId").and_then(|r| im.assets.get(r).copied());
    let full = [comp.width, comp.height];
    Some(match ty {
        0 => match asset(im) {
            Some((item, size)) => (Source::Comp(item), size),
            None => {
                im.cx.warn(format!("{nm}: the composition it shows is not in the document (left out)"));
                return None;
            }
        },
        1 => {
            let side = |k: &str, d: u32| f(j, k).map_or(d, |v| v.round().clamp(1.0, 30_000.0) as u32);
            let (sw, sh) = (side("sw", comp.width), side("sh", comp.height));
            let color = s(j, "sc").and_then(Rgba::from_hex).unwrap_or(Rgba::BLACK);
            (Source::Solid { color, width: sw, height: sh }, [sw, sh])
        }
        2 => match asset(im) {
            Some((item, size)) => (Source::Footage(item), size),
            None => {
                im.cx.warn(format!("{nm}: its picture is not in the document (left out)"));
                return None;
            }
        },
        3 => (Source::Null, [100, 100]),
        4 => (Source::Shape, full),
        5 => (Source::Text, full),
        13 => (Source::Camera, full),
        6 => match asset(im) {
            Some((item, size)) => (Source::Footage(item), size),
            None => {
                im.cx.warn(format!("{nm}: its sound is not in the document (left out)"));
                return None;
            }
        },
        other => {
            im.cx.warn(format!("{nm}: a layer of type {other} is not read"));
            return None;
        }
    })
}

/// A Lottie `ks` into a layer's transform.
fn transform_in(cx: &mut Ctx, ks: &Json, l: &mut Layer) {
    let camera = matches!(l.source, Source::Camera);
    let label = format!("{} › Transform", l.name);
    let Ok(tr) = l.props.group_mut("transform") else { return };
    let mut set = |key: &str, id: &str, conv: &dyn Fn(&Json) -> Option<Value>| {
        if let (Some(j), Ok(p)) = (ks.get(key), tr.prop_mut(id)) {
            prop_from(cx, j, p, conv, &label);
        }
    };
    set("a", if camera { "pointOfInterest" } else { "anchor" }, &vec3_or(0.0));
    set("s", "scale", &vec3_or(100.0));
    set(if ks.get("r").is_some() { "r" } else { "rz" }, "rotation", &values::number);
    set("rx", "rotationX", &values::number);
    set("ry", "rotationY", &values::number);
    set("or", "orientation", &vec3_or(0.0));
    set("o", "opacity", &values::number);
    if let Some(p) = ks.get("p") {
        if p.get("s").and_then(Json::as_bool) == Some(true) {
            for (key, id) in [("x", "positionX"), ("y", "positionY"), ("z", "positionZ")] {
                if let (Some(j), Ok(prop)) = (p.get(key), tr.prop_mut(id)) {
                    prop_from(cx, j, prop, &values::number, &label);
                }
            }
            l.separate_position = true;
        } else {
            set("p", "position", &vec3_or(0.0));
        }
    }
    if ks.get("sk").is_some_and(|k| values::numbers(k.get("k").unwrap_or(&Json::Null)).iter().any(|v| *v != 0.0) || k.get("a").and_then(Json::as_f64) == Some(1.0)) {
        cx.warn(format!("{}: layer skew is not read", l.name));
    }
    if camera && ks.get("a").is_some() {
        l.auto_orient = AutoOrient::TowardsPointOfInterest;
    }
}

fn masks_in(cx: &mut Ctx, list: &[Json], l: &mut Layer, ids: &mut dyn FnMut() -> u64) {
    let label = l.name.clone();
    let Ok(g) = l.props.group_mut("masks") else { return };
    for (i, m) in list.iter().enumerate() {
        let name = s(m, "nm").filter(|n| !n.is_empty()).map_or_else(|| format!("Mask {}", i + 1), str::to_string);
        let mut mg = Group::instance(&MASK, ids(), &name);
        let mode = MASK_MODES.iter().position(|c| Some(*c) == s(m, "mode")).unwrap_or(1);
        if let Ok(p) = mg.prop_mut("mode") {
            let _ = p.set_value(Value::Choice(mode as u32));
        }
        if let Ok(p) = mg.prop_mut("inverted") {
            let _ = p.set_value(Value::Bool(m.get("inv").and_then(Json::as_bool).unwrap_or(false)));
        }
        let label = format!("{label} › {name}");
        for (key, id, conv) in [("pt", "path", &path_value as &dyn Fn(&Json) -> Option<Value>), ("o", "opacity", &values::number), ("x", "expansion", &values::number)] {
            if let (Some(j), Ok(p)) = (m.get(key), mg.prop_mut(id)) {
                prop_from(cx, j, p, conv, &label);
            }
        }
        g.children.push(Node::Group(mg));
    }
}

fn effects_in(cx: &mut Ctx, list: &[Json], l: &mut Layer, by_ind: &HashMap<i64, u64>, ids: &mut dyn FnMut() -> u64) {
    let label = l.name.clone();
    let Ok(g) = l.props.group_mut("effects") else { return };
    for e in list {
        let ty = f(e, "ty").unwrap_or(-1.0).max(0.0) as u64;
        let params = e.get("ef").and_then(Json::as_array).map(Vec::as_slice).unwrap_or(&[]);
        let first = params.first();
        let inner = first.and_then(|p| f(p, "ty")).map(|t| t as u64);
        let dims = first.and_then(|p| p.get("v")).and_then(|v| v.get("k")).map_or(0, |k| match k {
            Json::Array(a) if a.first().is_some_and(|x| x.get("s").is_some()) => a[0].get("s").map_or(0, |s| values::numbers(s).len()),
            k => values::numbers(k).len(),
        });
        let nm = s(e, "nm").unwrap_or("Effect");
        let Some(fx) = fx_in(s(e, "mn"), ty, inner, dims) else {
            cx.warn(format!("{label} › {nm}: an effect of Lottie type {ty} is not read"));
            continue;
        };
        let Some(spec) = veymelo_effects::find(fx.id) else { continue };
        let mut fg = Group::instance(spec.def, ids(), nm);
        fg.enabled = f(e, "en") != Some(0.0);
        let elabel = format!("{label} › {nm}");
        for (i, param) in fx.params.iter().enumerate() {
            let Some(v) = params.get(i).and_then(|p| p.get("v")) else { continue };
            let Ok(p) = fg.prop_mut(param.id) else { continue };
            let layer = |j: &Json| values::numbers(j).first().map(|n| Value::Layer(by_ind.get(&(*n as i64)).copied()));
            let scale = over(param.scale);
            let conv: &dyn Fn(&Json) -> Option<Value> = match p.value {
                Value::Choice(_) => &values::choice_of,
                Value::Bool(_) => &flag_of,
                Value::Color(_) => &color_of,
                Value::Vec2(_) => &vec2_of,
                Value::Vec3(_) => &|j: &Json| vec3_or(0.0)(j),
                Value::Layer(_) => &layer,
                _ => &scale,
            };
            prop_from(cx, v, p, conv, &elabel);
        }
        g.children.push(Node::Group(fg));
    }
}

/// The layers of a composition (Lottie's first is the top, as ours).
fn build_layers(im: &mut In, project: &mut Project, comp: &Comp, layers: &[Json]) -> Vec<Layer> {
    // every layer its id first, so parents, mattes and layer controls can find the others
    let mut made: Vec<(&Json, Source, [u32; 2], u64)> = Vec::new();
    let mut by_ind: HashMap<i64, u64> = HashMap::new();
    for j in layers {
        let Some((source, size)) = source_of(im, j, comp) else { continue };
        let id = project.alloc();
        if let Some(ind) = f(j, "ind") {
            by_ind.insert(ind as i64, id);
        }
        made.push((j, source, size, id));
    }
    let mut next = project.next_id;
    let mut ids = || {
        next += 1;
        next - 1
    };
    let mut out = Vec::with_capacity(made.len());
    let mut links: Vec<(u64, Option<u64>, Option<Matte>)> = Vec::new();
    for (n, (j, source, _size, id)) in made.iter().enumerate() {
        let name = s(j, "nm").filter(|n| !n.is_empty()).map_or_else(|| format!("Layer {}", n + 1), str::to_string);
        let mut l = Layer::new(*id, &name, source.clone(), comp.duration);
        let ip = f(j, "ip").unwrap_or(0.0);
        let op = f(j, "op").unwrap_or(im.cx.frame(comp.duration)).max(ip);
        (l.in_point, l.out_point, l.start) = (im.cx.tick(ip), im.cx.tick(op), im.cx.tick(f(j, "st").unwrap_or(0.0)));
        let sr = f(j, "sr").filter(|v| *v != 0.0).unwrap_or(1.0);
        l.stretch = sr * 100.0;
        l.switches.three_d = f(j, "ddd") == Some(1.0) && !matches!(source, Source::Camera);
        if let Some(bm) = f(j, "bm") {
            l.blend = crate::blend_in(bm);
        }
        if f(j, "ao") == Some(1.0) {
            l.auto_orient = AutoOrient::AlongPath;
        }
        if j.get("hd").and_then(Json::as_bool) == Some(true) || f(j, "td") == Some(1.0) {
            l.switches.video = false;
        }
        if let Some(ks) = j.get("ks") {
            transform_in(&mut im.cx, ks, &mut l);
        }
        // a sound layer's level (a %) as its audio levels (dB)
        if let (Some(lv), Ok(p)) = (j.get("au").and_then(|a| a.get("lv")), l.props.prop_mut("audio/levels")) {
            let db = |pct: f64| if pct <= 0.0 { -192.0 } else { (20.0 * (pct / 100.0).log10()).clamp(-192.0, 24.0) };
            let conv = |v: &Json| -> Option<Value> {
                let n: Vec<f64> = match v {
                    Json::Array(a) => a.iter().filter_map(Json::as_f64).collect(),
                    other => other.as_f64().into_iter().collect(),
                };
                let first = *n.first()?;
                Some(Value::Vec2([db(first), db(*n.get(1).unwrap_or(&first))]))
            };
            prop_from(&mut im.cx, lv, p, &conv, &format!("{name} › Audio Levels"));
        }
        let label = name.clone();
        match source {
            Source::Shape => {
                let items = j.get("shapes").and_then(Json::as_array).map(Vec::as_slice).unwrap_or(&[]);
                let nodes = shapes::items_in(&mut im.cx, items, &mut ids, &label);
                if let Ok(c) = l.props.group_mut("contents") {
                    c.children = nodes;
                }
            }
            Source::Text => {
                // a font this computer has not, its glyphs carried: the letters as shapes
                let carried = j.get("t").and_then(|t| text::first_doc(t, &im.fonts)).filter(|d| !im.chars.is_empty() && veymelo_text::lay_out(d).no_font).and_then(|d| Some((text::letters_from_chars(&d, &im.chars)?, d)));
                if let Some((items, d)) = carried {
                    im.cx.warn(format!("{label}: the font \"{}\" is not on this computer, so its letters are drawn from the glyphs the document carries, as shapes (as the words are at its start)", d.font));
                    let mut s = Layer::new(l.id, &l.name, Source::Shape, l.out_point);
                    for g in ["transform", "masks", "effects", veymelo_project::styles::LIST] {
                        if let (Ok(from), Ok(to)) = (l.props.group(g), s.props.group_mut(g)) {
                            *to = from.clone();
                        }
                    }
                    let nodes = shapes::items_in(&mut im.cx, &items, &mut ids, &label);
                    if let Ok(c) = s.props.group_mut("contents") {
                        c.children = nodes;
                    }
                    (l.source, l.props) = (s.source, s.props);
                } else if let (Some(t), Ok(tg)) = (j.get("t"), l.props.group_mut("text")) {
                    text::text_in(&mut im.cx, t, tg, &im.fonts, &mut ids, &label);
                }
            }
            Source::Comp(_) => {
                if let (Some(tm), Ok(p)) = (j.get("tm"), l.props.prop_mut("timeRemap")) {
                    prop_from(&mut im.cx, tm, p, &values::number, &label);
                    l.time_remap = true;
                }
            }
            Source::Camera => {
                if let (Some(pe), Ok(p)) = (j.get("pe"), l.props.prop_mut("cameraOptions/zoom")) {
                    prop_from(&mut im.cx, pe, p, &values::number, &label);
                }
            }
            _ => {}
        }
        if let Some(list) = j.get("masksProperties").and_then(Json::as_array) {
            masks_in(&mut im.cx, list, &mut l, &mut ids);
        }
        if let Some(list) = j.get("ef").and_then(Json::as_array) {
            effects_in(&mut im.cx, list, &mut l, &by_ind, &mut ids);
        }
        if let Some(list) = j.get("sy").and_then(Json::as_array) {
            crate::styles::import(&mut im.cx, list, &mut l, &mut ids);
        }
        let parent = f(j, "parent").and_then(|p| by_ind.get(&(p as i64)).copied()).filter(|p| p != id);
        let kind = match f(j, "tt").unwrap_or(0.0) as i64 {
            1 => Some(MatteKind::Alpha),
            2 => Some(MatteKind::AlphaInverted),
            3 => Some(MatteKind::Luma),
            4 => Some(MatteKind::LumaInverted),
            _ => None,
        };
        let matte = kind.and_then(|kind| {
            // `tp` names the matte; older files use the layer just above
            let layer = match f(j, "tp") {
                Some(tp) => by_ind.get(&(tp as i64)).copied(),
                None => n.checked_sub(1).map(|a| made[a].3),
            };
            layer.filter(|m| m != id).map(|layer| Matte { layer, kind })
        });
        links.push((*id, parent, matte));
        out.push(l);
    }
    project.next_id = next;
    for (id, parent, matte) in links {
        if let Some(l) = out.iter_mut().find(|l| l.id == id) {
            (l.parent, l.matte) = (parent, matte);
        }
    }
    out
}

//! Commands about layers.

use veymelo_color::{BlendMode, Rgba};
use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::Value;
use veymelo_project::{AutoOrient, Label, Layer, Matte, MatteKind, Source, Switches};
use veymelo_time::Tick;

use super::{COMP, Command, LAYER, Run, comp_id, find_comp, find_layer, fresh_name, layer_ids};
use crate::{Args, Ctx, Error, Param, Result, Ty};

/// The layer types that can be added so far.
const ADDABLE: &[&str] = &["solid", "null", "adjustment", "shape", "text", "comp", "footage", "camera", "light", "primitive"];

/// A null's size: it draws nothing, but it has a centre to turn about.
const NULL_SIZE: f64 = 100.0;

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "layer.add",
            label: "New Layer",
            summary: "Add a layer to a composition: a solid colour, a null (a handle to parent other layers to), an adjustment layer (its effects change the layers under it), a shape layer (filled with shape.add), another composition, footage (a picture, video or sound brought in with footage.import), a camera (the 3D layers are seen through the top one), a light (lighting the 3D layers) or a 3D primitive (a cube, sphere, plane, torus, cone or cylinder, sized and coloured by its geometry and material options; see layer.newPrimitive). It is centred in the composition; a camera stands in front of the middle, looking at it, as far as shows the composition at its own size; a light stands up and to the left in front, aimed at the middle.",
            params: const { &[
                COMP,
                Param::need("type", Ty::Choice(ADDABLE), "what it shows"),
                Param::opt("name", Ty::Text, "what to call it (default: Solid 1, Null 1…, or the composition's name)"),
                Param::opt("color", Ty::Color, "a solid's colour (default white)"),
                Param::opt("width", Ty::Whole, "a solid's or adjustment layer's width (default: the composition's)"),
                Param::opt("height", Ty::Whole, "a solid's or adjustment layer's height (default: the composition's)"),
                Param::opt("item", Ty::Comp, "the composition a comp layer shows, or the footage a footage layer shows (an id or a name)"),
                Param::opt("lens", Ty::Number, "a camera's lens length in mm, on 36 mm film (default 50)"),
                Param::opt("twoNode", Ty::Bool, "a camera or light aims at its point of interest (default true)"),
                Param::opt("light", Ty::Choice(veymelo_project::LightKind::IDS), "a light's kind: parallel, spot, point, ambient or environment (a layer's picture all round; see layer.environment) (default spot)"),
                Param::opt("shape", Ty::Choice(veymelo_project::Primitive::IDS), "a primitive's solid: cube, sphere, plane, torus, cone or cylinder (default cube)"),
                Param::opt("text", Ty::Any, "a text layer's words, or its whole document {\"text\", \"font\"?, \"size\"?, \"fill\"?…} (see text.set)"),
                Param::opt("at", Ty::Whole, "its place from the top, 1 being the top (default 1)"),
                Param::opt("in", Ty::Time, "when it is first seen (default 0)"),
                Param::opt("out", Ty::Time, "when it stops being seen (default: the end of the composition, or of what it shows)"),
            ] },
            returns: "{layer, name, index}",
            run: Run::Edit(layer_add),
        },
        Command {
            id: "layer.remove",
            label: "Remove Layer",
            summary: "Take a layer out of its composition (refused while it is another layer's parent; layers using it as a matte lose their matte).",
            params: const { &[COMP, LAYER] },
            returns: "{removed}",
            run: Run::Edit(layer_remove),
        },
        Command {
            id: "layer.set",
            label: "Layer Settings",
            summary: "Change a layer's name, timing, blend mode, parent, matte, switches or label. A parented layer's transform is relative to its parent.",
            params: const { &[
                COMP,
                LAYER,
                Param::opt("name", Ty::Text, "what to call it"),
                Param::opt("in", Ty::Time, "when it is first seen"),
                Param::opt("out", Ty::Time, "when it stops being seen"),
                Param::opt("start", Ty::Time, "the composition time its own time starts from (moving it moves its keys too)"),
                Param::opt("stretch", Ty::Number, "how long its time takes, in % (200 = half speed, below zero = backwards)"),
                Param::opt("blend", Ty::Choice(&BlendMode::IDS), "how its colours mix with the layers under it"),
                Param::opt("parent", Ty::Any, "the layer it moves with, or null for none"),
                Param::opt("keep", Ty::Bool, "with `parent`: stay where it is on screen, its transform changed to fit (default true; false keeps its values and it moves with the parent from where it is)"),
                Param::opt("matte", Ty::Any, "{layer, kind?: alpha|alphaInverted|luma|lumaInverted}: a layer whose alpha or brightness says where this one shows (its video is switched off), or null for none"),
                Param::opt("switches", Ty::Object, "{video?, audio?, solo?, locked?, shy?, collapse?, effects?, motionBlur?, adjustment?, threeD?, guide?: true|false, quality?, sampling?, frameBlend?}"),
                Param::opt("preserveTransparency", Ty::Bool, "draw only where the layers under it are"),
                Param::opt("autoOrient", Ty::Choice(AutoOrient::IDS), "turn by itself: off, alongPath, towardsCamera"),
                Param::opt("separatePosition", Ty::Bool, "its position as three properties, positionX, positionY and positionZ, each animated on its own (keys are carried over)"),
                Param::opt("label", Ty::Choice(Label::IDS), "its colour tag"),
                Param::opt("comment", Ty::Text, "a note"),
            ] },
            returns: "{layer}",
            run: Run::Edit(layer_set),
        },
        Command {
            id: "layer.duplicate",
            label: "Duplicate Layer",
            summary: "Copy a layer, with everything about it, just above it.",
            params: const { &[COMP, LAYER, Param::opt("name", Ty::Text, "what to call the copy (default: the next number of the name)")] },
            returns: "{layer, name, index}",
            run: Run::Edit(layer_duplicate),
        },
        Command {
            id: "layer.split",
            label: "Split Layer",
            summary: "Cut a layer in two at a time: it ends there, and a copy just above it starts there.",
            params: const { &[COMP, LAYER, Param::opt("time", Ty::Time, "where to cut (default: the view's time)")] },
            returns: "{layer, name}",
            run: Run::Edit(layer_split),
        },
        Command {
            id: "layer.precompose",
            label: "Pre-compose",
            summary: "Move layers into a new composition of the same size, frame rate and length, and put a layer showing it where they were. Parents and mattes between moved and staying layers are let go (the notes say which).",
            params: const {
                &[COMP, Param::need("layers", Ty::List, "the layers to move: ids, names or \"#n\""), Param::opt("name", Ty::Text, "what to call the new composition (default: Pre-comp 1…)")]
            },
            returns: "{comp, layer, name, notes?}",
            run: Run::Edit(layer_precompose),
        },
        Command {
            id: "layer.move",
            label: "Move Layer",
            summary: "Put a layer at another place in the stack (1 is the top).",
            params: const { &[COMP, LAYER, Param::need("to", Ty::Whole, "its new place from the top")] },
            returns: "{layer, index}",
            run: Run::Edit(layer_move),
        },
    ]
}

fn layer_add(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let (cw, ch, duration) = (comp.width, comp.height, comp.duration);
    let ty = a.text("type").unwrap_or("");
    if a.has("color") && ty != "solid" {
        return Err(Error::params(format!("`color` is for solid layers, not a {ty} layer")));
    }
    for key in ["width", "height"] {
        if a.has(key) && !matches!(ty, "solid" | "adjustment") {
            return Err(Error::params(format!("`{key}` is for solid and adjustment layers, not a {ty} layer")));
        }
    }
    if a.has("text") && ty != "text" {
        return Err(Error::params(format!("`text` is for text layers, not a {ty} layer")));
    }
    if a.has("item") && !matches!(ty, "comp" | "footage") {
        return Err(Error::params(format!("`item` is for comp and footage layers, not a {ty} layer")));
    }
    if a.has("lens") && ty != "camera" {
        return Err(Error::params(format!("`lens` is for cameras, not a {ty} layer")));
    }
    if a.has("light") && ty != "light" {
        return Err(Error::params(format!("`light` is for lights, not a {ty} layer")));
    }
    if a.has("shape") && ty != "primitive" {
        return Err(Error::params(format!("`shape` is for primitives, not a {ty} layer")));
    }
    if a.has("twoNode") && !matches!(ty, "camera" | "light") {
        return Err(Error::params(format!("`twoNode` is for cameras and lights, not a {ty} layer")));
    }
    let side = |key: &str, default: u32| match a.whole(key) {
        None => Ok(default),
        Some(v) if (1..=veymelo_project::MAX_SIZE as u64).contains(&v) => Ok(v as u32),
        Some(v) => Err(Error::params(format!("`{key}` is from 1 to {} pixels, not {v}", veymelo_project::MAX_SIZE))),
    };
    let taken = comp.layers.iter().map(|l| l.name.as_str());
    let (source, name, size, length) = match ty {
        "solid" => {
            let (w, h) = (side("width", cw)?, side("height", ch)?);
            (Source::Solid { color: a.color("color").unwrap_or(Rgba::WHITE), width: w, height: h }, fresh_name("Solid", taken), (w as f64, h as f64), duration)
        }
        "null" => (Source::Null, fresh_name("Null", taken), (NULL_SIZE, NULL_SIZE), duration),
        "shape" => (Source::Shape, fresh_name("Shape Layer", taken), (0.0, 0.0), duration),
        "text" => {
            // a text layer is called by its words
            let words = a.json("text").and_then(|t| Value::from_json(veymelo_keyframe::Kind::Doc, t).ok()).and_then(|v| v.doc()).map(|d| d.text).unwrap_or_default();
            let short: String = words.lines().next().unwrap_or("").chars().take(24).collect();
            let name = if short.trim().is_empty() { fresh_name("Text", taken) } else { short.trim().to_string() };
            (Source::Text, name, (0.0, 0.0), duration)
        }
        "adjustment" => {
            let (w, h) = (side("width", cw)?, side("height", ch)?);
            (Source::Solid { color: Rgba::WHITE, width: w, height: h }, fresh_name("Adjustment Layer", taken), (w as f64, h as f64), duration)
        }
        "camera" => (Source::Camera, fresh_name("Camera", taken), (0.0, 0.0), duration),
        "light" => {
            let kind = a.text("light").and_then(veymelo_project::LightKind::from_id).unwrap_or(veymelo_project::LightKind::Spot);
            let base = match kind {
                veymelo_project::LightKind::Parallel => "Parallel Light",
                veymelo_project::LightKind::Spot => "Spot Light",
                veymelo_project::LightKind::Point => "Point Light",
                veymelo_project::LightKind::Ambient => "Ambient Light",
                veymelo_project::LightKind::Environment => "Environment Light",
            };
            (Source::Light(kind), fresh_name(base, taken), (0.0, 0.0), duration)
        }
        "primitive" => {
            let kind = a.text("shape").and_then(veymelo_project::Primitive::from_id).unwrap_or_default();
            (Source::Primitive(kind), fresh_name(primitive_name(kind), taken), (0.0, 0.0), duration)
        }
        "footage" => {
            let item = a.json("item").ok_or_else(|| Error::params("a footage layer needs `item`, the footage it shows"))?;
            let id = super::find_footage(cx.project, item)?;
            let it = cx.project.item(id).expect("found");
            let veymelo_project::ItemKind::Footage(f) = &it.kind else { unreachable!("found footage") };
            // a still lasts as long as the composition; the rest as long as they play
            let length = if f.kind == veymelo_project::FootageKind::Image || f.duration <= Tick::ZERO { duration } else { Tick(f.duration.0.saturating_mul(f.loops.max(1) as i64)) };
            if f.kind == veymelo_project::FootageKind::Model {
                // a model stands as long as the composition; its animation loops
                (Source::Model(id), it.name.clone(), (0.0, 0.0), duration)
            } else {
                let size = if f.kind == veymelo_project::FootageKind::Audio { (0.0, 0.0) } else { ((f.width as f64 * f.pixel_aspect).round(), f.height as f64) };
                (Source::Footage(id), it.name.clone(), size, length)
            }
        }
        _ => {
            let item = a.json("item").ok_or_else(|| Error::params("a comp layer needs `item`, the composition it shows"))?;
            let inner = find_comp(cx.project, item)?;
            if inner == cid {
                return Err(Error::refused("a composition cannot show itself"));
            }
            let c = cx.project.comp(inner).expect("found");
            (Source::Comp(inner), cx.project.item(inner).expect("found").name.clone(), (c.width as f64, c.height as f64), c.duration)
        }
    };
    let name = a.text("name").map(str::to_string).unwrap_or(name);
    let id = cx.project.alloc();
    let mut layer = Layer::new(id, &name, source, length);
    layer.switches.adjustment = ty == "adjustment";
    layer.in_point = a.time("in").unwrap_or(Tick::ZERO);
    layer.out_point = a.time("out").unwrap_or(layer.in_point + length);
    if layer.out_point <= layer.in_point {
        return Err(Error::params(format!("a layer ends after it starts: in {} s, out {} s", layer.in_point.seconds(), layer.out_point.seconds())));
    }
    if let Some(text) = a.json("text") {
        let v = Value::from_json(veymelo_keyframe::Kind::Doc, text).map_err(|e| Error::params(format!("`text`: {e}")))?;
        layer.props.prop_mut("text/sourceText").expect("defined").set_value(v).map_err(Error::params)?;
    }
    let (w, h) = (cw as f64, ch as f64);
    let t = &mut layer.props;
    let mut put = |path: &str, v: Value| t.prop_mut(path).expect("defined").set_value(v).map_err(Error::refused);
    match ty {
        "camera" | "light" => {
            let two_node = a.bool("twoNode").unwrap_or(true);
            put("transform/pointOfInterest", Value::Vec3([w / 2.0, h / 2.0, 0.0]))?;
            if ty == "camera" {
                let mm = a.num("lens").unwrap_or(50.0);
                if !(1.0..=10_000.0).contains(&mm) {
                    return Err(Error::params(format!("`lens` is from 1 to 10000 mm, not {mm}")));
                }
                let zoom = veymelo_project::scene::zoom_for_lens(w, mm);
                put("transform/position", Value::Vec3([w / 2.0, h / 2.0, -zoom]))?;
                put("cameraOptions/zoom", Value::Number(zoom))?;
                put("cameraOptions/focusDistance", Value::Number(zoom))?;
                put("cameraOptions/aperture", Value::Number(veymelo_project::scene::aperture_for(w)))?;
            } else {
                put("transform/position", Value::Vec3([w / 2.0 - w / 6.0, h / 2.0 - h / 4.0, -w / 3.0]))?;
            }
            if two_node {
                layer.auto_orient = AutoOrient::TowardsPointOfInterest;
            }
        }
        _ => {
            put("transform/position", Value::Vec3([w / 2.0, h / 2.0, 0.0]))?;
            put("transform/anchor", Value::Vec3([size.0 / 2.0, size.1 / 2.0, 0.0]))?;
        }
    }
    if matches!(layer.source, Source::Primitive(_)) {
        layer.switches.three_d = true;
    }
    if let Source::Model(item) = layer.source {
        // in 3D, as big as fills half the composition's height (its middle at the layer's anchor)
        layer.switches.three_d = true;
        let path = match cx.project.item(item).map(|i| &i.kind) {
            Some(veymelo_project::ItemKind::Footage(f)) => f.path.clone(),
            _ => String::new(),
        };
        if let Ok(m) = veymelo_media::model(&path)
            && let Some((lo, hi)) = m.bounds(&m.pose(None, 0.0))
        {
            let most = (hi.x - lo.x).max(hi.y - lo.y).max(hi.z - lo.z);
            if most > 1e-9 {
                layer.props.prop_mut("model/unitScale").expect("defined").set_value(Value::Number((h * 0.5 / most * 1000.0).round() / 1000.0)).map_err(Error::refused)?;
            }
        }
    }
    let comp = cx.project.comp_mut(cid).expect("found");
    let index = (a.whole("at").unwrap_or(1).max(1) as usize - 1).min(comp.layers.len());
    comp.layers.insert(index, layer);
    cx.label = Some(format!("New Layer {name}"));
    Ok(Json::object([("layer", Json::Number(id as f64)), ("name", Json::from(name)), ("index", Json::Number((index + 1) as f64))]))
}

/// What a new primitive layer is called.
pub(super) fn primitive_name(kind: veymelo_project::Primitive) -> &'static str {
    match kind {
        veymelo_project::Primitive::Cube => "Cube",
        veymelo_project::Primitive::Sphere => "Sphere",
        veymelo_project::Primitive::Plane => "Plane",
        veymelo_project::Primitive::Torus => "Torus",
        veymelo_project::Primitive::Cone => "Cone",
        veymelo_project::Primitive::Cylinder => "Cylinder",
    }
}

/// The size of what a layer shows, in its own pixels (a null's little square; nothing for text,
/// shapes, cameras and lights): its anchor starts in the middle of it.
pub(super) fn natural_size(project: &veymelo_project::Project, layer: &Layer) -> (f64, f64) {
    match &layer.source {
        Source::Solid { width, height, .. } => (*width as f64, *height as f64),
        Source::Null => (NULL_SIZE, NULL_SIZE),
        Source::Comp(id) => project.comp(*id).map_or((0.0, 0.0), |c| (c.width as f64, c.height as f64)),
        Source::Footage(id) => match project.item(*id).map(|i| &i.kind) {
            Some(veymelo_project::ItemKind::Footage(f)) if f.kind != veymelo_project::FootageKind::Audio => ((f.width as f64 * f.pixel_aspect).round(), f.height as f64),
            _ => (0.0, 0.0),
        },
        _ => (0.0, 0.0),
    }
}

fn layer_remove(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, id) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp_mut(cid).expect("found");
    let children: Vec<String> = comp.layers.iter().filter(|l| l.parent == Some(id)).map(|l| format!("\"{}\"", l.name)).collect();
    if !children.is_empty() {
        return Err(Error::refused(format!("it is the parent of {}; give them another parent (layer.set parent) first", children.join(", "))));
    }
    for l in &mut comp.layers {
        if l.matte.is_some_and(|m| m.layer == id) {
            l.matte = None;
        }
    }
    let name = comp.layer(id).expect("found").name.clone();
    comp.layers.retain(|l| l.id != id);
    cx.label = Some(format!("Remove Layer {name}"));
    Ok(Json::object([("removed", id as f64)]))
}

fn layer_set(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, id) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let parent = match a.json("parent") {
        None => None,
        Some(Json::Null) => Some(None),
        Some(p) => {
            let pid = find_layer(comp, p)?;
            if pid == id {
                return Err(Error::refused("a layer cannot be its own parent"));
            }
            Some(Some(pid))
        }
    };
    let matte = match a.json("matte") {
        None => None,
        Some(Json::Null) => Some(None),
        Some(m) => {
            let o = m.as_object().ok_or_else(|| Error::params("`matte` is {\"layer\": …, \"kind\"?: \"alpha\"} or null"))?;
            if let Some(k) = o.keys().find(|k| *k != "layer" && *k != "kind") {
                return Err(Error::params(format!("`matte` has no `{k}` (it has layer, kind)")));
            }
            let layer = find_layer(comp, o.get("layer").ok_or_else(|| Error::params("`matte` needs `layer`"))?)?;
            if layer == id {
                return Err(Error::refused("a layer cannot be its own matte"));
            }
            let kind = match o.get("kind") {
                None => MatteKind::Alpha,
                Some(k) => k.as_str().and_then(MatteKind::from_id).ok_or_else(|| Error::params(format!("a matte's `kind` is one of {}", MatteKind::IDS.join(", "))))?,
            };
            Some(Some(Matte { layer, kind }))
        }
    };
    let mut notes = Vec::new();
    if let Some(p) = parent {
        if a.bool("keep") != Some(false) {
            keep_in_place(cx.project, cid, id, p, cx.view.time, &mut notes);
        }
    } else if a.has("keep") {
        return Err(Error::params("`keep` goes with `parent`"));
    }
    if let Some(Some(m)) = matte {
        let comp = cx.project.comp_mut(cid).expect("found");
        let ml = comp.layer_mut(m.layer).expect("found");
        if ml.switches.video {
            ml.switches.video = false;
            notes.push(format!("\"{}\" is used as a matte now, so its own video was switched off", ml.name));
        }
    }
    let comp = cx.project.comp_mut(cid).expect("found");
    let layer = comp.layer_mut(id).expect("found");
    if let Some(n) = a.text("name") {
        layer.name = n.to_string();
    }
    if let Some(t) = a.time("in") {
        layer.in_point = t;
    }
    if let Some(t) = a.time("out") {
        layer.out_point = t;
    }
    if layer.out_point <= layer.in_point {
        return Err(Error::params(format!("a layer ends after it starts: in {} s, out {} s", layer.in_point.seconds(), layer.out_point.seconds())));
    }
    if let Some(t) = a.time("start") {
        layer.start = t;
    }
    if let Some(s) = a.num("stretch") {
        if s == 0.0 || s.abs() > 1e6 {
            return Err(Error::params("`stretch` is a percentage other than 0"));
        }
        layer.stretch = s;
    }
    if let Some(b) = a.text("blend") {
        layer.blend = BlendMode::from_id(b).expect("checked");
    }
    if let Some(p) = parent {
        layer.parent = p;
    }
    if let Some(m) = matte {
        layer.matte = m;
    }
    if let Some(Json::Object(given)) = a.json("switches") {
        // the switches as they are, with the given ones changed, read as a file reads them
        let mut merged = match layer.switches.to_json() {
            Some(Json::Object(m)) => m,
            _ => Map::new(),
        };
        for (k, v) in given.iter() {
            merged.insert(k, v.clone());
        }
        layer.switches = Switches::from_json(&Json::Object(merged), "switches").map_err(|e| Error::params(e.to_string()))?;
    }
    if let Some(p) = a.bool("preserveTransparency") {
        layer.preserve_transparency = p;
    }
    if let Some(o) = a.text("autoOrient") {
        layer.auto_orient = AutoOrient::from_id(o).expect("checked");
    }
    if let Some(on) = a.bool("separatePosition")
        && on != layer.separate_position
    {
        separate_position(layer, on, &mut notes);
    }
    if let Some(l) = a.text("label") {
        layer.label = Label::from_id(l).expect("checked");
    }
    if let Some(c) = a.text("comment") {
        layer.comment = c.to_string();
    }
    cx.label = Some(format!("Change Layer {}", layer.name));
    let mut out = Map::new();
    out.insert("layer", id as f64);
    if !notes.is_empty() {
        out.insert("notes", Json::array(notes));
    }
    Ok(Json::Object(out))
}

/// Change layer `id`'s transform so that, with `parent` as its parent (none: no parent), it is
/// where it was on screen at `t`. Its values change only where they hold still; animated ones
/// stay, relative to the new parent.
fn keep_in_place(project: &mut veymelo_project::Project, cid: u64, id: u64, parent: Option<u64>, t: Tick, notes: &mut Vec<String>) {
    use veymelo_geom::{Mat3, vec2};
    // where the layer and its new parent are now, expressions and all
    let (world, parent_world) = {
        let e = veymelo_expr::Eval::new(project);
        let comp = project.comp(cid).expect("found");
        let m = |l: &veymelo_project::Layer| e.matrix(e.place(comp, l), t);
        (m(comp.layer(id).expect("found")), parent.and_then(|p| comp.layer(p)).map_or(Mat3::default(), m))
    };
    let comp = project.comp_mut(cid).expect("found");
    let layer = comp.layer(id).expect("found");
    let animated: Vec<&str> =
        ["position", "positionX", "positionY", "scale", "rotation"].into_iter().filter(|p| layer.transform().prop(p).is_ok_and(|p| p.is_animated() || p.expression.is_some())).collect();
    if !animated.is_empty() {
        notes.push(format!("{} of \"{}\" is animated, so its values were kept: it now moves relative to its parent", animated.join(" and "), layer.name));
        return;
    }
    let Some(inv) = parent_world.inverse() else {
        notes.push("the parent is scaled to nothing, so the layer's values were kept".into());
        return;
    };
    let local = inv * world;
    let lt = layer.layer_time(t);
    let get = |p: &str| layer.transform().prop(p).map(|p| p.value_at(lt).as_vec3()).unwrap_or([0.0; 3]);
    let (anchor, pos, scale) = (get("anchor"), layer.position_at(lt), get("scale"));
    let p = local.apply(vec2(anchor[0], anchor[1]));
    let m = local.0;
    let (c0, c1) = (vec2(m[0][0], m[1][0]), vec2(m[0][1], m[1][1]));
    let det = c0.x * c1.y - c0.y * c1.x;
    let sx = c0.length();
    let rotation = c0.y.atan2(c0.x).to_degrees();
    let sy = if sx > 0.0 { det / sx } else { 0.0 };
    if sx > 0.0 && c1.length() > 0.0 && (c0.dot(c1) / (sx * c1.length())).abs() > 1e-6 {
        notes.push(format!("the parent of \"{}\" is slanted (scaled unevenly while turned), which a layer's own transform cannot show exactly; it was kept as near as it can be", layer.name));
    }
    let layer = comp.layer_mut(id).expect("found");
    let tr = |l: &mut veymelo_project::Layer, path: &str, v: Value| {
        let _ = l.props.prop_mut(path).map(|p| p.set_value(v));
    };
    if layer.separate_position {
        tr(layer, "transform/positionX", Value::Number(p.x));
        tr(layer, "transform/positionY", Value::Number(p.y));
    } else {
        tr(layer, "transform/position", Value::Vec3([p.x, p.y, pos[2]]));
    }
    tr(layer, "transform/scale", Value::Vec3([sx * 100.0, sy * 100.0, scale[2]]));
    tr(layer, "transform/rotation", Value::Number(rotation));
}

fn layer_move(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, id) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp_mut(cid).expect("found");
    let from = comp.index_of(id).expect("found");
    let to = (a.whole("to").unwrap_or(1).max(1) as usize - 1).min(comp.layers.len() - 1);
    let layer = comp.layers.remove(from);
    cx.label = Some(format!("Move Layer {}", layer.name));
    comp.layers.insert(to, layer);
    Ok(Json::object([("layer", Json::Number(id as f64)), ("index", Json::Number((to + 1) as f64))]))
}

/// A copy of `layer` with new ids for it and everything in it.
pub(super) fn copy_of(project: &mut veymelo_project::Project, layer: &Layer, name: String) -> Layer {
    let mut copy = layer.clone();
    copy.id = project.alloc();
    copy.name = name;
    copy.props.instances_mut(&mut |g| g.id = project.alloc());
    copy
}

/// The name for a copy of a layer called `name`: its next number.
pub(super) fn copy_name(comp: &veymelo_project::Comp, name: &str) -> String {
    let base = match name.rsplit_once(' ') {
        Some((b, n)) if n.parse::<u32>().is_ok() => b,
        _ => name,
    };
    fresh_name(base, comp.layers.iter().map(|l| l.name.as_str()))
}

fn layer_duplicate(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, id) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let (original, index) = (comp.layer(id).expect("found").clone(), comp.index_of(id).expect("found"));
    let name = a.text("name").map(str::to_string).unwrap_or_else(|| copy_name(comp, &original.name));
    let copy = copy_of(cx.project, &original, name.clone());
    let new_id = copy.id;
    cx.project.comp_mut(cid).expect("found").layers.insert(index, copy);
    cx.label = Some(format!("Duplicate Layer {}", original.name));
    Ok(Json::object([("layer", Json::Number(new_id as f64)), ("name", Json::from(name)), ("index", Json::Number((index + 1) as f64))]))
}

fn layer_split(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, id) = layer_ids(cx.project, cx.view, a)?;
    let t = a.time("time").unwrap_or(cx.view.time);
    let comp = cx.project.comp(cid).expect("found");
    let (original, index) = (comp.layer(id).expect("found").clone(), comp.index_of(id).expect("found"));
    if t <= original.in_point || t >= original.out_point {
        return Err(Error::refused(format!("\"{}\" is seen from {} s to {} s; cut it in between", original.name, original.in_point.seconds(), original.out_point.seconds())));
    }
    let name = copy_name(comp, &original.name);
    let mut copy = copy_of(cx.project, &original, name.clone());
    copy.in_point = t;
    let new_id = copy.id;
    let comp = cx.project.comp_mut(cid).expect("found");
    comp.layer_mut(id).expect("found").out_point = t;
    comp.layers.insert(index, copy);
    cx.label = Some(format!("Split Layer {}", original.name));
    Ok(Json::object([("layer", Json::Number(new_id as f64)), ("name", Json::from(name))]))
}

fn layer_precompose(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let refs = a.need("layers").as_array().cloned().unwrap_or_default();
    if refs.is_empty() {
        return Err(Error::params("`layers` needs at least one layer"));
    }
    let mut ids = Vec::new();
    for r in &refs {
        let id = find_layer(comp, r)?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let name = match a.text("name") {
        Some(n) => n.to_string(),
        None => fresh_name("Pre-comp", cx.project.items.iter().map(|i| i.name.as_str())),
    };
    // the moved layers keep their order; links across the cut are let go
    let mut notes = Vec::new();
    let mut moved: Vec<Layer> = comp.layers.iter().filter(|l| ids.contains(&l.id)).cloned().collect();
    let mut staying: Vec<Layer> = comp.layers.iter().filter(|l| !ids.contains(&l.id)).cloned().collect();
    let top = comp.layers.iter().position(|l| ids.contains(&l.id)).expect("found");
    let cut = |layers: &mut Vec<Layer>, inside: bool, notes: &mut Vec<String>, names: &dyn Fn(u64) -> String| {
        for l in layers.iter_mut() {
            if let Some(p) = l.parent.filter(|p| ids.contains(p) != inside) {
                notes.push(format!("\"{}\" lost its parent \"{}\"", l.name, names(p)));
                l.parent = None;
            }
            if let Some(m) = l.matte.filter(|m| ids.contains(&m.layer) != inside) {
                notes.push(format!("\"{}\" lost its matte \"{}\"", l.name, names(m.layer)));
                l.matte = None;
            }
        }
    };
    let names = |id: u64| comp.layer(id).map_or(String::new(), |l| l.name.clone());
    cut(&mut moved, true, &mut notes, &names);
    cut(&mut staying, false, &mut notes, &names);
    let mut inner = veymelo_project::Comp::new(comp.width, comp.height, comp.frame_rate, comp.duration);
    inner.pixel_aspect = comp.pixel_aspect;
    inner.background = comp.background;
    inner.layers = moved;
    let (w, h, duration) = (comp.width as f64, comp.height as f64, comp.duration);
    let new_comp = cx.project.add_comp(&name, inner);
    let mut nest = Layer::new(cx.project.alloc(), &name, Source::Comp(new_comp), duration);
    nest.props.prop_mut("transform/position").expect("defined").set_value(Value::Vec3([w / 2.0, h / 2.0, 0.0])).map_err(Error::refused)?;
    nest.props.prop_mut("transform/anchor").expect("defined").set_value(Value::Vec3([w / 2.0, h / 2.0, 0.0])).map_err(Error::refused)?;
    let nest_id = nest.id;
    staying.insert(top.min(staying.len()), nest);
    cx.project.comp_mut(cid).expect("found").layers = staying;
    cx.label = Some(format!("Pre-compose {name}"));
    let mut out = Map::new();
    out.insert("comp", new_comp as f64);
    out.insert("layer", nest_id as f64);
    out.insert("name", name.as_str());
    if !notes.is_empty() {
        out.insert("notes", Json::array(notes));
    }
    Ok(Json::Object(out))
}

/// Split a layer's position into x, y and z (or join them again), carrying the keys over.
fn separate_position(layer: &mut Layer, on: bool, notes: &mut Vec<String>) {
    use veymelo_keyframe::{Interp, Keyframe};
    let tr = layer.props.group_mut("transform").expect("every layer has one");
    if on {
        let pos = tr.prop("position").expect("defined").clone();
        for (d, path) in ["positionX", "positionY", "positionZ"].into_iter().enumerate() {
            let p = tr.prop_mut(path).expect("defined");
            if pos.keys.is_empty() {
                let _ = p.set_value(Value::Number(pos.value.as_vec3()[d]));
                continue;
            }
            // each number keeps the keys' times and the way they move; an ease along the path
            // becomes this number's share of its speed
            let keys: Vec<Keyframe> = pos
                .keys
                .iter()
                .enumerate()
                .map(|(i, k)| {
                    let mut n = Keyframe::new(k.time, Value::Number(k.value.as_vec3()[d]));
                    n.interp_in = k.interp_in;
                    n.interp_out = k.interp_out;
                    n.auto_bezier = k.auto_bezier;
                    n.continuous = k.continuous;
                    n.label = k.label;
                    let share = |j: usize| {
                        let (a, b) = (pos.keys[i.min(j)].value.as_vec3(), pos.keys[i.max(j)].value.as_vec3());
                        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
                        if len > 0.0 { (b[d] - a[d]) / len } else { 0.0 }
                    };
                    if k.interp_out == Interp::Bezier && i + 1 < pos.keys.len() && !k.ease_out.is_empty() {
                        let e = k.ease_out_of(0);
                        n.ease_out = vec![veymelo_keyframe::Ease { speed: e.speed * share(i + 1), influence: e.influence }];
                    }
                    if k.interp_in == Interp::Bezier && i > 0 && !k.ease_in.is_empty() {
                        let e = k.ease_in_of(0);
                        n.ease_in = vec![veymelo_keyframe::Ease { speed: e.speed * share(i - 1), influence: e.influence }];
                    }
                    n
                })
                .collect();
            let _ = p.set_keys(keys);
        }
        if pos.keys.iter().any(|k| k.roving) {
            notes.push("roving keys do not carry over to separate dimensions; they keep their times".into());
        }
        let _ = tr.prop_mut("position").expect("defined").set_value(Value::Vec3([0.0; 3]));
    } else {
        let parts: Vec<_> = ["positionX", "positionY", "positionZ"].iter().map(|p| tr.prop(p).expect("defined").clone()).collect();
        let mut times: Vec<Tick> = parts.iter().flat_map(|p| p.keys.iter().map(|k| k.time)).collect();
        times.sort();
        times.dedup();
        let p = tr.prop_mut("position").expect("defined");
        if times.is_empty() {
            let _ = p.set_value(Value::Vec3([0, 1, 2].map(|d| parts[d].value.as_f64())));
        } else {
            // a key wherever any number had one, its value where the three were then
            let keys = times
                .iter()
                .map(|&t| {
                    let mut k = Keyframe::new(t, Value::Vec3([0, 1, 2].map(|d| parts[d].value_at(t).as_f64())));
                    let eased = parts.iter().any(|q| q.keys.iter().any(|k| k.time == t && k.interp_out == Interp::Bezier));
                    if eased {
                        k = k.eased();
                    }
                    k
                })
                .collect();
            let _ = p.set_keys(keys);
            notes.push("the three positions were joined with a key wherever one of them had one; eases came back as easy eases".into());
        }
        for path in ["positionX", "positionY", "positionZ"] {
            let _ = tr.prop_mut(path).expect("defined").set_value(Value::Number(0.0));
        }
    }
    layer.separate_position = on;
}

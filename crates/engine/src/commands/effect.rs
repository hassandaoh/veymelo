//! Commands about effects: adding, taking away and ordering a layer's effects, the effects
//! there are, and the puppet's pins.

use veymelo_effects::liquid;
use veymelo_effects::puppet::{self, MeshAt, PinKind};
use veymelo_json::{Map, Value as Json};
use veymelo_project::{Group, Layer, Node};

use super::shape::{insert, remove_at, start_with};
use super::{COMP, Command, LAYER, Run, fresh_name, layer_ids};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

const PIN_KINDS: &[&str] = &["position", "advanced", "bend", "starch", "overlap"];

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "effect.add",
            label: "Add Effect",
            summary: "Add an effect to a layer (effect.types lists them). Effects change the layer's pixels top to bottom, after its masks; their settings are properties under effects/<name>/.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::need("type", Ty::Text, "the effect, like \"puppet\" (effect.types lists them)"),
                    Param::opt("name", Ty::Text, "what to call it (default: its name, numbered when there are more)"),
                    Param::opt("at", Ty::Whole, "its place in the list, 1 being the top (default: the bottom)"),
                    Param::opt("props", Ty::Object, "its settings to start with, by path"),
                ]
            },
            returns: "{path, id, name}",
            run: Run::Edit(effect_add),
        },
        Command {
            id: "effect.remove",
            label: "Remove Effect",
            summary: "Take an effect off a layer.",
            params: const { &[COMP, LAYER, Param::need("effect", Ty::Text, "the effect, like \"Puppet\" or \"#1\"")] },
            returns: "{removed}",
            run: Run::Edit(effect_remove),
        },
        Command {
            id: "effect.move",
            label: "Move Effect",
            summary: "Put a layer's effect at another place in its list (they apply top to bottom).",
            params: const { &[COMP, LAYER, Param::need("effect", Ty::Text, "the effect"), Param::need("to", Ty::Whole, "its new place, 1 being the top")] },
            returns: "{effects}",
            run: Run::Edit(effect_move),
        },
        Command {
            id: "effect.types",
            label: "Effect Types",
            summary: "The effects there are: each one's type, name, family, what it does and its settings.",
            params: const { &[Param::opt("category", Ty::Text, "only those of this family, like distort")] },
            returns: "[{type, name, category, about, settings}]",
            run: Run::Look(effect_types),
        },
        Command {
            id: "puppet.pin",
            label: "Add Puppet Pin",
            summary: "Put a puppet pin on a layer at a point of its own pixels. The layer gets a Puppet effect if it has none, and a mesh over the part of it under the point if no mesh covers it. Move the pin by giving its position keys (prop.keys on <path>/position); advanced and bend pins also turn (rotation) and grow (scale) the mesh about themselves; starch stiffens round itself; overlap says what is in front where parts cross.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::need("at", Ty::List, "[x, y] in the layer's own pixels"),
                    Param::opt("kind", Ty::Choice(PIN_KINDS), "what the pin does (default position)"),
                    Param::opt("mesh", Ty::Text, "the mesh to put it on (default: the one covering the point, or a new one)"),
                    Param::opt("effect", Ty::Text, "the Puppet effect (default: the layer's first)"),
                    Param::opt("name", Ty::Text, "what to call it (default: Puppet Pin 1, Starch 1, Overlap 1…)"),
                    Param::opt("props", Ty::Object, "its settings to start with: {\"rotation\": 30}, {\"amount\": 80, \"extent\": 25}…"),
                    Param::opt("time", Ty::Time, "when the layer's pixels are looked at for a new mesh (default: the view's time)"),
                ]
            },
            returns: "{path, pin, mesh, effect, note?}",
            run: Run::Edit(puppet_pin),
        },
        Command {
            id: "liquify.stroke",
            label: "Liquify Stroke",
            summary: "Brush a stroke over a layer with Liquify: the picture under the brush is pushed along the stroke (warp), shaken (turbulence), twirled, puckered, bloated, shifted to the side, reflected, cloned or put back (reconstruction); freeze and thaw mark where later strokes cannot reach. The layer gets a Liquify effect if it has none. Strokes are kept one a line in the effect's Distortion Mesh (prop.set it to \"\" to start again); when that has keys, the stroke goes into the key at `time`. A brush held still (the same point given again and again) twirls, puckers or bloats more each time.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::need("points", Ty::List, "[[x, y], …] where the brush goes, in the layer's own pixels (one point: a dab)"),
                    Param::opt("tool", Ty::Choice(liquid::TOOLS), "the brush (default: the effect's Tool)"),
                    Param::opt("size", Ty::Number, "the brush across, in the layer's pixels (default: the effect's Brush Size)"),
                    Param::opt("pressure", Ty::Number, "how hard, 1 to 100 (default: the effect's Brush Pressure)"),
                    Param::opt("jitter", Ty::Number, "for turbulence: how much it shakes, 1 to 100 (default: the effect's Turbulent Jitter)"),
                    Param::opt("cloneOffset", Ty::List, "for clone: [dx, dy] from where it paints to where it copies from (default: the effect's Clone Offset)"),
                    Param::opt("mode", Ty::Choice(&["revert", "displace", "amplitwist", "affine"]), "for reconstruction: back to nothing moved, or to how the stroke's start moved (default: the effect's Reconstruction Mode)"),
                    Param::opt("effect", Ty::Text, "the Liquify effect (default: the layer's first)"),
                    Param::opt("time", Ty::Time, "when the strokes have keys, the key that gets it (default: the view's time)"),
                ]
            },
            returns: "{effect, stroke, strokes}",
            run: Run::Edit(liquify_stroke),
        },
        Command {
            id: "puppet.remesh",
            label: "Remake Puppet Mesh",
            summary: "A puppet mesh keeps the outline of the part of the layer it was made over, so it stays the same while the layer's picture changes under it. This takes that outline again from the layer's pixels at a time (the part under the mesh's seed); with follow, the mesh keeps none and is made each frame from the pixels as they are then.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::need("mesh", Ty::Text, "the mesh (\"Mesh 1\"…)"),
                    Param::opt("effect", Ty::Text, "the Puppet effect (default: the layer's first)"),
                    Param::opt("time", Ty::Time, "when the layer's pixels are looked at (default: the view's time)"),
                    Param::opt("follow", Ty::Bool, "keep no outline: the mesh follows the pixels every frame"),
                ]
            },
            returns: "{mesh, points}",
            run: Run::Edit(puppet_remesh),
        },
        Command {
            id: "puppet.info",
            label: "Puppet Meshes",
            summary: "A layer's puppet meshes at a time: how many triangles, what part of the layer each covers, and its pins, where they were put and where they are.",
            params: const { &[COMP, LAYER, Param::opt("effect", Ty::Text, "the Puppet effect (default: the layer's first)"), Param::opt("time", Ty::Time, "when (default: the view's time)")] },
            returns: "{effect, meshes: [{name, path, triangles, vertices, bounds, pins: [{name, path, kind, rest, position}]}]}",
            run: Run::Look(puppet_info),
        },
    ]
}

/// The layer's effects list.
fn effects_of(layer: &mut Layer) -> Result<&mut Group> {
    let name = layer.name.clone();
    layer.props.group_mut("effects").map_err(|_| Error::refused(format!("\"{name}\" has no pixels of its own, so it takes no effects")))
}

fn effect_add(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let ty = a.text("type").unwrap_or("");
    let def = cx.catalog.find("effects", ty).ok_or_else(|| {
        let ids: Vec<&str> = veymelo_effects::all().map(|s| s.def.id).collect();
        Error::params(match crate::params::nearest(ty, ids.iter().copied()) {
            Some(n) => format!("there is no effect \"{ty}\"; did you mean \"{n}\"? (effect.types lists them)"),
            None => format!("there is no effect \"{ty}\" (there are: {})", ids.join(", ")),
        })
    })?;
    let id = cx.project.alloc();
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let rect = bounds(cx.project, cid, cx.project.comp(cid).expect("found").layer(lid).expect("found"));
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let list = effects_of(layer)?;
    let name = a.text("name").map_or_else(|| effect_name(list, def.name), str::to_string);
    // points start where they belong on this layer (a centre in its middle), then what was given
    let mut g = Group::instance(def, id, &name);
    if let Some(spec) = veymelo_effects::find(def.id) {
        spec.start(&mut g, rect);
    }
    start_with(&mut g, a.json("props"))?;
    let index = a.whole("at").map_or(list.children.len(), |k| (k.max(1) as usize - 1).min(list.children.len()));
    list.children.insert(index, Node::Group(g));
    cx.label = Some(format!("Add {name}"));
    Ok(Json::object([("path", Json::from(format!("effects/{name}"))), ("id", Json::Number(id as f64)), ("name", Json::from(name))]))
}

fn effect_remove(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    effects_of(layer)?;
    let which = a.text("effect").unwrap_or("").trim_matches('/').trim_start_matches("effects/");
    let id = remove_at(layer, &format!("effects/{which}"))?;
    cx.label = Some(format!("Remove {which}"));
    Ok(Json::object([("removed", id as f64)]))
}

fn effect_move(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let list = effects_of(layer)?;
    let which = a.text("effect").unwrap_or("").trim_matches('/').trim_start_matches("effects/");
    let id = list.group(which).map_err(Error::not_found)?.id;
    let from = list.children.iter().position(|n| matches!(n, Node::Group(g) if g.id == id)).expect("found");
    let node = list.children.remove(from);
    let to = (a.whole("to").unwrap_or(1).max(1) as usize - 1).min(list.children.len());
    list.children.insert(to, node);
    let names: Vec<Json> = list.children.iter().map(|n| Json::from(n.name())).collect();
    cx.label = Some(format!("Move {which}"));
    Ok(Json::object([("effects", Json::Array(names))]))
}

fn effect_types(_: &Look, a: &Args) -> Result<Json> {
    let only = a.text("category");
    let mut out = Vec::new();
    for s in veymelo_effects::all().filter(|s| only.is_none_or(|c| c.eq_ignore_ascii_case(s.category))) {
        let mut settings = Vec::new();
        Group::new(s.def).walk(&mut |path, p| settings.push(Json::from(format!("{path}: {}", p.def.summary()))));
        let mut o = Map::new();
        o.insert("type", s.def.id);
        o.insert("name", s.def.name);
        o.insert("category", s.category);
        o.insert("about", s.def.doc);
        o.insert("settings", Json::Array(settings));
        if !s.lists.is_empty() {
            let mut lists = Map::new();
            for (list, def) in s.lists {
                let mut keys = Vec::new();
                Group::new(def).walk(&mut |path, p| keys.push(Json::from(format!("{path}: {}", p.def.summary()))));
                lists.insert(format!("{list}/{}", def.id), Json::Array(keys));
            }
            o.insert("inside", Json::Object(lists));
        }
        out.push(Json::Object(o));
    }
    Ok(Json::Array(out))
}

/// The puppet effect the parameters name (or the layer's first), by name.
fn puppet_name(layer: &Layer, a: &Args) -> Option<String> {
    let list = layer.props.group("effects").ok()?;
    let found = match a.text("effect") {
        Some(n) => list.group(n.trim_start_matches("effects/")).ok().filter(|g| g.def.id == "puppet"),
        None => list.children.iter().find_map(|c| if let Node::Group(g) = c { (g.def.id == "puppet").then_some(g) } else { None }),
    };
    found.map(|g| g.name().to_string())
}

/// A layer's rectangle for its effects.
pub(super) fn bounds(cx_project: &veymelo_project::Project, cid: u64, layer: &Layer) -> veymelo_geom::Rect {
    let comp = cx_project.comp(cid).expect("found");
    let size = match &layer.source {
        veymelo_project::Source::Solid { width, height, .. } => Some((*width as f64, *height as f64)),
        veymelo_project::Source::Comp(c) => cx_project.comp(*c).map(|c| (c.width as f64, c.height as f64)),
        _ => None,
    };
    match size {
        Some((w, h)) => veymelo_geom::Rect::from_size(w, h),
        None => veymelo_geom::Rect::new(-(comp.width as f64) / 2.0, -(comp.height as f64) / 2.0, comp.width as f64 / 2.0, comp.height as f64 / 2.0),
    }
}

fn point(j: Option<&Json>, what: &str) -> Result<[f64; 2]> {
    let v: Option<Vec<f64>> = j.and_then(Json::as_array).map(|a| a.iter().filter_map(Json::as_f64).collect());
    match v {
        Some(v) if v.len() == 2 && v.iter().all(|x| x.is_finite()) => Ok([v[0], v[1]]),
        _ => Err(Error::params(format!("`{what}` is [x, y] in the layer's own pixels"))),
    }
}

fn puppet_pin(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let at = point(a.json("at"), "at")?;
    let kind = a.text("kind").unwrap_or("position");
    let def = puppet::PIN_TYPES[PIN_KINDS.iter().position(|k| *k == kind).unwrap_or(0)];
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let t = a.time("time").unwrap_or(cx.view.time);
    // which puppet and which mesh, from the layer's pixels as they are
    let (effect, mesh, note, outline) = {
        let project: &veymelo_project::Project = cx.project;
        let layer = project.comp(cid).expect("found").layer(lid).expect("found");
        if layer.props.group("effects").is_err() {
            return Err(Error::refused(format!("\"{}\" has no pixels of its own, so it cannot be a puppet", layer.name)));
        }
        let buf = veymelo_render::layer_pixels(project, cid, lid, t).ok_or_else(|| Error::refused(format!("\"{}\" shows nothing at {}s to put a pin on", layer.name, t.seconds())))?;
        let on = {
            let p = buf.pixel(veymelo_geom::vec2(at[0], at[1]));
            buf.img.get(p.x.floor() as i64, p.y.floor() as i64)[3] >= 0.5
        };
        let effect = puppet_name(layer, a);
        if a.has("effect") && effect.is_none() {
            return Err(Error::not_found(format!("\"{}\" has no Puppet effect \"{}\"", layer.name, a.text("effect").unwrap_or(""))));
        }
        let e = veymelo_expr::Eval::new(project);
        let place = e.place(project.comp(cid).expect("found"), layer);
        let read = |p: &veymelo_project::Property| e.value(p, place, t);
        let mesh = match (&effect, a.text("mesh")) {
            (None, Some(m)) => return Err(Error::not_found(format!("\"{}\" has no puppet mesh \"{m}\" yet", layer.name))),
            (None, None) => None,
            (Some(fx), wanted) => {
                let g = layer.props.group(&format!("effects/{fx}")).expect("found");
                let ctx = veymelo_effects::Ctx::new(g, &read, layer.layer_time(t), bounds(project, cid, layer), layer.id);
                let meshes = g.group("meshes").map(|l| puppet::groups(l)).unwrap_or_default();
                match wanted {
                    Some(m) => Some(g.group(&format!("meshes/{m}")).map_err(Error::not_found)?.name().to_string()),
                    None => meshes
                        .into_iter()
                        .find(|m| {
                            let made = puppet::mesh_for(&buf, &MeshAt { pins: Vec::new(), ..puppet::mesh_at(&ctx, m) });
                            puppet::mesh::inside(&made.outline, at)
                        })
                        .map(|m| m.name().to_string()),
                }
            }
        };
        let note = (!on && mesh.is_none()).then(|| format!("[{}, {}] is not on the layer's pixels; the new mesh covers the part nearest to it", at[0], at[1]));
        // a new mesh keeps the outline of the part it covers now
        let outline = mesh.is_none().then(|| puppet::outline_of(&buf, at));
        (effect, mesh, note, outline)
    };
    // new ids for what is made, then the change
    let (effect_id, mesh_id, pin_id) = (effect.is_none().then(|| cx.project.alloc()), mesh.is_none().then(|| cx.project.alloc()), cx.project.alloc());
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let layer_name = layer.name.clone();
    let list = effects_of(layer)?;
    let effect = match (effect, effect_id) {
        (Some(e), _) => e,
        (None, id) => {
            let name = effect_name(list, puppet::PUPPET.name);
            insert(list, &puppet::PUPPET, id.unwrap_or(0), Some(&name), None, None)?.0
        }
    };
    let fx = list.group_mut(&effect).expect("found");
    let pin = match a.text("name") {
        Some(n) => n.to_string(),
        None => pin_name(fx, def.name),
    };
    let meshes = fx.group_mut("meshes").expect("a puppet has meshes");
    let mesh = match (mesh, mesh_id) {
        (Some(m), _) => m,
        (None, id) => insert(meshes, &puppet::MESH, id.unwrap_or(0), None, None, Some(&Json::object([("seed", Json::from(at)), ("outline", outline_json(&outline.unwrap_or_default()))])))?.0,
    };
    let pins = meshes.group_mut(&format!("{mesh}/pins")).expect("a mesh has pins");
    // where it was put, then what was given
    let mut start = Map::new();
    start.insert("rest", Json::from(at));
    if def.id != "bendPin" {
        start.insert("position", Json::from(at));
    }
    if let Some(Json::Object(m)) = a.json("props") {
        for (k, v) in m.iter() {
            start.insert(k, v.clone());
        }
    }
    insert(pins, def, pin_id, Some(&pin), None, Some(&Json::Object(start)))?;
    let path = format!("effects/{effect}/meshes/{mesh}/pins/{pin}");
    cx.label = Some(format!("Add {pin} to {layer_name}"));
    let mut o = Map::new();
    o.insert("path", path.as_str());
    o.insert("pin", pin.as_str());
    o.insert("mesh", format!("effects/{effect}/meshes/{mesh}"));
    o.insert("effect", format!("effects/{effect}"));
    if let Some(n) = note {
        o.insert("note", n);
    }
    Ok(Json::Object(o))
}

/// An outline as a path property takes it.
fn outline_json(points: &[[f64; 2]]) -> Json {
    Json::object([("points", Json::array(points.iter().map(|p| Json::from(*p)))), ("closed", Json::from(true))])
}

fn puppet_remesh(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let t = a.time("time").unwrap_or(cx.view.time);
    let wanted = a.text("mesh").unwrap_or("").trim_start_matches("meshes/").to_string();
    let follow = a.bool("follow").unwrap_or(false);
    let (fx, outline) = {
        let project: &veymelo_project::Project = cx.project;
        let layer = project.comp(cid).expect("found").layer(lid).expect("found");
        let fx = puppet_name(layer, a).ok_or_else(|| Error::not_found(format!("\"{}\" has no Puppet effect (puppet.pin adds one)", layer.name)))?;
        let m = layer.props.group(&format!("effects/{fx}/meshes/{wanted}")).map_err(Error::not_found)?;
        let seed = m.prop("seed").map(|p| p.value.as_vec2()).unwrap_or([0.0; 2]);
        let outline = if follow {
            Vec::new()
        } else {
            let buf = veymelo_render::layer_pixels(project, cid, lid, t).ok_or_else(|| Error::refused(format!("\"{}\" shows nothing at {}s to make a mesh over", layer.name, t.seconds())))?;
            let o = puppet::outline_of(&buf, seed);
            if o.is_empty() {
                return Err(Error::refused(format!("\"{}\" has nothing of its own under the mesh's seed [{}, {}] at {}s", layer.name, seed[0], seed[1], t.seconds())));
            }
            o
        };
        (fx, outline)
    };
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let name = layer.name.clone();
    let p = layer.props.prop_mut(&format!("effects/{fx}/meshes/{wanted}/outline")).map_err(Error::not_found)?;
    let value = veymelo_keyframe::Value::from_json(veymelo_keyframe::Kind::Path, &outline_json(&outline)).map_err(Error::params)?;
    p.set_value(value).map_err(Error::params)?;
    cx.label = Some(format!("Remake {wanted} of {name}"));
    Ok(Json::object([("mesh", Json::from(format!("effects/{fx}/meshes/{wanted}"))), ("points", Json::Number(outline.len() as f64))]))
}

fn liquify_stroke(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let points: Vec<[f64; 2]> = match a.json("points").and_then(Json::as_array) {
        Some(list) if !list.is_empty() => list.iter().map(|p| point(Some(p), "points")).collect::<Result<_>>()?,
        _ => return Err(Error::params("`points` is [[x, y], …] with at least one point")),
    };
    let view_time = cx.view.time;
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let rect = bounds(cx.project, cid, cx.project.comp(cid).expect("found").layer(lid).expect("found"));
    let new_id = cx.project.alloc();
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let layer_name = layer.name.clone();
    let t = layer.layer_time(a.time("time").unwrap_or(view_time));
    let list = effects_of(layer)?;
    let found = match a.text("effect") {
        Some(n) => {
            let n = n.trim_start_matches("effects/");
            match list.group(n) {
                Ok(g) if g.def.id == "liquify" => Some(n.to_string()),
                _ => return Err(Error::not_found(format!("\"{layer_name}\" has no Liquify effect \"{n}\""))),
            }
        }
        None => list.children.iter().find_map(|c| if let Node::Group(g) = c { (g.def.id == "liquify").then(|| g.name().to_string()) } else { None }),
    };
    let effect = match found {
        Some(e) => e,
        None => {
            let name = effect_name(list, liquid::LIQUIFY_DEF.name);
            let mut g = Group::instance(&liquid::LIQUIFY_DEF, new_id, &name);
            liquid::LIQUIFY.start(&mut g, rect);
            list.children.push(Node::Group(g));
            name
        }
    };
    let fx = list.group_mut(&effect).expect("found");
    // what was not given comes from the effect's brush settings now
    let setting = |fx: &Group, id: &str| fx.prop(id).map(|p| p.value_at(t)).ok();
    let num = |id: &str, given: Option<f64>| given.or_else(|| setting(fx, id).map(|v| v.as_f64())).unwrap_or(0.0);
    let tool = match a.text("tool") {
        Some(name) => liquid::TOOLS.iter().position(|x| *x == name).unwrap_or(0),
        None => setting(fx, "tool").map_or(0, |v| v.as_choice() as usize),
    };
    let mode = match a.text("mode") {
        Some(name) => ["revert", "displace", "amplitwist", "affine"].iter().position(|x| *x == name).unwrap_or(0),
        None => setting(fx, "reconstructionMode").map_or(0, |v| v.as_choice() as usize),
    };
    let clone_offset = match a.json("cloneOffset") {
        Some(j) => point(Some(j), "cloneOffset")?,
        None => setting(fx, "cloneOffset").map_or([0.0; 2], |v| v.as_vec2()),
    };
    let stroke = liquid::Stroke {
        tool,
        size: num("brushSize", a.num("size")).clamp(1.0, 600.0),
        pressure: num("brushPressure", a.num("pressure")).clamp(1.0, 100.0),
        jitter: num("turbulentJitter", a.num("jitter")).clamp(1.0, 100.0),
        clone_offset,
        mode,
        points,
    };
    let line = stroke.line();
    let p = fx.prop_mut("distortionMesh").expect("liquify keeps its strokes");
    let add = |text: &str| if text.trim().is_empty() { line.clone() } else { format!("{}\n{line}", text.trim_end()) };
    let text = if p.is_animated() {
        let now = add(p.value_at(t).as_text());
        let mut keys = p.keys.clone();
        veymelo_keyframe::set_key(&mut keys, veymelo_keyframe::Keyframe::new(t, veymelo_keyframe::Value::Text(now.clone())));
        p.set_keys(keys).map_err(Error::params)?;
        now
    } else {
        let now = add(p.value_at(t).as_text());
        p.set_value(veymelo_keyframe::Value::Text(now.clone())).map_err(Error::params)?;
        now
    };
    cx.label = Some(format!("Liquify {layer_name}"));
    Ok(Json::object([("effect", Json::from(format!("effects/{effect}"))), ("stroke", Json::from(line)), ("strokes", Json::Number(liquid::strokes(&text).len() as f64))]))
}

/// An effect's name: its own, numbered from 2 when that is taken (Puppet, Puppet 2…).
fn effect_name(list: &Group, base: &str) -> String {
    let taken = |n: &str| list.children.iter().any(|c| c.name() == n);
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base} {n}")).find(|n| !taken(n)).expect("some number is free")
}

fn puppet_info(l: &Look, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(l.project, l.view, a)?;
    let comp = l.project.comp(cid).expect("found");
    let layer = comp.layer(lid).expect("found");
    let t = a.time("time").unwrap_or(l.view.time);
    let fx = puppet_name(layer, a).ok_or_else(|| Error::not_found(format!("\"{}\" has no Puppet effect (puppet.pin adds one)", layer.name)))?;
    let g = layer.props.group(&format!("effects/{fx}")).expect("found");
    let e = veymelo_expr::Eval::new(l.project);
    let place = e.place(comp, layer);
    let read = |p: &veymelo_project::Property| e.value(p, place, t);
    let ctx = veymelo_effects::Ctx::new(g, &read, layer.layer_time(t), bounds(l.project, cid, layer), layer.id);
    let buf = veymelo_render::layer_pixels(l.project, cid, lid, t);
    let mut meshes = Vec::new();
    if let Ok(list) = g.group("meshes") {
        for n in &list.children {
            let Node::Group(m) = n else { continue };
            let base = format!("effects/{fx}/meshes/{}", m.name());
            let at = puppet::mesh_at(&ctx, m);
            let mut o = Map::new();
            o.insert("name", m.name());
            o.insert("path", base.as_str());
            if !m.enabled {
                o.insert("enabled", false);
            }
            match &buf {
                Some(b) => {
                    let made = puppet::mesh_for(b, &at);
                    o.insert("triangles", made.tris.len() as f64);
                    o.insert("vertices", made.verts.len() as f64);
                    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
                    for p in &made.outline {
                        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
                    }
                    if !made.outline.is_empty() {
                        let r = |v: f64| (v * 10.0).round() / 10.0;
                        o.insert("bounds", Json::from([r(lo[0]), r(lo[1]), r(hi[0]), r(hi[1])]));
                    }
                }
                None => {
                    o.insert("note", "the layer shows nothing at this time");
                }
            }
            let mut pins = Vec::new();
            if let Ok(pl) = m.group("pins") {
                for pn in &pl.children {
                    let Node::Group(p) = pn else { continue };
                    let pin = puppet::pin_at(&ctx, p);
                    let mut q = Map::new();
                    q.insert("name", p.name());
                    q.insert("path", format!("{base}/pins/{}", p.name()));
                    q.insert("kind", PIN_KINDS[[PinKind::Position, PinKind::Advanced, PinKind::Bend, PinKind::Starch, PinKind::Overlap].iter().position(|k| *k == pin.kind).unwrap_or(0)]);
                    q.insert("rest", Json::from(pin.rest));
                    if pin.kind != PinKind::Bend {
                        q.insert("position", Json::from(pin.position));
                    }
                    pins.push(Json::Object(q));
                }
            }
            o.insert("pins", Json::Array(pins));
            meshes.push(Json::Object(o));
        }
    }
    Ok(Json::object([("effect", Json::from(format!("effects/{fx}"))), ("meshes", Json::Array(meshes))]))
}

/// A pin's next name, numbered across all the effect's meshes.
fn pin_name(effect: &Group, base: &str) -> String {
    let mut taken = Vec::new();
    if let Ok(list) = effect.group("meshes") {
        for m in puppet::groups(list) {
            if let Ok(pl) = m.group("pins") {
                taken.extend(pl.children.iter().map(|n| n.name().to_string()));
            }
        }
    }
    fresh_name(base, taken.iter().map(String::as_str))
}

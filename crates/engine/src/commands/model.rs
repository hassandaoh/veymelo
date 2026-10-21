//! 3D models: what a model file holds, and its cameras and lights made camera and light layers
//! standing where they are in the model as its layer places it; 3D primitive layers (solids made
//! from a few numbers); and the material options of 3D layers set, put back, or copied from one
//! layer to others.

use veymelo_color::Rgba;
use veymelo_geom::{Vec3, vec3};
use veymelo_json::Value as Json;
use veymelo_keyframe::Value;
use veymelo_model::{CameraKind, LightKind as ModelLight};
use veymelo_project::{AutoOrient, ItemKind, Source};
use veymelo_time::Tick;

use super::{COMP, Command, LAYER, Run, comp_id, find_footage, find_layer, layer_ids};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "model.info",
            label: "3D Model Info",
            summary: "What a 3D model holds (an imported .gltf, .glb, .obj, .stl or .fbx, or a 3D model layer's): its meshes and triangles, materials, pictures, animations and how long each is, cameras, lights, and the box round it in its own units.",
            params: const { &[Param::opt("item", Ty::Any, "the model's footage item (an id or a name)"), COMP, Param::opt("layer", Ty::Layer, "or a 3D model layer showing it")] },
            returns: "{item, meshes, triangles, materials, textures, animations: [{index, name, length}], cameras, lights, size, notes}",
            run: Run::Look(info),
        },
        Command {
            id: "camera.fromModel",
            label: "Create Cameras from 3D Model",
            summary: "Make a camera layer for each camera in a 3D model layer's model (or the one asked for), standing and looking as it does in the model where the layer places it, seeing as wide.",
            params: const { &[COMP, LAYER, Param::opt("camera", Ty::Whole, "only this camera of the model, from 1"), Param::opt("time", Ty::Time, "the moment of the model's animation (default: the view's time)")] },
            returns: "{layers, notes?}",
            run: Run::Edit(cameras),
        },
        Command {
            id: "light.fromModel",
            label: "Create Lights from 3D Model",
            summary: "Make a light layer for each light in a 3D model layer's model (glTF's lights: directional as parallel, point, spot with its cone), standing where it is in the model where the layer places it, its colour and brightness.",
            params: const { &[COMP, LAYER, Param::opt("time", Ty::Time, "the moment of the model's animation (default: the view's time)")] },
            returns: "{layers}",
            run: Run::Edit(lights),
        },
        Command {
            id: "layer.newPrimitive",
            label: "New 3D Primitive",
            summary: "Add a 3D primitive layer: a cube, sphere, plane, torus, cone or cylinder, sized in pixels, its middle at the composition's middle (or `position`), drawn in the 3D scene as models are (lit, shadowed, in focus or not) with a surface of its own (base colour, metallic, roughness, light given off).",
            params: const { &[
                COMP,
                Param::need("shape", Ty::Choice(veymelo_project::Primitive::IDS), "the solid"),
                Param::opt("name", Ty::Text, "what to call it (default: Cube, Sphere…)"),
                Param::opt("width", Ty::Number, "a cube's or plane's width (default 300; a plane's 400)"),
                Param::opt("height", Ty::Number, "a cube's, plane's, cone's or cylinder's height"),
                Param::opt("depth", Ty::Number, "a cube's depth"),
                Param::opt("radius", Ty::Number, "a sphere's, cone's or cylinder's radius; a torus's from its middle to its tube's (default 150)"),
                Param::opt("tubeRadius", Ty::Number, "a torus's tube's radius (default 50)"),
                Param::opt("segments", Ty::Number, "pieces round it (default 48)"),
                Param::opt("rings", Ty::Number, "a sphere's pieces from pole to pole, a torus's round its tube (default 24)"),
                Param::opt("position", Ty::List, "where its middle is, [x, y, z] in composition pixels"),
                Param::opt("baseColor", Ty::Color, "its colour (default light grey)"),
                Param::opt("metallic", Ty::Number, "how much it is metal, 0 to 100 (default 0)"),
                Param::opt("roughness", Ty::Number, "how rough its surface is, 0 (a mirror) to 100 (default 50)"),
                Param::opt("emissive", Ty::Color, "the light it gives off (default none)"),
                Param::opt("castsShadows", Ty::Choice(veymelo_project::scene::SHADOW_CHOICES), "it throws shadows: off, on or only (default off)"),
                Param::opt("acceptsShadows", Ty::Choice(veymelo_project::scene::SHADOW_CHOICES), "shadows fall on it: off, on or only (default on)"),
                Param::opt("acceptsLights", Ty::Bool, "it is lit by the lights (default true)"),
                Param::opt("at", Ty::Whole, "its place from the top, 1 being the top (default 1)"),
                Param::opt("in", Ty::Time, "when it is first seen (default 0)"),
                Param::opt("out", Ty::Time, "when it stops being seen (default: the end of the composition)"),
            ] },
            returns: "{layer, name, index}",
            run: Run::Edit(new_primitive),
        },
        Command {
            id: "material.set",
            label: "Material Options",
            summary: "Set how 3D layers take light and shadow (each layer takes what it has of these: a primitive also its base colour, metallic, roughness and the light it gives off). With `time`, keys there.",
            params: const { &[
                COMP,
                Param::opt("layers", Ty::List, "the layers (ids, names or \"#n\")"),
                Param::opt("layer", Ty::Layer, "or one layer"),
                Param::opt("castsShadows", Ty::Choice(veymelo_project::scene::SHADOW_CHOICES), "it throws shadows: off, on or only (only: its shadow, not it)"),
                Param::opt("acceptsShadows", Ty::Choice(veymelo_project::scene::SHADOW_CHOICES), "shadows fall on it: off, on or only (only: just the shadows)"),
                Param::opt("acceptsLights", Ty::Bool, "it is lit by the lights"),
                Param::opt("lightTransmission", Ty::Number, "light through it from behind, 0 to 100"),
                Param::opt("ambient", Ty::Number, "how much it takes of the ambient lights, 0 to 100"),
                Param::opt("diffuse", Ty::Number, "how much it takes of the light falling on it, 0 to 100"),
                Param::opt("specularIntensity", Ty::Number, "how bright its shine is, 0 to 100"),
                Param::opt("specularShininess", Ty::Number, "how small and sharp its shine is, 0 to 100"),
                Param::opt("metal", Ty::Number, "its shine coloured by it (100) or by the light (0)"),
                Param::opt("baseColor", Ty::Color, "a primitive's colour"),
                Param::opt("metallic", Ty::Number, "how much a primitive is metal, 0 to 100"),
                Param::opt("roughness", Ty::Number, "how rough a primitive's surface is, 0 to 100"),
                Param::opt("emissive", Ty::Color, "the light a primitive gives off"),
                Param::opt("time", Ty::Time, "set keys at this time instead"),
            ] },
            returns: "{set: [{layer, paths}]}",
            run: Run::Edit(material_set),
        },
        Command {
            id: "material.reset",
            label: "Reset Material",
            summary: "Put 3D layers' material options back as they start (keyed ones get a key at the view's time).",
            params: const { &[COMP, Param::opt("layers", Ty::List, "the layers (ids, names or \"#n\")"), Param::opt("layer", Ty::Layer, "or one layer")] },
            returns: "{layers}",
            run: Run::Edit(material_reset),
        },
        Command {
            id: "material.duplicateAssign",
            label: "Duplicate and Assign Material",
            summary: "Give layers the first one's material options: each takes the values, keys and expressions of those it has too.",
            params: const { &[COMP, Param::need("layers", Ty::List, "the layer to copy from, then those to give it to")] },
            returns: "{assigned}",
            run: Run::Edit(material_duplicate),
        },
        Command {
            id: "layer.environment",
            label: "Environment Layer",
            summary: "Make footage, composition or solid layers the environment: the picture (a panorama all round, as a 360° picture is laid out) an environment light with no source of its own lights the 3D scene by, layers and models taking its light and shining with it. An environment layer is not drawn itself.",
            params: const { &[COMP, Param::opt("layers", Ty::List, "the layers (ids, names or \"#n\")"), Param::opt("layer", Ty::Layer, "or one layer"), Param::opt("on", Ty::Bool, "the environment (default true) or not")] },
            returns: "{layers, environment}",
            run: Run::Edit(environment_layer),
        },
        Command {
            id: "light.environmentBackground",
            label: "Create Environment Light Background Layer",
            summary: "Add a layer showing an environment light's picture as the sky all round the 3D scene (seen through the camera as if endlessly far, turned as the light is), at the bottom of the composition.",
            params: const { &[COMP, Param::opt("light", Ty::Layer, "the environment light (default: the composition's first)")] },
            returns: "{layer, light, source}",
            run: Run::Edit(environment_background),
        },
    ]
}

/// Whether a layer can be an environment's picture: footage, a composition or a solid.
fn pictures(source: &Source) -> bool {
    matches!(source, Source::Footage(_) | Source::Comp(_) | Source::Solid { .. })
}

fn environment_layer(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, ids) = layers_given(cx, a)?;
    let on = a.bool("on").unwrap_or(true);
    let comp = cx.project.comp_mut(cid).expect("found");
    for &lid in &ids {
        let l = comp.layer(lid).expect("found");
        if on && !pictures(&l.source) {
            return Err(Error::params(format!("\"{}\" cannot be the environment (a footage, composition or solid layer can)", l.name)));
        }
    }
    for &lid in &ids {
        let l = comp.layer_mut(lid).expect("found");
        l.environment = on;
        if on {
            l.switches.three_d = true;
        }
    }
    cx.label = Some("Environment Layer".into());
    Ok(Json::object([("layers", Json::array(ids.iter().map(|&l| l as f64))), ("environment", Json::Bool(on))]))
}

fn environment_background(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let is_env = |l: &&veymelo_project::Layer| l.source == Source::Light(veymelo_project::LightKind::Environment);
    let light = match a.json("light") {
        Some(j) => comp.layer(find_layer(comp, j)?).filter(is_env).ok_or_else(|| Error::params("that layer is not an environment light"))?,
        None => comp.layers.iter().find(is_env).ok_or_else(|| Error::refused("the composition has no environment light (layer.add type light, light environment)"))?,
    };
    let source = light.props.prop("lightOptions/source").ok().and_then(|p| p.value.as_layer()).and_then(|id| comp.layer(id)).or_else(|| comp.layers.iter().find(|l| l.environment));
    let source = source.ok_or_else(|| Error::refused("the environment light has no source, and the composition no environment layer (layer.environment)"))?;
    if !pictures(&source.source) {
        return Err(Error::refused(format!("the environment's picture, \"{}\", is not a footage, composition or solid layer", source.name)));
    }
    let (light_id, source_id) = (light.id, source.id);
    let name = super::fresh_name(&format!("{} Background", source.name), comp.layers.iter().map(|l| l.name.as_str()));
    let (show, length) = (source.source.clone(), comp.duration);
    let size = super::layer::natural_size(cx.project, source);
    let (w, h) = (comp.width as f64, comp.height as f64);
    let id = cx.project.alloc();
    let mut layer = veymelo_project::Layer::new(id, &name, show, length);
    layer.switches.three_d = true;
    layer.environment_background = true;
    let t = &mut layer.props;
    t.prop_mut("transform/position").expect("defined").set_value(Value::Vec3([w / 2.0, h / 2.0, 0.0])).map_err(Error::refused)?;
    t.prop_mut("transform/anchor").expect("defined").set_value(Value::Vec3([size.0 / 2.0, size.1 / 2.0, 0.0])).map_err(Error::refused)?;
    cx.project.comp_mut(cid).expect("found").layers.push(layer);
    cx.label = Some("Create Environment Light Background Layer".into());
    Ok(Json::object([("layer", id as f64), ("light", light_id as f64), ("source", source_id as f64)]))
}

/// The material options `material.set` and `layer.newPrimitive` take.
const MATERIAL: &[&str] = &["castsShadows", "acceptsShadows", "acceptsLights", "lightTransmission", "ambient", "diffuse", "specularIntensity", "specularShininess", "metal", "baseColor", "metallic", "roughness", "emissive"];

/// The material options given in `a` set on a layer (those it has), keyed at `time` if given:
/// the paths set.
fn put_material(cx: &mut Ctx, a: &Args, cid: u64, lid: u64) -> Result<Vec<String>> {
    let mut set = Vec::new();
    for key in MATERIAL {
        let Some(v) = a.json(key) else { continue };
        let path = format!("materialOptions/{key}");
        if !cx.project.comp(cid).and_then(|c| c.layer(lid)).is_some_and(|l| l.props.prop(&path).is_ok()) {
            continue;
        }
        let mut params = vec![("comp", Json::Number(cid as f64)), ("layer", Json::Number(lid as f64)), ("path", Json::from(path.as_str())), ("value", v.clone())];
        if let Some(t) = a.json("time") {
            params.push(("time", t.clone()));
        }
        super::call(cx, "prop.set", Json::object(params))?;
        set.push(path);
    }
    Ok(set)
}

/// The layers `layers` (or `layer`) names.
fn layers_given(cx: &Ctx, a: &Args) -> Result<(u64, Vec<u64>)> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let mut ids = Vec::new();
    let list = a.json("layers").and_then(Json::as_array).cloned().unwrap_or_default();
    for j in list.iter().chain(a.json("layer")) {
        let id = find_layer(comp, j)?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return Err(Error::params("give `layers` (or `layer`)"));
    }
    Ok((cid, ids))
}

fn new_primitive(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let shape = a.text("shape").unwrap_or("cube");
    let kind = veymelo_project::Primitive::from_id(shape).unwrap_or_default();
    let cid = comp_id(cx.project, cx.view, a)?;
    let mut add = vec![("comp", Json::Number(cid as f64)), ("type", Json::from("primitive")), ("shape", Json::from(shape))];
    for key in ["name", "at", "in", "out"] {
        if let Some(v) = a.json(key) {
            add.push((key, v.clone()));
        }
    }
    let made = super::call(cx, "layer.add", Json::object(add))?;
    let lid = made.get("layer").and_then(Json::as_u64).expect("a new layer");
    for key in ["width", "height", "depth", "radius", "tubeRadius", "segments", "rings"] {
        let Some(v) = a.num(key) else { continue };
        let layer = cx.project.comp_mut(cid).and_then(|c| c.layer_mut(lid)).expect("added");
        let prop = layer.props.prop_mut(&format!("geometryOptions/{key}")).map_err(|_| Error::params(format!("a {shape} has no {key}")))?;
        prop.set_value(Value::Number(v)).map_err(|e| Error::params(format!("`{key}`: {e}")))?;
    }
    if let Some(p) = a.json("position") {
        super::call(cx, "prop.set", Json::object([("comp", Json::Number(cid as f64)), ("layer", Json::Number(lid as f64)), ("path", Json::from("transform/position")), ("value", p.clone())]))?;
    }
    put_material(cx, a, cid, lid)?;
    cx.label = Some(format!("New {}", super::layer::primitive_name(kind)));
    Ok(made)
}

fn material_set(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, ids) = layers_given(cx, a)?;
    let mut out = Vec::new();
    for lid in ids {
        let set = put_material(cx, a, cid, lid)?;
        if !set.is_empty() {
            out.push(Json::object([("layer", Json::Number(lid as f64)), ("paths", Json::array(set.iter().map(|p| Json::from(p.as_str()))))]));
        }
    }
    if out.is_empty() {
        return Err(Error::params(format!("none of the layers has any of the material options given ({})", MATERIAL.join(", "))));
    }
    cx.label = Some("Material Options".into());
    Ok(Json::object([("set", Json::Array(out))]))
}

fn material_reset(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, ids) = layers_given(cx, a)?;
    for &lid in &ids {
        super::call(cx, "prop.reset", Json::object([("comp", Json::Number(cid as f64)), ("layer", Json::Number(lid as f64)), ("path", Json::from("materialOptions"))]))?;
    }
    cx.label = Some("Reset Material".into());
    Ok(Json::object([("layers", Json::array(ids.iter().map(|&l| l as f64)))]))
}

fn material_duplicate(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, ids) = layers_given(cx, a)?;
    if ids.len() < 2 {
        return Err(Error::params("give the layer to copy from, then one or more to give its material to"));
    }
    let comp = cx.project.comp_mut(cid).expect("found");
    let from = comp.layer(ids[0]).and_then(|l| l.props.group("materialOptions").ok()).cloned().ok_or_else(|| Error::params("the first layer has no material options"))?;
    let mut assigned = 0;
    for &lid in &ids[1..] {
        let Some(to) = comp.layer_mut(lid).and_then(|l| l.props.group_mut("materialOptions").ok()) else { continue };
        // the options they both have (a primitive has more than a picture)
        for node in &from.children {
            if let veymelo_project::Node::Prop(src) = node
                && let Ok(dst) = to.prop_mut(src.def.id)
            {
                dst.value = src.value.clone();
                dst.keys = src.keys.clone();
                dst.expression = src.expression.clone();
            }
        }
        assigned += 1;
    }
    cx.label = Some("Duplicate and Assign Material".into());
    Ok(Json::object([("assigned", assigned as f64)]))
}

fn info(l: &Look, a: &Args) -> Result<Json> {
    let item = match (a.json("item"), a.json("layer")) {
        (Some(j), None) => find_footage(l.project, j)?,
        (None, Some(_)) => {
            let (cid, lid) = layer_ids(l.project, l.view, a)?;
            match l.project.comp(cid).expect("found").layer(lid).expect("found").source {
                Source::Model(id) => id,
                _ => return Err(Error::params("that layer is not a 3D model layer")),
            }
        }
        _ => return Err(Error::params("give `item` (the model's footage) or `layer` (a 3D model layer)")),
    };
    let path = match l.project.item(item).map(|i| &i.kind) {
        Some(ItemKind::Footage(f)) if f.kind == veymelo_project::FootageKind::Model => f.path.clone(),
        _ => return Err(Error::params("that item is not a 3D model")),
    };
    let m = veymelo_media::model(&path).map_err(Error::not_found)?;
    let size = m.bounds(&m.pose(None, 0.0)).map(|(lo, hi)| Json::array([hi.x - lo.x, hi.y - lo.y, hi.z - lo.z].map(|v| Json::Number((v * 1e4).round() / 1e4))));
    Ok(Json::object([
        ("item", Json::Number(item as f64)),
        ("meshes", Json::Number(m.meshes.len() as f64)),
        ("triangles", Json::Number(m.triangles() as f64)),
        ("materials", Json::array(m.materials.iter().map(|x| Json::from(x.name.as_str())))),
        ("textures", Json::Number(m.textures.len() as f64)),
        (
            "animations",
            Json::array(m.animations.iter().enumerate().map(|(i, an)| Json::object([("index", Json::Number((i + 1) as f64)), ("name", Json::from(an.name.as_str())), ("length", Json::Number((an.length() * 1000.0).round() / 1000.0))]))),
        ),
        ("cameras", Json::Number(m.cameras.len() as f64)),
        ("lights", Json::Number(m.lights.len() as f64)),
        ("size", size.unwrap_or(Json::Null)),
        ("notes", Json::array(m.notes.iter().map(|n| Json::from(n.as_str())))),
    ]))
}

/// The model of a 3D model layer posed at `t`, the map from its units to the world, and each
/// of its nodes' places in the world.
fn placed(cx: &Ctx, cid: u64, lid: u64, t: Tick) -> Result<(std::sync::Arc<veymelo_model::Model>, Vec<veymelo_geom::Mat4>)> {
    let layer = cx.project.comp(cid).expect("found").layer(lid).expect("found");
    if !matches!(layer.source, Source::Model(_)) {
        return Err(Error::params(format!("\"{}\" is not a 3D model layer", layer.name)));
    }
    let (model, pose, world) = veymelo_render::model_in_world(cx.project, cid, lid, t).ok_or_else(|| Error::not_found(format!("the model of \"{}\" cannot be read", layer.name)))?;
    let nodes = pose.world.iter().map(|n| world * *n).collect();
    Ok((model, nodes))
}

/// A new layer of `kind` added at the top, named `name`.
fn add(cx: &mut Ctx, cid: u64, kind: &str, name: &str, extra: &[(&str, Json)]) -> Result<u64> {
    let mut p = veymelo_json::Map::new();
    p.insert("comp", cid as f64);
    p.insert("type", kind);
    p.insert("name", name);
    for (k, v) in extra {
        p.insert(*k, v.clone());
    }
    let made = super::call(cx, "layer.add", Json::Object(p))?;
    Ok(made.get("layer").and_then(Json::as_u64).expect("made"))
}

fn put(cx: &mut Ctx, cid: u64, lid: u64, path: &str, v: Value) -> Result<()> {
    let l = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    l.props.prop_mut(path).map_err(Error::refused)?.set_value(v).map_err(|e| Error::params(format!("{path}: {e}")))
}

fn point(v: Vec3) -> Value {
    Value::Vec3([v.x, v.y, v.z])
}

/// A name not taken in the composition.
fn named(cx: &Ctx, cid: u64, base: &str) -> String {
    let comp = cx.project.comp(cid).expect("found");
    if !comp.layers.iter().any(|l| l.name == base) {
        return base.to_string();
    }
    (2..).map(|k| format!("{base} {k}")).find(|n| !comp.layers.iter().any(|l| l.name == *n)).expect("some number is free")
}

fn cameras(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let t = a.time("time").unwrap_or(cx.view.time);
    let (model, nodes) = placed(cx, cid, lid, t)?;
    let comp_h = cx.project.comp(cid).expect("found").height as f64;
    let only = a.whole("camera").map(|c| c as usize);
    let found: Vec<(usize, usize)> = model.nodes.iter().enumerate().filter_map(|(n, node)| node.camera.map(|c| (n, c))).filter(|(_, c)| only.is_none_or(|o| o == c + 1)).collect();
    if found.is_empty() {
        return Err(Error::not_found(if only.is_some() { "the model has no such camera".to_string() } else { "the model has no cameras".to_string() }));
    }
    let (mut made, mut notes) = (Vec::new(), Vec::new());
    for (n, c) in found {
        let cam = &model.cameras[c];
        let m = nodes[n];
        let eye = m.apply(vec3(0.0, 0.0, 0.0));
        // it looks down its −Z, its +Y up
        let forward = m.apply_vec(vec3(0.0, 0.0, -1.0)).normalize();
        let up = m.apply_vec(vec3(0.0, 1.0, 0.0)).normalize();
        let zoom = match cam.kind {
            CameraKind::Perspective { yfov } => comp_h / 2.0 / (yfov / 2.0).tan().max(1e-6),
            CameraKind::Orthographic { .. } => veymelo_project::scene::default_zoom(cx.project.comp(cid).expect("found").width as f64),
        };
        let name = named(cx, cid, if cam.name.is_empty() { "Model Camera" } else { &cam.name });
        let id = add(cx, cid, "camera", &name, &[])?;
        put(cx, cid, id, "transform/position", point(eye))?;
        put(cx, cid, id, "transform/pointOfInterest", point(eye + forward * zoom))?;
        put(cx, cid, id, "cameraOptions/zoom", Value::Number(zoom))?;
        put(cx, cid, id, "cameraOptions/focusDistance", Value::Number(zoom))?;
        // turned about its view as the model's camera is: its down against the level one's
        let level = veymelo_expr::facing(forward);
        let (right, down) = (level.apply_vec(vec3(1.0, 0.0, 0.0)), level.apply_vec(vec3(0.0, 1.0, 0.0)));
        let want = -up;
        let roll = want.dot(right).atan2(want.dot(down)).to_degrees();
        put(cx, cid, id, "transform/rotation", Value::Number(-roll))?;
        let l = cx.project.comp_mut(cid).expect("found").layer_mut(id).expect("found");
        l.auto_orient = AutoOrient::TowardsPointOfInterest;
        if matches!(cam.kind, CameraKind::Orthographic { .. }) {
            notes.push(format!("\"{name}\" was an orthographic camera in the model: it is a perspective one here"));
        }
        made.push(id);
    }
    cx.label = Some("Create Cameras from 3D Model".into());
    let mut o = veymelo_json::Map::new();
    o.insert("layers", Json::array(made.into_iter().map(|i| Json::Number(i as f64))));
    if !notes.is_empty() {
        o.insert("notes", Json::array(notes.into_iter().map(Json::from)));
    }
    Ok(Json::Object(o))
}

fn lights(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let t = a.time("time").unwrap_or(cx.view.time);
    let (model, nodes) = placed(cx, cid, lid, t)?;
    let comp = cx.project.comp(cid).expect("found");
    let reach = comp.width.max(comp.height) as f64;
    let found: Vec<(usize, usize)> = model.nodes.iter().enumerate().filter_map(|(n, node)| node.light.map(|l| (n, l))).filter(|(_, l)| *l < model.lights.len()).collect();
    if found.is_empty() {
        return Err(Error::not_found("the model has no lights"));
    }
    let k = {
        let l = comp.layer(lid).expect("found");
        l.props.prop("model/unitScale").map_or(100.0, |p| p.value.as_f64())
    };
    let mut made = Vec::new();
    for (n, i) in found {
        let light = &model.lights[i];
        let m = nodes[n];
        let at = m.apply(vec3(0.0, 0.0, 0.0));
        let toward = m.apply_vec(vec3(0.0, 0.0, -1.0)).normalize();
        let kind = match light.kind {
            ModelLight::Directional => "parallel",
            ModelLight::Point => "point",
            ModelLight::Spot { .. } => "spot",
        };
        let name = named(cx, cid, if light.name.is_empty() { "Model Light" } else { &light.name });
        let id = add(cx, cid, "light", &name, &[("light", Json::from(kind))])?;
        put(cx, cid, id, "transform/position", point(at))?;
        put(cx, cid, id, "transform/pointOfInterest", point(at + toward * reach))?;
        let c = light.color.map(|v| (veymelo_color::linear_to_srgb(v.max(0.0) as f64) * 1e6).round() / 1e6);
        put(cx, cid, id, "lightOptions/color", Value::Color(Rgba { r: c[0].min(1.0), g: c[1].min(1.0), b: c[2].min(1.0), a: 1.0 }))?;
        // a unit of the file's (lux or candela) as 100 %
        put(cx, cid, id, "lightOptions/intensity", Value::Number((light.intensity as f64 * 100.0).clamp(0.0, 1000.0)))?;
        if let ModelLight::Spot { inner, outer } = light.kind {
            put(cx, cid, id, "lightOptions/coneAngle", Value::Number((outer * 2.0).to_degrees().clamp(0.0, 180.0)))?;
            put(cx, cid, id, "lightOptions/coneFeather", Value::Number(((1.0 - inner / outer.max(1e-9)) * 100.0).clamp(0.0, 100.0)))?;
        }
        if let Some(range) = light.range {
            put(cx, cid, id, "lightOptions/falloff", Value::Choice(1))?;
            put(cx, cid, id, "lightOptions/radius", Value::Number(0.0))?;
            put(cx, cid, id, "lightOptions/falloffDistance", Value::Number(range as f64 * k))?;
        }
        made.push(id);
    }
    cx.label = Some("Create Lights from 3D Model".into());
    Ok(Json::object([("layers", Json::array(made.into_iter().map(|i| Json::Number(i as f64))))]))
}

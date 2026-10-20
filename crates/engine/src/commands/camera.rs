//! Cameras moved the way a camera operator moves one — orbited about the point it looks at,
//! panned sideways, dollied forward — turned to frame layers, focused on a layer (or kept in
//! focus on it by an expression), hung on a null that orbits it; a stereo rig of two eye cameras
//! seen through 3D glasses; lights carried by a camera, shining where it looks.
//!
//! A camera is moved in the composition's world (an orbit turns about the world's up, whatever
//! its parent does) and written back in its parent's space: an animated property gets a key at
//! the time, a still one a new value.

use veymelo_effects::controls::{CONVERGE_TO, STEREO_CONFIGURATIONS};
use veymelo_effects::perspective::VIEWS_3D;
use veymelo_expr::Eval;
use veymelo_geom::{Mat4, Quat, Vec3, vec3};
use veymelo_json::Value as Json;
use veymelo_keyframe::Value;
use veymelo_project::{AutoOrient, Layer, Project, Source};
use veymelo_time::Tick;

use super::arrange::{content_box, put, put_position};
use super::{COMP, Command, Run, comp_id, find_layer};
use crate::{Args, Ctx, Error, Param, Result, Ty};

const CAMERA: Param = Param::opt("camera", Ty::Layer, "the camera (default: the one the composition is seen through at the time)");
const TIME: Param = Param::opt("time", Ty::Time, "when (default: the view's time); what is animated gets keys there");

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "camera.orbit",
            label: "Orbit Camera",
            summary: "Swing a camera around the point it looks at (a one-node camera's: as far ahead as its zoom), still looking at it: `yaw` about the world's up, `pitch` up or down (kept short of straight above or below).",
            params: const {
                &[
                    COMP,
                    CAMERA,
                    Param::opt("yaw", Ty::Number, "degrees around, positive taking the camera to its right"),
                    Param::opt("pitch", Ty::Number, "degrees up, positive raising the camera to look down on it"),
                    TIME,
                ]
            },
            returns: "{camera, position, pointOfInterest}",
            run: Run::Edit(orbit),
        },
        Command {
            id: "camera.pan",
            label: "Track Camera",
            summary: "Move a camera sideways and up or down in its own picture's plane, still facing the same way (the point it looks at moves with it).",
            params: const {
                &[
                    COMP,
                    CAMERA,
                    Param::opt("x", Ty::Number, "how far right (below 0: left), in pixels of its picture where it looks"),
                    Param::opt("y", Ty::Number, "how far down (below 0: up), in pixels of its picture where it looks"),
                    TIME,
                ]
            },
            returns: "{camera, position, pointOfInterest}",
            run: Run::Edit(pan),
        },
        Command {
            id: "camera.dolly",
            label: "Dolly Camera",
            summary: "Move a camera forward along the way it looks (or back): the point it looks at moves with it, or with `still` stays where it is and the camera nears it (stopping a pixel short).",
            params: const {
                &[
                    COMP,
                    CAMERA,
                    Param::need("by", Ty::Number, "how far forward in pixels (below 0: back)"),
                    Param::opt("still", Ty::Bool, "the point it looks at stays (default false)"),
                    TIME,
                ]
            },
            returns: "{camera, position, pointOfInterest}",
            run: Run::Edit(dolly),
        },
        Command {
            id: "camera.lookAt",
            label: "Look at Layers",
            summary: "Turn a camera toward layers (the middle of what they show) from where it stands, then move it along its new view until they fill its picture. With `keepDirection` it keeps looking the way it does and moves sideways to them instead of turning; with `fit` false it does not move nearer or farther.",
            params: const {
                &[
                    COMP,
                    CAMERA,
                    Param::opt("layers", Ty::List, "the 3D layers (default: every 3D layer seen at the time)"),
                    Param::opt("keepDirection", Ty::Bool, "keep facing the same way (default false)"),
                    Param::opt("fit", Ty::Bool, "move to frame them (default true)"),
                    Param::opt("margin", Ty::Number, "the room left at each edge, in % of the picture (default 5)"),
                    TIME,
                ]
            },
            returns: "{camera, position, pointOfInterest, distance}",
            run: Run::Edit(look_at),
        },
        Command {
            id: "camera.focus",
            label: "Focus Camera",
            summary: "Set how far away a camera is sharp: at a layer (its anchor point, along the camera's view) or at its point of interest. With `link` an expression keeps it so as they move.",
            params: const {
                &[
                    COMP,
                    CAMERA,
                    Param::need("to", Ty::Any, "a layer (an id, name or \"#n\"), or \"pointOfInterest\""),
                    Param::opt("link", Ty::Bool, "keep it in focus as they move, by an expression (default false: set once)"),
                    TIME,
                ]
            },
            returns: "{camera, focusDistance, expression?, notes?}",
            run: Run::Edit(focus),
        },
        Command {
            id: "camera.orbitNull",
            label: "Create Orbit Null",
            summary: "Hang a camera on a new 3D null at the point it looks at: turning the null swings the camera around that point, moving it carries the camera along. The camera stays where it is.",
            params: const { &[COMP, CAMERA, TIME] },
            returns: "{null, camera, position}",
            run: Run::Edit(orbit_null),
        },
        Command {
            id: "camera.stereoRig",
            label: "Create Stereo 3D Rig",
            summary: "Make a composition stereo: a Stereo 3D Controls null by its camera (a new Master Camera if it has none), a left and a right eye composition each showing it collapsed through an eye camera that follows the master by expressions (set apart by the scene depth, turning in to meet if converging), and an output composition putting both eyes together through 3D Glasses.",
            params: const {
                &[
                    COMP,
                    Param::opt("configuration", Ty::Choice(STEREO_CONFIGURATIONS), "where the eyes stand: either side of the camera (stereoPair, default), or the camera the left eye (centerRight) or the right (centerLeft)"),
                    Param::opt("sceneDepth", Ty::Number, "how far apart the eyes are, in % of the composition's width (default 3)"),
                    Param::opt("convergence", Ty::Bool, "the eyes turn in to meet (default false)"),
                    Param::opt("convergeTo", Ty::Choice(CONVERGE_TO), "where they meet: as far as the camera's point of interest (default), its zoom or its focus distance"),
                    Param::opt("zOffset", Ty::Number, "where they meet moved farther (below 0: nearer), in pixels"),
                    Param::opt("view", Ty::Choice(VIEWS_3D), "how 3D Glasses puts the eyes together (default balancedColoredRedBlue)"),
                ]
            },
            returns: "{controls, master, leftComp, rightComp, stereoComp, leftCamera, rightCamera}",
            run: Run::Edit(stereo_rig),
        },
        Command {
            id: "light.followCamera",
            label: "Control Light with Camera",
            summary: "Let lights ride on a camera: parented to it at its eye, shining the way it looks, wherever it goes. With `on` false they are let go, staying where they are then.",
            params: const {
                &[
                    COMP,
                    Param::need("lights", Ty::List, "the lights (ids, names or \"#n\")"),
                    CAMERA,
                    Param::opt("on", Ty::Bool, "ride on the camera (default true) or let go"),
                    TIME,
                ]
            },
            returns: "{camera, lights, following}",
            run: Run::Edit(follow_camera),
        },
    ]
}

fn num(n: f64) -> Json {
    Json::Number((n * 1000.0).round() / 1000.0)
}

fn point(v: Vec3) -> Json {
    Json::array([num(v.x), num(v.y), num(v.z)])
}

/// The camera `camera` names, or the one the composition is seen through at `t`.
fn camera_of(project: &Project, a: &Args, cid: u64, t: Tick) -> Result<u64> {
    let comp = project.comp(cid).expect("found");
    let id = match a.json("camera") {
        Some(j) => find_layer(comp, j)?,
        None => comp
            .layers
            .iter()
            .find(|l| l.source == Source::Camera && l.switches.video && l.is_active_at(t))
            .map(|l| l.id)
            .ok_or_else(|| Error::not_found("the composition is seen through no camera then (layer.add with type camera makes one)"))?,
    };
    let l = comp.layer(id).expect("found");
    if l.source != Source::Camera {
        return Err(Error::params(format!("\"{}\" is not a camera", l.name)));
    }
    Ok(id)
}

/// A turn of `deg` degrees about `axis` (the right-hand way).
fn turn(axis: Vec3, deg: f64) -> Mat4 {
    Quat::from_axis_deg(axis, deg).to_mat4()
}

/// A matrix's turn and scale alone (what it does to directions).
fn linear(m: Mat4) -> Mat4 {
    let mut m = m;
    for row in m.0.iter_mut().take(3) {
        row[3] = 0.0;
    }
    m
}

/// The turn taking direction `from` to `to` the shortest way.
fn between(from: Vec3, to: Vec3) -> Mat4 {
    let (a, b) = (from.normalize(), to.normalize());
    let axis = a.cross(b);
    let cos = a.dot(b).clamp(-1.0, 1.0);
    if axis.length() < 1e-12 {
        if cos > 0.0 {
            return Mat4::IDENTITY;
        }
        // turned right round: about any axis across it
        let across = if a.x.abs() < 0.9 { vec3(1.0, 0.0, 0.0) } else { vec3(0.0, 1.0, 0.0) };
        return turn(a.cross(across), 180.0);
    }
    turn(axis, cos.acos().to_degrees())
}

/// A camera (`lid` of composition `cid`) as its operator sees it at comp time `t`, in the
/// composition's world: where it is, the way it looks, its picture's right and down, the point it
/// looks at (a one-node camera's: as far ahead as its zoom), and its parent's world (to write it
/// back).
struct Rig {
    cid: u64,
    lid: u64,
    t: Tick,
    eye: Vec3,
    forward: Vec3,
    right: Vec3,
    down: Vec3,
    poi: Vec3,
    zoom: f64,
    two_node: bool,
    parent: Mat4,
}

impl Rig {
    fn of(project: &Project, cid: u64, lid: u64, t: Tick) -> Rig {
        let e = Eval::new(project);
        let comp = project.comp(cid).expect("found");
        let at = e.place(comp, comp.layer(lid).expect("found"));
        let world = e.world(at, t, None);
        let parent = e.parent_world(at, t, None);
        let dir = |v: Vec3| world.apply_vec(v).normalize();
        let eye = world.apply(Vec3::ZERO);
        let forward = dir(vec3(0.0, 0.0, 1.0));
        let zoom = e.prop(at, "cameraOptions/zoom", t).map_or(veymelo_project::scene::default_zoom(comp.width as f64), |v| v.as_f64()).max(1e-3);
        let two_node = at.layer.auto_orient == AutoOrient::TowardsPointOfInterest;
        let poi = match e.prop(at, "transform/pointOfInterest", t) {
            Some(v) if two_node => parent.apply(Vec3::from(v.as_vec3())),
            _ => eye + forward * zoom,
        };
        Rig { cid, lid, t, eye, forward, right: dir(vec3(1.0, 0.0, 0.0)), down: dir(vec3(0.0, 1.0, 0.0)), poi, zoom, two_node, parent }
    }
}

/// Refused when what a camera move writes follows an expression (the move would not show).
fn not_driven(layer: &Layer, paths: &[&str]) -> Result<()> {
    for path in paths {
        if layer.props.prop(path).is_ok_and(|p| p.expression.as_ref().is_some_and(|e| e.enabled)) {
            return Err(Error::refused(format!("{path} of \"{}\" follows an expression; change that (or switch it off) instead", layer.name)));
        }
    }
    Ok(())
}

/// What a camera move writes: its position, and its point of interest (two-node) or its
/// orientation (one-node).
fn written(layer: &Layer, two_node: bool) -> Vec<&'static str> {
    let mut paths = if layer.separate_position { vec!["transform/positionX", "transform/positionY", "transform/positionZ"] } else { vec!["transform/position"] };
    paths.push(if two_node { "transform/pointOfInterest" } else { "transform/orientation" });
    paths
}

/// Put a camera's eye at `eye` and the point it looks at at `poi` (in the world) at its rig's
/// time; a one-node camera is turned by `spin` (a turn in the world) instead of aiming.
fn place(cx: &mut Ctx, rig: &Rig, eye: Vec3, poi: Vec3, spin: Mat4) -> Result<Json> {
    let (cid, lid, t) = (rig.cid, rig.lid, rig.t);
    let back = rig.parent.inverse().ok_or_else(|| Error::refused("the camera's parent is scaled to nothing"))?;
    let (e, p) = (back.apply(eye), back.apply(poi));
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    not_driven(layer, &written(layer, rig.two_node))?;
    let lt = layer.layer_time(t);
    put_position(layer, lt, |_| [e.x, e.y, e.z])?;
    if rig.two_node {
        put(layer, "transform/pointOfInterest", Value::Vec3([p.x, p.y, p.z]), lt)?;
    } else {
        // the same turn, seen from its parent's space, before how it faced
        let o = layer.props.prop("transform/orientation").map(|q| q.value_at(lt).as_vec3()).unwrap_or([0.0; 3]);
        let pl = linear(rig.parent);
        let inv = pl.inverse().unwrap_or(Mat4::IDENTITY);
        let o = (inv * spin * pl * Mat4::orientation(Vec3::from(o))).to_orientation();
        put(layer, "transform/orientation", Value::Vec3([o.x, o.y, o.z]), lt)?;
    }
    Ok(Json::object([("camera", Json::Number(lid as f64)), ("position", point(eye)), ("pointOfInterest", point(poi))]))
}

/// The camera a camera command works on, as it stands at the time.
fn begin(cx: &Ctx, a: &Args) -> Result<Rig> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let t = a.time("time").unwrap_or(cx.view.time);
    let lid = camera_of(cx.project, a, cid, t)?;
    Ok(Rig::of(cx.project, cid, lid, t))
}

fn orbit(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let rig = begin(cx, a)?;
    let (yaw, pitch) = (a.num("yaw").unwrap_or(0.0), a.num("pitch").unwrap_or(0.0));
    let up = vec3(0.0, -1.0, 0.0);
    let off = rig.eye - rig.poi;
    if off.length() < 1e-9 {
        return Err(Error::refused("the camera stands on the point it looks at, so there is nothing to orbit about"));
    }
    // up or down about the level line across the view, kept short of straight above or below
    let height = off.normalize().dot(up).clamp(-1.0, 1.0).asin().to_degrees();
    let pitch = (height + pitch).clamp(-89.0, 89.0) - height;
    let across = off.cross(up);
    let across = if across.length() < 1e-9 { rig.right * -1.0 } else { across.normalize() };
    let spin = turn(up, yaw) * turn(across, pitch);
    let eye = rig.poi + spin.apply_vec(off);
    let out = place(cx, &rig, eye, rig.poi, spin)?;
    cx.label = Some("Orbit Camera".into());
    Ok(out)
}

fn pan(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let rig = begin(cx, a)?;
    // a pixel of its picture where it looks is this far there
    let k = (rig.poi - rig.eye).dot(rig.forward).abs().max(1e-6) / rig.zoom;
    let d = rig.right * (a.num("x").unwrap_or(0.0) * k) + rig.down * (a.num("y").unwrap_or(0.0) * k);
    let out = place(cx, &rig, rig.eye + d, rig.poi + d, Mat4::IDENTITY)?;
    cx.label = Some("Track Camera".into());
    Ok(out)
}

fn dolly(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let rig = begin(cx, a)?;
    let by = a.num("by").unwrap_or(0.0);
    let (eye, poi) = if a.bool("still").unwrap_or(false) {
        let to = rig.poi - rig.eye;
        let gap = to.length();
        if gap < 1.0 + 1e-9 && by > 0.0 {
            return Err(Error::refused("the camera is already at the point it looks at"));
        }
        (rig.eye + to.normalize() * by.min(gap - 1.0), rig.poi)
    } else {
        (rig.eye + rig.forward * by, rig.poi + rig.forward * by)
    };
    let out = place(cx, &rig, eye, poi, Mat4::IDENTITY)?;
    cx.label = Some("Dolly Camera".into());
    Ok(out)
}

/// The corners of what 3D layers show (their anchor points, for those showing nothing), in the
/// composition's world at `t`.
fn corners(project: &Project, cid: u64, ids: &[u64], t: Tick) -> Vec<Vec3> {
    let e = Eval::new(project);
    let comp = project.comp(cid).expect("found");
    let mut out = Vec::new();
    for id in ids {
        let layer = comp.layer(*id).expect("found");
        let at = e.place(comp, layer);
        let world = e.world(at, t, None);
        match content_box(project, cid, *id, t) {
            Some([x0, y0, x1, y1]) => out.extend([(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| world.apply(vec3(x, y, 0.0)))),
            None => out.push(world.apply(Vec3::from(e.prop(at, "transform/anchor", t).map_or([0.0; 3], |v| v.as_vec3())))),
        }
    }
    out
}

fn look_at(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let rig = begin(cx, a)?;
    let (cid, lid, t) = (rig.cid, rig.lid, rig.t);
    let comp = cx.project.comp(cid).expect("found");
    let (w, h) = (comp.width as f64, comp.height as f64);
    let ids: Vec<u64> = match a.json("layers").and_then(Json::as_array) {
        Some(list) => {
            let mut ids = Vec::new();
            for j in list {
                let id = find_layer(comp, j)?;
                let l = comp.layer(id).expect("found");
                if matches!(l.source, Source::Camera | Source::Light(_)) {
                    return Err(Error::params(format!("\"{}\" is a {}: it shows nothing to look at", l.name, if l.source == Source::Camera { "camera" } else { "light" })));
                }
                if !l.is_3d() {
                    return Err(Error::params(format!("\"{}\" is 2D, so the camera does not see it (layer.set switches.threeD makes it 3D)", l.name)));
                }
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            ids
        }
        None => comp.layers.iter().filter(|l| l.is_3d() && l.switches.video && l.is_active_at(t) && !matches!(l.source, Source::Camera | Source::Light(_))).map(|l| l.id).collect(),
    };
    if ids.is_empty() {
        return Err(Error::not_found("there are no 3D layers to look at then"));
    }
    let pts = corners(cx.project, cid, &ids, t);
    let (lo, hi) = pts.iter().fold((vec3(f64::INFINITY, f64::INFINITY, f64::INFINITY), vec3(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY)), |(lo, hi), p| {
        (vec3(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z)), vec3(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z)))
    });
    let middle = (lo + hi) * 0.5;
    let keep = a.bool("keepDirection").unwrap_or(false);
    let way = if keep { rig.forward } else { middle - rig.eye };
    if way.length() < 1e-9 {
        return Err(Error::refused("the camera stands where they are: move it back first (camera.dolly)"));
    }
    let way = way.normalize();
    let spin = between(rig.forward, way);
    // turned (or moved sideways to them) first, as far from them as it is
    let reach = (middle - rig.eye).dot(way).max(1.0);
    let mut out = place(cx, &rig, middle - way * reach, middle, spin)?;
    let mut distance = reach;
    if a.bool("fit").unwrap_or(true) {
        // then as near as keeps every corner inside its picture, with the margin
        let now = Rig::of(cx.project, cid, lid, t);
        let margin = a.num("margin").unwrap_or(5.0).clamp(0.0, 45.0) / 100.0;
        let (hw, hh) = (w / 2.0 * (1.0 - 2.0 * margin), h / 2.0 * (1.0 - 2.0 * margin));
        let mut need = f64::NEG_INFINITY;
        let mut front = 1.0_f64;
        for p in &pts {
            let q = *p - middle;
            let (x, y, z) = (q.dot(now.right), q.dot(now.down), q.dot(now.forward));
            need = need.max(now.zoom * x.abs() / hw - z).max(now.zoom * y.abs() / hh - z);
            front = front.max(1.0 - z);
        }
        distance = if (hi - lo).length() < 1e-6 { now.zoom } else { need.max(front) };
        out = place(cx, &now, middle - now.forward * distance, middle, Mat4::IDENTITY)?;
    }
    if let Json::Object(o) = &mut out {
        o.insert("distance", num(distance));
    }
    cx.label = Some("Look at Layers".into());
    Ok(out)
}

/// A JavaScript string of `s`.
fn quote(s: &str) -> String {
    Json::from(s).to_json()
}

fn focus(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let rig = begin(cx, a)?;
    let (cid, lid, t) = (rig.cid, rig.lid, rig.t);
    let comp = cx.project.comp(cid).expect("found");
    let to = a.need("to");
    let (distance, text) = if to.as_str() == Some("pointOfInterest") {
        if !rig.two_node {
            return Err(Error::params("a one-node camera has no point of interest: focus it on a layer"));
        }
        ((rig.poi - rig.eye).length(), "length(sub(transform.pointOfInterest, transform.position))".to_string())
    } else {
        let id = find_layer(comp, to)?;
        if id == lid {
            return Err(Error::params("a camera cannot focus on itself"));
        }
        let target = comp.layer(id).expect("found");
        let e = Eval::new(cx.project);
        let at = e.place(comp, target);
        let anchor = Vec3::from(e.prop(at, "transform/anchor", t).map_or([0.0; 3], |v| v.as_vec3()));
        let there = e.world(at, t, None).apply(anchor);
        let d = (there - rig.eye).dot(rig.forward);
        if d <= 0.0 {
            return Err(Error::refused(format!("\"{}\" is behind the camera then", target.name)));
        }
        let text = format!(
            "var target = thisComp.layer({});\ndot(sub(target.toWorld(target.transform.anchorPoint), toWorld([0, 0, 0])), normalize(toWorldVec([0, 0, 1])));",
            quote(&target.name)
        );
        (d, text)
    };
    let dof = comp.layer(lid).expect("found").props.prop("cameraOptions/depthOfField").is_ok_and(|p| p.value.as_bool());
    let path = "cameraOptions/focusDistance";
    let mut o = veymelo_json::Map::new();
    o.insert("camera", lid as f64);
    o.insert("focusDistance", num(distance));
    if a.bool("link").unwrap_or(false) {
        let r = super::call(cx, "prop.expression", Json::object([("comp", Json::Number(cid as f64)), ("layer", Json::Number(lid as f64)), ("path", Json::from(path)), ("text", Json::from(text.as_str()))]))?;
        if let Some(err) = r.get("error").and_then(Json::as_str) {
            return Err(Error::refused(format!("the focus expression does not work: {err}")));
        }
        o.insert("expression", text.as_str());
    } else {
        let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
        not_driven(layer, &[path])?;
        let lt = layer.layer_time(t);
        put(layer, path, Value::Number(distance), lt)?;
    }
    if !dof {
        o.insert("notes", Json::array([Json::from("its depth of field is off, so all is sharp: prop.set cameraOptions/depthOfField true to see it")]));
    }
    cx.label = Some("Focus Camera".into());
    Ok(Json::Object(o))
}

fn orbit_null(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let rig = begin(cx, a)?;
    let (cid, lid) = (rig.cid, rig.lid);
    let comp = cx.project.comp(cid).expect("found");
    let cam = comp.layer(lid).expect("found");
    if let Some(p) = cam.parent.and_then(|p| comp.layer(p)) {
        return Err(Error::refused(format!("\"{}\" already hangs on \"{}\"", cam.name, p.name)));
    }
    not_driven(cam, &written(cam, rig.two_node))?;
    let (name, index) = (format!("{} Orbit Null", cam.name), comp.layers.iter().position(|l| l.id == lid).expect("found"));
    let made = super::call(cx, "layer.add", Json::object([("comp", Json::Number(cid as f64)), ("type", Json::from("null")), ("name", Json::from(name.as_str())), ("at", Json::Number((index + 1) as f64))]))?;
    let nid = made.get("layer").and_then(Json::as_u64).expect("made");
    let comp = cx.project.comp_mut(cid).expect("found");
    let null = comp.layer_mut(nid).expect("made");
    null.switches.three_d = true;
    let target = rig.poi;
    null.props.prop_mut("transform/position").expect("defined").set_value(Value::Vec3([target.x, target.y, target.z])).map_err(Error::refused)?;
    let anchor = Vec3::from(null.props.prop("transform/anchor").map(|p| p.value.as_vec3()).unwrap_or([0.0; 3]));
    // in the null's space (moved, not turned or scaled): a point there is where it was, less
    // the null's place, plus its anchor; every key alike, so the camera moves as it did
    let shift = anchor - target;
    let cam = comp.layer_mut(lid).expect("found");
    cam.parent = Some(nid);
    let moved = |p: &mut veymelo_project::Property, f: &dyn Fn(&Value) -> Value| {
        p.value = f(&p.value);
        for k in &mut p.keys {
            k.value = f(&k.value);
        }
    };
    let by = |v: &Value| {
        let q = v.as_vec3();
        Value::Vec3([q[0] + shift.x, q[1] + shift.y, q[2] + shift.z])
    };
    if cam.separate_position {
        for (axis, d) in [("positionX", shift.x), ("positionY", shift.y), ("positionZ", shift.z)] {
            if let Ok(p) = cam.props.prop_mut(&format!("transform/{axis}")) {
                moved(p, &|v| Value::Number(v.as_f64() + d));
            }
        }
    } else if let Ok(p) = cam.props.prop_mut("transform/position") {
        moved(p, &by);
    }
    if let Ok(p) = cam.props.prop_mut("transform/pointOfInterest") {
        moved(p, &by);
    }
    cx.label = Some("Create Orbit Null".into());
    Ok(Json::object([("null", Json::Number(nid as f64)), ("camera", Json::Number(lid as f64)), ("position", point(target))]))
}

/// Where each eye stands along the camera's right, in scene depths: left and right, for each
/// configuration.
const EYES: [[f64; 2]; 3] = [[-0.5, 0.5], [0.0, 1.0], [-1.0, 0.0]];

/// The expressions an eye camera follows its master by: its position, point of interest and
/// roll. `ahead` is the master's distance to what it looks at.
fn eye_expressions(comp: &str, master: &str, controls: &str, ahead: &str, eye: usize) -> [String; 3] {
    let (c, m) = (quote(comp), quote(master));
    let head = format!(
        "var src = comp({c});\nvar m = src.layer({m});\nvar fx = src.layer({x}).effect(\"Stereo 3D Controls\");\nvar cfg = fx(\"Configuration\").value;\n\
         var k = cfg == 2 ? {k1} : (cfg == 3 ? {k2} : {k0});\nvar E = m.toWorld([0, 0, 0]);\nvar X = normalize(m.toWorldVec([1, 0, 0]));\n\
         var eye = add(E, mul(X, k * fx(\"Stereo Scene Depth\").value / 100 * src.width));\n",
        x = quote(controls),
        k0 = EYES[0][eye],
        k1 = EYES[1][eye],
        k2 = EYES[2][eye],
    );
    let position = format!("{head}eye;");
    let poi = format!(
        "{head}var F = normalize(m.toWorldVec([0, 0, 1]));\nvar ahead = {ahead};\nvar to = fx(\"Converge To\").value;\n\
         var meet = (to == 2 ? m.cameraOption.zoom : (to == 3 ? m.cameraOption.focusDistance : ahead)) + fx(\"Convergence Z Offset\").value;\n\
         fx(\"Enable Convergence\").value ? add(E, mul(F, Math.max(meet, 1))) : add(eye, mul(F, ahead));"
    );
    // its roll: how far the master's right is turned from level about the way it looks
    let roll = format!(
        "var m = comp({c}).layer({m});\nvar F = normalize(m.toWorldVec([0, 0, 1]));\nvar X = normalize(m.toWorldVec([1, 0, 0]));\nvar D = sub([0, 1, 0], mul(F, F[1]));\n\
         length(D) < 1e-9 ? value : radiansToDegrees(Math.atan2(dot(X, normalize(D)), dot(X, cross(normalize(D), F))));"
    );
    [position, poi, roll]
}

fn stereo_rig(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let t = cx.view.time;
    let src = cx.project.comp(cid).expect("found").clone();
    let item = cx.project.item(cid).expect("found");
    let (src_name, folder) = (item.name.clone(), item.folder);
    if cx.project.items.iter().any(|i| i.as_comp().is_some() && i.name == src_name && i.id != cid) {
        return Err(Error::refused(format!("there is more than one composition called \"{src_name}\": the eye cameras find it by its name, so rename one first")));
    }
    let pick = |key: &str, ids: &[&str], default: usize| a.text(key).and_then(|v| ids.iter().position(|i| *i == v)).unwrap_or(default);
    let configuration = pick("configuration", STEREO_CONFIGURATIONS, 0);
    let converge_to = pick("convergeTo", CONVERGE_TO, 0);
    let view = pick("view", VIEWS_3D, 5);
    let depth = a.num("sceneDepth").unwrap_or(3.0);
    if !(0.0..=100.0).contains(&depth) {
        return Err(Error::params(format!("`sceneDepth` is from 0 to 100 %, not {depth}")));
    }
    let id = |j: &Json, k: &str| j.get(k).and_then(Json::as_u64).expect("made");
    let at = |c: u64| Json::Number(c as f64);
    // the master: the camera it is seen through, or a new one
    let master = match src.layers.iter().find(|l| l.source == Source::Camera && l.switches.video && l.is_active_at(t)) {
        Some(l) => l.id,
        None => id(&super::call(cx, "layer.add", Json::object([("comp", at(cid)), ("type", Json::from("camera")), ("name", Json::from("Master Camera"))]))?, "layer"),
    };
    let comp = cx.project.comp(cid).expect("found");
    let master_layer = comp.layer(master).expect("found");
    let master_name = master_layer.name.clone();
    if comp.layers.iter().filter(|l| l.name == master_name).count() > 1 {
        return Err(Error::refused(format!("more than one layer is called \"{master_name}\": the eye cameras find the master by its name, so rename one first")));
    }
    let ahead = if master_layer.auto_orient == AutoOrient::TowardsPointOfInterest { "length(sub(m.transform.pointOfInterest, m.transform.position))" } else { "m.cameraOption.zoom" };
    let controls_name = super::fresh_name("Stereo 3D Controls", comp.layers.iter().map(|l| l.name.as_str()));
    let index = comp.layers.iter().position(|l| l.id == master).expect("found");
    let controls = id(&super::call(cx, "layer.add", Json::object([("comp", at(cid)), ("type", Json::from("null")), ("name", Json::from(controls_name.as_str())), ("at", Json::Number((index + 1) as f64))]))?, "layer");
    super::call(
        cx,
        "effect.add",
        Json::object([
            ("comp", at(cid)),
            ("layer", at(controls)),
            ("type", Json::from("stereoControls")),
            (
                "props",
                Json::object([
                    ("configuration", Json::from(STEREO_CONFIGURATIONS[configuration])),
                    ("sceneDepth", Json::Number(depth)),
                    ("convergence", Json::Bool(a.bool("convergence").unwrap_or(false))),
                    ("convergeTo", Json::from(CONVERGE_TO[converge_to])),
                    ("zOffset", Json::Number(a.num("zOffset").unwrap_or(0.0))),
                ]),
            ),
        ]),
    )?;
    // a composition like this one, empty
    let sibling = |cx: &mut Ctx, name: &str| {
        let mut c = src.clone();
        c.layers.clear();
        c.markers.clear();
        c.essentials = Default::default();
        let name = super::fresh_name(name, cx.project.items.iter().map(|i| i.name.as_str()));
        let name = name.strip_suffix(" 1").filter(|n| !cx.project.items.iter().any(|i| i.name == *n)).map_or(name.clone(), str::to_string);
        let id = cx.project.add_comp(&name, c);
        cx.project.item_mut(id).expect("made").folder = folder;
        (id, name)
    };
    let master_options: Vec<(String, Value)> = {
        let l = cx.project.comp(cid).expect("found").layer(master).expect("found");
        let mut all = Vec::new();
        if let Ok(g) = l.props.group("cameraOptions") {
            g.walk(&mut |path, p| all.push((format!("cameraOptions/{path}"), p.value_at(l.layer_time(t)))));
        }
        all
    };
    let mut eyes = Vec::new();
    for (eye, side) in ["Left", "Right"].into_iter().enumerate() {
        let (eid, _) = sibling(cx, &format!("{src_name} {side} Eye"));
        let nest = id(&super::call(cx, "layer.add", Json::object([("comp", at(eid)), ("type", Json::from("comp")), ("item", at(cid))]))?, "layer");
        {
            let l = cx.project.comp_mut(eid).expect("made").layer_mut(nest).expect("made");
            l.switches.three_d = true;
            l.switches.collapse = true;
        }
        let camera = id(&super::call(cx, "layer.add", Json::object([("comp", at(eid)), ("type", Json::from("camera")), ("name", Json::from(format!("{side} Eye Camera")))]))?, "layer");
        {
            let l = cx.project.comp_mut(eid).expect("made").layer_mut(camera).expect("made");
            for (path, v) in &master_options {
                if let Ok(p) = l.props.prop_mut(path) {
                    let _ = p.set_value(v.clone());
                }
            }
        }
        let [position, poi, roll] = eye_expressions(&src_name, &master_name, &controls_name, ahead, eye);
        let same = |k: &str| format!("comp({}).layer({}).cameraOption.{k};", quote(&src_name), quote(&master_name));
        for (path, text) in [
            ("transform/position", position),
            ("transform/pointOfInterest", poi),
            ("transform/rotation", roll),
            ("cameraOptions/zoom", same("zoom")),
            ("cameraOptions/focusDistance", same("focusDistance")),
            ("cameraOptions/aperture", same("aperture")),
            ("cameraOptions/blurLevel", same("blurLevel")),
        ] {
            let r = super::call(cx, "prop.expression", Json::object([("comp", at(eid)), ("layer", at(camera)), ("path", Json::from(path)), ("text", Json::from(text))]))?;
            if let Some(err) = r.get("error").and_then(Json::as_str) {
                return Err(Error::refused(format!("the {side} eye camera's {path} expression does not work: {err}")));
            }
        }
        eyes.push((eid, camera));
    }
    // both eyes through 3D Glasses: the left one shows, the right one is read
    let (out, _) = sibling(cx, &format!("{src_name} Stereo 3D"));
    let right = id(&super::call(cx, "layer.add", Json::object([("comp", at(out)), ("type", Json::from("comp")), ("item", at(eyes[1].0))]))?, "layer");
    cx.project.comp_mut(out).expect("made").layer_mut(right).expect("made").switches.video = false;
    let left = id(&super::call(cx, "layer.add", Json::object([("comp", at(out)), ("type", Json::from("comp")), ("item", at(eyes[0].0))]))?, "layer");
    super::call(
        cx,
        "effect.add",
        Json::object([
            ("comp", at(out)),
            ("layer", at(left)),
            ("type", Json::from("glasses3d")),
            ("props", Json::object([("leftView", at(left)), ("rightView", at(right)), ("view3d", Json::from(VIEWS_3D[view]))])),
        ]),
    )?;
    cx.label = Some("Create Stereo 3D Rig".into());
    Ok(Json::object([
        ("controls", at(controls)),
        ("master", at(master)),
        ("leftComp", at(eyes[0].0)),
        ("rightComp", at(eyes[1].0)),
        ("stereoComp", at(out)),
        ("leftCamera", at(eyes[0].1)),
        ("rightCamera", at(eyes[1].1)),
    ]))
}

fn follow_camera(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let t = a.time("time").unwrap_or(cx.view.time);
    let cam = camera_of(cx.project, a, cid, t)?;
    let comp = cx.project.comp(cid).expect("found");
    let mut lights = Vec::new();
    for j in a.json("lights").and_then(Json::as_array).cloned().unwrap_or_default() {
        let id = find_layer(comp, &j)?;
        let l = comp.layer(id).expect("found");
        if !matches!(l.source, Source::Light(_)) {
            return Err(Error::params(format!("\"{}\" is not a light", l.name)));
        }
        if !lights.contains(&id) {
            lights.push(id);
        }
    }
    if lights.is_empty() {
        return Err(Error::params("give one light or more"));
    }
    let on = a.bool("on").unwrap_or(true);
    let e = Eval::new(cx.project);
    let seen: Vec<(u64, Mat4)> = lights.iter().map(|id| (*id, e.world(e.place(comp, comp.layer(*id).expect("found")), t, None))).collect();
    let (w, h) = (comp.width as f64, comp.height as f64);
    let mut notes = Vec::new();
    for (id, world) in seen {
        let layer = cx.project.comp_mut(cid).expect("found").layer_mut(id).expect("found");
        let mut paths = written(layer, true);
        paths.extend(["transform/orientation", "transform/rotationX", "transform/rotationY", "transform/rotation"]);
        not_driven(layer, &paths)?;
        let reset = |l: &mut Layer, path: &str, v: Value| {
            if let Ok(p) = l.props.prop_mut(path) {
                p.keys.clear();
                let _ = p.set_value(v);
            }
        };
        let set_position = |l: &mut Layer, v: Vec3| {
            if l.separate_position {
                for (axis, x) in [("positionX", v.x), ("positionY", v.y), ("positionZ", v.z)] {
                    reset(l, &format!("transform/{axis}"), Value::Number(x));
                }
            } else {
                reset(l, "transform/position", Value::Vec3([v.x, v.y, v.z]));
            }
        };
        if on {
            if layer.parent.is_some_and(|p| p != cam) {
                notes.push(format!("\"{}\" was let go of its parent", layer.name));
            }
            if layer.props.prop("transform/position").is_ok_and(|p| p.is_animated()) || layer.props.prop("transform/pointOfInterest").is_ok_and(|p| p.is_animated()) {
                notes.push(format!("the keys of \"{}\" were taken away: it moves with the camera now", layer.name));
            }
            layer.parent = Some(cam);
            set_position(layer, Vec3::ZERO);
            reset(layer, "transform/pointOfInterest", Value::Vec3([0.0, 0.0, w.max(h)]));
            reset(layer, "transform/orientation", Value::Vec3([0.0; 3]));
            for k in ["rotationX", "rotationY", "rotation"] {
                reset(layer, &format!("transform/{k}"), Value::Number(0.0));
            }
            layer.auto_orient = AutoOrient::TowardsPointOfInterest;
        } else if layer.parent == Some(cam) {
            // let go where it is then, shining the same way
            let (eye, fwd) = (world.apply(Vec3::ZERO), world.apply_vec(vec3(0.0, 0.0, 1.0)).normalize());
            layer.parent = None;
            set_position(layer, eye);
            let poi = eye + fwd * w.max(h);
            reset(layer, "transform/pointOfInterest", Value::Vec3([poi.x, poi.y, poi.z]));
            // what turned it besides aiming, as an orientation after it
            let aim = veymelo_expr::facing(fwd);
            let rest = aim.inverse().unwrap_or(Mat4::IDENTITY) * linear(world);
            let o = rest.to_orientation();
            reset(layer, "transform/orientation", Value::Vec3([o.x, o.y, o.z]));
            for k in ["rotationX", "rotationY", "rotation"] {
                reset(layer, &format!("transform/{k}"), Value::Number(0.0));
            }
            layer.auto_orient = AutoOrient::TowardsPointOfInterest;
        }
    }
    cx.label = Some(if on { "Control Light with Camera" } else { "Release Light from Camera" }.into());
    let mut o = veymelo_json::Map::new();
    o.insert("camera", cam as f64);
    o.insert("lights", Json::array(lights.iter().map(|id| Json::Number(*id as f64))));
    o.insert("following", on);
    if !notes.is_empty() {
        o.insert("notes", Json::array(notes.into_iter().map(Json::from)));
    }
    Ok(Json::Object(o))
}

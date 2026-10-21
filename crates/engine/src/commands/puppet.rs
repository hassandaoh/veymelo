//! A puppet's pins animated by hand: a pin dragged over time recorded as its position's keys
//! (other pins dragged along with it), a pin sent along a path, and pins that trail another the
//! way hair, cloth or a tail trails what pulls it (follow-through), by expressions.

use veymelo_json::Value as Json;
use veymelo_keyframe::Value;
use veymelo_project::{Group, Layer, Node};

use super::assist::{Take, recorded};
use super::key::store;
use super::link::{plain_reference, reference};
use super::{COMP, Command, LAYER, Run, find_layer, layer_ids};
use crate::{Args, Ctx, Error, Param, Result, Ty};

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "puppet.record",
            label: "Record Puppet Pin",
            summary: "Record a puppet pin dragged over time: where it went, sampled as it moved ([t, x, y], t in seconds from the drag's start, in the layer's pixels), becomes its position's keys, a key a frame from `start` on, then smoothed, in place of the keys there. Other pins in `pins` are dragged along: each moves as much as it does, from where it is then.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::need("pin", Ty::Text, "the pin dragged: its path (puppet.pin gives it) or its name"),
                    Param::need("points", Ty::List, "[[t, x, y], …]: where it was at each moment of the drag"),
                    Param::opt("pins", Ty::List, "more pins dragged along with it"),
                    Param::opt("start", Ty::Time, "where the keys start (default: the view's time)"),
                    Param::opt("speed", Ty::Number, "% (100: plays back as fast as it was dragged; 200: half as fast)"),
                    Param::opt("smoothing", Ty::Number, "pixels the keys may stray from the drag (0: a key every frame, as dragged) (default 1)"),
                ]
            },
            returns: "{keys, start, end, pins}",
            run: Run::Edit(record),
        },
        Command {
            id: "puppet.alongPath",
            label: "Pin Along a Path",
            summary: "Send a puppet pin along a path at one speed from `start` for `duration`: a mask's or shape's path (`path`, on this layer or on `from`), or points given in the layer's pixels. Its position gets a key at each of the path's corners, the curve between them the path's own, the keys between the first and the last roving (so it never speeds up or slows down); its keys there before are replaced.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::need("pin", Ty::Text, "the pin: its path (puppet.pin gives it) or its name"),
                    Param::opt("path", Ty::Text, "the path to follow: a property holding one, as \"masks/Mask 1/path\" or \"contents/Shape 1/contents/Path 1/path\""),
                    Param::opt("from", Ty::Layer, "the layer `path` is on (default this one); its path is taken where it is in the composition at `start`"),
                    Param::opt("points", Ty::List, "or the path as [[x, y], …] in the layer's pixels (straight between them)"),
                    Param::opt("start", Ty::Time, "when it sets out (default: the view's time)"),
                    Param::opt("duration", Ty::Number, "how long it takes, in seconds (default 2)"),
                    Param::opt("reverse", Ty::Bool, "go the other way round"),
                ]
            },
            returns: "{pin, keys, start, end}",
            run: Run::Edit(along_path),
        },
        Command {
            id: "puppet.follow",
            label: "Follow-Through",
            summary: "Make pins trail a leader pin, as hair, cloth or a tail trails what pulls it: each moves as the leader moved a little while before (`delay`), by an expression on its position (its own keys still count). With `cascade` the pin nearest the leader trails by the delay, the next by twice it, and so on.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::need("leader", Ty::Text, "the pin they trail: its path or its name"),
                    Param::need("pins", Ty::List, "the pins that trail it"),
                    Param::opt("delay", Ty::Number, "how far behind, in seconds (default 0.1)"),
                    Param::opt("amount", Ty::Number, "how much of the leader's move they take, in % (default 100)"),
                    Param::opt("cascade", Ty::Bool, "each farther pin trails by one delay more (default true)"),
                ]
            },
            returns: "{leader, pins: [{pin, delay}]}",
            run: Run::Edit(follow),
        },
    ]
}

/// The instances in list `list` of `g`.
fn groups<'a>(g: &'a Group, list: &str) -> Vec<&'a Group> {
    g.group(list).map(|l| l.children.iter().filter_map(Node::as_group).collect()).unwrap_or_default()
}

/// Every pin of a layer's puppets, with its path.
fn pins(layer: &Layer) -> Vec<(String, &Group)> {
    let mut out = Vec::new();
    let Ok(fx) = layer.props.group("effects") else { return out };
    for e in fx.children.iter().filter_map(Node::as_group).filter(|g| g.def.id == "puppet") {
        for m in groups(e, "meshes") {
            for p in groups(m, "pins") {
                out.push((format!("effects/{}/meshes/{}/pins/{}", e.name(), m.name(), p.name()), p));
            }
        }
    }
    out
}

/// The path of the pin `name` names (its path, or its name) on `layer`, which must move: have a
/// position.
fn pin(layer: &Layer, name: &str) -> Result<String> {
    let all = pins(layer);
    let found = all.iter().find(|(path, _)| path == name.trim_matches('/')).or_else(|| all.iter().find(|(_, g)| g.name() == name));
    let Some((path, g)) = found else {
        let have = if all.is_empty() { "it has no puppet pins (puppet.pin puts them)".to_string() } else { format!("it has {}", all.iter().map(|(p, _)| format!("\"{p}\"")).collect::<Vec<_>>().join(", ")) };
        return Err(Error::not_found(format!("\"{}\" has no puppet pin \"{name}\" ({have})", layer.name)));
    };
    if g.prop("position").is_err() {
        return Err(Error::params(format!("\"{}\" is a bend pin, which has no position: it goes where the other pins take it", g.name())));
    }
    Ok(path.clone())
}

/// The pins a list names, each once, leaving out `but`.
fn pin_list(layer: &Layer, a: &Args, key: &str, but: &str) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for j in a.json(key).and_then(Json::as_array).cloned().unwrap_or_default() {
        let name = j.as_str().ok_or_else(|| Error::params(format!("`{key}` is a list of pins' paths or names, not {}", j.to_json())))?;
        let p = pin(layer, name)?;
        if p != but && !out.contains(&p) {
            out.push(p);
        }
    }
    Ok(out)
}

fn record(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let (rate, end) = (comp.frame_rate, comp.duration);
    let layer = comp.layer(lid).expect("found");
    let lead = pin(layer, a.text("pin").unwrap_or(""))?;
    let mut moved = vec![lead.clone()];
    moved.extend(pin_list(layer, a, "pins", &lead)?);
    let mut drawn: Vec<(f64, [f64; 2])> = Vec::new();
    for (i, v) in a.json("points").and_then(Json::as_array).cloned().unwrap_or_default().iter().enumerate() {
        let n: Vec<f64> = v.as_array().map(|a| a.iter().filter_map(Json::as_f64).collect()).unwrap_or_default();
        if n.len() != 3 || n.iter().any(|x| !x.is_finite()) {
            return Err(Error::params(format!("point {} is [t, x, y], not {}", i + 1, v.to_json())));
        }
        drawn.push((n[0], [n[1], n[2]]));
    }
    if drawn.is_empty() {
        return Err(Error::params("`points` needs a point at least"));
    }
    drawn.sort_by(|p, q| p.0.total_cmp(&q.0));
    let start = rate.snap_nearest(a.time("start").unwrap_or(cx.view.time));
    let take = Take { start, speed: a.num("speed").unwrap_or(100.0).clamp(1.0, 10_000.0) / 100.0, smoothing: a.num("smoothing").unwrap_or(1.0).max(0.0), rate, end };
    let lt = layer.layer_time(start);
    let from = drawn[0].1;
    // each pin's new keys: the drag itself, or as much as it, from where the pin is then
    let mut made = Vec::new();
    for path in &moved {
        let pos = layer.props.prop(&format!("{path}/position")).map_err(Error::not_found)?;
        let base = if *path == lead { from } else { pos.value_at(lt).as_vec2() };
        let along: Vec<(f64, [f64; 2])> = drawn.iter().map(|(t, q)| (*t, [base[0] + q[0] - from[0], base[1] + q[1] - from[1]])).collect();
        let (keys, t0, t1) = recorded(&pos.keys, &along, &take, layer, Value::Vec2).ok_or_else(|| Error::params("the drag falls after the composition's end"))?;
        made.push((path.clone(), keys, t0, t1));
    }
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let (t0, t1) = (made[0].2, made[0].3);
    let n = made[0].1.iter().filter(|k| k.time >= t0 && k.time <= t1).count();
    for (path, keys, _, _) in made {
        store(layer, &format!("{path}/position"), keys)?;
    }
    cx.label = Some("Record Puppet Pin".into());
    Ok(Json::object([
        ("keys", Json::Number(n as f64)),
        ("start", Json::Number(layer.comp_time(t0).seconds())),
        ("end", Json::Number(layer.comp_time(t1).seconds())),
        ("pins", Json::array(moved.into_iter().map(Json::from))),
    ]))
}

fn follow(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp(cid).expect("found").layer(lid).expect("found");
    let leader = pin(layer, a.text("leader").unwrap_or(""))?;
    let mut followers = pin_list(layer, a, "pins", &leader)?;
    if followers.is_empty() {
        return Err(Error::params("give `pins`: the pins that trail the leader"));
    }
    let delay = a.num("delay").unwrap_or(0.1);
    if !(0.0..=10.0).contains(&delay) {
        return Err(Error::params(format!("`delay` is from 0 to 10 seconds, not {delay}")));
    }
    let amount = a.num("amount").unwrap_or(100.0) / 100.0;
    let cascade = a.bool("cascade").unwrap_or(true);
    let rest = |path: &str| layer.props.prop(&format!("{path}/rest")).map_or([0.0; 2], |p| p.value.as_vec2());
    let at = rest(&leader);
    // nearest the leader first
    let far = |p: &String| (rest(p)[0] - at[0]).hypot(rest(p)[1] - at[1]);
    followers.sort_by(|p, q| far(p).total_cmp(&far(q)));
    let path = format!("{leader}/position");
    let refs = [reference(layer, layer, &path)?, plain_reference(layer, layer, &path)?];
    let mut out = Vec::new();
    for (k, f) in followers.iter().enumerate() {
        let d = ((if cascade { delay * (k + 1) as f64 } else { delay }) * 1e6).round() / 1e6;
        let mut last = None;
        for r in &refs {
            let text = format!("var lead = {r};\nvalue + (lead.valueAtTime(time - {d}) - [{}, {}]) * {amount}", at[0], at[1]);
            let got = super::call(cx, "prop.expression", Json::object([("comp", Json::Number(cid as f64)), ("layer", Json::Number(lid as f64)), ("path", Json::from(format!("{f}/position"))), ("text", Json::from(text))]))?;
            last = got.get("error").and_then(Json::as_str).map(str::to_string);
            if last.is_none() {
                break;
            }
        }
        if let Some(why) = last {
            return Err(Error::refused(format!("the follow-through expression on {f} does not work: {why}")));
        }
        out.push(Json::object([("pin", Json::from(f.as_str())), ("delay", Json::Number(d))]));
    }
    cx.label = Some("Follow-Through".into());
    Ok(Json::object([("leader", Json::from(leader)), ("pins", Json::Array(out))]))
}

/// A path's corners, their in and out tangents, and whether it closes.
type Corners = (Vec<[f64; 2]>, Vec<[f64; 2]>, Vec<[f64; 2]>, bool);

fn along_path(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let layer = comp.layer(lid).expect("found");
    let target = pin(layer, a.text("pin").unwrap_or(""))?;
    let start = comp.frame_rate.snap_nearest(a.time("start").unwrap_or(cx.view.time));
    let duration = a.num("duration").unwrap_or(2.0);
    if !(duration > 0.0 && duration <= 3600.0) {
        return Err(Error::params(format!("`duration` is more than 0 and at most 3600 seconds, not {duration}")));
    }
    // the path: its corners and their tangents, in this layer's pixels
    let (mut points, mut ins, mut outs, closed): Corners = match (a.text("path"), a.json("points")) {
        (Some(path), _) => {
            let from = match a.json("from") {
                Some(j) => find_layer(comp, j)?,
                None => lid,
            };
            let other = comp.layer(from).expect("found");
            let eval = veymelo_expr::Eval::new(cx.project);
            let p = other.props.prop(path).map_err(Error::not_found)?;
            let Value::Path(shape) = eval.value(p, eval.place(comp, other), start) else {
                return Err(Error::params(format!("\"{path}\" on \"{}\" is not a path", other.name)));
            };
            // from its layer to the composition, then into this one
            let there = eval.world(eval.place(comp, other), start, None);
            let here = eval.world(eval.place(comp, layer), start, None).inverse().ok_or_else(|| Error::refused(format!("\"{}\" is squashed flat at that time", layer.name)))?;
            let m = here * there;
            let point = |q: [f64; 2]| {
                let r = m.apply(veymelo_geom::vec3(q[0], q[1], 0.0));
                [r.x, r.y]
            };
            let way = |q: [f64; 2]| {
                let r = m.apply_vec(veymelo_geom::vec3(q[0], q[1], 0.0));
                [r.x, r.y]
            };
            let n = shape.points.len();
            let tangent = |list: &Vec<[f64; 2]>, i: usize| list.get(i).copied().unwrap_or([0.0; 2]);
            (shape.points.iter().map(|q| point(*q)).collect(), (0..n).map(|i| way(tangent(&shape.in_tangents, i))).collect(), (0..n).map(|i| way(tangent(&shape.out_tangents, i))).collect(), shape.closed)
        }
        (None, Some(Json::Array(list))) => {
            let pts: Vec<[f64; 2]> = list
                .iter()
                .enumerate()
                .map(|(i, v)| match v.as_array().map(|q| q.iter().filter_map(Json::as_f64).collect::<Vec<_>>()) {
                    Some(q) if q.len() == 2 => Ok([q[0], q[1]]),
                    _ => Err(Error::params(format!("point {} is [x, y], not {}", i + 1, v.to_json()))),
                })
                .collect::<Result<_>>()?;
            let n = pts.len();
            (pts, vec![[0.0; 2]; n], vec![[0.0; 2]; n], false)
        }
        _ => return Err(Error::params("give `path` (a property holding a path) or `points`")),
    };
    if points.len() < 2 {
        return Err(Error::params("the path needs two points at least"));
    }
    // round a closed path back to where it began
    if closed {
        points.push(points[0]);
        ins.push(ins[0]);
        outs.push(outs[0]);
    }
    if a.bool("reverse").unwrap_or(false) {
        points.reverse();
        ins.reverse();
        outs.reverse();
        std::mem::swap(&mut ins, &mut outs);
    }
    let (t0, t1) = (layer.layer_time(start), layer.layer_time(start + veymelo_time::Tick::from_seconds(duration)));
    let pos = layer.props.prop(&format!("{target}/position")).map_err(Error::not_found)?;
    // its keys outside the trip kept; one at each corner, the middle ones roving
    let mut keys: Vec<veymelo_keyframe::Keyframe> = pos.keys.iter().filter(|k| k.time < t0 || k.time > t1).cloned().collect();
    let n = points.len();
    for i in 0..n {
        let f = i as f64 / (n - 1) as f64;
        let mut k = veymelo_keyframe::Keyframe::new(t0 + veymelo_time::Tick::from_seconds((t1 - t0).seconds() * f), Value::Vec2(points[i]));
        k.spatial_auto = false;
        k.spatial_in = [ins[i][0], ins[i][1], 0.0];
        k.spatial_out = [outs[i][0], outs[i][1], 0.0];
        k.roving = i > 0 && i < n - 1;
        keys.push(k);
    }
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    store(layer, &format!("{target}/position"), keys)?;
    cx.label = Some("Pin Along a Path".into());
    Ok(Json::object([
        ("pin", Json::from(target)),
        ("keys", Json::Number(n as f64)),
        ("start", Json::Number(start.seconds())),
        ("end", Json::Number(start.seconds() + duration)),
    ]))
}

//! More for masks and paths: the points along a mask that soften its edge by different
//! amounts, a mask made a rectangle or an ellipse, a mask put back as it starts, and a path's
//! points moved, scaled and turned together (free transform).

use veymelo_json::Value as Json;
use veymelo_keyframe::{FeatherPoint, ShapePath, Value};
use veymelo_project::Source;

use super::paths::change_path;
use super::{COMP, Command, LAYER, Run, layer_ids};
use crate::{Args, Ctx, Error, Param, Result, Ty};

const MASK: Param = Param::need("mask", Ty::Text, "the mask, like \"Mask 1\"");

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "mask.feather",
            label: "Mask Feather Points",
            summary: "The points along a mask that soften its edge by different amounts: add one (on a piece, from point `piece` to the next, `at` along it from 0 to 1, softening `width` pixels outward or, below zero, inward; `tension` 0 goes over to the next smoothly, 1 straight), change one, remove one, or list them. On an animated mask, add and remove change every key alike; set changes the key at `time`.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    MASK,
                    Param::need("op", Ty::Choice(&["add", "set", "remove", "list"]), "what to do"),
                    Param::opt("index", Ty::Whole, "set, remove: which point (from 0, in the order they are listed)"),
                    Param::opt("piece", Ty::Whole, "add: the piece it is on (from 0)"),
                    Param::opt("at", Ty::Number, "add, set: where on the piece, 0 to 1 (default 0.5)"),
                    Param::opt("width", Ty::Number, "add, set: how far it softens, pixels (below zero: inward)"),
                    Param::opt("tension", Ty::Number, "add, set: 0 smooth to 1 straight (default 0)"),
                    Param::opt("time", Ty::Time, "on an animated mask, the key to change (default: the view's time)"),
                ]
            },
            returns: "{points: [{piece, at, width, tension?}]}",
            run: Run::Edit(feather),
        },
        Command {
            id: "mask.shape",
            label: "Mask Shape",
            summary: "Make a mask a rectangle or an ellipse in a box of the layer's pixels (its softening points go); on an animated mask, as a key at `time`.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    MASK,
                    Param::opt("rect", Ty::List, "[x, y, width, height]"),
                    Param::opt("ellipse", Ty::List, "[x, y, width, height] of the box round it"),
                    Param::opt("time", Ty::Time, "on an animated mask, when (default: the view's time)"),
                ]
            },
            returns: "{points, closed}",
            run: Run::Edit(shape),
        },
        Command {
            id: "mask.reset",
            label: "Reset Mask",
            summary: "Put a mask back as a new one starts: a rectangle as big as the layer (or the composition), adding, not inverted, no softening, wholly seen, not grown; its keys and expressions gone.",
            params: const { &[COMP, LAYER, MASK] },
            returns: "{path}",
            run: Run::Edit(reset),
        },
        Command {
            id: "path.transform",
            label: "Free Transform Points",
            summary: "Move, scale and turn a path's points together (all, or those `points` names) about the middle of them or `anchor`: a mask's path, or a shape layer's. On an animated path, the key at `time` (made there if there is none).",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::need("path", Ty::Text, "the path property, like masks/Mask 1/path or contents/Shape 1/contents/Path 1/path"),
                    Param::opt("scale", Ty::Any, "% to scale by: one number, or [across, down]"),
                    Param::opt("rotation", Ty::Number, "degrees to turn by (clockwise)"),
                    Param::opt("offset", Ty::List, "[x, y] to move by"),
                    Param::opt("anchor", Ty::List, "[x, y] to scale and turn about (default: the middle of the points)"),
                    Param::opt("points", Ty::List, "only these points (from 0)"),
                    Param::opt("time", Ty::Time, "on an animated path, when (default: the view's time)"),
                ]
            },
            returns: "{points, closed}",
            run: Run::Edit(transform),
        },
    ]
}

fn pair(j: Option<&Json>) -> Option<[f64; 2]> {
    let a = j?.as_array()?;
    (a.len() == 2).then(|| Some([a[0].as_f64()?, a[1].as_f64()?])).flatten()
}

fn feather_json(p: &ShapePath) -> Json {
    Json::object([("points", Json::array(p.feather.iter().map(FeatherPoint::to_json)))])
}

fn feather(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let view = cx.view.time;
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let mask = a.text("mask").unwrap_or("").trim_matches('/').trim_start_matches("masks/").to_string();
    let lt = layer.layer_time(a.time("time").unwrap_or(view));
    let prop = layer.props.prop_mut(&format!("masks/{mask}/path")).map_err(Error::not_found)?;
    let op = a.text("op").unwrap_or("list");
    if op == "list" {
        let Value::Path(p) = prop.value_at(lt) else { return Err(Error::params("that mask has no path")) };
        return Ok(feather_json(&p));
    }
    let number = |key: &str, lo: f64, hi: f64| -> Result<Option<f64>> {
        match a.num(key) {
            Some(v) if (lo..=hi).contains(&v) => Ok(Some(v)),
            Some(v) => Err(Error::params(format!("`{key}` is from {lo} to {hi}, not {v}"))),
            None => Ok(None),
        }
    };
    let (at, width, tension) = (number("at", 0.0, 1.0)?, number("width", -10_000.0, 10_000.0)?, number("tension", 0.0, 1.0)?);
    let index = a.whole("index").map(|i| i as usize);
    let change = |p: &ShapePath| -> Result<ShapePath> {
        let mut q = p.clone();
        match op {
            "add" => {
                let piece = a.whole("piece").map(|i| i as usize).ok_or_else(|| Error::params("give `piece`, the piece it goes on (from 0)"))?;
                if piece >= q.pieces() {
                    return Err(Error::params(format!("there is no piece {piece} (the mask has {})", q.pieces())));
                }
                let width = width.ok_or_else(|| Error::params("give `width`, how far it softens (pixels; below zero: inward)"))?;
                q.feather.push(FeatherPoint { piece, at: at.unwrap_or(0.5), width, tension: tension.unwrap_or(0.0) });
            }
            _ => {
                let n = q.feather.len();
                let i = index.filter(|i| *i < n).ok_or_else(|| Error::params(if n == 0 { "the mask has no softening points".to_string() } else { format!("give `index`, from 0 to {}", n - 1) }))?;
                if op == "remove" {
                    q.feather.remove(i);
                } else {
                    let f = &mut q.feather[i];
                    f.at = at.unwrap_or(f.at);
                    f.width = width.unwrap_or(f.width);
                    f.tension = tension.unwrap_or(f.tension);
                }
            }
        }
        Ok(q)
    };
    let q = change_path(prop, lt, op != "set", &change)?;
    cx.label = Some(format!("Mask Feather Points of {mask}"));
    Ok(feather_json(&q))
}

/// A rectangle or an ellipse in a box.
fn boxed(a: &Args) -> Result<ShapePath> {
    match (a.json("rect"), a.json("ellipse")) {
        (Some(r), None) => {
            let [x, y, w, h] = super::shape::bounds(r, "rect")?;
            Ok(ShapePath::polygon(&[[x, y], [x + w, y], [x + w, y + h], [x, y + h]], true))
        }
        (None, Some(e)) => {
            let [x, y, w, h] = super::shape::bounds(e, "ellipse")?;
            let s = veymelo_path::shape::ellipse([w, h], [x + w / 2.0, y + h / 2.0]);
            Ok(ShapePath { points: s.v.iter().map(|v| v.p).collect(), in_tangents: s.v.iter().map(|v| v.i).collect(), out_tangents: s.v.iter().map(|v| v.o).collect(), closed: true, feather: veymelo_keyframe::Feathers::NONE })
        }
        _ => Err(Error::params("give `rect` or `ellipse`, [x, y, width, height]")),
    }
}

fn points_json(p: &ShapePath) -> Json {
    Json::object([("points", Json::array(p.points.iter().map(|q| Json::from(*q)))), ("closed", Json::Bool(p.closed))])
}

fn shape(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let view = cx.view.time;
    let path = boxed(a)?;
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let mask = a.text("mask").unwrap_or("").trim_matches('/').trim_start_matches("masks/").to_string();
    let lt = layer.layer_time(a.time("time").unwrap_or(view));
    let prop = layer.props.prop_mut(&format!("masks/{mask}/path")).map_err(Error::not_found)?;
    let q = change_path(prop, lt, false, &|_| Ok(path.clone()))?;
    cx.label = Some(format!("Mask Shape of {mask}"));
    Ok(points_json(&q))
}

fn reset(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let (cw, ch) = cx.project.comp(cid).map(|c| (c.width as f64, c.height as f64)).expect("found");
    let size = {
        let layer = cx.project.comp(cid).expect("found").layer(lid).expect("found");
        let s = super::layer::natural_size(cx.project, layer);
        if s.0 > 0.0 && s.1 > 0.0 && !matches!(layer.source, Source::Null) { s } else { (cw, ch) }
    };
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let mask = a.text("mask").unwrap_or("").trim_matches('/').trim_start_matches("masks/").to_string();
    let g = layer.props.group_mut(&format!("masks/{mask}")).map_err(Error::not_found)?;
    let (w, h) = size;
    let starts: [(&str, Value); 9] = [
        ("path", Value::Path(ShapePath::polygon(&[[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]], true))),
        ("mode", Value::Choice(1)),
        ("inverted", Value::Bool(false)),
        ("feather", Value::Vec2([0.0; 2])),
        ("opacity", Value::Number(100.0)),
        ("expansion", Value::Number(0.0)),
        ("featherFalloff", Value::Choice(0)),
        ("rotoBezier", Value::Bool(false)),
        ("motionBlur", Value::Choice(0)),
    ];
    for (key, v) in starts {
        let p = g.prop_mut(key).map_err(Error::not_found)?;
        p.expression = None;
        p.set_value(v).map_err(Error::params)?;
    }
    cx.label = Some(format!("Reset {mask}"));
    Ok(Json::object([("path", Json::from(format!("masks/{mask}")))]))
}

fn transform(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let view = cx.view.time;
    let scale = match a.json("scale") {
        None => [1.0, 1.0],
        Some(Json::Number(k)) => [*k / 100.0, *k / 100.0],
        Some(j) => pair(Some(j)).map(|s| [s[0] / 100.0, s[1] / 100.0]).ok_or_else(|| Error::params("`scale` is a % or [across %, down %]"))?,
    };
    let turn = a.num("rotation").unwrap_or(0.0).to_radians();
    let offset = match a.json("offset") {
        None => [0.0; 2],
        j => pair(j).ok_or_else(|| Error::params("`offset` is [x, y]"))?,
    };
    if scale == [1.0, 1.0] && turn == 0.0 && offset == [0.0; 2] {
        return Err(Error::params("give `scale`, `rotation` or `offset`"));
    }
    let anchor = match a.json("anchor") {
        None => None,
        j => Some(pair(j).ok_or_else(|| Error::params("`anchor` is [x, y]"))?),
    };
    let only: Option<Vec<usize>> = match a.json("points") {
        None => None,
        Some(j) => Some(j.as_array().map(|v| v.iter().filter_map(|x| x.as_f64().filter(|x| *x >= 0.0 && x.fract() == 0.0).map(|x| x as usize)).collect()).ok_or_else(|| Error::params("`points` is a list of point numbers from 0"))?),
    };
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let path = a.text("path").unwrap_or("").to_string();
    let lt = layer.layer_time(a.time("time").unwrap_or(view));
    let prop = layer.props.prop_mut(&path).map_err(Error::not_found)?;
    let (c, s) = (turn.cos(), turn.sin());
    let shape = |v: [f64; 2]| {
        let (x, y) = (v[0] * scale[0], v[1] * scale[1]);
        [x * c - y * s, x * s + y * c]
    };
    let change = |p: &ShapePath| -> Result<ShapePath> {
        let n = p.points.len();
        let which: Vec<usize> = match &only {
            Some(list) => {
                if let Some(bad) = list.iter().find(|i| **i >= n) {
                    return Err(Error::params(format!("there is no point {bad} (the path has {n})")));
                }
                list.clone()
            }
            None => (0..n).collect(),
        };
        if which.is_empty() {
            return Err(Error::params("no points to move"));
        }
        let middle = anchor.unwrap_or_else(|| {
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for &i in &which {
                let v = p.points[i];
                lo = [lo[0].min(v[0]), lo[1].min(v[1])];
                hi = [hi[0].max(v[0]), hi[1].max(v[1])];
            }
            [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]
        });
        let mut q = p.clone();
        for &i in &which {
            let v = shape([p.points[i][0] - middle[0], p.points[i][1] - middle[1]]);
            q.points[i] = [v[0] + middle[0] + offset[0], v[1] + middle[1] + offset[1]];
            if let (Some(t), Some(o)) = (q.in_tangents.get_mut(i), p.in_tangents.get(i)) {
                *t = shape(*o);
            }
            if let (Some(t), Some(o)) = (q.out_tangents.get_mut(i), p.out_tangents.get(i)) {
                *t = shape(*o);
            }
        }
        Ok(q)
    };
    let q = change_path(prop, lt, false, &change)?;
    cx.label = Some("Free Transform Points".into());
    Ok(points_json(&q))
}

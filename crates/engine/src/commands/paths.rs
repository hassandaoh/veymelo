//! Commands for paths — masks' and shapes' alike: their vertices edited one by one, the shapes
//! between two mask keys made (smart mask interpolation), rectangles, ellipses and stars turned
//! into paths, shape items grouped and ungrouped, nulls that follow a path's points (or that
//! its points follow), a null that travels along a path, and outlines traced from a layer's
//! pixels (auto-trace).

use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::{Keyframe, ShapePath, Value};
use veymelo_path::bezier;
use veymelo_path::morph::{self, Morph};
use veymelo_project::{Group, Layer, MASK, Node, Source};
use veymelo_time::Tick;

use super::{COMP, Command, LAYER, Run, comp_id, layer_ids};
use crate::{Args, Ctx, Error, Param, Result, Ty};

const PATH: Param = Param::need("path", Ty::Text, "the path property, like masks/Mask 1/path or contents/Shape 1/contents/Path 1/path");

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "path.vertex",
            label: "Edit Path Vertices",
            summary: "Edit a path's vertices: set one's point and tangents, insert one part way along a piece (the outline kept), remove one, make one a corner or smooth, make one the first, close or open the path, or turn it round. On an animated path, insert, remove, first and reverse change every key alike (so they keep as many vertices as each other); set, corner and smooth change the key at `time`.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    PATH,
                    Param::need("op", Ty::Choice(&["set", "insert", "remove", "corner", "smooth", "first", "close", "open", "reverse"]), "what to do"),
                    Param::opt("index", Ty::Whole, "the vertex (from 0); for insert, the piece it goes into (from vertex `index` to the next)"),
                    Param::opt("point", Ty::List, "set: [x, y]"),
                    Param::opt("in", Ty::List, "set: the tangent in, [x, y] from the point"),
                    Param::opt("out", Ty::List, "set: the tangent out"),
                    Param::opt("at", Ty::Number, "insert: how far along the piece, 0 to 1 by length (default 0.5)"),
                    Param::opt("time", Ty::Time, "on an animated path, the key to change (default: the view's time)"),
                ]
            },
            returns: "{points, closed}",
            run: Run::Edit(vertex),
        },
        Command {
            id: "mask.interpolate",
            label: "Smart Mask Interpolation",
            summary: "Make the shapes between a path's keys: each two keys given the same vertices matched vertex to vertex, and keys between them every 1 / rate seconds, the shape turning and growing as a whole (bending resistance) or going straight.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    PATH,
                    Param::opt("times", Ty::List, "the keys to make shapes between (default: all of them)"),
                    Param::opt("rate", Ty::Number, "keys a second between (default: the composition's frame rate)"),
                    Param::opt("linear", Ty::Bool, "vertices go in straight lines (default false)"),
                    Param::opt("bendingResistance", Ty::Number, "0 to 100 (default 50)"),
                    Param::opt("spacing", Ty::Number, "pixels between vertices once added (default 15; 0: only as many as the larger has)"),
                    Param::opt("firstVerticesMatch", Ty::Bool, "the two first vertices match (default false)"),
                    Param::opt("oneToOne", Ty::Bool, "shapes with as many vertices are matched as they are (default false)"),
                ]
            },
            returns: "{keys, vertices}",
            run: Run::Edit(interpolate),
        },
        Command {
            id: "shape.toPath",
            label: "Convert to Bezier Path",
            summary: "Turn a rectangle, ellipse or star shape item into a path item of the same outline (as it is at `time`), in its place.",
            params: const { &[COMP, LAYER, Param::need("item", Ty::Text, "the item, like \"Rectangle 1\" or \"Group 1/contents/Ellipse 1\""), Param::opt("time", Ty::Time, "when (default: the view's time)")] },
            returns: "{path, points}",
            run: Run::Edit(to_path),
        },
        Command {
            id: "shape.group",
            label: "Group Shapes",
            summary: "Put shape items (of one contents list) into a new group in the first one's place, in their order.",
            params: const { &[COMP, LAYER, Param::need("items", Ty::List, "the items, like [\"Rectangle 1\", \"Fill 1\"] or paths inside groups"), Param::opt("name", Ty::Text, "the group's name (default Group n)")] },
            returns: "{group}",
            run: Run::Edit(group),
        },
        Command {
            id: "shape.ungroup",
            label: "Ungroup Shapes",
            summary: "Take a group's items out into the list it is in, in its place (its transform must be as it starts, unless `force`, which leaves it out).",
            params: const { &[COMP, LAYER, Param::need("item", Ty::Text, "the group"), Param::opt("force", Ty::Bool, "ungroup even though the group's transform is changed")] },
            returns: "{items}",
            run: Run::Edit(ungroup),
        },
        Command {
            id: "path.nulls",
            label: "Create Nulls from Paths",
            summary: "Nulls for a mask's or shape's path: nullsFollowPoints (a null on each vertex, following it), pointsFollowNulls (a null on each vertex that the vertex follows: move the null, the path changes), or trace (one null travelling along the path, as its Progress slider says, turned along it).",
            params: const { &[COMP, LAYER, PATH, Param::need("mode", Ty::Choice(&["nullsFollowPoints", "pointsFollowNulls", "trace"]), "which")] },
            returns: "{nulls}",
            run: Run::Edit(nulls),
        },
        Command {
            id: "layer.autoTrace",
            label: "Auto-trace",
            summary: "Masks from a layer's pixels: the outlines of where its alpha (or a colour, or its brightness) passes a threshold, at a time or a key a frame over the work area. Holes become subtracting masks.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::opt("channel", Ty::Choice(&["alpha", "red", "green", "blue", "luminance"]), "default alpha"),
                    Param::opt("invert", Ty::Bool, "trace where it is below the threshold"),
                    Param::opt("threshold", Ty::Number, "0 to 100 % (default 50)"),
                    Param::opt("blur", Ty::Number, "pixels to soften the pixels first (default 1)"),
                    Param::opt("tolerance", Ty::Number, "pixels the outline may stray (default 1)"),
                    Param::opt("minimumArea", Ty::Number, "square pixels an outline must enclose (default 10)"),
                    Param::opt("cornerRoundness", Ty::Number, "0 to 100 % (default 50)"),
                    Param::opt("workArea", Ty::Bool, "a key a frame over the work area (default: only at `time`)"),
                    Param::opt("time", Ty::Time, "when (default: the view's time)"),
                    Param::opt("newLayer", Ty::Bool, "put the masks on a new solid instead"),
                ]
            },
            returns: "{masks, layer}",
            run: Run::Edit(auto_trace),
        },
    ]
}

fn pair(j: Option<&Json>) -> Option<[f64; 2]> {
    let v: Vec<f64> = j?.as_array()?.iter().filter_map(Json::as_f64).collect();
    (v.len() >= 2).then(|| [v[0], v[1]])
}

fn paths_json(p: &ShapePath) -> Json {
    Json::object([("points", Json::from(p.points.len() as f64)), ("closed", Json::from(p.closed))])
}

/// A copy of a path with vertex `i`'s tangents from its neighbours (a sixth of the way from the
/// one before to the one after).
fn smoothed_at(p: &ShapePath, i: usize) -> ShapePath {
    let mut q = p.clone();
    let n = p.points.len();
    if n < 2 {
        return q;
    }
    let prev = if i > 0 { p.points[i - 1] } else if p.closed { p.points[n - 1] } else { p.points[i] };
    let next = if i + 1 < n { p.points[i + 1] } else if p.closed { p.points[0] } else { p.points[i] };
    let d = bezier::mul(bezier::sub(next, prev), 1.0 / 6.0);
    q.out_tangents[i] = d;
    q.in_tangents[i] = bezier::mul(d, -1.0);
    q
}

fn inserted(p: &ShapePath, piece: usize, at: f64) -> Result<ShapePath> {
    if piece >= bezier::pieces(p) {
        return Err(Error::params(format!("there is no piece {piece} (the path has {})", bezier::pieces(p))));
    }
    let c = bezier::piece(p, piece);
    // the parameter that far along by length
    let (mut acc, mut last, total) = (0.0, c[0], bezier::piece_length(&c));
    let want = at.clamp(0.0, 1.0) * total;
    let mut t = 0.5;
    for k in 1..=256 {
        let q = bezier::at(&c, k as f64 / 256.0);
        let step = bezier::dist(last, q);
        if acc + step >= want {
            t = ((k - 1) as f64 + if step > 0.0 { (want - acc) / step } else { 0.0 }) / 256.0;
            break;
        }
        acc += step;
        last = q;
    }
    let (a, b) = bezier::split(&c, t);
    let n = p.points.len();
    let j = (piece + 1) % n;
    let mut q = p.clone();
    q.out_tangents[piece] = bezier::sub(a[1], a[0]);
    q.in_tangents[j] = bezier::sub(b[2], b[3]);
    q.points.insert(piece + 1, a[3]);
    q.in_tangents.insert(piece + 1, bezier::sub(a[2], a[3]));
    q.out_tangents.insert(piece + 1, bezier::sub(b[1], b[0]));
    q.feather_after_insert(piece + 1, Some(t));
    Ok(q)
}

fn vertex(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let view = cx.view.time;
    let (cid, id) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(id).expect("found");
    let path = a.text("path").unwrap_or("").to_string();
    let lt = layer.layer_time(a.time("time").unwrap_or(view));
    let prop = layer.props.prop_mut(&path).map_err(Error::not_found)?;
    if !matches!(prop.value, Value::Path(_)) {
        return Err(Error::params(format!("{path} is not a path")));
    }
    let op = a.text("op").unwrap_or("set");
    let index = a.whole("index").map(|i| i as usize);
    let need = |n: usize| index.filter(|i| *i < n).ok_or_else(|| Error::params(format!("give `index`, a vertex from 0 to {}", n.saturating_sub(1))));
    let change = |p: &ShapePath| -> Result<ShapePath> {
        let n = p.points.len();
        Ok(match op {
            "set" => {
                let i = need(n)?;
                let mut q = p.clone();
                if let Some(pt) = pair(a.json("point")) {
                    q.points[i] = pt;
                }
                if let Some(t) = pair(a.json("in")) {
                    q.in_tangents[i] = t;
                }
                if let Some(t) = pair(a.json("out")) {
                    q.out_tangents[i] = t;
                }
                q
            }
            "insert" => inserted(p, need(bezier::pieces(p).max(1))?, a.num("at").unwrap_or(0.5))?,
            "remove" => {
                let i = need(n)?;
                if n <= 2 {
                    return Err(Error::refused("a path keeps two vertices at least"));
                }
                let mut q = p.clone();
                q.points.remove(i);
                q.in_tangents.remove(i);
                q.out_tangents.remove(i);
                q.feather_after_remove(i, p.pieces());
                q
            }
            "corner" => {
                let i = need(n)?;
                let mut q = p.clone();
                (q.in_tangents[i], q.out_tangents[i]) = ([0.0; 2], [0.0; 2]);
                q
            }
            "smooth" => smoothed_at(p, need(n)?),
            "first" => {
                if !p.closed {
                    return Err(Error::refused("only a closed path can start at another vertex"));
                }
                bezier::starting_at(p, need(n)?)
            }
            "close" => ShapePath { closed: true, ..p.clone() },
            "open" => {
                // the piece back to the first point goes, and the softening on it
                let mut q = ShapePath { closed: false, ..p.clone() };
                let pieces = q.pieces();
                q.feather.retain(|f| f.piece < pieces);
                q
            }
            _ => bezier::reversed(p),
        })
    };
    let every_key = matches!(op, "insert" | "remove" | "first" | "reverse" | "close" | "open");
    let q = change_path(prop, lt, every_key, &change)?;
    cx.label = Some("Edit Path".into());
    Ok(paths_json(&q))
}

/// A path property changed: its one value, or, with keys, every key (`every_key`: what keeps
/// the keys alike, as how many points they have) or the key at layer time `lt` (made there from
/// what the path is then, if there is none). The path as it is at `lt` after.
pub(super) fn change_path(prop: &mut veymelo_project::Property, lt: Tick, every_key: bool, change: &dyn Fn(&ShapePath) -> Result<ShapePath>) -> Result<ShapePath> {
    if prop.keys.is_empty() {
        let Value::Path(p) = &prop.value else { return Err(Error::params("that is not a path")) };
        let q = change(p)?;
        prop.set_value(Value::Path(q.clone())).map_err(Error::params)?;
        return Ok(q);
    }
    let mut keys = prop.keys.clone();
    let half = Tick::from_seconds(0.5 / 120.0);
    let mut here = None;
    for k in keys.iter_mut() {
        let now = (k.time - lt).abs() <= half;
        if !every_key && !now {
            continue;
        }
        if let Value::Path(p) = &k.value {
            let q = change(p)?;
            if now {
                here = Some(q.clone());
            }
            k.value = Value::Path(q);
        }
    }
    if here.is_none() && !every_key {
        let Value::Path(p) = prop.value_at(lt) else { return Err(Error::params("that is not a path")) };
        let q = change(&p)?;
        veymelo_keyframe::set_key(&mut keys, Keyframe::new(lt, Value::Path(q.clone())));
        here = Some(q);
    }
    prop.set_keys(keys).map_err(Error::params)?;
    match here {
        Some(q) => Ok(q),
        None => match prop.value_at(lt) {
            Value::Path(p) => Ok(p),
            _ => Err(Error::params("that is not a path")),
        },
    }
}

fn interpolate(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let fps = cx.project.comp(cid).expect("found").frame_rate.as_f64();
    let rate = a.num("rate").unwrap_or(fps).clamp(0.1, 1000.0);
    let m = Morph {
        linear: a.bool("linear").unwrap_or(false),
        bending_resistance: a.num("bendingResistance").unwrap_or(50.0),
        spacing: Some(a.num("spacing").unwrap_or(15.0)).filter(|s| *s > 0.0),
        first_vertices_match: a.bool("firstVerticesMatch").unwrap_or(false),
        one_to_one: a.bool("oneToOne").unwrap_or(false),
    };
    let (layer, path, near) = super::key::target(cx, a)?;
    let keys = layer.props.prop(&path).map_err(Error::not_found)?.keys.clone();
    if !keys.iter().all(|k| matches!(k.value, Value::Path(_))) {
        return Err(Error::params(format!("{path} is not a path")));
    }
    let idx = super::key::chosen(layer, &keys, a, near)?;
    if idx.len() < 2 {
        return Err(Error::params("mask interpolation needs two keys or more"));
    }
    let mut out: Vec<Keyframe> = keys.iter().enumerate().filter(|(i, _)| !idx.contains(i)).map(|(_, k)| k.clone()).collect();
    let (mut made, mut vertices) = (0usize, 0usize);
    for w in idx.windows(2) {
        let (ka, kb) = (&keys[w[0]], &keys[w[1]]);
        let (Value::Path(pa), Value::Path(pb)) = (&ka.value, &kb.value) else { continue };
        let (aa, bb) = morph::correspond(pa, pb, &m);
        vertices = aa.points.len();
        let mut first = ka.clone();
        first.value = Value::Path(aa.clone());
        out.retain(|k| k.time != first.time);
        out.push(first);
        let span = (kb.time - ka.time).seconds();
        let n = (span * rate).floor() as usize;
        for s in 1..n {
            let t = s as f64 / rate;
            if t >= span - 1e-9 {
                break;
            }
            let mut k = Keyframe::new(ka.time + Tick::from_seconds(t), Value::Path(morph::between(&aa, &bb, t / span, &m)));
            k.interp_in = veymelo_keyframe::Interp::Linear;
            out.push(k);
            made += 1;
        }
        let mut last = kb.clone();
        last.value = Value::Path(bb);
        out.retain(|k| k.time != last.time);
        out.push(last);
    }
    out.sort_by_key(|k| k.time);
    super::key::store(layer, &path, out)?;
    cx.label = Some("Smart Mask Interpolation".into());
    Ok(Json::object([("keys", Json::from(made as f64)), ("vertices", Json::from(vertices as f64))]))
}

fn sub_to_path(s: &veymelo_path::Sub) -> ShapePath {
    ShapePath { points: s.v.iter().map(|v| v.p).collect(), in_tangents: s.v.iter().map(|v| v.i).collect(), out_tangents: s.v.iter().map(|v| v.o).collect(), closed: s.closed, feather: veymelo_keyframe::Feathers::NONE }
}

/// The contents list `path` is in, and the item's place in it.
fn place(layer: &mut Layer, item: &str) -> Result<(String, usize)> {
    let full = format!("contents/{}", item.trim_matches('/'));
    let (list, _) = full.rsplit_once('/').unwrap_or(("contents", ""));
    let target = layer.props.node(&full).map_err(Error::not_found)?;
    let id = target.as_group().map(|g| g.id).ok_or_else(|| Error::params(format!("{item} is not a shape item")))?;
    let g = layer.props.group(list).map_err(Error::not_found)?;
    let at = g.children.iter().position(|c| c.as_group().is_some_and(|x| x.id == id)).ok_or_else(|| Error::not_found(format!("{item} is not in {list}")))?;
    Ok((list.to_string(), at))
}

fn to_path(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let view = cx.view.time;
    let t = a.time("time").unwrap_or(view);
    let id = cx.project.alloc();
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let project = cx.project.clone();
    let comp = project.comp(cid).expect("found");
    let layer_ro = comp.layer(lid).expect("found");
    let eval = veymelo_expr::Eval::new(&project);
    let at = veymelo_expr::Place { layer: layer_ro, comp, comp_id: cid };
    let item = a.text("item").unwrap_or("");
    let full = format!("contents/{}", item.trim_matches('/'));
    let g = layer_ro.props.group(&full).map_err(Error::not_found)?;
    let v = |k: &str| g.prop(k).map(|p| eval.value(p, at, t)).map_err(Error::not_found);
    let sub = match g.def.id {
        "rect" => veymelo_path::shape::rect(v("size")?.as_vec2(), v("position")?.as_vec2(), v("roundness")?.as_f64()),
        "ellipse" => veymelo_path::shape::ellipse(v("size")?.as_vec2(), v("position")?.as_vec2()),
        "star" => veymelo_path::shape::polystar(&veymelo_path::shape::Polystar {
            star: v("type")?.as_choice() == 0,
            points: v("points")?.as_f64(),
            centre: v("position")?.as_vec2(),
            rotation: v("rotation")?.as_f64(),
            inner_radius: v("innerRadius")?.as_f64(),
            outer_radius: v("outerRadius")?.as_f64(),
            inner_roundness: v("innerRoundness")?.as_f64(),
            outer_roundness: v("outerRoundness")?.as_f64(),
        }),
        other => return Err(Error::params(format!("{item} is a {other}; only rectangles, ellipses and stars turn into paths"))),
    };
    let reversed = g.prop("direction").is_ok_and(|p| p.value.as_choice() == 1);
    let shape = sub_to_path(&if reversed { sub.reversed() } else { sub });
    let name = g.name().replace("Rectangle", "Path").replace("Ellipse", "Path").replace("Polystar", "Path");
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let (list, idx) = place(layer, item)?;
    let mut new = Group::instance(&veymelo_project::shapes::PATH, id, &name);
    new.prop_mut("path").map_err(Error::not_found)?.set_value(Value::Path(shape.clone())).map_err(Error::params)?;
    layer.props.group_mut(&list).map_err(Error::not_found)?.children[idx] = Node::Group(new);
    cx.label = Some("Convert To Bezier Path".into());
    Ok(Json::object([("path", Json::from(format!("{}/{name}", list.trim_start_matches("contents").trim_start_matches('/')).trim_start_matches('/'))), ("points", Json::from(shape.points.len() as f64))]))
}

fn group(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let id = cx.project.alloc();
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let items: Vec<String> = a.json("items").and_then(Json::as_array).cloned().unwrap_or_default().iter().filter_map(|j| j.as_str().map(str::to_string)).collect();
    if items.is_empty() {
        return Err(Error::params("give the items to group"));
    }
    let mut places = Vec::new();
    for it in &items {
        places.push(place(layer, it)?);
    }
    let list = places[0].0.clone();
    if places.iter().any(|(l, _)| *l != list) {
        return Err(Error::params("the items to group are in one list"));
    }
    let mut at: Vec<usize> = places.iter().map(|(_, i)| *i).collect();
    at.sort_unstable();
    at.dedup();
    let g = layer.props.group_mut(&list).map_err(Error::not_found)?;
    let taken: Vec<Node> = at.iter().map(|i| g.children[*i].clone()).collect();
    let existing: Vec<String> = g.children.iter().map(|c| c.name().to_string()).collect();
    let name = a.text("name").map_or_else(|| (1..).map(|n| format!("Group {n}")).find(|n| !existing.contains(n)).unwrap_or_default(), str::to_string);
    let mut new = Group::instance(&veymelo_project::shapes::GROUP, id, &name);
    new.group_mut("contents").map_err(Error::not_found)?.children = taken;
    for i in at.iter().rev() {
        g.children.remove(*i);
    }
    g.children.insert(at[0], Node::Group(new));
    cx.label = Some("Group Shapes".into());
    Ok(Json::object([("group", Json::from(name))]))
}

fn ungroup(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let item = a.text("item").unwrap_or("");
    let (list, idx) = place(layer, item)?;
    let g = layer.props.group(&format!("contents/{}", item.trim_matches('/'))).map_err(Error::not_found)?.clone();
    if g.def.id != "group" {
        return Err(Error::params(format!("{item} is not a group")));
    }
    let untouched = g.group("transform").is_ok_and(|t| {
        let mut plain = true;
        t.walk(&mut |_, p| plain &= p.is_default());
        plain
    });
    if !untouched && !a.bool("force").unwrap_or(false) {
        return Err(Error::refused("its transform is changed and would be lost; force: true ungroups all the same"));
    }
    let inner = g.group("contents").map(|c| c.children.clone()).unwrap_or_default();
    let names: Vec<Json> = inner.iter().map(|n| Json::from(n.name())).collect();
    let parent = layer.props.group_mut(&list).map_err(Error::not_found)?;
    parent.children.remove(idx);
    for (k, n) in inner.into_iter().enumerate() {
        parent.children.insert(idx + k, n);
    }
    cx.label = Some("Ungroup Shapes".into());
    Ok(Json::object([("items", Json::Array(names))]))
}

/// How an expression reaches a path property of layer `l` (`mask("Mask 1").path`,
/// `content("Shape 1").content("Path 1").path`), and whether groups with transforms lie on the
/// way (their points are then not in the layer's own pixels).
fn reach(l: &Layer, path: &str) -> Result<(String, bool)> {
    let segs: Vec<&str> = path.trim_matches('/').split('/').collect();
    let mut out = String::new();
    let mut moved = false;
    let mut g: &Group = &l.props;
    let mut i = 0;
    while i + 1 < segs.len() {
        let (list, item) = (segs[i], segs[i + 1]);
        let lg = g.group(list).map_err(Error::not_found)?;
        if i + 2 == segs.len() {
            break;
        }
        let ig = lg.group(item).map_err(Error::not_found)?;
        let call = match list {
            "masks" => "mask",
            "contents" => "content",
            "effects" => "effect",
            other => return Err(Error::params(format!("{other} cannot be followed by a null"))),
        };
        out.push_str(&format!(".{call}(\"{}\")", ig.name().replace('"', "\\\"")));
        if ig.def.id == "group" && ig.group("transform").is_ok_and(|t| {
            let mut changed = false;
            t.walk(&mut |_, p| changed |= !p.is_default());
            changed
        }) {
            moved = true;
        }
        g = ig;
        i += 2;
    }
    let last = segs.last().copied().unwrap_or("path");
    out.push_str(&format!(".{last}"));
    Ok((out, moved))
}

fn nulls(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let view = cx.view.time;
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let layer = comp.layer(lid).expect("found").clone();
    let path = a.text("path").unwrap_or("").to_string();
    let prop = layer.props.prop(&path).map_err(Error::not_found)?;
    let Value::Path(shape) = prop.value_at(layer.layer_time(view)) else { return Err(Error::params(format!("{path} is not a path"))) };
    let (access, moved) = reach(&layer, &path)?;
    if moved {
        return Err(Error::refused("the path is in a group whose transform is changed; its points are not in the layer's own pixels"));
    }
    let lname = layer.name.replace('"', "\\\"");
    let target = format!("thisComp.layer(\"{lname}\")");
    let duration = comp.duration;
    // where a vertex is in the composition now (for placing new nulls)
    let project = cx.project.clone();
    let eval = veymelo_expr::Eval::new(&project);
    let to_comp = |p: [f64; 2]| -> [f64; 3] {
        let at = eval.place(project.comp(cid).expect("found"), project.comp(cid).expect("found").layer(lid).expect("found"));
        let q = eval.world(at, view, None).apply(veymelo_geom::vec3(p[0], p[1], 0.0));
        [q.x, q.y, q.z]
    };
    let mode = a.text("mode").unwrap_or("nullsFollowPoints");
    let mut made = Vec::new();
    let mut new_layers = Vec::new();
    let mut add_null = |cx: &mut Ctx, name: String, pos: [f64; 3], expr: Option<String>| -> Result<u64> {
        let mut n = Layer::new(cx.project.alloc(), &name, Source::Null, duration);
        n.props.prop_mut("transform/position").map_err(Error::not_found)?.set_value(Value::Vec3(pos)).map_err(Error::params)?;
        if let Some(e) = expr {
            n.props.prop_mut("transform/position").map_err(Error::not_found)?.expression = Some(veymelo_project::Expression { text: e, enabled: true });
        }
        let id = n.id;
        new_layers.push(n);
        Ok(id)
    };
    match mode {
        "nullsFollowPoints" => {
            for (i, p) in shape.points.iter().enumerate() {
                let e = format!("var L = {target}; L.toComp(L{access}.points()[{i}])");
                made.push(add_null(cx, format!("{} Point {}", layer.name, i + 1), to_comp(*p), Some(e))?);
            }
        }
        "pointsFollowNulls" => {
            let mut refs = Vec::new();
            for (i, p) in shape.points.iter().enumerate() {
                let name = format!("{} Point {}", layer.name, i + 1);
                made.push(add_null(cx, name.clone(), to_comp(*p), None)?);
                refs.push(format!("thisLayer.fromComp(thisComp.layer(\"{}\").toComp([0, 0, 0]))", name.replace('"', "\\\"")));
            }
            let e = format!("createPath([{}], inTangents(), outTangents(), isClosed())", refs.join(", "));
            let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
            layer.props.prop_mut(&path).map_err(Error::not_found)?.expression = Some(veymelo_project::Expression { text: e, enabled: true });
        }
        _ => {
            let e = format!("var L = {target}; L.toComp(L{access}.pointOnPath(effect(\"Progress\")(\"Slider\") / 100))");
            let id = add_null(cx, format!("Trace {}", layer.name), to_comp(shape.points.first().copied().unwrap_or([0.0; 2])), Some(e))?;
            made.push(id);
            let n = new_layers.last_mut().expect("just made");
            let mut slider = Group::instance(veymelo_effects::find("sliderControl").expect("defined").def, cx.project.alloc(), "Progress");
            slider.prop_mut("slider").map_err(Error::not_found)?.set_keys(vec![Keyframe::new(Tick::ZERO, Value::Number(0.0)), Keyframe::new(duration, Value::Number(100.0))]).map_err(Error::params)?;
            n.props.group_mut("effects").map_err(Error::not_found)?.children.push(Node::Group(slider));
            let turn = format!("var L = {target}; var d = L{access}.tangentOnPath(effect(\"Progress\")(\"Slider\") / 100); radiansToDegrees(Math.atan2(d[1], d[0]))");
            n.props.prop_mut("transform/rotation").map_err(Error::not_found)?.expression = Some(veymelo_project::Expression { text: turn, enabled: true });
        }
    }
    let comp = cx.project.comp_mut(cid).expect("found");
    let at = comp.index_of(lid).unwrap_or(0);
    for (k, n) in new_layers.into_iter().enumerate() {
        comp.layers.insert(at + k, n);
    }
    cx.label = Some("Create Nulls from Paths".into());
    Ok(Json::object([("nulls", Json::array(made.iter().map(|i| Json::Number(*i as f64))))]))
}

fn auto_trace(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let view = cx.view.time;
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found").clone();
    let layer = comp.layer(lid).expect("found").clone();
    let times: Vec<Tick> = if a.bool("workArea").unwrap_or(false) {
        let (s, e) = (comp.frame_rate.frame_at(comp.work_area.start), comp.frame_rate.frame_at(comp.work_area.end()));
        (s..e).map(|f| comp.frame_rate.tick_of(f)).collect()
    } else {
        vec![a.time("time").unwrap_or(view)]
    };
    let channel = a.text("channel").unwrap_or("alpha");
    let invert = a.bool("invert").unwrap_or(false);
    let opts = veymelo_path::trace::Trace {
        threshold: (a.num("threshold").unwrap_or(50.0) / 100.0).clamp(0.0, 1.0) as f32,
        tolerance: a.num("tolerance").unwrap_or(1.0).max(0.01),
        min_area: a.num("minimumArea").unwrap_or(10.0).max(0.0),
        corner_roundness: (a.num("cornerRoundness").unwrap_or(50.0) / 100.0).clamp(0.0, 1.0),
    };
    let blur = a.num("blur").unwrap_or(1.0).max(0.0);
    // the layer alone, drawn in the composition, its outlines brought back into its own pixels
    let mut solo = cx.project.clone();
    if let Some(veymelo_project::ItemKind::Comp(c)) = solo.item_mut(cid).map(|i| &mut i.kind) {
        for l in &mut std::sync::Arc::make_mut(c).layers {
            l.switches.video = l.id == lid;
            if l.id == lid {
                // its own masks do not cut what it traces
                if let Ok(m) = l.props.group_mut("masks") {
                    m.children.clear();
                }
            }
        }
    }
    let mut per_time: Vec<(Tick, Vec<veymelo_path::trace::Traced>)> = Vec::new();
    for &t in &times {
        let r = veymelo_render::render(&solo, cid, t, &veymelo_render::Options { background: false, ..Default::default() }).map_err(Error::refused)?;
        let img = if blur > 0.0 { veymelo_raster::filter::gaussian(&r.image, blur, blur, veymelo_raster::filter::Edge::Clear) } else { r.image };
        let field: Vec<f32> = img
            .px
            .iter()
            .map(|p| {
                let a = p[3];
                let s = |v: f32| if a > 1e-6 { v / a } else { 0.0 };
                let v = match channel {
                    "red" => s(p[0]),
                    "green" => s(p[1]),
                    "blue" => s(p[2]),
                    "luminance" => 0.2126 * s(p[0]) + 0.7152 * s(p[1]) + 0.0722 * s(p[2]),
                    _ => a,
                }
                .clamp(0.0, 1.0);
                if invert { 1.0 - v } else { v }
            })
            .collect();
        per_time.push((t, veymelo_path::trace::trace(&field, img.width, img.height, &opts)));
    }
    let count = per_time.iter().map(|(_, o)| o.len()).max().unwrap_or(0);
    if count == 0 {
        return Err(Error::refused("nothing passes the threshold to trace"));
    }
    // comp pixels into the layer's own
    let eval = veymelo_expr::Eval::new(&solo);
    let solo_comp = solo.comp(cid).expect("found");
    let at = veymelo_expr::Place { layer: solo_comp.layer(lid).expect("found"), comp: solo_comp, comp_id: cid };
    let to_layer = |t: Tick| eval.world(at, t, None).inverse();
    let target = if a.bool("newLayer").unwrap_or(false) {
        let id = cx.project.alloc();
        let mut s = Layer::new(id, &format!("Auto-traced {}", layer.name), Source::Solid { color: veymelo_color::Rgba::WHITE, width: comp.width, height: comp.height }, comp.duration);
        s.props.prop_mut("transform/position").map_err(Error::not_found)?.set_value(Value::Vec3([comp.width as f64 / 2.0, comp.height as f64 / 2.0, 0.0])).map_err(Error::params)?;
        s.props.prop_mut("transform/anchor").map_err(Error::not_found)?.set_value(Value::Vec3([comp.width as f64 / 2.0, comp.height as f64 / 2.0, 0.0])).map_err(Error::params)?;
        let c = cx.project.comp_mut(cid).expect("found");
        let k = c.index_of(lid).unwrap_or(0);
        c.layers.insert(k, s);
        id
    } else {
        lid
    };
    let new_layer = target != lid;
    let mut masks = Vec::new();
    for m in 0..count {
        let mid = cx.project.alloc();
        let mut g = Group::instance(&MASK, mid, &format!("Auto-trace {}", m + 1));
        let mut keys = Vec::new();
        let mut hole = false;
        for (t, outlines) in &per_time {
            let Some(o) = outlines.get(m) else { continue };
            hole = o.depth % 2 == 1;
            // into the layer's pixels (a new solid's are the composition's)
            let map = if new_layer { None } else { to_layer(*t) };
            let tf = |p: [f64; 2]| match &map {
                Some(mx) => {
                    let q = mx.apply(veymelo_geom::vec3(p[0], p[1], 0.0));
                    [q.x, q.y]
                }
                None => p,
            };
            let tv = |o: [f64; 2], v: [f64; 2]| {
                let (a, b) = (tf(o), tf([o[0] + v[0], o[1] + v[1]]));
                [b[0] - a[0], b[1] - a[1]]
            };
            let shape = ShapePath {
                points: o.points.iter().map(|p| tf(*p)).collect(),
                in_tangents: o.points.iter().zip(&o.in_tangents).map(|(p, v)| tv(*p, *v)).collect(),
                out_tangents: o.points.iter().zip(&o.out_tangents).map(|(p, v)| tv(*p, *v)).collect(),
                closed: true,
                feather: veymelo_keyframe::Feathers::NONE,
            };
            let lt = cx.project.comp(cid).expect("found").layer(target).expect("found").layer_time(*t);
            keys.push(Keyframe::new(lt, Value::Path(shape)));
        }
        let p = g.prop_mut("path").map_err(Error::not_found)?;
        if keys.len() == 1 { p.set_value(keys.remove(0).value) } else { p.set_keys(keys) }.map_err(Error::params)?;
        // holes cut out of what is around them
        g.prop_mut("mode").map_err(Error::not_found)?.set_value(Value::Choice(if hole { 2 } else { 1 })).map_err(Error::params)?;
        masks.push(mid);
        let l = cx.project.comp_mut(cid).expect("found").layer_mut(target).expect("found");
        l.props.group_mut("masks").map_err(Error::not_found)?.children.push(Node::Group(g));
    }
    cx.label = Some("Auto-trace".into());
    let mut o = Map::new();
    o.insert("masks", Json::array(masks.iter().map(|m| Json::Number(*m as f64))));
    o.insert("layer", target as f64);
    Ok(Json::Object(o))
}

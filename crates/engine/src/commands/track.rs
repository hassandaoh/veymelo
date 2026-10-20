//! Motion tracking: trackers made on a layer showing footage or a composition, its frames
//! analysed (each track point followed, a key a frame), and the track applied — another layer
//! (or a point of it) moved, turned and scaled with what was followed, the tracked layer held
//! still, or another layer's corners pinned to three or four points.

use veymelo_geom::{Mat3, vec2};
use veymelo_json::Value as Json;
use veymelo_keyframe::{Keyframe, Value};
use veymelo_project::tracking::{CHANNELS, KINDS, LOW_CONFIDENCE, TRACK_POINT, TRACKER, point_count};
use veymelo_project::{Group, Layer, Node, Project};
use veymelo_time::Tick;
use veymelo_track::{Channel, Frame, How, Options, Point, Tracker, Unsure, Warp, angle_and_length, nearest_turn};

use super::{COMP, Command, LAYER, Run, find_layer, layer_ids};
use crate::{Args, Ctx, Error, Param, Result, Ty};

const TRACKER_PARAM: Param = Param::opt("tracker", Ty::Text, "the tracker, by name or path (default: the layer's last)");

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "track.new",
            label: "Track Motion",
            summary: "Put a motion tracker on a layer showing footage or a composition: what it is for (transform: another layer moves with it; stabilize: this layer is held still; parallel or perspective: another layer's corners pinned to its three or four points; raw: data alone), what it follows (position, and with rotation or scale two points), its track points (or ones laid out in the middle) and how it matches. track.analyze then follows them.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::opt("kind", Ty::Choice(KINDS), "what it is for (default transform)"),
                    Param::opt("rotation", Ty::Bool, "follow turning too (two points)"),
                    Param::opt("scale", Ty::Bool, "follow growing too (two points)"),
                    Param::opt("position", Ty::Bool, "follow where it goes (default true)"),
                    Param::opt("points", Ty::List, "[{center: [x, y], featureSize?: [w, h], searchSize?: [w, h], searchOffset?: [x, y], attachOffset?: [x, y]}, …] in the layer's pixels: as many as it needs (1; 2 with rotation or scale; 3 parallel: upper left, upper right, lower left; 4 perspective: and lower right)"),
                    Param::opt("target", Ty::Layer, "the layer it moves"),
                    Param::opt("targetProperty", Ty::Text, "a point of the target to move instead of its position, like \"effects/Lens Flare/center\""),
                    Param::opt("channel", Ty::Choice(CHANNELS), "what it compares (default luminance)"),
                    Param::opt("blur", Ty::Number, "soften the frames first, in pixels"),
                    Param::opt("enhance", Ty::Bool, "bring out edges first"),
                    Param::opt("subpixel", Ty::Bool, "to a fraction of a pixel (default true)"),
                    Param::opt("adaptEveryFrame", Ty::Bool, "take the patch afresh every frame"),
                    Param::opt("threshold", Ty::Number, "the confidence, in %, below which it is unsure (default 80)"),
                    Param::opt("lowConfidence", Ty::Choice(LOW_CONFIDENCE), "then: continue, stop, extrapolate or adapt (default)"),
                    Param::opt("followShape", Ty::Bool, "follow each patch turning and growing (default true)"),
                    Param::opt("name", Ty::Text, "what to call it (default: Tracker 1…)"),
                ]
            },
            returns: "{tracker, points}",
            run: Run::Edit(new),
        },
        Command {
            id: "track.analyze",
            label: "Analyze Track",
            summary: "Follow a tracker's points through the layer's frames, from `from` to `to` (backwards when `to` is earlier), each frame a key on their Feature Center, Confidence and Attach Point. It stops early where it is unsure and set to stop, or where the layer shows nothing.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    TRACKER_PARAM,
                    Param::opt("from", Ty::Time, "where it starts (default: the view's time); the points are where they are then"),
                    Param::opt("to", Ty::Time, "where it ends (default: the layer's last frame)"),
                ]
            },
            returns: "{frames, from, to, lowest, stopped?}",
            run: Run::Edit(analyze),
        },
        Command {
            id: "track.apply",
            label: "Apply Track",
            summary: "Apply an analysed tracker: a transform tracker keys its target's position (or target property) at its attach point, and its rotation and scale if it follows them; stabilize keys the tracked layer's anchor point, position, rotation and scale so what was followed holds still; a corner pin tracker gives its target a Corner Pin effect keyed to its points.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    TRACKER_PARAM,
                    Param::opt("target", Ty::Layer, "the layer it moves (default: the tracker's; kept as it)"),
                    Param::opt("targetProperty", Ty::Text, "a point of the target to move instead of its position"),
                    Param::opt("dims", Ty::Choice(&["xy", "x", "y"]), "which of a position it moves (default xy)"),
                ]
            },
            returns: "{keys, target, paths}",
            run: Run::Edit(apply),
        },
        Command {
            id: "effect.analyze",
            label: "Analyze",
            summary: "Work out what an effect needs from all of a layer's frames before it can draw: how the camera moved, for Warp Stabilizer and Rolling Shutter Repair. It looks at the frames the effect works on (the layer's, cut by its masks and changed by the effects above it) from the layer's first frame to its last (or `from` to `to`), and keeps what it found in the effect's Analysis; changing the effect's other settings afterwards needs no new analysis.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::opt("effect", Ty::Text, "the effect (default: the layer's first that needs analysing)"),
                    Param::opt("from", Ty::Time, "the first frame (default: the layer's first)"),
                    Param::opt("to", Ty::Time, "the last frame (default: the layer's last)"),
                ]
            },
            returns: "{effect, frames, followed, agreed}",
            run: Run::Edit(analyze_effect),
        },
        Command {
            id: "mask.track",
            label: "Track Mask",
            summary: "Make a mask follow what is inside it through the layer's frames (from `from` to `to`, backwards when `to` is earlier), a key a frame on its path: moved; moved, turned and scaled; slanted too; or in perspective. It stops early where too little inside it can be followed.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::need("mask", Ty::Text, "the mask, by name or \"#n\""),
                    Param::opt("method", Ty::Choice(MASK_METHODS), "how it may move (default positionScaleRotation)"),
                    Param::opt("from", Ty::Time, "where it starts (default: the view's time); the mask is as it is then"),
                    Param::opt("to", Ty::Time, "where it ends (default: the layer's last frame)"),
                ]
            },
            returns: "{mask, frames, from, to, stopped?}",
            run: Run::Edit(track_mask),
        },
    ]
}

/// How a tracked mask may move.
const MASK_METHODS: &[&str] = &["position", "positionScaleRotation", "positionScaleRotationSkew", "perspective"];

fn track_mask(cx: &mut Ctx, a: &Args) -> Result<Json> {
    use veymelo_track::fit::{Model, times};
    use veymelo_track::region::Region;
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let (rate, layer) = (comp.frame_rate, comp.layer(lid).expect("found"));
    let masks: Vec<&Group> = layer.props.group("masks").map(|l| l.children.iter().filter_map(Node::as_group).collect()).unwrap_or_default();
    let want = a.text("mask").unwrap_or("");
    let mask = match want.strip_prefix('#').map(str::parse::<usize>) {
        Some(Ok(n)) => n.checked_sub(1).and_then(|i| masks.get(i)),
        _ => masks.iter().find(|m| m.name() == want.trim_start_matches("masks/")),
    }
    .ok_or_else(|| Error::not_found(format!("\"{}\" has no mask \"{want}\" (it has {})", layer.name, if masks.is_empty() { "none".to_string() } else { masks.iter().map(|m| format!("\"{}\"", m.name())).collect::<Vec<_>>().join(", ") })))?;
    let path = format!("masks/{}/path", mask.name());
    let model = [Model::Move, Model::Similar, Model::Slant, Model::Perspective][a.text("method").and_then(|m| MASK_METHODS.iter().position(|x| *x == m)).unwrap_or(1)];
    let frame = rate.frame_duration();
    let from = rate.snap_nearest(a.time("from").unwrap_or(cx.view.time));
    let to = rate.snap_nearest(a.time("to").unwrap_or(rate.snap(layer.out_point.min(comp.duration) - frame)));
    if to == from {
        return Err(Error::params("`from` and `to` are the same frame: nothing to follow"));
    }
    let (f0, f1) = (rate.frame_at(from), rate.frame_at(to));
    let times_: Vec<Tick> = if f1 > f0 { (f0..=f1).map(|f| rate.tick_of(f)).collect() } else { (f1..=f0).rev().map(|f| rate.tick_of(f)).collect() };
    let start = layer.props.prop(&path).map_err(Error::not_found)?.value_at(layer.layer_time(from));
    let shape = start.as_path().cloned().ok_or_else(|| Error::refused("the mask has no path"))?;
    if shape.points.len() < 2 {
        return Err(Error::refused("the mask's path has too few points to follow what is inside it"));
    }
    // its outline, its curves cut fine
    let n = shape.points.len();
    let mut outline = Vec::new();
    for i in 0..n {
        let j = (i + 1) % n;
        if j == 0 && !shape.closed {
            break;
        }
        let (p, q) = (shape.points[i], shape.points[j]);
        let (c1, c2) = ([p[0] + shape.out_tangents[i][0], p[1] + shape.out_tangents[i][1]], [q[0] + shape.in_tangents[j][0], q[1] + shape.in_tangents[j][1]]);
        for k in 0..8 {
            let u = k as f64 / 8.0;
            let v = 1.0 - u;
            outline.push([0, 1].map(|d| v * v * v * p[d] + 3.0 * v * v * u * c1[d] + 3.0 * v * u * u * c2[d] + u * u * u * q[d]));
        }
    }
    let picture = |t: Tick| veymelo_render::source_pixels(cx.project, cid, lid, t).filter(|b| b.img.width > 0);
    let first = picture(from).ok_or_else(|| Error::refused(format!("\"{}\" shows nothing at {} s to follow", layer.name, from.seconds())))?;
    let offset = |b: &veymelo_effects::Buf| [-b.origin.x * b.scale, -b.origin.y * b.scale];
    let mut region = Region::new(&first.img, offset(&first), outline, model);
    let mut total = Warp::NONE.0;
    let mut keys: Vec<(Tick, Value)> = vec![(layer.layer_time(from), start.clone())];
    let mut stopped = None;
    for &t in &times_[1..] {
        let Some(b) = picture(t) else {
            stopped = Some(format!("the layer shows nothing at {} s", t.seconds()));
            break;
        };
        let Some((w, _)) = region.step(&b.img, offset(&b)) else {
            stopped = Some(format!("too little inside it could be followed at {} s", t.seconds()));
            break;
        };
        total = times(&w.0, &total);
        let warp = Warp(total);
        let mut moved = shape.clone();
        for i in 0..n {
            let (p, ti, to_) = (shape.points[i], shape.in_tangents[i], shape.out_tangents[i]);
            let q = warp.apply(p);
            let (qi, qo) = (warp.apply([p[0] + ti[0], p[1] + ti[1]]), warp.apply([p[0] + to_[0], p[1] + to_[1]]));
            moved.points[i] = q;
            moved.in_tangents[i] = [qi[0] - q[0], qi[1] - q[1]];
            moved.out_tangents[i] = [qo[0] - q[0], qo[1] - q[1]];
        }
        keys.push((layer.layer_time(t), Value::Path(moved)));
    }
    let end = layer.comp_time(keys.last().map_or(Tick::ZERO, |k| k.0));
    let frames = keys.len();
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    keyed(layer, &path, &keys)?;
    cx.label = Some("Track Mask".into());
    let mut o = veymelo_json::Map::new();
    o.insert("mask", path.trim_end_matches("/path"));
    o.insert("frames", frames as f64);
    o.insert("from", from.seconds());
    o.insert("to", end.seconds());
    if let Some(why) = stopped {
        o.insert("stopped", why);
    }
    Ok(Json::Object(o))
}

fn analyze_effect(cx: &mut Ctx, a: &Args) -> Result<Json> {
    use veymelo_effects::steady::ANALYSED;
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let (rate, layer) = (comp.frame_rate, comp.layer(lid).expect("found"));
    let list: Vec<&Group> = layer.props.group("effects").map(|l| l.children.iter().filter_map(Node::as_group).collect()).unwrap_or_default();
    let index = match a.text("effect") {
        Some(name) => list.iter().position(|g| g.name() == name.trim_start_matches("effects/")).ok_or_else(|| Error::not_found(format!("\"{}\" has no effect \"{name}\"", layer.name)))?,
        None => list.iter().position(|g| ANALYSED.contains(&g.def.id)).ok_or_else(|| Error::not_found(format!("\"{}\" has no effect that needs analysing ({})", layer.name, ANALYSED.join(", "))))?,
    };
    let fx = list[index];
    if !ANALYSED.contains(&fx.def.id) {
        return Err(Error::params(format!("{} needs no analysis (only {} do)", fx.name(), ANALYSED.join(", "))));
    }
    let name = fx.name().to_string();
    let frame = rate.frame_duration();
    let from = rate.snap_nearest(a.time("from").unwrap_or(layer.in_point.max(Tick::ZERO)));
    let to = rate.snap_nearest(a.time("to").unwrap_or(rate.snap(layer.out_point.min(comp.duration) - frame)));
    if to < from {
        return Err(Error::params("`to` is before `from`"));
    }
    let times: Vec<Tick> = (rate.frame_at(from)..=rate.frame_at(to)).map(|f| rate.tick_of(f)).collect();
    let step = if times.len() > 1 { (layer.layer_time(times[1]) - layer.layer_time(times[0])).seconds() } else { frame.seconds() };
    let mut analyzer = None;
    for &t in &times {
        let (buf, _) = veymelo_render::effect_input(cx.project, cid, lid, index, t).ok_or_else(|| Error::refused(format!("\"{}\" shows nothing at {} s to analyse", layer.name, t.seconds())))?;
        let (o, s) = (buf.origin, buf.scale);
        let an = analyzer.get_or_insert_with(|| veymelo_track::stabilize::Analyzer::new(layer.layer_time(t).seconds(), step, [o.x, o.y, o.x + buf.img.width as f64 / s, o.y + buf.img.height as f64 / s]));
        an.add(&buf.img, [-o.x * s, -o.y * s]);
    }
    let analysis = analyzer.expect("a frame at least").finish();
    let n = analysis.frames.len().saturating_sub(1).max(1) as f64;
    let followed = analysis.frames.iter().skip(1).map(|m| m.followed as f64).sum::<f64>() / n;
    let agreed = analysis.frames.iter().skip(1).map(|m| if m.followed > 0 { m.agreed as f64 / m.followed as f64 } else { 0.0 }).sum::<f64>() / n;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    layer.props.prop_mut(&format!("effects/{name}/analysis")).map_err(Error::refused)?.set_value(Value::Text(analysis.to_text())).map_err(Error::refused)?;
    cx.label = Some(format!("Analyze {name}"));
    Ok(Json::object([
        ("effect", Json::from(format!("effects/{name}"))),
        ("frames", Json::Number(analysis.frames.len() as f64)),
        ("followed", Json::Number(followed.round())),
        ("agreed", Json::Number((agreed * 1000.0).round() / 10.0)),
    ]))
}

fn pair(j: Option<&Json>, what: &str) -> Result<Option<[f64; 2]>> {
    let Some(j) = j else { return Ok(None) };
    let v: Vec<f64> = j.as_array().map(|a| a.iter().filter_map(Json::as_f64).collect()).unwrap_or_default();
    match v[..] {
        [x, y] if x.is_finite() && y.is_finite() => Ok(Some([x, y])),
        _ => Err(Error::params(format!("`{what}` is [x, y], not {}", j.to_json()))),
    }
}

/// The trackers of a layer, with their paths.
fn trackers(layer: &Layer) -> Vec<(String, &Group)> {
    let Ok(list) = layer.props.group("motionTrackers") else { return Vec::new() };
    list.children.iter().filter_map(Node::as_group).map(|g| (format!("motionTrackers/{}", g.name()), g)).collect()
}

/// The path of the tracker the parameters name (or the layer's last).
fn tracker_of(layer: &Layer, a: &Args) -> Result<String> {
    let all = trackers(layer);
    let found = match a.text("tracker") {
        Some(n) => all.iter().find(|(path, g)| path == n.trim_matches('/') || g.name() == n),
        None => all.last(),
    };
    found.map(|(p, _)| p.clone()).ok_or_else(|| {
        let have = if all.is_empty() { "it has none (track.new makes one)".to_string() } else { format!("it has {}", all.iter().map(|(p, _)| format!("\"{p}\"")).collect::<Vec<_>>().join(", ")) };
        Error::not_found(format!("\"{}\" has no tracker {} ({have})", layer.name, a.text("tracker").map_or(String::new(), |n| format!("\"{n}\""))))
    })
}

/// A tracker's settings and its track points' paths.
struct Setup {
    kind: u32,
    position: bool,
    rotation: bool,
    scale: bool,
    target: Option<u64>,
    target_property: String,
    options: Options,
    points: Vec<String>,
}

fn setup(layer: &Layer, path: &str) -> Setup {
    let g = layer.props.group(path).expect("found");
    let get = |p: &str| g.prop(p).map(|p| p.value.clone()).unwrap_or(Value::Number(0.0));
    Setup {
        kind: get("kind").as_choice(),
        position: get("position").as_bool(),
        rotation: get("rotation").as_bool(),
        scale: get("scale").as_bool(),
        target: get("target").as_layer(),
        target_property: get("targetProperty").as_text().to_string(),
        options: Options {
            channel: Channel::from_index(get("channel").as_choice()),
            blur: get("blur").as_f64(),
            enhance: get("enhance").as_bool(),
            subpixel: get("subpixel").as_bool(),
            adapt_every_frame: get("adaptEveryFrame").as_bool(),
            threshold: get("threshold").as_f64(),
            unsure: Unsure::from_index(get("lowConfidence").as_choice()),
            follow_shape: get("followShape").as_bool(),
        },
        points: g.group("points").map(|l| l.children.iter().filter_map(Node::as_group).map(|p| format!("{path}/points/{}", p.name())).collect()).unwrap_or_default(),
    }
}

fn new(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let layer = comp.layer(lid).expect("found");
    if layer.props.group("motionTrackers").is_err() {
        return Err(Error::refused(format!("\"{}\" shows no footage or composition, so there is nothing moving in it to track", layer.name)));
    }
    let kind = a.text("kind").and_then(|k| KINDS.iter().position(|i| *i == k)).unwrap_or(0) as u32;
    let (rotation, scale) = (a.bool("rotation").unwrap_or(false), a.bool("scale").unwrap_or(false));
    let n = point_count(kind, rotation || scale);
    let target = match a.json("target") {
        Some(j) => Some(find_layer(comp, j)?),
        None => None,
    };
    if target == Some(lid) {
        return Err(Error::params("a layer cannot follow its own track (a stabilize tracker holds it still)"));
    }
    // the points: as given, or laid out round the layer's middle
    let (w, h) = super::layer::natural_size(cx.project, layer);
    let feature = (w.min(h) * 0.04).clamp(16.0, 80.0).round();
    let search = 2.0 * feature;
    let mut points: Vec<[[f64; 2]; 5]> = Vec::new();
    match a.json("points").and_then(Json::as_array) {
        Some(list) => {
            if list.len() != n {
                return Err(Error::params(format!("a {} tracker{} has {n} track point{}, not {}", KINDS[kind as usize], if rotation || scale { " following turning or growing" } else { "" }, if n == 1 { "" } else { "s" }, list.len())));
            }
            for (i, q) in list.iter().enumerate() {
                let field = |k: &str| pair(q.get(k), &format!("points[{i}].{k}"));
                let center = field("center")?.ok_or_else(|| Error::params(format!("point {} needs `center`", i + 1)))?;
                points.push([center, field("featureSize")?.unwrap_or([feature; 2]), field("searchSize")?.unwrap_or([search; 2]), field("searchOffset")?.unwrap_or([0.0; 2]), field("attachOffset")?.unwrap_or([0.0; 2])]);
            }
        }
        None => {
            let (cx0, cy0) = (w / 2.0, h / 2.0);
            let (dx, dy) = ((w * 0.2).max(2.0 * search), (h * 0.2).max(2.0 * search));
            let at: Vec<[f64; 2]> = match n {
                1 => vec![[cx0, cy0]],
                2 => vec![[cx0 - dx, cy0], [cx0 + dx, cy0]],
                3 => vec![[cx0 - dx, cy0 - dy], [cx0 + dx, cy0 - dy], [cx0 - dx, cy0 + dy]],
                _ => vec![[cx0 - dx, cy0 - dy], [cx0 + dx, cy0 - dy], [cx0 - dx, cy0 + dy], [cx0 + dx, cy0 + dy]],
            };
            points.extend(at.into_iter().map(|c| [c, [feature; 2], [search; 2], [0.0; 2], [0.0; 2]]));
        }
    }
    let mut set: Vec<(&str, Value)> = vec![("kind", Value::Choice(kind)), ("rotation", Value::Bool(rotation)), ("scale", Value::Bool(scale))];
    for key in ["position", "enhance", "subpixel", "adaptEveryFrame", "followShape"] {
        if let Some(v) = a.bool(key) {
            set.push((key, Value::Bool(v)));
        }
    }
    for key in ["blur", "threshold"] {
        if let Some(v) = a.num(key) {
            set.push((key, Value::Number(v)));
        }
    }
    for (key, ids) in [("channel", CHANNELS), ("lowConfidence", LOW_CONFIDENCE)] {
        if let Some(i) = a.text(key).and_then(|v| ids.iter().position(|x| *x == v)) {
            set.push((key, Value::Choice(i as u32)));
        }
    }
    if let Some(t) = target {
        set.push(("target", Value::Layer(Some(t))));
    }
    if let Some(p) = a.text("targetProperty") {
        set.push(("targetProperty", Value::Text(p.to_string())));
    }
    let list = layer.props.group("motionTrackers").expect("checked");
    let name = match a.text("name") {
        Some(n) => n.to_string(),
        None => (1..).map(|k| format!("Tracker {k}")).find(|n| !list.children.iter().any(|c| c.name() == n)).expect("some number is free"),
    };
    if list.children.iter().any(|c| c.name() == name) {
        return Err(Error::params(format!("\"{}\" has a tracker \"{name}\" already", layer.name)));
    }
    let mut g = Group::instance(&TRACKER, cx.project.alloc(), &name);
    for (path, v) in set {
        g.prop_mut(path).expect("defined").set_value(v).map_err(|e| Error::params(format!("{path}: {e}")))?;
    }
    let mut paths = Vec::new();
    for (i, [center, size, search, offset, attach]) in points.into_iter().enumerate() {
        let pname = format!("Track Point {}", i + 1);
        let mut pg = Group::instance(&TRACK_POINT, cx.project.alloc(), &pname);
        for (path, v) in [("featureCenter", center), ("featureSize", size), ("searchSize", search), ("searchOffset", offset), ("attachPointOffset", attach), ("attachPoint", [center[0] + attach[0], center[1] + attach[1]])] {
            pg.prop_mut(path).expect("defined").set_value(Value::Vec2(v)).map_err(|e| Error::params(format!("{pname} {path}: {e}")))?;
        }
        g.group_mut("points").expect("defined").children.push(Node::Group(pg));
        paths.push(format!("motionTrackers/{name}/points/{pname}"));
    }
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    layer.props.group_mut("motionTrackers").expect("checked").children.push(Node::Group(g));
    cx.label = Some("Track Motion".into());
    Ok(Json::object([("tracker", Json::from(format!("motionTrackers/{name}"))), ("points", Json::array(paths.into_iter().map(Json::from)))]))
}

/// Where the attach points are when the feature centres `now` were `start`, the attach points
/// then being `attach`: through the perspective (four points) or parallel (three) map between
/// them, turned and scaled with the first two (following turning or growing), else each moved as
/// its own feature moved.
fn attached(kind: u32, turns: bool, start: &[[f64; 2]], attach: &[[f64; 2]], now: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let moved = || now.iter().zip(start.iter().zip(attach)).map(|(c, (s, a))| [c[0] + a[0] - s[0], c[1] + a[1] - s[1]]).collect::<Vec<_>>();
    let through = |w: Option<Warp>| w.map_or_else(moved, |w| attach.iter().map(|a| w.apply(*a)).collect());
    match KINDS.get(kind as usize) {
        Some(&"perspective") if now.len() >= 4 => through(Warp::perspective([start[0], start[1], start[2], start[3]], [now[0], now[1], now[2], now[3]])),
        Some(&"parallel") if now.len() >= 3 => through(Warp::parallel([start[0], start[1], start[2]], [now[0], now[1], now[2]])),
        _ if turns && now.len() >= 2 => through(Warp::similar([start[0], start[1]], [now[0], now[1]])),
        _ => moved(),
    }
}

/// Keys at layer times put on property `path` of `layer` (the keys it had at other times kept).
fn keyed(layer: &mut Layer, path: &str, keys: &[(Tick, Value)]) -> Result<()> {
    let p = layer.props.prop_mut(path).map_err(Error::not_found)?;
    let mut all = p.keys.clone();
    for (t, v) in keys {
        veymelo_keyframe::set_key(&mut all, Keyframe::new(*t, v.clone()));
    }
    p.set_keys(all).map_err(|e| Error::params(format!("{path}: {e}")))
}

fn analyze(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let (rate, layer) = (comp.frame_rate, comp.layer(lid).expect("found"));
    let path = tracker_of(layer, a)?;
    let s = setup(layer, &path);
    if s.points.is_empty() {
        return Err(Error::refused(format!("{path} has no track points")));
    }
    let frame = rate.frame_duration();
    let from = rate.snap_nearest(a.time("from").unwrap_or(cx.view.time));
    let last = rate.snap(layer.out_point.min(comp.duration) - frame);
    let to = rate.snap_nearest(a.time("to").unwrap_or(last));
    if to == from {
        return Err(Error::params("`from` and `to` are the same frame: nothing to follow"));
    }
    let (f0, f1) = (rate.frame_at(from), rate.frame_at(to));
    let times: Vec<Tick> = if f1 > f0 { (f0..=f1).map(|f| rate.tick_of(f)).collect() } else { (f1..=f0).rev().map(|f| rate.tick_of(f)).collect() };
    // where the points are when it starts
    let lt0 = layer.layer_time(from);
    let v = |p: &str, k: &str| layer.props.prop(&format!("{p}/{k}")).map_or([0.0; 2], |q| q.value_at(lt0).as_vec2());
    let points: Vec<Point> = s.points.iter().map(|p| Point { center: v(p, "featureCenter"), feature: v(p, "featureSize"), search: v(p, "searchSize"), search_offset: v(p, "searchOffset") }).collect();
    let start: Vec<[f64; 2]> = points.iter().map(|p| p.center).collect();
    let attach: Vec<[f64; 2]> = s.points.iter().zip(&start).map(|(p, c)| {
        let o = v(p, "attachPointOffset");
        [c[0] + o[0], c[1] + o[1]]
    }).collect();
    let turns = s.rotation || s.scale;
    let picture = |t: Tick| veymelo_render::source_pixels(cx.project, cid, lid, t).filter(|b| b.img.width > 0 && b.img.height > 0);
    let first = picture(from).ok_or_else(|| Error::refused(format!("\"{}\" shows nothing at {} s to track", layer.name, from.seconds())))?;
    let frame_of = |b: &veymelo_effects::Buf| [-b.origin.x * b.scale, -b.origin.y * b.scale];
    let mut tracker = Tracker::new(s.options.clone(), &points, &Frame { img: &first.img, offset: frame_of(&first) });
    // (layer time, each point's centre, confidence and attach point)
    let mut done: Vec<(Tick, Vec<Placed>)> = vec![(lt0, start.iter().zip(&attach).map(|(c, a)| (*c, 100.0, *a)).collect())];
    let mut stopped = None;
    for &t in &times[1..] {
        let Some(b) = picture(t) else {
            stopped = Some(format!("the layer shows nothing at {} s", t.seconds()));
            break;
        };
        let found = tracker.step(&Frame { img: &b.img, offset: frame_of(&b) });
        if found.iter().any(|f| f.how == How::Stopped) {
            stopped = Some(format!("unsure at {} s (below {} %)", t.seconds(), s.options.threshold));
            break;
        }
        let now: Vec<[f64; 2]> = found.iter().map(|f| f.center).collect();
        let at = attached(s.kind, turns, &start, &attach, &now);
        done.push((layer.layer_time(t), found.iter().zip(at).map(|(f, a)| (f.center, f.confidence, a)).collect()));
    }
    let lowest = done.iter().flat_map(|(_, p)| p.iter().map(|q| q.1)).fold(100.0f64, f64::min);
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    for (i, p) in s.points.iter().enumerate() {
        let column = |f: &dyn Fn(&Placed) -> Value| done.iter().map(|(t, q)| (*t, f(&q[i]))).collect::<Vec<_>>();
        keyed(layer, &format!("{p}/featureCenter"), &column(&|q| Value::Vec2(q.0)))?;
        keyed(layer, &format!("{p}/confidence"), &column(&|q| Value::Number((q.1 * 100.0).round() / 100.0)))?;
        keyed(layer, &format!("{p}/attachPoint"), &column(&|q| Value::Vec2(q.2)))?;
    }
    let end = layer.comp_time(done.last().map_or(lt0, |d| d.0));
    cx.label = Some("Analyze Track".into());
    let mut o = veymelo_json::Map::new();
    o.insert("frames", done.len() as f64);
    o.insert("from", from.seconds());
    o.insert("to", end.seconds());
    o.insert("lowest", (lowest * 100.0).round() / 100.0);
    if let Some(why) = stopped {
        o.insert("stopped", why);
    }
    Ok(Json::Object(o))
}

/// A point's place on a frame: its feature's middle, the confidence, its attach point.
type Placed = ([f64; 2], f64, [f64; 2]);

/// From a layer's own space to its parent's (the composition's when it has none), at `t`.
fn to_parent(e: &veymelo_expr::Eval, project: &Project, cid: u64, layer: &Layer, t: Tick) -> Mat3 {
    let comp = project.comp(cid).expect("found");
    match layer.parent.and_then(|p| comp.layer(p)) {
        Some(p) => e.matrix(e.place(comp, p), t).inverse().unwrap_or_default(),
        None => Mat3::default(),
    }
}

fn apply(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let source = comp.layer(lid).expect("found");
    let path = tracker_of(source, a)?;
    let s = setup(source, &path);
    // the attach points at each analysed time
    let track: Vec<(Tick, Vec<[f64; 2]>)> = {
        let p0 = source.props.prop(&format!("{}/attachPoint", s.points.first().ok_or_else(|| Error::refused("it has no track points"))?)).expect("defined");
        p0.keys.iter().map(|k| (k.time, s.points.iter().map(|p| source.props.prop(&format!("{p}/attachPoint")).map_or([0.0; 2], |q| q.value_at(k.time).as_vec2())).collect())).collect()
    };
    if track.is_empty() {
        return Err(Error::refused(format!("{path} has not been analysed yet (track.analyze)")));
    }
    let kind = KINDS[s.kind as usize];
    let target = match (kind, a.json("target")) {
        ("raw", _) => return Err(Error::refused("a raw tracker is data alone: it has nothing to apply")),
        ("stabilize", _) => lid,
        (_, Some(j)) => find_layer(comp, j)?,
        (_, None) => s.target.filter(|t| comp.layer(*t).is_some()).ok_or_else(|| Error::params("give `target`: the layer it moves"))?,
    };
    if target == lid && kind != "stabilize" {
        return Err(Error::params("a layer cannot follow its own track (a stabilize tracker holds it still)"));
    }
    let tl = comp.layer(target).expect("found");
    if source.is_3d() || tl.is_3d() {
        return Err(Error::refused("a track is applied to 2D layers (a 3D layer is placed in depth too)"));
    }
    let property = a.text("targetProperty").map(str::to_string).unwrap_or(s.target_property.clone());
    if !property.is_empty() {
        let p = tl.props.prop(&property).map_err(Error::not_found)?;
        if !matches!(p.def.kind, veymelo_keyframe::Kind::Vec2 | veymelo_keyframe::Kind::Vec3) {
            return Err(Error::params(format!("{property} is not a point")));
        }
    }
    let dims = a.text("dims").unwrap_or("xy");
    let mix = |old: [f64; 3], new: [f64; 2]| match dims {
        "x" => [new[0], old[1], old[2]],
        "y" => [old[0], new[1], old[2]],
        _ => [new[0], new[1], old[2]],
    };
    let e = veymelo_expr::Eval::new(cx.project);
    let (sp, tp) = (e.place(comp, source), e.place(comp, tl));
    let at = |m: &Mat3, p: [f64; 2]| {
        let q = m.apply(vec2(p[0], p[1]));
        [q.x, q.y]
    };
    let ct0 = source.comp_time(track[0].0);
    let turns = (s.rotation || s.scale) && track[0].1.len() >= 2;
    let (angle0, length0) = if turns { angle_and_length(track[0].1[0], track[0].1[1]) } else { (0.0, 1.0) };
    let tt0 = tl.layer_time(ct0);
    let rot0 = tl.props.prop("transform/rotation").map_or(0.0, |p| p.value_at(tt0).as_f64());
    let scale0 = tl.props.prop("transform/scale").map_or([100.0; 3], |p| p.value_at(tt0).as_vec3());
    // where the followed point stays when stabilizing: where it was at the first frame
    let still = at(&to_parent(&e, cx.project, cid, source, ct0), at(&e.matrix(sp, ct0), track[0].1[0]));
    let mut writes: Vec<(String, Vec<(Tick, Value)>)> = Vec::new();
    let mut put = |path: &str, t: Tick, v: Value| match writes.iter_mut().find(|(p, _)| p == path) {
        Some((_, keys)) => keys.push((t, v)),
        None => writes.push((path.to_string(), vec![(t, v)])),
    };
    let mut turned = 0.0;
    let mut corners: Vec<(Tick, [[f64; 2]; 4])> = Vec::new();
    for (lt, pts) in &track {
        let ct = source.comp_time(*lt);
        let tt = tl.layer_time(ct);
        let (angle, length) = if turns { angle_and_length(pts[0], pts[1]) } else { (angle0, length0) };
        turned = nearest_turn(turned, angle - angle0);
        let grow = if length0 > 1e-9 { length / length0 } else { 1.0 };
        let old_pos = tl.position_at(tt);
        let in_comp = |p: [f64; 2]| at(&e.matrix(sp, ct), p);
        match kind {
            "stabilize" => {
                if s.position {
                    let anchor = tl.props.prop("transform/anchor").map_or([0.0; 3], |p| p.value_at(tt).as_vec3());
                    put("transform/anchor", tt, Value::Vec3(mix(anchor, pts[0])));
                    put("transform/position", tt, Value::Vec3(mix(old_pos, still)));
                }
                if s.rotation && turns {
                    put("transform/rotation", tt, Value::Number(rot0 - turned));
                }
                if s.scale && turns {
                    put("transform/scale", tt, Value::Vec3([scale0[0] / grow, scale0[1] / grow, scale0[2]]));
                }
            }
            "transform" => {
                if s.position {
                    let c = in_comp(pts[0]);
                    if property.is_empty() {
                        put("transform/position", tt, Value::Vec3(mix(old_pos, at(&to_parent(&e, cx.project, cid, tl, ct), c))));
                    } else {
                        let q = at(&e.matrix(tp, ct).inverse().unwrap_or_default(), c);
                        let old = tl.props.prop(&property).map_or([0.0; 3], |p| p.value_at(tt).as_vec3());
                        let v = mix(old, q);
                        put(&property, tt, if tl.props.prop(&property).is_ok_and(|p| p.def.kind == veymelo_keyframe::Kind::Vec2) { Value::Vec2([v[0], v[1]]) } else { Value::Vec3(v) });
                    }
                }
                if s.rotation && turns {
                    put("transform/rotation", tt, Value::Number(rot0 + turned));
                }
                if s.scale && turns {
                    put("transform/scale", tt, Value::Vec3([scale0[0] * grow, scale0[1] * grow, scale0[2]]));
                }
            }
            _ => {
                if pts.len() < 3 {
                    return Err(Error::refused("a corner pin tracker needs three or four track points"));
                }
                // points 1 to 4: upper left, upper right, lower left, lower right
                let lower_right = if kind == "parallel" || pts.len() < 4 { [pts[1][0] + pts[2][0] - pts[0][0], pts[1][1] + pts[2][1] - pts[0][1]] } else { pts[3] };
                let back = e.matrix(tp, ct).inverse().ok_or_else(|| Error::refused("the target is scaled to nothing"))?;
                let pin = |p: [f64; 2]| at(&back, in_comp(p));
                corners.push((tt, [pin(pts[0]), pin(pts[1]), pin(pts[2]), pin(lower_right)]));
            }
        }
    }
    if !corners.is_empty() {
        let made = super::call(cx, "effect.add", Json::object([("comp", Json::Number(cid as f64)), ("layer", Json::Number(target as f64)), ("type", Json::from("cornerPin"))]))?;
        let fx = made.get("path").and_then(Json::as_str).expect("made").to_string();
        for (k, corner) in ["upperLeft", "upperRight", "lowerLeft", "lowerRight"].iter().enumerate() {
            writes.push((format!("{fx}/{corner}"), corners.iter().map(|(t, c)| (*t, Value::Vec2(c[k]))).collect()));
        }
    }
    // a tracker remembers the target it was applied to
    if kind != "stabilize" {
        let src = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
        src.props.prop_mut(&format!("{path}/target")).expect("defined").set_value(Value::Layer(Some(target))).map_err(Error::refused)?;
    }
    let tl = cx.project.comp_mut(cid).expect("found").layer_mut(target).expect("found");
    let separate = tl.separate_position;
    let mut paths = Vec::new();
    for (path, keys) in &writes {
        if path == "transform/position" && separate {
            for (axis, k) in [("transform/positionX", 0), ("transform/positionY", 1)] {
                keyed(tl, axis, &keys.iter().map(|(t, v)| (*t, Value::Number(v.as_vec3()[k]))).collect::<Vec<_>>())?;
                paths.push(axis.to_string());
            }
        } else {
            keyed(tl, path, keys)?;
            paths.push(path.clone());
        }
    }
    cx.label = Some("Apply Track".into());
    Ok(Json::object([("keys", Json::Number(track.len() as f64)), ("target", Json::Number(target as f64)), ("paths", Json::array(paths.into_iter().map(Json::from)))]))
}


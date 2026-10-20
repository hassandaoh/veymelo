//! Commands about time: time remapping and freeze frames, playing a layer backwards or at
//! another speed, and markers on compositions and layers.

use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::{Keyframe, Value};
use veymelo_project::{Label, Layer, Marker, Source};
use veymelo_time::Tick;

use super::{COMP, Command, LAYER, Run, comp_id, find_layer, layer_ids};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "layer.timeRemap",
            label: "Time Remapping",
            summary: "Turn time remapping of a composition layer on (its timeRemap property then says what time of the composition is shown at each moment, starting as a straight line from its in point to its out point) or off.",
            params: const { &[COMP, LAYER, Param::need("on", Ty::Bool, "on or off")] },
            returns: "{layer, keys}",
            run: Run::Edit(time_remap),
        },
        Command {
            id: "layer.freeze",
            label: "Freeze Frame",
            summary: "Hold a composition layer on one frame for its whole length (time remapping with one held key).",
            params: const { &[COMP, LAYER, Param::opt("time", Ty::Time, "the moment to hold (default: the view's time)")] },
            returns: "{layer, held}",
            run: Run::Edit(freeze),
        },
        Command {
            id: "layer.reverse",
            label: "Time-Reverse Layer",
            summary: "Play a layer backwards, staying where it is in the composition.",
            params: const { &[COMP, LAYER] },
            returns: "{layer, stretch}",
            run: Run::Edit(reverse),
        },
        Command {
            id: "layer.stretch",
            label: "Time Stretch",
            summary: "Make a layer's time go slower or faster: a stretch of 200 % lasts twice as long. Its in point stays (or its out point, or the view's time).",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::need("stretch", Ty::Number, "the new stretch in % (below zero plays backwards)"),
                    Param::opt("hold", Ty::Choice(&["in", "out", "time"]), "what stays where it is: the in point (default), the out point, or the frame at the view's time"),
                ]
            },
            returns: "{layer, in, out, start}",
            run: Run::Edit(stretch),
        },
        Command {
            id: "marker.add",
            label: "Add Marker",
            summary: "Mark a moment (or a stretch of time) of a composition, or of a layer (it moves with the layer), with a note.",
            params: const {
                &[
                    COMP,
                    Param::opt("layer", Ty::Layer, "mark this layer instead of the composition"),
                    Param::opt("time", Ty::Time, "when (default: the view's time)"),
                    Param::opt("duration", Ty::Time, "how long it lasts (default 0)"),
                    Param::opt("comment", Ty::Text, "its note"),
                    Param::opt("label", Ty::Choice(Label::IDS), "its colour tag"),
                    Param::opt("chapter", Ty::Text, "a chapter name, for videos that have chapters"),
                    Param::opt("url", Ty::Text, "a web link"),
                    Param::opt("protected", Ty::Bool, "its stretch of time keeps its length when the time around it is stretched"),
                    Param::opt("frameTarget", Ty::Text, "the web page frame its link opens in"),
                    Param::opt("cuePoint", Ty::Any, "a cue point for players that read them: {name, navigation?, params?: [[name, value]…]}"),
                ]
            },
            returns: "{time, index}",
            run: Run::Edit(marker_add),
        },
        Command {
            id: "marker.set",
            label: "Marker Settings",
            summary: "Change a marker: move it (`to`), how long it lasts, its note, colour tag, chapter, link and the frame it opens in, its cue point (null takes it away), whether its stretch keeps its length.",
            params: const {
                &[
                    COMP,
                    Param::opt("layer", Ty::Layer, "a layer's marker instead of the composition's"),
                    Param::need("time", Ty::Time, "the marker's time (within half a frame)"),
                    Param::opt("to", Ty::Time, "move it to this time"),
                    Param::opt("duration", Ty::Time, "how long it lasts"),
                    Param::opt("comment", Ty::Text, "its note"),
                    Param::opt("label", Ty::Choice(Label::IDS), "its colour tag"),
                    Param::opt("chapter", Ty::Text, "a chapter name"),
                    Param::opt("url", Ty::Text, "a web link"),
                    Param::opt("frameTarget", Ty::Text, "the web page frame its link opens in"),
                    Param::opt("cuePoint", Ty::Any, "{name, navigation?, params?: [[name, value]…]}, or null to take it away"),
                    Param::opt("protected", Ty::Bool, "its stretch of time keeps its length when the time around it is stretched"),
                ]
            },
            returns: "{time, index}",
            run: Run::Edit(marker_set),
        },
        Command {
            id: "marker.convert",
            label: "Convert Marker",
            summary: "Make a layer's marker the composition's (give `layer`), or the composition's a layer's (give `toLayer`), at the same moment.",
            params: const {
                &[
                    COMP,
                    Param::opt("layer", Ty::Layer, "the layer whose marker becomes the composition's"),
                    Param::need("time", Ty::Time, "the marker's time"),
                    Param::opt("toLayer", Ty::Layer, "the layer the composition's marker goes to"),
                ]
            },
            returns: "{time, to}",
            run: Run::Edit(marker_convert),
        },
        Command {
            id: "marker.nested",
            label: "Nested Composition Markers",
            summary: "The markers of the composition a layer shows, at the moments they come in this one (moved, stretched and time-remapped as the layer shows it), those while the layer is on.",
            params: const { &[COMP, LAYER] },
            returns: "[{time, nestedTime, index, comment?, duration?, …}]",
            run: Run::Look(marker_nested),
        },
        Command {
            id: "marker.fromSource",
            label: "Update Markers From Source",
            summary: "Copy the markers of the compositions layers show onto the layers, where they come in them (the layers' own markers kept; one already at a moment is not doubled).",
            params: const { &[COMP, Param::need("layers", Ty::List, "composition layers (ids, names or \"#n\")")] },
            returns: "{added}",
            run: Run::Edit(marker_from_source),
        },
        Command {
            id: "marker.list",
            label: "Markers",
            summary: "The markers of a composition, or of a layer.",
            params: const { &[COMP, Param::opt("layer", Ty::Layer, "a layer's markers instead")] },
            returns: "[{index, time, duration?, comment?, label?, …}]",
            run: Run::Look(marker_list),
        },
        Command {
            id: "marker.remove",
            label: "Remove Marker",
            summary: "Take away the marker at a time (within half a frame), or all of them.",
            params: const { &[COMP, Param::opt("layer", Ty::Layer, "a layer's marker instead"), Param::opt("time", Ty::Time, "the marker's time"), Param::opt("all", Ty::Bool, "take every one away")] },
            returns: "{removed}",
            run: Run::Edit(marker_remove),
        },
    ]
}

/// The layer, which must show a composition (or footage) for its time to be remapped.
fn timed<'a>(cx: &'a mut Ctx, a: &Args) -> Result<&'a mut Layer> {
    let (cid, id) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(id).expect("found");
    if !matches!(layer.source, Source::Comp(_) | Source::Footage(_)) {
        return Err(Error::refused(format!("\"{}\" is a {} layer; only composition and footage layers show a time of their own", layer.name, layer.source.id())));
    }
    Ok(layer)
}

fn time_remap(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let layer = timed(cx, a)?;
    let on = a.bool("on").unwrap_or(false);
    let (t_in, t_out) = (layer.layer_time(layer.in_point), layer.layer_time(layer.out_point));
    let was = layer.time_remap;
    layer.time_remap = on;
    let p = layer.props.prop_mut("timeRemap").expect("defined");
    if on && !was {
        // a straight line: each moment shows the time it showed before
        let keys = vec![Keyframe::new(t_in.min(t_out), Value::Number(t_in.min(t_out).seconds())), Keyframe::new(t_in.max(t_out), Value::Number(t_in.max(t_out).seconds()))];
        p.set_keys(keys).map_err(Error::params)?;
    } else if !on {
        p.set_value(Value::Number(0.0)).map_err(Error::params)?;
    }
    let keys = veymelo_keyframe::keys_to_json(&p.keys, p.def.kind);
    let out = Json::object([("layer", Json::Number(layer.id as f64)), ("keys", keys)]);
    cx.label = Some(if on { "Enable Time Remapping".into() } else { "Disable Time Remapping".into() });
    Ok(out)
}

fn freeze(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let view = cx.view.time;
    let t = a.time("time").unwrap_or(view);
    let held = {
        let (cid, id) = layer_ids(cx.project, cx.view, a)?;
        let project: &veymelo_project::Project = cx.project;
        project.comp(cid).expect("found").layer(id).expect("found").shown_time(project, t)
    };
    let layer = timed(cx, a)?;
    let at = layer.layer_time(t);
    layer.time_remap = true;
    let p = layer.props.prop_mut("timeRemap").expect("defined");
    p.set_keys(vec![Keyframe::new(at, Value::Number(held.seconds())).hold()]).map_err(Error::params)?;
    let out = Json::object([("layer", Json::Number(layer.id as f64)), ("held", Json::Number(held.seconds()))]);
    cx.label = Some("Freeze Frame".into());
    Ok(out)
}

fn reverse(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, id) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(id).expect("found");
    // its in point now shows what its out point showed, and the other way round
    layer.start = layer.in_point + layer.out_point - layer.start;
    layer.stretch = -layer.stretch;
    cx.label = Some(format!("Time-Reverse Layer {}", layer.name));
    Ok(Json::object([("layer", Json::Number(id as f64)), ("stretch", Json::Number(layer.stretch))]))
}

fn stretch(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let view = cx.view.time;
    let (cid, id) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(id).expect("found");
    let new = a.num("stretch").unwrap_or(100.0);
    if new == 0.0 || new.abs() > 1e6 {
        return Err(Error::params("`stretch` is a percentage other than 0"));
    }
    let old = layer.stretch;
    let k = new / old;
    let pin = match a.text("hold").unwrap_or("in") {
        "out" => layer.out_point,
        "time" => view,
        _ => layer.in_point,
    };
    // the pinned moment shows the same layer time as before
    let scaled = |t: Tick| pin + Tick(((t - pin).0 as f64 * k.abs()).round() as i64);
    let lt = layer.layer_time(pin);
    let (a_in, a_out) = (scaled(layer.in_point), scaled(layer.out_point));
    layer.stretch = new;
    layer.start = pin - Tick((lt.0 as f64 * new / 100.0).round() as i64);
    layer.in_point = a_in.min(a_out);
    layer.out_point = a_in.max(a_out);
    if layer.out_point <= layer.in_point {
        return Err(Error::refused("that stretch leaves the layer no time at all"));
    }
    cx.label = Some(format!("Time Stretch {}", layer.name));
    Ok(Json::object([
        ("layer", Json::Number(id as f64)),
        ("in", Json::Number(layer.in_point.seconds())),
        ("out", Json::Number(layer.out_point.seconds())),
        ("start", Json::Number(layer.start.seconds())),
    ]))
}

/// How composition time becomes the own time of a list of markers: the same for a
/// composition's, the layer's time for a layer's (its markers move with it).
#[derive(Clone, Copy)]
struct Clock {
    start: Tick,
    stretch: f64,
}

impl Clock {
    const COMP: Clock = Clock { start: Tick::ZERO, stretch: 100.0 };

    fn of(layer: &Layer) -> Clock {
        Clock { start: layer.start, stretch: layer.stretch }
    }

    fn own(self, t: Tick) -> Tick {
        if self.stretch == 100.0 { t - self.start } else { Tick(((t - self.start).0 as f64 * 100.0 / self.stretch).round() as i64) }
    }

    fn comp(self, t: Tick) -> Tick {
        if self.stretch == 100.0 { self.start + t } else { self.start + Tick((t.0 as f64 * self.stretch / 100.0).round() as i64) }
    }
}

/// The markers the parameters point at (a composition's or a layer's), their clock, and half a
/// frame.
fn markers<'a>(cx: &'a mut Ctx, a: &Args) -> Result<(&'a mut Vec<Marker>, Clock, Tick)> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let half = Tick(comp.frame_rate.frame_duration().0 / 2);
    let layer = a.json("layer").map(|j| find_layer(comp, j)).transpose()?;
    let comp = cx.project.comp_mut(cid).expect("found");
    Ok(match layer {
        None => (&mut comp.markers, Clock::COMP, half),
        Some(id) => {
            let l = comp.layer_mut(id).expect("found");
            let clock = Clock::of(l);
            (&mut l.markers, clock, half)
        }
    })
}

fn marker_json(i: usize, m: &Marker, clock: Clock) -> Json {
    let mut o = Map::new();
    o.insert("index", i as f64);
    if let Json::Object(fields) = m.to_json() {
        for (k, v) in fields.iter() {
            o.insert(k, v.clone());
        }
    }
    o.insert("t", m.time.seconds());
    o.insert("time", clock.comp(m.time).seconds());
    Json::Object(o)
}

fn marker_add(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let view = cx.view.time;
    let (list, clock, half) = markers(cx, a)?;
    let at = clock.own(a.time("time").unwrap_or(view));
    if list.iter().any(|m| (m.time - at).abs() < half) {
        return Err(Error::refused("there is a marker at that time already"));
    }
    let mut m = Marker { time: at, ..Marker::default() };
    fields(&mut m, a)?;
    list.push(m);
    list.sort_by_key(|m| m.time);
    let index = list.iter().position(|m| m.time == at).expect("just added");
    cx.label = Some("Add Marker".into());
    Ok(Json::object([("time", Json::Number(a.time("time").unwrap_or(view).seconds())), ("index", Json::Number(index as f64))]))
}

/// A marker given what the parameters say of it (all but its time).
fn fields(m: &mut Marker, a: &Args) -> Result<()> {
    if let Some(d) = a.time("duration") {
        if d < Tick::ZERO {
            return Err(Error::params("`duration` cannot be below zero"));
        }
        m.duration = d;
    }
    for (key, field) in [("comment", &mut m.comment), ("chapter", &mut m.chapter), ("url", &mut m.url), ("frameTarget", &mut m.frame_target)] {
        if let Some(t) = a.text(key) {
            *field = t.to_string();
        }
    }
    if let Some(l) = a.text("label") {
        m.label = Label::from_id(l).unwrap_or_default();
    }
    if let Some(p) = a.bool("protected") {
        m.protected = p;
    }
    match a.json("cuePoint") {
        Some(Json::Null) => m.cue = None,
        Some(j) => m.cue = Some(veymelo_project::CuePoint::from_json(j, "cuePoint").map_err(|e| Error::params(e.to_string()))?),
        None => {}
    }
    Ok(())
}

/// Where the marker near composition time `t` is in `list`, or what is wrong.
fn marker_at(list: &[Marker], clock: Clock, t: Tick, half: Tick) -> Result<usize> {
    let at = clock.own(t);
    list.iter().position(|m| (m.time - at).abs() <= half).ok_or_else(|| {
        let there: Vec<String> = list.iter().map(|m| format!("{}", clock.comp(m.time).seconds())).collect();
        let there = if there.is_empty() { "there are none".to_string() } else { format!("there are markers at {} s", there.join(", ")) };
        Error::not_found(format!("there is no marker at {} s ({there})", t.seconds()))
    })
}

fn marker_set(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (list, clock, half) = markers(cx, a)?;
    let i = marker_at(list, clock, a.time("time").unwrap_or(Tick::ZERO), half)?;
    let mut m = list.remove(i);
    if let Some(to) = a.time("to") {
        let at = clock.own(to);
        if list.iter().any(|o| (o.time - at).abs() < half) {
            list.insert(i, m);
            return Err(Error::refused(format!("there is a marker at {} s already", to.seconds())));
        }
        m.time = at;
    }
    if let Err(e) = fields(&mut m, a) {
        list.insert(i, m);
        return Err(e);
    }
    let at = m.time;
    list.push(m);
    list.sort_by_key(|m| m.time);
    let index = list.iter().position(|m| m.time == at).expect("put back");
    cx.label = Some("Marker Settings".into());
    Ok(Json::object([("time", Json::Number(clock.comp(at).seconds())), ("index", Json::Number(index as f64))]))
}

fn marker_convert(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let half = Tick(comp.frame_rate.frame_duration().0 / 2);
    let t = a.time("time").unwrap_or(Tick::ZERO);
    let (from, to) = match (a.json("layer"), a.json("toLayer")) {
        (Some(l), None) => (Some(find_layer(comp, l)?), None),
        (None, Some(l)) => (None, Some(find_layer(comp, l)?)),
        _ => return Err(Error::params("give `layer` (its marker becomes the composition's) or `toLayer` (the composition's marker goes to it)")),
    };
    let comp = cx.project.comp_mut(cid).expect("found");
    let (moved, at) = match from {
        Some(id) => {
            let l = comp.layer_mut(id).expect("found");
            let clock = Clock::of(l);
            let i = marker_at(&l.markers, clock, t, half)?;
            let m = l.markers.remove(i);
            let at = clock.comp(m.time);
            (m, at)
        }
        None => {
            let i = marker_at(&comp.markers, Clock::COMP, t, half)?;
            let m = comp.markers.remove(i);
            let at = m.time;
            (m, at)
        }
    };
    let (list, clock, name) = match to {
        Some(id) => {
            let l = comp.layer_mut(id).expect("found");
            let clock = Clock::of(l);
            let name = l.name.clone();
            (&mut l.markers, clock, Json::from(name))
        }
        None => (&mut comp.markers, Clock::COMP, Json::from("composition")),
    };
    let own = clock.own(at);
    if list.iter().any(|m| (m.time - own).abs() < half) {
        return Err(Error::refused(format!("there is a marker at {} s there already", at.seconds())));
    }
    list.push(Marker { time: own, ..moved });
    list.sort_by_key(|m| m.time);
    cx.label = Some(if to.is_some() { "Convert to Layer Marker" } else { "Convert to Composition Marker" }.into());
    Ok(Json::object([("time", Json::Number(at.seconds())), ("to", name)]))
}

/// The markers of the composition `layer` shows, each with the composition time it comes at
/// in `comp` (while the layer is on) and its place in its own list.
fn nested_markers<'p>(project: &'p veymelo_project::Project, cid: u64, layer: &Layer) -> Vec<(Tick, usize, &'p Marker)> {
    let Source::Comp(inner) = layer.source else { return Vec::new() };
    let (Some(comp), Some(nested)) = (project.comp(cid), project.comp(inner)) else { return Vec::new() };
    let on = |t: Tick| t >= layer.in_point && t < layer.out_point;
    if !layer.time_remap {
        return nested.markers.iter().enumerate().map(|(i, m)| (layer.comp_time(m.time), i, m)).filter(|(t, ..)| on(*t)).collect();
    }
    // remapped: each marker at the first frame that shows its moment
    let e = veymelo_expr::Eval::new(project);
    let at = e.place(comp, layer);
    let frame = comp.frame_rate.frame_duration();
    let n = ((layer.out_point - layer.in_point).0 / frame.0).clamp(0, 100_000);
    let shown: Vec<(Tick, Tick)> = (0..n).map(|k| layer.in_point + Tick(k * frame.0)).map(|t| (t, e.source_time(at, t))).collect();
    let near = Tick(nested.frame_rate.frame_duration().0 / 1000);
    nested.markers.iter().enumerate().filter_map(|(i, m)| shown.iter().find(|(_, s)| *s + near >= m.time).map(|(t, _)| (*t, i, m))).collect()
}

fn marker_nested(l: &Look, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(l.project, l.view, a)?;
    let layer = l.project.comp(cid).expect("found").layer(lid).expect("found");
    if !matches!(layer.source, Source::Comp(_)) {
        return Err(Error::refused(format!("\"{}\" does not show a composition", layer.name)));
    }
    Ok(Json::array(nested_markers(l.project, cid, layer).into_iter().map(|(t, i, m)| {
        let mut o = Map::new();
        if let Json::Object(fields) = m.to_json() {
            for (k, v) in fields.iter().filter(|(k, _)| *k != "t") {
                o.insert(k, v.clone());
            }
        }
        o.insert("time", t.seconds());
        o.insert("nestedTime", m.time.seconds());
        o.insert("index", i as f64);
        Json::Object(o)
    })))
}

fn marker_from_source(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let half = Tick(comp.frame_rate.frame_duration().0 / 2);
    let mut adds: Vec<(u64, Vec<Marker>)> = Vec::new();
    for j in a.json("layers").and_then(Json::as_array).cloned().unwrap_or_default() {
        let id = find_layer(comp, &j)?;
        let layer = comp.layer(id).expect("found");
        if !matches!(layer.source, Source::Comp(_)) {
            return Err(Error::refused(format!("\"{}\" does not show a composition", layer.name)));
        }
        // where each comes on the layer, in the layer's own time
        let found = nested_markers(cx.project, cid, layer).into_iter().map(|(t, _, m)| Marker { time: layer.layer_time(t), ..m.clone() }).collect();
        adds.push((id, found));
    }
    let comp = cx.project.comp_mut(cid).expect("found");
    let mut added = 0;
    for (id, found) in adds {
        let l = comp.layer_mut(id).expect("found");
        for m in found {
            if !l.markers.iter().any(|o| (o.time - m.time).abs() < half) {
                l.markers.push(m);
                added += 1;
            }
        }
        l.markers.sort_by_key(|m| m.time);
    }
    cx.label = Some("Update Markers From Source".into());
    Ok(Json::object([("added", Json::Number(added as f64))]))
}

fn marker_list(l: &Look, a: &Args) -> Result<Json> {
    let cid = comp_id(l.project, l.view, a)?;
    let comp = l.project.comp(cid).expect("found");
    let (list, clock) = match a.json("layer") {
        None => (&comp.markers, Clock::COMP),
        Some(j) => {
            let layer = comp.layer(find_layer(comp, j)?).expect("found");
            (&layer.markers, Clock::of(layer))
        }
    };
    Ok(Json::array(list.iter().enumerate().map(|(i, m)| marker_json(i, m, clock))))
}

fn marker_remove(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (all, time) = (a.bool("all") == Some(true), a.time("time"));
    let (list, clock, half) = markers(cx, a)?;
    if all {
        let n = list.len();
        list.clear();
        cx.label = Some("Delete All Markers".into());
        return Ok(Json::object([("removed", Json::Number(n as f64))]));
    }
    let i = marker_at(list, clock, time.ok_or_else(|| Error::params("give `time` (the marker's), or `all`"))?, half)?;
    list.remove(i);
    cx.label = Some("Remove Marker".into());
    Ok(Json::object([("removed", Json::Number(i as f64))]))
}

//! Editing in time and space: a layer's content slipped under its in and out, a layer stretched
//! so its in or out lands at a time, held on its last frame; a composition trimmed to its work
//! area, the work area lifted out of layers (leaving a gap) or extracted (closing it), a
//! composition cropped to a region or to a layer.

use veymelo_json::Value as Json;
use veymelo_keyframe::{Keyframe, Value};
use veymelo_project::{ItemKind, Layer, Source};
use veymelo_time::Tick;

use super::{COMP, Command, LAYER, Run, comp_id, find_layer, layer_ids};
use crate::{Args, Ctx, Error, Param, Result, Ty};

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "layer.slip",
            label: "Slip Edit",
            summary: "Move what a composition or footage layer shows under its in and out points: its content (and keys) later or earlier, the layer staying where it is, as far as its content reaches.",
            params: const { &[COMP, LAYER, Param::need("by", Ty::Number, "seconds to move the content (above zero: later)")] },
            returns: "{layer, start, moved}",
            run: Run::Edit(slip),
        },
        Command {
            id: "layer.stretchTo",
            label: "Stretch to Time",
            summary: "Stretch a layer so its in point lands at `in` (its out point staying) or its out point lands at `out` (its in point staying): what it shows sped up or slowed down to fit.",
            params: const { &[COMP, LAYER, Param::opt("in", Ty::Time, "where its in point goes"), Param::opt("out", Ty::Time, "where its out point goes")] },
            returns: "{layer, in, out, stretch}",
            run: Run::Edit(stretch_to),
        },
        Command {
            id: "layer.freezeLast",
            label: "Freeze on Last Frame",
            summary: "Let a composition or footage layer play as it does, then hold its last frame to the composition's end (time remapping: a held key at its last frame).",
            params: const { &[COMP, LAYER] },
            returns: "{layer, held, out}",
            run: Run::Edit(freeze_last),
        },
        Command {
            id: "comp.trimToWorkArea",
            label: "Trim Comp to Work Area",
            summary: "Make a composition as long as its work area: everything moved so the work area starts at 0, what falls outside it cut off.",
            params: const { &[COMP] },
            returns: "{comp, duration}",
            run: Run::Edit(trim_to_work_area),
        },
        Command {
            id: "comp.liftWorkArea",
            label: "Lift Work Area",
            summary: "Take the work area's stretch of time out of layers (all, or those given), leaving a gap: a layer across it is cut in two.",
            params: const { &[COMP, Param::opt("layers", Ty::List, "only these layers")] },
            returns: "{removed, cut, layers}",
            run: Run::Edit(lift),
        },
        Command {
            id: "comp.extractWorkArea",
            label: "Extract Work Area",
            summary: "Take the work area's stretch of time out of layers (all, or those given) and close the gap: what came after moves earlier by its length.",
            params: const { &[COMP, Param::opt("layers", Ty::List, "only these layers")] },
            returns: "{removed, cut, layers}",
            run: Run::Edit(extract),
        },
        Command {
            id: "comp.crop",
            label: "Crop Comp",
            summary: "Make a composition the size of a region of it ([x, y, width, height]) or of a layer's bounds at a time; its layers moved so they stay where they were in the picture.",
            params: const { &[COMP, Param::opt("rect", Ty::List, "[x, y, width, height] in its pixels"), Param::opt("layer", Ty::Layer, "crop to this layer's bounds"), Param::opt("time", Ty::Time, "when to take the layer's bounds (default: the view's time)")] },
            returns: "{comp, width, height}",
            run: Run::Edit(crop),
        },
    ]
}

/// How long what a layer shows lasts in its own time (none: it shows no time of its own).
fn content_length(cx: &Ctx, l: &Layer) -> Option<Tick> {
    match &l.source {
        Source::Comp(id) => cx.project.comp(*id).map(|c| c.duration),
        Source::Footage(id) => match cx.project.item(*id).map(|i| &i.kind) {
            Some(ItemKind::Footage(f)) if f.kind != veymelo_project::FootageKind::Image => Some(Tick(f.duration.0.saturating_mul(f.loops.max(1) as i64))),
            _ => None,
        },
        _ => None,
    }
}

fn slip(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, id) = layer_ids(cx.project, cx.view, a)?;
    let rate = cx.project.comp(cid).expect("found").frame_rate;
    let l = cx.project.comp(cid).expect("found").layer(id).expect("found").clone();
    if l.time_remap {
        return Err(Error::refused("its time is remapped; change the remap's keys instead"));
    }
    let length = content_length(cx, &l).ok_or_else(|| Error::refused(format!("\"{}\" shows no time of its own to slip", l.name)))?;
    let by = rate.snap_nearest(Tick::from_seconds(a.num("by").unwrap_or(0.0)));
    // the content must still cover the in and out points
    let (s0, s1) = (l.comp_time(Tick::ZERO), l.comp_time(length));
    let (lo, hi) = (s0.min(s1), s0.max(s1));
    let (least, most) = (l.out_point - hi, l.in_point - lo);
    let d = by.max(least).min(most);
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(id).expect("found");
    layer.start += d;
    cx.label = Some(format!("Slip Edit {}", layer.name));
    Ok(Json::object([("layer", Json::Number(id as f64)), ("start", Json::Number(layer.start.seconds())), ("moved", Json::Number(d.seconds()))]))
}

fn stretch_to(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, id) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(id).expect("found");
    let (old_in, old_out) = (layer.in_point, layer.out_point);
    let (lin, lout) = (layer.layer_time(old_in), layer.layer_time(old_out));
    let (new_in, new_out, pin, lpin) = match (a.time("in"), a.time("out")) {
        (Some(t), None) => (t, old_out, old_out, lout),
        (None, Some(t)) => (old_in, t, old_in, lin),
        _ => return Err(Error::params("give `in` or `out`")),
    };
    if new_out <= new_in {
        return Err(Error::params(format!("the layer would end before it starts ({} s to {} s)", new_in.seconds(), new_out.seconds())));
    }
    let k = (new_out - new_in).seconds() / (old_out - old_in).seconds();
    layer.stretch *= k;
    // the moment that stays shows the same layer time as before
    layer.start = pin - Tick((lpin.0 as f64 * layer.stretch / 100.0).round() as i64);
    (layer.in_point, layer.out_point) = (new_in, new_out);
    cx.label = Some(format!("Stretch {}", layer.name));
    Ok(Json::object([("layer", Json::Number(id as f64)), ("in", Json::Number(new_in.seconds())), ("out", Json::Number(new_out.seconds())), ("stretch", Json::Number(layer.stretch))]))
}

fn freeze_last(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, id) = layer_ids(cx.project, cx.view, a)?;
    let (rate, end) = {
        let c = cx.project.comp(cid).expect("found");
        (c.frame_rate, c.duration)
    };
    let l = cx.project.comp(cid).expect("found").layer(id).expect("found").clone();
    let length = content_length(cx, &l).ok_or_else(|| Error::refused(format!("\"{}\" shows no time of its own to freeze", l.name)))?;
    // its last frame: where its content ends, or its out point if that comes first
    let content_end = l.comp_time(length).max(l.comp_time(Tick::ZERO));
    let last = (l.out_point.min(content_end) - rate.frame_duration()).max(l.in_point);
    let (held, from) = (l.shown_time(cx.project, last), l.shown_time(cx.project, l.in_point));
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(id).expect("found");
    let first = Keyframe::new(layer.layer_time(layer.in_point), Value::Number(from.seconds()));
    let hold = Keyframe::new(layer.layer_time(last), Value::Number(held.seconds())).hold();
    layer.time_remap = true;
    let keys = if first.time < hold.time { vec![first, hold] } else { vec![hold] };
    layer.props.prop_mut("timeRemap").map_err(Error::not_found)?.set_keys(keys).map_err(Error::params)?;
    layer.out_point = end.max(layer.out_point);
    cx.label = Some("Freeze on Last Frame".into());
    Ok(Json::object([("layer", Json::Number(id as f64)), ("held", Json::Number(held.seconds())), ("out", Json::Number(layer.out_point.seconds()))]))
}

fn shift(l: &mut Layer, d: Tick) {
    (l.start, l.in_point, l.out_point) = (l.start + d, l.in_point + d, l.out_point + d);
}

fn trim_to_work_area(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let comp = cx.project.comp_mut(cid).expect("found");
    let (ws, we) = (comp.work_area.start, comp.work_area.end());
    let len = we - ws;
    for l in &mut comp.layers {
        shift(l, Tick::ZERO - ws);
        l.in_point = l.in_point.max(Tick::ZERO);
        l.out_point = l.out_point.min(len);
    }
    // layers left with no time at all go
    comp.layers.retain(|l| l.out_point > l.in_point);
    for m in &mut comp.markers {
        m.time -= ws;
    }
    comp.markers.retain(|m| m.time >= Tick::ZERO && m.time < len);
    comp.duration = len;
    comp.work_area = veymelo_time::TimeRange::new(Tick::ZERO, len);
    cx.label = Some("Trim Comp to Work Area".into());
    Ok(Json::object([("comp", Json::Number(cid as f64)), ("duration", Json::Number(len.seconds()))]))
}

/// The work area taken out of layers; with `close`, what came after moved up.
fn take_out(cx: &mut Ctx, a: &Args, close: bool) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let only: Option<Vec<u64>> = match a.json("layers") {
        Some(list) => {
            let comp = cx.project.comp(cid).expect("found");
            Some(list.as_array().cloned().unwrap_or_default().iter().map(|j| find_layer(comp, j)).collect::<Result<_>>()?)
        }
        None => None,
    };
    let (ws, we) = {
        let c = cx.project.comp(cid).expect("found");
        (c.work_area.start, c.work_area.end())
    };
    let len = we - ws;
    let originals = cx.project.comp(cid).expect("found").layers.clone();
    let (mut removed, mut cut) = (0, 0);
    let mut out: Vec<Layer> = Vec::new();
    for l in originals {
        if !only.as_ref().is_none_or(|o| o.contains(&l.id)) {
            out.push(l);
            continue;
        }
        let mut l = l;
        if l.out_point <= ws {
            out.push(l);
        } else if l.in_point >= we {
            if close {
                shift(&mut l, Tick::ZERO - len);
            }
            out.push(l);
        } else if l.in_point >= ws && l.out_point <= we {
            removed += 1;
        } else if l.in_point < ws && l.out_point > we {
            // cut in two: the first part ends at the work area, a copy starts after it
            let name = l.name.clone();
            let mut after = super::layer::copy_of(cx.project, &l, name);
            after.in_point = we;
            if close {
                shift(&mut after, Tick::ZERO - len);
            }
            l.out_point = ws;
            out.push(after);
            out.push(l);
            cut += 1;
        } else if l.in_point < ws {
            l.out_point = ws;
            out.push(l);
        } else {
            l.in_point = we;
            if close {
                shift(&mut l, Tick::ZERO - len);
            }
            out.push(l);
        }
    }
    let n = out.len();
    cx.project.comp_mut(cid).expect("found").layers = out;
    cx.label = Some(if close { "Extract Work Area" } else { "Lift Work Area" }.into());
    Ok(Json::object([("removed", Json::Number(removed as f64)), ("cut", Json::Number(cut as f64)), ("layers", Json::Number(n as f64))]))
}

fn lift(cx: &mut Ctx, a: &Args) -> Result<Json> {
    take_out(cx, a, false)
}

fn extract(cx: &mut Ctx, a: &Args) -> Result<Json> {
    take_out(cx, a, true)
}

fn crop(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let rect = match (a.json("rect"), a.json("layer")) {
        (Some(r), None) => {
            let v: Vec<f64> = r.as_array().map(|x| x.iter().filter_map(Json::as_f64).collect()).unwrap_or_default();
            if v.len() != 4 || v[2] < 1.0 || v[3] < 1.0 {
                return Err(Error::params("`rect` is [x, y, width, height], at least a pixel each way"));
            }
            [v[0], v[1], v[2], v[3]]
        }
        (None, Some(j)) => {
            let t = a.time("time").unwrap_or(cx.view.time);
            let comp = cx.project.comp(cid).expect("found");
            let lid = find_layer(comp, j)?;
            // the layer's bounds: where its pixels are drawn at that time
            let mut solo = cx.project.clone();
            if let Some(ItemKind::Comp(c)) = solo.item_mut(cid).map(|i| &mut i.kind) {
                for l in &mut std::sync::Arc::make_mut(c).layers {
                    l.switches.video = l.id == lid;
                }
            }
            let r = veymelo_render::render(&solo, cid, t, &veymelo_render::Options { background: false, ..Default::default() }).map_err(Error::refused)?;
            let img = &r.image;
            let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
            for y in 0..img.height {
                for x in 0..img.width {
                    if img.px[y * img.width + x][3] > 1e-3 {
                        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                    }
                }
            }
            if x0 == usize::MAX {
                return Err(Error::refused("the layer draws nothing then"));
            }
            [x0 as f64, y0 as f64, (x1 - x0 + 1) as f64, (y1 - y0 + 1) as f64]
        }
        _ => return Err(Error::params("give `rect` or `layer`")),
    };
    let comp = cx.project.comp_mut(cid).expect("found");
    let [x, y, w, h] = rect;
    for l in comp.layers.iter_mut().filter(|l| l.parent.is_none()) {
        let move_point = |l: &mut Layer, path: &str| -> Result<()> {
            let Ok(p) = l.props.prop_mut(path) else { return Ok(()) };
            let shift = |v: &Value| match v {
                Value::Vec3(q) => Value::Vec3([q[0] - x, q[1] - y, q[2]]),
                Value::Vec2(q) => Value::Vec2([q[0] - x, q[1] - y]),
                v => v.clone(),
            };
            if p.keys.is_empty() {
                let v = shift(&p.value);
                p.set_value(v).map_err(Error::params)
            } else {
                let keys: Vec<Keyframe> = p.keys.iter().map(|k| Keyframe { value: shift(&k.value), ..k.clone() }).collect();
                p.set_keys(keys).map_err(Error::params)
            }
        };
        if l.separate_position {
            for (path, d) in [("transform/positionX", x), ("transform/positionY", y)] {
                if let Ok(p) = l.props.prop_mut(path) {
                    if p.keys.is_empty() {
                        let v = p.value.as_f64() - d;
                        p.set_value(Value::Number(v)).map_err(Error::params)?;
                    } else {
                        let keys: Vec<Keyframe> = p.keys.iter().map(|k| Keyframe { value: Value::Number(k.value.as_f64() - d), ..k.clone() }).collect();
                        p.set_keys(keys).map_err(Error::params)?;
                    }
                }
            }
        } else {
            move_point(l, "transform/position")?;
        }
        move_point(l, "transform/pointOfInterest")?;
    }
    (comp.width, comp.height) = (w.round().max(1.0) as u32, h.round().max(1.0) as u32);
    let (cw, ch) = (comp.width, comp.height);
    cx.label = Some("Crop Comp".into());
    Ok(Json::object([("comp", Json::Number(cid as f64)), ("width", Json::Number(cw as f64)), ("height", Json::Number(ch as f64))]))
}

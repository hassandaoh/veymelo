//! Commands about compositions.

use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::{Kind, Value};
use veymelo_project::{Comp, MAX_SECONDS, MAX_SIZE, fps_json};
use veymelo_time::{FrameRate, Tick, TimeRange};

use super::{COMP, Command, Run, comp_id, fresh_name};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "comp.new",
            label: "New Composition",
            summary: "Make a composition and work in it from now on.",
            params: const { &[
                Param::opt("name", Ty::Text, "what to call it (default: Comp 1, Comp 2…)"),
                Param::opt("width", Ty::Whole, "its width in pixels (default 1920)"),
                Param::opt("height", Ty::Whole, "its height in pixels (default 1080)"),
                Param::opt("fps", Ty::Fps, "frames a second (default 30)"),
                Param::opt("duration", Ty::Time, "how long it lasts (default 10 s)"),
                Param::opt("background", Ty::Color, "the colour behind its layers (default black)"),
                Param::opt("pixelAspect", Ty::Number, "a pixel's width for its height (default 1)"),
                Param::opt("preset", Ty::Text, "start from this composition preset (queue.templates lists them): what else is given changes it"),
                Param::opt("folder", Ty::Text, "where saved presets are kept (default: $VEYMELO_TEMPLATES, or the program's Application Support folder)"),
            ] },
            returns: "{comp, name}",
            run: Run::Edit(comp_new),
        },
        Command {
            id: "comp.responsiveTime",
            label: "Responsive Design – Time",
            summary: "Keep a stretch of a composition at its own speed when a layer showing it is stretched (the rest takes all of the stretch): an intro (from the start to the view's time), an outro (from the view's time to the end), or its work area. It is kept as a protected marker.",
            params: const { &[COMP, Param::need("op", Ty::Choice(&["intro", "outro", "workArea"]), "which stretch")] },
            returns: "{start, end}",
            run: Run::Edit(responsive_time),
        },
        Command {
            id: "comp.set",
            label: "Composition Settings",
            summary: "Change a composition's name, size, frame rate, length, background or work area.",
            params: const { &[
                COMP,
                Param::opt("name", Ty::Text, "what to call it"),
                Param::opt("width", Ty::Whole, "its width in pixels"),
                Param::opt("height", Ty::Whole, "its height in pixels"),
                Param::opt("fps", Ty::Fps, "frames a second"),
                Param::opt("duration", Ty::Time, "how long it lasts (layers that go on longer keep their length)"),
                Param::opt("background", Ty::Color, "the colour behind its layers"),
                Param::opt("pixelAspect", Ty::Number, "a pixel's width for its height"),
                Param::opt("start", Ty::Time, "what its first frame's time is called"),
                Param::opt("workArea", Ty::List, "[start, end] in seconds: what previews and renders by default"),
                Param::opt("posterTime", Ty::Time, "the frame that stands for it (a thumbnail)"),
                Param::opt("motionBlur", Ty::Bool, "blur the layers whose motion blur switch is on (default true)"),
                Param::opt("shutterAngle", Ty::Number, "how long the shutter is open, in degrees of a frame: 360 is the whole frame (default 180)"),
                Param::opt("shutterPhase", Ty::Number, "when it opens, in degrees from the frame's time (default -90, so the blur is centred)"),
                Param::opt("samples", Ty::Whole, "the fewest pictures a motion blur is made of, 2 to 64 (default 16)"),
                Param::opt("adaptiveLimit", Ty::Whole, "the most pictures, for layers that move far, 16 to 256 (default 128)"),
                Param::opt("frameBlending", Ty::Bool, "blend frames of layers whose frame blending switch is on (default true)"),
                Param::opt("hideShy", Ty::Bool, "leave shy layers out of the layer list"),
                Param::opt("lightBounce", Ty::Number, "how much light bounces from surface to surface in its 3D scenes, in percent (global illumination: a red wall tints the floor by it, shadows fill with the light round them); 0 (the default) none, 100 all of it"),
                Param::opt("bounces", Ty::Whole, "how many times light bounces on, 1 to 4 (default 2)"),
            ] },
            returns: "{comp}",
            run: Run::Edit(comp_set),
        },
        Command {
            id: "comp.info",
            label: "Composition Info",
            summary: "A composition's settings and its layers, top first.",
            params: const { &[COMP] },
            returns: "{comp, name, width, height, fps, duration, frames, background, layers: [{index, id, name, type, in, out, parent?}]}",
            run: Run::Look(comp_info),
        },
        Command {
            id: "comp.remove",
            label: "Remove Composition",
            summary: "Take a composition out of the project (refused while a layer shows it).",
            params: const { &[Param::need("comp", Ty::Comp, "the composition")] },
            returns: "{removed}",
            run: Run::Edit(comp_remove),
        },
    ]
}

fn size(a: &Args, key: &str, default: u32) -> Result<u32> {
    match a.whole(key) {
        None => Ok(default),
        Some(v) if (1..=MAX_SIZE as u64).contains(&v) => Ok(v as u32),
        Some(v) => Err(Error::params(format!("`{key}` is from 1 to {MAX_SIZE} pixels, not {v}"))),
    }
}

fn duration(a: &Args, default: Tick) -> Result<Tick> {
    match a.time("duration") {
        None => Ok(default),
        Some(d) if d > Tick::ZERO && d.seconds() <= MAX_SECONDS => Ok(d),
        Some(d) => Err(Error::params(format!("`duration` is more than 0 and at most {MAX_SECONDS} s, not {}", d.seconds()))),
    }
}

fn aspect(a: &Args, default: f64) -> Result<f64> {
    match a.num("pixelAspect") {
        None => Ok(default),
        Some(v) if (0.01..=100.0).contains(&v) => Ok(v),
        Some(v) => Err(Error::params(format!("`pixelAspect` is from 0.01 to 100, not {v}"))),
    }
}

fn comp_new(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let base = match a.text("preset") {
        Some(p) => super::templates::comp_preset(a, p)?,
        None => super::templates::CompPreset { width: 1920, height: 1080, fps: FrameRate::FPS_30, duration: Tick::from_seconds(10.0), pixel_aspect: 1.0 },
    };
    let mut c = Comp::new(size(a, "width", base.width)?, size(a, "height", base.height)?, a.fps("fps").unwrap_or(base.fps), duration(a, base.duration)?);
    c.pixel_aspect = aspect(a, base.pixel_aspect)?;
    if let Some(b) = a.color("background") {
        c.background = b.with_alpha(1.0);
    }
    let name = match a.text("name") {
        Some(n) => n.to_string(),
        None => fresh_name("Comp", cx.project.items.iter().map(|i| i.name.as_str())),
    };
    let id = cx.project.add_comp(&name, c);
    cx.view.comp = Some(id);
    cx.view.time = Tick::ZERO;
    cx.label = Some(format!("New Composition {name}"));
    Ok(Json::object([("comp", Json::Number(id as f64)), ("name", Json::from(name))]))
}

fn comp_set(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let id = comp_id(cx.project, cx.view, a)?;
    if let Some(name) = a.text("name") {
        cx.project.item_mut(id).expect("found").name = name.to_string();
    }
    let c = cx.project.comp_mut(id).expect("found");
    c.width = size(a, "width", c.width)?;
    c.height = size(a, "height", c.height)?;
    if let Some(r) = a.fps("fps") {
        c.frame_rate = r;
    }
    let was = c.duration;
    c.duration = duration(a, c.duration)?;
    if let Some(b) = a.color("background") {
        c.background = b.with_alpha(1.0);
    }
    c.pixel_aspect = aspect(a, c.pixel_aspect)?;
    if let Some(t) = a.time("start") {
        c.start = t;
    }
    if let Some(t) = a.time("posterTime") {
        if t < Tick::ZERO || t >= c.duration {
            return Err(Error::params(format!("`posterTime` is from 0 to before {} s", c.duration.seconds())));
        }
        c.poster = t;
    } else if c.poster >= c.duration {
        c.poster = Tick::ZERO;
    }
    if let Some(w) = a.json("workArea") {
        let pair = w.as_array().filter(|p| p.len() == 2).and_then(|p| Some((Tick::from_seconds(p[0].as_f64()?), Tick::from_seconds(p[1].as_f64()?))));
        match pair {
            Some((s, e)) if Tick::ZERO <= s && s < e && e <= c.duration => c.work_area = TimeRange::from_bounds(s, e),
            _ => return Err(Error::params(format!("`workArea` is [start, end] in seconds, from 0 to {} s", c.duration.seconds()))),
        }
    } else if c.work_area == TimeRange::new(Tick::ZERO, was) {
        // a work area that was the whole composition stays the whole of it
        c.work_area = TimeRange::new(Tick::ZERO, c.duration);
    } else if c.work_area.end() > c.duration {
        c.work_area = TimeRange::from_bounds(c.work_area.start.min(c.duration - c.frame_rate.frame_duration()).max(Tick::ZERO), c.duration);
    }
    if let Some(on) = a.bool("motionBlur") {
        c.enable_motion_blur = on;
    }
    if let Some(v) = a.num("shutterAngle") {
        if !(0.0..=720.0).contains(&v) {
            return Err(Error::params(format!("`shutterAngle` is 0 to 720 degrees, not {v}")));
        }
        c.motion_blur.angle = v;
    }
    if let Some(v) = a.num("shutterPhase") {
        if !(-360.0..=360.0).contains(&v) {
            return Err(Error::params(format!("`shutterPhase` is -360 to 360 degrees, not {v}")));
        }
        c.motion_blur.phase = v;
    }
    if let Some(v) = a.whole("samples") {
        if !(2..=64).contains(&v) {
            return Err(Error::params(format!("`samples` is 2 to 64, not {v}")));
        }
        c.motion_blur.samples = v as u32;
    }
    if let Some(v) = a.whole("adaptiveLimit") {
        if !(16..=256).contains(&v) {
            return Err(Error::params(format!("`adaptiveLimit` is 16 to 256, not {v}")));
        }
        c.motion_blur.adaptive_limit = v as u32;
    }
    if let Some(on) = a.bool("frameBlending") {
        c.enable_frame_blending = on;
    }
    if let Some(on) = a.bool("hideShy") {
        c.hide_shy = on;
    }
    if let Some(v) = a.num("lightBounce") {
        if !(0.0..=400.0).contains(&v) {
            return Err(Error::params(format!("`lightBounce` is 0 to 400 percent, not {v}")));
        }
        c.bounce.amount = v / 100.0;
    }
    if let Some(v) = a.whole("bounces") {
        if !(1..=veymelo_project::MOST_BOUNCES as u64).contains(&v) {
            return Err(Error::params(format!("`bounces` is 1 to {}, not {v}", veymelo_project::MOST_BOUNCES)));
        }
        c.bounce.bounces = v as u32;
    }
    Ok(Json::object([("comp", id as f64)]))
}

fn comp_info(l: &Look, a: &Args) -> Result<Json> {
    let id = comp_id(l.project, l.view, a)?;
    let (item, c) = (l.project.item(id).expect("found"), l.project.comp(id).expect("found"));
    let layers = c.layers.iter().enumerate().map(|(i, layer)| {
        let mut o = Map::new();
        o.insert("index", (i + 1) as f64);
        o.insert("id", layer.id as f64);
        o.insert("name", layer.name.as_str());
        o.insert("type", layer.source.id());
        o.insert("in", layer.in_point.seconds());
        o.insert("out", layer.out_point.seconds());
        if let Some(p) = layer.parent {
            o.insert("parent", p as f64);
        }
        if !layer.switches.video {
            o.insert("hidden", true);
        }
        Json::Object(o)
    });
    Ok(Json::object([
        ("comp", Json::Number(id as f64)),
        ("name", Json::from(item.name.as_str())),
        ("width", Json::Number(c.width as f64)),
        ("height", Json::Number(c.height as f64)),
        ("fps", fps_json(c.frame_rate)),
        ("duration", Json::Number(c.duration.seconds())),
        ("frames", Json::Number(c.frames() as f64)),
        ("background", Value::Color(c.background).to_json(Kind::Color)),
        ("workArea", Json::array([Json::Number(c.work_area.start.seconds()), Json::Number(c.work_area.end().seconds())])),
        ("posterTime", Json::Number(c.poster.seconds())),
        ("lightBounce", Json::object([("amount", Json::Number(c.bounce.amount * 100.0)), ("bounces", Json::Number(c.bounce.bounces as f64))])),
        ("layers", Json::array(layers)),
    ]))
}

fn comp_remove(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let id = comp_id(cx.project, cx.view, a)?;
    let users: Vec<String> = cx
        .project
        .items
        .iter()
        .filter_map(|i| Some((i, i.as_comp()?)))
        .flat_map(|(i, c)| c.layers.iter().filter(|l| l.source.item() == Some(id) && matches!(l.source, veymelo_project::Source::Comp(_))).map(move |l| format!("\"{}\" in \"{}\"", l.name, i.name)))
        .collect();
    if !users.is_empty() {
        return Err(Error::refused(format!("layers show this composition: {} (remove them first)", users.join(", "))));
    }
    let name = cx.project.item(id).expect("found").name.clone();
    cx.project.items.retain(|i| i.id != id);
    // nothing left to make of it
    cx.project.queue.retain(|q| q.comp != id);
    if cx.view.comp == Some(id) {
        cx.view.comp = cx.project.items.iter().find(|i| i.as_comp().is_some()).map(|i| i.id);
    }
    cx.label = Some(format!("Remove Composition {name}"));
    Ok(Json::object([("removed", id as f64)]))
}

fn responsive_time(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let id = comp_id(cx.project, cx.view, a)?;
    let now = cx.view.time;
    let c = cx.project.comp_mut(id).expect("found");
    let (start, end, note) = match a.text("op").unwrap_or("workArea") {
        "intro" => (Tick::ZERO, now, "Intro"),
        "outro" => (now, c.duration, "Outro"),
        _ => (c.work_area.start, c.work_area.start + c.work_area.duration, "Protected Region"),
    };
    if end <= start {
        return Err(Error::refused("that stretch would be empty (move the view's time first)"));
    }
    c.markers.retain(|m| m.time != start);
    c.markers.push(veymelo_project::Marker { time: start, duration: end - start, comment: note.into(), protected: true, ..Default::default() });
    c.markers.sort_by_key(|m| m.time);
    cx.label = Some(format!("Create {note}"));
    Ok(Json::object([("start", Json::Number(start.seconds())), ("end", Json::Number(end.seconds()))]))
}

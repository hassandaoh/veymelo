//! Commands for a composition's sound: how loud it is (as broadcasters and the apps that play
//! clips measure it), and a picture of it — an agent cannot hear, so it reads and looks.

use std::sync::Arc;

use veymelo_json::{Map, Value as Json};
use veymelo_project::{ItemKind, Project};
use veymelo_render::audio::{self, Loudness};
use veymelo_time::Tick;

use super::{COMP, Command, Run, comp_id, find_layer};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

const SPAN: [Param; 3] = [
    Param::opt("start", Ty::Time, "from when (default 0)"),
    Param::opt("end", Ty::Time, "to when (default: the composition's end)"),
    Param::opt("layer", Ty::Layer, "only this layer's sound (default: the whole mix)"),
];

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "audio.levels",
            label: "Audio Levels",
            summary: "How loud a composition's sound is: integrated loudness and the loudest three seconds in LUFS (ITU-R BS.1770, as broadcasters and clip apps measure; clip apps aim near −14), each channel's peak and mean level in dBFS, and whether it clips.",
            params: const { &[COMP, SPAN[0], SPAN[1], SPAN[2]] },
            returns: "{heard, seconds, integrated?, shortTermMax?, peak: [l, r], rms: [l, r], clips}",
            run: Run::Look(audio_levels),
        },
        Command {
            id: "audio.toKeys",
            label: "Convert Audio to Keyframes",
            summary: "Turn a composition's sound into keys: a null \"Audio Amplitude\" at the top with three sliders (Left Channel, Right Channel, Both Channels) keyed every frame with how loud the sound is then (its square-mean level, 0 silent to 100 full scale), for expressions to follow the sound: thisComp.layer(\"Audio Amplitude\").effect(\"Both Channels\")(\"Slider\").",
            params: const { &[COMP, SPAN[0], SPAN[1], SPAN[2], Param::opt("name", Ty::Text, "the null's name (default Audio Amplitude)")] },
            returns: "{layer, name, keys}",
            run: Run::Edit(audio_to_keys),
        },
        Command {
            id: "audio.waveform",
            label: "Audio Waveform",
            summary: "A picture of a composition's sound written as a PNG: the left channel above the right, lines at 0 and ±6 dB, clipping in red; with its loudness.",
            params: const { &[COMP, SPAN[0], SPAN[1], SPAN[2], Param::opt("width", Ty::Whole, "pixels across (default 1200)"), Param::opt("height", Ty::Whole, "pixels down (default 300)"), Param::need("path", Ty::Text, "the .png file to write")] },
            returns: "{path, width, height, …audio.levels}",
            run: Run::Look(audio_waveform),
        },
    ]
}

/// The sound of a composition (or only one of its layers) over the span asked, at 48 kHz.
pub fn sound(l: &Look, a: &Args) -> Result<(Vec<f32>, f64, bool)> {
    let cid = comp_id(l.project, l.view, a)?;
    let comp = l.project.comp(cid).expect("found");
    let start = a.time("start").unwrap_or(Tick::ZERO);
    let end = a.time("end").unwrap_or(comp.duration);
    if end <= start {
        return Err(Error::params(format!("the span ends after it starts: {} s to {} s", start.seconds(), end.seconds())));
    }
    if (end - start).seconds() > 3600.0 {
        return Err(Error::params("an hour at most at a time"));
    }
    // one layer: the composition with only it soloed
    let alone: Option<Project> = match a.json("layer") {
        Some(j) => {
            let id = find_layer(comp, j)?;
            let mut p = l.project.clone();
            if let Some(ItemKind::Comp(c)) = p.item_mut(cid).map(|i| &mut i.kind) {
                for layer in &mut Arc::make_mut(c).layers {
                    layer.switches.solo = layer.id == id;
                    if layer.id == id {
                        layer.switches.audio = true;
                    }
                }
            }
            Some(p)
        }
        None => None,
    };
    let project = alone.as_ref().unwrap_or(l.project);
    let frames = ((end - start).seconds() * audio::RATE as f64).round() as usize;
    let mut s = Vec::with_capacity(frames * 2);
    let mut done = 0usize;
    while done < frames {
        let n = (audio::RATE as usize).min(frames - done);
        s.extend(audio::mix(project, cid, start + Tick::from_seconds(done as f64 / audio::RATE as f64), n, audio::RATE));
        done += n;
    }
    Ok((s, (end - start).seconds(), audio::heard(project, cid)))
}

fn levels_json(loud: &Loudness, seconds: f64, heard: bool, samples: &[f32]) -> Map {
    let db = |v: f64| if v.is_finite() { Json::Number((v * 100.0).round() / 100.0) } else { Json::Null };
    let mut o = Map::new();
    o.insert("heard", heard);
    o.insert("seconds", seconds);
    if let Some(v) = loud.integrated {
        o.insert("integrated", db(v));
    }
    if let Some(v) = loud.short_max {
        o.insert("shortTermMax", db(v));
    }
    o.insert("peak", Json::array(loud.peak.map(db)));
    o.insert("rms", Json::array(loud.rms.map(db)));
    o.insert("clips", samples.iter().any(|v| v.abs() > 1.0));
    o
}

fn audio_levels(l: &Look, a: &Args) -> Result<Json> {
    let (s, seconds, heard) = sound(l, a)?;
    Ok(Json::Object(levels_json(&audio::loudness(&s, audio::RATE), seconds, heard, &s)))
}

/// What a picture of the sound takes (as `audio.waveform`, without a path).
pub const PICTURE: [Param; 6] = [COMP, SPAN[0], SPAN[1], SPAN[2], Param::opt("width", Ty::Whole, "pixels across (default 1200)"), Param::opt("height", Ty::Whole, "pixels down (default 300)")];

/// A picture of the sound (PNG) and how loud it is.
pub fn picture(l: &Look, a: &Args) -> Result<(Vec<u8>, Json)> {
    let (s, seconds, heard) = sound(l, a)?;
    let (w, h) = (a.whole("width").unwrap_or(1200).clamp(16, 8000) as usize, a.whole("height").unwrap_or(300).clamp(16, 4000) as usize);
    let png = veymelo_render::to_png(&audio::waveform(&s, w, h));
    Ok((png, Json::Object(levels_json(&audio::loudness(&s, audio::RATE), seconds, heard, &s))))
}

fn audio_waveform(l: &Look, a: &Args) -> Result<Json> {
    let (s, seconds, heard) = sound(l, a)?;
    let (w, h) = (a.whole("width").unwrap_or(1200).clamp(16, 8000) as usize, a.whole("height").unwrap_or(300).clamp(16, 4000) as usize);
    let img = audio::waveform(&s, w, h);
    let path = a.text("path").unwrap_or("");
    std::fs::write(path, veymelo_render::to_png(&img)).map_err(|e| Error::not_found(format!("cannot write {path}: {e}")))?;
    let mut o = levels_json(&audio::loudness(&s, audio::RATE), seconds, heard, &s);
    o.insert("path", path);
    o.insert("width", w as f64);
    o.insert("height", h as f64);
    Ok(Json::Object(o))
}

fn audio_to_keys(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let (s, _, heard) = sound(&cx.look(), a)?;
    if !heard {
        return Err(Error::refused("nothing in the composition is heard, so there is nothing to turn into keys"));
    }
    let comp = cx.project.comp(cid).expect("found");
    let (rate, start) = (comp.frame_rate, a.time("start").unwrap_or(Tick::ZERO));
    let first = rate.frame_at(start);
    let per = audio::RATE as f64 / rate.as_f64();
    let frames = ((s.len() / 2) as f64 / per).floor() as usize;
    // each frame's square-mean level, left, right and both
    let mut keys: [Vec<veymelo_keyframe::Keyframe>; 3] = Default::default();
    for f in 0..frames {
        let (a0, a1) = ((f as f64 * per).round() as usize, (((f + 1) as f64 * per).round() as usize).min(s.len() / 2));
        let (mut l, mut r) = (0.0f64, 0.0f64);
        for k in a0..a1 {
            l += (s[k * 2] as f64).powi(2);
            r += (s[k * 2 + 1] as f64).powi(2);
        }
        let n = (a1 - a0).max(1) as f64;
        let (l, r) = ((l / n).sqrt() * 100.0, (r / n).sqrt() * 100.0);
        let both = ((l * l + r * r) / 2.0).sqrt();
        let at = rate.tick_of(first + f as i64);
        for (list, v) in keys.iter_mut().zip([l, r, both]) {
            list.push(veymelo_keyframe::Keyframe::new(at, veymelo_keyframe::Value::Number((v * 1000.0).round() / 1000.0)));
        }
    }
    let name = a.text("name").unwrap_or("Audio Amplitude").to_string();
    let (w, h, duration) = (comp.width as f64, comp.height as f64, comp.duration);
    let mut layer = veymelo_project::Layer::new(cx.project.alloc(), &name, veymelo_project::Source::Null, duration);
    layer.props.prop_mut("transform/position").map_err(Error::refused)?.set_value(veymelo_keyframe::Value::Vec3([w / 2.0, h / 2.0, 0.0])).map_err(Error::refused)?;
    let slider = veymelo_effects::find("sliderControl").expect("defined").def;
    for (title, list) in ["Left Channel", "Right Channel", "Both Channels"].iter().zip(keys) {
        let mut g = veymelo_project::Group::instance(slider, cx.project.alloc(), title);
        g.prop_mut("slider").map_err(Error::refused)?.set_keys(list).map_err(Error::refused)?;
        layer.props.group_mut("effects").map_err(Error::refused)?.children.push(veymelo_project::Node::Group(g));
    }
    let id = layer.id;
    cx.project.comp_mut(cid).expect("found").layers.insert(0, layer);
    cx.label = Some("Convert Audio to Keyframes".into());
    Ok(Json::object([("layer", Json::Number(id as f64)), ("name", Json::from(name)), ("keys", Json::Number(frames as f64))]))
}

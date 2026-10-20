//! A composition as a timeline video editors read, and their timelines made compositions: an
//! edit decision list (EDL), OpenTimelineIO, Final Cut Pro 7 XML (Premiere Pro's too) or FCPXML.
//! Footage layers become clips; a timeline's clips become footage layers (placeholders for files
//! not there, to be found later).

use std::path::Path;

use veymelo_interchange::{Clip, Format, Kind, Media, Rate, Timeline, Track};
use veymelo_json::Value as Json;
use veymelo_project::{Footage, FootageKind, ItemKind, Source};
use veymelo_time::{FrameRate, Tick};

use super::{COMP, Command, Run, comp_id};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "timeline.export",
            label: "Export Timeline",
            summary: "Write a composition as a timeline video editors read: .edl (an edit decision list), .otio (OpenTimelineIO), .xml (Final Cut Pro 7 XML, which Premiere Pro reads) or .fcpxml (Final Cut Pro). Its footage layers become clips, on as few tracks as keep them as they are stacked; what is not footage (solids, words, shapes, compositions) is not a file and is left out, said in the notes.",
            params: const { &[COMP, Param::need("path", Ty::Text, "the file to write"), Param::opt("format", Ty::Choice(Format::IDS), "the kind of file (default: as its name ends)")] },
            returns: "{path, format, clips, tracks, notes}",
            run: Run::Look(export),
        },
        Command {
            id: "timeline.import",
            label: "Import Timeline",
            summary: "Make a timeline from a video editor a composition: each clip a footage layer at its place (picture tracks one above another from the first up, sound tracks after), each file footage once; a file not there comes in as a placeholder of its size and length (footage.find or footage.relink finds it later).",
            params: const {
                &[
                    Param::need("path", Ty::Text, "the timeline file: .edl, .otio, .xml or .fcpxml"),
                    Param::opt("format", Ty::Choice(Format::IDS), "the kind of file (default: as its name ends, or its text says)"),
                    Param::opt("edlFps", Ty::Number, "an EDL's frames a second (it does not say; default 30)"),
                ]
            },
            returns: "{comp, name, layers, missing: [path], notes}",
            run: Run::Edit(import),
        },
    ]
}

fn format_of(a: &Args, path: &str) -> Result<Option<Format>> {
    match a.text("format") {
        Some(f) => Format::from_id(f).map(Some).ok_or_else(|| Error::params(format!("there is no timeline format \"{f}\""))),
        None => Ok(Format::of_path(path)),
    }
}

/// Clips placed on as few tracks as keep their stacking: each (from the bottom up) on the track
/// above the highest of those below it that it overlaps, else the first.
fn pack(kind: Kind, clips: Vec<Clip>) -> Vec<Track> {
    let mut tracks: Vec<Track> = Vec::new();
    let mut placed: Vec<(f64, f64, usize)> = Vec::new();
    for c in clips {
        let (a, b) = (c.at, c.at + c.length);
        let k = placed.iter().filter(|(x, y, _)| *x < b - 1e-9 && a < *y - 1e-9).map(|(_, _, t)| t + 1).max().unwrap_or(0);
        while tracks.len() <= k {
            tracks.push(Track { kind, clips: Vec::new() });
        }
        tracks[k].clips.push(c);
        placed.push((a, b, k));
    }
    for t in &mut tracks {
        t.clips.sort_by(|x, y| x.at.total_cmp(&y.at));
    }
    tracks
}

fn export(l: &Look, a: &Args) -> Result<Json> {
    let cid = comp_id(l.project, l.view, a)?;
    let comp = l.project.comp(cid).expect("found");
    let path = a.text("path").unwrap_or("").to_string();
    let format = format_of(a, &path)?.ok_or_else(|| Error::params(format!("{path}: give `format`, or a name ending in .edl, .otio, .xml or .fcpxml")))?;
    let mut notes = Vec::new();
    let (mut video, mut audio) = (Vec::new(), Vec::new());
    let (mut not_files, mut stretched) = (0, 0);
    // from the bottom layer up
    for layer in comp.layers.iter().rev() {
        let Source::Footage(id) = layer.source else {
            if layer.switches.video && !matches!(layer.source, Source::Camera | Source::Light(_) | Source::Null) {
                not_files += 1;
            }
            continue;
        };
        let Some(item) = l.project.item(id) else { continue };
        let ItemKind::Footage(f) = &item.kind else { continue };
        if layer.stretch != 100.0 || layer.time_remap {
            stretched += 1;
        }
        let at = layer.in_point.max(Tick::ZERO);
        let end = layer.out_point.min(comp.duration);
        if end <= at {
            continue;
        }
        let media = Media { length: if f.kind == FootageKind::Image { 0.0 } else { f.duration.seconds() }, width: f.width, height: f.height, video: f.kind != FootageKind::Audio, audio: f.has_audio || f.kind == FootageKind::Audio };
        let clip = Clip { name: layer.name.clone(), path: f.path.clone(), at: at.seconds(), length: (end - at).seconds(), source_in: layer.layer_time(at).seconds().max(0.0), media: media.clone() };
        if layer.switches.video && media.video {
            video.push(clip.clone());
        }
        if layer.switches.audio && media.audio {
            audio.push(clip);
        }
    }
    if not_files > 0 {
        notes.push(format!("{not_files} layer{} not footage (solids, words, shapes, compositions) left out: render them first (comp.preRender) to bring them along", if not_files == 1 { " is" } else { "s are" }));
    }
    if stretched > 0 {
        notes.push(format!("{stretched} layer{} stretched or time-remapped: written at their own speed", if stretched == 1 { " is" } else { "s are" }));
    }
    let mut tracks = pack(Kind::Video, video);
    tracks.extend(pack(Kind::Audio, audio));
    let clips: usize = tracks.iter().map(|t| t.clips.len()).sum();
    let timeline = Timeline {
        name: l.project.item(cid).map_or("Timeline".into(), |i| i.name.clone()),
        rate: Rate { num: comp.frame_rate.num, den: comp.frame_rate.den },
        width: comp.width,
        height: comp.height,
        tracks,
    };
    let (text, more) = veymelo_interchange::write(&timeline, format);
    notes.extend(more);
    std::fs::write(&path, text).map_err(|e| Error::not_found(format!("cannot write {path}: {e}")))?;
    Ok(Json::object([
        ("path", Json::from(path)),
        ("format", Json::from(format.id())),
        ("clips", Json::Number(clips as f64)),
        ("tracks", Json::Number(timeline.tracks.len() as f64)),
        ("notes", Json::array(notes.into_iter().map(Json::from))),
    ]))
}

fn import(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let path = a.text("path").unwrap_or("").to_string();
    let text = std::fs::read_to_string(&path).map_err(|e| Error::not_found(format!("cannot read {path}: {e}")))?;
    let edl_rate = match a.num("edlFps") {
        Some(f) if f > 0.0 && f <= 1000.0 => Some(Rate::near(f)),
        Some(f) => return Err(Error::params(format!("`edlFps` is above 0 and at most 1000, not {f}"))),
        None => None,
    };
    let (t, mut notes) = veymelo_interchange::read(&text, format_of(a, &path)?, edl_rate).map_err(Error::params)?;
    let rate = FrameRate::new(t.rate.num, t.rate.den).unwrap_or(FrameRate::FPS_30);
    let length = Tick::from_seconds(t.length().max(rate.frame_duration().seconds()));
    let made = super::call(
        cx,
        "comp.new",
        Json::object([("name", Json::from(t.name.as_str())), ("width", Json::Number(t.width.clamp(1, 30000) as f64)), ("height", Json::Number(t.height.clamp(1, 30000) as f64)), ("fps", Json::Number(rate.as_f64())), ("duration", Json::Number(length.seconds()))]),
    )?;
    let cid = made.get("comp").and_then(Json::as_u64).expect("made");
    // each file once: footage, or a placeholder where it is not
    let mut items: Vec<(String, u64)> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for c in t.tracks.iter().flat_map(|k| &k.clips) {
        if items.iter().any(|(p, _)| *p == c.path) {
            continue;
        }
        let name = if c.path.is_empty() { c.name.clone() } else { Path::new(&c.path).file_name().map_or(c.name.clone(), |n| n.to_string_lossy().into_owned()) };
        let id = match veymelo_media::probe(&c.path, false) {
            Ok(f) if !c.path.is_empty() => cx.project.add(&name, None, ItemKind::Footage(f)),
            _ => {
                if !c.path.is_empty() {
                    missing.push(c.path.clone());
                }
                let mut f = Footage::new(&c.path, FootageKind::Placeholder);
                let longest = t.tracks.iter().flat_map(|k| &k.clips).filter(|x| x.path == c.path).map(|x| x.source_in + x.length).fold(c.media.length, f64::max);
                (f.width, f.height, f.frame_rate, f.duration) = (if c.media.width > 0 { c.media.width } else { t.width.max(1) }, if c.media.height > 0 { c.media.height } else { t.height.max(1) }, rate, Tick::from_seconds(longest));
                f.has_audio = c.media.audio;
                cx.project.add(&name, None, ItemKind::Footage(f))
            }
        };
        items.push((c.path.clone(), id));
    }
    // the first picture track at the bottom: each next one above it; sound after
    let mut layers = 0;
    for track in &t.tracks {
        for c in &track.clips {
            let item = items.iter().find(|(p, _)| *p == c.path).map(|(_, id)| *id).expect("made");
            let added = super::call(cx, "layer.add", Json::object([("comp", Json::Number(cid as f64)), ("type", Json::from("footage")), ("item", Json::Number(item as f64)), ("name", Json::from(c.name.as_str()))]))?;
            let lid = added.get("layer").and_then(Json::as_u64).expect("added");
            let layer = cx.project.comp_mut(cid).expect("made").layer_mut(lid).expect("added");
            let (at, length, from) = (Tick::from_seconds(c.at), Tick::from_seconds(c.length), Tick::from_seconds(c.source_in));
            (layer.start, layer.in_point, layer.out_point) = (at - from, at, at + length);
            // a picture track plays pictures, a sound track sound
            match track.kind {
                Kind::Video => layer.switches.audio = false,
                Kind::Audio => layer.switches.video = false,
            }
            layers += 1;
        }
    }
    if !missing.is_empty() {
        notes.push(format!("{} file{} not there: placeholders stand in (footage.find looks for them)", missing.len(), if missing.len() == 1 { " is" } else { "s are" }));
    }
    cx.label = Some(format!("Import Timeline {}", t.name));
    Ok(Json::object([
        ("comp", Json::Number(cid as f64)),
        ("name", Json::from(t.name)),
        ("layers", Json::Number(layers as f64)),
        ("missing", Json::array(missing.into_iter().map(Json::from))),
        ("notes", Json::array(notes.into_iter().map(Json::from))),
    ]))
}

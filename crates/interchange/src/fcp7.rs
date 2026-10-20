//! Final Cut Pro 7 XML (xmeml), which Premiere Pro reads and writes too: a sequence with its
//! rate (a whole timebase, NTSC or not), video and audio tracks of clip items, each with where
//! it is in the sequence (start, end) and in its file (in, out), and the file it shows. Times
//! are frames of the sequence's rate.

use crate::xml::{self, Element, Writer};
use crate::{Clip, Kind, Media, Rate, Timeline, Track, file_name, file_url, path_of};

fn rate_tag(w: &mut Writer, rate: Rate) {
    w.open("rate", &[]);
    w.text("timebase", &rate.timebase().to_string());
    w.text("ntsc", if rate.ntsc() { "TRUE" } else { "FALSE" });
    w.close("rate");
}

pub(crate) fn write(t: &Timeline) -> String {
    let rate = t.rate;
    let mut w = Writer::new();
    w.raw("<!DOCTYPE xmeml>");
    w.open("xmeml", &[("version", "4".into())]);
    w.open("sequence", &[("id", "sequence-1".into())]);
    w.text("name", &t.name);
    w.text("duration", &rate.frames(t.length()).to_string());
    rate_tag(&mut w, rate);
    w.open("media", &[]);
    // each file once, its id given the first time it is written in full
    let mut files: Vec<String> = Vec::new();
    let mut clip_n = 0;
    for kind in [Kind::Video, Kind::Audio] {
        let tag = if kind == Kind::Video { "video" } else { "audio" };
        w.open(tag, &[]);
        if kind == Kind::Video {
            w.open("format", &[]);
            w.open("samplecharacteristics", &[]);
            rate_tag(&mut w, rate);
            w.text("width", &t.width.to_string());
            w.text("height", &t.height.to_string());
            w.close("samplecharacteristics");
            w.close("format");
        }
        for track in t.tracks.iter().filter(|k| k.kind == kind) {
            w.open("track", &[]);
            for c in &track.clips {
                clip_n += 1;
                w.open("clipitem", &[("id", format!("clipitem-{clip_n}"))]);
                w.text("name", &c.name);
                w.text("enabled", "TRUE");
                let media_frames = if c.media.length > 0.0 { rate.frames(c.media.length) } else { rate.frames(c.source_in + c.length) };
                w.text("duration", &media_frames.to_string());
                rate_tag(&mut w, rate);
                w.text("start", &rate.frames(c.at).to_string());
                w.text("end", &rate.frames(c.at + c.length).to_string());
                w.text("in", &rate.frames(c.source_in).to_string());
                w.text("out", &rate.frames(c.source_in + c.length).to_string());
                let known = files.iter().position(|f| *f == c.path);
                let id = format!("file-{}", known.unwrap_or(files.len()) + 1);
                if known.is_some() {
                    w.empty("file", &[("id", id)]);
                } else {
                    files.push(c.path.clone());
                    w.open("file", &[("id", id)]);
                    w.text("name", file_name(&c.path));
                    w.text("pathurl", &file_url(&c.path));
                    rate_tag(&mut w, rate);
                    w.text("duration", &media_frames.to_string());
                    w.open("media", &[]);
                    if c.media.video || kind == Kind::Video {
                        w.open("video", &[]);
                        w.open("samplecharacteristics", &[]);
                        w.text("width", &c.media.width.max(1).to_string());
                        w.text("height", &c.media.height.max(1).to_string());
                        w.close("samplecharacteristics");
                        w.close("video");
                    }
                    if c.media.audio || kind == Kind::Audio {
                        w.open("audio", &[]);
                        w.text("channelcount", "2");
                        w.close("audio");
                    }
                    w.close("media");
                    w.close("file");
                }
                w.close("clipitem");
            }
            w.close("track");
        }
        w.close(tag);
    }
    w.close("media");
    w.close("sequence");
    w.close("xmeml");
    w.out
}

/// A rate tag's rate.
fn rate_of(e: Option<&Element>) -> Option<Rate> {
    let e = e?;
    let base: u32 = e.text_of("timebase")?.parse().ok()?;
    Some(Rate::of(base, e.text_of("ntsc").is_some_and(|n| n.eq_ignore_ascii_case("TRUE"))))
}

pub(crate) fn read(text: &str) -> Result<(Timeline, Vec<String>), String> {
    let root = xml::parse(text)?;
    if root.name != "xmeml" {
        return Err(format!("a Final Cut Pro XML file starts with <xmeml>, not <{}>", root.name));
    }
    let seq = root.descendants().into_iter().find(|e| e.name == "sequence").ok_or("there is no sequence in it")?.clone();
    let mut notes = Vec::new();
    let rate = rate_of(seq.child("rate")).unwrap_or(Rate::FPS_30);
    let frames = |e: &Element, k: &str| e.text_of(k).and_then(|v| v.parse::<i64>().ok());
    // the files given in full, by id
    let mut files: Vec<(String, Element)> = Vec::new();
    for f in root.descendants().into_iter().filter(|e| e.name == "file" && (e.child("pathurl").is_some() || e.child("name").is_some())) {
        if let Some(id) = f.attr("id") {
            files.push((id.to_string(), f.clone()));
        }
    }
    let format = seq.at("media/video/format/samplecharacteristics");
    let size = |k: &str, d: u32| format.and_then(|f| f.text_of(k)).and_then(|v| v.parse().ok()).unwrap_or(d);
    let mut tracks = Vec::new();
    for (tag, kind) in [("video", Kind::Video), ("audio", Kind::Audio)] {
        let Some(media) = seq.at(&format!("media/{tag}")) else { continue };
        for track in media.all("track") {
            let mut clips = Vec::new();
            for item in track.all("clipitem") {
                let (Some(start), Some(end)) = (frames(item, "start"), frames(item, "end")) else { continue };
                if start < 0 || end <= start {
                    // a clip under a transition has -1 here: its place is the transition's
                    notes.push(format!("\"{}\" at a transition is left out", item.text_of("name").unwrap_or("a clip")));
                    continue;
                }
                let r = rate_of(item.child("rate")).unwrap_or(rate);
                let into = frames(item, "in").unwrap_or(0);
                let file = item.child("file").and_then(|f| f.attr("id").and_then(|id| files.iter().find(|(k, _)| k == id)).map(|(_, e)| e.clone()).or_else(|| Some(f.clone())));
                let path = file.as_ref().and_then(|f| f.text_of("pathurl")).map(path_of).unwrap_or_default();
                let fr = file.as_ref().and_then(|f| rate_of(f.child("rate"))).unwrap_or(r);
                let media = Media {
                    length: file.as_ref().and_then(|f| frames(f, "duration")).map_or(0.0, |d| fr.seconds(d)),
                    width: file.as_ref().and_then(|f| f.at("media/video/samplecharacteristics")).and_then(|s| s.text_of("width")).and_then(|v| v.parse().ok()).unwrap_or(0),
                    height: file.as_ref().and_then(|f| f.at("media/video/samplecharacteristics")).and_then(|s| s.text_of("height")).and_then(|v| v.parse().ok()).unwrap_or(0),
                    video: file.as_ref().is_some_and(|f| f.at("media/video").is_some()),
                    audio: file.as_ref().is_some_and(|f| f.at("media/audio").is_some()),
                };
                let name = item.text_of("name").filter(|n| !n.is_empty()).map_or_else(|| file_name(&path).to_string(), str::to_string);
                clips.push(Clip { name, path, at: rate.seconds(start), length: rate.seconds(end - start), source_in: r.seconds(into), media });
            }
            tracks.push(Track { kind, clips });
        }
    }
    let name = seq.text_of("name").unwrap_or("Timeline").to_string();
    Ok((Timeline { name, rate, width: size("width", 1920), height: size("height", 1080), tracks }, notes))
}

//! FCPXML, Final Cut Pro's: resources (the format, and an asset for each file) and a project's
//! sequence whose spine holds clips one after another, other clips hanging on them in lanes
//! above (pictures) or below (sound). Times are fractions of a second ("1001/30000s").
//!
//! Written, the spine is one gap as long as the timeline and every clip hangs on it in the lane
//! of its track: the tracks kept as they are. Read, the spine's clips and every clip hanging on
//! them are placed by their offsets, lane by lane.

use crate::xml::{self, Element, Writer};
use crate::{Clip, Kind, Media, Rate, Timeline, Track, file_name, file_url, path_of};

/// Seconds as FCPXML writes them: frames of the rate over its rate.
fn time(seconds: f64, rate: Rate) -> String {
    let frames = rate.frames(seconds);
    if frames == 0 {
        return "0s".into();
    }
    if rate.den == 1 {
        if frames % rate.num as i64 == 0 { format!("{}s", frames / rate.num as i64) } else { format!("{}/{}s", frames, rate.num) }
    } else {
        format!("{}/{}s", frames * rate.den as i64, rate.num)
    }
}

/// FCPXML's time ("10s", "1001/30000s") in seconds.
fn seconds(t: &str) -> Option<f64> {
    let t = t.trim().strip_suffix('s')?;
    match t.split_once('/') {
        Some((n, d)) => Some(n.parse::<f64>().ok()? / d.parse::<f64>().ok().filter(|d| *d != 0.0)?),
        None => t.parse().ok(),
    }
}

pub(crate) fn write(t: &Timeline) -> String {
    let rate = t.rate;
    let frame = if rate.den == 1 { format!("1/{}s", rate.num) } else { format!("{}/{}s", rate.den, rate.num) };
    let mut w = Writer::new();
    w.raw("<!DOCTYPE fcpxml>");
    w.open("fcpxml", &[("version", "1.9".into())]);
    w.open("resources", &[]);
    w.empty("format", &[("id", "r1".into()), ("frameDuration", frame), ("width", t.width.to_string()), ("height", t.height.to_string())]);
    // an asset for each file
    let mut assets: Vec<String> = Vec::new();
    for c in t.tracks.iter().flat_map(|k| &k.clips) {
        if assets.contains(&c.path) {
            continue;
        }
        assets.push(c.path.clone());
        let id = format!("r{}", assets.len() + 1);
        let length = if c.media.length > 0.0 { c.media.length } else { c.source_in + c.length };
        let has_video = c.media.video || t.tracks.iter().any(|k| k.kind == Kind::Video && k.clips.iter().any(|x| x.path == c.path));
        let has_audio = c.media.audio || t.tracks.iter().any(|k| k.kind == Kind::Audio && k.clips.iter().any(|x| x.path == c.path));
        let name = file_name(&c.path).rsplit_once('.').map_or(file_name(&c.path), |(n, _)| n).to_string();
        w.open(
            "asset",
            &[
                ("id", id),
                ("name", name),
                ("start", "0s".into()),
                ("duration", time(length, rate)),
                ("hasVideo", if has_video { "1" } else { "0" }.into()),
                ("hasAudio", if has_audio { "1" } else { "0" }.into()),
                ("format", "r1".into()),
            ],
        );
        w.empty("media-rep", &[("kind", "original-media".into()), ("src", file_url(&c.path))]);
        w.close("asset");
    }
    w.close("resources");
    w.open("library", &[]);
    w.open("event", &[("name", t.name.clone())]);
    w.open("project", &[("name", t.name.clone())]);
    let length = time(t.length(), rate);
    w.open("sequence", &[("format", "r1".into()), ("duration", length.clone()), ("tcStart", "0s".into()), ("tcFormat", "NDF".into())]);
    w.open("spine", &[]);
    w.open("gap", &[("name", "Gap".into()), ("offset", "0s".into()), ("start", "0s".into()), ("duration", length)]);
    let (mut v, mut a) = (0, 0);
    for track in &t.tracks {
        let lane = match track.kind {
            Kind::Video => {
                v += 1;
                v
            }
            Kind::Audio => {
                a -= 1;
                a
            }
        };
        for c in &track.clips {
            let id = format!("r{}", assets.iter().position(|p| *p == c.path).unwrap_or(0) + 2);
            let mut attrs = vec![("ref", id), ("lane", lane.to_string()), ("offset", time(c.at, rate)), ("name", c.name.clone()), ("start", time(c.source_in, rate)), ("duration", time(c.length, rate))];
            if track.kind == Kind::Audio {
                attrs.push(("srcEnable", "audio".into()));
            } else if c.media.audio {
                attrs.push(("srcEnable", "video".into()));
            }
            w.empty("asset-clip", &attrs);
        }
    }
    w.close("gap");
    w.close("spine");
    w.close("sequence");
    w.close("project");
    w.close("event");
    w.close("library");
    w.close("fcpxml");
    w.out
}

/// One clip found: where it is in the timeline, its lane, and itself.
struct Found {
    lane: i64,
    kind: Kind,
    clip: Clip,
}

/// The clips of a container (the spine, or a clip others hang on) placed in the timeline: the
/// container's own time `from` is timeline time `to`. In a clip only what hangs on it (in a
/// lane) is another clip; the rest is its own media.
#[allow(clippy::too_many_arguments)]
fn place(e: &Element, from: f64, to: f64, lane: i64, assets: &[(String, String, Media)], found: &mut Vec<Found>, notes: &mut Vec<String>, hanging: bool) {
    for c in e.children.iter().filter(|c| !hanging || c.attr("lane").is_some()) {
        let offset = c.attr("offset").and_then(seconds).unwrap_or(from);
        let at = to + (offset - from);
        let length = c.attr("duration").and_then(seconds).unwrap_or(0.0);
        let start = c.attr("start").and_then(seconds).unwrap_or(0.0);
        let lane = c.attr("lane").and_then(|l| l.parse().ok()).unwrap_or(lane);
        match c.name.as_str() {
            "asset-clip" | "clip" | "video" | "audio" | "ref-clip" | "sync-clip" | "mc-clip" | "gap" | "title" => {
                // the media it shows: its own ref, or that of its own picture or sound in it (not
                // of what hangs on it); a gap shows none
                let own = |k: &&Element| k.attr("lane").is_none() && matches!(k.name.as_str(), "video" | "audio" | "asset-clip");
                let reference = if c.name == "gap" { None } else { c.attr("ref").or_else(|| c.children.iter().filter(own).find_map(|k| k.attr("ref"))) };
                if let Some(asset) = reference.and_then(|r| assets.iter().find(|(id, _, _)| id == r))
                    && length > 0.0
                {
                    let kind = if c.name == "audio" || c.attr("srcEnable") == Some("audio") || lane < 0 || (!asset.2.video && asset.2.audio) { Kind::Audio } else { Kind::Video };
                    let name = c.attr("name").map_or_else(|| file_name(&asset.1).to_string(), str::to_string);
                    found.push(Found { lane, kind, clip: Clip { name, path: asset.1.clone(), at, length, source_in: start, media: asset.2.clone() } });
                } else if matches!(c.name.as_str(), "title" | "ref-clip" | "mc-clip") && length > 0.0 {
                    notes.push(format!("a {} \"{}\" is left out (it is no file)", c.name, c.attr("name").unwrap_or("")));
                }
                // what hangs on it, in its own time
                place(c, start, at, lane, assets, found, notes, true);
            }
            "spine" => place(c, from, to, lane, assets, found, notes, false),
            "transition" => notes.push("a transition is made a cut".into()),
            _ => {}
        }
    }
}

pub(crate) fn read(text: &str) -> Result<(Timeline, Vec<String>), String> {
    let root = xml::parse(text)?;
    if root.name != "fcpxml" {
        return Err(format!("an FCPXML file starts with <fcpxml>, not <{}>", root.name));
    }
    let mut notes = Vec::new();
    let resources = root.child("resources");
    let formats: Vec<&Element> = resources.map(|r| r.all("format").collect()).unwrap_or_default();
    let assets: Vec<(String, String, Media)> = resources
        .map(|r| {
            r.all("asset")
                .map(|a| {
                    let src = a.attr("src").or_else(|| a.child("media-rep").and_then(|m| m.attr("src"))).unwrap_or("");
                    let format = a.attr("format").and_then(|f| formats.iter().find(|x| x.attr("id") == Some(f)));
                    let media = Media {
                        length: a.attr("duration").and_then(seconds).unwrap_or(0.0),
                        width: format.and_then(|f| f.attr("width")).and_then(|v| v.parse().ok()).unwrap_or(0),
                        height: format.and_then(|f| f.attr("height")).and_then(|v| v.parse().ok()).unwrap_or(0),
                        video: a.attr("hasVideo") == Some("1"),
                        audio: a.attr("hasAudio") == Some("1"),
                    };
                    (a.attr("id").unwrap_or("").to_string(), path_of(src), media)
                })
                .collect()
        })
        .unwrap_or_default();
    let all = root.descendants();
    let project = all.iter().find(|e| e.name == "project");
    let sequence = all.iter().find(|e| e.name == "sequence").ok_or("there is no sequence in it")?;
    let format = sequence.attr("format").and_then(|f| formats.iter().find(|x| x.attr("id") == Some(f)));
    let frame = format.and_then(|f| f.attr("frameDuration")).and_then(seconds).filter(|d| *d > 0.0);
    let rate = frame.map(|d| Rate::near(1.0 / d));
    let spine = sequence.child("spine").ok_or("the sequence has no spine")?;
    let mut found = Vec::new();
    place(spine, 0.0, 0.0, 0, &assets, &mut found, &mut notes, false);
    // a track for each lane: pictures from the lowest lane up, then sound
    let mut lanes: Vec<(Kind, i64)> = found.iter().map(|f| (f.kind, f.lane)).collect();
    lanes.sort_by_key(|(k, l)| (*k == Kind::Audio, if *k == Kind::Audio { -*l } else { *l }));
    lanes.dedup();
    let tracks = lanes.iter().map(|(k, l)| Track { kind: *k, clips: found.iter().filter(|f| f.kind == *k && f.lane == *l).map(|f| f.clip.clone()).collect() }).collect();
    let name = project.and_then(|p| p.attr("name")).or_else(|| all.iter().find(|e| e.name == "event").and_then(|e| e.attr("name"))).unwrap_or("Timeline").to_string();
    let size = |k: &str, d: u32| format.and_then(|f| f.attr(k)).and_then(|v| v.parse().ok()).unwrap_or(d);
    Ok((Timeline { name, rate: rate.unwrap_or(Rate::FPS_30), width: size("width", 1920), height: size("height", 1080), tracks }, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(seconds("10s"), Some(10.0));
        assert_eq!(seconds("1001/30000s"), Some(1001.0 / 30000.0));
        assert_eq!(time(2.0, Rate::FPS_30), "2s");
        assert_eq!(time(0.5, Rate::FPS_30), "15/30s");
        assert_eq!(time(1.001, Rate::of(30, true)), "30030/30000s");
    }
}

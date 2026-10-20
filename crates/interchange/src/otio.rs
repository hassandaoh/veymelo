//! OpenTimelineIO: a timeline as JSON, each object saying what it is (`OTIO_SCHEMA`); a stack of
//! tracks, each track clips and gaps one after another, each clip a range of its media (a
//! rational time: a value at a rate) and a reference to its file.

use veymelo_json::{Map, Value as Json};

use crate::{Clip, Kind, Media, Rate, Timeline, Track, file_name, file_url, path_of};

fn time(seconds: f64, rate: Rate) -> Json {
    Json::object([("OTIO_SCHEMA", Json::from("RationalTime.1")), ("rate", Json::Number(rate.fps())), ("value", Json::Number(rate.frames(seconds) as f64))])
}

fn range(start: f64, length: f64, rate: Rate) -> Json {
    Json::object([("OTIO_SCHEMA", Json::from("TimeRange.1")), ("start_time", time(start, rate)), ("duration", time(length, rate))])
}

pub(crate) fn write(t: &Timeline) -> String {
    let rate = t.rate;
    let mut tracks = Vec::new();
    let (mut v, mut a) = (0, 0);
    for track in &t.tracks {
        let (name, kind) = match track.kind {
            Kind::Video => {
                v += 1;
                (format!("V{v}"), "Video")
            }
            Kind::Audio => {
                a += 1;
                (format!("A{a}"), "Audio")
            }
        };
        let mut clips: Vec<&Clip> = track.clips.iter().collect();
        clips.sort_by(|x, y| x.at.total_cmp(&y.at));
        let mut children = Vec::new();
        let mut at = 0.0;
        for c in clips {
            if c.at > at + 1e-9 {
                children.push(Json::object([("OTIO_SCHEMA", Json::from("Gap.1")), ("name", Json::from("")), ("source_range", range(0.0, c.at - at, rate))]));
            }
            let mut media = Map::new();
            media.insert("OTIO_SCHEMA", "ExternalReference.1");
            media.insert("target_url", file_url(&c.path));
            if c.media.length > 0.0 {
                media.insert("available_range", range(0.0, c.media.length, rate));
            }
            let mut clip = Map::new();
            clip.insert("OTIO_SCHEMA", "Clip.2");
            clip.insert("name", c.name.as_str());
            clip.insert("source_range", range(c.source_in, c.length, rate));
            clip.insert("media_references", Json::object([("DEFAULT_MEDIA", Json::Object(media))]));
            clip.insert("active_media_reference_key", "DEFAULT_MEDIA");
            children.push(Json::Object(clip));
            at = c.at + c.length;
        }
        tracks.push(Json::object([("OTIO_SCHEMA", Json::from("Track.1")), ("name", Json::from(name)), ("kind", Json::from(kind)), ("children", Json::Array(children))]));
    }
    let mut meta = Map::new();
    meta.insert("width", t.width as f64);
    meta.insert("height", t.height as f64);
    let doc = Json::object([
        ("OTIO_SCHEMA", Json::from("Timeline.1")),
        ("name", Json::from(t.name.as_str())),
        ("metadata", Json::object([("veymelo", Json::Object(meta))])),
        ("global_start_time", time(0.0, rate)),
        ("tracks", Json::object([("OTIO_SCHEMA", Json::from("Stack.1")), ("name", Json::from("tracks")), ("children", Json::Array(tracks))])),
    ]);
    doc.to_pretty()
}

fn schema(j: &Json) -> &str {
    j.get("OTIO_SCHEMA").and_then(Json::as_str).unwrap_or("")
}

/// A rational time in seconds, and its rate.
fn seconds(j: Option<&Json>) -> Option<(f64, f64)> {
    let j = j?;
    let (rate, value) = (j.get("rate").and_then(Json::as_f64)?, j.get("value").and_then(Json::as_f64)?);
    (rate > 0.0).then_some((value / rate, rate))
}

/// A time range: (start, length) in seconds, and its rate.
fn span(j: Option<&Json>) -> Option<(f64, f64, f64)> {
    let j = j?;
    let (start, rate) = seconds(j.get("start_time"))?;
    let (length, _) = seconds(j.get("duration"))?;
    Some((start, length, rate))
}

pub(crate) fn read(text: &str) -> Result<(Timeline, Vec<String>), String> {
    let doc = Json::parse(text).map_err(|e| format!("not JSON: {e}"))?;
    if !schema(&doc).starts_with("Timeline") {
        return Err(format!("an OTIO timeline is a Timeline, not {}", if schema(&doc).is_empty() { "this" } else { schema(&doc) }));
    }
    let mut notes = Vec::new();
    let name = doc.get("name").and_then(Json::as_str).unwrap_or("Timeline").to_string();
    let meta = doc.get("metadata").and_then(|m| m.get("veymelo"));
    let size = |k: &str, d: u32| meta.and_then(|m| m.get(k)).and_then(Json::as_f64).map_or(d, |v| v as u32);
    let mut rate: Option<Rate> = None;
    let mut tracks = Vec::new();
    let stack = doc.get("tracks").ok_or("an OTIO timeline has its tracks in \"tracks\"")?;
    for track in stack.get("children").and_then(Json::as_array).cloned().unwrap_or_default() {
        if !schema(&track).starts_with("Track") {
            notes.push(format!("a {} among the tracks is left out", schema(&track)));
            continue;
        }
        let kind = if track.get("kind").and_then(Json::as_str) == Some("Audio") { Kind::Audio } else { Kind::Video };
        let mut clips = Vec::new();
        let mut at = 0.0;
        for item in track.get("children").and_then(Json::as_array).cloned().unwrap_or_default() {
            let Some((start, length, r)) = span(item.get("source_range")) else {
                notes.push(format!("a {} with no range is left out", schema(&item)));
                continue;
            };
            rate.get_or_insert_with(|| Rate::near(r));
            match schema(&item) {
                s if s.starts_with("Gap") => at += length,
                s if s.starts_with("Clip") => {
                    // the media: the one it says (Clip.2), or the one it has (Clip.1)
                    let media = match item.get("media_references") {
                        Some(refs) => {
                            let key = item.get("active_media_reference_key").and_then(Json::as_str).unwrap_or("DEFAULT_MEDIA");
                            refs.get(key).cloned()
                        }
                        None => item.get("media_reference").cloned(),
                    };
                    let url = media.as_ref().and_then(|m| m.get("target_url")).and_then(Json::as_str).unwrap_or("");
                    let path = if url.is_empty() { String::new() } else { path_of(url) };
                    let available = media.as_ref().and_then(|m| span(m.get("available_range")));
                    let name = item.get("name").and_then(Json::as_str).filter(|n| !n.is_empty()).map_or_else(|| file_name(&path).to_string(), str::to_string);
                    let media = Media { length: available.map_or(0.0, |a| a.1), video: kind == Kind::Video, audio: kind == Kind::Audio, ..Media::default() };
                    // a range of the media's own time, from where its available range starts
                    let source_in = start - available.map_or(0.0, |a| a.0);
                    clips.push(Clip { name, path, at, length, source_in: source_in.max(0.0), media });
                    at += length;
                }
                s if s.starts_with("Transition") => notes.push("a transition is made a cut".into()),
                s => {
                    notes.push(format!("a {s} is left out (its time kept)"));
                    at += length;
                }
            }
        }
        tracks.push(Track { kind, clips });
    }
    tracks.sort_by_key(|t| t.kind == Kind::Audio);
    Ok((Timeline { name, rate: rate.unwrap_or(Rate::FPS_30), width: size("width", 1920), height: size("height", 1080), tracks }, notes))
}

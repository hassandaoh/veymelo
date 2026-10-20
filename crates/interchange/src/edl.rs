//! The CMX 3600 edit decision list: a line for each event (its number, reel, track, the cut,
//! where in its source it starts and ends, where in the record it starts and ends, as
//! timecodes), notes after it naming its clip and file. It holds one video track (the first)
//! and the sound tracks.

use crate::{Clip, Kind, Media, Rate, Timeline, Track, file_name};

/// Frames as a timecode, dropping frame numbers at NTSC rates (with ';').
fn timecode(frames: i64, rate: Rate) -> String {
    let base = rate.timebase() as i64;
    let mut f = frames.max(0);
    let drop = if rate.ntsc() && base % 30 == 0 { base / 15 } else { 0 };
    if drop > 0 {
        let ten = base * 600 - drop * 9;
        let minute = base * 60 - drop;
        let (d, m) = (f / ten, f % ten);
        f += drop * 9 * d + if m > drop { drop * ((m - drop) / minute) } else { 0 };
    }
    let (h, m, s, ff) = (f / (base * 3600), f / (base * 60) % 60, f / base % 60, f % base);
    format!("{h:02}:{m:02}:{s:02}{}{ff:02}", if drop > 0 { ';' } else { ':' })
}

/// A timecode as frames (frame numbers dropped where it says so with ';', or `drop` asks).
fn frames_of(tc: &str, rate: Rate, drop_frame: bool) -> Option<i64> {
    let parts: Vec<i64> = tc.split([':', ';', '.']).map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let [h, m, s, f] = parts[..] else { return None };
    let base = rate.timebase() as i64;
    let mut frames = ((h * 60 + m) * 60 + s) * base + f;
    if drop_frame || tc.contains(';') {
        let drop = base / 15;
        let minutes = h * 60 + m;
        frames -= drop * (minutes - minutes / 10);
    }
    Some(frames)
}

pub(crate) fn write(t: &Timeline) -> (String, Vec<String>) {
    let rate = t.rate;
    let mut notes = Vec::new();
    let mut out = format!("TITLE: {}\nFCM: {}\n\n", t.name, if rate.ntsc() && rate.timebase().is_multiple_of(30) { "DROP FRAME" } else { "NON-DROP FRAME" });
    let video: Vec<&Track> = t.tracks.iter().filter(|k| k.kind == Kind::Video).collect();
    if video.len() > 1 {
        notes.push(format!("an EDL holds one video track: the first is kept, {} more left out", video.len() - 1));
    }
    let mut events: Vec<(&Clip, String)> = Vec::new();
    if let Some(v) = video.first() {
        events.extend(v.clips.iter().map(|c| (c, "V".to_string())));
    }
    for (k, a) in t.tracks.iter().filter(|k| k.kind == Kind::Audio).enumerate() {
        events.extend(a.clips.iter().map(|c| (c, if k == 0 { "A".to_string() } else { format!("A{}", k + 1) })));
    }
    for (n, (c, track)) in events.iter().enumerate() {
        let (si, so) = (rate.frames(c.source_in), rate.frames(c.source_in + c.length));
        let (ri, ro) = (rate.frames(c.at), rate.frames(c.at + c.length));
        out.push_str(&format!("{:03}  {:<8} {:<5} C        {} {} {} {}\n", n + 1, "AX", track, timecode(si, rate), timecode(so, rate), timecode(ri, rate), timecode(ro, rate)));
        out.push_str(&format!("* FROM CLIP NAME: {}\n", c.name));
        if !c.path.is_empty() {
            out.push_str(&format!("* SOURCE FILE: {}\n", c.path));
        }
        out.push('\n');
    }
    (out, notes)
}

pub(crate) fn read(text: &str, rate: Rate) -> Result<(Timeline, Vec<String>), String> {
    let mut name = String::from("Timeline");
    let mut drop_frame = false;
    let mut notes = Vec::new();
    // each event: its track and clip
    let mut events: Vec<(String, Clip)> = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if let Some(t) = l.strip_prefix("TITLE:") {
            name = t.trim().to_string();
        } else if let Some(f) = l.strip_prefix("FCM:") {
            drop_frame = f.trim().eq_ignore_ascii_case("DROP FRAME");
        } else if let Some(c) = l.strip_prefix("* FROM CLIP NAME:") {
            if let Some((_, clip)) = events.last_mut() {
                clip.name = c.trim().to_string();
            }
        } else if let Some(p) = l.strip_prefix("* SOURCE FILE:") {
            if let Some((_, clip)) = events.last_mut() {
                clip.path = p.trim().to_string();
            }
        } else if l.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 8 {
                continue;
            }
            // the last four are the timecodes; a dissolve or wipe has its length before them
            let tcs = &f[f.len() - 4..];
            let (track, how) = (f[2], f[3]);
            if how != "C" {
                notes.push(format!("event {}: its {} is made a cut", f[0], if how.starts_with('D') { "dissolve" } else { "wipe" }));
            }
            let get = |k: usize| frames_of(tcs[k], rate, drop_frame).ok_or_else(|| format!("event {}: \"{}\" is not a timecode", f[0], tcs[k]));
            let (si, ri, ro) = (get(0)?, get(2)?, get(3)?);
            get(1)?;
            if ro <= ri {
                continue;
            }
            let clip = Clip { name: f[1].to_string(), path: String::new(), at: rate.seconds(ri), length: rate.seconds(ro - ri), source_in: rate.seconds(si), media: Media { video: track.contains('V'), audio: track.contains('A'), ..Media::default() } };
            events.push((track.to_string(), clip));
        }
    }
    // the record often starts at an hour: the timeline starts at its first event
    let start = events.iter().map(|(_, c)| c.at).fold(f64::INFINITY, f64::min);
    let start = if start.is_finite() && start >= 3600.0 - 1e-6 { (start / 3600.0).floor() * 3600.0 } else { 0.0 };
    let mut tracks: Vec<(String, Track)> = Vec::new();
    for (track, mut clip) in events {
        clip.at -= start;
        if clip.name.is_empty() || clip.name == "AX" || clip.name == "BL" {
            clip.name = if clip.path.is_empty() { "Clip".into() } else { file_name(&clip.path).to_string() };
        }
        // video events on the video track; a sound event on each of its sound tracks
        let mut lanes: Vec<(String, Kind)> = Vec::new();
        if track.starts_with('V') || track == "B" {
            lanes.push(("V".into(), Kind::Video));
        }
        if track.contains('A') {
            let a = track.trim_start_matches('V').trim_start_matches('/');
            lanes.push((if a == "AA" { "A".into() } else { a.to_string() }, Kind::Audio));
        }
        for (lane, kind) in lanes {
            match tracks.iter_mut().find(|(n, _)| *n == lane) {
                Some((_, t)) => t.clips.push(clip.clone()),
                None => tracks.push((lane, Track { kind, clips: vec![clip.clone()] })),
            }
        }
    }
    tracks.sort_by_key(|(n, t)| (t.kind == Kind::Audio, n.clone()));
    Ok((Timeline { name, rate, width: 1920, height: 1080, tracks: tracks.into_iter().map(|(_, t)| t).collect() }, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecodes() {
        let r = Rate::FPS_30;
        assert_eq!(timecode(30 * 3661 + 5, r), "01:01:01:05");
        assert_eq!(frames_of("01:01:01:05", r, false), Some(30 * 3661 + 5));
        // 29.97 drop frame: a minute in, frames 0 and 1 are not counted
        let ntsc = Rate::of(30, true);
        assert_eq!(timecode(1800, ntsc), "00:01:00;02");
        assert_eq!(frames_of("00:01:00;02", ntsc, false), Some(1800));
        assert_eq!(timecode(17982, ntsc), "00:10:00;00", "the tenth minute drops none");
        assert_eq!(frames_of("00:10:00;00", ntsc, false), Some(17982));
    }
}

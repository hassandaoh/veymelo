//! Timelines as video editors exchange them, read and written our own way from the formats'
//! public descriptions: the CMX 3600 edit decision list (EDL), OpenTimelineIO (OTIO, JSON),
//! Final Cut Pro 7 XML (xmeml, which Premiere Pro reads and writes too) and FCPXML (Final Cut
//! Pro's).
//!
//! A timeline is tracks of clips: each clip a stretch of a file placed at a time. Times are
//! seconds here, made frames of the timeline's rate where a format counts frames.

mod edl;
mod fcp7;
mod fcpxml;
mod otio;
pub mod xml;

/// Frames a second as a fraction (`30000/1001` for 29.97).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    pub num: u32,
    pub den: u32,
}

impl Rate {
    pub const FPS_30: Rate = Rate { num: 30, den: 1 };

    pub fn fps(self) -> f64 {
        self.num as f64 / self.den.max(1) as f64
    }

    /// The whole number of frames a second it is counted by (30 for 29.97).
    pub fn timebase(self) -> u32 {
        self.fps().round().max(1.0) as u32
    }

    /// One of the NTSC rates (29.97, 23.976, 59.94…).
    pub fn ntsc(self) -> bool {
        self.den == 1001
    }

    /// The rate a whole timebase stands for, NTSC or not.
    pub fn of(timebase: u32, ntsc: bool) -> Rate {
        if ntsc { Rate { num: timebase * 1000, den: 1001 } } else { Rate { num: timebase.max(1), den: 1 } }
    }

    /// The nearest rate to so many frames a second (29.97… made NTSC).
    pub fn near(fps: f64) -> Rate {
        let base = fps.round();
        if (fps - base).abs() > 1e-3 && (fps * 1.001 - base).abs() < 1e-2 { Rate::of(base as u32, true) } else { Rate::of(base.max(1.0) as u32, false) }
    }

    pub fn frames(self, seconds: f64) -> i64 {
        (seconds * self.fps()).round() as i64
    }

    pub fn seconds(self, frames: i64) -> f64 {
        frames as f64 / self.fps()
    }
}

/// Whether a track holds pictures or sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Video,
    Audio,
}

/// What a clip's file is, as far as is known: its length (0: unknown, or a still), its size,
/// and whether it has pictures and sound.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Media {
    pub length: f64,
    pub width: u32,
    pub height: u32,
    pub video: bool,
    pub audio: bool,
}

/// A stretch of a file (from `source_in`, `length` seconds long) placed `at` seconds into the
/// timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub name: String,
    /// The file (none: a clip with nothing behind it).
    pub path: String,
    pub at: f64,
    pub length: f64,
    pub source_in: f64,
    pub media: Media,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub kind: Kind,
    pub clips: Vec<Clip>,
}

/// A timeline: its name, rate, picture size, and tracks (the first video track at the bottom).
#[derive(Clone, Debug, PartialEq)]
pub struct Timeline {
    pub name: String,
    pub rate: Rate,
    pub width: u32,
    pub height: u32,
    pub tracks: Vec<Track>,
}

impl Timeline {
    /// How long it lasts: to the end of its last clip.
    pub fn length(&self) -> f64 {
        self.tracks.iter().flat_map(|t| &t.clips).map(|c| c.at + c.length).fold(0.0, f64::max)
    }
}

/// The formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Edl,
    Otio,
    Fcp7,
    Fcpxml,
}

impl Format {
    pub const IDS: &'static [&'static str] = &["edl", "otio", "xml", "fcpxml"];

    pub fn id(self) -> &'static str {
        match self {
            Format::Edl => "edl",
            Format::Otio => "otio",
            Format::Fcp7 => "xml",
            Format::Fcpxml => "fcpxml",
        }
    }

    pub fn from_id(id: &str) -> Option<Format> {
        match id.to_ascii_lowercase().as_str() {
            "edl" => Some(Format::Edl),
            "otio" => Some(Format::Otio),
            "xml" | "fcp7" | "xmeml" => Some(Format::Fcp7),
            "fcpxml" | "fcpxmld" => Some(Format::Fcpxml),
            _ => None,
        }
    }

    /// The format a file's name says.
    pub fn of_path(path: &str) -> Option<Format> {
        Format::from_id(path.rsplit_once('.')?.1)
    }

    /// The format a file's text is (an XML file told by its root).
    pub fn of_text(text: &str) -> Option<Format> {
        let t = text.trim_start();
        if t.starts_with('{') {
            Some(Format::Otio)
        } else if t.contains("<fcpxml") {
            Some(Format::Fcpxml)
        } else if t.contains("<xmeml") {
            Some(Format::Fcp7)
        } else if t.lines().any(|l| l.trim_start().starts_with("TITLE:") || l.trim_start().starts_with("FCM:")) {
            Some(Format::Edl)
        } else {
            None
        }
    }
}

/// A timeline written in `format`, and notes (what that format cannot keep).
pub fn write(t: &Timeline, format: Format) -> (String, Vec<String>) {
    match format {
        Format::Edl => edl::write(t),
        Format::Otio => (otio::write(t), Vec::new()),
        Format::Fcp7 => (fcp7::write(t), Vec::new()),
        Format::Fcpxml => (fcpxml::write(t), Vec::new()),
    }
}

/// A timeline read from a file's text (`format`, or as the text says), and notes; an EDL has no
/// rate of its own, so `edl_rate` gives it (default 30).
pub fn read(text: &str, format: Option<Format>, edl_rate: Option<Rate>) -> Result<(Timeline, Vec<String>), String> {
    match format.or_else(|| Format::of_text(text)).ok_or("this is not a timeline file (EDL, OTIO, Final Cut Pro XML or FCPXML)")? {
        Format::Edl => edl::read(text, edl_rate.unwrap_or(Rate::FPS_30)),
        Format::Otio => otio::read(text),
        Format::Fcp7 => fcp7::read(text),
        Format::Fcpxml => fcpxml::read(text),
    }
}

/// A file's path as a URL (`file:///…`), its odd characters escaped.
pub fn file_url(path: &str) -> String {
    let mut out = String::from("file://");
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A `file://` URL's path (with `localhost` or not), its escapes undone; anything else as it is.
pub fn path_of(url: &str) -> String {
    let rest = url.strip_prefix("file://localhost").or_else(|| url.strip_prefix("file://")).unwrap_or(url);
    let b = rest.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&rest[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A file's name from its path.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

#[cfg(test)]
mod tests;

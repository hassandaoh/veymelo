//! The render queue: compositions waiting to be made into files, each with how it is drawn
//! (render settings) and what file it becomes (an output module). It is kept in the project.

use veymelo_json::{Map, Value as Json};
use veymelo_time::Tick;

use crate::read::{LoadError, Obj, fail, named};

named! {
    /// How carefully layers are drawn.
    #[derive(Default)]
    pub enum Quality {
        /// As each layer's own switch says.
        Current = "current",
        #[default]
        Best = "best",
        Draft = "draft",
        Wireframe = "wireframe",
    }
}

named! {
    /// Which part of the composition is made.
    #[derive(Default)]
    pub enum Span {
        #[default]
        WorkArea = "workArea",
        Comp = "comp",
        /// From `start` to `end`.
        Custom = "custom",
    }
}

named! {
    /// A switch the render sets for every layer, or leaves as each has it.
    #[derive(Default)]
    pub enum Override {
        #[default]
        Current = "current",
        On = "on",
        Off = "off",
    }
}

named! {
    /// Which proxies a render draws in place of what they stand for.
    #[derive(Default)]
    pub enum ProxyUse {
        /// None: everything as itself.
        #[default]
        None = "none",
        /// Those switched on.
        Current = "current",
        /// Every proxy, on or not.
        All = "all",
        /// Compositions' proxies only (every one).
        Comps = "comps",
    }
}

named! {
    /// What happens once the file is made.
    #[derive(Default)]
    pub enum After {
        #[default]
        Nothing = "none",
        /// The file comes into the project as footage.
        Import = "import",
        /// …and takes the composition's place wherever it is used.
        ImportReplace = "importReplace",
        /// …as the composition's proxy, switched on.
        SetProxy = "setProxy",
    }
}

named! {
    /// What file a composition becomes.
    #[derive(Default)]
    pub enum Format {
        /// H.264 video in an MP4 file.
        #[default]
        Mp4 = "mp4",
        /// HEVC (H.265) video in an MP4 file.
        Hevc = "hevc",
        /// ProRes video in a QuickTime file.
        ProRes = "prores",
        /// A PNG picture for each frame.
        PngSequence = "pngSequence",
        JpegSequence = "jpegSequence",
        TiffSequence = "tiffSequence",
        /// An OpenEXR picture for each frame: floating point, in linear light (32-bit floats in
        /// a 32-bit render, else halves).
        ExrSequence = "exrSequence",
        /// A Photoshop picture for each frame (flattened).
        PsdSequence = "psdSequence",
        /// An animated GIF.
        Gif = "gif",
        /// Its sound only, as a WAV file.
        Wav = "wav",
    }
}

impl Format {
    /// Whether it is one file a frame.
    pub fn is_sequence(self) -> bool {
        matches!(self, Format::PngSequence | Format::JpegSequence | Format::TiffSequence | Format::ExrSequence | Format::PsdSequence)
    }

    /// The file name ending it takes.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Mp4 | Format::Hevc => "mp4",
            Format::ProRes => "mov",
            Format::PngSequence => "png",
            Format::JpegSequence => "jpg",
            Format::TiffSequence => "tif",
            Format::ExrSequence => "exr",
            Format::PsdSequence => "psd",
            Format::Gif => "gif",
            Format::Wav => "wav",
        }
    }

    /// Whether it can keep see-through parts.
    pub fn has_alpha(self) -> bool {
        matches!(self, Format::ProRes | Format::PngSequence | Format::TiffSequence | Format::ExrSequence | Format::Gif)
    }
}

named! {
    /// What of the picture is kept.
    #[derive(Default)]
    pub enum Channels {
        #[default]
        Rgb = "rgb",
        Rgba = "rgba",
        /// The alpha alone, as greys.
        Alpha = "alpha",
    }
}

named! {
    /// The kinds of ProRes, from the smallest files to the best.
    #[derive(Default)]
    pub enum ProRes {
        Proxy = "proxy",
        Lt = "lt",
        #[default]
        Standard = "422",
        Hq = "hq",
        P4444 = "4444",
        Xq = "4444xq",
    }
}

named! {
    /// How far an item has come.
    #[derive(Default)]
    pub enum Status {
        #[default]
        Queued = "queued",
        Done = "done",
        Failed = "failed",
        Stopped = "stopped",
    }
}

/// How a composition is drawn for the render.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderSettings {
    pub quality: Quality,
    /// The size drawn at, 1 for the composition's own (0.5 half, …).
    pub resolution: f64,
    pub span: Span,
    /// A custom span, in composition time.
    pub start: Tick,
    pub end: Tick,
    /// Frames a second made (none: the composition's).
    pub frame_rate: Option<f64>,
    pub motion_blur: Override,
    pub frame_blending: Override,
    /// Effects as each layer's switch says, all on, or all off.
    pub effects: Override,
    /// Solo switches as they are, or off.
    pub solo: Override,
    /// Guide layers drawn (on) or left out.
    pub guides: Override,
    /// Bits a colour: 8, 16 or 32 (0: the project's).
    pub depth: u8,
    pub proxies: ProxyUse,
}

impl Default for RenderSettings {
    fn default() -> RenderSettings {
        RenderSettings {
            quality: Quality::Best,
            resolution: 1.0,
            span: Span::WorkArea,
            start: Tick::ZERO,
            end: Tick::ZERO,
            frame_rate: None,
            motion_blur: Override::Current,
            frame_blending: Override::Current,
            effects: Override::Current,
            solo: Override::Current,
            guides: Override::Off,
            depth: 0,
            proxies: ProxyUse::None,
        }
    }
}

/// What file a composition becomes.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputModule {
    pub format: Format,
    pub channels: Channels,
    /// See-through colours kept multiplied by their alpha (else straight).
    pub premultiplied: bool,
    /// Where the file goes; a sequence's frame number goes where `#` are (`shot_####.png`), or
    /// before the ending.
    pub path: String,
    /// 0 to 100: how good the picture is kept (for those that lose some).
    pub quality: u8,
    /// Kilobits a second of video (0: by the quality).
    pub bitrate: u32,
    /// Frames between whole pictures (0: the encoder's choice).
    pub keyframes: u32,
    pub prores: ProRes,
    /// Pixels cut off the top, left, bottom and right (below zero: added).
    pub crop: [i32; 4],
    /// The size the frames are made (none: as drawn).
    pub resize: Option<[u32; 2]>,
    /// A GIF plays round and round.
    pub gif_loop: bool,
    /// The composition's sound goes in too.
    pub audio: bool,
    /// Its samples a second, channels (1 or 2), bits a sample in WAV and QuickTime PCM (16, 24,
    /// or 32 for floating point), and kilobits a second of AAC (MP4).
    pub audio_rate: u32,
    pub audio_channels: u8,
    pub audio_bits: u8,
    pub audio_bitrate: u32,
}

impl Default for OutputModule {
    fn default() -> OutputModule {
        OutputModule {
            format: Format::Mp4,
            channels: Channels::Rgb,
            premultiplied: false,
            path: String::new(),
            quality: 80,
            bitrate: 0,
            keyframes: 0,
            prores: ProRes::Standard,
            crop: [0; 4],
            resize: None,
            gif_loop: true,
            audio: true,
            audio_rate: 48_000,
            audio_channels: 2,
            audio_bits: 24,
            audio_bitrate: 256,
        }
    }
}

impl OutputModule {
    /// As JSON, only what is not as it starts (its format and path always).
    pub fn to_json(&self) -> Json {
        let (m, d) = (self, OutputModule::default());
        let mut out = Map::new();
        out.insert("format", m.format.id());
        out.insert("path", m.path.as_str());
        if m.channels != d.channels {
            out.insert("channels", m.channels.id());
        }
        if m.premultiplied {
            out.insert("premultiplied", true);
        }
        if m.quality != d.quality {
            out.insert("quality", m.quality as f64);
        }
        if m.bitrate != d.bitrate {
            out.insert("bitrate", m.bitrate as f64);
        }
        if m.keyframes != d.keyframes {
            out.insert("keyframes", m.keyframes as f64);
        }
        if m.prores != d.prores {
            out.insert("prores", m.prores.id());
        }
        if m.crop != d.crop {
            out.insert("crop", Json::array(m.crop.iter().map(|v| Json::Number(*v as f64))));
        }
        if let Some([w, h]) = m.resize {
            out.insert("resize", Json::array([Json::Number(w as f64), Json::Number(h as f64)]));
        }
        if !m.gif_loop {
            out.insert("gifLoop", false);
        }
        if !m.audio {
            out.insert("audio", false);
        }
        for (key, v, dv) in [("audioRate", m.audio_rate, d.audio_rate), ("audioChannels", m.audio_channels as u32, d.audio_channels as u32), ("audioBits", m.audio_bits as u32, d.audio_bits as u32), ("audioBitrate", m.audio_bitrate, d.audio_bitrate)] {
            if v != dv {
                out.insert(key, v as f64);
            }
        }
        Json::Object(out)
    }

    /// An output module from JSON (`at`: where it is, for mistakes).
    pub fn from_json(j: &Json, at: &str) -> Result<OutputModule, LoadError> {
        let m = Obj::new(j, at, "an output module")?;
        m.only(&["format", "path", "channels", "premultiplied", "quality", "bitrate", "keyframes", "prores", "crop", "resize", "gifLoop", "audio", "audioRate", "audioChannels", "audioBits", "audioBitrate"])?;
        let mut out = OutputModule {
            format: m.choice("format", Format::IDS, Format::from_id)?.unwrap_or_default(),
            path: m.text("path")?.unwrap_or("").to_string(),
            channels: m.choice("channels", Channels::IDS, Channels::from_id)?.unwrap_or_default(),
            premultiplied: m.flag("premultiplied", false)?,
            quality: m.whole("quality", 0, 100)?.map_or(80, |v| v as u8),
            bitrate: m.whole("bitrate", 0, 10_000_000)?.map_or(0, |v| v as u32),
            keyframes: m.whole("keyframes", 0, 100_000)?.map_or(0, |v| v as u32),
            prores: m.choice("prores", ProRes::IDS, ProRes::from_id)?.unwrap_or_default(),
            ..OutputModule::default()
        };
        if let Some(c) = m.get("crop") {
            let list = c.as_array().filter(|a| a.len() == 4).ok_or_else(|| LoadError { at: m.at("crop"), message: "the crop is [top, left, bottom, right]".into() })?;
            for (k, v) in list.iter().enumerate() {
                out.crop[k] = v.as_f64().ok_or_else(|| LoadError { at: m.at("crop"), message: "the crop is four numbers".into() })? as i32;
            }
        }
        if let Some(r) = m.get("resize") {
            let list = r.as_array().filter(|a| a.len() == 2).ok_or_else(|| LoadError { at: m.at("resize"), message: "the size is [width, height]".into() })?;
            let side = |v: &Json| v.as_f64().filter(|v| *v >= 1.0 && *v <= crate::MAX_SIZE as f64).map(|v| v as u32);
            out.resize = Some([
                side(&list[0]).ok_or_else(|| LoadError { at: m.at("resize"), message: format!("a width is from 1 to {}", crate::MAX_SIZE) })?,
                side(&list[1]).ok_or_else(|| LoadError { at: m.at("resize"), message: format!("a height is from 1 to {}", crate::MAX_SIZE) })?,
            ]);
        }
        out.gif_loop = m.flag("gifLoop", true)?;
        out.audio = m.flag("audio", true)?;
        out.audio_rate = m.whole("audioRate", 8_000, 192_000)?.map_or(48_000, |v| v as u32);
        out.audio_channels = m.whole("audioChannels", 1, 2)?.map_or(2, |v| v as u8);
        out.audio_bits = match m.whole("audioBits", 16, 32)? {
            None => 24,
            Some(b @ (16 | 24 | 32)) => b as u8,
            Some(b) => return Err(LoadError { at: m.at("audioBits"), message: format!("bits a sample are 16, 24 or 32 (floating point), not {b}") }),
        };
        out.audio_bitrate = m.whole("audioBitrate", 32, 512)?.map_or(256, |v| v as u32);
        Ok(out)
    }
}

/// A composition in the render queue.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderItem {
    pub id: u64,
    pub comp: u64,
    /// Made when the queue is rendered.
    pub enabled: bool,
    pub settings: RenderSettings,
    pub output: OutputModule,
    /// More files made of the same frames, each as its own output module says.
    pub extra: Vec<OutputModule>,
    pub status: Status,
    /// What happened when it was last made.
    pub message: String,
    /// What happens once it is made.
    pub after: After,
}

impl RenderItem {
    pub fn new(id: u64, comp: u64) -> RenderItem {
        RenderItem { id, comp, enabled: true, settings: RenderSettings::default(), output: OutputModule::default(), extra: Vec::new(), status: Status::Queued, message: String::new(), after: After::Nothing }
    }

    /// As JSON, only what is not as it starts.
    pub fn to_json(&self) -> Json {
        let mut o = Map::new();
        if self.id != 0 {
            o.insert("id", self.id as f64);
        }
        o.insert("comp", self.comp as f64);
        if !self.enabled {
            o.insert("enabled", false);
        }
        let (s, d) = (&self.settings, RenderSettings::default());
        let mut r = Map::new();
        if s.quality != d.quality {
            r.insert("quality", s.quality.id());
        }
        if s.resolution != d.resolution {
            r.insert("resolution", s.resolution);
        }
        if s.span != d.span {
            r.insert("span", s.span.id());
        }
        if s.span == Span::Custom {
            r.insert("start", s.start.seconds());
            r.insert("end", s.end.seconds());
        }
        if let Some(f) = s.frame_rate {
            r.insert("frameRate", f);
        }
        for (key, v, dv) in [("motionBlur", s.motion_blur, d.motion_blur), ("frameBlending", s.frame_blending, d.frame_blending), ("effects", s.effects, d.effects), ("solo", s.solo, d.solo), ("guides", s.guides, d.guides)] {
            if v != dv {
                r.insert(key, v.id());
            }
        }
        if s.depth != d.depth {
            r.insert("depth", s.depth as f64);
        }
        if s.proxies != d.proxies {
            r.insert("proxies", s.proxies.id());
        }
        if !r.is_empty() {
            o.insert("settings", r);
        }
        o.insert("output", self.output.to_json());
        if !self.extra.is_empty() {
            o.insert("moreOutputs", Json::array(self.extra.iter().map(OutputModule::to_json)));
        }
        if self.status != Status::Queued {
            o.insert("status", self.status.id());
        }
        if !self.message.is_empty() {
            o.insert("message", self.message.as_str());
        }
        if self.after != After::Nothing {
            o.insert("after", self.after.id());
        }
        Json::Object(o)
    }

    pub fn from_json(j: &Json, at: &str) -> Result<RenderItem, LoadError> {
        let o = Obj::new(j, at, "a render queue item")?;
        o.only(&["id", "comp", "enabled", "settings", "output", "status", "message", "after"])?;
        let Some(comp) = o.id("comp")? else { return fail(at, "a render queue item needs \"comp\", the composition it makes") };
        let mut item = RenderItem::new(o.id("id")?.unwrap_or(0), comp);
        item.enabled = o.flag("enabled", true)?;
        if let Some(r) = o.get("settings") {
            let at = o.at("settings");
            let r = Obj::new(r, &at, "render settings")?;
            r.only(&["quality", "resolution", "span", "start", "end", "frameRate", "motionBlur", "frameBlending", "effects", "solo", "guides", "depth", "proxies"])?;
            let s = &mut item.settings;
            s.quality = r.choice("quality", Quality::IDS, Quality::from_id)?.unwrap_or_default();
            s.resolution = match r.num("resolution")? {
                None => 1.0,
                Some(v) if v > 0.0 && v <= 1.0 => v,
                Some(_) => return fail(&r.at("resolution"), "the resolution is above 0 and at most 1"),
            };
            s.span = r.choice("span", Span::IDS, Span::from_id)?.unwrap_or_default();
            s.start = Tick::from_seconds(r.num("start")?.unwrap_or(0.0));
            s.end = Tick::from_seconds(r.num("end")?.unwrap_or(0.0));
            s.frame_rate = r.num("frameRate")?;
            if s.frame_rate.is_some_and(|f| !(f > 0.0 && f <= 1000.0)) {
                return fail(&r.at("frameRate"), "the frame rate is above 0 and at most 1000");
            }
            let pick = |key: &str| r.choice(key, Override::IDS, Override::from_id);
            s.motion_blur = pick("motionBlur")?.unwrap_or_default();
            s.frame_blending = pick("frameBlending")?.unwrap_or_default();
            s.effects = pick("effects")?.unwrap_or_default();
            s.solo = pick("solo")?.unwrap_or_default();
            s.guides = pick("guides")?.unwrap_or(Override::Off);
            s.depth = match r.whole("depth", 0, 32)? {
                None | Some(0) => 0,
                Some(b @ (8 | 16 | 32)) => b as u8,
                Some(_) => return fail(&r.at("depth"), "the depth is 8, 16 or 32 (or 0 for the project's)"),
            };
            s.proxies = r.choice("proxies", ProxyUse::IDS, ProxyUse::from_id)?.unwrap_or_default();
        }
        if let Some(m) = o.get("output") {
            item.output = OutputModule::from_json(m, &o.at("output"))?;
        }
        for (k, m) in o.list("moreOutputs")?.iter().enumerate() {
            item.extra.push(OutputModule::from_json(m, &format!("{}[{k}]", o.at("moreOutputs")))?);
        }
        item.status = o.choice("status", Status::IDS, Status::from_id)?.unwrap_or_default();
        item.message = o.text("message")?.unwrap_or("").to_string();
        item.after = o.choice("after", After::IDS, After::from_id)?.unwrap_or_default();
        Ok(item)
    }
}

/// The file of frame `frame` of a sequence going to `path`: the number where the `#` are
/// (as many digits as there are), or before the ending (five digits).
pub fn frame_path(path: &str, frame: u64) -> String {
    if let Some(at) = path.find('#') {
        let n = path[at..].chars().take_while(|c| *c == '#').count();
        return format!("{}{:0n$}{}", &path[..at], frame, &path[at + n..], n = n);
    }
    match path.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.contains('/') => format!("{stem}_{frame:05}.{ext}"),
        _ => format!("{path}_{frame:05}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_read_back_the_same() {
        let mut item = RenderItem::new(7, 3);
        let plain = RenderItem::from_json(&item.to_json(), "item").unwrap();
        assert_eq!(plain, item);
        item.settings = RenderSettings { quality: Quality::Draft, resolution: 0.5, span: Span::Custom, start: Tick::from_seconds(1.0), end: Tick::from_seconds(3.5), frame_rate: Some(12.0), motion_blur: Override::Off, frame_blending: Override::On, effects: Override::Off, solo: Override::Off, guides: Override::On, depth: 16, proxies: ProxyUse::Current };
        item.output = OutputModule { format: Format::ProRes, channels: Channels::Rgba, premultiplied: true, path: "out/shot.mov".into(), quality: 60, bitrate: 8000, keyframes: 30, prores: ProRes::P4444, crop: [2, 4, 6, -8], resize: Some([960, 540]), gif_loop: false, audio: false, audio_rate: 44_100, audio_channels: 1, audio_bits: 16, audio_bitrate: 128 };
        item.status = Status::Done;
        item.message = "made in 3 s".into();
        item.after = After::ImportReplace;
        let back = RenderItem::from_json(&item.to_json(), "item").unwrap();
        assert_eq!(back, item);
    }

    #[test]
    fn mistakes_say_where() {
        let bad = Json::parse(r#"{"comp": 1, "settings": {"resolution": 2}}"#).unwrap();
        assert_eq!(RenderItem::from_json(&bad, "renderQueue[0]").unwrap_err().to_string(), "renderQueue[0].settings.resolution: the resolution is above 0 and at most 1");
    }

    #[test]
    fn sequence_frames_are_numbered() {
        assert_eq!(frame_path("out/shot_####.png", 7), "out/shot_0007.png");
        assert_eq!(frame_path("out/shot.png", 12), "out/shot_00012.png");
        assert_eq!(frame_path("frames", 3), "frames_00003");
    }
}

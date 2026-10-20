//! Compositions made into files, as a render queue item says: each frame drawn as its render
//! settings ask (quality, size, which part, how many a second, the switches it overrides), cut
//! and sized as its output module asks, and written:
//!
//! | Format | File | Picture | See-through |
//! |---|---|---|---|
//! | mp4 | MP4, H.264 (the system's encoder) | 8-bit, BT.709 | no |
//! | hevc | MP4, HEVC (the system's encoder) | 8-bit, BT.709 | no |
//! | prores | QuickTime, ProRes proxy … 4444 XQ (the system's encoder) | 10/12-bit | 4444 and 4444 XQ |
//! | pngSequence | a PNG a frame (our own) | 8-bit | yes |
//! | jpegSequence | a JPEG a frame (the system's) | 8-bit | no |
//! | tiffSequence | a TIFF a frame (the system's) | 8-bit | yes |
//! | gif | animated GIF (our own) | up to 256 colours a frame | 1-bit |
//! | wav | WAV (our own): the sound only, 16 or 24-bit PCM or 32-bit float | — | — |
//!
//! Video files take the composition's sound too (when it has any and the output module says
//! so): AAC in MP4, PCM in QuickTime (AAC when a bit rate is given), mixed frame by frame.
//!
//! H.264 and HEVC need an even width and height: an odd one loses its last row or column.

pub mod apart;
pub use veymelo_exr::write as exr;
mod gif;
pub mod psd;
pub mod wav;

use std::borrow::Cow;

use veymelo_project::queue::{Channels, Format, OutputModule, Override, Quality, RenderItem, RenderSettings, Span, frame_path};
use veymelo_project::{Comp, Project};
use veymelo_raster::Image;
use veymelo_render::Options;
use veymelo_time::{FrameRate, Tick};

pub use gif::Gif;

/// What a render made.
#[derive(Clone, Debug, PartialEq)]
pub struct Made {
    pub frames: u64,
    /// The files written (a sequence: every frame's).
    pub files: Vec<String>,
    /// The frames' size.
    pub size: [usize; 2],
    /// What could not be done as asked, and what the frames said.
    pub notes: Vec<String>,
}

/// The frames a render makes: their rate, and their times in the composition.
pub fn frame_times(comp: &Comp, s: &RenderSettings) -> Result<(FrameRate, Vec<Tick>), String> {
    let rate = match s.frame_rate {
        Some(f) => FrameRate::from_f64(f).ok_or_else(|| format!("{f} frames a second cannot be made"))?,
        None => comp.frame_rate,
    };
    let (start, end) = match s.span {
        Span::WorkArea => (comp.work_area.start, comp.work_area.end()),
        Span::Comp => (Tick::ZERO, comp.duration),
        Span::Custom => (s.start, s.end),
    };
    let (start, end) = (start.max(Tick::ZERO), end.min(comp.duration));
    if end <= start {
        return Err("there is no time to make: the span ends where it starts".into());
    }
    // every frame that starts in the span
    let mut f = rate.frame_at(start);
    if rate.tick_of(f) < start {
        f += 1;
    }
    let mut times = Vec::new();
    while rate.tick_of(f) < end {
        times.push(rate.tick_of(f));
        f += 1;
    }
    Ok((rate, times))
}

/// How the frames are drawn, from the render settings (`clear`: without the composition's
/// background).
pub fn options(s: &RenderSettings, clear: bool) -> Options {
    let on = |o: Override| match o {
        Override::Current => None,
        Override::On => Some(true),
        Override::Off => Some(false),
    };
    Options {
        scale: s.resolution,
        background: !clear,
        motion_blur: s.motion_blur != Override::Off,
        guides: s.guides == Override::On,
        quality: match s.quality {
            Quality::Current => None,
            Quality::Best => Some(veymelo_project::Quality::Best),
            Quality::Draft => Some(veymelo_project::Quality::Draft),
            Quality::Wireframe => Some(veymelo_project::Quality::Wireframe),
        },
        effects: on(s.effects),
        solo: s.solo != Override::Off,
        depth: (s.depth != 0).then_some(s.depth),
        proxies: s.proxies,
        ..Options::default()
    }
}

/// A picture with `crop` (top, left, bottom, right pixels) cut off (below zero: clear added).
fn crop(img: &Image, c: [i32; 4]) -> Cow<'_, Image> {
    if c == [0; 4] {
        return Cow::Borrowed(img);
    }
    let [top, left, bottom, right] = c.map(|v| v as i64);
    let (w, h) = ((img.width as i64 - left - right).max(1) as usize, (img.height as i64 - top - bottom).max(1) as usize);
    let mut out = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = (x as i64 + left, y as i64 + top);
            if sx >= 0 && sy >= 0 && (sx as usize) < img.width && (sy as usize) < img.height {
                out.px[y * w + x] = img.px[sy as usize * img.width + sx as usize];
            }
        }
    }
    Cow::Owned(out)
}

/// A picture made `w` × `h`: each new pixel the mean of the old ones it covers where it gets
/// smaller (so nothing shimmers), between them where it gets bigger.
pub fn resize(img: &Image, w: usize, h: usize) -> Image {
    if (img.width, img.height) == (w, h) {
        return img.clone();
    }
    // one way at a time: across, then down
    let line = |src: &[[f32; 4]], n_out: usize| -> Vec<[f32; 4]> {
        let n = src.len();
        let k = n as f64 / n_out as f64;
        (0..n_out)
            .map(|i| {
                if k > 1.0 {
                    // the old pixels from a to b, the ends counted for the part they cover
                    let (a, b) = (i as f64 * k, (i + 1) as f64 * k);
                    let mut sum = [0f64; 4];
                    let mut j = a.floor() as usize;
                    while (j as f64) < b && j < n {
                        let part = ((j + 1) as f64).min(b) - (j as f64).max(a);
                        (0..4).for_each(|c| sum[c] += src[j][c] as f64 * part);
                        j += 1;
                    }
                    sum.map(|v| (v / k) as f32)
                } else {
                    let x = ((i as f64 + 0.5) * k - 0.5).clamp(0.0, (n - 1) as f64);
                    let (j, t) = (x.floor() as usize, (x - x.floor()) as f32);
                    let (p, q) = (src[j], src[(j + 1).min(n - 1)]);
                    std::array::from_fn(|c| p[c] + (q[c] - p[c]) * t)
                }
            })
            .collect()
    };
    let mut across = vec![[0f32; 4]; w * img.height];
    for y in 0..img.height {
        let row = line(&img.px[y * img.width..(y + 1) * img.width], w);
        across[y * w..(y + 1) * w].copy_from_slice(&row);
    }
    let mut out = Image::new(w, h);
    for x in 0..w {
        let col: Vec<[f32; 4]> = (0..img.height).map(|y| across[y * w + x]).collect();
        for (y, p) in line(&col, h).into_iter().enumerate() {
            out.px[y * w + x] = p;
        }
    }
    out
}

/// A drawn frame as straight (or `premultiplied`) eight-bit RGBA with only the channels asked
/// for: the colour (alpha full), all four, or the alpha as greys.
fn channels(img: &Image, ch: Channels, premultiplied: bool) -> Vec<u8> {
    let mut rgba = img.to_rgba8();
    for p in rgba.as_chunks_mut::<4>().0.iter_mut() {
        match ch {
            Channels::Rgb => p[3] = 255,
            Channels::Alpha => {
                let a = p[3];
                p.copy_from_slice(&[a, a, a, 255]);
            }
            Channels::Rgba if premultiplied => {
                let a = p[3] as u32;
                for c in p.iter_mut().take(3) {
                    *c = ((*c as u32 * a + 127) / 255) as u8;
                }
            }
            Channels::Rgba => {}
        }
    }
    rgba
}

/// How one output module's frames are made from the drawn ones (on the drawing threads, each
/// its own frames): cut, sized, kept as the module keeps pixels, with the sound that goes with
/// each frame of a video; a picture sequence's frames written as files there too.
struct Making {
    out: OutputModule,
    channels: Channels,
    size: [usize; 2],
    /// It keeps see-through pixels (its frames drawn on nothing, not on the background).
    clear: bool,
    /// Each frame's sound goes with it (a video with a sound track).
    sound: bool,
}

/// A frame made ready for its output module, handed on in order.
enum Ready {
    /// Eight-bit pixels as the module keeps them, and the frame's sound (a video's).
    Pixels(Vec<u8>, Vec<f32>),
    /// A picture file written already.
    File(String),
}

impl Making {
    /// Frame `k` (at composition time `times[k]`), drawn as `img`, made ready.
    fn ready(&self, project: &Project, item: &RenderItem, img: &Image, k: usize, times: &[Tick], rate: FrameRate) -> Result<Ready, String> {
        let out = &self.out;
        let cut = crop(img, out.crop);
        let img = if [cut.width, cut.height] != self.size { Cow::Owned(resize(&cut, self.size[0], self.size[1])) } else { cut };
        let path = || frame_path(&out.path, rate.frame_at(times[k]).max(0) as u64);
        match out.format {
            // floating point and layers' pictures are made from the drawn pixels themselves
            Format::ExrSequence | Format::PsdSequence => {
                let path = path();
                let linear = project.settings.linear_blending;
                let bytes = if out.format == Format::ExrSequence {
                    let deep = item.settings.depth == 32 || (item.settings.depth == 0 && project.settings.bit_depth == 32);
                    apart::exr_file(&img, linear, !deep)
                } else {
                    let seen: Vec<[f32; 4]> = img.px.iter().map(|p| {
                        if p[3] <= 0.0 {
                            return [0.0; 4];
                        }
                        let c = |v: f32| {
                            let s = (v / p[3]).clamp(0.0, 1.0) as f64;
                            (if linear { veymelo_color::linear_to_srgb(s) } else { s }) as f32
                        };
                        [c(p[0]), c(p[1]), c(p[2]), p[3]]
                    }).collect();
                    psd::encode(img.width, img.height, &[], &seen)
                };
                std::fs::write(&path, bytes).map_err(|e| format!("cannot write {path}: {e}"))?;
                Ok(Ready::File(path))
            }
            Format::PngSequence => {
                let path = path();
                let rgba = channels(&img, self.channels, out.premultiplied);
                std::fs::write(&path, veymelo_render::png::encode(img.width as u32, img.height as u32, &rgba)).map_err(|e| format!("cannot write {path}: {e}"))?;
                Ok(Ready::File(path))
            }
            #[cfg(target_os = "macos")]
            Format::JpegSequence | Format::TiffSequence => {
                let path = path();
                let kind = if out.format == Format::JpegSequence { "public.jpeg" } else { "public.tiff" };
                let rgba = channels(&img, self.channels, out.premultiplied);
                veymelo_os::write_picture(&path, img.width, img.height, &rgba, kind, self.channels == Channels::Rgba, out.quality as f64 / 100.0)?;
                Ok(Ready::File(path))
            }
            Format::Gif => Ok(Ready::Pixels(channels(&img, self.channels, out.premultiplied), Vec::new())),
            Format::Mp4 | Format::Hevc | Format::ProRes => {
                let rgba = channels(&img, self.channels, out.premultiplied);
                let mut sound = Vec::new();
                if self.sound {
                    // this frame's sound: from its sample to the next frame's
                    let ar = out.audio_rate as f64;
                    let from = |k: usize| ((times[k] - times[0]).seconds() * ar).round() as i64;
                    let (a, b) = (from(k), if k + 1 < times.len() { from(k + 1) } else { from(k) + (ar / rate.as_f64()).round() as i64 });
                    let start = times[0] + Tick::from_seconds(a as f64 / ar);
                    sound = veymelo_render::audio::mix(project, item.comp, start, (b - a).max(0) as usize, out.audio_rate);
                    if out.audio_channels == 1 {
                        sound = sound.as_chunks::<2>().0.iter().map(|f| (f[0] + f[1]) * 0.5).collect();
                    }
                }
                Ok(Ready::Pixels(rgba, sound))
            }
            f => Err(format!("{} pictures cannot be made here", f.id())),
        }
    }
}

/// Where the frames go.
enum Sink {
    Pictures,
    Gif(Box<Gif>),
    #[cfg(target_os = "macos")]
    Video(veymelo_os::Video),
}

/// One output module being written: where its frames go, in order.
struct Writer {
    path: String,
    sequence: bool,
    sink: Sink,
    files: Vec<String>,
}

impl Writer {
    fn open(project: &Project, item: &RenderItem, comp: &Comp, out: &OutputModule, rate: FrameRate, notes: &mut Vec<String>) -> Result<(Making, Writer), String> {
        let s = &item.settings;
        let channels = match (out.channels, out.format.has_alpha()) {
            (Channels::Rgba, false) => {
                notes.push(format!("{} keeps no alpha: the colour only", out.format.id()));
                Channels::Rgb
            }
            (c, _) => c,
        };
        let prores_alpha = matches!(out.prores, veymelo_project::queue::ProRes::P4444 | veymelo_project::queue::ProRes::Xq);
        let channels = if out.format == Format::ProRes && channels == Channels::Rgba && !prores_alpha {
            notes.push("only ProRes 4444 keeps alpha: the colour only".into());
            Channels::Rgb
        } else {
            channels
        };
        // the frames' size: drawn, cut, resized (even for H.264 and HEVC)
        let drawn = [((comp.width as f64 * s.resolution).round() as usize).max(1), ((comp.height as f64 * s.resolution).round() as usize).max(1)];
        let cut = [(drawn[0] as i64 - out.crop[1] as i64 - out.crop[3] as i64).max(1) as usize, (drawn[1] as i64 - out.crop[0] as i64 - out.crop[2] as i64).max(1) as usize];
        let mut size = out.resize.map_or(cut, |[w, h]| [w as usize, h as usize]);
        if matches!(out.format, Format::Mp4 | Format::Hevc) && (size[0] % 2 == 1 || size[1] % 2 == 1) {
            notes.push(format!("H.264 and HEVC need an even size: {}×{} made {}×{}", size[0], size[1], size[0] & !1, size[1] & !1));
            size = [(size[0] & !1).max(2), (size[1] & !1).max(2)];
        }
        if let Some(dir) = std::path::Path::new(&out.path).parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot make the folder {}: {e}", dir.display()))?;
        }
        let mut sound = false;
        let sink = match out.format {
            Format::Gif => Sink::Gif(Box::new(Gif::new(size[0], size[1], rate.as_f64(), out.gif_loop, true))),
            Format::Mp4 | Format::Hevc | Format::ProRes => {
                #[cfg(target_os = "macos")]
                {
                    let codec = match out.format {
                        Format::Mp4 => veymelo_os::Codec::H264,
                        Format::Hevc => veymelo_os::Codec::Hevc,
                        _ => veymelo_os::Codec::ProRes(out.prores as u8),
                    };
                    let track = (out.audio && veymelo_render::audio::heard(project, item.comp)).then(|| veymelo_os::Sound {
                        rate: out.audio_rate,
                        channels: out.audio_channels as u32,
                        // QuickTime keeps PCM; MP4 takes AAC
                        aac_bitrate: if out.format == Format::ProRes { 0 } else { out.audio_bitrate * 1000 },
                    });
                    sound = track.is_some();
                    Sink::Video(veymelo_os::Video::new(&out.path, size[0], size[1], rate.num as i32, rate.den as i64, codec, out.quality as f64 / 100.0, out.bitrate as u64 * 1000, out.keyframes, track)?)
                }
                #[cfg(not(target_os = "macos"))]
                return Err("video files are made by the system's encoders, which only macOS has here".into());
            }
            _ => Sink::Pictures,
        };
        let making = Making { out: out.clone(), channels, size, clear: channels != Channels::Rgb, sound };
        Ok((making, Writer { path: out.path.clone(), sequence: out.format.is_sequence(), sink, files: Vec::new() }))
    }

    /// The next frame, made ready.
    fn frame(&mut self, ready: Ready) -> Result<(), String> {
        match (&mut self.sink, ready) {
            (_, Ready::File(path)) => self.files.push(path),
            (Sink::Gif(g), Ready::Pixels(rgba, _)) => g.frame(&rgba),
            #[cfg(target_os = "macos")]
            (Sink::Video(v), Ready::Pixels(rgba, sound)) => {
                v.frame(&rgba)?;
                if v.has_sound() {
                    v.sound(&sound)?;
                }
            }
            (Sink::Pictures, Ready::Pixels(..)) => return Err("a picture sequence's frame was not written".into()),
        }
        Ok(())
    }

    /// The files it made, once every frame is in.
    fn finish(mut self) -> Result<Vec<String>, String> {
        match self.sink {
            Sink::Gif(g) => {
                std::fs::write(&self.path, g.finish()).map_err(|e| format!("cannot write {}: {e}", self.path))?;
                self.files.push(self.path.clone());
            }
            #[cfg(target_os = "macos")]
            Sink::Video(v) => {
                v.finish()?;
                self.files.push(self.path.clone());
            }
            Sink::Pictures => {}
        }
        Ok(self.files)
    }

    /// Stopped part way: a single file left half made goes.
    fn abandon(self) {
        if !self.sequence {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Make render queue item `item` of `project`, telling `progress` how many frames are done of
/// how many after each (it stops the render by giving back false). Every frame is drawn once
/// and written by each of its output modules (twice where some keep see-through pixels and some
/// do not); sound-only ones are written after.
pub fn make(project: &Project, item: &RenderItem, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<Made, String> {
    let comp = project.comp(item.comp).ok_or_else(|| format!("there is no composition {}", item.comp))?;
    let modules: Vec<&OutputModule> = std::iter::once(&item.output).chain(&item.extra).collect();
    if modules.iter().any(|m| m.path.trim().is_empty()) {
        return Err("an output module has no path to write to".into());
    }
    let s = &item.settings;
    let mut notes = Vec::new();
    let (rate, times) = frame_times(comp, s)?;
    let (mut makings, mut writers) = (Vec::new(), Vec::new());
    for out in modules.iter().filter(|m| m.format != Format::Wav) {
        let (m, w) = Writer::open(project, item, comp, out, rate, &mut notes)?;
        makings.push(m);
        writers.push(w);
    }
    let mut files = Vec::new();
    let mut frames = times.len() as u64;
    let size = makings.first().map_or([0, 0], |m| m.size);
    if !writers.is_empty() {
        let (clear, opaque) = (makings.iter().any(|m| m.clear), makings.iter().any(|m| !m.clear));
        let (on_nothing, on_background) = (options(s, true), options(s, false));
        let total = times.len();
        // the frames drawn several at once, each on a thread of its own and made ready for each
        // output module there, then written here in their order as they come (the same frames
        // as one at a time, sooner)
        let draw = |k: usize| -> Result<Drawn, String> {
            let mut found = Vec::new();
            let mut drawn = |o: &Options| -> Result<Image, String> {
                let r = veymelo_render::render(project, item.comp, times[k], o)?;
                found.extend(r.notes);
                Ok(r.image)
            };
            let see_through = if clear { Some(drawn(&on_nothing)?) } else { None };
            let solid = if opaque { Some(drawn(&on_background)?) } else { None };
            let mut ready = Vec::with_capacity(makings.len());
            for m in &makings {
                let img = if m.clear { see_through.as_ref() } else { solid.as_ref() }.expect("drawn");
                ready.push(m.ready(project, item, img, k, &times, rate)?);
            }
            Ok(Drawn { ready, notes: found })
        };
        let mut write = |k: usize, d: Drawn| -> Result<bool, String> {
            for n in d.notes {
                if !notes.contains(&n) {
                    notes.push(n);
                }
            }
            for (w, r) in writers.iter_mut().zip(d.ready) {
                w.frame(r)?;
            }
            Ok(progress(k as u64 + 1, total as u64))
        };
        match in_order(total, drawers(size), &draw, &mut write) {
            Ok(None) => {}
            Ok(Some(done)) => {
                for w in writers {
                    w.abandon();
                }
                return Err(format!("stopped after {done} of {total} frames"));
            }
            Err(e) => {
                for w in writers {
                    w.abandon();
                }
                return Err(e);
            }
        }
        for w in writers {
            files.extend(w.finish()?);
        }
    }
    for out in modules.iter().filter(|m| m.format == Format::Wav) {
        let made = wav_file(project, item, comp, out)?;
        if size == [0, 0] {
            frames = made.frames;
        }
        files.extend(made.files);
        notes.extend(made.notes.into_iter().filter(|n| !notes.contains(n)).collect::<Vec<_>>());
    }
    Ok(Made { frames, files, size, notes })
}

/// A frame drawn (on nothing for files that keep see-through pixels, on the background for
/// those that do not) and made ready for each output module, and what drawing it found to say.
struct Drawn {
    ready: Vec<Ready>,
    notes: Vec<String>,
}

/// How many frames `size` pixels big are drawn at once to start with, on this computer as it
/// is now: one a core (only the fast ones when it runs hot or saves power, two when it is too
/// hot), fewer when frames that big would not fit in the memory that is free.
fn drawers(size: [usize; 2]) -> usize {
    let m = veymelo_os::machine::machine();
    let cores = match (m.saving, m.heat) {
        (_, 3) => 2,
        (true, _) | (_, 2) => m.fast.max(2),
        _ => m.cores(),
    };
    // a frame's pictures while it is drawn: some full-size floating point layers
    let per = (size[0] * size[1] * 16 * 6) as u64;
    cores.min((room(&m) / per.max(1)) as usize).max(1)
}

/// The memory a render lets itself hold on this computer: some of what is free, never more than
/// two fifths of it all (bytes).
fn room(m: &veymelo_os::machine::Machine) -> u64 {
    (m.free * 3 / 5).min(m.memory * 2 / 5).max(512 << 20)
}

/// How many of the drawing threads may take a new frame, kept to what the computer can give:
/// fewer while the render holds more memory than it lets itself or the computer runs hot, more
/// again (up to `most`) when there is room.
struct Pace {
    allowed: std::sync::atomic::AtomicUsize,
    most: usize,
    budget: u64,
    checked: std::sync::Mutex<(std::time::Instant, std::time::Instant)>,
}

impl Pace {
    fn new(most: usize) -> Pace {
        let m = veymelo_os::machine::machine();
        // what it holds already is not counted against it
        let budget = room(&m) + veymelo_os::machine::held();
        let now = std::time::Instant::now();
        Pace { allowed: std::sync::atomic::AtomicUsize::new(most), most, budget, checked: std::sync::Mutex::new((now, now)) }
    }

    fn allowed(&self) -> usize {
        self.allowed.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Looks again at what the render holds (a few times a second) and how hot the computer is
    /// (every couple of seconds), and lets more or fewer threads draw.
    fn look(&self) {
        use std::sync::atomic::Ordering;
        let Ok(mut when) = self.checked.try_lock() else { return };
        if when.0.elapsed().as_millis() < 150 {
            return;
        }
        when.0 = std::time::Instant::now();
        let held = veymelo_os::machine::held();
        let mut cap = self.most;
        if when.1.elapsed().as_secs_f64() > 2.0 {
            when.1 = std::time::Instant::now();
            let m = veymelo_os::machine::machine();
            cap = match (m.saving, m.heat) {
                (_, 3) => 1,
                (true, _) | (_, 2) => (self.most / 2).max(1),
                _ => self.most,
            };
            // the computer itself short of memory: as if over budget
            if m.free < m.memory / 10 {
                cap = cap.min(self.allowed().saturating_sub(1).max(1));
            }
        }
        let now = self.allowed();
        let next = if held > self.budget {
            now.saturating_sub(1).max(1)
        } else if held < self.budget / 2 && now < cap {
            now + 1
        } else {
            now.min(cap)
        };
        self.allowed.store(next.clamp(1, self.most), Ordering::Relaxed);
    }
}

/// Draws frames `0..total` with `draw`, `drawers` at once at most (each thread keeping its own
/// work to itself, below what the person is doing, and as many drawing as [`Pace`] lets), and
/// gives each to `write` on this thread in their order as soon as those before it are written.
/// Drawing runs at most a few frames ahead of writing. `write` gives false to stop: then how
/// many were written comes back.
fn in_order<D: Send>(total: usize, drawers: usize, draw: &(dyn Fn(usize) -> Result<D, String> + Sync), write: &mut dyn FnMut(usize, D) -> Result<bool, String>) -> Result<Option<usize>, String> {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let drawers = drawers.clamp(1, total.max(1));
    let (next, written, stop) = (AtomicUsize::new(0), AtomicUsize::new(0), AtomicBool::new(false));
    let ahead = drawers * 2;
    let pace = Pace::new(drawers);
    let (tx, rx) = std::sync::mpsc::channel::<(usize, Result<D, String>)>();
    std::thread::scope(|scope| {
        for me in 0..drawers {
            let tx = tx.clone();
            let (next, written, stop, pace) = (&next, &written, &stop, &pace);
            scope.spawn(move || {
                veymelo_os::machine::give_way();
                veymelo_raster::par::alone(|| {
                    loop {
                        // only as many threads draw as the computer has room for now (one waiting
                        // lets go of what it keeps for the GPU)
                        if me >= pace.allowed() {
                            veymelo_render::let_go();
                        }
                        while me >= pace.allowed() && !stop.load(Ordering::Relaxed) {
                            pace.look();
                            std::thread::sleep(std::time::Duration::from_millis(20));
                        }
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        if k >= total || stop.load(Ordering::Relaxed) {
                            break;
                        }
                        // not too far ahead of what is written
                        while k >= written.load(Ordering::Relaxed) + ahead && !stop.load(Ordering::Relaxed) {
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                        if stop.load(Ordering::Relaxed) || tx.send((k, draw(k))).is_err() {
                            break;
                        }
                        pace.look();
                    }
                    veymelo_render::let_go();
                })
            });
        }
        drop(tx);
        let mut waiting = std::collections::BTreeMap::new();
        let mut k = 0;
        let mut outcome = Ok(None);
        'frames: for (at, drawn) in &rx {
            waiting.insert(at, drawn);
            while let Some(drawn) = waiting.remove(&k) {
                let go_on = drawn.and_then(|d| write(k, d));
                k += 1;
                written.store(k, Ordering::Relaxed);
                match go_on {
                    Ok(true) => {}
                    Ok(false) => {
                        outcome = Ok(Some(k));
                        break 'frames;
                    }
                    Err(e) => {
                        outcome = Err(e);
                        break 'frames;
                    }
                }
            }
        }
        stop.store(true, Ordering::Relaxed);
        outcome
    })
}

/// A frame written as a JPEG (no see-through) or TIFF picture, by the system's encoders.
pub fn write_picture(img: &Image, path: &str, tiff: bool) -> Result<(), String> {
    let rgba = channels(img, if tiff { Channels::Rgba } else { Channels::Rgb }, false);
    #[cfg(target_os = "macos")]
    return veymelo_os::write_picture(path, img.width, img.height, &rgba, if tiff { "public.tiff" } else { "public.jpeg" }, tiff, 0.9);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (rgba, path);
        Err("JPEG and TIFF pictures are made by the system's encoders, which only macOS has here".into())
    }
}

/// A WAV file of the composition's sound over the span the settings say.
fn wav_file(project: &Project, item: &RenderItem, comp: &Comp, out: &OutputModule) -> Result<Made, String> {
    let (rate, times) = frame_times(comp, &item.settings)?;
    let (start, end) = (times[0], *times.last().unwrap_or(&times[0]) + rate.frame_duration());
    let ar = out.audio_rate;
    let frames = ((end - start).seconds() * ar as f64).round().max(0.0) as usize;
    let mut notes = Vec::new();
    if !veymelo_render::audio::heard(project, item.comp) {
        notes.push("the composition has no sound: the file is silent".into());
    }
    // a second at a time (the mix is the same wherever its blocks fall)
    let mut samples = Vec::with_capacity(frames * 2);
    let mut done = 0usize;
    while done < frames {
        let n = (ar as usize).min(frames - done);
        samples.extend(veymelo_render::audio::mix(project, item.comp, start + Tick::from_seconds(done as f64 / ar as f64), n, ar));
        done += n;
    }
    let peak = samples.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak > 1.0 && out.audio_bits != 32 {
        notes.push(format!("the sound goes {:.1} dB past full scale and is clipped", 20.0 * peak.log10()));
    }
    std::fs::write(&out.path, wav::encode(&samples, ar, out.audio_channels as u16, out.audio_bits as u16)).map_err(|e| format!("cannot write {}: {e}", out.path))?;
    Ok(Made { frames: frames as u64, files: vec![out.path.clone()], size: [0, 0], notes })
}

#[cfg(test)]
mod tests;

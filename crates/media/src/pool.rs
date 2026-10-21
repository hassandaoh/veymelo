//! Frames and sound kept in memory: decoded frames by file and number, least lately used let go
//! first past the budget; one video reader a file, left where it stopped; an animated
//! picture's frame times; each file's sound at each rate asked for.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use veymelo_project::{AlphaMode, Footage, FootageKind};
use veymelo_raster::Image;

use crate::{PICTURES, ext_of};

/// What the pool holds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub frames: usize,
    pub bytes: usize,
    pub budget: usize,
    pub readers: usize,
    pub sounds: usize,
    /// Frames found kept, and frames decoded.
    pub hits: u64,
    pub decoded: u64,
}

/// A video reader and the number of the frame it gives next.
struct Reader {
    #[cfg(target_os = "macos")]
    frames: veymelo_os::Frames,
    next: u64,
    /// Its file's own frames a second.
    fps: f64,
}

// the reader is only used while the pool's lock is held
unsafe impl Send for Reader {}

struct Pool {
    budget: usize,
    bytes: usize,
    clock: u64,
    frames: HashMap<(String, u64), (Arc<Image>, u64)>,
    readers: HashMap<String, Reader>,
    /// An animated picture's frame start times, and its length.
    times: HashMap<String, (Vec<f64>, f64)>,
    sounds: HashMap<(String, u32), Arc<Vec<f32>>>,
    /// Each file read, as it was when first read: when it was last changed and how big it was.
    seen: HashMap<String, (Option<std::time::SystemTime>, u64)>,
    hits: u64,
    decoded: u64,
}

/// When a file was last changed and how big it is (none if it is not there).
fn stamp(path: &str) -> Option<(Option<std::time::SystemTime>, u64)> {
    std::fs::metadata(path).ok().map(|m| (m.modified().ok(), m.len()))
}

impl Pool {
    /// Remember how a file was when it is first read.
    fn note(&mut self, path: &str) {
        if !self.seen.contains_key(path)
            && let Some(s) = stamp(path)
        {
            self.seen.insert(path.to_string(), s);
        }
    }
}

/// Whether a file has changed since it was first read (since it was let go of last): none read
/// yet is not changed.
pub fn changed(path: &str) -> bool {
    let p = lock();
    match (p.seen.get(path), stamp(path)) {
        (Some(was), Some(now)) => *was != now,
        (Some(_), None) => true,
        _ => false,
    }
}

/// A gigabyte and a half of frames.
const BUDGET: usize = 1536 << 20;

fn pool() -> &'static Mutex<Pool> {
    static POOL: OnceLock<Mutex<Pool>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(Pool { budget: BUDGET, bytes: 0, clock: 0, frames: HashMap::new(), readers: HashMap::new(), times: HashMap::new(), sounds: HashMap::new(), seen: HashMap::new(), hits: 0, decoded: 0 }))
}

fn lock() -> std::sync::MutexGuard<'static, Pool> {
    pool().lock().unwrap_or_else(|e| e.into_inner())
}

/// How much memory frames may take.
pub fn set_budget(bytes: usize) {
    let mut p = lock();
    p.budget = bytes.max(16 << 20);
    p.trim();
}

pub fn stats() -> Stats {
    let p = lock();
    Stats { frames: p.frames.len(), bytes: p.bytes, budget: p.budget, readers: p.readers.len(), sounds: p.sounds.len(), hits: p.hits, decoded: p.decoded }
}

/// Lets go of everything kept of a file (after it changed or was replaced); every file when
/// none is given.
pub fn forget(path: Option<&str>) {
    let mut p = lock();
    match path {
        Some(path) => {
            let gone: usize = p.frames.iter().filter(|((f, _), _)| f.starts_with(path)).map(|(_, (img, _))| size(img)).sum();
            p.frames.retain(|(f, _), _| !f.starts_with(path));
            p.bytes -= gone.min(p.bytes);
            p.readers.retain(|f, _| !f.starts_with(path));
            p.times.retain(|f, _| !f.starts_with(path));
            p.sounds.retain(|(f, _), _| !f.starts_with(path));
            p.seen.retain(|f, _| !f.starts_with(path));
        }
        None => {
            p.frames.clear();
            p.bytes = 0;
            p.readers.clear();
            p.times.clear();
            p.sounds.clear();
            p.seen.clear();
        }
    }
}

fn size(img: &Image) -> usize {
    img.px.len() * 16 + 64
}

impl Pool {
    fn keep(&mut self, key: (String, u64), img: Arc<Image>) {
        self.clock += 1;
        self.bytes += size(&img);
        if let Some((old, _)) = self.frames.insert(key, (img, self.clock)) {
            self.bytes -= size(&old).min(self.bytes);
        }
        self.trim();
    }

    fn trim(&mut self) {
        while self.bytes > self.budget && self.frames.len() > 1 {
            let Some(oldest) = self.frames.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| k.clone()) else { break };
            if let Some((img, _)) = self.frames.remove(&oldest) {
                self.bytes -= size(&img).min(self.bytes);
            }
        }
    }

    fn kept(&mut self, key: &(String, u64)) -> Option<Arc<Image>> {
        self.clock += 1;
        let clock = self.clock;
        let (img, used) = self.frames.get_mut(key)?;
        *used = clock;
        self.hits += 1;
        Some(img.clone())
    }
}

/// A frame as the footage is interpreted: alpha as it says, pixels made square.
fn interpret(f: &Footage, p: veymelo_os::Picture) -> Image {
    let mut px = p.px;
    match f.alpha {
        AlphaMode::Straight => {}
        // the file's colours were already multiplied by its alpha: once is undone
        AlphaMode::Premultiplied => px.iter_mut().filter(|q| q[3] > 0.0).for_each(|q| (0..3).for_each(|k| q[k] /= q[3])),
        // the colours as they are, solid
        AlphaMode::Ignore => px.iter_mut().for_each(|q| {
            if q[3] > 0.0 && q[3] < 1.0 {
                (0..3).for_each(|k| q[k] /= q[3]);
            }
            q[3] = 1.0;
        }),
    }
    square(f, Image { width: p.width, height: p.height, px: px.into() })
}

/// A frame's pixels made square, as wide as the footage's pixel aspect says.
fn square(f: &Footage, img: Image) -> Image {
    if (f.pixel_aspect - 1.0).abs() < 1e-6 || f.pixel_aspect <= 0.0 {
        return img;
    }
    // wider (or narrower) pixels made square: each column of the result an average across
    let w = ((img.width as f64 * f.pixel_aspect).round() as usize).max(1);
    let mut out = Image::new(w, img.height);
    let k = img.width as f64 / w as f64;
    for x in 0..w {
        let (a, b) = (x as f64 * k, (x + 1) as f64 * k);
        let (i0, i1) = (a.floor() as usize, (b.ceil() as usize).min(img.width));
        for y in 0..img.height {
            let mut acc = [0.0f32; 4];
            let mut wsum = 0.0f32;
            for i in i0..i1 {
                let cover = ((i + 1) as f64).min(b) - (i as f64).max(a);
                let cover = if k < 1.0 { 1.0 } else { cover.max(0.0) } as f32;
                let q = img.px[y * img.width + i];
                for c in 0..4 {
                    acc[c] += q[c] * cover;
                }
                wsum += cover;
            }
            out.px[y * w + x] = acc.map(|v| if wsum > 0.0 { v / wsum } else { 0.0 });
        }
    }
    out
}

/// Frame `n` of footage `f` (counted at its frame rate), decoded or as kept.
pub fn frame(f: &Footage, n: u64) -> Result<Arc<Image>, String> {
    let key_path = format!("{}|{:?}|{}|{:?}", f.path, f.alpha, f.pixel_aspect, f.layer);
    let mut p = lock();
    let animated = f.kind == FootageKind::Video && PICTURES.contains(&ext_of(&f.path).as_str());
    let native = f.native_rate.unwrap_or(f.frame_rate).as_f64();
    // the file's own frame that frame `n` is
    let own = match f.kind {
        FootageKind::Image => 0,
        FootageKind::Sequence => n,
        _ if animated => {
            if !p.times.contains_key(&f.path) {
                let info = crate::picture_info(&f.path)?;
                let mut at = 0.0;
                let starts: Vec<f64> = info.delays.iter().map(|d| {
                    let s = at;
                    at += d;
                    s
                }).collect();
                p.times.insert(f.path.clone(), (starts, at));
            }
            let (starts, _) = &p.times[&f.path];
            let t = n as f64 / native + 1e-9;
            (starts.partition_point(|s| *s <= t).max(1) - 1) as u64
        }
        _ => n,
    };
    let key = (key_path, own);
    if let Some(img) = p.kept(&key) {
        return Ok(img);
    }
    let file = f.frame_path(own);
    p.note(&file);
    match f.kind {
        FootageKind::Image if f.layer.is_some() => {
            let img = Arc::new(crate::layer_frame(&file, f.layer.as_ref().expect("a layer"))?);
            p.decoded += 1;
            p.keep(key, img.clone());
            Ok(img)
        }
        FootageKind::Image if crate::is_vector(&file) => {
            let img = Arc::new(crate::vector_frame(&file)?);
            p.decoded += 1;
            p.keep(key, img.clone());
            Ok(img)
        }
        FootageKind::Image | FootageKind::Sequence => {
            // OpenEXR pictures by our own decoder, alpha as the footage says
            let img = match crate::is_exr(&file).then(|| crate::exr::picture(&file, f.alpha)) {
                Some(Ok(pic)) => square(f, Image { width: pic.width, height: pic.height, px: pic.px.into() }),
                _ => interpret(f, veymelo_os::read_picture(&file, 0)?),
            };
            p.decoded += 1;
            let img = Arc::new(img);
            p.keep(key, img.clone());
            Ok(img)
        }
        _ if animated => {
            let pic = crate::read_picture(&f.path, own as usize)?;
            p.decoded += 1;
            let img = Arc::new(interpret(f, pic));
            p.keep(key, img.clone());
            Ok(img)
        }
        FootageKind::Video => video_frame(&mut p, f, key, native),
        FootageKind::Audio => Err(format!("{} is sound, with no pictures", f.path)),
        FootageKind::Placeholder => Err(format!("{} is a placeholder, with no pictures of its own", f.path)),
        FootageKind::Model => Err(format!("{} is a 3D model: a 3D model layer draws it", f.path)),
    }
}

#[cfg(target_os = "macos")]
fn video_frame(p: &mut Pool, f: &Footage, key: (String, u64), fps: f64) -> Result<Arc<Image>, String> {
    let n = key.1;
    // read on from where the reader is when the frame is a little ahead; else start again there
    let near = p.readers.get(&f.path).is_some_and(|r| r.next <= n && n < r.next + 48);
    if !near {
        // a little before the frame, so the frame showing at its time is among those given
        let from = (n as f64 - 0.5).max(0.0) / fps;
        let frames = veymelo_os::Frames::open(&f.path, from)?;
        p.readers.insert(f.path.clone(), Reader { frames, next: 0, fps });
    }
    let mut found: Option<Arc<Image>> = None;
    while let Some(r) = p.readers.get_mut(&f.path) {
        let Some(next) = r.frames.next_frame() else {
            p.readers.remove(&f.path);
            break;
        };
        let (at, pic) = next?;
        // the frame showing at `at`: a reader started partway into a frame gives that frame
        // stamped with the start, not its own time
        let idx = (at * r.fps + 1e-3).floor().max(0.0) as u64;
        r.next = idx + 1;
        p.decoded += 1;
        let img = Arc::new(interpret(f, pic));
        p.keep((key.0.clone(), idx), img.clone());
        if idx >= n {
            if idx == n || found.is_none() {
                found = Some(img);
            }
            break;
        }
        found = Some(img);
    }
    found.ok_or_else(|| format!("{} has no frame {n}", f.path))
}

#[cfg(not(target_os = "macos"))]
fn video_frame(_: &mut Pool, f: &Footage, key: (String, u64), _: f64) -> Result<Arc<Image>, String> {
    Err(format!("{} frame {}: video is only read on macOS so far", f.path, key.1))
}

/// `frames` sample frames of footage `f`'s sound from its own time `start` (seconds), at
/// `rate`, stereo interleaved; silent where it has none (before it, after it).
pub fn sound(f: &Footage, start: f64, frames: usize, rate: u32) -> Result<Vec<f32>, String> {
    let mut out = vec![0.0f32; frames * 2];
    if !f.has_audio || rate == 0 {
        return Ok(out);
    }
    let all = {
        let mut p = lock();
        let key = (f.path.clone(), rate);
        match p.sounds.get(&key) {
            Some(s) => s.clone(),
            None => {
                p.note(&f.path);
                let s = Arc::new(veymelo_os::read_sound(&f.path, rate, 2)?);
                p.sounds.insert(key, s.clone());
                s
            }
        }
    };
    let have = all.len() / 2;
    if have == 0 {
        return Ok(out);
    }
    let loops = f.loops.max(1) as i64;
    let first = (start * rate as f64).round() as i64;
    for (i, o) in out.as_chunks_mut::<2>().0.iter_mut().enumerate() {
        let k = first + i as i64;
        if k < 0 || k >= have as i64 * loops {
            continue;
        }
        let k = (k % have as i64) as usize;
        *o = [all[k * 2], all[k * 2 + 1]];
    }
    Ok(out)
}

//! Footage: what a file is (a picture, an animated picture, a numbered picture sequence, a
//! video, a sound) and its frames and sound when they are drawn and heard, decoded by the
//! system (`veymelo-os`) and kept in memory within a budget.
//!
//! Frames come out premultiplied in floating point with square pixels, as the footage's
//! interpretation says (alpha straight, premultiplied or left out; its pixel aspect). A video
//! keeps one reader a file, left where it stopped, so frames in order are read on without
//! seeking; frames it passes on the way are kept too. Sound is decoded once a file and rate.

mod exr;
mod pool;
mod scenes;

use std::path::Path;

use veymelo_project::{AlphaMode, Footage, FootageKind};
use veymelo_time::{FrameRate, Tick};

pub use exr::{is_exr, passes, passes_of};
pub use pool::{Stats, changed, forget, frame, set_budget, sound, stats};
pub use scenes::{Cut, scene_cuts};

/// The rate a picture sequence plays at unless it is told another.
pub const SEQUENCE_RATE: FrameRate = FrameRate::FPS_30;

/// Kinds of file read as stills (or animated pictures).
pub const PICTURES: &[&str] = &["png", "jpg", "jpeg", "heic", "heif", "avif", "tif", "tiff", "gif", "webp", "bmp", "exr", "hdr", "psd", "tga", "jp2", "ico", "dng", "cr2", "cr3", "nef", "arw", "raf", "orf", "rw2"];
/// Kinds of file read as video.
pub const VIDEOS: &[&str] = &["mov", "mp4", "m4v", "3gp", "3g2", "mts", "m2ts"];
/// Kinds of file read as sound only.
pub const SOUNDS: &[&str] = &["wav", "wave", "aif", "aiff", "aifc", "mp3", "m4a", "aac", "caf", "flac", "ac3"];

/// What a picture file is: OpenEXR files read by our own decoder (the system reads them too,
/// but not all), others by the system.
pub fn picture_info(path: &str) -> Result<veymelo_os::PictureInfo, String> {
    if exr::is_exr(path) {
        return exr::info(path).or_else(|e| veymelo_os::picture_info(path).map_err(|_| e));
    }
    veymelo_os::picture_info(path)
}

/// Frame `index` of a picture file, premultiplied in extended sRGB (OpenEXR files by our own
/// decoder, their colours taken as premultiplied, as the format has them).
pub fn read_picture(path: &str, index: usize) -> Result<veymelo_os::Picture, String> {
    if exr::is_exr(path) && index == 0 {
        return exr::picture(path, AlphaMode::Premultiplied).or_else(|e| veymelo_os::read_picture(path, 0).map_err(|_| e));
    }
    veymelo_os::read_picture(path, index)
}

fn ext_of(path: &str) -> String {
    Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// Whether a file is of a kind footage can be made from.
pub fn importable(path: &str) -> bool {
    let e = ext_of(path);
    PICTURES.contains(&e.as_str()) || VECTORS.contains(&e.as_str()) || VIDEOS.contains(&e.as_str()) || SOUNDS.contains(&e.as_str()) || MODELS.contains(&e.as_str())
}

/// The vector pictures footage is made from (drawn at whatever size they are shown).
pub const VECTORS: &[&str] = &["svg"];

/// Whether footage's file is a vector picture.
pub fn is_vector(path: &str) -> bool {
    VECTORS.contains(&ext_of(path).as_str())
}

/// The 3D model files footage is made from.
pub const MODELS: &[&str] = &["gltf", "glb", "obj", "stl", "fbx"];

/// A frame rate near `fps` (whole rates and the NTSC ones exactly).
fn rate_near(fps: f64) -> FrameRate {
    for (n, d) in [(24000, 1001), (30000, 1001), (60000, 1001), (48000, 1001), (120000, 1001)] {
        if (fps - n as f64 / d as f64).abs() < 0.005 {
            return FrameRate::new(n, d).unwrap_or(FrameRate::FPS_30);
        }
    }
    if (fps - fps.round()).abs() < 0.01 && fps >= 1.0 {
        return FrameRate::new(fps.round() as u32, 1).unwrap_or(FrameRate::FPS_30);
    }
    FrameRate::from_f64(fps).unwrap_or(FrameRate::FPS_30)
}

/// How a picture's alpha is taken unless it is told otherwise: OpenEXR's colours are
/// premultiplied, as the format has them; the system's pictures come premultiplied already.
fn alpha_of(path: &str, alpha: bool) -> AlphaMode {
    match (alpha, exr::is_exr(path)) {
        (false, _) => AlphaMode::Ignore,
        (true, true) => AlphaMode::Premultiplied,
        (true, false) => AlphaMode::Straight,
    }
}

/// What a file is, as footage. `sequence`: a picture whose name ends in a number is read with
/// the pictures beside it numbered the same way, as frames.
pub fn probe(path: &str, sequence: bool) -> Result<Footage, String> {
    let e = ext_of(path);
    if veymelo_model::Model::is_model(path) {
        let m = model(path)?;
        let mut f = Footage::new(path, FootageKind::Model);
        f.duration = Tick::from_seconds(m.length());
        return Ok(f);
    }
    if VECTORS.contains(&e.as_str()) {
        let doc = vector(path)?;
        let mut f = Footage::new(path, FootageKind::Image);
        (f.width, f.height) = (doc.width.ceil().clamp(1.0, 30_000.0) as u32, doc.height.ceil().clamp(1.0, 30_000.0) as u32);
        f.alpha = AlphaMode::Straight;
        return Ok(f);
    }
    if PICTURES.contains(&e.as_str()) {
        let info = picture_info(path)?;
        if sequence && let Some((pattern, start, count)) = find_sequence(path) {
            let rate = SEQUENCE_RATE;
            let mut f = Footage::new(&pattern, FootageKind::Sequence);
            (f.width, f.height, f.start, f.frame_rate) = (info.width, info.height, start, rate);
            f.duration = rate.tick_of(count as i64);
            f.alpha = alpha_of(path, info.alpha);
            return Ok(f);
        }
        if info.frames > 1 {
            // an animated picture: a video whose frames last as the file says
            let total: f64 = info.delays.iter().sum();
            let shortest = info.delays.iter().copied().fold(f64::INFINITY, f64::min);
            let mut f = Footage::new(path, FootageKind::Video);
            (f.width, f.height) = (info.width, info.height);
            f.frame_rate = rate_near((1.0 / shortest).clamp(1.0, 120.0));
            f.duration = Tick::from_seconds(total);
            f.alpha = if info.alpha { AlphaMode::Straight } else { AlphaMode::Ignore };
            return Ok(f);
        }
        let mut f = Footage::new(path, FootageKind::Image);
        (f.width, f.height) = (info.width, info.height);
        f.alpha = alpha_of(path, info.alpha);
        return Ok(f);
    }
    let info = veymelo_os::media_info(path)?;
    let mut f = Footage::new(path, if info.video.is_some() { FootageKind::Video } else { FootageKind::Audio });
    f.duration = Tick::from_seconds(info.duration);
    f.has_audio = info.sound.is_some();
    if let Some(v) = &info.video {
        (f.width, f.height) = (v.width, v.height);
        f.frame_rate = rate_near(if v.fps > 0.0 { v.fps } else { 30.0 });
        f.alpha = if v.alpha { AlphaMode::Straight } else { AlphaMode::Ignore };
    }
    Ok(f)
}

/// The numbered pictures a picture belongs to: a pattern with a `#` for each digit (one `#`
/// when the numbers are not padded), the first number and how many from it to the last.
pub fn find_sequence(path: &str) -> Option<(String, u32, u32)> {
    let p = Path::new(path);
    let name = p.file_name()?.to_str()?;
    let stem_end = name.rfind('.').unwrap_or(name.len());
    let (stem, ext) = (&name[..stem_end], &name[stem_end..]);
    let digits = stem.chars().rev().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 9 {
        return None;
    }
    let prefix = &stem[..stem.len() - digits];
    let dir = p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut numbers: Vec<(u32, usize)> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| {
            let rest = n.strip_prefix(prefix)?.strip_suffix(ext)?;
            (!rest.is_empty() && rest.len() <= 9 && rest.chars().all(|c| c.is_ascii_digit())).then(|| rest.parse::<u32>().ok().map(|v| (v, rest.len())))?
        })
        .collect();
    numbers.sort();
    if numbers.len() < 2 {
        return None;
    }
    let padded = numbers.iter().all(|(_, w)| *w == digits);
    let hashes = if padded { "#".repeat(digits) } else { "#".into() };
    let (first, last) = (numbers.first()?.0, numbers.last()?.0);
    let pattern = dir.join(format!("{prefix}{hashes}{ext}")).to_string_lossy().into_owned();
    let pattern = if p.parent().is_some_and(|d| d.as_os_str().is_empty()) { pattern.trim_start_matches("./").to_string() } else { pattern };
    Some((pattern, first, last - first + 1))
}

/// Which frame of the footage shows at its own time `t` (seconds from its start, after its
/// layer's timing), and how far toward the next (0 to 1); none before it starts or after it
/// ends (loops counted). A still is always its one frame.
pub fn frame_at(f: &Footage, t: f64) -> Option<(u64, f64)> {
    match f.kind {
        FootageKind::Image => Some((0, 0.0)),
        FootageKind::Placeholder if f.duration <= veymelo_time::Tick::ZERO => Some((0, 0.0)),
        FootageKind::Audio => None,
        _ => {
            let length = f.duration.seconds();
            if t.is_nan() || t < 0.0 || length <= 0.0 || t >= length * f.loops.max(1) as f64 {
                return None;
            }
            let t = t % length;
            let x = t * f.frame_rate.as_f64();
            let n = (x + 1e-6).floor();
            let frames = (length * f.frame_rate.as_f64()).round().max(1.0) as u64;
            Some(((n as u64).min(frames - 1), (x - n).clamp(0.0, 1.0)))
        }
    }
}

#[cfg(test)]
mod tests;

/// Whether a file keeps layers our own reader reads (a Photoshop document).
pub fn is_layered(path: &str) -> bool {
    matches!(ext_of(path).as_str(), "psd" | "psb")
}

/// The Photoshop document in the file at `path`, read once and kept while the file stays as it is.
pub fn layered(path: &str) -> Result<std::sync::Arc<veymelo_psd::Psd>, String> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::SystemTime;
    type Kept = HashMap<String, (Option<SystemTime>, Arc<veymelo_psd::Psd>)>;
    static KEPT: Mutex<Option<Kept>> = Mutex::new(None);
    let when = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if when.is_none() && !std::path::Path::new(path).exists() {
        return Err(format!("there is no file {path}"));
    }
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    let all = kept.get_or_insert_with(HashMap::new);
    if let Some((t, d)) = all.get(path)
        && *t == when
    {
        return Ok(d.clone());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let d = Arc::new(veymelo_psd::parse(bytes).map_err(|e| format!("{path}: {e}"))?);
    all.insert(path.to_string(), (when, d.clone()));
    Ok(d)
}

/// One layer of a layered file, premultiplied: just its own rectangle (`cropped`), or the whole
/// document with the layer where it is in it.
fn layer_frame(path: &str, layer: &veymelo_project::SourceLayer) -> Result<veymelo_raster::Image, String> {
    let doc = layered(path)?;
    let (dw, dh) = (doc.width as usize, doc.height as usize);
    let Some(veymelo_psd::Pixels { left, top, width: w, height: h, px }) = doc.pixels(layer.index as usize) else {
        return Ok(if layer.cropped { veymelo_raster::Image::new(1, 1) } else { veymelo_raster::Image::new(dw.max(1), dh.max(1)) });
    };
    let pre = |p: [f32; 4]| [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]];
    if layer.cropped {
        return Ok(veymelo_raster::Image { width: w, height: h, px: px.into_iter().map(pre).collect() });
    }
    let mut img = veymelo_raster::Image::new(dw.max(1), dh.max(1));
    for y in 0..h {
        let dy = top + y as i32;
        if dy < 0 || dy as usize >= dh {
            continue;
        }
        for x in 0..w {
            let dx = left + x as i32;
            if dx >= 0 && (dx as usize) < dw {
                img.px[dy as usize * dw + dx as usize] = pre(px[y * w + x]);
            }
        }
    }
    Ok(img)
}

/// The SVG picture in the file at `path`, read once and kept while the file stays as it is.
pub fn vector(path: &str) -> Result<std::sync::Arc<veymelo_svg::Doc>, String> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::SystemTime;
    type Kept = HashMap<String, (Option<SystemTime>, Arc<veymelo_svg::Doc>)>;
    static KEPT: Mutex<Option<Kept>> = Mutex::new(None);
    let when = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if when.is_none() && !std::path::Path::new(path).exists() {
        return Err(format!("there is no file {path}"));
    }
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    let all = kept.get_or_insert_with(HashMap::new);
    if let Some((t, d)) = all.get(path)
        && *t == when
    {
        return Ok(d.clone());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let d = Arc::new(veymelo_svg::parse(&bytes).map_err(|e| format!("{path}: {e}"))?);
    all.insert(path.to_string(), (when, d.clone()));
    Ok(d)
}

/// A vector picture drawn at its own size, premultiplied, its colours as they are.
fn vector_frame(path: &str) -> Result<veymelo_raster::Image, String> {
    let doc = vector(path)?;
    let (w, h) = (doc.width.ceil().clamp(1.0, 30_000.0) as usize, doc.height.ceil().clamp(1.0, 30_000.0) as usize);
    let color = |c: [f64; 4]| [(c[0] * c[3]) as f32, (c[1] * c[3]) as f32, (c[2] * c[3]) as f32, c[3] as f32];
    Ok(veymelo_svg::draw(&doc, veymelo_geom::Mat3::IDENTITY, w, h, &color))
}

/// The 3D model in the file at `path`, read once and kept while the file stays as it is.
pub fn model(path: &str) -> Result<std::sync::Arc<veymelo_model::Model>, String> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::SystemTime;
    /// Each file's model, with when the file was last changed.
    type Kept = HashMap<String, (Option<SystemTime>, Arc<veymelo_model::Model>)>;
    static KEPT: Mutex<Option<Kept>> = Mutex::new(None);
    let when = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if when.is_none() && !std::path::Path::new(path).exists() {
        return Err(format!("there is no file {path}"));
    }
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    let all = kept.get_or_insert_with(HashMap::new);
    if let Some((t, m)) = all.get(path)
        && *t == when
    {
        return Ok(m.clone());
    }
    let m = Arc::new(veymelo_model::Model::open(path)?);
    all.insert(path.to_string(), (when, m.clone()));
    Ok(m)
}

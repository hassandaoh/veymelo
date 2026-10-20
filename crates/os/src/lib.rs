//! What talks to the operating system: pictures, video and sound read and written through the
//! system's own frameworks. On macOS they are called straight through the Objective-C
//! runtime's and the frameworks' C functions (ImageIO for pictures, AVFoundation for video and
//! sound, decoded and encoded by the system, in hardware where it can); elsewhere each says it
//! is not there yet.

#[cfg(target_os = "macos")]
mod mac;

pub mod gpu;
pub mod machine;

#[cfg(target_os = "macos")]
pub use mac::asset::Frames;
#[cfg(target_os = "macos")]
pub use mac::write::{Codec, Sound, Video};

/// A picture read in: premultiplied RGBA in floating point (sRGB, past white for brighter
/// pictures), the top row first.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub px: Vec<[f32; 4]>,
    /// The file keeps alpha.
    pub alpha: bool,
}

/// What a picture file is.
#[derive(Clone, Debug, PartialEq)]
pub struct PictureInfo {
    /// Its size as it shows (turned as the file says).
    pub width: u32,
    pub height: u32,
    /// How many frames (more than one: animated), and how long each shows, in seconds.
    pub frames: usize,
    pub delays: Vec<f64>,
    pub alpha: bool,
    /// The EXIF orientation, 1 (as it is) to 8.
    pub orientation: u8,
    /// Bits a channel.
    pub depth: u32,
    /// Its kind ("public.png", "public.jpeg"…).
    pub kind: String,
}

/// A video file's pictures.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoTrack {
    /// Its size as it shows (turned).
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// How far its pictures are turned to show upright (clockwise degrees).
    pub turn: u32,
    /// Its codec ("avc1", "hvc1", "apcn", "ap4h"…).
    pub codec: String,
    pub alpha: bool,
}

/// A file's sound.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundTrack {
    pub rate: f64,
    pub channels: u32,
    pub codec: String,
}

/// What a video or sound file holds.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaInfo {
    /// Its length in seconds.
    pub duration: f64,
    pub video: Option<VideoTrack>,
    pub sound: Option<SoundTrack>,
}

/// A picture turned as EXIF orientation `o` says (1: as it is; 6: a quarter turn clockwise…).
pub fn turned(p: Picture, o: u8) -> Picture {
    if o <= 1 || o > 8 {
        return p;
    }
    let (w, h) = (p.width, p.height);
    let (ow, oh) = if o >= 5 { (h, w) } else { (w, h) };
    let mut px = vec![[0.0f32; 4]; w * h];
    for y in 0..oh {
        for x in 0..ow {
            // the pixel of the file shown at (x, y)
            let (sx, sy) = match o {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (y, h - 1 - x),
                7 => (w - 1 - y, h - 1 - x),
                _ => (w - 1 - y, x),
            };
            px[y * ow + x] = p.px[sy * w + sx];
        }
    }
    Picture { width: ow, height: oh, px, alpha: p.alpha }
}

/// From now on, an exception the system raises says what it is about (on standard error)
/// before it ends the program.
pub fn say_exceptions() {
    #[cfg(target_os = "macos")]
    mac::say_exceptions();
}

#[cfg(not(target_os = "macos"))]
const NOT_HERE: &str = "reading and writing media through the system is only on macOS so far";

/// What a picture file is.
pub fn picture_info(path: &str) -> Result<PictureInfo, String> {
    #[cfg(target_os = "macos")]
    return mac::picture::info(path);
    #[cfg(not(target_os = "macos"))]
    return Err(format!("{path}: {NOT_HERE}"));
}

/// Frame `index` of a picture file (0 for a still), turned as the file says.
pub fn read_picture(path: &str, index: usize) -> Result<Picture, String> {
    #[cfg(target_os = "macos")]
    return mac::picture::read(path, index);
    #[cfg(not(target_os = "macos"))]
    return Err(format!("{path} ({index}): {NOT_HERE}"));
}

/// A picture from a file's bytes already in memory (PNG, JPEG…), turned as it says.
pub fn read_picture_bytes(bytes: &[u8]) -> Result<Picture, String> {
    #[cfg(target_os = "macos")]
    return mac::picture::read_bytes(bytes);
    #[cfg(not(target_os = "macos"))]
    return Err(format!("{} bytes: {NOT_HERE}", bytes.len()));
}

/// What a video or sound file holds.
pub fn media_info(path: &str) -> Result<MediaInfo, String> {
    #[cfg(target_os = "macos")]
    return mac::asset::info(path);
    #[cfg(not(target_os = "macos"))]
    return Err(format!("{path}: {NOT_HERE}"));
}

/// A file's sound at `rate` samples a second, in `channels` (1 or 2) channels interleaved, its
/// sound tracks mixed; empty when it has none.
pub fn read_sound(path: &str, rate: u32, channels: u32) -> Result<Vec<f32>, String> {
    #[cfg(target_os = "macos")]
    return mac::asset::sound(path, rate, channels);
    #[cfg(not(target_os = "macos"))]
    return Err(format!("{path} ({rate}, {channels}): {NOT_HERE}"));
}

/// A picture written by the system: `kind` "public.jpeg" or "public.tiff", straight eight-bit
/// RGBA (`alpha`: keep it), `quality` 0 to 1 for kinds that lose some.
pub fn write_picture(path: &str, w: usize, h: usize, rgba: &[u8], kind: &str, alpha: bool, quality: f64) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return mac::write::picture(path, w, h, rgba, kind, alpha, quality);
    #[cfg(not(target_os = "macos"))]
    return Err(format!("{path} ({w} × {h}, {} bytes, {kind}, {alpha}, {quality}): {NOT_HERE}", rgba.len()));
}

#[cfg(test)]
mod tests;

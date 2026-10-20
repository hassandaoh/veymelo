//! Video and sound files read through AVFoundation: what a file holds (its length, its
//! picture's size, rate, turn and codec, its sound's rate and channels), its frames one after
//! another from any moment (decoded by the system, hardware where it can), and its sound as
//! floating-point samples at any rate (the system resamples and mixes the channels).

use super::{CFRelease, CMTime, CMTimeRange, Id, NIL, Pool, Sel, boolean, class, dict, error_text, file_url, int, items, msg, number, sel, send, send1, send2, yes};
use crate::{MediaInfo, Picture, SoundTrack, VideoTrack};

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeVideo: Id;
    static AVMediaTypeAudio: Id;
    static AVURLAssetPreferPreciseDurationAndTimingKey: Id;
    static AVFormatIDKey: Id;
    static AVSampleRateKey: Id;
    static AVNumberOfChannelsKey: Id;
    static AVLinearPCMBitDepthKey: Id;
    static AVLinearPCMIsFloatKey: Id;
    static AVLinearPCMIsBigEndianKey: Id;
    static AVLinearPCMIsNonInterleaved: Id;
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMSampleBufferGetImageBuffer(buf: Id) -> Id;
    fn CMSampleBufferGetPresentationTimeStamp(buf: Id) -> CMTime;
    fn CMSampleBufferGetDataBuffer(buf: Id) -> Id;
    fn CMBlockBufferGetDataLength(block: Id) -> usize;
    fn CMBlockBufferCopyDataBytes(block: Id, offset: usize, length: usize, dest: *mut u8) -> i32;
    fn CMFormatDescriptionGetMediaSubType(desc: Id) -> u32;
    fn CMAudioFormatDescriptionGetStreamBasicDescription(desc: Id) -> *const super::write::Asbd;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferPixelFormatTypeKey: Id;
    fn CVPixelBufferLockBaseAddress(pb: Id, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pb: Id, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddress(pb: Id) -> *mut u8;
    fn CVPixelBufferGetBytesPerRow(pb: Id) -> usize;
    fn CVPixelBufferGetWidth(pb: Id) -> usize;
    fn CVPixelBufferGetHeight(pb: Id) -> usize;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    w: f64,
    h: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct CGAffineTransform {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
}

const BGRA: i64 = 0x4247_5241;
const LPCM: i64 = 0x6C70_636D;

fn fourcc(v: u32) -> String {
    v.to_be_bytes().iter().map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '?' }).collect::<String>().trim_end().to_string()
}

/// The asset of a file (autoreleased), or why there is none.
unsafe fn asset(path: &str) -> Result<Id, String> {
    if !std::path::Path::new(path).is_file() {
        return Err(format!("there is no file {path}"));
    }
    let options = dict(&[(unsafe { AVURLAssetPreferPreciseDurationAndTimingKey }, boolean(true))]);
    let a = unsafe { send2(class("AVURLAsset"), "URLAssetWithURL:options:", file_url(path), options) };
    if a.is_null() { Err(format!("{path} is not a file the system can play")) } else { Ok(a) }
}

unsafe fn tracks(asset: Id, kind: Id) -> Vec<Id> {
    unsafe { items(send1(asset, "tracksWithMediaType:", kind)) }
}

unsafe fn duration(obj: Id) -> f64 {
    unsafe { msg!(unsafe extern "C" fn(Id, Sel) -> CMTime)(obj, sel("duration")) }.as_seconds()
}

/// How far a video's pictures are turned to show upright (0, 90, 180, 270, clockwise).
unsafe fn turn(track: Id) -> u32 {
    let t = unsafe { msg!(unsafe extern "C" fn(Id, Sel) -> CGAffineTransform)(track, sel("preferredTransform")) };
    let deg = t.b.atan2(t.a).to_degrees().round() as i64;
    (deg.rem_euclid(360) as u32 + 45) / 90 % 4 * 90
}

/// What a video or sound file holds.
pub fn info(path: &str) -> Result<MediaInfo, String> {
    let _pool = Pool::new();
    unsafe {
        let a = asset(path)?;
        let length = duration(a);
        let video = tracks(a, AVMediaTypeVideo).first().map(|&t| {
            let size = msg!(unsafe extern "C" fn(Id, Sel) -> CGSize)(t, sel("naturalSize"));
            let fps = msg!(unsafe extern "C" fn(Id, Sel) -> f32)(t, sel("nominalFrameRate")) as f64;
            let desc = items(send(t, "formatDescriptions")).first().copied();
            let codec = desc.map_or(String::new(), |d| fourcc(CMFormatDescriptionGetMediaSubType(d)));
            let turn = turn(t);
            let (w, h) = (size.w.abs().round() as u32, size.h.abs().round() as u32);
            let (width, height) = if turn % 180 == 90 { (h, w) } else { (w, h) };
            // ProRes 4444 (and XQ) keep alpha
            let alpha = matches!(codec.as_str(), "ap4h" | "ap4x");
            VideoTrack { width, height, fps, turn, codec, alpha }
        });
        let sound = tracks(a, AVMediaTypeAudio).first().map(|&t| {
            let desc = items(send(t, "formatDescriptions")).first().copied();
            let asbd = desc.map(|d| CMAudioFormatDescriptionGetStreamBasicDescription(d)).filter(|p| !p.is_null()).map(|p| *p);
            SoundTrack { rate: asbd.map_or(0.0, |a| a.sample_rate), channels: asbd.map_or(0, |a| a.channels_per_frame), codec: desc.map_or(String::new(), |d| fourcc(CMFormatDescriptionGetMediaSubType(d))) }
        });
        if video.is_none() && sound.is_none() {
            return Err(format!("{path} has neither pictures nor sound the system can read"));
        }
        Ok(MediaInfo { duration: length, video, sound })
    }
}

/// A video's frames one after another from a moment, decoded by the system.
pub struct Frames {
    reader: Id,
    output: Id,
    turn: u32,
    alpha: bool,
}

impl Frames {
    /// The frames of `path` from `from` seconds on (the frame showing then comes first).
    pub fn open(path: &str, from: f64) -> Result<Frames, String> {
        let _pool = Pool::new();
        unsafe {
            let a = asset(path)?;
            let Some(&track) = tracks(a, AVMediaTypeVideo).first() else { return Err(format!("{path} has no pictures")) };
            let mut err: Id = NIL;
            let reader = msg!(unsafe extern "C" fn(Id, Sel, Id, *mut Id) -> Id)(class("AVAssetReader"), sel("assetReaderWithAsset:error:"), a, &mut err);
            if reader.is_null() {
                return Err(format!("the system cannot read {path}: {}", error_text(err)));
            }
            let settings = dict(&[(kCVPixelBufferPixelFormatTypeKey, int(BGRA))]);
            let output = send2(class("AVAssetReaderTrackOutput"), "assetReaderTrackOutputWithTrack:outputSettings:", track, settings);
            msg!(unsafe extern "C" fn(Id, Sel, bool))(output, sel("setAlwaysCopiesSampleData:"), false);
            if !msg!(unsafe extern "C" fn(Id, Sel, Id) -> bool)(reader, sel("canAddOutput:"), output) {
                return Err(format!("the system cannot give the pictures of {path}"));
            }
            send1(reader, "addOutput:", output);
            let length = duration(a);
            let from = from.clamp(0.0, length.max(0.0));
            let range = CMTimeRange { start: CMTime::seconds(from), duration: CMTime::seconds((length - from).max(0.0) + 1.0) };
            msg!(unsafe extern "C" fn(Id, Sel, CMTimeRange))(reader, sel("setTimeRange:"), range);
            if !yes(reader, "startReading") {
                return Err(format!("the system cannot read {path}: {}", error_text(send(reader, "error"))));
            }
            let codec = items(send(track, "formatDescriptions")).first().map_or(String::new(), |&d| fourcc(CMFormatDescriptionGetMediaSubType(d)));
            // kept past this pool (the reader keeps its asset and output)
            send(reader, "retain");
            Ok(Frames { reader, output, turn: turn(track), alpha: matches!(codec.as_str(), "ap4h" | "ap4x") })
        }
    }

    /// The next frame and the time it shows from (seconds), or none at the end.
    pub fn next_frame(&mut self) -> Option<Result<(f64, Picture), String>> {
        let _pool = Pool::new();
        unsafe {
            loop {
                let buf = send(self.output, "copyNextSampleBuffer");
                if buf.is_null() {
                    // 3: failed
                    if super::integer(self.reader, "status") == 3 {
                        return Some(Err(format!("the system stopped reading the video: {}", error_text(send(self.reader, "error")))));
                    }
                    return None;
                }
                let at = CMSampleBufferGetPresentationTimeStamp(buf).as_seconds();
                let pb = CMSampleBufferGetImageBuffer(buf);
                if pb.is_null() {
                    CFRelease(buf);
                    continue;
                }
                CVPixelBufferLockBaseAddress(pb, 1);
                let (w, h, row, base) = (CVPixelBufferGetWidth(pb), CVPixelBufferGetHeight(pb), CVPixelBufferGetBytesPerRow(pb), CVPixelBufferGetBaseAddress(pb));
                let mut px = vec![[0.0f32; 4]; w * h];
                if !base.is_null() {
                    for y in 0..h {
                        let line = std::slice::from_raw_parts(base.add(y * row), w * 4);
                        for (p, b) in px[y * w..(y + 1) * w].iter_mut().zip(line.as_chunks::<4>().0) {
                            // blue, green, red, alpha (straight)
                            let a = if self.alpha { b[3] as f32 / 255.0 } else { 1.0 };
                            *p = [b[2] as f32 / 255.0 * a, b[1] as f32 / 255.0 * a, b[0] as f32 / 255.0 * a, a];
                        }
                    }
                }
                CVPixelBufferUnlockBaseAddress(pb, 1);
                CFRelease(buf);
                let o = match self.turn {
                    90 => 6,
                    180 => 3,
                    270 => 8,
                    _ => 1,
                };
                return Some(Ok((at, crate::turned(Picture { width: w, height: h, px, alpha: self.alpha }, o))));
            }
        }
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        unsafe {
            send(self.reader, "cancelReading");
            send(self.reader, "release");
        }
    }
}

/// A file's sound at `rate` samples a second in `channels` (1 or 2) channels, interleaved,
/// all its sound tracks mixed; empty when it has none.
pub fn sound(path: &str, rate: u32, channels: u32) -> Result<Vec<f32>, String> {
    let _pool = Pool::new();
    let channels = channels.clamp(1, 2);
    unsafe {
        let a = asset(path)?;
        let list = send1(a, "tracksWithMediaType:", AVMediaTypeAudio);
        if items(list).is_empty() {
            return Ok(Vec::new());
        }
        let mut err: Id = NIL;
        let reader = msg!(unsafe extern "C" fn(Id, Sel, Id, *mut Id) -> Id)(class("AVAssetReader"), sel("assetReaderWithAsset:error:"), a, &mut err);
        if reader.is_null() {
            return Err(format!("the system cannot read {path}: {}", error_text(err)));
        }
        let settings = dict(&[
            (AVFormatIDKey, int(LPCM)),
            (AVSampleRateKey, number(rate as f64)),
            (AVNumberOfChannelsKey, int(channels as i64)),
            (AVLinearPCMBitDepthKey, int(32)),
            (AVLinearPCMIsFloatKey, boolean(true)),
            (AVLinearPCMIsBigEndianKey, boolean(false)),
            (AVLinearPCMIsNonInterleaved, boolean(false)),
        ]);
        let output = send2(class("AVAssetReaderAudioMixOutput"), "assetReaderAudioMixOutputWithAudioTracks:audioSettings:", list, settings);
        if output.is_null() || !msg!(unsafe extern "C" fn(Id, Sel, Id) -> bool)(reader, sel("canAddOutput:"), output) {
            return Err(format!("the system cannot give the sound of {path}"));
        }
        send1(reader, "addOutput:", output);
        if !yes(reader, "startReading") {
            return Err(format!("the system cannot read {path}: {}", error_text(send(reader, "error"))));
        }
        let mut out: Vec<f32> = Vec::new();
        loop {
            let inner = Pool::new();
            let buf = send(output, "copyNextSampleBuffer");
            if buf.is_null() {
                drop(inner);
                break;
            }
            let block = CMSampleBufferGetDataBuffer(buf);
            if !block.is_null() {
                let n = CMBlockBufferGetDataLength(block);
                let mut bytes = vec![0u8; n];
                if CMBlockBufferCopyDataBytes(block, 0, n, bytes.as_mut_ptr()) == 0 {
                    out.extend(bytes.as_chunks::<4>().0.iter().map(|b| f32::from_ne_bytes(*b)));
                }
            }
            CFRelease(buf);
            drop(inner);
        }
        if super::integer(reader, "status") == 3 {
            return Err(format!("the system stopped reading the sound of {path}: {}", error_text(send(reader, "error"))));
        }
        Ok(out)
    }
}

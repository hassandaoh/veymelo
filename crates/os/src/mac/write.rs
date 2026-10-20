//! Writing through the system's encoders: video (and its sound) through AVFoundation's asset
//! writer (H.264, HEVC, ProRes; AAC or PCM sound), pictures through ImageIO (JPEG, TIFF).

use std::ffi::c_void;

use super::{CFDataCreate, CFRelease, CFURLCreateFromFileSystemRepresentation, CMTime, Id, NIL, Pool, Sel, boolean, class, dict, error_text, int, msg, nsstring, number, objc_autoreleasePoolPop, objc_autoreleasePoolPush, sel, send, send1, send2, yes};

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVFileTypeMPEG4: Id;
    static AVFileTypeQuickTimeMovie: Id;
    static AVMediaTypeVideo: Id;
    static AVMediaTypeAudio: Id;
    static AVVideoCodecKey: Id;
    static AVVideoCodecTypeH264: Id;
    static AVVideoCodecTypeHEVC: Id;
    static AVVideoCodecTypeAppleProRes422: Id;
    static AVVideoCodecTypeAppleProRes422HQ: Id;
    static AVVideoCodecTypeAppleProRes422LT: Id;
    static AVVideoCodecTypeAppleProRes422Proxy: Id;
    static AVVideoCodecTypeAppleProRes4444: Id;
    static AVVideoWidthKey: Id;
    static AVVideoHeightKey: Id;
    static AVVideoCompressionPropertiesKey: Id;
    static AVVideoAverageBitRateKey: Id;
    static AVVideoMaxKeyFrameIntervalKey: Id;
    static AVVideoQualityKey: Id;
    static AVVideoProfileLevelKey: Id;
    static AVVideoProfileLevelH264HighAutoLevel: Id;
    static AVVideoColorPropertiesKey: Id;
    static AVVideoColorPrimariesKey: Id;
    static AVVideoColorPrimaries_ITU_R_709_2: Id;
    static AVVideoTransferFunctionKey: Id;
    static AVVideoTransferFunction_ITU_R_709_2: Id;
    static AVVideoYCbCrMatrixKey: Id;
    static AVVideoYCbCrMatrix_ITU_R_709_2: Id;
    static AVFormatIDKey: Id;
    static AVSampleRateKey: Id;
    static AVNumberOfChannelsKey: Id;
    static AVEncoderBitRateKey: Id;
    static AVLinearPCMBitDepthKey: Id;
    static AVLinearPCMIsFloatKey: Id;
    static AVLinearPCMIsBigEndianKey: Id;
    static AVLinearPCMIsNonInterleaved: Id;
    static AVChannelLayoutKey: Id;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferPixelFormatTypeKey: Id;
    static kCVPixelBufferWidthKey: Id;
    static kCVPixelBufferHeightKey: Id;
    fn CVPixelBufferPoolCreatePixelBuffer(alloc: Id, pool: Id, out: *mut Id) -> i32;
    fn CVPixelBufferLockBaseAddress(pb: Id, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pb: Id, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddress(pb: Id) -> *mut u8;
    fn CVPixelBufferGetBytesPerRow(pb: Id) -> usize;
    fn CVPixelBufferRelease(pb: Id);
}

/// How sound is laid out in memory (Core Audio's AudioStreamBasicDescription).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Asbd {
    pub sample_rate: f64,
    pub format_id: u32,
    pub format_flags: u32,
    pub bytes_per_packet: u32,
    pub frames_per_packet: u32,
    pub bytes_per_frame: u32,
    pub channels_per_frame: u32,
    pub bits_per_channel: u32,
    pub reserved: u32,
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMAudioFormatDescriptionCreate(alloc: Id, asbd: *const Asbd, layout_size: usize, layout: *const c_void, cookie_size: usize, cookie: *const c_void, extensions: Id, out: *mut Id) -> i32;
    fn CMBlockBufferCreateWithMemoryBlock(alloc: Id, block: *mut c_void, length: usize, block_alloc: Id, source: *const c_void, offset: usize, data_length: usize, flags: u32, out: *mut Id) -> i32;
    fn CMBlockBufferReplaceDataBytes(source: *const c_void, dest: Id, offset: usize, length: usize) -> i32;
    fn CMAudioSampleBufferCreateReadyWithPacketDescriptions(alloc: Id, data: Id, format: Id, samples: isize, pts: CMTime, packets: *const c_void, out: *mut Id) -> i32;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGColorSpaceSRGB: Id;
    fn CGColorSpaceCreateWithName(name: Id) -> Id;
    fn CGDataProviderCreateWithCFData(data: Id) -> Id;
    fn CGImageCreate(width: usize, height: usize, bits: usize, bits_per_pixel: usize, row: usize, space: Id, info: u32, provider: Id, decode: *const f64, interpolate: bool, intent: i32) -> Id;
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    static kCGImageDestinationLossyCompressionQuality: Id;
    fn CGImageDestinationCreateWithURL(url: Id, kind: Id, count: usize, options: Id) -> Id;
    fn CGImageDestinationAddImage(dest: Id, image: Id, props: Id);
    fn CGImageDestinationFinalize(dest: Id) -> bool;
}

/// Linear PCM, AAC.
const LPCM: i64 = 0x6C70_636D;
const AAC: i64 = 0x6161_6320;

/// The video kinds the system writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Codec {
    H264,
    Hevc,
    /// ProRes: proxy, lt, 422, hq, 4444, 4444 xq.
    ProRes(u8),
}

/// The sound a video carries: its rate, how many channels, and AAC at a bit rate (bits a
/// second) or, at 0, 24-bit PCM (QuickTime only).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sound {
    pub rate: u32,
    pub channels: u32,
    pub aac_bitrate: u32,
}

/// A channel layout (Core Audio's AudioChannelLayout) for mono or stereo, as bytes.
fn layout(channels: u32) -> Id {
    // the layout tag: mono (100 << 16 | 1) or stereo (101 << 16 | 2)
    let tag: u32 = if channels == 1 { 100 << 16 | 1 } else { 101 << 16 | 2 };
    let mut bytes = Vec::new();
    for v in [tag, 0, 0] {
        bytes.extend(v.to_ne_bytes());
    }
    unsafe { msg!(unsafe extern "C" fn(Id, Sel, *const u8, usize) -> Id)(class("NSData"), sel("dataWithBytes:length:"), bytes.as_ptr(), bytes.len()) }
}

/// A video file being written by the system's encoder.
pub struct Video {
    pool: *mut c_void,
    writer: Id,
    input: Id,
    adaptor: Id,
    w: usize,
    h: usize,
    /// Each frame is `den` / `num` seconds.
    num: i32,
    den: i64,
    frames: i64,
    sound: Option<(Id, Sound, Id)>,
    /// Sound frames written, samples given but not yet taken, and whether the encoder was told
    /// the sound is over.
    heard: i64,
    waiting: std::collections::VecDeque<f32>,
    sound_done: bool,
    /// Frames made but not yet taken, with their times.
    pictures: std::collections::VecDeque<(Id, CMTime)>,
}

impl Video {
    /// A video at `path`, `w` × `h` (H.264 and HEVC need them even), `num` / `den` frames a
    /// second; `quality` 0 to 1 or `bitrate` (bits a second, 0: by the quality), a whole picture
    /// every `keyframes` frames (0: the encoder's choice); with `sound`, a sound track too.
    #[allow(clippy::too_many_arguments)]
    pub fn new(path: &str, w: usize, h: usize, num: i32, den: i64, codec: Codec, quality: f64, bitrate: u64, keyframes: u32, sound: Option<Sound>) -> Result<Video, String> {
        let pool = unsafe { objc_autoreleasePoolPush() };
        let fail = |e: String| {
            unsafe { objc_autoreleasePoolPop(pool) };
            Err(e)
        };
        let _ = std::fs::remove_file(path);
        unsafe {
            let url = super::file_url(path);
            let mov = matches!(codec, Codec::ProRes(_));
            let mut err: Id = NIL;
            let writer = msg!(unsafe extern "C" fn(Id, Sel, Id, Id, *mut Id) -> Id)(send(class("AVAssetWriter"), "alloc"), sel("initWithURL:fileType:error:"), url, if mov { AVFileTypeQuickTimeMovie } else { AVFileTypeMPEG4 }, &mut err);
            if writer.is_null() {
                return fail(format!("cannot make the video file: {}", error_text(err)));
            }
            let codec_id = match codec {
                Codec::H264 => AVVideoCodecTypeH264,
                Codec::Hevc => AVVideoCodecTypeHEVC,
                Codec::ProRes(0) => AVVideoCodecTypeAppleProRes422Proxy,
                Codec::ProRes(1) => AVVideoCodecTypeAppleProRes422LT,
                Codec::ProRes(3) => AVVideoCodecTypeAppleProRes422HQ,
                Codec::ProRes(4) => AVVideoCodecTypeAppleProRes4444,
                Codec::ProRes(5) => nsstring("ap4x"),
                Codec::ProRes(_) => AVVideoCodecTypeAppleProRes422,
            };
            let mut compression: Vec<(Id, Id)> = Vec::new();
            if !mov {
                if bitrate > 0 {
                    compression.push((AVVideoAverageBitRateKey, int(bitrate as i64)));
                } else {
                    // by the quality: bits for each pixel a second, more for better
                    let bits = (w * h) as f64 * (num as f64 / den as f64) * (0.02 + 0.28 * quality * quality);
                    compression.push((AVVideoAverageBitRateKey, int(bits as i64)));
                    if codec == Codec::Hevc {
                        compression.push((AVVideoQualityKey, number(quality.clamp(0.0, 1.0))));
                    }
                }
                if keyframes > 0 {
                    compression.push((AVVideoMaxKeyFrameIntervalKey, int(keyframes as i64)));
                }
                if codec == Codec::H264 {
                    compression.push((AVVideoProfileLevelKey, AVVideoProfileLevelH264HighAutoLevel));
                }
            }
            let colour = dict(&[(AVVideoColorPrimariesKey, AVVideoColorPrimaries_ITU_R_709_2), (AVVideoTransferFunctionKey, AVVideoTransferFunction_ITU_R_709_2), (AVVideoYCbCrMatrixKey, AVVideoYCbCrMatrix_ITU_R_709_2)]);
            let mut settings = vec![(AVVideoCodecKey, codec_id), (AVVideoWidthKey, int(w as i64)), (AVVideoHeightKey, int(h as i64)), (AVVideoColorPropertiesKey, colour)];
            if !compression.is_empty() {
                settings.push((AVVideoCompressionPropertiesKey, dict(&compression)));
            }
            let input = send2(send(class("AVAssetWriterInput"), "alloc"), "initWithMediaType:outputSettings:", AVMediaTypeVideo, dict(&settings));
            if input.is_null() {
                return fail("the system cannot make that kind of video".into());
            }
            msg!(unsafe extern "C" fn(Id, Sel, bool))(input, sel("setExpectsMediaDataInRealTime:"), false);
            // 32-bit BGRA pictures
            let attrs = dict(&[(kCVPixelBufferPixelFormatTypeKey, int(0x4247_5241)), (kCVPixelBufferWidthKey, int(w as i64)), (kCVPixelBufferHeightKey, int(h as i64))]);
            let adaptor = send2(send(class("AVAssetWriterInputPixelBufferAdaptor"), "alloc"), "initWithAssetWriterInput:sourcePixelBufferAttributes:", input, attrs);
            if !msg!(unsafe extern "C" fn(Id, Sel, Id) -> bool)(writer, sel("canAddInput:"), input) {
                return fail("the video file cannot take that video".into());
            }
            send1(writer, "addInput:", input);
            let mut track = None;
            if let Some(s) = sound {
                let channels = s.channels.clamp(1, 2);
                let mut a = vec![(AVSampleRateKey, number(s.rate as f64)), (AVNumberOfChannelsKey, int(channels as i64)), (AVChannelLayoutKey, layout(channels))];
                if s.aac_bitrate > 0 || !mov {
                    a.push((AVFormatIDKey, int(AAC)));
                    a.push((AVEncoderBitRateKey, int(if s.aac_bitrate > 0 { s.aac_bitrate } else { 256_000 } as i64)));
                } else {
                    a.extend([(AVFormatIDKey, int(LPCM)), (AVLinearPCMBitDepthKey, int(24)), (AVLinearPCMIsFloatKey, boolean(false)), (AVLinearPCMIsBigEndianKey, boolean(false)), (AVLinearPCMIsNonInterleaved, boolean(false))]);
                }
                let ain = send2(send(class("AVAssetWriterInput"), "alloc"), "initWithMediaType:outputSettings:", AVMediaTypeAudio, dict(&a));
                if ain.is_null() || !msg!(unsafe extern "C" fn(Id, Sel, Id) -> bool)(writer, sel("canAddInput:"), ain) {
                    return fail("the video file cannot take that sound".into());
                }
                msg!(unsafe extern "C" fn(Id, Sel, bool))(ain, sel("setExpectsMediaDataInRealTime:"), false);
                send1(writer, "addInput:", ain);
                // what the samples given are: 32-bit floats, interleaved
                let asbd = Asbd { sample_rate: s.rate as f64, format_id: LPCM as u32, format_flags: 1 | 8, bytes_per_packet: 4 * channels, frames_per_packet: 1, bytes_per_frame: 4 * channels, channels_per_frame: channels, bits_per_channel: 32, reserved: 0 };
                let mut format: Id = NIL;
                if CMAudioFormatDescriptionCreate(NIL, &asbd, 0, std::ptr::null(), 0, std::ptr::null(), NIL, &mut format) != 0 || format.is_null() {
                    return fail("the system cannot describe the sound".into());
                }
                track = Some((ain, Sound { channels, ..s }, format));
            }
            if !yes(writer, "startWriting") {
                return fail(format!("cannot start writing the video: {}", error_text(send(writer, "error"))));
            }
            msg!(unsafe extern "C" fn(Id, Sel, CMTime))(writer, sel("startSessionAtSourceTime:"), CMTime::new(0, num.max(1)));
            Ok(Video { pool, writer, input, adaptor, w, h, num: num.max(1), den: den.max(1), frames: 0, sound: track, heard: 0, waiting: Default::default(), sound_done: false, pictures: Default::default() })
        }
    }

    /// Fails when the writer has failed.
    fn check(&self) -> Result<(), String> {
        if unsafe { super::integer(self.writer, "status") } == 3 {
            return Err(format!("the video encoder failed: {}", error_text(unsafe { send(self.writer, "error") })));
        }
        Ok(())
    }

    /// The next frame: straight eight-bit RGBA, `w` × `h`. It is given to the encoder as it
    /// takes it (it may wait for the sound to go with it).
    pub fn frame(&mut self, rgba: &[u8]) -> Result<(), String> {
        unsafe {
            let pool = send(self.adaptor, "pixelBufferPool");
            let mut pb: Id = NIL;
            if pool.is_null() || CVPixelBufferPoolCreatePixelBuffer(NIL, pool, &mut pb) != 0 || pb.is_null() {
                return Err(format!("no picture buffer for the video: {}", error_text(send(self.writer, "error"))));
            }
            CVPixelBufferLockBaseAddress(pb, 0);
            let base = CVPixelBufferGetBaseAddress(pb);
            let row = CVPixelBufferGetBytesPerRow(pb);
            for y in 0..self.h {
                let dst = std::slice::from_raw_parts_mut(base.add(y * row), self.w * 4);
                let src = &rgba[y * self.w * 4..(y + 1) * self.w * 4];
                for (d, s) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<4>().0) {
                    d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
                }
            }
            CVPixelBufferUnlockBaseAddress(pb, 0);
            self.pictures.push_back((pb, CMTime::new(self.frames * self.den, self.num)));
        }
        self.frames += 1;
        self.pump()?;
        // not too many frames held: wait for the encoder (it takes them as the sound comes)
        let mut waited = 0;
        while self.pictures.len() > 64 {
            if !self.pump()? {
                self.check()?;
                std::thread::sleep(std::time::Duration::from_millis(1));
                waited += 1;
                if waited > 60_000 {
                    return Err("the video encoder stopped taking frames".into());
                }
            }
        }
        Ok(())
    }

    /// Whether it has a sound track.
    pub fn has_sound(&self) -> bool {
        self.sound.is_some()
    }

    /// More sound: interleaved 32-bit float samples, as many channels as the track has. It is
    /// given to the encoder as it takes it (it may wait for the pictures to go with it).
    pub fn sound(&mut self, samples: &[f32]) -> Result<(), String> {
        if self.sound.is_none() {
            return Err("this video has no sound track".into());
        }
        self.waiting.extend(samples);
        self.pump()?;
        Ok(())
    }

    /// Gives the encoder what is waiting that it takes now, pictures and sound; whether it took
    /// any.
    fn pump(&mut self) -> Result<bool, String> {
        let mut took = false;
        loop {
            let mut more = false;
            // pictures
            if let Some(&(pb, at)) = self.pictures.front()
                && unsafe { yes(self.input, "isReadyForMoreMediaData") }
            {
                let ok = unsafe { msg!(unsafe extern "C" fn(Id, Sel, Id, CMTime) -> bool)(self.adaptor, sel("appendPixelBuffer:withPresentationTime:"), pb, at) };
                unsafe { CVPixelBufferRelease(pb) };
                self.pictures.pop_front();
                if !ok {
                    return Err(format!("the video encoder would not take a frame: {}", error_text(unsafe { send(self.writer, "error") })));
                }
                more = true;
            }
            // sound, in pieces of about a tenth of a second
            if let Some((input, s, format)) = self.sound {
                let per = s.channels as usize;
                let n = (s.rate as usize / 10).max(1).min(self.waiting.len() / per);
                if n > 0 && !self.sound_done && unsafe { yes(input, "isReadyForMoreMediaData") } {
                    let piece: Vec<f32> = self.waiting.drain(..n * per).collect();
                    unsafe {
                        let bytes = n * per * 4;
                        let mut block: Id = NIL;
                        if CMBlockBufferCreateWithMemoryBlock(NIL, std::ptr::null_mut(), bytes, NIL, std::ptr::null(), 0, bytes, 1, &mut block) != 0 || block.is_null() {
                            return Err("no buffer for the sound".into());
                        }
                        CMBlockBufferReplaceDataBytes(piece.as_ptr().cast(), block, 0, bytes);
                        let mut buf: Id = NIL;
                        let status = CMAudioSampleBufferCreateReadyWithPacketDescriptions(NIL, block, format, n as isize, CMTime::new(self.heard, s.rate as i32), std::ptr::null(), &mut buf);
                        CFRelease(block);
                        if status != 0 || buf.is_null() {
                            return Err(format!("the system cannot hold that sound ({status})"));
                        }
                        let ok = msg!(unsafe extern "C" fn(Id, Sel, Id) -> bool)(input, sel("appendSampleBuffer:"), buf);
                        CFRelease(buf);
                        if !ok {
                            return Err(format!("the encoder would not take the sound: {}", error_text(send(self.writer, "error"))));
                        }
                    }
                    self.heard += n as i64;
                    more = true;
                }
            }
            if !more {
                return Ok(took);
            }
            took = true;
        }
    }

    /// The file finished.
    pub fn finish(mut self) -> Result<(), String> {
        // everything waiting given; when the sound runs out first the encoder is told, so it
        // stops waiting for more of it
        let mut waited = 0;
        while !self.pictures.is_empty() || (self.sound.is_some() && !self.sound_done && !self.waiting.is_empty()) {
            if self.pump()? {
                continue;
            }
            self.check()?;
            if let Some((input, s, _)) = self.sound
                && !self.sound_done
                && self.waiting.len() < s.channels as usize
            {
                unsafe { send(input, "markAsFinished") };
                self.sound_done = true;
                continue;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
            waited += 1;
            if waited > 60_000 {
                return Err("the encoder stopped taking frames".into());
            }
        }
        unsafe {
            send(self.input, "markAsFinished");
            if let Some((input, _, _)) = self.sound
                && !self.sound_done
            {
                send(input, "markAsFinished");
            }
            // finished off the main thread, as the system asks (it waits there)
            let writer = self.writer as usize;
            let ok = std::thread::spawn(move || {
                let _pool = Pool::new();
                yes(writer as Id, "finishWriting")
            })
            .join()
            .unwrap_or(false);
            let status = super::integer(self.writer, "status");
            let why = error_text(send(self.writer, "error"));
            if let Some((input, _, format)) = self.sound {
                send(input, "release");
                CFRelease(format);
            }
            for o in [self.adaptor, self.input, self.writer] {
                send(o, "release");
            }
            objc_autoreleasePoolPop(self.pool);
            // 2: completed
            if !ok || status != 2 { Err(format!("the video could not be finished: {why}")) } else { Ok(()) }
        }
    }
}

/// A picture written by ImageIO: `kind` "public.jpeg" or "public.tiff", straight eight-bit
/// RGBA (`alpha`: keep it), `quality` 0 to 1 for those that lose some.
pub fn picture(path: &str, w: usize, h: usize, rgba: &[u8], kind: &str, alpha: bool, quality: f64) -> Result<(), String> {
    let _pool = Pool::new();
    unsafe {
        let data = CFDataCreate(NIL, rgba.as_ptr(), rgba.len() as isize);
        let provider = CGDataProviderCreateWithCFData(data);
        let space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        // 3: alpha last, straight; 5: alpha last, left out
        let image = CGImageCreate(w, h, 8, 32, w * 4, space, if alpha { 3 } else { 5 }, provider, std::ptr::null(), false, 0);
        let url = CFURLCreateFromFileSystemRepresentation(NIL, path.as_ptr(), path.len() as isize, false);
        let dest = CGImageDestinationCreateWithURL(url, nsstring(kind), 1, NIL);
        let result = if dest.is_null() || image.is_null() {
            Err(format!("cannot write {path}"))
        } else {
            CGImageDestinationAddImage(dest, image, dict(&[(kCGImageDestinationLossyCompressionQuality, number(quality.clamp(0.0, 1.0)))]));
            if CGImageDestinationFinalize(dest) { Ok(()) } else { Err(format!("cannot write {path}")) }
        };
        for o in [dest, url, image, space, provider, data] {
            if !o.is_null() {
                CFRelease(o);
            }
        }
        result
    }
}

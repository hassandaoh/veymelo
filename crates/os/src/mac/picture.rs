//! Pictures read through ImageIO: every format the system reads (PNG, JPEG, HEIC, TIFF, GIF,
//! WebP, BMP, OpenEXR, Radiance HDR, Photoshop's merged picture…), the frames of animated
//! ones with their times, and the turn a camera wrote into the file (EXIF orientation).
//!
//! Pixels come out premultiplied in floating point, in sRGB (extended past white for pictures
//! brighter than it), colour-managed from whatever space the file is in.

use super::{CFDataCreate, CFRelease, Id, NIL, Pool, dict_number, file_url, send1};
use crate::{Picture, PictureInfo};

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    static kCGImagePropertyPixelWidth: Id;
    static kCGImagePropertyPixelHeight: Id;
    static kCGImagePropertyHasAlpha: Id;
    static kCGImagePropertyOrientation: Id;
    static kCGImagePropertyDepth: Id;
    static kCGImagePropertyGIFDictionary: Id;
    static kCGImagePropertyGIFDelayTime: Id;
    static kCGImagePropertyGIFUnclampedDelayTime: Id;
    static kCGImagePropertyPNGDictionary: Id;
    static kCGImagePropertyAPNGDelayTime: Id;
    static kCGImagePropertyAPNGUnclampedDelayTime: Id;
    static kCGImagePropertyWebPDictionary: Id;
    static kCGImagePropertyWebPDelayTime: Id;
    static kCGImagePropertyWebPUnclampedDelayTime: Id;
    static kCGImagePropertyHEICSDictionary: Id;
    static kCGImagePropertyHEICSDelayTime: Id;
    static kCGImagePropertyHEICSUnclampedDelayTime: Id;
    fn CGImageSourceCreateWithURL(url: Id, options: Id) -> Id;
    fn CGImageSourceCreateWithData(data: Id, options: Id) -> Id;
    fn CGImageSourceGetCount(src: Id) -> usize;
    fn CGImageSourceGetType(src: Id) -> Id;
    fn CGImageSourceCopyPropertiesAtIndex(src: Id, index: usize, options: Id) -> Id;
    fn CGImageSourceCreateImageAtIndex(src: Id, index: usize, options: Id) -> Id;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGColorSpaceExtendedSRGB: Id;
    static kCGColorSpaceSRGB: Id;
    fn CGColorSpaceCreateWithName(name: Id) -> Id;
    fn CGImageGetWidth(image: Id) -> usize;
    fn CGImageGetHeight(image: Id) -> usize;
    fn CGBitmapContextCreate(data: *mut u8, w: usize, h: usize, bits: usize, row: usize, space: Id, info: u32) -> Id;
    fn CGContextDrawImage(ctx: Id, rect: CGRect, image: Id);
    fn CGContextRelease(ctx: Id);
}

/// The source of a file, or why there is none.
fn source(path: &str) -> Result<Id, String> {
    if !std::path::Path::new(path).is_file() {
        return Err(format!("there is no file {path}"));
    }
    let src = unsafe { CGImageSourceCreateWithURL(file_url(path), NIL) };
    if src.is_null() { Err(format!("{path} is not a picture the system can read")) } else { Ok(src) }
}

/// How long frame `props` shows, from whichever animated format's own dictionary it has.
unsafe fn delay(props: Id) -> Option<f64> {
    let kinds = unsafe {
        [
            (kCGImagePropertyGIFDictionary, kCGImagePropertyGIFUnclampedDelayTime, kCGImagePropertyGIFDelayTime),
            (kCGImagePropertyPNGDictionary, kCGImagePropertyAPNGUnclampedDelayTime, kCGImagePropertyAPNGDelayTime),
            (kCGImagePropertyWebPDictionary, kCGImagePropertyWebPUnclampedDelayTime, kCGImagePropertyWebPDelayTime),
            (kCGImagePropertyHEICSDictionary, kCGImagePropertyHEICSUnclampedDelayTime, kCGImagePropertyHEICSDelayTime),
        ]
    };
    for (dict, unclamped, clamped) in kinds {
        let d = unsafe { send1(props, "objectForKey:", dict) };
        if d.is_null() {
            continue;
        }
        let t = unsafe { dict_number(d, unclamped).filter(|t| *t > 0.0).or_else(|| dict_number(d, clamped)) };
        if t.is_some() {
            return t;
        }
    }
    None
}

/// What a picture file is: its size (as it shows, turned), how many frames and how long each
/// shows, whether it has alpha.
pub fn info(path: &str) -> Result<PictureInfo, String> {
    let _pool = Pool::new();
    let src = source(path)?;
    unsafe {
        let frames = CGImageSourceGetCount(src);
        let kind = super::rust_string(CGImageSourceGetType(src));
        let props = CGImageSourceCopyPropertiesAtIndex(src, 0, NIL);
        let get = |k: Id| dict_number(props, k);
        let (w, h) = (get(kCGImagePropertyPixelWidth).unwrap_or(0.0) as u32, get(kCGImagePropertyPixelHeight).unwrap_or(0.0) as u32);
        let orientation = get(kCGImagePropertyOrientation).map_or(1, |o| o as u8).clamp(1, 8);
        let alpha = get(kCGImagePropertyHasAlpha).is_some_and(|a| a != 0.0);
        let depth = get(kCGImagePropertyDepth).map_or(8, |d| d as u32);
        let mut delays = Vec::new();
        if frames > 1 {
            for i in 0..frames {
                let p = if i == 0 { props } else { CGImageSourceCopyPropertiesAtIndex(src, i, NIL) };
                // browsers show frames under a fiftieth of a second for a tenth; so do we
                delays.push(delay(p).map_or(0.1, |d| if d < 0.02 { 0.1 } else { d }));
                if i > 0 && !p.is_null() {
                    CFRelease(p);
                }
            }
        }
        if !props.is_null() {
            CFRelease(props);
        }
        CFRelease(src);
        if w == 0 || h == 0 {
            return Err(format!("{path} has no size the system can read"));
        }
        let (width, height) = if orientation >= 5 { (h, w) } else { (w, h) };
        Ok(PictureInfo { width, height, frames: frames.max(1), delays, alpha, orientation, depth, kind })
    }
}

/// Frame `index` of a picture file, turned as the file says.
pub fn read(path: &str, index: usize) -> Result<Picture, String> {
    let _pool = Pool::new();
    let src = source(path)?;
    read_from(src, path, index)
}

/// A picture file's bytes, already in memory (a texture inside a 3D model), turned as it says.
pub fn read_bytes(bytes: &[u8]) -> Result<Picture, String> {
    let _pool = Pool::new();
    let src = unsafe {
        let data = CFDataCreate(NIL, bytes.as_ptr(), bytes.len() as isize);
        if data.is_null() {
            return Err("the system cannot hold those bytes".into());
        }
        let src = CGImageSourceCreateWithData(data, NIL);
        CFRelease(data);
        src
    };
    if src.is_null() {
        return Err("those bytes are not a picture the system can read".into());
    }
    read_from(src, "the picture", 0)
}

/// Frame `index` of picture source `src` (released here), called `path` in what goes wrong.
fn read_from(src: Id, path: &str, index: usize) -> Result<Picture, String> {
    unsafe {
        let props = CGImageSourceCopyPropertiesAtIndex(src, index, NIL);
        let orientation = dict_number(props, kCGImagePropertyOrientation).map_or(1, |o| o as u8).clamp(1, 8);
        let alpha = dict_number(props, kCGImagePropertyHasAlpha).is_some_and(|a| a != 0.0);
        if !props.is_null() {
            CFRelease(props);
        }
        let image = CGImageSourceCreateImageAtIndex(src, index, NIL);
        CFRelease(src);
        if image.is_null() {
            return Err(format!("{path} has no frame {index}"));
        }
        let (w, h) = (CGImageGetWidth(image), CGImageGetHeight(image));
        let mut px = vec![[0.0f32; 4]; w * h];
        // floating point, premultiplied, alpha last, little-endian; else eight bits
        let space = CGColorSpaceCreateWithName(kCGColorSpaceExtendedSRGB);
        let ctx = CGBitmapContextCreate(px.as_mut_ptr().cast(), w, h, 32, w * 16, space, 1 | 1 << 8 | 2 << 12);
        CFRelease(space);
        if ctx.is_null() {
            let mut bytes = vec![0u8; w * h * 4];
            let space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
            let ctx = CGBitmapContextCreate(bytes.as_mut_ptr(), w, h, 8, w * 4, space, 1);
            CFRelease(space);
            if ctx.is_null() {
                CFRelease(image);
                return Err(format!("the system cannot draw {path}"));
            }
            CGContextDrawImage(ctx, CGRect { x: 0.0, y: 0.0, w: w as f64, h: h as f64 }, image);
            CGContextRelease(ctx);
            for (p, b) in px.iter_mut().zip(bytes.as_chunks::<4>().0) {
                *p = b.map(|v| v as f32 / 255.0);
            }
        } else {
            CGContextDrawImage(ctx, CGRect { x: 0.0, y: 0.0, w: w as f64, h: h as f64 }, image);
            CGContextRelease(ctx);
        }
        CFRelease(image);
        Ok(crate::turned(Picture { width: w, height: h, px, alpha }, orientation))
    }
}

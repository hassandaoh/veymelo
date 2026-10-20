//! Zip archives (PKWARE's APPNOTE): files kept one after another, each stored as it is or
//! deflated, with a directory at the end saying where each one is.
//!
//! Written: each file deflated when that makes it smaller, names in UTF-8. Read: stored and
//! deflated files, their check sums checked; archives over 4 GB (zip64) and encrypted files
//! are not read.

use crate::{crc32, deflate, inflate_up_to};

const LOCAL: u32 = 0x0403_4B50;
const CENTRAL: u32 = 0x0201_4B50;
const END: u32 = 0x0605_4B50;
/// The names are UTF-8.
const UTF8: u16 = 0x0800;
/// 1 January 1980, the first day a zip date can say.
const DATE: u16 = 1 << 5 | 1;

/// The most a read archive may hold, inflated, in all.
const MOST: usize = 1 << 30;

fn u16le(out: &mut Vec<u8>, v: u16) {
    out.extend(v.to_le_bytes());
}

fn u32le(out: &mut Vec<u8>, v: u32) {
    out.extend(v.to_le_bytes());
}

/// An archive of these files (name, contents), in order.
pub fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut dir = Vec::new();
    for (name, data) in files {
        let packed = deflate(data);
        let (method, body): (u16, &[u8]) = if packed.len() < data.len() { (8, &packed) } else { (0, data) };
        let (crc, at) = (crc32(data), out.len() as u32);
        let head = |out: &mut Vec<u8>, sig: u32| {
            u32le(out, sig);
            if sig == CENTRAL {
                u16le(out, 20); // made by: version 2.0
            }
            u16le(out, 20); // needed: 2.0, for deflate
            u16le(out, UTF8);
            u16le(out, method);
            u16le(out, 0); // time
            u16le(out, DATE);
            u32le(out, crc);
            u32le(out, body.len() as u32);
            u32le(out, data.len() as u32);
            u16le(out, name.len() as u16);
            u16le(out, 0); // extra
        };
        head(&mut out, LOCAL);
        out.extend(name.as_bytes());
        out.extend(body);
        head(&mut dir, CENTRAL);
        u16le(&mut dir, 0); // comment
        u16le(&mut dir, 0); // disk
        u16le(&mut dir, 0); // internal attributes
        u32le(&mut dir, 0); // external attributes
        u32le(&mut dir, at);
        dir.extend(name.as_bytes());
    }
    let (dir_at, dir_len) = (out.len() as u32, dir.len() as u32);
    out.extend(dir);
    u32le(&mut out, END);
    u16le(&mut out, 0);
    u16le(&mut out, 0);
    u16le(&mut out, files.len() as u16);
    u16le(&mut out, files.len() as u16);
    u32le(&mut out, dir_len);
    u32le(&mut out, dir_at);
    u16le(&mut out, 0);
    out
}

fn get16(d: &[u8], at: usize) -> Result<u16, String> {
    d.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or_else(|| "the archive is cut off".to_string())
}

fn get32(d: &[u8], at: usize) -> Result<u32, String> {
    d.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or_else(|| "the archive is cut off".to_string())
}

/// The files in an archive (name, contents), in the order its directory lists them; folders
/// are left out.
pub fn unzip(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    // the end record is in the last 64 kB and 22 bytes (it may carry a comment)
    let from = data.len().saturating_sub(22 + 65_535);
    let end = (from..data.len().saturating_sub(21)).rev().find(|&i| get32(data, i) == Ok(END)).ok_or("not a zip archive")?;
    let count = get16(data, end + 10)? as usize;
    let mut at = get32(data, end + 16)? as usize;
    if at == 0xFFFF_FFFF || count == 0xFFFF {
        return Err("a zip64 archive (over 4 GB), which is not read".into());
    }
    let mut files = Vec::with_capacity(count.min(4096));
    let mut total = 0usize;
    for _ in 0..count {
        if get32(data, at)? != CENTRAL {
            return Err("the archive's directory is broken".into());
        }
        let (flags, method, crc) = (get16(data, at + 8)?, get16(data, at + 10)?, get32(data, at + 16)?);
        let (packed, size) = (get32(data, at + 20)? as usize, get32(data, at + 24)? as usize);
        let (name_len, extra_len, note_len) = (get16(data, at + 28)? as usize, get16(data, at + 30)? as usize, get16(data, at + 32)? as usize);
        let local = get32(data, at + 42)? as usize;
        let name = data.get(at + 46..at + 46 + name_len).ok_or("the archive is cut off")?;
        let name = String::from_utf8_lossy(name).into_owned();
        at += 46 + name_len + extra_len + note_len;
        if name.ends_with('/') {
            continue;
        }
        if flags & 1 != 0 {
            return Err(format!("{name} is encrypted"));
        }
        if get32(data, local)? != LOCAL {
            return Err(format!("{name}: its header is not where the directory says"));
        }
        let start = local + 30 + get16(data, local + 26)? as usize + get16(data, local + 28)? as usize;
        let body = data.get(start..start + packed).ok_or_else(|| format!("{name} is cut off"))?;
        let contents = match method {
            0 => body.to_vec(),
            8 => inflate_up_to(body, MOST - total).map_err(|e| format!("{name}: {e}"))?,
            m => return Err(format!("{name} is packed a way that is not read (method {m})")),
        };
        if contents.len() != size || crc32(&contents) != crc {
            return Err(format!("{name} does not match its check sum"));
        }
        total += contents.len();
        files.push((name, contents));
    }
    Ok(files)
}

//! A stream's bytes undone through its filters: Flate (zlib; what it holds up to where it goes
//! wrong, when it does), LZW, ASCII hex and base-85, run lengths, and the PNG and TIFF predictors.
//! Picture codecs the system reads (JPEG, JPEG 2000) are left for it, as are those we do not read
//! (CCITT fax, JBIG2), named.

use crate::object::{Dict, Obj, hexval, white};

/// What a stream's filters leave: its bytes, and the picture codec they are still in (`DCTDecode`,
/// `JPXDecode`, `CCITTFaxDecode`, `JBIG2Decode`) when the last filter is one.
pub struct Decoded {
    pub data: Vec<u8>,
    pub codec: Option<String>,
    pub params: Option<Dict>,
}

/// `data` through filter `name` (its abbreviation in an inline picture too), with `params`.
fn one(name: &str, data: &[u8], params: Option<&Dict>) -> Result<Vec<u8>, String> {
    let int = |k: &str, d: i64| params.and_then(|p| p.get(k)).and_then(Obj::as_i64).unwrap_or(d);
    let out = match name {
        "FlateDecode" | "Fl" => {
            // a zlib stream's two bytes of header (when it has them), then deflate
            let body = if data.len() >= 2 && data[0] & 0x0F == 8 && (u16::from(data[0]) << 8 | u16::from(data[1])) % 31 == 0 { &data[2..] } else { data };
            let (out, err) = veymelo_zip::inflate_partial(body);
            if out.is_empty()
                && let Some(e) = err
            {
                return Err(format!("its Flate data cannot be read ({e})"));
            }
            out
        }
        "LZWDecode" | "LZW" => lzw(data, int("EarlyChange", 1) != 0),
        "ASCIIHexDecode" | "AHx" => {
            let digits: Vec<u8> = data.iter().copied().take_while(|&b| b != b'>').filter(u8::is_ascii_hexdigit).collect();
            digits.chunks(2).map(|c| (hexval(c[0]) << 4) | c.get(1).map_or(0, |&b| hexval(b))).collect()
        }
        "ASCII85Decode" | "A85" => ascii85(data),
        "RunLengthDecode" | "RL" => run_length(data),
        "Crypt" => data.to_vec(),
        other => return Err(format!("the filter {other} is not read")),
    };
    if matches!(name, "FlateDecode" | "Fl" | "LZWDecode" | "LZW") {
        return Ok(predict(out, int("Predictor", 1), int("Colors", 1).max(1) as usize, int("BitsPerComponent", 8).max(1) as usize, int("Columns", 1).max(1) as usize));
    }
    Ok(out)
}

/// The names and parameters of a stream's filters, in order.
pub fn filters_of(dict: &Dict, inline: bool) -> Vec<(String, Option<Dict>)> {
    let (fk, pk) = if inline { ("F", "DP") } else { ("Filter", "DecodeParms") };
    let get = |k: &str, long: &str| dict.get(k).or_else(|| if inline { dict.get(long) } else { None });
    let names: Vec<String> = match get(fk, "Filter") {
        Some(Obj::Name(n)) => vec![n.clone()],
        Some(Obj::Array(a)) => a.iter().filter_map(|o| o.as_name().map(str::to_string)).collect(),
        _ => Vec::new(),
    };
    let params: Vec<Option<Dict>> = match get(pk, "DecodeParms") {
        Some(Obj::Dict(d)) => vec![Some(d.clone())],
        Some(Obj::Array(a)) => a.iter().map(|o| o.as_dict().cloned()).collect(),
        _ => Vec::new(),
    };
    names.into_iter().enumerate().map(|(i, n)| (n, params.get(i).cloned().flatten())).collect()
}

/// `data` through `filters` (parameters already looked up).
pub fn decode(data: &[u8], filters: &[(String, Option<Dict>)]) -> Result<Decoded, String> {
    let mut out = data.to_vec();
    for (i, (name, params)) in filters.iter().enumerate() {
        let codec = match name.as_str() {
            "DCTDecode" | "DCT" => Some("DCTDecode"),
            "JPXDecode" => Some("JPXDecode"),
            "CCITTFaxDecode" | "CCF" => Some("CCITTFaxDecode"),
            "JBIG2Decode" => Some("JBIG2Decode"),
            _ => None,
        };
        if let Some(c) = codec {
            // a picture codec: the rest is the system's (it should be the last)
            let _ = i;
            return Ok(Decoded { data: out, codec: Some(c.to_string()), params: params.clone() });
        }
        out = one(name, &out, params.as_ref())?;
    }
    Ok(Decoded { data: out, codec: None, params: None })
}

fn lzw(data: &[u8], early: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut table: Vec<Vec<u8>> = Vec::new();
    let reset = |t: &mut Vec<Vec<u8>>| {
        t.clear();
        t.extend((0..256).map(|b| vec![b as u8]));
        t.push(Vec::new());
        t.push(Vec::new());
    };
    reset(&mut table);
    let (mut bits, mut have, mut acc) = (9u32, 0u32, 0u32);
    let mut prev: Option<Vec<u8>> = None;
    for &b in data {
        acc = (acc << 8) | b as u32;
        have += 8;
        while have >= bits {
            let code = ((acc >> (have - bits)) & ((1 << bits) - 1)) as usize;
            have -= bits;
            if code == 256 {
                reset(&mut table);
                bits = 9;
                prev = None;
                continue;
            }
            if code == 257 {
                return out;
            }
            let entry = if code < table.len() {
                table[code].clone()
            } else if let Some(p) = &prev {
                let mut e = p.clone();
                e.push(p[0]);
                e
            } else {
                return out;
            };
            out.extend(&entry);
            if let Some(p) = prev {
                let mut e = p;
                e.push(entry[0]);
                table.push(e);
            }
            prev = Some(entry);
            let next = table.len() + usize::from(early);
            bits = if next >= 2048 { 12 } else if next >= 1024 { 11 } else if next >= 512 { 10 } else { 9 };
        }
    }
    out
}

fn ascii85(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut group = [0u32; 5];
    let mut n = 0;
    let mut p = 0;
    // a leading <~ is passed over
    if data.starts_with(b"<~") {
        p = 2;
    }
    while p < data.len() {
        let b = data[p];
        p += 1;
        if white(b) {
            continue;
        }
        if b == b'~' {
            break;
        }
        if b == b'z' && n == 0 {
            out.extend([0, 0, 0, 0]);
            continue;
        }
        if !(b'!'..=b'u').contains(&b) {
            continue;
        }
        group[n] = (b - b'!') as u32;
        n += 1;
        if n == 5 {
            let v = group.iter().fold(0u32, |v, &d| v.wrapping_mul(85).wrapping_add(d));
            out.extend(v.to_be_bytes());
            n = 0;
        }
    }
    if n > 1 {
        for slot in group.iter_mut().skip(n) {
            *slot = 84;
        }
        let v = group.iter().fold(0u32, |v, &d| v.wrapping_mul(85).wrapping_add(d));
        out.extend(&v.to_be_bytes()[..n - 1]);
    }
    out
}

fn run_length(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut p = 0;
    while p < data.len() {
        let len = data[p] as usize;
        p += 1;
        match len {
            128 => break,
            0..=127 => {
                out.extend(data.get(p..p + len + 1).unwrap_or(&data[p.min(data.len())..]));
                p += len + 1;
            }
            _ => {
                if let Some(&b) = data.get(p) {
                    out.extend(std::iter::repeat_n(b, 257 - len));
                }
                p += 1;
            }
        }
    }
    out
}

/// Rows undone from a predictor: TIFF's (2) or PNG's (10 and on, each row its own).
fn predict(data: Vec<u8>, predictor: i64, colors: usize, bpc: usize, columns: usize) -> Vec<u8> {
    if predictor < 2 {
        return data;
    }
    let bpp = (colors * bpc).div_ceil(8).max(1);
    let row = (colors * bpc * columns).div_ceil(8);
    if predictor == 2 {
        // TIFF: each sample less the one before it (8 bits here; others left)
        let mut out = data;
        if bpc == 8 {
            for r in out.chunks_mut(row) {
                for i in colors..r.len() {
                    r[i] = r[i].wrapping_add(r[i - colors]);
                }
            }
        }
        return out;
    }
    let mut out = Vec::with_capacity(data.len());
    let mut prior = vec![0u8; row];
    for chunk in data.chunks(row + 1) {
        let (kind, line) = (chunk[0], &chunk[1..]);
        let mut cur = line.to_vec();
        cur.resize(row, 0);
        for i in 0..row {
            let a = if i >= bpp { cur[i - bpp] } else { 0 };
            let b = prior[i];
            let c = if i >= bpp { prior[i - bpp] } else { 0 };
            cur[i] = match kind {
                1 => cur[i].wrapping_add(a),
                2 => cur[i].wrapping_add(b),
                3 => cur[i].wrapping_add(((a as u16 + b as u16) / 2) as u8),
                4 => {
                    let p = a as i16 + b as i16 - c as i16;
                    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
                    cur[i].wrapping_add(if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c })
                }
                _ => cur[i],
            };
        }
        out.extend(&cur[..line.len().min(row)]);
        prior = cur;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_undo_what_writers_do() {
        assert_eq!(one("ASCIIHexDecode", b"48 65 6c6c 6F7>", None).unwrap(), b"Hello\x70");
        assert_eq!(one("ASCII85Decode", b"<~87cURD]i,\"Ebo80~>", None).unwrap(), b"Hello World!");
        assert_eq!(one("RunLengthDecode", &[2, b'a', b'b', b'c', 254, b'x', 128], None).unwrap(), b"abcxxx");
        let z = veymelo_zip::zlib(b"some text some text some text");
        assert_eq!(one("FlateDecode", &z, None).unwrap(), b"some text some text some text");
        // cut off: what came before it
        let long: Vec<u8> = (0..5000u32).map(|i| (i * 7 % 251) as u8).collect();
        let z = veymelo_zip::zlib(&long);
        let part = one("FlateDecode", &z[..z.len() / 2], None).unwrap();
        assert!(!part.is_empty() && long.starts_with(&part));
        // LZW: the PDF reference's example
        let lzw_data = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        assert_eq!(one("LZWDecode", &lzw_data, None).unwrap(), [45, 45, 45, 45, 45, 65, 45, 45, 45, 66]);
        // PNG "up" rows
        let rows = [2, 1, 2, 3, 2, 1, 1, 1];
        assert_eq!(predict(rows.to_vec(), 12, 1, 8, 3), [1, 2, 3, 2, 3, 4]);
    }
}

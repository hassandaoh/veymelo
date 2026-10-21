//! A content stream read into its operators, each with its operands; a picture written into the
//! content itself (`BI … ID … EI`) read with its bytes.

use crate::object::{Dict, Lexer, Obj, white};

#[derive(Clone, Debug)]
pub struct Op {
    pub name: String,
    pub args: Vec<Obj>,
    /// An inline picture's dictionary and bytes (`BI`).
    pub inline: Option<(Dict, Vec<u8>)>,
}

/// How many bytes an inline picture with no filters holds.
fn plain_length(d: &Dict) -> Option<usize> {
    if d.get("F").or_else(|| d.get("Filter")).is_some() {
        return None;
    }
    let int = |a: &str, b: &str| d.get(a).or_else(|| d.get(b)).and_then(Obj::as_i64);
    let (w, h) = (int("W", "Width")? as usize, int("H", "Height")? as usize);
    let mask = d.get("IM").or_else(|| d.get("ImageMask")).and_then(Obj::as_bool) == Some(true);
    let bpc = if mask { 1 } else { int("BPC", "BitsPerComponent").unwrap_or(8) as usize };
    let comps = if mask {
        1
    } else {
        match d.get("CS").or_else(|| d.get("ColorSpace")) {
            Some(Obj::Name(n)) => match n.as_str() {
                "RGB" | "DeviceRGB" => 3,
                "CMYK" | "DeviceCMYK" => 4,
                "G" | "DeviceGray" | "I" | "Indexed" => 1,
                _ => return None,
            },
            Some(Obj::Array(a)) if a.first().and_then(Obj::as_name).is_some_and(|n| n == "I" || n == "Indexed") => 1,
            None => 1,
            _ => return None,
        }
    };
    Some((w * comps * bpc).div_ceil(8) * h)
}

/// The operators of a content stream.
pub fn ops(data: &[u8]) -> Vec<Op> {
    let mut l = Lexer::new(data, 0);
    let mut out = Vec::new();
    let mut args = Vec::new();
    while let Some(o) = l.object() {
        match o {
            Obj::Word(w) if w == "BI" => {
                // key value pairs, then ID, one white byte, the bytes, EI
                let mut dict = Dict::default();
                loop {
                    let save = l.p;
                    match l.object() {
                        Some(Obj::Word(w)) if w == "ID" => break,
                        Some(Obj::Name(k)) => {
                            let v = l.object().unwrap_or(Obj::Null);
                            dict.0.push((k, v));
                        }
                        Some(_) => {}
                        None => {
                            l.p = save;
                            break;
                        }
                    }
                }
                let start = (l.p + 1).min(data.len());
                let end = match plain_length(&dict) {
                    Some(n) if start + n <= data.len() => start + n,
                    _ => {
                        // "EI" with white round it, where what follows reads as content
                        let mut p = start;
                        loop {
                            let Some(at) = data.get(p..).and_then(|d| d.windows(2).position(|w| w == b"EI")).map(|q| q + p) else { break data.len() };
                            let before = at == 0 || white(data[at - 1]);
                            let after = data.get(at + 2).is_none_or(|&b| white(b));
                            if before && after {
                                break if at > start && white(data[at - 1]) { at - 1 } else { at };
                            }
                            p = at + 2;
                        }
                    }
                };
                let bytes = data[start..end.max(start)].to_vec();
                // past EI
                l.p = end;
                if let Some(at) = data.get(end..).and_then(|d| d.windows(2).position(|w| w == b"EI")) {
                    l.p = end + at + 2;
                }
                out.push(Op { name: "BI".into(), args: std::mem::take(&mut args), inline: Some((dict, bytes)) });
            }
            Obj::Word(w) => out.push(Op { name: w, args: std::mem::take(&mut args), inline: None }),
            o => {
                if args.len() < 1 << 16 {
                    args.push(o);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operators_and_an_inline_picture_are_read() {
        let src = b"q 1 0 0 1 0 0 cm BI /W 2 /H 2 /BPC 8 /CS /G ID \x00EI\xff\x10 EI Q (x) Tj";
        let o = ops(src);
        let names: Vec<&str> = o.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["q", "cm", "BI", "Q", "Tj"]);
        assert_eq!(o[2].inline.as_ref().unwrap().1, b"\x00EI\xff");
        assert_eq!(o[4].args, vec![Obj::Str(b"x".to_vec())]);
    }
}

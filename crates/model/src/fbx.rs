//! FBX, Autodesk's interchange format, version 7 (2011 on), binary and ASCII, read from how the
//! files lay out their node records: a binary file's records (their ends, properties and
//! nested records; 64-bit from version 7.5) and their properties (numbers, strings, raw bytes,
//! and arrays kept as they are or zlib-compressed), or an ASCII file's `Name: values {…}` lines,
//! both read into the same tree. From it: the objects and the connections between them; the
//! file's axes and unit turned to the model's (+Y up, +Z toward the viewer, metres).
//!
//! Meshes are polygons (made triangles), with normals, texture points and colours by control
//! point, by polygon vertex or by polygon, direct or by index, and a material per polygon.
//! Models are placed as FBX places them: translation, rotation (in its Euler order), scaling,
//! pre- and post-rotation, pivots and offsets; their geometric transform moves their mesh
//! alone. Materials take their diffuse colour and factor, emission, opacity and shininess and
//! their diffuse picture (a file beside the model, or the picture inside it); animations move
//! models' translation, rotation and scaling (each moment a key is at, worked out whole; at
//! the times the file gives).
//!
//! Not read, said in the model's notes: skins and blend shapes (meshes stay as they rest),
//! cameras and lights.

use std::collections::HashMap;
use std::path::Path;

use crate::{AlphaMode, Animation, Channel, Interp, Map, Mat4, Material, Mesh, Model, Moves, Node, Primitive, Quat, Texture, Wrap};

/// A property of a node record.
#[derive(Clone, Debug, PartialEq)]
enum Prop {
    Int(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    Ints(Vec<i64>),
    Floats(Vec<f64>),
}

impl Prop {
    fn int(&self) -> Option<i64> {
        match self {
            Prop::Int(v) => Some(*v),
            Prop::Float(v) => Some(*v as i64),
            _ => None,
        }
    }

    fn num(&self) -> Option<f64> {
        match self {
            Prop::Int(v) => Some(*v as f64),
            Prop::Float(v) => Some(*v),
            _ => None,
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Prop::Text(s) => Some(s),
            _ => None,
        }
    }

    fn floats(&self) -> Option<Vec<f64>> {
        match self {
            Prop::Floats(v) => Some(v.clone()),
            Prop::Ints(v) => Some(v.iter().map(|&x| x as f64).collect()),
            _ => None,
        }
    }

    fn ints(&self) -> Option<Vec<i64>> {
        match self {
            Prop::Ints(v) => Some(v.clone()),
            Prop::Floats(v) => Some(v.iter().map(|&x| x as i64).collect()),
            _ => None,
        }
    }
}

/// A node record: its name, properties and the records inside it.
#[derive(Clone, Debug, Default, PartialEq)]
struct Rec {
    name: String,
    props: Vec<Prop>,
    kids: Vec<Rec>,
}

impl Rec {
    fn kid(&self, name: &str) -> Option<&Rec> {
        self.kids.iter().find(|k| k.name == name)
    }

    fn kids_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Rec> + 'a {
        self.kids.iter().filter(move |k| k.name == name)
    }

    /// The first property of the record `name` inside it.
    fn value(&self, name: &str) -> Option<&Prop> {
        self.kid(name).and_then(|k| k.props.first())
    }

    /// Its `Properties70` (or older `Properties60`): each one's values by its name.
    fn settings(&self) -> HashMap<&str, &[Prop]> {
        let mut out = HashMap::new();
        for group in ["Properties70", "Properties60"] {
            if let Some(g) = self.kid(group) {
                for p in g.kids.iter().filter(|p| p.name == "P" || p.name == "Property") {
                    // name, type, label, flags, then its values (Properties60: name, type, flags)
                    let skip = if p.name == "P" { 4 } else { 3 };
                    if let Some(name) = p.props.first().and_then(Prop::text) {
                        out.insert(name, p.props.get(skip..).unwrap_or(&[]));
                    }
                }
            }
        }
        out
    }
}

/// A setting's numbers (as many as `d` has, each `d`'s where there is none).
fn setting<const N: usize>(s: &HashMap<&str, &[Prop]>, name: &str, d: [f64; N]) -> [f64; N] {
    match s.get(name) {
        Some(v) => std::array::from_fn(|i| v.get(i).and_then(Prop::num).unwrap_or(d[i])),
        None => d,
    }
}

/// The most items an array is read with.
const MOST: usize = 1 << 28;

/// A binary file's records.
fn binary(b: &[u8]) -> Result<(u32, Vec<Rec>), String> {
    let version = u32::from_le_bytes(b.get(23..27).ok_or("the FBX file is cut short")?.try_into().expect("four bytes"));
    let wide = version >= 7500;
    let mut at = 27;
    let mut out = Vec::new();
    while at < b.len() {
        match record(b, &mut at, wide, 0)? {
            Some(r) => out.push(r),
            None => break,
        }
    }
    Ok((version, out))
}

/// The record at `at` (none: the empty record that ends a list), `at` then past it.
fn record(b: &[u8], at: &mut usize, wide: bool, depth: usize) -> Result<Option<Rec>, String> {
    if depth > 64 {
        return Err("the FBX file's records nest too deep".into());
    }
    let short = || "the FBX file is cut short".to_string();
    let uint = |at: &mut usize| -> Result<u64, String> {
        if wide {
            let v = u64::from_le_bytes(b.get(*at..*at + 8).ok_or_else(short)?.try_into().expect("eight bytes"));
            *at += 8;
            Ok(v)
        } else {
            let v = u32::from_le_bytes(b.get(*at..*at + 4).ok_or_else(short)?.try_into().expect("four bytes"));
            *at += 4;
            Ok(v as u64)
        }
    };
    let start = *at;
    let end = uint(at)? as usize;
    let count = uint(at)?;
    let _list = uint(at)?;
    let name_len = *b.get(*at).ok_or_else(short)? as usize;
    *at += 1;
    if end == 0 {
        return Ok(None);
    }
    if end <= start || end > b.len() {
        return Err(format!("an FBX record at byte {start} ends outside the file"));
    }
    let name = String::from_utf8_lossy(b.get(*at..*at + name_len).ok_or_else(short)?).into_owned();
    *at += name_len;
    let mut props = Vec::new();
    for _ in 0..count {
        props.push(property(b, at, end)?);
    }
    let mut kids = Vec::new();
    while *at < end {
        match record(b, at, wide, depth + 1)? {
            Some(r) => kids.push(r),
            None => break,
        }
    }
    *at = end;
    Ok(Some(Rec { name, props, kids }))
}

/// The property at `at` (inside a record ending at `end`).
fn property(b: &[u8], at: &mut usize, end: usize) -> Result<Prop, String> {
    let short = || "an FBX property runs past its record".to_string();
    let take = |at: &mut usize, n: usize| -> Result<&[u8], String> {
        let s = b.get(*at..*at + n).filter(|_| *at + n <= end).ok_or_else(short)?;
        *at += n;
        Ok(s)
    };
    let kind = take(at, 1)?[0];
    let le4 = |s: &[u8]| u32::from_le_bytes(s.try_into().expect("four bytes"));
    Ok(match kind {
        b'Y' => Prop::Int(i16::from_le_bytes(take(at, 2)?.try_into().expect("two bytes")) as i64),
        b'C' => Prop::Int(take(at, 1)?[0] as i64),
        b'I' => Prop::Int(i32::from_le_bytes(take(at, 4)?.try_into().expect("four bytes")) as i64),
        b'F' => Prop::Float(f32::from_le_bytes(take(at, 4)?.try_into().expect("four bytes")) as f64),
        b'D' => Prop::Float(f64::from_le_bytes(take(at, 8)?.try_into().expect("eight bytes"))),
        b'L' => Prop::Int(i64::from_le_bytes(take(at, 8)?.try_into().expect("eight bytes"))),
        b'S' | b'R' => {
            let n = le4(take(at, 4)?) as usize;
            let s = take(at, n)?;
            if kind == b'S' { Prop::Text(String::from_utf8_lossy(s).into_owned()) } else { Prop::Bytes(s.to_vec()) }
        }
        b'f' | b'd' | b'l' | b'i' | b'b' => {
            let (n, encoding, size) = (le4(take(at, 4)?) as usize, le4(take(at, 4)?), le4(take(at, 4)?) as usize);
            let raw = take(at, size)?;
            let width = match kind {
                b'd' | b'l' => 8,
                b'f' | b'i' => 4,
                _ => 1,
            };
            if n > MOST {
                return Err(format!("an FBX array of {n} items is more than this reads"));
            }
            let data = match encoding {
                0 => raw.to_vec(),
                1 => veymelo_zip::unzlib(raw).map_err(|e| format!("an FBX array cannot be uncompressed: {e}"))?,
                e => return Err(format!("an FBX array is kept a way this does not read ({e})")),
            };
            if data.len() < n * width {
                return Err("an FBX array has fewer items than it says".into());
            }
            let items = data[..n * width].chunks_exact(width);
            match kind {
                b'f' => Prop::Floats(items.map(|c| f32::from_le_bytes(c.try_into().expect("four bytes")) as f64).collect()),
                b'd' => Prop::Floats(items.map(|c| f64::from_le_bytes(c.try_into().expect("eight bytes"))).collect()),
                b'l' => Prop::Ints(items.map(|c| i64::from_le_bytes(c.try_into().expect("eight bytes"))).collect()),
                b'i' => Prop::Ints(items.map(|c| i32::from_le_bytes(c.try_into().expect("four bytes")) as i64).collect()),
                _ => Prop::Ints(items.map(|c| c[0] as i64).collect()),
            }
        }
        k => return Err(format!("an FBX property of a kind this does not know ({:?})", k as char)),
    })
}

/// A piece of an ASCII file.
#[derive(Clone, Debug, PartialEq)]
enum Token {
    Name(String),
    Text(String),
    Word(String),
    Number(String),
    Comma,
    Open,
    Close,
    Star,
}

fn tokens(text: &str) -> Vec<Token> {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            ';' => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
            }
            ch if ch.is_whitespace() => i += 1,
            ',' => {
                out.push(Token::Comma);
                i += 1;
            }
            '{' => {
                out.push(Token::Open);
                i += 1;
            }
            '}' => {
                out.push(Token::Close);
                i += 1;
            }
            '*' => {
                out.push(Token::Star);
                i += 1;
            }
            '"' => {
                let mut s = String::new();
                i += 1;
                while i < c.len() && c[i] != '"' {
                    s.push(c[i]);
                    i += 1;
                }
                i += 1;
                out.push(Token::Text(s.replace("&quot;", "\"")));
            }
            ch if ch.is_ascii_digit() || ch == '-' || ch == '+' || ch == '.' => {
                let mut s = String::new();
                while i < c.len() && (c[i].is_ascii_alphanumeric() || matches!(c[i], '-' | '+' | '.')) {
                    s.push(c[i]);
                    i += 1;
                }
                out.push(Token::Number(s));
            }
            _ => {
                let mut s = String::new();
                while i < c.len() && (c[i].is_alphanumeric() || matches!(c[i], '_' | '|' | '.' | '-')) {
                    s.push(c[i]);
                    i += 1;
                }
                if s.is_empty() {
                    i += 1;
                    continue;
                }
                if c.get(i) == Some(&':') {
                    i += 1;
                    out.push(Token::Name(s));
                } else {
                    out.push(Token::Word(s));
                }
            }
        }
    }
    out
}

/// An ASCII file's records.
fn ascii(text: &str) -> Result<(u32, Vec<Rec>), String> {
    let t = tokens(text);
    let mut at = 0;
    let recs = ascii_list(&t, &mut at, 0)?;
    let version = recs.iter().find(|r| r.name == "FBXHeaderExtension").and_then(|h| h.value("FBXVersion")).and_then(Prop::int).unwrap_or(0) as u32;
    Ok((version, recs))
}

fn ascii_list(t: &[Token], at: &mut usize, depth: usize) -> Result<Vec<Rec>, String> {
    if depth > 64 {
        return Err("the FBX file's records nest too deep".into());
    }
    let mut out = Vec::new();
    while *at < t.len() {
        match &t[*at] {
            Token::Close => {
                *at += 1;
                return Ok(out);
            }
            Token::Name(n) => {
                *at += 1;
                let mut rec = Rec { name: n.clone(), ..Rec::default() };
                // its values, comma after comma (an array's: `*count { a: … }`)
                loop {
                    match t.get(*at) {
                        Some(Token::Text(s)) => rec.props.push(Prop::Text(s.clone())),
                        Some(Token::Word(w)) => rec.props.push(Prop::Text(w.clone())),
                        Some(Token::Number(s)) => rec.props.push(number(s)),
                        Some(Token::Comma) => {}
                        Some(Token::Star) => {
                            if let Some(Token::Number(_)) = t.get(*at + 1) {
                                *at += 1;
                            }
                        }
                        _ => break,
                    }
                    *at += 1;
                }
                if let Some(Token::Open) = t.get(*at) {
                    *at += 1;
                    let kids = ascii_list(t, at, depth + 1)?;
                    // an array's `a:` becomes the record's values
                    if rec.props.is_empty() && kids.len() == 1 && kids[0].name == "a" {
                        let vals = &kids[0].props;
                        let whole = vals.iter().all(|p| matches!(p, Prop::Int(_)));
                        rec.props.push(if whole { Prop::Ints(vals.iter().filter_map(Prop::int).collect()) } else { Prop::Floats(vals.iter().filter_map(Prop::num).collect()) });
                    } else {
                        rec.kids = kids;
                    }
                }
                out.push(rec);
            }
            _ => *at += 1,
        }
    }
    Ok(out)
}

fn number(s: &str) -> Prop {
    match s.parse::<i64>() {
        Ok(v) => Prop::Int(v),
        Err(_) => Prop::Float(s.parse().unwrap_or(0.0)),
    }
}

/// A part of a model an animation moves: its curve node, and its curves along x, y and z.
type Moved<'r> = (&'r Rec, Vec<Option<&'r Rec>>);

/// A curve: its keys' times (seconds) and values, and whether it holds each key's value.
type Curve = (Vec<f64>, Vec<f64>, bool);

/// The FBX time unit: this many to a second.
const TICKS: f64 = 46_186_158_000.0;

/// What an object is called (without its class).
fn object_name(p: Option<&Prop>) -> String {
    let s = p.and_then(Prop::text).unwrap_or("");
    if let Some((name, _)) = s.split_once("\u{0}\u{1}") {
        return name.to_string();
    }
    s.split_once("::").map_or(s, |(_, n)| n).to_string()
}

/// Turns `deg` (x, y, z) in Euler order `order` (FBX's: 0 XYZ, 1 XZY, 2 YZX, 3 YXZ, 4 ZXY, 5
/// ZYX; the first named turns first).
fn euler(deg: [f64; 3], order: i64) -> [[f64; 3]; 3] {
    let r = |axis: usize| {
        let (s, c) = deg[axis].to_radians().sin_cos();
        match axis {
            0 => [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]],
            1 => [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]],
            _ => [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]],
        }
    };
    let seq = match order {
        1 => [0, 2, 1],
        2 => [1, 2, 0],
        3 => [1, 0, 2],
        4 => [2, 0, 1],
        5 => [2, 1, 0],
        _ => [0, 1, 2],
    };
    mul3(r(seq[2]), mul3(r(seq[1]), r(seq[0])))
}

fn mul3(a: [[f64; 3]; 3], b: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

fn mat(r: [[f64; 3]; 3], t: [f64; 3]) -> Mat4 {
    Mat4([[r[0][0], r[0][1], r[0][2], t[0]], [r[1][0], r[1][1], r[1][2], t[1]], [r[2][0], r[2][1], r[2][2], t[2]], [0.0, 0.0, 0.0, 1.0]])
}

const SAME: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn moved(t: [f64; 3]) -> Mat4 {
    mat(SAME, t)
}

fn scaled(s: [f64; 3]) -> Mat4 {
    mat([[s[0], 0.0, 0.0], [0.0, s[1], 0.0], [0.0, 0.0, s[2]]], [0.0; 3])
}

/// A model's place from its parent, as FBX works it out: translation · rotation offset ·
/// rotation pivot · pre-rotation · rotation · post-rotation⁻¹ · pivot⁻¹ · scaling offset ·
/// scaling pivot · scaling · scaling pivot⁻¹.
fn place(s: &HashMap<&str, &[Prop]>, t: [f64; 3], r: [f64; 3], sc: [f64; 3]) -> Mat4 {
    let order = s.get("RotationOrder").and_then(|v| v.first()).and_then(Prop::int).unwrap_or(0);
    let pre = euler(setting(s, "PreRotation", [0.0; 3]), 0);
    let post = euler(setting(s, "PostRotation", [0.0; 3]), 0);
    let post_back = [[post[0][0], post[1][0], post[2][0]], [post[0][1], post[1][1], post[2][1]], [post[0][2], post[1][2], post[2][2]]];
    let (rp, sp) = (setting(s, "RotationPivot", [0.0; 3]), setting(s, "ScalingPivot", [0.0; 3]));
    let neg = |v: [f64; 3]| v.map(|x| -x);
    moved(t)
        * moved(setting(s, "RotationOffset", [0.0; 3]))
        * moved(rp)
        * mat(mul3(pre, mul3(euler(r, order), post_back)), [0.0; 3])
        * moved(neg(rp))
        * moved(setting(s, "ScalingOffset", [0.0; 3]))
        * moved(sp)
        * scaled(sc)
        * moved(neg(sp))
}

/// A matrix (no shear) as a move, a turn and a scale.
fn parts(m: &Mat4) -> ([f64; 3], Quat, [f64; 3]) {
    let a = &m.0;
    let col = |j: usize| [a[0][j], a[1][j], a[2][j]];
    let len = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let mut s = [len(col(0)), len(col(1)), len(col(2))];
    let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0]) + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
    if det < 0.0 {
        s[0] = -s[0];
    }
    let r: [[f64; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| if s[j].abs() > 1e-300 { a[i][j] / s[j] } else { 0.0 }));
    ([a[0][3], a[1][3], a[2][3]], quat(r), s)
}

/// A turn as a quaternion.
fn quat(r: [[f64; 3]; 3]) -> Quat {
    let tr = r[0][0] + r[1][1] + r[2][2];
    let q = if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
        Quat { w: 0.25 * s, x: (r[2][1] - r[1][2]) / s, y: (r[0][2] - r[2][0]) / s, z: (r[1][0] - r[0][1]) / s }
    } else if r[0][0] > r[1][1] && r[0][0] > r[2][2] {
        let s = (1.0 + r[0][0] - r[1][1] - r[2][2]).sqrt() * 2.0;
        Quat { w: (r[2][1] - r[1][2]) / s, x: 0.25 * s, y: (r[0][1] + r[1][0]) / s, z: (r[0][2] + r[2][0]) / s }
    } else if r[1][1] > r[2][2] {
        let s = (1.0 + r[1][1] - r[0][0] - r[2][2]).sqrt() * 2.0;
        Quat { w: (r[0][2] - r[2][0]) / s, x: (r[0][1] + r[1][0]) / s, y: 0.25 * s, z: (r[1][2] + r[2][1]) / s }
    } else {
        let s = (1.0 + r[2][2] - r[0][0] - r[1][1]).sqrt() * 2.0;
        Quat { w: (r[1][0] - r[0][1]) / s, x: (r[0][2] + r[2][0]) / s, y: (r[1][2] + r[2][1]) / s, z: 0.25 * s }
    };
    q.normalize()
}

/// A layer of a mesh's numbers (normals, texture points, colours, materials): read a way, how
/// many numbers to an item.
struct Layer {
    data: Vec<f64>,
    index: Option<Vec<i64>>,
    by: By,
    width: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum By {
    PolygonVertex,
    ControlPoint,
    Polygon,
    All,
}

impl Layer {
    fn of(rec: &Rec, data: &str, index: &str, width: usize) -> Option<Layer> {
        let by = match rec.value("MappingInformationType").and_then(Prop::text).unwrap_or("ByPolygonVertex") {
            "ByVertice" | "ByVertex" | "ByControlPoint" => By::ControlPoint,
            "ByPolygon" => By::Polygon,
            "AllSame" => By::All,
            _ => By::PolygonVertex,
        };
        let indexed = matches!(rec.value("ReferenceInformationType").and_then(Prop::text), Some("IndexToDirect" | "Index"));
        let data = rec.value(data).and_then(Prop::floats)?;
        let index = if indexed { rec.value(index).and_then(Prop::ints) } else { None };
        Some(Layer { data, index, by, width })
    }

    /// Its item for polygon `p`, polygon vertex `pv`, control point `cp`.
    fn at(&self, p: usize, pv: usize, cp: usize) -> Option<&[f64]> {
        let i = match self.by {
            By::PolygonVertex => pv,
            By::ControlPoint => cp,
            By::Polygon => p,
            By::All => 0,
        };
        let i = match &self.index {
            Some(ix) => usize::try_from(*ix.get(i)?).ok()?,
            None => i,
        };
        self.data.get(i * self.width..(i + 1) * self.width)
    }
}

/// The triangles of polygon `poly` (its corners, in the mesh's `points`): an ear at a time (a
/// fan where that finds none).
fn triangles(poly: &[usize], points: &[[f64; 3]]) -> Vec<[usize; 3]> {
    let n = poly.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![[0, 1, 2]];
    }
    // the polygon's facing (Newell's), and its corners on its plane
    let mut nrm = [0.0f64; 3];
    for i in 0..n {
        let (a, b) = (points[poly[i]], points[poly[(i + 1) % n]]);
        nrm[0] += (a[1] - b[1]) * (a[2] + b[2]);
        nrm[1] += (a[2] - b[2]) * (a[0] + b[0]);
        nrm[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let axis = (0..3).max_by(|&i, &j| nrm[i].abs().total_cmp(&nrm[j].abs())).unwrap_or(2);
    let (u, v) = match axis {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let sign = if nrm[axis] < 0.0 { -1.0 } else { 1.0 };
    let flat: Vec<[f64; 2]> = poly.iter().map(|&i| [points[i][u], points[i][v]]).collect();
    let cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) * sign;
    let mut left: Vec<usize> = (0..n).collect();
    let mut out = Vec::new();
    let mut guard = 0;
    while left.len() > 3 && guard < n * n {
        guard += 1;
        let m = left.len();
        let ear = (0..m).find(|&k| {
            let (a, b, c) = (left[(k + m - 1) % m], left[k], left[(k + 1) % m]);
            if cross(flat[a], flat[b], flat[c]) <= 0.0 {
                return false;
            }
            !left.iter().any(|&o| o != a && o != b && o != c && cross(flat[a], flat[b], flat[o]) >= 0.0 && cross(flat[b], flat[c], flat[o]) >= 0.0 && cross(flat[c], flat[a], flat[o]) >= 0.0)
        });
        match ear {
            Some(k) => {
                out.push([left[(k + m - 1) % m], left[k], left[(k + 1) % m]]);
                left.remove(k);
            }
            None => break,
        }
    }
    if left.len() > 3 {
        // no ear: a fan of what is left
        out.extend((1..left.len() - 1).map(|k| [left[0], left[k], left[k + 1]]));
    } else if left.len() == 3 {
        out.push([left[0], left[1], left[2]]);
    }
    out
}

/// The FBX file `bytes` (its pictures looked for beside it, in `dir`).
pub fn read(bytes: &[u8], dir: &Path) -> Result<Model, String> {
    let (version, recs) = if bytes.starts_with(b"Kaydara FBX Binary") { binary(bytes)? } else { ascii(&String::from_utf8_lossy(bytes))? };
    if version != 0 && version < 7000 {
        return Err(format!("this reads FBX 7 (2011 on), not version {}.{}", version / 1000, version % 1000 / 100));
    }
    let top = |name: &str| recs.iter().find(|r| r.name == name);
    let objects = top("Objects").ok_or("the FBX file has no objects")?;
    let mut notes: Vec<String> = Vec::new();
    // every object by its number; what each is connected to (in order), with the property
    let by_id: HashMap<i64, &Rec> = objects.kids.iter().filter_map(|o| Some((o.props.first()?.int()?, o))).collect();
    let mut kids_of: HashMap<i64, Vec<(i64, Option<&str>)>> = HashMap::new();
    let mut parents_of: HashMap<i64, Vec<(i64, Option<&str>)>> = HashMap::new();
    for c in top("Connections").map_or(&[][..], |c| &c.kids[..]).iter().filter(|c| c.name == "C" || c.name == "Connect") {
        let (Some(child), Some(parent)) = (c.props.get(1).and_then(Prop::int), c.props.get(2).and_then(Prop::int)) else { continue };
        let prop = c.props.get(3).and_then(Prop::text);
        kids_of.entry(parent).or_default().push((child, prop));
        parents_of.entry(child).or_default().push((parent, prop));
    }
    let kids = |id: i64, class: &str| -> Vec<i64> { kids_of.get(&id).map_or(Vec::new(), |k| k.iter().filter(|(c, _)| by_id.get(c).is_some_and(|o| o.name == class)).map(|(c, _)| *c).collect()) };
    // the file's axes and unit
    let global = top("GlobalSettings").map(Rec::settings).unwrap_or_default();
    let int = |name: &str, d: i64| global.get(name).and_then(|v| v.first()).and_then(Prop::int).unwrap_or(d);
    let axis = |a: i64, sign: i64| -> [f64; 3] {
        let mut v = [0.0; 3];
        v[a.clamp(0, 2) as usize] = if sign < 0 { -1.0 } else { 1.0 };
        v
    };
    let (up, front, right) = (axis(int("UpAxis", 1), int("UpAxisSign", 1)), axis(int("FrontAxis", 2), int("FrontAxisSign", 1)), axis(int("CoordAxis", 0), int("CoordAxisSign", 1)));
    let unit = global.get("UnitScaleFactor").and_then(|v| v.first()).and_then(Prop::num).filter(|v| *v > 0.0).unwrap_or(1.0) / 100.0;
    let turn = [right, up, front];
    let t = &turn;
    let flipped = t[0][0] * (t[1][1] * t[2][2] - t[1][2] * t[2][1]) - t[0][1] * (t[1][0] * t[2][2] - t[1][2] * t[2][0]) + t[0][2] * (t[1][0] * t[2][1] - t[1][1] * t[2][0]) < 0.0;
    let mut model = Model::default();
    // pictures, each read once
    let mut pictures: HashMap<i64, Option<usize>> = HashMap::new();
    let mut picture = |tex_id: i64, model: &mut Model, notes: &mut Vec<String>| -> Option<usize> {
        if let Some(p) = pictures.get(&tex_id) {
            return *p;
        }
        let tex = by_id.get(&tex_id)?;
        let video = kids_of.get(&tex_id).and_then(|k| k.iter().map(|(c, _)| *c).find(|c| by_id.get(c).is_some_and(|o| o.name == "Video"))).and_then(|v| by_id.get(&v));
        let inside = video.and_then(|v| match v.value("Content") {
            Some(Prop::Bytes(b)) if !b.is_empty() => Some(b.clone()),
            _ => None,
        });
        let file = |r: &Rec, key: &str| r.value(key).and_then(Prop::text).filter(|s| !s.is_empty()).map(str::to_string);
        let names: Vec<String> = [file(tex, "RelativeFilename"), file(tex, "FileName"), file(tex, "Filename"), video.and_then(|v| file(v, "RelativeFilename")), video.and_then(|v| file(v, "Filename"))].into_iter().flatten().collect();
        let bytes = inside.or_else(|| {
            names.iter().find_map(|n| {
                let n = n.replace('\\', "/");
                let p = Path::new(&n);
                let base = p.file_name().map(|f| dir.join(f));
                [dir.join(&n), p.to_path_buf()].into_iter().chain(base).find_map(|q| std::fs::read(q).ok())
            })
        });
        let read = bytes.ok_or_else(|| format!("its file {} is not found", names.first().map_or("(none named)", String::as_str))).and_then(|b| veymelo_os::read_picture_bytes(&b));
        let out = match read {
            Ok(p) => {
                let s = tex.settings();
                let wrap = |key: &str| if s.get(key).and_then(|v| v.first()).and_then(Prop::int) == Some(1) { Wrap::Clamp } else { Wrap::Repeat };
                let mut t = Texture::from_picture(p);
                t.wrap = [wrap("WrapModeU"), wrap("WrapModeV")];
                model.textures.push(t);
                Some(model.textures.len() - 1)
            }
            Err(e) => {
                notes.push(format!("picture \"{}\" cannot be read ({e}): white stands in", object_name(tex.props.get(1))));
                None
            }
        };
        pictures.insert(tex_id, out);
        out
    };
    // materials, each made once
    let mut materials: HashMap<i64, usize> = HashMap::new();
    let mut material = |id: i64, model: &mut Model, notes: &mut Vec<String>| -> Option<usize> {
        if let Some(m) = materials.get(&id) {
            return Some(*m);
        }
        let rec = by_id.get(&id)?;
        let s = rec.settings();
        let diffuse = setting(&s, "DiffuseColor", setting(&s, "Diffuse", [0.8; 3]));
        let k = setting(&s, "DiffuseFactor", [1.0])[0];
        let glow = setting(&s, "EmissiveColor", setting(&s, "Emissive", [0.0; 3]));
        let gk = setting(&s, "EmissiveFactor", [1.0])[0];
        let opacity = match s.get("Opacity").and_then(|v| v.first()).and_then(Prop::num) {
            Some(o) => o,
            None => 1.0 - setting(&s, "TransparencyFactor", [0.0])[0].clamp(0.0, 1.0),
        };
        // shininess 0 to 100, as the square of how smooth it is (as Blender writes it)
        let shine = setting(&s, "ShininessExponent", setting(&s, "Shininess", [20.0]))[0].clamp(0.0, 100.0);
        let map = kids_of
            .get(&id)
            .and_then(|k| k.iter().find(|(c, p)| by_id.get(c).is_some_and(|o| o.name == "Texture") && matches!(p, Some("DiffuseColor" | "Maya|baseColor" | "3dsMax|Parameters|base_color_map"))))
            .and_then(|(c, _)| picture(*c, model, notes))
            .map(|texture| Map { texture, set: 0 });
        let opacity = opacity.clamp(0.0, 1.0) as f32;
        model.materials.push(Material {
            name: object_name(rec.props.get(1)),
            base: [(diffuse[0] * k) as f32, (diffuse[1] * k) as f32, (diffuse[2] * k) as f32, opacity],
            base_map: map,
            metallic: 0.0,
            roughness: (1.0 - (shine / 100.0).sqrt()) as f32,
            emissive: [(glow[0] * gk) as f32, (glow[1] * gk) as f32, (glow[2] * gk) as f32],
            alpha: if opacity < 1.0 { AlphaMode::Blend } else { AlphaMode::Opaque },
            two_sided: true,
            ..Material::default()
        });
        materials.insert(id, model.materials.len() - 1);
        Some(model.materials.len() - 1)
    };
    // the models (nodes), in the file's order
    let models: Vec<(i64, &Rec)> = objects.kids.iter().filter(|o| o.name == "Model").filter_map(|o| Some((o.props.first()?.int()?, o))).collect();
    let node_of: HashMap<i64, usize> = models.iter().enumerate().map(|(i, (id, _))| (*id, i)).collect();
    for (id, rec) in &models {
        let s = rec.settings();
        let local = place(&s, setting(&s, "Lcl Translation", [0.0; 3]), setting(&s, "Lcl Rotation", [0.0; 3]), setting(&s, "Lcl Scaling", [1.0; 3]));
        let mut node = Node { name: object_name(rec.props.get(1)), matrix: Some(local), ..Node::default() };
        // its mesh: the geometry, moved by its geometric transform, in its materials
        if let Some(geo) = kids(*id, "Geometry").first().and_then(|g| by_id.get(g)) {
            let gt = setting(&s, "GeometricTranslation", [0.0; 3]);
            let gr = euler(setting(&s, "GeometricRotation", [0.0; 3]), 0);
            let gs = setting(&s, "GeometricScaling", [1.0; 3]);
            let mats: Vec<Option<usize>> = kids(*id, "Material").into_iter().map(|m| material(m, &mut model, &mut notes)).collect();
            if let Some(mesh) = mesh(geo, &mats, gt, gr, gs, flipped, &mut notes) {
                model.meshes.push(mesh);
                node.mesh = Some(model.meshes.len() - 1);
            }
        }
        model.nodes.push(node);
    }
    // the tree, and the file's axes and unit at its top
    let mut tops = Vec::new();
    for (i, (id, _)) in models.iter().enumerate() {
        match parents_of.get(id).and_then(|p| p.iter().find_map(|(p, _)| node_of.get(p))) {
            Some(&parent) if parent != i => model.nodes[parent].children.push(i),
            _ => tops.push(i),
        }
    }
    let r = turn.map(|row| row.map(|v| v * unit));
    model.nodes.push(Node { name: "FBX".into(), matrix: Some(mat(r, [0.0; 3])), children: tops, ..Node::default() });
    model.roots = vec![model.nodes.len() - 1];
    // animations: each stack's moved models, worked out at every moment a key is at
    for stack in objects.kids.iter().filter(|o| o.name == "AnimationStack") {
        let Some(sid) = stack.props.first().and_then(Prop::int) else { continue };
        let mut moved: HashMap<usize, [Option<Moved>; 3]> = HashMap::new();
        for layer in kids(sid, "AnimationLayer") {
            for cn in kids(layer, "AnimationCurveNode") {
                let Some(node_rec) = by_id.get(&cn) else { continue };
                let Some((target, what)) = parents_of.get(&cn).and_then(|p| p.iter().find_map(|(p, prop)| Some((*node_of.get(p)?, (*prop)?)))) else { continue };
                let k = match what {
                    "Lcl Translation" => 0,
                    "Lcl Rotation" => 1,
                    "Lcl Scaling" => 2,
                    _ => continue,
                };
                let curves: Vec<Option<&Rec>> = ["d|X", "d|Y", "d|Z"].iter().map(|axis| kids_of.get(&cn).and_then(|c| c.iter().find(|(_, p)| *p == Some(*axis))).and_then(|(c, _)| by_id.get(c).copied())).collect();
                moved.entry(target).or_default()[k] = Some((*node_rec, curves));
            }
        }
        if moved.is_empty() {
            continue;
        }
        let mut anim = Animation { name: object_name(stack.props.get(1)), channels: Vec::new() };
        let mut order: Vec<usize> = moved.keys().copied().collect();
        order.sort_unstable();
        for target in order {
            let parts_of = &moved[&target];
            let rec = models[target].1;
            let s = rec.settings();
            let rest = [setting(&s, "Lcl Translation", [0.0; 3]), setting(&s, "Lcl Rotation", [0.0; 3]), setting(&s, "Lcl Scaling", [1.0; 3])];
            // each part's curves (their times in seconds and values), else its resting value
            let read: Vec<Vec<Option<Curve>>> = parts_of
                .iter()
                .map(|p| match p {
                    None => vec![None, None, None],
                    Some((_, curves)) => curves
                        .iter()
                        .map(|c| {
                            let c = (*c)?;
                            let times: Vec<f64> = c.value("KeyTime").and_then(Prop::ints)?.iter().map(|&t| t as f64 / TICKS).collect();
                            let values = c.value("KeyValueFloat").and_then(Prop::floats)?;
                            // held (constant) keys: the second bit of their first flags
                            let step = c.value("KeyAttrFlags").and_then(Prop::ints).and_then(|f| f.first().copied()).is_some_and(|f| f & 0x2 != 0);
                            (times.len() == values.len() && !times.is_empty()).then_some((times, values, step))
                        })
                        .collect(),
                })
                .collect();
            let defaults: Vec<[f64; 3]> = (0..3)
                .map(|k| match &parts_of[k] {
                    Some((cn, _)) => {
                        let s = cn.settings();
                        [setting(&s, "d|X", [rest[k][0]])[0], setting(&s, "d|Y", [rest[k][1]])[0], setting(&s, "d|Z", [rest[k][2]])[0]]
                    }
                    None => rest[k],
                })
                .collect();
            let mut times: Vec<f64> = read.iter().flatten().flatten().flat_map(|(t, ..)| t.iter().copied()).collect();
            times.sort_by(f64::total_cmp);
            times.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
            let value = |k: usize, i: usize, t: f64| -> f64 {
                match &read[k][i] {
                    Some((ts, vs, step)) => {
                        let n = ts.len();
                        if t <= ts[0] {
                            return vs[0];
                        }
                        if t >= ts[n - 1] {
                            return vs[n - 1];
                        }
                        let j = ts.partition_point(|x| *x <= t).clamp(1, n - 1) - 1;
                        if *step {
                            return vs[j];
                        }
                        let f = (t - ts[j]) / (ts[j + 1] - ts[j]).max(1e-12);
                        vs[j] + (vs[j + 1] - vs[j]) * f
                    }
                    None => defaults[k][i],
                }
            };
            let mut tv = Vec::new();
            let mut rv: Vec<f32> = Vec::new();
            let mut sv = Vec::new();
            let mut last: Option<Quat> = None;
            for &t in &times {
                let at = |k: usize| [value(k, 0, t), value(k, 1, t), value(k, 2, t)];
                let (tr, mut q, sc) = parts(&place(&s, at(0), at(1), at(2)));
                // the shorter way from the turn before
                if let Some(p) = last
                    && p.w * q.w + p.x * q.x + p.y * q.y + p.z * q.z < 0.0
                {
                    q = Quat { w: -q.w, x: -q.x, y: -q.y, z: -q.z };
                }
                last = Some(q);
                tv.extend(tr.map(|v| v as f32));
                rv.extend([q.x, q.y, q.z, q.w].map(|v| v as f32));
                sv.extend(sc.map(|v| v as f32));
            }
            let times: Vec<f32> = times.iter().map(|&t| t as f32).collect();
            for (moves, values, width) in [(Moves::Translation, tv, 3), (Moves::Rotation, rv, 4), (Moves::Scale, sv, 3)] {
                anim.channels.push(Channel { node: target, moves, interp: Interp::Linear, times: times.clone(), values, width });
            }
            // moved by its parts from now on, as they rest when nothing moves it
            let (tr, q, sc) = parts(&place(&s, rest[0], rest[1], rest[2]));
            let node = &mut model.nodes[target];
            (node.matrix, node.translation, node.rotation, node.scale) = (None, tr, q, sc);
        }
        model.animations.push(anim);
    }
    // what is not read
    let has = |class: &str, kind: &str| objects.kids.iter().any(|o| o.name == class && o.props.get(2).and_then(Prop::text).is_some_and(|k| k == kind));
    if has("Deformer", "Skin") {
        notes.push("its skins are not read: meshes stay as they rest".into());
    }
    if has("Deformer", "BlendShape") {
        notes.push("its blend shapes are not read".into());
    }
    if has("NodeAttribute", "Camera") || has("NodeAttribute", "Light") {
        notes.push("its cameras and lights are not read".into());
    }
    model.notes = notes;
    Ok(model)
}

/// Geometry `geo` as a mesh: its polygons made triangles (a primitive for each of its model's
/// materials `mats`), moved by the geometric transform (`gt`, `gr`, `gs`), turned round where the
/// file's axes are mirrored (`flipped`).
fn mesh(geo: &Rec, mats: &[Option<usize>], gt: [f64; 3], gr: [[f64; 3]; 3], gs: [f64; 3], flipped: bool, notes: &mut Vec<String>) -> Option<Mesh> {
    let name = object_name(geo.props.get(1));
    if geo.props.get(2).and_then(Prop::text).is_some_and(|k| k != "Mesh") {
        return None;
    }
    let raw = geo.value("Vertices").and_then(Prop::floats)?;
    let points: Vec<[f64; 3]> = raw.as_chunks::<3>().0.to_vec();
    let corners = geo.value("PolygonVertexIndex").and_then(Prop::ints)?;
    let first_of = |name: &'static str| geo.kids_named(name).min_by_key(|l| l.props.first().and_then(Prop::int).unwrap_or(0));
    let normals = first_of("LayerElementNormal").and_then(|l| Layer::of(l, "Normals", "NormalsIndex", 3));
    let mut uv_layers: Vec<&Rec> = geo.kids_named("LayerElementUV").collect();
    uv_layers.sort_by_key(|l| l.props.first().and_then(Prop::int).unwrap_or(0));
    let uvs: Vec<Layer> = uv_layers.iter().take(2).filter_map(|l| Layer::of(l, "UV", "UVIndex", 2)).collect();
    let colors = first_of("LayerElementColor").and_then(|l| Layer::of(l, "Colors", "ColorIndex", 4));
    let materials = first_of("LayerElementMaterial").and_then(|l| Layer::of(l, "Materials", "", 1));
    // the geometric transform, for points and (its inverse turned over) for normals
    let at = |p: [f64; 3]| -> [f32; 3] { std::array::from_fn(|i| ((0..3).map(|j| gr[i][j] * p[j] * gs[j]).sum::<f64>() + gt[i]) as f32) };
    let way = |n: &[f64]| -> [f32; 3] {
        let v: [f64; 3] = std::array::from_fn(|i| (0..3).map(|j| gr[i][j] * n[j] / gs[j]).sum::<f64>());
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-30);
        v.map(|x| (x / l) as f32)
    };
    let mut prims: Vec<Primitive> = (0..mats.len().max(1)).map(|i| Primitive { material: mats.get(i).copied().flatten(), ..Primitive::default() }).collect();
    let (mut poly, mut first_pv, mut p) = (Vec::new(), 0usize, 0usize);
    let mut bad = 0usize;
    for (pv, &c) in corners.iter().enumerate() {
        let (idx, last) = if c < 0 { (!c, true) } else { (c, false) };
        poly.push(idx as usize);
        if !last {
            continue;
        }
        if poly.iter().any(|&i| i >= points.len()) {
            bad += 1;
        } else {
            let which = materials.as_ref().and_then(|m| m.at(p, first_pv, 0)).map_or(0, |v| v[0].max(0.0) as usize).min(prims.len() - 1);
            let prim = &mut prims[which];
            for tri in triangles(&poly, &points) {
                let tri = if flipped { [tri[0], tri[2], tri[1]] } else { tri };
                for k in tri {
                    let (cp, pvk) = (poly[k], first_pv + k);
                    prim.indices.push(prim.positions.len() as u32);
                    prim.positions.push(at(points[cp]));
                    if let Some(n) = normals.as_ref().and_then(|l| l.at(p, pvk, cp)) {
                        prim.normals.push(way(n));
                    }
                    for (s, l) in uvs.iter().enumerate() {
                        let uv = l.at(p, pvk, cp).unwrap_or(&[0.0, 0.0]);
                        // FBX's v goes up from the picture's bottom
                        prim.uvs[s].push([uv[0] as f32, 1.0 - uv[1] as f32]);
                    }
                    if let Some(c) = colors.as_ref().and_then(|l| l.at(p, pvk, cp)) {
                        prim.colors.push([c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32]);
                    }
                }
            }
        }
        poly.clear();
        first_pv = pv + 1;
        p += 1;
    }
    if bad > 0 {
        notes.push(format!("mesh \"{name}\": {bad} polygons name points it does not have: left out"));
    }
    let mut out = Vec::new();
    for mut prim in prims.into_iter().filter(|p| !p.indices.is_empty()) {
        let n = prim.positions.len();
        // each kind of number for every vertex, or not at all
        if prim.normals.len() != n {
            prim.normals.clear();
        }
        for s in 0..2 {
            if prim.uvs[s].len() != n {
                prim.uvs[s].clear();
            }
        }
        if prim.colors.len() != n {
            prim.colors.clear();
        }
        if prim.normals.is_empty() {
            prim.flat();
        }
        out.push(prim);
    }
    (!out.is_empty()).then_some(Mesh { name, primitives: out, weights: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A binary FBX file (version `version`) of records, written as the format lays them out.
    fn write(version: u32, recs: &[Rec]) -> Vec<u8> {
        let wide = version >= 7500;
        let mut out = b"Kaydara FBX Binary  \x00\x1a\x00".to_vec();
        out.extend(version.to_le_bytes());
        for r in recs {
            put(&mut out, r, wide);
        }
        out.extend(vec![0u8; if wide { 25 } else { 13 }]);
        out
    }

    fn put(out: &mut Vec<u8>, r: &Rec, wide: bool) {
        let start = out.len();
        let uint = |out: &mut Vec<u8>, v: u64| if wide { out.extend(v.to_le_bytes()) } else { out.extend((v as u32).to_le_bytes()) };
        uint(out, 0);
        uint(out, r.props.len() as u64);
        uint(out, 0);
        out.push(r.name.len() as u8);
        out.extend(r.name.as_bytes());
        let props_at = out.len();
        for p in &r.props {
            match p {
                Prop::Int(v) => {
                    out.push(b'L');
                    out.extend(v.to_le_bytes());
                }
                Prop::Float(v) => {
                    out.push(b'D');
                    out.extend(v.to_le_bytes());
                }
                Prop::Text(s) => {
                    out.push(b'S');
                    out.extend((s.len() as u32).to_le_bytes());
                    out.extend(s.as_bytes());
                }
                Prop::Bytes(b) => {
                    out.push(b'R');
                    out.extend((b.len() as u32).to_le_bytes());
                    out.extend(b);
                }
                // whole numbers kept compressed (32-bit where they fit), the others as they are
                Prop::Ints(v) => {
                    let narrow = v.iter().all(|x| i32::try_from(*x).is_ok());
                    let raw: Vec<u8> = if narrow { v.iter().flat_map(|x| (*x as i32).to_le_bytes()).collect() } else { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
                    let z = veymelo_zip::zlib(&raw);
                    out.push(if narrow { b'i' } else { b'l' });
                    out.extend((v.len() as u32).to_le_bytes());
                    out.extend(1u32.to_le_bytes());
                    out.extend((z.len() as u32).to_le_bytes());
                    out.extend(z);
                }
                Prop::Floats(v) => {
                    out.push(b'd');
                    out.extend((v.len() as u32).to_le_bytes());
                    out.extend(0u32.to_le_bytes());
                    out.extend(((v.len() * 8) as u32).to_le_bytes());
                    out.extend(v.iter().flat_map(|x| x.to_le_bytes()));
                }
            }
        }
        let list = out.len() - props_at;
        for k in &r.kids {
            put(out, k, wide);
        }
        if !r.kids.is_empty() {
            out.extend(vec![0u8; if wide { 25 } else { 13 }]);
        }
        let end = out.len() as u64;
        let mut at = start;
        let mut set = |v: u64| {
            if wide {
                out[at..at + 8].copy_from_slice(&v.to_le_bytes());
                at += 8;
            } else {
                out[at..at + 4].copy_from_slice(&(v as u32).to_le_bytes());
                at += 4;
            }
        };
        set(end);
        set(r.props.len() as u64);
        set(list as u64);
    }

    fn rec(name: &str, props: Vec<Prop>, kids: Vec<Rec>) -> Rec {
        Rec { name: name.into(), props, kids }
    }

    fn p(name: &str, values: &[f64]) -> Rec {
        let mut props = vec![Prop::Text(name.into()), Prop::Text("".into()), Prop::Text("".into()), Prop::Text("A".into())];
        props.extend(values.iter().map(|v| Prop::Float(*v)));
        rec("P", props, vec![])
    }

    fn text(s: &str) -> Prop {
        Prop::Text(s.into())
    }

    /// A unit cube of six quads (Z up in the file, centimetres), red, raised by a pre-rotation
    /// and a translation; the translation keyed from 0 to 1 second.
    fn cube(version: u32) -> Vec<u8> {
        let v = [[-1.0, -1.0, -1.0], [1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [-1.0, 1.0, -1.0], [-1.0, -1.0, 1.0], [1.0, -1.0, 1.0], [1.0, 1.0, 1.0], [-1.0, 1.0, 1.0]];
        let quads: [[i64; 4]; 6] = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]];
        let corners: Vec<i64> = quads.iter().flat_map(|q| [q[0], q[1], q[2], !q[3]]).collect();
        let uv: Vec<f64> = vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
        let uv_index: Vec<i64> = (0..24).map(|i| i % 4).collect();
        let global = rec("GlobalSettings", vec![], vec![rec("Properties70", vec![], vec![p("UpAxis", &[2.0]), p("UpAxisSign", &[1.0]), p("FrontAxis", &[1.0]), p("FrontAxisSign", &[-1.0]), p("CoordAxis", &[0.0]), p("CoordAxisSign", &[1.0]), p("UnitScaleFactor", &[1.0])])]);
        let geometry = rec(
            "Geometry",
            vec![Prop::Int(10), text("Cube\u{0}\u{1}Geometry"), text("Mesh")],
            vec![
                rec("Vertices", vec![Prop::Floats(v.iter().flatten().copied().collect())], vec![]),
                rec("PolygonVertexIndex", vec![Prop::Ints(corners)], vec![]),
                rec("LayerElementUV", vec![Prop::Int(0)], vec![rec("MappingInformationType", vec![text("ByPolygonVertex")], vec![]), rec("ReferenceInformationType", vec![text("IndexToDirect")], vec![]), rec("UV", vec![Prop::Floats(uv)], vec![]), rec("UVIndex", vec![Prop::Ints(uv_index)], vec![])]),
                rec("LayerElementMaterial", vec![Prop::Int(0)], vec![rec("MappingInformationType", vec![text("AllSame")], vec![]), rec("ReferenceInformationType", vec![text("IndexToDirect")], vec![]), rec("Materials", vec![Prop::Ints(vec![0])], vec![])]),
            ],
        );
        let model = rec("Model", vec![Prop::Int(20), text("Box\u{0}\u{1}Model"), text("Mesh")], vec![rec("Properties70", vec![], vec![p("Lcl Translation", &[0.0, 0.0, 100.0]), p("PreRotation", &[0.0, 0.0, 90.0])])]);
        let material = rec("Material", vec![Prop::Int(30), text("Red\u{0}\u{1}Material"), text("")], vec![rec("Properties70", vec![], vec![p("DiffuseColor", &[0.8, 0.1, 0.1]), p("DiffuseFactor", &[0.5])])]);
        let ticks = TICKS as i64;
        let stack = rec("AnimationStack", vec![Prop::Int(40), text("Take\u{0}\u{1}AnimStack"), text("")], vec![]);
        let layer = rec("AnimationLayer", vec![Prop::Int(41), text("Base\u{0}\u{1}AnimLayer"), text("")], vec![]);
        let node = rec("AnimationCurveNode", vec![Prop::Int(42), text("T\u{0}\u{1}AnimCurveNode"), text("")], vec![rec("Properties70", vec![], vec![p("d|X", &[0.0]), p("d|Y", &[0.0]), p("d|Z", &[100.0])])]);
        let curve = rec("AnimationCurve", vec![Prop::Int(43), text("\u{0}\u{1}AnimCurve"), text("")], vec![rec("KeyTime", vec![Prop::Ints(vec![0, ticks])], vec![]), rec("KeyValueFloat", vec![Prop::Floats(vec![100.0, 300.0])], vec![])]);
        let c = |kind: &str, a: i64, b: i64, prop: Option<&str>| {
            let mut props = vec![text(kind), Prop::Int(a), Prop::Int(b)];
            props.extend(prop.map(text));
            rec("C", props, vec![])
        };
        let connections = rec("Connections", vec![], vec![c("OO", 20, 0, None), c("OO", 10, 20, None), c("OO", 30, 20, None), c("OO", 41, 40, None), c("OO", 42, 41, None), c("OP", 42, 20, Some("Lcl Translation")), c("OP", 43, 42, Some("d|Z"))]);
        let objects = rec("Objects", vec![], vec![geometry, model, material, stack, layer, node, curve]);
        write(version, &[rec("FBXHeaderExtension", vec![], vec![rec("FBXVersion", vec![Prop::Int(version as i64)], vec![])]), global, objects, connections])
    }

    #[test]
    fn a_binary_cube_is_read_turned_to_y_up_in_metres() {
        for version in [7400, 7500] {
            let mut bytes = cube(version);
            let m = read(&bytes, Path::new(".")).unwrap();
            assert_eq!(m.triangles(), 12, "version {version}");
            assert_eq!(m.materials.len(), 1);
            assert!((m.materials[0].base[0] - 0.4).abs() < 1e-6 && (m.materials[0].base[1] - 0.05).abs() < 1e-6);
            let prim = &m.meshes[0].primitives[0];
            assert_eq!(prim.positions.len(), 36);
            assert_eq!(prim.uvs[0].len(), 36);
            // flat normals made (the file gives none), pointing out of the cube
            assert!(prim.normals.iter().zip(prim.positions.chunks(3).flat_map(|t| [t[0], t[0], t[0]])).all(|(n, _)| (n[0] * n[0] + n[1] * n[1] + n[2] * n[2] - 1.0).abs() < 1e-5));
            // the file's z (up, 1 m above the ground at rest) is the model's y; at 1 s it is 3 m
            let at = |t: f64| {
                let pose = m.pose(Some(0), t);
                let n = m.nodes.iter().position(|n| n.mesh.is_some()).unwrap();
                pose.world[n].apply(veymelo_geom::vec3(0.0, 0.0, 0.0))
            };
            let rest = m.pose(None, 0.0).world[0].apply(veymelo_geom::vec3(0.0, 0.0, 0.0));
            assert!((rest.y - 1.0).abs() < 1e-9 && rest.x.abs() < 1e-9 && rest.z.abs() < 1e-9, "{rest:?}");
            assert!((at(0.0).y - 1.0).abs() < 1e-6, "{:?}", at(0.0));
            assert!((at(0.5).y - 2.0).abs() < 1e-6 && (at(1.0).y - 3.0).abs() < 1e-6);
            // the cube spans 2 cm in the file: 0.02 m
            let pose = m.pose(None, 0.0);
            let xs: Vec<f64> = prim.positions.iter().map(|p| pose.world[0].apply(veymelo_geom::vec3(p[0] as f64, p[1] as f64, p[2] as f64)).x).collect();
            let w = xs.iter().fold(f64::NEG_INFINITY, |a, b| a.max(*b)) - xs.iter().fold(f64::INFINITY, |a, b| a.min(*b));
            assert!((w - 0.02).abs() < 1e-9, "{w}");
            bytes.truncate(40);
            assert!(read(&bytes, Path::new(".")).is_err());
        }
    }

    #[test]
    fn an_ascii_file_reads_as_the_binary_one_does() {
        let text = r#"; FBX 7.4.0 project file
FBXHeaderExtension:  {
	FBXVersion: 7400
}
GlobalSettings:  {
	Properties70:  {
		P: "UnitScaleFactor", "double", "Number", "",100
	}
}
Objects:  {
	Geometry: 1, "Geometry::Tri", "Mesh" {
		Vertices: *12 {
			a: 0,0,0,1,0,0,1,1,0,0,1,0
		}
		PolygonVertexIndex: *4 {
			a: 0,1,2,-4
		}
		LayerElementNormal: 0 {
			MappingInformationType: "ByVertice"
			ReferenceInformationType: "Direct"
			Normals: *12 {
				a: 0,0,1,0,0,1,0,0,1,0,0,1
			}
		}
	}
	Model: 2, "Model::Quad", "Mesh" {
		Properties70:  {
			P: "Lcl Rotation", "Lcl Rotation", "", "A",0,0,90
		}
	}
	Material: 3, "Material::Green", "" {
		Properties70:  {
			P: "DiffuseColor", "Color", "", "A",0.1,0.7,0.2
			P: "Opacity", "double", "Number", "",0.5
		}
	}
}
Connections:  {
	C: "OO",2,0
	C: "OO",1,2
	C: "OO",3,2
}
"#;
        let m = read(text.as_bytes(), Path::new(".")).unwrap();
        assert_eq!(m.triangles(), 2);
        let prim = &m.meshes[0].primitives[0];
        assert_eq!(prim.normals[0], [0.0, 0.0, 1.0]);
        assert_eq!(m.materials[0].alpha, AlphaMode::Blend);
        assert!((m.materials[0].base[3] - 0.5).abs() < 1e-6);
        // turned a quarter about z (counter-clockwise): (1, 0) goes to (0, 1)
        let n = m.nodes.iter().position(|n| n.mesh.is_some()).unwrap();
        let p = m.pose(None, 0.0).world[n].apply(veymelo_geom::vec3(1.0, 0.0, 0.0));
        assert!((p.x).abs() < 1e-9 && (p.y - 1.0).abs() < 1e-9, "{p:?}");
    }

    #[test]
    fn concave_polygons_are_cut_into_ears() {
        // an L of six corners
        let pts = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0], [1.0, 1.0, 0.0], [1.0, 2.0, 0.0], [0.0, 2.0, 0.0]];
        let tris = triangles(&[0, 1, 2, 3, 4, 5], &pts);
        assert_eq!(tris.len(), 4);
        let area: f64 = tris.iter().map(|t| {
            let (a, b, c) = (pts[t[0]], pts[t[1]], pts[t[2]]);
            ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])) / 2.0
        }).sum();
        assert!((area - 3.0).abs() < 1e-12, "{area}");
    }

    #[test]
    fn turns_come_back_as_they_went() {
        for deg in [[10.0, 20.0, 30.0], [170.0, -80.0, 45.0], [0.0, 180.0, 0.0]] {
            for order in 0..6 {
                let r = euler(deg, order);
                let q = quat(r);
                let back = q.to_mat4();
                for i in 0..3 {
                    for j in 0..3 {
                        assert!((back.0[i][j] - r[i][j]).abs() < 1e-9, "{deg:?} {order}");
                    }
                }
            }
        }
    }
}

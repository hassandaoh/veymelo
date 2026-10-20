//! OpenEXR files (`.exr`), read and written by our own code from the OpenEXR file layout
//! (Industrial Light & Magic, now the Academy Software Foundation): a file of one or more parts
//! (multi-part files), each of scanlines or of tiles (of one level, mipmaps or ripmaps), each
//! channel kept as 32-bit whole numbers, 16-bit halves or 32-bit floats, some at a lower rate
//! (sampled every so many pixels), in every compression there is but the newest: none, RLE,
//! ZIPS and ZIP, PIZ (wavelets and Huffman codes), PXR24, B44 and B44A, and DWAA and DWAB (the
//! lossy DCT ones).
//!
//! What is read:
//! - the headers, every attribute (the ones we know as values, the rest as bytes);
//! - each part's chunks, from its chunk table, or found one after another when the table is
//!   broken (a file cut short while it was written);
//! - every channel of every part, at its first (largest) level, as a [`Plane`] of floats over
//!   its part's data window;
//! - planes named as layers (`part.channel`, unless the channel already carries its part's
//!   name);
//! - the picture of a file with many layers ([`picture`]): its unnamed layer with colour, or
//!   its beauty pass by name, never a data pass; luminance and chroma (`Y`, `RY`, `BY`) made
//!   red, green and blue.
//!
//! Deep files (many samples a pixel) and the HTJ2K compressions are not read, and say so.

mod b44;
mod codec;
mod dwa;
pub mod half;
mod huf;
mod piz;
mod picture;
pub mod write;

pub use picture::{Picture, layer_rgba, layers, picture, picture_layer, read_picture};

/// The first four bytes of every OpenEXR file.
pub const MAGIC: [u8; 4] = [0x76, 0x2f, 0x31, 0x01];

/// Whether `bytes` start as an OpenEXR file does.
pub fn looks_like(bytes: &[u8]) -> bool {
    bytes.starts_with(&MAGIC)
}

/// How a channel keeps its samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    /// 32-bit whole numbers (object numbers…).
    Uint,
    Half,
    Float,
}

impl Sample {
    /// Bytes a sample.
    pub fn size(self) -> usize {
        match self {
            Sample::Half => 2,
            _ => 4,
        }
    }
}

/// A channel of a part.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub name: String,
    pub sample: Sample,
    /// Its values are perceptually even already (the lossy compressions leave them so).
    pub linear: bool,
    /// Kept every so many pixels across and down (1, 1: every pixel).
    pub sampling: [i32; 2],
}

/// A rectangle of pixels, both its corners inside it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Window {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Window {
    pub fn width(&self) -> usize {
        (self.x1 as i64 - self.x0 as i64 + 1).max(0) as usize
    }

    pub fn height(&self) -> usize {
        (self.y1 as i64 - self.y0 as i64 + 1).max(0) as usize
    }
}

/// How a part's chunks are made smaller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None,
    /// Runs of bytes.
    Rle,
    /// Deflate, a scanline a chunk.
    Zips,
    /// Deflate, sixteen scanlines a chunk.
    Zip,
    /// Wavelets and Huffman codes.
    Piz,
    /// Floats cut to 24 bits, then deflate.
    Pxr24,
    /// Halves in 4 × 4 blocks, a fixed size each (lossy).
    B44,
    /// B44 with blocks of one value smaller.
    B44a,
    /// DCT of 8 × 8 blocks (lossy), 32 scanlines a chunk.
    Dwaa,
    /// DWAA, 256 scanlines a chunk.
    Dwab,
    /// One we do not read (the HTJ2K ones, or newer).
    Other(u8),
}

impl Compression {
    fn of(v: u8) -> Compression {
        use Compression::*;
        match v {
            0 => None,
            1 => Rle,
            2 => Zips,
            3 => Zip,
            4 => Piz,
            5 => Pxr24,
            6 => B44,
            7 => B44a,
            8 => Dwaa,
            9 => Dwab,
            v => Other(v),
        }
    }

    pub fn name(self) -> String {
        use Compression::*;
        match self {
            None => "none",
            Rle => "RLE",
            Zips => "ZIPS",
            Zip => "ZIP",
            Piz => "PIZ",
            Pxr24 => "PXR24",
            B44 => "B44",
            B44a => "B44A",
            Dwaa => "DWAA",
            Dwab => "DWAB",
            Other(10 | 11) => "HTJ2K",
            Other(v) => return format!("number {v}"),
        }
        .to_string()
    }

    /// Scanlines a chunk holds.
    fn lines(self) -> Option<i32> {
        use Compression::*;
        Some(match self {
            None | Rle | Zips => 1,
            Zip | Pxr24 => 16,
            Piz | B44 | B44a | Dwaa => 32,
            Dwab => 256,
            Other(_) => return Option::None,
        })
    }
}

/// How a tiled part keeps smaller copies of itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Levels {
    One,
    /// Halved both ways each level.
    Mipmap,
    /// Halved each way on its own.
    Ripmap,
}

/// A tiled part's tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tiles {
    pub width: u32,
    pub height: u32,
    pub levels: Levels,
    /// Halving rounds up (else down).
    pub round_up: bool,
}

/// What a part holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Scanlines,
    Tiles,
    /// Many samples a pixel (not read).
    DeepScanlines,
    DeepTiles,
}

/// An attribute's value, as far as it is read.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i32),
    Float(f32),
    Double(f64),
    Text(String),
    Texts(Vec<String>),
    /// `box2i`, `v2i`, `v3i`, `rational`.
    Ints(Vec<i32>),
    /// `box2f`, `v2f`, `v3f`, `m33f`, `m44f`, `chromaticities`.
    Floats(Vec<f32>),
    /// `v2d`, `v3d`, `m33d`, `m44d`.
    Doubles(Vec<f64>),
    /// Any other kind, as it is kept.
    Bytes(Vec<u8>),
}

/// An attribute of a part's header: its name, the name of its kind, its value.
#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub kind: String,
    pub value: Value,
}

/// A part of a file: one picture of channels.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    /// Its name (in multi-part files).
    pub name: Option<String>,
    pub kind: Kind,
    /// Its channels, by name (as the file keeps them).
    pub channels: Vec<Channel>,
    pub compression: Compression,
    /// Where it has pixels.
    pub data: Window,
    /// The frame it is seen in.
    pub display: Window,
    /// Its scanlines were written bottom first.
    pub decreasing: bool,
    pub tiles: Option<Tiles>,
    pub pixel_aspect: f32,
    /// Every attribute of its header.
    pub attributes: Vec<Attribute>,
    /// Where its chunks start in the file.
    pub chunks: Vec<usize>,
}

impl Part {
    /// An attribute's value.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.attributes.iter().find(|a| a.name == name).map(|a| &a.value)
    }

    /// A text attribute.
    pub fn text(&self, name: &str) -> Option<&str> {
        match self.get(name)? {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }

    /// The red, green, blue and white its colours are made of (x, y each), when it says.
    pub fn chromaticities(&self) -> Option<[[f32; 2]; 4]> {
        match self.get("chromaticities")? {
            Value::Floats(v) if v.len() == 8 => Some([[v[0], v[1]], [v[2], v[3]], [v[4], v[5]], [v[6], v[7]]]),
            _ => None,
        }
    }

    /// The pixels a chunk covers: `lines` scanlines, or a tile.
    fn tile_size(&self) -> (i32, i32) {
        match self.tiles {
            Some(t) => (t.width.min(i32::MAX as u32) as i32, t.height.min(i32::MAX as u32) as i32),
            None => (self.data.width() as i32, self.compression.lines().unwrap_or(1)),
        }
    }
}

/// An OpenEXR file's headers and where its chunks are.
#[derive(Clone, Debug, PartialEq)]
pub struct Exr {
    /// The format's version (2) and its flags.
    pub version: u32,
    pub multipart: bool,
    pub parts: Vec<Part>,
}

/// One channel's samples over its part's data window, row by row, as floats (whole numbers
/// for `Uint` channels, exact up to 2²⁴). A channel kept at a lower rate is widened to every
/// pixel (each takes the sample at or before it).
#[derive(Clone, Debug, PartialEq)]
pub struct Plane {
    pub name: String,
    /// The part it is of.
    pub part: usize,
    pub window: Window,
    pub samples: Vec<f32>,
}

impl Plane {
    /// Its samples over another window (the display window, say): `fill` where it has none.
    pub fn placed(&self, frame: Window, fill: f32) -> Vec<f32> {
        let (w, h) = (frame.width(), frame.height());
        if self.window == frame {
            return self.samples.clone();
        }
        let mut out = vec![fill; w * h];
        let pw = self.window.width();
        for y in self.window.y0.max(frame.y0)..=self.window.y1.min(frame.y1) {
            let (x0, x1) = (self.window.x0.max(frame.x0), self.window.x1.min(frame.x1));
            if x0 > x1 {
                break;
            }
            let from = (y - self.window.y0) as usize * pw + (x0 - self.window.x0) as usize;
            let to = (y - frame.y0) as usize * w + (x0 - frame.x0) as usize;
            let n = (x1 - x0 + 1) as usize;
            out[to..to + n].copy_from_slice(&self.samples[from..from + n]);
        }
        out
    }
}

/// The most samples a plane may have (a picture 32768 pixels square).
const MOST: usize = 1 << 30;

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.b.len()).ok_or("the file ends too soon")?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().expect("four bytes")))
    }

    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("eight bytes")))
    }

    /// A name ended by a zero byte.
    fn name(&mut self) -> Result<String, String> {
        let rest = &self.b[self.at.min(self.b.len())..];
        let n = rest.iter().position(|b| *b == 0).ok_or("a name that never ends")?;
        if n > 255 {
            return Err("a name longer than 255 bytes".into());
        }
        let s = String::from_utf8_lossy(&rest[..n]).into_owned();
        self.at += n + 1;
        Ok(s)
    }
}

fn le_i32(b: &[u8], k: usize) -> i32 {
    i32::from_le_bytes([b[4 * k], b[4 * k + 1], b[4 * k + 2], b[4 * k + 3]])
}

fn value(kind: &str, b: &[u8]) -> Value {
    let ints = || (0..b.len() / 4).map(|k| le_i32(b, k)).collect();
    let floats = || (0..b.len() / 4).map(|k| f32::from_bits(le_i32(b, k) as u32)).collect();
    let doubles = || b.as_chunks::<8>().0.iter().map(|c| f64::from_le_bytes(*c)).collect();
    match kind {
        "int" if b.len() == 4 => Value::Int(le_i32(b, 0)),
        "float" if b.len() == 4 => Value::Float(f32::from_bits(le_i32(b, 0) as u32)),
        "double" if b.len() == 8 => Value::Double(f64::from_le_bytes(b.try_into().expect("eight bytes"))),
        "compression" | "lineOrder" | "envmap" | "deepImageState" if b.len() == 1 => Value::Int(b[0] as i32),
        "string" => Value::Text(String::from_utf8_lossy(b).into_owned()),
        "stringvector" => {
            let (mut v, mut at) = (Vec::new(), 0);
            while at + 4 <= b.len() {
                let n = (le_i32(&b[at..], 0).max(0) as usize).min(b.len() - at - 4);
                v.push(String::from_utf8_lossy(&b[at + 4..at + 4 + n]).into_owned());
                at += 4 + n;
            }
            Value::Texts(v)
        }
        "box2i" | "v2i" | "v3i" | "rational" => Value::Ints(ints()),
        "box2f" | "v2f" | "v3f" | "m33f" | "m44f" | "chromaticities" => Value::Floats(floats()),
        "v2d" | "v3d" | "m33d" | "m44d" => Value::Doubles(doubles()),
        _ => Value::Bytes(b.to_vec()),
    }
}

fn channels(b: &[u8]) -> Result<Vec<Channel>, String> {
    let mut r = Reader { b, at: 0 };
    let mut out = Vec::new();
    loop {
        let name = r.name()?;
        if name.is_empty() {
            return Ok(out);
        }
        let sample = match r.i32()? {
            0 => Sample::Uint,
            1 => Sample::Half,
            2 => Sample::Float,
            k => return Err(format!("channel {name} keeps samples of an unknown kind ({k})")),
        };
        let linear = r.u8()? != 0;
        r.take(3)?;
        let sampling = [r.i32()?, r.i32()?];
        if sampling[0] < 1 || sampling[1] < 1 {
            return Err(format!("channel {name} is sampled every {} × {} pixels", sampling[0], sampling[1]));
        }
        out.push(Channel { name, sample, linear, sampling });
    }
}

fn window(v: Option<&Value>, what: &str) -> Result<Window, String> {
    match v {
        Some(Value::Ints(v)) if v.len() == 4 => Ok(Window { x0: v[0], y0: v[1], x1: v[2], y1: v[3] }),
        _ => Err(format!("a part with no {what}")),
    }
}

/// A part from its header's attributes (`tiled`, `deep`: the file says so for its only part).
fn part(attributes: Vec<Attribute>, tiled: bool, deep: bool) -> Result<Part, String> {
    let find = |n: &str| attributes.iter().find(|a| a.name == n);
    let channels = match find("channels") {
        Some(a) if a.kind == "chlist" => match &a.value {
            Value::Bytes(b) => channels(b)?,
            _ => Vec::new(),
        },
        _ => return Err("a part with no channels".into()),
    };
    let compression = match find("compression").map(|a| &a.value) {
        Some(Value::Int(c)) => Compression::of(*c as u8),
        _ => return Err("a part that does not say how it is compressed".into()),
    };
    let data = window(find("dataWindow").map(|a| &a.value), "data window")?;
    let display = window(find("displayWindow").map(|a| &a.value), "display window")?;
    let decreasing = matches!(find("lineOrder").map(|a| &a.value), Some(Value::Int(1)));
    let kind = match find("type").map(|a| &a.value) {
        Some(Value::Text(t)) => match t.as_str() {
            "scanlineimage" => Kind::Scanlines,
            "tiledimage" => Kind::Tiles,
            "deepscanline" => Kind::DeepScanlines,
            "deeptile" => Kind::DeepTiles,
            t => return Err(format!("a part of an unknown type ({t})")),
        },
        _ => match (tiled, deep) {
            (false, false) => Kind::Scanlines,
            (true, false) => Kind::Tiles,
            (false, true) => Kind::DeepScanlines,
            (true, true) => Kind::DeepTiles,
        },
    };
    let tiles = match find("tiles").map(|a| &a.value) {
        Some(Value::Bytes(b)) if b.len() == 9 => {
            let (w, h) = (le_i32(b, 0) as u32, le_i32(b, 1) as u32);
            if w == 0 || h == 0 {
                return Err("tiles of no size".into());
            }
            let levels = match b[8] & 0xf {
                0 => Levels::One,
                1 => Levels::Mipmap,
                2 => Levels::Ripmap,
                k => return Err(format!("tiles with an unknown kind of levels ({k})")),
            };
            Some(Tiles { width: w, height: h, levels, round_up: b[8] >> 4 & 0xf == 1 })
        }
        _ if matches!(kind, Kind::Tiles | Kind::DeepTiles) => return Err("a tiled part that does not say its tiles".into()),
        _ => None,
    };
    let pixel_aspect = match find("pixelAspectRatio").map(|a| &a.value) {
        Some(Value::Float(v)) if v.is_finite() && *v > 0.0 => *v,
        _ => 1.0,
    };
    let name = match find("name").map(|a| &a.value) {
        Some(Value::Text(t)) => Some(t.clone()),
        _ => None,
    };
    if data.width() == 0 || data.height() == 0 {
        return Err("a part with no pixels".into());
    }
    Ok(Part { name, kind, channels, compression, data, display, decreasing, tiles, pixel_aspect, attributes, chunks: Vec::new() })
}

/// The levels of a tiled part one way (`n` pixels at the first), as `Levels` and rounding say.
fn level_count(n: usize, t: &Tiles) -> usize {
    let log = |n: usize| {
        let floor = usize::BITS - 1 - n.max(1).leading_zeros();
        (if t.round_up && !n.is_power_of_two() { floor + 1 } else { floor }) as usize
    };
    match t.levels {
        Levels::One => 1,
        _ => log(n) + 1,
    }
}

/// Pixels at level `l` of a side `n` long.
fn level_size(n: usize, l: usize, round_up: bool) -> usize {
    let s = if round_up { (n + (1 << l) - 1) >> l } else { n >> l };
    s.max(1)
}

/// The chunks a part has, when the file does not say.
fn chunk_count(p: &Part) -> Result<usize, String> {
    match p.tiles {
        None => {
            let lines = p.compression.lines().ok_or_else(|| format!("{} compression is not read yet", p.compression.name()))? as usize;
            Ok(p.data.height().div_ceil(lines))
        }
        Some(t) => {
            let (w, h) = (p.data.width(), p.data.height());
            let tiles = |lw: usize, lh: usize| lw.div_ceil(t.width as usize) * lh.div_ceil(t.height as usize);
            let n = match t.levels {
                Levels::One => tiles(w, h),
                Levels::Mipmap => (0..level_count(w.max(h), &t)).map(|l| tiles(level_size(w, l, t.round_up), level_size(h, l, t.round_up))).sum(),
                Levels::Ripmap => {
                    let (nx, ny) = (level_count(w, &t), level_count(h, &t));
                    (0..ny).flat_map(|ly| (0..nx).map(move |lx| (lx, ly))).map(|(lx, ly)| tiles(level_size(w, lx, t.round_up), level_size(h, ly, t.round_up))).sum()
                }
            };
            Ok(n)
        }
    }
}

/// An OpenEXR file's headers and its chunks' places.
pub fn parse(bytes: &[u8]) -> Result<Exr, String> {
    if !looks_like(bytes) {
        return Err("not an OpenEXR file".into());
    }
    let mut r = Reader { b: bytes, at: 4 };
    let version = r.i32()? as u32;
    if version & 0xff != 2 && version & 0xff != 1 {
        return Err(format!("OpenEXR version {} is not read", version & 0xff));
    }
    let (tiled, deep, multipart) = (version & 0x200 != 0, version & 0x800 != 0, version & 0x1000 != 0);
    let mut parts = Vec::new();
    loop {
        let mut attributes = Vec::new();
        loop {
            let name = r.name()?;
            if name.is_empty() {
                break;
            }
            let kind = r.name()?;
            let n = r.i32()?;
            let b = r.take(usize::try_from(n).map_err(|_| format!("attribute {name} has a size below nothing"))?)?;
            attributes.push(Attribute { value: value(&kind, b), name, kind });
        }
        if attributes.is_empty() {
            // the empty header that ends a multi-part file's headers
            if multipart && !parts.is_empty() {
                break;
            }
            return Err("a part with an empty header".into());
        }
        parts.push(part(attributes, tiled && !multipart, deep && !multipart)?);
        if !multipart {
            break;
        }
    }
    // each part's chunk table, then the chunks
    let mut counts = Vec::new();
    for p in &parts {
        let n = match p.get("chunkCount") {
            Some(Value::Int(n)) if *n >= 0 => *n as usize,
            _ if multipart => return Err("a part of a multi-part file that does not say how many chunks it has".into()),
            _ => chunk_count(p)?,
        };
        if n > bytes.len() / 8 {
            return Err("more chunks than the file could hold".into());
        }
        counts.push(n);
    }
    let mut tables = Vec::new();
    for n in &counts {
        tables.push((0..*n).map(|_| r.u64()).collect::<Result<Vec<u64>, String>>()?);
    }
    let first = r.at;
    let fine = tables.iter().flatten().all(|at| *at as usize >= first && (*at as usize) < bytes.len());
    if fine {
        for (p, t) in parts.iter_mut().zip(tables) {
            p.chunks = t.into_iter().map(|at| at as usize).collect();
        }
    } else {
        find_chunks(bytes, first, &mut parts, multipart)?;
    }
    Ok(Exr { version, multipart, parts })
}

/// Each part's chunks found one after another from `at`, for a file whose chunk table is broken.
fn find_chunks(bytes: &[u8], at: usize, parts: &mut [Part], multipart: bool) -> Result<(), String> {
    let mut r = Reader { b: bytes, at };
    while r.at < bytes.len() {
        let start = r.at;
        let k = if multipart { r.i32()? } else { 0 };
        let p = parts.get_mut(usize::try_from(k).map_err(|_| "a chunk of no part")?).ok_or("a chunk of a part there is not")?;
        let head = match p.kind {
            Kind::Scanlines => 4,
            Kind::Tiles => 16,
            _ => return Err("deep pictures (many samples a pixel) are not read yet".into()),
        };
        r.take(head)?;
        let n = r.i32()?;
        if n < 0 || r.take(n as usize).is_err() {
            // cut short here: the chunks before it are read
            break;
        }
        p.chunks.push(start);
    }
    Ok(())
}

/// Number of samples a channel sampled every `s` pixels has from `a` to `b`.
fn samples(s: i32, a: i32, b: i32) -> usize {
    let (a1, b1) = (a.div_euclid(s), b.div_euclid(s));
    (b1 - a1 + if a1 * s < a { 0 } else { 1 }).max(0) as usize
}

/// The first sample at or after `a` of a channel sampled every `s` pixels, counted from 0.
fn ceil_div(a: i32, s: i32) -> i32 {
    -(-a).div_euclid(s)
}

/// A chunk's pixels for each channel, as many as each has there.
struct Grid {
    nx: usize,
    ny: usize,
}

fn grids(channels: &[Channel], w: Window) -> Vec<Grid> {
    channels.iter().map(|c| Grid { nx: samples(c.sampling[0], w.x0, w.x1), ny: samples(c.sampling[1], w.y0, w.y1) }).collect()
}

impl Exr {
    /// The frame the file is seen in: its first part's display window.
    pub fn display(&self) -> Window {
        self.parts.first().map(|p| p.display).unwrap_or_default()
    }

    /// The pixels and bytes of the chunk at `at` of part `k`: none when it is of a smaller level.
    fn chunk<'b>(&self, bytes: &'b [u8], k: usize, at: usize) -> Result<Option<(Window, &'b [u8])>, String> {
        let p = &self.parts[k];
        let mut r = Reader { b: bytes, at };
        if self.multipart && r.i32()? != k as i32 {
            return Err("a chunk table that points at another part's chunk".into());
        }
        let (tw, th) = p.tile_size();
        let d = p.data;
        let w = match p.kind {
            Kind::Scanlines => {
                let y = r.i32()?;
                if y < d.y0 || y > d.y1 || (y as i64 - d.y0 as i64) % th as i64 != 0 {
                    return Err(format!("a chunk of scanlines at {y}, which is not where one starts"));
                }
                Window { x0: d.x0, y0: y, x1: d.x1, y1: (y as i64 + th as i64 - 1).min(d.y1 as i64) as i32 }
            }
            Kind::Tiles => {
                let (tx, ty, lx, ly) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
                if lx != 0 || ly != 0 {
                    let n = r.i32()?;
                    r.take(n.max(0) as usize)?;
                    return Ok(None);
                }
                let (x0, y0) = (d.x0 as i64 + tx as i64 * tw as i64, d.y0 as i64 + ty as i64 * th as i64);
                if tx < 0 || ty < 0 || x0 > d.x1 as i64 || y0 > d.y1 as i64 {
                    return Err(format!("a tile ({tx}, {ty}) outside its part"));
                }
                Window { x0: x0 as i32, y0: y0 as i32, x1: (x0 + tw as i64 - 1).min(d.x1 as i64) as i32, y1: (y0 + th as i64 - 1).min(d.y1 as i64) as i32 }
            }
            _ => return Err("deep pictures (many samples a pixel) are not read yet".into()),
        };
        let n = r.i32()?;
        let data = r.take(usize::try_from(n).map_err(|_| "a chunk of a size below nothing")?)?;
        Ok(Some((w, data)))
    }

    /// Part `k`'s channels at their first level, as the part names them.
    pub fn part_planes(&self, bytes: &[u8], k: usize) -> Result<Vec<Plane>, String> {
        let p = self.parts.get(k).ok_or("no such part")?;
        if matches!(p.kind, Kind::DeepScanlines | Kind::DeepTiles) {
            return Err("deep pictures (many samples a pixel) are not read yet".into());
        }
        if let Compression::Other(_) = p.compression {
            return Err(format!("OpenEXR's {} compression is not read yet: save the file with ZIP, PIZ or DWAA", p.compression.name()));
        }
        let d = p.data;
        if d.width().saturating_mul(d.height()) > MOST {
            return Err(format!("a part {} × {} pixels is too big to read", d.width(), d.height()));
        }
        if p.tiles.is_some() && p.channels.iter().any(|c| c.sampling != [1, 1]) {
            return Err("a tiled part with channels at a lower rate".into());
        }
        let whole = grids(&p.channels, d);
        let mut out: Vec<Vec<f32>> = whole.iter().map(|g| vec![0.0; g.nx * g.ny]).collect();
        let mut chunks = Vec::new();
        for &at in &p.chunks {
            if let Some(c) = self.chunk(bytes, k, at)? {
                chunks.push(c);
            }
        }
        // the chunks unpacked a share a thread, then their samples put in place
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(chunks.len()).max(1);
        let share = chunks.len().div_ceil(threads).max(1);
        let unpacked: Vec<Result<Vec<Vec<u8>>, String>> = std::thread::scope(|s| {
            let jobs: Vec<_> = chunks.chunks(share).map(|some| s.spawn(move || some.iter().map(|(w, data)| unpack(p, *w, data)).collect::<Result<Vec<_>, String>>())).collect();
            jobs.into_iter().map(|j| j.join().unwrap_or_else(|_| Err("a chunk that could not be unpacked".into()))).collect()
        });
        let mut raws = Vec::with_capacity(chunks.len());
        for u in unpacked {
            raws.extend(u?);
        }
        for ((w, _), raw) in chunks.iter().zip(&raws) {
            let here = grids(&p.channels, *w);
            // scanline by scanline, channel by channel
            let mut at = 0;
            for y in w.y0..=w.y1 {
                for (c, ch) in p.channels.iter().enumerate() {
                    let [sx, sy] = ch.sampling;
                    if y.rem_euclid(sy) != 0 {
                        continue;
                    }
                    let n = here[c].nx;
                    let row = (y.div_euclid(sy) - ceil_div(d.y0, sy)) as usize;
                    let col = (ceil_div(w.x0, sx) - ceil_div(d.x0, sx)) as usize;
                    let dst = &mut out[c][row * whole[c].nx + col..][..n];
                    let src = &raw[at..at + n * ch.sample.size()];
                    match ch.sample {
                        Sample::Half => dst.iter_mut().zip(src.as_chunks::<2>().0).for_each(|(o, b)| *o = half::to_f32(u16::from_le_bytes(*b))),
                        Sample::Float => dst.iter_mut().zip(src.as_chunks::<4>().0).for_each(|(o, b)| *o = f32::from_le_bytes(*b)),
                        Sample::Uint => dst.iter_mut().zip(src.as_chunks::<4>().0).for_each(|(o, b)| *o = u32::from_le_bytes(*b) as f32),
                    }
                    at += n * ch.sample.size();
                }
            }
        }
        Ok(p.channels.iter().zip(whole).zip(out).map(|((ch, g), s)| Plane { name: ch.name.clone(), part: k, window: d, samples: widen(s, &g, ch.sampling, d) }).collect())
    }

    /// A channel's name as a layer's: its part's name, a dot, the channel's (unless the channel
    /// already starts so, as Blender writes them); just the channel's in a one-part file.
    fn layer_name(&self, k: usize, channel: &str) -> String {
        match self.parts[k].name.as_deref().filter(|n| self.multipart && !n.is_empty()) {
            Some(p) if !channel.strip_prefix(p).is_some_and(|rest| rest.starts_with('.')) => format!("{p}.{channel}"),
            _ => channel.to_string(),
        }
    }

    /// Every channel of every part, named as layers (see [`Exr::planes`]), with its part.
    pub fn names(&self) -> Vec<(String, usize)> {
        self.parts.iter().enumerate().flat_map(|(k, p)| p.channels.iter().map(move |c| (self.layer_name(k, &c.name), k))).collect()
    }

    /// Part `k`'s channels at their first level, named as layers.
    pub fn named_planes(&self, bytes: &[u8], k: usize) -> Result<Vec<Plane>, String> {
        let mut planes = self.part_planes(bytes, k)?;
        for p in &mut planes {
            p.name = self.layer_name(k, &p.name);
        }
        Ok(planes)
    }

    /// Every part's channels at their first level, named as layers: a part's name, a dot,
    /// the channel's (unless the channel already starts so, as Blender writes them).
    pub fn planes(&self, bytes: &[u8]) -> Result<Vec<Plane>, String> {
        let mut out = Vec::new();
        for k in 0..self.parts.len() {
            out.extend(self.named_planes(bytes, k)?);
        }
        Ok(out)
    }
}

/// A chunk of part `p` over `w` unpacked to its pixels' bytes.
fn unpack(p: &Part, w: Window, data: &[u8]) -> Result<Vec<u8>, String> {
    let size: usize = p.channels.iter().zip(grids(&p.channels, w)).map(|(c, g)| g.nx * g.ny * c.sample.size()).sum();
    if data.len() < size && p.compression != Compression::None {
        return codec::decompress(p.compression, data, &p.channels, w, size);
    }
    Ok(data.get(..size).ok_or("a chunk smaller than its pixels")?.to_vec())
}

/// Samples kept every `sampling` pixels widened to every pixel of `d`.
fn widen(s: Vec<f32>, g: &Grid, sampling: [i32; 2], d: Window) -> Vec<f32> {
    if sampling == [1, 1] || g.nx == 0 || g.ny == 0 {
        return s;
    }
    let at = |v: i32, step: i32, from: i32, n: usize| ((v.div_euclid(step) - ceil_div(from, step)).max(0) as usize).min(n - 1);
    let cols: Vec<usize> = (d.x0..=d.x1).map(|x| at(x, sampling[0], d.x0, g.nx)).collect();
    let mut out = Vec::with_capacity(d.width() * d.height());
    for y in d.y0..=d.y1 {
        let row = at(y, sampling[1], d.y0, g.ny) * g.nx;
        out.extend(cols.iter().map(|c| s[row + c]));
    }
    out
}

/// A file's headers and every part's channels (see [`Exr::planes`]).
pub fn read(bytes: &[u8]) -> Result<(Exr, Vec<Plane>), String> {
    let exr = parse(bytes)?;
    let planes = exr.planes(bytes)?;
    Ok((exr, planes))
}

#[cfg(test)]
mod tests;

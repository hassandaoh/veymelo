//! Type layers (`TySh`): where the text is (a transform), its descriptor, and in that its
//! EngineData — a PostScript-like tree of dictionaries (`<< /Key value >>`), arrays, numbers,
//! flags and strings (UTF-16 after a byte order mark, `\` before a byte kept as it is) holding
//! the text, its runs of character styles and of paragraph styles, the fonts they name, and
//! whether it is point text or set in a box.

use crate::Psd;
use crate::descriptor::{self, Desc};

/// A value of EngineData.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Dict(Vec<(String, Value)>),
    Array(Vec<Value>),
    Number(f64),
    Bool(bool),
    Text(String),
    Name(String),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Dict(d) => d.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The value down a path of keys (`"EngineDict/Editor/Text"`).
    pub fn at(&self, path: &str) -> Option<&Value> {
        path.split('/').try_fold(self, |v, k| v.get(k))
    }

    pub fn num(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            Value::Bool(b) => Some(*b as u8 as f64),
            _ => None,
        }
    }

    pub fn flag(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            Value::Number(n) => Some(*n != 0.0),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            Value::Text(t) | Value::Name(t) => Some(t),
            _ => None,
        }
    }

    pub fn array(&self) -> &[Value] {
        match self {
            Value::Array(a) => a,
            _ => &[],
        }
    }
}

/// Reads EngineData.
struct Reader<'a> {
    d: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn skip(&mut self) {
        while self.at < self.d.len() && matches!(self.d[self.at], b' ' | b'\t' | b'\n' | b'\r' | 0) {
            self.at += 1;
        }
    }

    fn word(&mut self) -> &[u8] {
        let start = self.at;
        while self.at < self.d.len() && !matches!(self.d[self.at], b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'[' | b']' | b'<' | b'>' | b'(' | b')') {
            self.at += 1;
        }
        &self.d[start..self.at]
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > 128 {
            return Err("text settings nested too deep".into());
        }
        self.skip();
        let rest = &self.d[self.at.min(self.d.len())..];
        if rest.starts_with(b"<<") {
            self.at += 2;
            let mut items = Vec::new();
            loop {
                self.skip();
                if self.d[self.at.min(self.d.len())..].starts_with(b">>") {
                    self.at += 2;
                    return Ok(Value::Dict(items));
                }
                if self.at >= self.d.len() {
                    return Err("text settings cut short".into());
                }
                if self.d[self.at] != b'/' {
                    return Err("a key in the text settings without its slash".into());
                }
                self.at += 1;
                let key = String::from_utf8_lossy(self.word()).into_owned();
                let v = self.value(depth + 1)?;
                items.push((key, v));
            }
        }
        match rest.first() {
            None => Err("text settings cut short".into()),
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    self.skip();
                    match self.d.get(self.at) {
                        Some(b']') => {
                            self.at += 1;
                            return Ok(Value::Array(items));
                        }
                        None => return Err("text settings cut short".into()),
                        _ => items.push(self.value(depth + 1)?),
                    }
                }
            }
            Some(b'(') => {
                self.at += 1;
                let mut bytes = Vec::new();
                loop {
                    match self.d.get(self.at) {
                        None => return Err("a string in the text settings cut short".into()),
                        Some(b'\\') => {
                            if let Some(&b) = self.d.get(self.at + 1) {
                                bytes.push(b);
                            }
                            self.at += 2;
                        }
                        Some(b')') => {
                            self.at += 1;
                            break;
                        }
                        Some(&b) => {
                            bytes.push(b);
                            self.at += 1;
                        }
                    }
                }
                Ok(Value::Text(string_of(&bytes)))
            }
            Some(b'/') => {
                self.at += 1;
                Ok(Value::Name(String::from_utf8_lossy(self.word()).into_owned()))
            }
            Some(_) => {
                let w = self.word();
                if w.is_empty() {
                    return Err(format!("text settings with a stray {:?}", self.d[self.at] as char));
                }
                let s = String::from_utf8_lossy(w);
                match &*s {
                    "true" => Ok(Value::Bool(true)),
                    "false" => Ok(Value::Bool(false)),
                    _ => s.parse::<f64>().map(Value::Number).map_err(|_| format!("a value in the text settings not read ({s})")),
                }
            }
        }
    }
}

/// A string's bytes as text: UTF-16 after its byte order mark, otherwise one character a byte.
fn string_of(b: &[u8]) -> String {
    if b.starts_with(&[0xfe, 0xff]) {
        let units: Vec<u16> = b[2..].chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
        String::from_utf16_lossy(&units)
    } else {
        b.iter().map(|&c| c as char).collect()
    }
}

/// EngineData's tree.
pub fn engine_data(d: &[u8]) -> Result<Value, String> {
    let mut r = Reader { d, at: 0 };
    r.value(0)
}

/// A run of characters' look.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    /// The font's PostScript name.
    pub font: Option<String>,
    /// Its size (in the text's own units: the transform scales it).
    pub size: f64,
    /// Fill and stroke colours (straight sRGB and alpha, 0 to 1), where they are on.
    pub fill: Option<[f64; 4]>,
    pub stroke: Option<[f64; 4]>,
    pub stroke_width: f64,
    /// Thousandths of the size.
    pub tracking: f64,
    /// Baseline to baseline (None: automatic).
    pub leading: Option<f64>,
    /// None: the font's own kerning; a number, thousandths of the size.
    pub kerning: Option<f64>,
    pub baseline_shift: f64,
    /// 0 as written, 1 small capitals, 2 all capitals.
    pub caps: u8,
    /// 0 on the baseline, 1 superscript, 2 subscript.
    pub position: u8,
    pub underline: bool,
    pub strikethrough: bool,
    /// Percent.
    pub h_scale: f64,
    pub v_scale: f64,
    pub faux_bold: bool,
    pub faux_italic: bool,
    pub ligatures: bool,
}

/// A run of paragraphs' look.
#[derive(Clone, Debug, PartialEq)]
pub struct Paragraph {
    /// 0 left, 1 right, 2 centre, 3 to 5 justified with the last line left, right, centred, 6
    /// all justified.
    pub justification: u8,
    pub first_indent: f64,
    pub start_indent: f64,
    pub end_indent: f64,
    pub space_before: f64,
    pub space_after: f64,
}

/// A type layer.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeLayer {
    /// Where its own space goes in the document: x' = a·x + c·y + e, y' = b·x + d·y + f.
    pub transform: [f64; 6],
    /// Its text, paragraphs ended by `\n`.
    pub text: String,
    pub vertical: bool,
    /// The box it is set in (left, top, right, bottom in its own space), when it is not point
    /// text.
    pub boxed: Option<[f64; 4]>,
    /// Runs of characters (each as many characters as it says, in order) and their looks.
    pub runs: Vec<(usize, Style)>,
    pub paragraphs: Vec<(usize, Paragraph)>,
    /// Its descriptor (for what is not read here).
    pub desc: Desc,
}

/// A style sheet's look, over a default.
fn style(s: Option<&Value>, base: &Style, fonts: &[String]) -> Style {
    let Some(s) = s else { return base.clone() };
    let num = |k: &str| s.get(k).and_then(Value::num);
    let flag = |k: &str| s.get(k).and_then(Value::flag);
    let color = |k: &str| {
        let v = s.get(k)?.get("Values")?.array();
        let n: Vec<f64> = v.iter().filter_map(Value::num).collect();
        // alpha first, then red, green and blue
        (n.len() >= 4).then(|| [n[1], n[2], n[3], n[0]].map(|c| c.clamp(0.0, 1.0)))
    };
    let mut o = base.clone();
    if let Some(f) = num("Font") {
        o.font = fonts.get(f as usize).cloned().or(o.font);
    }
    o.size = num("FontSize").unwrap_or(o.size);
    let (fill_on, stroke_on) = (flag("FillFlag"), flag("StrokeFlag"));
    if let Some(c) = color("FillColor") {
        o.fill = Some(c);
    }
    if fill_on == Some(false) {
        o.fill = None;
    }
    if let Some(c) = color("StrokeColor") {
        o.stroke = Some(c);
    }
    match stroke_on {
        Some(false) => o.stroke = None,
        Some(true) if o.stroke.is_none() => o.stroke = Some([0.0, 0.0, 0.0, 1.0]),
        _ => {}
    }
    o.stroke_width = num("OutlineWidth").unwrap_or(o.stroke_width);
    o.tracking = num("Tracking").unwrap_or(o.tracking);
    match (flag("AutoLeading"), num("Leading")) {
        (Some(true), _) => o.leading = None,
        (Some(false), Some(l)) => o.leading = Some(l),
        _ => {}
    }
    match flag("AutoKerning") {
        Some(true) => o.kerning = None,
        Some(false) => o.kerning = Some(num("Kerning").unwrap_or(0.0)),
        None => {}
    }
    o.baseline_shift = num("BaselineShift").unwrap_or(o.baseline_shift);
    o.caps = num("FontCaps").map_or(o.caps, |v| v as u8);
    o.position = num("FontBaseline").map_or(o.position, |v| v as u8);
    o.underline = flag("Underline").unwrap_or(o.underline);
    o.strikethrough = flag("Strikethrough").unwrap_or(o.strikethrough);
    o.h_scale = num("HorizontalScale").map_or(o.h_scale, |v| v * 100.0);
    o.v_scale = num("VerticalScale").map_or(o.v_scale, |v| v * 100.0);
    o.faux_bold = flag("FauxBold").unwrap_or(o.faux_bold);
    o.faux_italic = flag("FauxItalic").unwrap_or(o.faux_italic);
    o.ligatures = flag("Ligatures").unwrap_or(o.ligatures);
    o
}

fn paragraph(p: Option<&Value>, base: &Paragraph) -> Paragraph {
    let Some(p) = p else { return base.clone() };
    let num = |k: &str, v: f64| p.get(k).and_then(Value::num).unwrap_or(v);
    Paragraph {
        justification: num("Justification", base.justification as f64) as u8,
        first_indent: num("FirstLineIndent", base.first_indent),
        start_indent: num("StartIndent", base.start_indent),
        end_indent: num("EndIndent", base.end_indent),
        space_before: num("SpaceBefore", base.space_before),
        space_after: num("SpaceAfter", base.space_after),
    }
}

/// Runs' lengths in UTF-16 units of `units` as lengths in characters, the last run taking what
/// is left.
fn char_runs(units: &[u16], lengths: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(lengths.len());
    let mut at = 0usize;
    for &n in lengths {
        let end = (at + n).min(units.len());
        out.push(char::decode_utf16(units[at..end].iter().copied()).count());
        at = end;
    }
    out
}

/// A type layer from its `TySh` block.
pub fn type_layer(d: &[u8]) -> Result<TypeLayer, String> {
    if d.len() < 56 {
        return Err("a type layer's settings cut short".into());
    }
    let mut transform = [0.0; 6];
    for (k, t) in transform.iter_mut().enumerate() {
        *t = f64::from_be_bytes(d[2 + 8 * k..10 + 8 * k].try_into().expect("eight bytes"));
    }
    // the text's version (2 bytes), then its descriptor's
    let desc = descriptor::read(&d[52..], true)?;
    let tree = desc.data("EngineData").map(engine_data).transpose()?;
    let raw = tree.as_ref().and_then(|t| t.at("EngineDict/Editor/Text")).and_then(Value::text).map(str::to_string).or_else(|| desc.text("Txt ").map(str::to_string)).unwrap_or_default();
    let units: Vec<u16> = raw.encode_utf16().collect();
    let empty = Value::Dict(Vec::new());
    let t = tree.as_ref().unwrap_or(&empty);
    let res = t.get("ResourceDict").or(t.get("DocumentResources")).unwrap_or(&empty);
    let fonts: Vec<String> = res.get("FontSet").map_or(&[][..], Value::array).iter().map(|f| f.get("Name").and_then(Value::text).unwrap_or("").to_string()).collect();
    // the document's normal style and paragraph, over Photoshop's own defaults
    let mut base = Style {
        font: None,
        size: 12.0,
        fill: Some([0.0, 0.0, 0.0, 1.0]),
        stroke: None,
        stroke_width: 1.0,
        tracking: 0.0,
        leading: None,
        kerning: None,
        baseline_shift: 0.0,
        caps: 0,
        position: 0,
        underline: false,
        strikethrough: false,
        h_scale: 100.0,
        v_scale: 100.0,
        faux_bold: false,
        faux_italic: false,
        ligatures: true,
    };
    let normal = res.get("TheNormalStyleSheet").and_then(Value::num).unwrap_or(0.0) as usize;
    base = style(res.get("StyleSheetSet").and_then(|s| s.array().get(normal)).and_then(|s| s.get("StyleSheetData")), &base, &fonts);
    base = style(t.at("EngineDict/StyleRun/DefaultRunData/StyleSheet/StyleSheetData"), &base, &fonts);
    let normal_p = res.get("TheNormalParagraphSheet").and_then(Value::num).unwrap_or(0.0) as usize;
    let pbase = Paragraph { justification: 0, first_indent: 0.0, start_indent: 0.0, end_indent: 0.0, space_before: 0.0, space_after: 0.0 };
    let pbase = paragraph(res.get("ParagraphSheetSet").and_then(|s| s.array().get(normal_p)).and_then(|s| s.get("Properties")), &pbase);
    let lengths = |path: &str| -> Vec<usize> { t.at(path).map_or(&[][..], Value::array).iter().filter_map(Value::num).map(|n| n.max(0.0) as usize).collect() };
    let style_runs: Vec<Style> = t.at("EngineDict/StyleRun/RunArray").map_or(&[][..], Value::array).iter().map(|r| style(r.at("StyleSheet/StyleSheetData"), &base, &fonts)).collect();
    let para_runs: Vec<Paragraph> = t.at("EngineDict/ParagraphRun/RunArray").map_or(&[][..], Value::array).iter().map(|r| paragraph(r.at("ParagraphSheet/Properties"), &pbase)).collect();
    let runs = char_runs(&units, &lengths("EngineDict/StyleRun/RunLengthArray")).into_iter().zip(style_runs).collect::<Vec<_>>();
    let paragraphs = char_runs(&units, &lengths("EngineDict/ParagraphRun/RunLengthArray")).into_iter().zip(para_runs).collect::<Vec<_>>();
    let runs = if runs.is_empty() { vec![(raw.chars().count(), base)] } else { runs };
    let paragraphs = if paragraphs.is_empty() { vec![(raw.chars().count(), pbase)] } else { paragraphs };
    // point text, or set in a box
    let shape = t.at("EngineDict/Rendered/Shapes/Children").and_then(|c| c.array().first());
    let boxed = shape.filter(|s| s.get("ShapeType").and_then(Value::num) == Some(1.0)).and_then(|s| s.at("Cookie/Photoshop/BoxBounds")).map(|b| {
        let n: Vec<f64> = b.array().iter().filter_map(Value::num).collect();
        [n.first().copied().unwrap_or(0.0), n.get(1).copied().unwrap_or(0.0), n.get(2).copied().unwrap_or(0.0), n.get(3).copied().unwrap_or(0.0)]
    });
    let vertical = desc.enumeration("Ornt") == Some("Vrtc") || t.at("EngineDict/Rendered/Shapes/WritingDirection").and_then(Value::num) == Some(2.0);
    // paragraphs end in a carriage return, the last one too
    let text = raw.replace('\r', "\n");
    Ok(TypeLayer { transform, text, vertical, boxed, runs, paragraphs, desc })
}

impl Psd {
    /// Layer `i`'s text, where it is a type layer.
    pub fn type_layer(&self, i: usize) -> Option<Result<TypeLayer, String>> {
        self.block(i, b"TySh").map(type_layer)
    }
}

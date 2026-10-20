//! A text layer's document: its words and how they look. Written in a file as just the words
//! when the rest is as it starts, else as an object with what differs.

use veymelo_color::Rgba;
use veymelo_json::{Map, Value as Json};

use crate::describe;

/// How lines sit in their width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Justify {
    #[default]
    Left,
    Center,
    Right,
    /// Stretched to the box's width, the last line to the left, centre or right, or stretched too.
    JustifyLeft,
    JustifyCenter,
    JustifyRight,
    JustifyAll,
}

impl Justify {
    pub const IDS: &'static [&'static str] = &["left", "center", "right", "justifyLeft", "justifyCenter", "justifyRight", "justifyAll"];
    const ALL: [Justify; 7] = [Justify::Left, Justify::Center, Justify::Right, Justify::JustifyLeft, Justify::JustifyCenter, Justify::JustifyRight, Justify::JustifyAll];

    pub fn id(self) -> &'static str {
        Justify::IDS[Justify::ALL.iter().position(|j| *j == self).unwrap_or(0)]
    }

    pub fn from_id(id: &str) -> Option<Justify> {
        Justify::IDS.iter().position(|i| *i == id).map(|k| Justify::ALL[k])
    }
}

/// Capitals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Caps {
    #[default]
    Normal,
    AllCaps,
    /// Lowercase letters as smaller capitals.
    SmallCaps,
}

impl Caps {
    pub const IDS: &'static [&'static str] = &["normal", "allCaps", "smallCaps"];

    pub fn id(self) -> &'static str {
        match self {
            Caps::Normal => "normal",
            Caps::AllCaps => "allCaps",
            Caps::SmallCaps => "smallCaps",
        }
    }

    pub fn from_id(id: &str) -> Option<Caps> {
        match id {
            "normal" => Some(Caps::Normal),
            "allCaps" => Some(Caps::AllCaps),
            "smallCaps" => Some(Caps::SmallCaps),
            _ => None,
        }
    }
}

/// Letters raised or lowered as a small figure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Position {
    #[default]
    Normal,
    Superscript,
    Subscript,
}

impl Position {
    pub const IDS: &'static [&'static str] = &["normal", "superscript", "subscript"];
    pub fn id(self) -> &'static str {
        Self::IDS[self as usize]
    }
    pub fn from_id(id: &str) -> Option<Position> {
        Some(match id {
            "normal" => Position::Normal,
            "superscript" => Position::Superscript,
            "subscript" => Position::Subscript,
            _ => return None,
        })
    }
}

/// How the room between two letters is set: as the font says, by their shapes, or by hand (in
/// thousandths of the size, before the letter).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Kerning {
    #[default]
    Metrics,
    Optical,
    Manual(f64),
}

impl Kerning {
    fn to_json(self) -> Json {
        match self {
            Kerning::Metrics => Json::from("metrics"),
            Kerning::Optical => Json::from("optical"),
            Kerning::Manual(v) => Json::Number(v),
        }
    }
    fn from_json(j: &Json) -> Option<Kerning> {
        match j {
            Json::Number(v) if v.is_finite() => Some(Kerning::Manual(*v)),
            Json::String(s) if s == "metrics" => Some(Kerning::Metrics),
            Json::String(s) if s == "optical" => Some(Kerning::Optical),
            _ => None,
        }
    }
}

/// Which way a paragraph goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    /// As its first letter that has a direction goes.
    #[default]
    Auto,
    Ltr,
    Rtl,
}

impl Direction {
    pub const IDS: &'static [&'static str] = &["auto", "ltr", "rtl"];
    pub fn id(self) -> &'static str {
        Self::IDS[self as usize]
    }
    pub fn from_id(id: &str) -> Option<Direction> {
        Some(match id {
            "auto" => Direction::Auto,
            "ltr" => Direction::Ltr,
            "rtl" => Direction::Rtl,
            _ => return None,
        })
    }
}

/// How a paragraph's lines are broken: looking at the whole paragraph for the evenest lines,
/// or each line as full as it goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Composer {
    #[default]
    EveryLine,
    SingleLine,
}

impl Composer {
    pub const IDS: &'static [&'static str] = &["everyLine", "singleLine"];
    pub fn id(self) -> &'static str {
        Self::IDS[self as usize]
    }
    pub fn from_id(id: &str) -> Option<Composer> {
        Some(match id {
            "everyLine" => Composer::EveryLine,
            "singleLine" => Composer::SingleLine,
            _ => return None,
        })
    }
}

/// How a character looks.
#[derive(Clone, Debug, PartialEq)]
pub struct CharStyle {
    /// A family ("Georgia"), a full or PostScript name ("Tahoma-Bold"), or a font file's path.
    pub font: String,
    /// 100 (thin) to 900 (black): a variable font takes it exactly, others the nearest they have.
    pub weight: f64,
    pub italic: bool,
    /// The font size, in pixels.
    pub size: f64,
    /// Fill and stroke colours (none: not filled, not stroked).
    pub fill: Option<Rgba>,
    pub stroke: Option<Rgba>,
    pub stroke_width: f64,
    /// Room added after the letter, in thousandths of the size.
    pub tracking: f64,
    /// From one baseline to the next, in pixels (none: 1.2 times the size).
    pub leading: Option<f64>,
    /// Raised (above zero) or lowered from the baseline, in pixels.
    pub baseline_shift: f64,
    pub caps: Caps,
    /// Stretched across and up, in percent.
    pub h_scale: f64,
    pub v_scale: f64,
    /// The room either side of the letter taken in, in percent (for CJK).
    pub tsume: f64,
    /// Made heavier or slanted by drawing (for a font without the weight or slant).
    pub faux_bold: bool,
    pub faux_italic: bool,
    pub position: Position,
    pub kerning: Kerning,
    pub ligatures: bool,
    /// OpenType features asked for (`onum`, `smcp`, `ss01`…).
    pub features: Vec<String>,
    /// A variable font's axes set (beyond weight): tag and value.
    pub axes: Vec<(String, f64)>,
    pub underline: bool,
    pub strikethrough: bool,
}

impl Default for CharStyle {
    fn default() -> CharStyle {
        CharStyle {
            font: "System Font".into(),
            weight: 400.0,
            italic: false,
            size: 72.0,
            fill: Some(Rgba::WHITE),
            stroke: None,
            stroke_width: 2.0,
            tracking: 0.0,
            leading: None,
            baseline_shift: 0.0,
            caps: Caps::Normal,
            h_scale: 100.0,
            v_scale: 100.0,
            tsume: 0.0,
            faux_bold: false,
            faux_italic: false,
            position: Position::Normal,
            kerning: Kerning::Metrics,
            ligatures: true,
            features: Vec::new(),
            axes: Vec::new(),
            underline: false,
            strikethrough: false,
        }
    }
}

impl CharStyle {
    /// The baseline-to-baseline distance it asks for.
    pub fn line_height(&self) -> f64 {
        self.leading.unwrap_or(self.size * 1.2)
    }

    /// The look fields a document's JSON may hold.
    pub const FIELDS: &'static [&'static str] = &CHAR_FIELDS;

    /// Set what `o` gives of the look fields (others in it are not looked at).
    pub fn apply_json(&mut self, o: &Map) -> Result<(), String> {
        read_char(o, self)
    }
}

/// How a paragraph is set.
#[derive(Clone, Debug, PartialEq)]
pub struct ParaStyle {
    pub justify: Justify,
    /// Room kept at the left and right, and the first line's extra indent, in pixels.
    pub indent_left: f64,
    pub indent_right: f64,
    pub indent_first: f64,
    /// Room before and after the paragraph, in pixels.
    pub space_before: f64,
    pub space_after: f64,
    pub direction: Direction,
    pub composer: Composer,
    /// Punctuation at the ends of lines hangs past the edges.
    pub hanging: bool,
}

impl ParaStyle {
    /// The paragraph fields a document's JSON may hold.
    pub const FIELDS: &'static [&'static str] = &PARA_FIELDS;

    /// Set what `o` gives of the paragraph fields.
    pub fn apply_json(&mut self, o: &Map) -> Result<(), String> {
        read_para(o, self)
    }
}

impl Default for ParaStyle {
    fn default() -> ParaStyle {
        ParaStyle { justify: Justify::Left, indent_left: 0.0, indent_right: 0.0, indent_first: 0.0, space_before: 0.0, space_after: 0.0, direction: Direction::Auto, composer: Composer::EveryLine, hanging: false }
    }
}

/// A stretch of characters that look the same.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// How many characters it covers.
    pub len: usize,
    pub style: CharStyle,
}

/// A text layer's words and their look: one look for all of it (the fields here), or a look for
/// each stretch of characters (`runs`); one setting for every paragraph, or each its own
/// (`paragraphs`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextDoc {
    pub text: String,
    /// A family ("Georgia"), a full or PostScript name ("Tahoma-Bold"), or a font file's path.
    pub font: String,
    /// 100 (thin) to 900 (black): a variable font takes it exactly, others the nearest they have.
    pub weight: f64,
    pub italic: bool,
    /// The font size, in pixels.
    pub size: f64,
    /// Fill and stroke colours (none: not filled, not stroked).
    pub fill: Option<Rgba>,
    pub stroke: Option<Rgba>,
    pub stroke_width: f64,
    /// The stroke drawn over the fill (else under it).
    pub stroke_over: bool,
    /// Room added between letters, in thousandths of the size.
    pub tracking: f64,
    /// From one baseline to the next, in pixels (none: 1.2 times the size).
    pub leading: Option<f64>,
    pub justify: Justify,
    /// Paragraph text: lines wrap in a box this size, its top left at the layer's origin. None:
    /// point text, lines only where the text breaks them, the first baseline at the origin.
    pub box_size: Option<[f64; 2]>,
    /// Letters raised (above zero) or lowered from the baseline, in pixels.
    pub baseline_shift: f64,
    pub caps: Caps,
    pub h_scale: f64,
    pub v_scale: f64,
    pub tsume: f64,
    pub faux_bold: bool,
    pub faux_italic: bool,
    pub position: Position,
    pub kerning: Kerning,
    pub ligatures: bool,
    pub features: Vec<String>,
    pub axes: Vec<(String, f64)>,
    pub underline: bool,
    pub strikethrough: bool,
    pub indent_left: f64,
    pub indent_right: f64,
    pub indent_first: f64,
    pub space_before: f64,
    pub space_after: f64,
    pub direction: Direction,
    pub composer: Composer,
    pub hanging: bool,
    /// Written in columns, top to bottom, the columns right to left.
    pub vertical: bool,
    /// Each stretch's look, covering the words (none: the look above for all of it).
    pub runs: Vec<Run>,
    /// Each paragraph's setting (none: the one above for all of them).
    pub paragraphs: Vec<ParaStyle>,
}

impl Default for TextDoc {
    fn default() -> TextDoc {
        let mut d = TextDoc {
            text: String::new(),
            font: String::new(),
            weight: 0.0,
            italic: false,
            size: 0.0,
            fill: None,
            stroke: None,
            stroke_width: 0.0,
            stroke_over: false,
            tracking: 0.0,
            leading: None,
            justify: Justify::Left,
            box_size: None,
            baseline_shift: 0.0,
            caps: Caps::Normal,
            h_scale: 0.0,
            v_scale: 0.0,
            tsume: 0.0,
            faux_bold: false,
            faux_italic: false,
            position: Position::Normal,
            kerning: Kerning::Metrics,
            ligatures: true,
            features: Vec::new(),
            axes: Vec::new(),
            underline: false,
            strikethrough: false,
            indent_left: 0.0,
            indent_right: 0.0,
            indent_first: 0.0,
            space_before: 0.0,
            space_after: 0.0,
            direction: Direction::Auto,
            composer: Composer::EveryLine,
            hanging: false,
            vertical: false,
            runs: Vec::new(),
            paragraphs: Vec::new(),
        };
        d.set_base_style(&CharStyle::default());
        d.set_base_para(&ParaStyle::default());
        d
    }
}

/// The look a document writes, before the paragraph and layout fields.
const CHAR_FIELDS: [&str; 23] = [
    "font", "weight", "italic", "size", "fill", "stroke", "strokeWidth", "tracking", "leading", "baselineShift", "caps", "hScale", "vScale", "tsume", "fauxBold", "fauxItalic", "position", "kerning", "ligatures", "features", "axes", "underline",
    "strikethrough",
];
const PARA_FIELDS: [&str; 9] = ["justify", "indentLeft", "indentRight", "indentFirst", "spaceBefore", "spaceAfter", "direction", "composer", "hanging"];
const DOC_FIELDS: [&str; 6] = ["text", "strokeOver", "box", "vertical", "runs", "paragraphs"];

fn color(c: Option<Rgba>) -> Json {
    c.map_or(Json::Null, |c| crate::Value::Color(c).to_json(crate::Kind::Color))
}

/// What of `s` differs from `base`, into `o` (in the usual order).
fn char_json(s: &CharStyle, base: &CharStyle, o: &mut Map) {
    if s.font != base.font {
        o.insert("font", s.font.as_str());
    }
    if s.weight != base.weight {
        o.insert("weight", s.weight);
    }
    if s.italic != base.italic {
        o.insert("italic", s.italic);
    }
    if s.size != base.size {
        o.insert("size", s.size);
    }
    if s.fill != base.fill {
        o.insert("fill", color(s.fill));
    }
    if s.stroke != base.stroke {
        o.insert("stroke", color(s.stroke));
    }
    if s.stroke_width != base.stroke_width {
        o.insert("strokeWidth", s.stroke_width);
    }
    if s.tracking != base.tracking {
        o.insert("tracking", s.tracking);
    }
    if s.leading != base.leading {
        o.insert("leading", s.leading.map_or(Json::from("auto"), Json::Number));
    }
    if s.baseline_shift != base.baseline_shift {
        o.insert("baselineShift", s.baseline_shift);
    }
    if s.caps != base.caps {
        o.insert("caps", s.caps.id());
    }
    if s.h_scale != base.h_scale {
        o.insert("hScale", s.h_scale);
    }
    if s.v_scale != base.v_scale {
        o.insert("vScale", s.v_scale);
    }
    if s.tsume != base.tsume {
        o.insert("tsume", s.tsume);
    }
    if s.faux_bold != base.faux_bold {
        o.insert("fauxBold", s.faux_bold);
    }
    if s.faux_italic != base.faux_italic {
        o.insert("fauxItalic", s.faux_italic);
    }
    if s.position != base.position {
        o.insert("position", s.position.id());
    }
    if s.kerning != base.kerning {
        o.insert("kerning", s.kerning.to_json());
    }
    if s.ligatures != base.ligatures {
        o.insert("ligatures", s.ligatures);
    }
    if s.features != base.features {
        o.insert("features", Json::array(s.features.iter().map(|f| Json::from(f.as_str()))));
    }
    if s.axes != base.axes {
        o.insert("axes", Json::Object(s.axes.iter().map(|(t, v)| (t.clone(), Json::Number(*v))).collect()));
    }
    if s.underline != base.underline {
        o.insert("underline", s.underline);
    }
    if s.strikethrough != base.strikethrough {
        o.insert("strikethrough", s.strikethrough);
    }
}

fn para_json(p: &ParaStyle, base: &ParaStyle, o: &mut Map) {
    if p.justify != base.justify {
        o.insert("justify", p.justify.id());
    }
    for (key, v, d) in [("indentLeft", p.indent_left, base.indent_left), ("indentRight", p.indent_right, base.indent_right), ("indentFirst", p.indent_first, base.indent_first), ("spaceBefore", p.space_before, base.space_before), ("spaceAfter", p.space_after, base.space_after)] {
        if v != d {
            o.insert(key, v);
        }
    }
    if p.direction != base.direction {
        o.insert("direction", p.direction.id());
    }
    if p.composer != base.composer {
        o.insert("composer", p.composer.id());
    }
    if p.hanging != base.hanging {
        o.insert("hanging", p.hanging);
    }
}

/// Read `o`'s look fields onto `s`.
fn read_char(o: &Map, s: &mut CharStyle) -> Result<(), String> {
    let num = |key: &str, lo: f64, hi: f64| -> Result<Option<f64>, String> {
        match o.get(key) {
            None => Ok(None),
            Some(v) => match v.as_f64().filter(|x| x.is_finite() && (lo..=hi).contains(x)) {
                Some(x) => Ok(Some(x)),
                None => Err(format!("\"{key}\" is a number from {lo} to {hi}, got {}", describe(v))),
            },
        }
    };
    let flag = |key: &str| -> Result<Option<bool>, String> {
        match o.get(key) {
            None => Ok(None),
            Some(v) => v.as_bool().map(Some).ok_or_else(|| format!("\"{key}\" is true or false, got {}", describe(v))),
        }
    };
    let colour = |key: &str| -> Result<Option<Option<Rgba>>, String> {
        match o.get(key) {
            None => Ok(None),
            Some(Json::Null) => Ok(Some(None)),
            Some(v) => crate::Value::from_json(crate::Kind::Color, v).map(|c| Some(Some(c.as_color()))).map_err(|e| format!("\"{key}\": {e} (or null for none)")),
        }
    };
    if let Some(v) = o.get("font") {
        s.font = v.as_str().filter(|x| !x.is_empty()).ok_or_else(|| format!("\"font\" is a font's name or file, got {}", describe(v)))?.into();
    }
    s.weight = num("weight", 1.0, 1000.0)?.unwrap_or(s.weight);
    s.italic = flag("italic")?.unwrap_or(s.italic);
    s.size = num("size", 0.0, 10_000.0)?.unwrap_or(s.size);
    if let Some(c) = colour("fill")? {
        s.fill = c;
    }
    if let Some(c) = colour("stroke")? {
        s.stroke = c;
    }
    s.stroke_width = num("strokeWidth", 0.0, 10_000.0)?.unwrap_or(s.stroke_width);
    s.tracking = num("tracking", -10_000.0, 10_000.0)?.unwrap_or(s.tracking);
    match o.get("leading") {
        None => {}
        Some(v) if v.as_str() == Some("auto") || v.is_null() => s.leading = None,
        Some(_) => s.leading = num("leading", 0.0, 100_000.0)?,
    }
    s.baseline_shift = num("baselineShift", -100_000.0, 100_000.0)?.unwrap_or(s.baseline_shift);
    if let Some(v) = o.get("caps") {
        s.caps = v.as_str().and_then(Caps::from_id).ok_or_else(|| format!("\"caps\" is one of {}", Caps::IDS.join(", ")))?;
    }
    s.h_scale = num("hScale", 1.0, 10_000.0)?.unwrap_or(s.h_scale);
    s.v_scale = num("vScale", 1.0, 10_000.0)?.unwrap_or(s.v_scale);
    s.tsume = num("tsume", 0.0, 100.0)?.unwrap_or(s.tsume);
    s.faux_bold = flag("fauxBold")?.unwrap_or(s.faux_bold);
    s.faux_italic = flag("fauxItalic")?.unwrap_or(s.faux_italic);
    if let Some(v) = o.get("position") {
        s.position = v.as_str().and_then(Position::from_id).ok_or_else(|| format!("\"position\" is one of {}", Position::IDS.join(", ")))?;
    }
    if let Some(v) = o.get("kerning") {
        s.kerning = Kerning::from_json(v).ok_or("\"kerning\" is \"metrics\", \"optical\" or a number (thousandths of the size)")?;
    }
    s.ligatures = flag("ligatures")?.unwrap_or(s.ligatures);
    if let Some(v) = o.get("features") {
        let list = v.as_array().ok_or("\"features\" is a list of OpenType feature tags, like [\"onum\", \"ss01\"]")?;
        s.features = list
            .iter()
            .map(|f| f.as_str().filter(|t| t.len() == 4 && t.is_ascii()).map(str::to_string).ok_or_else(|| format!("a feature is a four-letter tag, got {}", describe(f))))
            .collect::<Result<_, _>>()?;
    }
    if let Some(v) = o.get("axes") {
        let m = v.as_object().ok_or("\"axes\" is an object of axis values, like {\"wdth\": 80}")?;
        s.axes = m
            .iter()
            .map(|(t, x)| x.as_f64().filter(|x| x.is_finite() && t.len() == 4).map(|x| (t.to_string(), x)).ok_or_else(|| format!("an axis is a four-letter tag and a number, got {t}")))
            .collect::<Result<_, _>>()?;
    }
    s.underline = flag("underline")?.unwrap_or(s.underline);
    s.strikethrough = flag("strikethrough")?.unwrap_or(s.strikethrough);
    Ok(())
}

fn read_para(o: &Map, p: &mut ParaStyle) -> Result<(), String> {
    let num = |key: &str| -> Result<Option<f64>, String> {
        match o.get(key) {
            None => Ok(None),
            Some(v) => v.as_f64().filter(|x| x.is_finite() && x.abs() <= 100_000.0).map(Some).ok_or_else(|| format!("\"{key}\" is a number of pixels, got {}", describe(v))),
        }
    };
    if let Some(v) = o.get("justify") {
        p.justify = v.as_str().and_then(Justify::from_id).ok_or_else(|| format!("\"justify\" is one of {}", Justify::IDS.join(", ")))?;
    }
    p.indent_left = num("indentLeft")?.unwrap_or(p.indent_left);
    p.indent_right = num("indentRight")?.unwrap_or(p.indent_right);
    p.indent_first = num("indentFirst")?.unwrap_or(p.indent_first);
    p.space_before = num("spaceBefore")?.unwrap_or(p.space_before);
    p.space_after = num("spaceAfter")?.unwrap_or(p.space_after);
    if let Some(v) = o.get("direction") {
        p.direction = v.as_str().and_then(Direction::from_id).ok_or_else(|| format!("\"direction\" is one of {}", Direction::IDS.join(", ")))?;
    }
    if let Some(v) = o.get("composer") {
        p.composer = v.as_str().and_then(Composer::from_id).ok_or_else(|| format!("\"composer\" is one of {}", Composer::IDS.join(", ")))?;
    }
    if let Some(v) = o.get("hanging") {
        p.hanging = v.as_bool().ok_or("\"hanging\" is true or false")?;
    }
    Ok(())
}

impl TextDoc {
    /// The words with every look as it starts.
    pub fn of(text: &str) -> TextDoc {
        TextDoc { text: text.into(), ..TextDoc::default() }
    }

    /// The baseline-to-baseline distance its own look asks for.
    pub fn line_height(&self) -> f64 {
        self.leading.unwrap_or(self.size * 1.2)
    }

    /// How many characters it has.
    pub fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    /// Its own look (what characters without a run of their own take).
    pub fn base_style(&self) -> CharStyle {
        CharStyle {
            font: self.font.clone(),
            weight: self.weight,
            italic: self.italic,
            size: self.size,
            fill: self.fill,
            stroke: self.stroke,
            stroke_width: self.stroke_width,
            tracking: self.tracking,
            leading: self.leading,
            baseline_shift: self.baseline_shift,
            caps: self.caps,
            h_scale: self.h_scale,
            v_scale: self.v_scale,
            tsume: self.tsume,
            faux_bold: self.faux_bold,
            faux_italic: self.faux_italic,
            position: self.position,
            kerning: self.kerning,
            ligatures: self.ligatures,
            features: self.features.clone(),
            axes: self.axes.clone(),
            underline: self.underline,
            strikethrough: self.strikethrough,
        }
    }

    pub fn set_base_style(&mut self, s: &CharStyle) {
        (self.font, self.weight, self.italic, self.size, self.fill, self.stroke, self.stroke_width) = (s.font.clone(), s.weight, s.italic, s.size, s.fill, s.stroke, s.stroke_width);
        (self.tracking, self.leading, self.baseline_shift, self.caps, self.h_scale, self.v_scale, self.tsume) = (s.tracking, s.leading, s.baseline_shift, s.caps, s.h_scale, s.v_scale, s.tsume);
        (self.faux_bold, self.faux_italic, self.position, self.kerning, self.ligatures) = (s.faux_bold, s.faux_italic, s.position, s.kerning, s.ligatures);
        (self.features, self.axes, self.underline, self.strikethrough) = (s.features.clone(), s.axes.clone(), s.underline, s.strikethrough);
    }

    /// Its own paragraph setting (what paragraphs without one of their own take).
    pub fn base_para(&self) -> ParaStyle {
        ParaStyle {
            justify: self.justify,
            indent_left: self.indent_left,
            indent_right: self.indent_right,
            indent_first: self.indent_first,
            space_before: self.space_before,
            space_after: self.space_after,
            direction: self.direction,
            composer: self.composer,
            hanging: self.hanging,
        }
    }

    pub fn set_base_para(&mut self, p: &ParaStyle) {
        (self.justify, self.indent_left, self.indent_right, self.indent_first, self.space_before, self.space_after) = (p.justify, p.indent_left, p.indent_right, p.indent_first, p.space_before, p.space_after);
        (self.direction, self.composer, self.hanging) = (p.direction, p.composer, p.hanging);
    }

    /// Each character's look.
    pub fn styles(&self) -> Vec<CharStyle> {
        let n = self.char_count();
        if self.runs.is_empty() {
            return vec![self.base_style(); n];
        }
        let mut out = Vec::with_capacity(n);
        for r in &self.runs {
            out.extend(std::iter::repeat_n(r.style.clone(), r.len));
        }
        out.resize(n, self.runs.last().map_or_else(|| self.base_style(), |r| r.style.clone()));
        out
    }

    /// The look of character `i`.
    pub fn style_at(&self, i: usize) -> CharStyle {
        let mut at = 0;
        for r in &self.runs {
            if i < at + r.len {
                return r.style.clone();
            }
            at += r.len;
        }
        self.runs.last().map_or_else(|| self.base_style(), |r| r.style.clone())
    }

    /// Give each character the look `per_char` says (kept as runs; one look for all: its own).
    pub fn set_styles(&mut self, per_char: Vec<CharStyle>) {
        let mut runs: Vec<Run> = Vec::new();
        for s in per_char {
            match runs.last_mut() {
                Some(r) if r.style == s => r.len += 1,
                _ => runs.push(Run { len: 1, style: s }),
            }
        }
        match runs.len() {
            0 => self.runs.clear(),
            1 => {
                let s = runs.pop().expect("one").style;
                self.set_base_style(&s);
                self.runs.clear();
            }
            _ => self.runs = runs,
        }
    }

    /// Change the look of the characters `range`.
    pub fn style_range(&mut self, range: std::ops::Range<usize>, f: &mut dyn FnMut(&mut CharStyle)) {
        let mut all = self.styles();
        let end = range.end.min(all.len());
        for s in &mut all[range.start.min(end)..end] {
            f(s);
        }
        if range.start >= end && all.is_empty() {
            let mut b = self.base_style();
            f(&mut b);
            self.set_base_style(&b);
            return;
        }
        self.set_styles(all);
    }

    /// The characters of each paragraph (paragraphs end at a line break, which they keep).
    pub fn paragraph_ranges(&self) -> Vec<std::ops::Range<usize>> {
        let chars: Vec<char> = self.text.chars().collect();
        let mut out = Vec::new();
        let mut start = 0;
        for (i, &c) in chars.iter().enumerate() {
            if c == '\n' || c == '\r' {
                out.push(start..i + 1);
                start = i + 1;
            }
        }
        out.push(start..chars.len());
        out
    }

    /// Each paragraph's setting.
    pub fn para_styles(&self) -> Vec<ParaStyle> {
        let n = self.paragraph_ranges().len();
        let mut out = self.paragraphs.clone();
        out.resize(n, self.paragraphs.last().cloned().unwrap_or_else(|| self.base_para()));
        if self.paragraphs.is_empty() {
            out = vec![self.base_para(); n];
        }
        out.truncate(n);
        out
    }

    /// The setting of paragraph `p`.
    pub fn para_at(&self, p: usize) -> ParaStyle {
        self.para_styles().get(p).cloned().unwrap_or_else(|| self.base_para())
    }

    /// Give each paragraph its setting (one setting for all: its own).
    pub fn set_paras(&mut self, per_para: Vec<ParaStyle>) {
        if per_para.windows(2).all(|w| w[0] == w[1]) {
            if let Some(p) = per_para.first() {
                self.set_base_para(p);
            }
            self.paragraphs.clear();
        } else {
            self.paragraphs = per_para;
        }
    }

    /// Change the setting of paragraphs `range`.
    pub fn para_range(&mut self, range: std::ops::Range<usize>, f: &mut dyn FnMut(&mut ParaStyle)) {
        let mut all = self.para_styles();
        let end = range.end.min(all.len());
        for p in &mut all[range.start.min(end)..end] {
            f(p);
        }
        self.set_paras(all);
    }

    /// The paragraph character `i` is in.
    pub fn paragraph_of(&self, i: usize) -> usize {
        let ranges = self.paragraph_ranges();
        ranges.iter().position(|r| i < r.end).unwrap_or(ranges.len() - 1)
    }

    /// Put `text` in place of the characters `range`: what comes in takes the look of the first
    /// character it replaces (or the one before it), and paragraphs it makes take the setting
    /// of the paragraph it is in.
    pub fn replace(&mut self, range: std::ops::Range<usize>, text: &str) {
        let chars: Vec<char> = self.text.chars().collect();
        let n = chars.len();
        let (a, b) = (range.start.min(n), range.end.min(n).max(range.start.min(n)));
        let styles = self.styles();
        let paras = self.para_styles();
        let (p_start, p_end) = (self.paragraph_of(a), self.paragraph_of(b.saturating_sub(1).max(a)));
        let take = if a < n { styles.get(a).cloned() } else { styles.last().cloned() }.unwrap_or_else(|| self.base_style());
        let inserted: Vec<char> = text.chars().collect();
        let mut new_chars = chars[..a].to_vec();
        new_chars.extend(&inserted);
        new_chars.extend(&chars[b..]);
        let mut new_styles = styles[..a].to_vec();
        new_styles.extend(std::iter::repeat_n(take, inserted.len()));
        new_styles.extend(styles[b..].iter().cloned());
        self.text = new_chars.into_iter().collect();
        let after = paras.len() - p_end - 1;
        let now = self.paragraph_ranges().len();
        let middle = now.saturating_sub(p_start + after).max(1);
        let mut new_paras = paras[..p_start].to_vec();
        new_paras.extend(std::iter::repeat_n(paras[p_start].clone(), middle));
        new_paras.extend(paras[p_end + 1..].iter().cloned());
        new_paras.truncate(now);
        let base_para = self.base_para();
        new_paras.resize(now, base_para);
        self.set_styles(new_styles);
        self.set_paras(new_paras);
    }

    /// As JSON: just the words when nothing else differs from the start.
    pub fn to_json(&self) -> Json {
        let d = TextDoc::default();
        let mut o = Map::new();
        o.insert("text", self.text.as_str());
        let base = self.base_style();
        let mut look = Map::new();
        char_json(&base, &d.base_style(), &mut look);
        // the usual order: the look, then how it is set
        for k in ["font", "weight", "italic", "size", "fill", "stroke", "strokeWidth"] {
            if let Some(v) = look.get(k) {
                o.insert(k, v.clone());
            }
        }
        if self.stroke_over {
            o.insert("strokeOver", true);
        }
        for k in ["tracking", "leading"] {
            if let Some(v) = look.get(k) {
                o.insert(k, v.clone());
            }
        }
        let bp = self.base_para();
        let mut para = Map::new();
        para_json(&bp, &d.base_para(), &mut para);
        if let Some(v) = para.get("justify") {
            o.insert("justify", v.clone());
        }
        if let Some(b) = self.box_size {
            o.insert("box", b);
        }
        for k in CHAR_FIELDS.iter().skip(9) {
            if let Some(v) = look.get(k) {
                o.insert(*k, v.clone());
            }
        }
        for k in PARA_FIELDS.iter().skip(1) {
            if let Some(v) = para.get(k) {
                o.insert(*k, v.clone());
            }
        }
        if self.vertical {
            o.insert("vertical", true);
        }
        if !self.runs.is_empty() {
            o.insert(
                "runs",
                Json::array(self.runs.iter().map(|r| {
                    let mut m = Map::new();
                    m.insert("length", r.len as f64);
                    char_json(&r.style, &base, &mut m);
                    Json::Object(m)
                })),
            );
        }
        if !self.paragraphs.is_empty() {
            o.insert(
                "paragraphs",
                Json::array(self.paragraphs.iter().map(|p| {
                    let mut m = Map::new();
                    para_json(p, &bp, &mut m);
                    Json::Object(m)
                })),
            );
        }
        if o.len() == 1 { Json::from(self.text.as_str()) } else { Json::Object(o) }
    }

    /// A document from JSON (a string is the words), or what is wrong with it.
    pub fn from_json(j: &Json) -> Result<TextDoc, String> {
        if let Some(t) = j.as_str() {
            return Ok(TextDoc::of(t));
        }
        let o = j.as_object().ok_or_else(|| format!("text is a string, or {{\"text\": …, \"font\"?, \"size\"?, \"fill\"?…}}, got {}", describe(j)))?;
        if let Some(k) = o.keys().find(|k| !CHAR_FIELDS.contains(k) && !PARA_FIELDS.contains(k) && !DOC_FIELDS.contains(k)) {
            let mut all: Vec<&str> = DOC_FIELDS.to_vec();
            all.extend(CHAR_FIELDS);
            all.extend(PARA_FIELDS);
            return Err(format!("text has no \"{k}\" (it has {})", all.join(", ")));
        }
        let mut t = TextDoc::default();
        if let Some(v) = o.get("text") {
            t.text = v.as_str().ok_or_else(|| format!("\"text\" is a string, got {}", describe(v)))?.into();
        }
        let mut base = t.base_style();
        read_char(o, &mut base)?;
        t.set_base_style(&base);
        let mut bp = t.base_para();
        read_para(o, &mut bp)?;
        t.set_base_para(&bp);
        if let Some(v) = o.get("strokeOver") {
            t.stroke_over = v.as_bool().ok_or("\"strokeOver\" is true or false")?;
        }
        if let Some(v) = o.get("vertical") {
            t.vertical = v.as_bool().ok_or("\"vertical\" is true or false")?;
        }
        if let Some(v) = o.get("box") {
            let b: Option<Vec<f64>> = v.as_array().map(|a| a.iter().filter_map(Json::as_f64).collect());
            t.box_size = match (v, b) {
                (Json::Null, _) => None,
                (_, Some(b)) if b.len() == 2 && b.iter().all(|x| *x > 0.0) => Some([b[0], b[1]]),
                _ => return Err("\"box\" is [width, height], both above zero (or null for point text)".into()),
            };
        }
        if let Some(v) = o.get("runs") {
            let list = v.as_array().ok_or("\"runs\" is a list of {\"length\": n, …look}")?;
            let mut runs = Vec::new();
            for (k, r) in list.iter().enumerate() {
                let m = r.as_object().ok_or_else(|| format!("run {k} is an object"))?;
                if let Some(key) = m.keys().find(|key| *key != "length" && !CHAR_FIELDS.contains(key)) {
                    return Err(format!("run {k} has no \"{key}\""));
                }
                let len = m.get("length").and_then(Json::as_f64).filter(|l| *l >= 0.0 && l.fract() == 0.0).ok_or_else(|| format!("run {k} needs \"length\", how many characters it covers"))? as usize;
                let mut s = base.clone();
                read_char(m, &mut s).map_err(|e| format!("run {k}: {e}"))?;
                runs.push(Run { len, style: s });
            }
            // the runs cover the words: the last stretches or stops to fit
            let n = t.char_count();
            let mut covered = 0;
            runs.retain_mut(|r| {
                if covered >= n {
                    return false;
                }
                r.len = r.len.min(n - covered);
                covered += r.len;
                r.len > 0
            });
            if covered < n
                && let Some(r) = runs.last_mut()
            {
                r.len += n - covered;
            }
            let per_char: Vec<CharStyle> = runs.iter().flat_map(|r| std::iter::repeat_n(r.style.clone(), r.len)).collect();
            if !per_char.is_empty() {
                t.set_styles(per_char);
            }
        }
        if let Some(v) = o.get("paragraphs") {
            let list = v.as_array().ok_or("\"paragraphs\" is a list of each paragraph's setting")?;
            let mut paras = Vec::new();
            for (k, p) in list.iter().enumerate() {
                let m = p.as_object().ok_or_else(|| format!("paragraph {k} is an object"))?;
                if let Some(key) = m.keys().find(|key| !PARA_FIELDS.contains(key)) {
                    return Err(format!("paragraph {k} has no \"{key}\""));
                }
                let mut s = bp.clone();
                read_para(m, &mut s).map_err(|e| format!("paragraph {k}: {e}"))?;
                paras.push(s);
            }
            let n = t.paragraph_ranges().len();
            paras.truncate(n);
            let last = paras.last().cloned().unwrap_or_else(|| bp.clone());
            paras.resize(n, last);
            t.set_paras(paras);
        }
        Ok(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_alone_or_with_their_look() {
        let plain = TextDoc::of("สวัสดี");
        assert_eq!(plain.to_json().to_json(), r#""สวัสดี""#);
        assert_eq!(TextDoc::from_json(&plain.to_json()), Ok(plain));
        let styled = TextDoc { font: "Tahoma".into(), size: 48.0, stroke: Some(Rgba::BLACK), justify: Justify::Center, box_size: Some([600.0, 200.0]), leading: Some(60.0), ..TextDoc::of("Hello") };
        let j = styled.to_json();
        assert_eq!(j.to_json(), r##"{"text":"Hello","font":"Tahoma","size":48,"stroke":"#000000","leading":60,"justify":"center","box":[600,200]}"##);
        assert_eq!(TextDoc::from_json(&j), Ok(styled));
        let none = TextDoc::from_json(&Json::parse(r#"{"text": "x", "fill": null}"#).unwrap()).unwrap();
        assert_eq!(none.fill, None);
    }

    #[test]
    fn mistakes() {
        let bad = |t: &str| TextDoc::from_json(&Json::parse(t).unwrap()).unwrap_err();
        assert_eq!(bad(r#"{"text": "a", "colour": 1}"#).split(" (").next().unwrap(), "text has no \"colour\"");
        assert_eq!(bad(r#"{"text": "a", "justify": "middle"}"#), "\"justify\" is one of left, center, right, justifyLeft, justifyCenter, justifyRight, justifyAll");
        assert_eq!(bad(r#"{"text": "a", "size": -3}"#), "\"size\" is a number from 0 to 10000, got -3");
        assert!(bad("5").starts_with("text is a string"));
    }

    #[test]
    fn runs_and_paragraphs() {
        let mut d = TextDoc { font: "Georgia".into(), size: 40.0, ..TextDoc::of("Hello world\nsecond line") };
        // "world" bold and red
        d.style_range(6..11, &mut |s| {
            s.weight = 700.0;
            s.fill = Some(veymelo_color::rgba(1.0, 0.0, 0.0, 1.0));
        });
        assert_eq!(d.runs.len(), 3);
        assert_eq!(d.style_at(7).weight, 700.0);
        assert_eq!(d.style_at(2).weight, 400.0);
        assert_eq!(d.style_at(13).font, "Georgia");
        // the second paragraph centred
        d.para_range(1..2, &mut |p| p.justify = Justify::Center);
        assert_eq!(d.para_at(1).justify, Justify::Center);
        assert_eq!(d.para_at(0).justify, Justify::Left);
        let j = d.to_json();
        let back = TextDoc::from_json(&j).unwrap();
        assert_eq!(back, d, "{}", j.to_json());
        assert!(j.to_json().contains(r##""runs":[{"length":6},{"length":5,"weight":700,"fill":"#ff0000"},{"length":12}]"##), "{}", j.to_json());
        // one look for all again: no runs
        d.style_range(0..23, &mut |s| s.weight = 300.0);
        d.style_range(0..23, &mut |s| s.fill = Some(Rgba::WHITE));
        assert!(d.runs.is_empty() && d.weight == 300.0);
    }

    #[test]
    fn replacing_keeps_looks_and_settings() {
        let mut d = TextDoc::of("one two");
        d.style_range(4..7, &mut |s| s.italic = true);
        // typed inside the italic word: italic too
        d.replace(5..5, "XX");
        assert_eq!(d.text, "one tXXwo");
        assert!(d.style_at(5).italic && d.style_at(6).italic && !d.style_at(2).italic);
        // a line break makes a paragraph with the same setting
        d.set_base_para(&ParaStyle { justify: Justify::Right, ..ParaStyle::default() });
        d.replace(3..4, "\n");
        assert_eq!(d.paragraph_ranges().len(), 2);
        assert_eq!(d.para_at(1).justify, Justify::Right);
        // two paragraphs set apart, joined again: the first one's setting
        d.para_range(1..2, &mut |p| p.justify = Justify::Center);
        assert_eq!(d.paragraphs.len(), 2);
        d.replace(3..4, " ");
        assert_eq!(d.paragraph_ranges().len(), 1);
        assert!(d.paragraphs.is_empty() && d.justify == Justify::Right);
    }

    #[test]
    fn new_look_fields_read_back() {
        let j = Json::parse(r#"{"text": "x²", "hScale": 120, "fauxBold": true, "position": "superscript", "kerning": "optical", "ligatures": false, "features": ["onum", "ss01"], "axes": {"wdth": 80}, "underline": true, "indentFirst": 20, "direction": "rtl", "composer": "singleLine", "hanging": true, "vertical": true}"#).unwrap();
        let d = TextDoc::from_json(&j).unwrap();
        assert_eq!((d.h_scale, d.faux_bold, d.position, d.kerning, d.ligatures), (120.0, true, Position::Superscript, Kerning::Optical, false));
        assert_eq!(d.features, vec!["onum".to_string(), "ss01".to_string()]);
        assert_eq!(d.axes, vec![("wdth".to_string(), 80.0)]);
        assert_eq!((d.indent_first, d.direction, d.composer, d.hanging, d.vertical), (20.0, Direction::Rtl, Composer::SingleLine, true, true));
        assert_eq!(TextDoc::from_json(&d.to_json()).unwrap(), d);
        assert!(TextDoc::from_json(&Json::parse(r#"{"text": "a", "kerning": "loose"}"#).unwrap()).is_err());
    }
}

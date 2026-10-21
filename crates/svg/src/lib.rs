//! SVG pictures, read by our own XML and SVG readers (written from the W3C's SVG 1.1 Second
//! Edition and SVG 2 recommendations, and CSS Color Level 3 for colours), drawn at any size, and
//! laid out for shape layers.
//!
//! What is read: the document (`svg` with its size, `viewBox` and `preserveAspectRatio`, nested
//! `svg`, `g`, `a`, `defs`, `use` with `symbol`); the shapes (`path` with every command, `rect`
//! with round corners, `circle`, `ellipse`, `line`, `polyline`, `polygon`); `transform` lists;
//! paints (`fill`, `fill-rule`, `fill-opacity`, `stroke` and its width, caps, joins, miter limit,
//! dashes and opacity, `opacity`, `display`, `visibility`, `color` and `currentColor`); linear and
//! radial gradients (with a focal point, their units, transform, spread and inheritance by
//! `href`); styles as attributes, in the `style` attribute and in `<style>` sheets (type, class,
//! id and universal selectors). Text, pictures, filters, clipping, masks, patterns and markers are
//! passed over (and listed).
//!
//! What it draws holds more than SVG gives it, for pictures read from other files (PDF): groups
//! cut to paths and seen through masks, blend modes, pictures, meshes of shaded triangles, and
//! gradients between two circles that may paint nothing past their ends.

mod color;
mod draw;
mod pathdata;
mod xml;

use std::collections::HashMap;
use std::sync::Arc;

pub use color::parse_color;
pub use draw::draw;
pub use pathdata::{numbers, parse as parse_path};
use veymelo_geom::{Mat3, vec2};
pub use veymelo_color::BlendMode;
pub use veymelo_path::{Cap, Join, Sub, Vertex};
pub use veymelo_raster::Image;

/// How a gradient goes on past its ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Spread {
    #[default]
    Pad,
    Reflect,
    Repeat,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GradientKind {
    Linear { x1: f64, y1: f64, x2: f64, y2: f64 },
    Radial { cx: f64, cy: f64, r: f64, fx: f64, fy: f64 },
    /// From the circle about (`x0`, `y0`) of radius `r0` to the one about (`x1`, `y1`) of `r1`,
    /// each later circle painted over the earlier (a PDF's radial shading).
    Circles { x0: f64, y0: f64, r0: f64, x1: f64, y1: f64, r1: f64 },
}

/// A gradient: its line or circles, its stops (place from 0 to 1, straight RGBA), whether its
/// coordinates are parts of the shape's box (else the shape's own units), its own transform,
/// how it goes on past its ends, and whether it paints at all before its start and past its end.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub kind: GradientKind,
    pub stops: Vec<(f64, [f64; 4])>,
    pub box_units: bool,
    pub transform: Mat3,
    pub spread: Spread,
    pub ends: [bool; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Color([f64; 3]),
    Gradient(Gradient),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub paint: Paint,
    pub opacity: f64,
    pub width: f64,
    pub cap: Cap,
    pub join: Join,
    pub miter: f64,
    pub dashes: Vec<f64>,
    pub dash_offset: f64,
}

/// A shape's outline in its own units: rectangles and ellipses kept as they are (so they can be
/// shape layers' own), the rest as runs of points.
#[derive(Clone, Debug, PartialEq)]
pub enum Geom {
    Rect { x: f64, y: f64, w: f64, h: f64, rx: f64, ry: f64 },
    Ellipse { cx: f64, cy: f64, rx: f64, ry: f64 },
    Path(Vec<Sub>),
}

/// How far along a quarter circle its cubic's handles reach.
const QUARTER: f64 = 0.552_284_749_830_793_6;

impl Geom {
    /// Its outline as runs of points.
    pub fn subs(&self) -> Vec<Sub> {
        match *self {
            Geom::Rect { x, y, w, h, rx, ry } => {
                if rx <= 0.0 || ry <= 0.0 {
                    return vec![Sub::polygon(&[[x, y], [x + w, y], [x + w, y + h], [x, y + h]], true)];
                }
                let (kx, ky) = (rx * QUARTER, ry * QUARTER);
                let v = |p: [f64; 2], i: [f64; 2], o: [f64; 2]| Vertex { p, i, o };
                vec![Sub {
                    v: vec![
                        v([x + rx, y], [-kx, 0.0], [0.0, 0.0]),
                        v([x + w - rx, y], [0.0, 0.0], [kx, 0.0]),
                        v([x + w, y + ry], [0.0, -ky], [0.0, 0.0]),
                        v([x + w, y + h - ry], [0.0, 0.0], [0.0, ky]),
                        v([x + w - rx, y + h], [kx, 0.0], [0.0, 0.0]),
                        v([x + rx, y + h], [0.0, 0.0], [-kx, 0.0]),
                        v([x, y + h - ry], [0.0, ky], [0.0, 0.0]),
                        v([x, y + ry], [0.0, 0.0], [0.0, -ky]),
                    ],
                    closed: true,
                }]
            }
            Geom::Ellipse { cx, cy, rx, ry } => {
                let (kx, ky) = (rx * QUARTER, ry * QUARTER);
                vec![Sub {
                    v: vec![
                        Vertex { p: [cx + rx, cy], i: [0.0, -ky], o: [0.0, ky] },
                        Vertex { p: [cx, cy + ry], i: [kx, 0.0], o: [-kx, 0.0] },
                        Vertex { p: [cx - rx, cy], i: [0.0, ky], o: [0.0, -ky] },
                        Vertex { p: [cx, cy - ry], i: [-kx, 0.0], o: [kx, 0.0] },
                    ],
                    closed: true,
                }]
            }
            Geom::Path(ref subs) => subs.clone(),
        }
    }

    /// The box round it, in its own units (x0, y0, x1, y1).
    pub fn bounds(&self) -> Option<[f64; 4]> {
        let mut out: Option<[f64; 4]> = None;
        for s in self.subs() {
            if let Some(e) = s.extent() {
                out = Some(match out {
                    None => e,
                    Some(o) => [o[0].min(e[0]), o[1].min(e[1]), o[2].max(e[2]), o[3].max(e[3])],
                });
            }
        }
        out
    }
}

/// A shape: its name (its id, or what it is), outline, transform (its units to its parent's),
/// fill (with its opacity and rule), stroke, opacity and how it mixes with what is under it.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub name: String,
    pub geom: Geom,
    pub transform: Mat3,
    pub fill: Option<Paint>,
    pub fill_opacity: f64,
    pub even_odd: bool,
    pub stroke: Option<Stroke>,
    pub opacity: f64,
    pub blend: BlendMode,
}

/// An outline what is in a group is cut to (in the group's own units), by its rule.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub subs: Vec<Sub>,
    pub even_odd: bool,
}

/// What a group is seen through: `content` (in the group's own units) drawn, and each point of
/// the group shown as much as its alpha, or (`luminosity`) as bright as it is over `backdrop`
/// (straight sRGB), through `transfer` (how much shows for each of 256 steps of it, when given).
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    pub content: Group,
    pub luminosity: bool,
    pub backdrop: [f64; 3],
    pub transfer: Option<Vec<f64>>,
}

/// A group: its name, transform (its units to its parent's), opacity, children (drawn first
/// first), the outlines it is cut to (all of them), its mask, and how it mixes with what is
/// under it.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub name: String,
    pub transform: Mat3,
    pub opacity: f64,
    pub children: Vec<Node>,
    pub clip: Vec<Clip>,
    pub mask: Option<Box<Mask>>,
    pub blend: BlendMode,
}

impl Group {
    /// A group of `children` that is not cut, masked or blended.
    pub fn new(name: impl Into<String>, transform: Mat3, opacity: f64, children: Vec<Node>) -> Group {
        Group { name: name.into(), transform, opacity, children, clip: Vec::new(), mask: None, blend: BlendMode::Normal }
    }
}

/// A picture of pixels: its pixels (premultiplied sRGB), its transform (from its pixels, y down
/// from its top left, to its parent's units), its opacity, whether it is smoothed between its
/// pixels when drawn bigger, and how it mixes with what is under it.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub name: String,
    pub image: Arc<Image>,
    pub transform: Mat3,
    pub opacity: f64,
    pub smooth: bool,
    pub blend: BlendMode,
}

/// Triangles each shaded between its corners' colours (straight sRGB RGBA) (a PDF's gradient
/// mesh), in its own units, with its transform, opacity and blend mode.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub name: String,
    pub triangles: Vec<[([f64; 2], [f64; 4]); 3]>,
    pub transform: Mat3,
    pub opacity: f64,
    pub blend: BlendMode,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Group(Group),
    Shape(Box<Shape>),
    Picture(Box<Picture>),
    Mesh(Box<Mesh>),
}

/// A picture: its size in pixels, what it draws (the root group's transform takes its
/// `viewBox` to that size), and the kinds of element passed over.
#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    pub width: f64,
    pub height: f64,
    pub root: Group,
    pub skipped: Vec<String>,
}

impl Doc {
    /// Every shape with its whole transform (its units to the picture's pixels) and opacity, in
    /// the order they are drawn.
    pub fn shapes(&self) -> Vec<(Mat3, f64, &Shape)> {
        fn walk<'a>(g: &'a Group, m: Mat3, o: f64, out: &mut Vec<(Mat3, f64, &'a Shape)>) {
            let (m, o) = (m * g.transform, o * g.opacity);
            for c in &g.children {
                match c {
                    Node::Group(sub) => walk(sub, m, o, out),
                    Node::Shape(s) => out.push((m * s.transform, o * s.opacity, &**s)),
                    Node::Picture(_) | Node::Mesh(_) => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, Mat3::IDENTITY, 1.0, &mut out);
        out
    }
}

/// Whether bytes look like an SVG picture: XML whose root is `<svg`.
pub fn looks_like(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_string();
    head.contains("<svg") && head.trim_start_matches('\u{feff}').trim_start().starts_with('<')
}

/// The style a shape inherits.
#[derive(Clone, Debug)]
struct Style {
    fill: Option<PaintRef>,
    fill_opacity: f64,
    even_odd: bool,
    stroke: Option<PaintRef>,
    stroke_opacity: f64,
    stroke_width: f64,
    cap: Cap,
    join: Join,
    miter: f64,
    dashes: Vec<f64>,
    dash_offset: f64,
    color: [f64; 4],
    visible: bool,
    font_size: f64,
}

impl Default for Style {
    fn default() -> Style {
        Style {
            fill: Some(PaintRef::Color([0.0, 0.0, 0.0, 1.0])),
            fill_opacity: 1.0,
            even_odd: false,
            stroke: None,
            stroke_opacity: 1.0,
            stroke_width: 1.0,
            cap: Cap::Butt,
            join: Join::Miter,
            miter: 4.0,
            dashes: Vec::new(),
            dash_offset: 0.0,
            color: [0.0, 0.0, 0.0, 1.0],
            visible: true,
            font_size: 16.0,
        }
    }
}

#[derive(Clone, Debug)]
enum PaintRef {
    Color([f64; 4]),
    /// A gradient by id, and the colour to use if there is none.
    Url(String, Option<[f64; 4]>),
    Current,
}

/// A rule of a style sheet: what it picks (a type, classes, an id), how strongly, when it came,
/// and what it sets.
#[derive(Clone, Debug)]
struct Rule {
    tag: Option<String>,
    classes: Vec<String>,
    id: Option<String>,
    strength: u32,
    order: usize,
    sets: Vec<(String, String)>,
}

/// `name: value; …`
fn declarations(s: &str) -> Vec<(String, String)> {
    s.split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            Some((k.trim().to_ascii_lowercase(), v.trim().trim_end_matches("!important").trim().to_string()))
        })
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

/// The rules of a style sheet (only simple selectors: no combinators, pseudo-classes or
/// attributes).
fn sheet(css: &str, rules: &mut Vec<Rule>) {
    let mut text = String::new();
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        text.push_str(&rest[..i]);
        rest = rest[i + 2..].find("*/").map_or("", |j| &rest[i + 2 + j + 2..]);
    }
    text.push_str(rest);
    let mut rest = text.as_str();
    while let Some(open) = rest.find('{') {
        let selectors = rest[..open].trim().to_string();
        let Some(close) = rest[open..].find('}') else { break };
        let sets = declarations(&rest[open + 1..open + close]);
        rest = &rest[open + close + 1..];
        if selectors.starts_with('@') {
            continue;
        }
        for one in selectors.split(',').map(str::trim) {
            if one.is_empty() || one.contains([' ', '>', '+', '~', ':', '[']) {
                continue;
            }
            let (mut tag, mut classes, mut id) = (None, Vec::new(), None);
            let mut part = String::new();
            let mut kind = 't';
            let mut flush = |kind: char, part: &mut String| {
                if !part.is_empty() {
                    let p = std::mem::take(part);
                    match kind {
                        '.' => classes.push(p),
                        '#' => id = Some(p),
                        _ if p != "*" => tag = Some(p),
                        _ => {}
                    }
                }
            };
            for c in one.chars() {
                if c == '.' || c == '#' {
                    flush(kind, &mut part);
                    kind = c;
                } else {
                    part.push(c);
                }
            }
            flush(kind, &mut part);
            let strength = id.is_some() as u32 * 100 + classes.len() as u32 * 10 + tag.is_some() as u32;
            let order = rules.len();
            rules.push(Rule { tag, classes, id, strength, order, sets: sets.clone() });
        }
    }
}

/// The properties styles may set.
const PROPERTIES: &[&str] = &[
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-dasharray",
    "stroke-dashoffset",
    "opacity",
    "display",
    "visibility",
    "color",
    "font-size",
    "stop-color",
    "stop-opacity",
];

/// The most elements a picture may make, `use` copies counted (pictures that use each other over
/// and over could make billions).
const MOST: usize = 500_000;

struct Reader<'x> {
    xml: &'x xml::Xml,
    ids: HashMap<String, usize>,
    rules: Vec<Rule>,
    /// The viewport's size, for percentages.
    vw: f64,
    vh: f64,
    skipped: Vec<String>,
    made: usize,
    budget: usize,
    /// The `use` targets being followed (one leading back into them is passed over).
    using: Vec<usize>,
}

/// A `transform` list as a matrix.
pub fn transform(s: &str) -> Mat3 {
    let mut m = Mat3::IDENTITY;
    let mut rest = s;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let Some(close) = rest[open..].find(')') else { break };
        let a = numbers(&rest[open + 1..open + close]);
        rest = &rest[open + close + 1..];
        let t = match (name, a.len()) {
            ("matrix", 6) => Mat3([[a[0], a[2], a[4]], [a[1], a[3], a[5]], [0.0, 0.0, 1.0]]),
            ("translate", 1) => Mat3::translate(vec2(a[0], 0.0)),
            ("translate", 2) => Mat3::translate(vec2(a[0], a[1])),
            ("scale", 1) => Mat3::scale(vec2(a[0], a[0])),
            ("scale", 2) => Mat3::scale(vec2(a[0], a[1])),
            ("rotate", 1) => Mat3::rotate_deg(a[0]),
            ("rotate", 3) => Mat3::translate(vec2(a[1], a[2])) * Mat3::rotate_deg(a[0]) * Mat3::translate(vec2(-a[1], -a[2])),
            ("skewX", 1) => Mat3([[1.0, a[0].to_radians().tan(), 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
            ("skewY", 1) => Mat3([[1.0, 0.0, 0.0], [a[0].to_radians().tan(), 1.0, 0.0], [0.0, 0.0, 1.0]]),
            _ => Mat3::IDENTITY,
        };
        m = m * t;
    }
    m
}

impl Reader<'_> {
    fn el(&self, i: usize) -> &xml::Element {
        &self.xml.elements[i]
    }

    /// What an element sets: its presentation attributes, then the style sheets' rules that pick
    /// it (weakest first), then its `style` attribute.
    fn declared(&self, i: usize) -> HashMap<String, String> {
        let e = self.el(i);
        let mut out = HashMap::new();
        for (k, v) in &e.attrs {
            if PROPERTIES.contains(&k.as_str()) {
                out.insert(k.clone(), v.clone());
            }
        }
        let classes: Vec<&str> = e.attr("class").map(|c| c.split_whitespace().collect()).unwrap_or_default();
        let id = e.attr("id");
        let mut picked: Vec<&Rule> = self
            .rules
            .iter()
            .filter(|r| r.tag.as_deref().is_none_or(|t| t == e.name) && r.id.as_deref().is_none_or(|x| Some(x) == id) && r.classes.iter().all(|c| classes.contains(&c.as_str())))
            .collect();
        picked.sort_by_key(|r| (r.strength, r.order));
        for r in picked {
            for (k, v) in &r.sets {
                out.insert(k.clone(), v.clone());
            }
        }
        if let Some(st) = e.attr("style") {
            out.extend(declarations(st));
        }
        out
    }

    /// A length in pixels (`axis` 'x' or 'y' for percentages of the viewport's width or
    /// height; else of its diagonal).
    fn length(&self, s: &str, axis: char, font: f64) -> Option<f64> {
        let s = s.trim();
        let end = s.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))).unwrap_or(s.len());
        let (mut num, mut unit) = (&s[..end], &s[end..]);
        // an "e" that starts "em" or "ex"
        if num.parse::<f64>().is_err() && (num.ends_with('e') || num.ends_with('E')) {
            num = &s[..end - 1];
            unit = &s[end - 1..];
        }
        let v: f64 = num.parse().ok()?;
        Some(match unit.trim() {
            "pt" => v * 4.0 / 3.0,
            "pc" => v * 16.0,
            "mm" => v * 96.0 / 25.4,
            "cm" => v * 96.0 / 2.54,
            "in" => v * 96.0,
            "em" => v * font,
            "ex" => v * font / 2.0,
            "%" => {
                v / 100.0
                    * match axis {
                        'x' => self.vw,
                        'y' => self.vh,
                        _ => ((self.vw * self.vw + self.vh * self.vh) / 2.0).sqrt(),
                    }
            }
            _ => v,
        })
    }

    fn attr_length(&self, i: usize, name: &str, axis: char, st: &Style) -> f64 {
        self.el(i).attr(name).and_then(|v| self.length(v, axis, st.font_size)).unwrap_or(0.0)
    }

    /// A paint as written: none (Some(None)), a colour, a gradient, the current colour; or
    /// inherited (None).
    fn paint(&self, v: &str) -> Option<Option<PaintRef>> {
        let v = v.trim();
        match v {
            "none" => return Some(None),
            "currentColor" | "currentcolor" => return Some(Some(PaintRef::Current)),
            "inherit" => return None,
            _ => {}
        }
        if let Some(rest) = v.strip_prefix("url(") {
            let (url, fallback) = rest.split_once(')').unwrap_or((rest, ""));
            let id = url.trim().trim_matches(['"', '\'']).trim_start_matches('#').to_string();
            let fallback = fallback.trim();
            return Some(Some(PaintRef::Url(id, if fallback.is_empty() || fallback == "none" { None } else { parse_color(fallback) })));
        }
        parse_color(v).map(|c| Some(PaintRef::Color(c)))
    }

    /// An element's settings applied over what it inherits; its opacity and whether it is shown.
    fn cascade(&self, i: usize, st: &mut Style) -> (f64, bool) {
        let d = self.declared(i);
        let part = |k: &str| {
            d.get(k).and_then(|v| {
                let v = v.trim();
                match v.strip_suffix('%') {
                    Some(p) => p.parse::<f64>().ok().map(|x| x / 100.0),
                    None => v.parse::<f64>().ok(),
                }
            })
        };
        if let Some(c) = d.get("color").and_then(|v| parse_color(v)) {
            st.color = c;
        }
        if let Some(v) = d.get("font-size").and_then(|v| self.length(v, 'o', st.font_size)) {
            st.font_size = v;
        }
        if let Some(p) = d.get("fill").and_then(|v| self.paint(v)) {
            st.fill = p;
        }
        if let Some(p) = d.get("stroke").and_then(|v| self.paint(v)) {
            st.stroke = p;
        }
        if let Some(v) = part("fill-opacity") {
            st.fill_opacity = v.clamp(0.0, 1.0);
        }
        if let Some(v) = part("stroke-opacity") {
            st.stroke_opacity = v.clamp(0.0, 1.0);
        }
        if let Some(v) = d.get("stroke-width").and_then(|v| self.length(v, 'o', st.font_size)) {
            st.stroke_width = v.max(0.0);
        }
        if let Some(v) = part("stroke-miterlimit") {
            st.miter = v.max(1.0);
        }
        if let Some(v) = d.get("fill-rule") {
            st.even_odd = v.trim() == "evenodd";
        }
        if let Some(v) = d.get("stroke-linecap") {
            st.cap = match v.trim() {
                "round" => Cap::Round,
                "square" => Cap::Square,
                _ => Cap::Butt,
            };
        }
        if let Some(v) = d.get("stroke-linejoin") {
            st.join = match v.trim() {
                "round" => Join::Round,
                "bevel" => Join::Bevel,
                _ => Join::Miter,
            };
        }
        if let Some(v) = d.get("stroke-dasharray") {
            let dashes: Vec<f64> = v.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()).filter_map(|t| self.length(t, 'o', st.font_size)).collect();
            st.dashes = if v.trim() != "none" && dashes.iter().all(|x| *x >= 0.0) && dashes.iter().any(|x| *x > 0.0) {
                // an odd count is said twice
                if dashes.len() % 2 == 1 { dashes.iter().chain(&dashes).copied().collect() } else { dashes }
            } else {
                Vec::new()
            };
        }
        if let Some(v) = d.get("stroke-dashoffset").and_then(|v| self.length(v, 'o', st.font_size)) {
            st.dash_offset = v;
        }
        if let Some(v) = d.get("visibility") {
            st.visible = !matches!(v.trim(), "hidden" | "collapse");
        }
        (part("opacity").unwrap_or(1.0).clamp(0.0, 1.0), d.get("display").is_none_or(|v| v.trim() != "none"))
    }

    /// What a paint comes to, and the opacity it brings.
    fn resolve(&self, p: &PaintRef, st: &Style) -> Option<(Paint, f64)> {
        match p {
            PaintRef::Color(c) => Some((Paint::Color([c[0], c[1], c[2]]), c[3])),
            PaintRef::Current => Some((Paint::Color([st.color[0], st.color[1], st.color[2]]), st.color[3])),
            PaintRef::Url(id, fallback) => match self.gradient(id) {
                // a single stop is a plain colour
                Some(g) if g.stops.len() == 1 => {
                    let c = g.stops[0].1;
                    Some((Paint::Color([c[0], c[1], c[2]]), c[3]))
                }
                Some(g) if !g.stops.is_empty() => Some((Paint::Gradient(g), 1.0)),
                Some(_) => None,
                None => fallback.map(|c| (Paint::Color([c[0], c[1], c[2]]), c[3])),
            },
        }
    }

    /// The element an element's `href` names.
    fn href(&self, i: usize) -> Option<usize> {
        let h = self.el(i).attr("href")?;
        self.ids.get(h.trim().trim_start_matches('#')).copied()
    }

    /// A gradient by id: its attributes and stops from it or what it inherits by `href`.
    fn gradient(&self, id: &str) -> Option<Gradient> {
        let first = *self.ids.get(id)?;
        let tag = self.el(first).name.clone();
        if tag != "linearGradient" && tag != "radialGradient" {
            return None;
        }
        let mut chain = vec![first];
        while let Some(next) = self.href(*chain.last().expect("one at least")) {
            if chain.len() > 16 || chain.contains(&next) {
                break;
            }
            chain.push(next);
        }
        let attr = |name: &str| chain.iter().find_map(|&e| self.el(e).attr(name));
        let box_units = attr("gradientUnits") != Some("userSpaceOnUse");
        let st = Style::default();
        let len = |name: &str, axis: char, default: &str| -> f64 {
            let v = attr(name).unwrap_or(default).trim();
            if box_units {
                match v.strip_suffix('%') {
                    Some(p) => p.parse::<f64>().unwrap_or(0.0) / 100.0,
                    None => v.parse().unwrap_or(0.0),
                }
            } else {
                self.length(v, axis, st.font_size).unwrap_or(0.0)
            }
        };
        let kind = if tag == "linearGradient" {
            GradientKind::Linear { x1: len("x1", 'x', "0%"), y1: len("y1", 'y', "0%"), x2: len("x2", 'x', "100%"), y2: len("y2", 'y', "0%") }
        } else {
            let (cx, cy, r) = (len("cx", 'x', "50%"), len("cy", 'y', "50%"), len("r", 'o', "50%"));
            let fx = if attr("fx").is_some() { len("fx", 'x', "50%") } else { cx };
            let fy = if attr("fy").is_some() { len("fy", 'y', "50%") } else { cy };
            GradientKind::Radial { cx, cy, r, fx, fy }
        };
        let spread = match attr("spreadMethod") {
            Some("reflect") => Spread::Reflect,
            Some("repeat") => Spread::Repeat,
            _ => Spread::Pad,
        };
        // the stops of the first in the chain that has any, each no earlier than the last
        let mut stops = Vec::new();
        if let Some(&from) = chain.iter().find(|&&e| self.el(e).children.iter().any(|&c| self.el(c).name == "stop")) {
            let mut last = 0.0f64;
            for &s in self.el(from).children.iter().filter(|&&c| self.el(c).name == "stop") {
                let d = self.declared(s);
                let place = self
                    .el(s)
                    .attr("offset")
                    .map(|o| {
                        let o = o.trim();
                        match o.strip_suffix('%') {
                            Some(p) => p.parse::<f64>().unwrap_or(0.0) / 100.0,
                            None => o.parse().unwrap_or(0.0),
                        }
                    })
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0)
                    .max(last);
                last = place;
                let c = d.get("stop-color").and_then(|c| if c.trim() == "currentColor" { d.get("color").and_then(|x| parse_color(x)) } else { parse_color(c) }).unwrap_or([0.0, 0.0, 0.0, 1.0]);
                let a = d.get("stop-opacity").and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(1.0).clamp(0.0, 1.0);
                stops.push((place, [c[0], c[1], c[2], c[3] * a]));
            }
        }
        Some(Gradient { kind, stops, box_units, transform: attr("gradientTransform").map(transform).unwrap_or(Mat3::IDENTITY), spread, ends: [true, true] })
    }

    fn name_of(&self, i: usize) -> String {
        let e = self.el(i);
        e.attr("id").map_or_else(|| e.name.clone(), str::to_string)
    }

    /// A group of the children of `i` (styled by `st`).
    fn group_of(&mut self, i: usize, name: String, transform: Mat3, opacity: f64, st: &Style, depth: usize) -> Group {
        let mut children = Vec::new();
        for c in self.el(i).children.clone() {
            self.node(c, st, &mut children, depth + 1);
        }
        Group::new(name, transform, opacity, children)
    }

    /// Element `i` (and what is inside it) made nodes onto `out`.
    fn node(&mut self, i: usize, parent: &Style, out: &mut Vec<Node>, depth: usize) {
        if depth > 64 || self.made > self.budget {
            return;
        }
        self.made += 1;
        let tag = self.el(i).name.clone();
        match tag.as_str() {
            "defs" | "title" | "desc" | "metadata" | "style" | "linearGradient" | "radialGradient" | "symbol" | "clipPath" | "mask" | "pattern" | "marker" | "script" => return,
            "text" | "image" | "filter" | "foreignObject" | "switch" => {
                self.skipped.push(tag);
                return;
            }
            _ => {}
        }
        let mut st = parent.clone();
        let (opacity, shown) = self.cascade(i, &mut st);
        if !shown {
            return;
        }
        let mut place = self.el(i).attr("transform").map(transform).unwrap_or(Mat3::IDENTITY);
        match tag.as_str() {
            "g" | "a" => {
                let g = self.group_of(i, self.name_of(i), place, opacity, &st, depth);
                out.push(Node::Group(g));
            }
            "svg" => {
                // a viewport inside
                let (x, y) = (self.attr_length(i, "x", 'x', &st), self.attr_length(i, "y", 'y', &st));
                let w = self.el(i).attr("width").and_then(|v| self.length(v, 'x', st.font_size)).filter(|v| v.is_finite()).unwrap_or(self.vw);
                let h = self.el(i).attr("height").and_then(|v| self.length(v, 'y', st.font_size)).filter(|v| v.is_finite()).unwrap_or(self.vh);
                let fit = viewbox_fit(self.el(i), w, h);
                let saved = (self.vw, self.vh);
                (self.vw, self.vh) = viewbox(self.el(i)).map_or((w, h), |(_, vw, vh)| (vw, vh));
                let g = self.group_of(i, self.name_of(i), Mat3::translate(vec2(x, y)) * fit, opacity, &st, depth);
                (self.vw, self.vh) = saved;
                out.push(Node::Group(g));
            }
            "use" => {
                let Some(target) = self.href(i) else { return };
                if target == i || self.using.contains(&target) {
                    return;
                }
                self.using.push(target);
                place = place * Mat3::translate(vec2(self.attr_length(i, "x", 'x', &st), self.attr_length(i, "y", 'y', &st)));
                let mut children = Vec::new();
                if self.el(target).name == "symbol" {
                    let w = self.el(i).attr("width").and_then(|v| self.length(v, 'x', st.font_size)).unwrap_or(self.vw);
                    let h = self.el(i).attr("height").and_then(|v| self.length(v, 'y', st.font_size)).unwrap_or(self.vh);
                    let fit = viewbox_fit(self.el(target), w, h);
                    let mut inner = st.clone();
                    let (o, _) = self.cascade(target, &mut inner);
                    let g = self.group_of(target, self.name_of(target), fit, o, &inner, depth);
                    children.push(Node::Group(g));
                } else {
                    self.node(target, &st, &mut children, depth + 1);
                }
                self.using.pop();
                out.push(Node::Group(Group::new(self.name_of(i), place, opacity, children)));
            }
            "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" => {
                if !st.visible {
                    return;
                }
                let Some(geom) = self.geometry(i, &st) else { return };
                let fill = if tag == "line" { None } else { st.fill.as_ref().and_then(|p| self.resolve(p, &st)) };
                let (fill, fill_alpha) = match fill {
                    Some((p, a)) => (Some(p), a),
                    None => (None, 1.0),
                };
                let stroke = st.stroke.as_ref().and_then(|p| self.resolve(p, &st)).filter(|_| st.stroke_width > 0.0).map(|(paint, a)| Stroke {
                    paint,
                    opacity: st.stroke_opacity * a,
                    width: st.stroke_width,
                    cap: st.cap,
                    join: st.join,
                    miter: st.miter,
                    dashes: st.dashes.clone(),
                    dash_offset: st.dash_offset,
                });
                out.push(Node::Shape(Box::new(Shape { name: self.name_of(i), geom, transform: place, fill, fill_opacity: st.fill_opacity * fill_alpha, even_odd: st.even_odd, stroke, opacity, blend: BlendMode::Normal })));
            }
            _ => {
                // elements not known that hold others: their children drawn
                if !self.el(i).children.is_empty() {
                    let g = self.group_of(i, self.name_of(i), place, opacity, &st, depth);
                    if !g.children.is_empty() {
                        out.push(Node::Group(g));
                    }
                }
            }
        }
    }

    /// A shape element's outline.
    fn geometry(&self, i: usize, st: &Style) -> Option<Geom> {
        let e = self.el(i);
        let l = |name: &str, axis: char| self.attr_length(i, name, axis, st);
        Some(match e.name.as_str() {
            "path" => {
                let subs = parse_path(e.attr("d")?);
                if subs.is_empty() {
                    return None;
                }
                Geom::Path(subs)
            }
            "rect" => {
                let (w, h) = (l("width", 'x'), l("height", 'y'));
                if !(w > 0.0 && h > 0.0) {
                    return None;
                }
                let rx = e.attr("rx").and_then(|v| self.length(v, 'x', st.font_size));
                let ry = e.attr("ry").and_then(|v| self.length(v, 'y', st.font_size));
                let (rx, ry) = match (rx, ry) {
                    (Some(x), Some(y)) => (x, y),
                    (Some(x), None) => (x, x),
                    (None, Some(y)) => (y, y),
                    (None, None) => (0.0, 0.0),
                };
                Geom::Rect { x: l("x", 'x'), y: l("y", 'y'), w, h, rx: rx.clamp(0.0, w / 2.0), ry: ry.clamp(0.0, h / 2.0) }
            }
            "circle" => {
                let r = l("r", 'o');
                (r > 0.0).then_some(())?;
                Geom::Ellipse { cx: l("cx", 'x'), cy: l("cy", 'y'), rx: r, ry: r }
            }
            "ellipse" => {
                let (rx, ry) = (l("rx", 'x'), l("ry", 'y'));
                (rx > 0.0 && ry > 0.0).then_some(())?;
                Geom::Ellipse { cx: l("cx", 'x'), cy: l("cy", 'y'), rx, ry }
            }
            "line" => Geom::Path(vec![Sub::polygon(&[[l("x1", 'x'), l("y1", 'y')], [l("x2", 'x'), l("y2", 'y')]], false)]),
            "polyline" | "polygon" => {
                let nums = numbers(e.attr("points")?);
                if nums.len() < 4 {
                    return None;
                }
                let pts: Vec<[f64; 2]> = nums.as_chunks::<2>().0.to_vec();
                Geom::Path(vec![Sub::polygon(&pts, e.name == "polygon")])
            }
            _ => return None,
        })
    }
}

/// A `viewBox`: its corner and size.
fn viewbox(e: &xml::Element) -> Option<([f64; 2], f64, f64)> {
    let v = numbers(e.attr("viewBox")?);
    (v.len() == 4 && v.iter().all(|x| x.is_finite()) && v[2] > 0.0 && v[3] > 0.0).then(|| ([v[0], v[1]], v[2], v[3]))
}

/// The `viewBox` of `e` fitted to `w` × `h` as its `preserveAspectRatio` says (by default:
/// whole, in the middle).
fn viewbox_fit(e: &xml::Element, w: f64, h: f64) -> Mat3 {
    let Some(([x, y], vw, vh)) = viewbox(e) else { return Mat3::IDENTITY };
    let par = e.attr("preserveAspectRatio").unwrap_or("xMidYMid meet");
    let mut parts = par.split_whitespace();
    let mut align = parts.next().unwrap_or("xMidYMid");
    if align == "defer" {
        align = parts.next().unwrap_or("xMidYMid");
    }
    let slice = parts.next() == Some("slice");
    let (sx, sy) = (w / vw, h / vh);
    if align == "none" {
        return Mat3::scale(vec2(sx, sy)) * Mat3::translate(vec2(-x, -y));
    }
    let s = if slice { sx.max(sy) } else { sx.min(sy) };
    let along = |mid: &str, max: &str| if align.contains(mid) { 0.5 } else if align.contains(max) { 1.0 } else { 0.0 };
    Mat3::translate(vec2((w - vw * s) * along("xMid", "xMax"), (h - vh * s) * along("YMid", "YMax"))) * Mat3::scale(vec2(s, s)) * Mat3::translate(vec2(-x, -y))
}

/// An SVG picture from its file's bytes.
pub fn parse(bytes: &[u8]) -> Result<Doc, String> {
    let text = String::from_utf8_lossy(bytes);
    let xml = xml::parse(text.trim_start_matches('\u{feff}'))?;
    let root = xml.root();
    if root.name != "svg" {
        return Err("it is not an SVG picture (its root is not <svg>)".into());
    }
    let mut ids = HashMap::new();
    let mut rules = Vec::new();
    let all = xml.descendants(0);
    for &i in &all {
        let e = &xml.elements[i];
        if let Some(id) = e.attr("id") {
            ids.entry(id.to_string()).or_insert(i);
        }
        if e.name == "style" {
            sheet(&e.text, &mut rules);
        }
    }
    let vb = viewbox(root);
    let mut r = Reader { xml: &xml, ids, rules, vw: vb.map_or(300.0, |v| v.1), vh: vb.map_or(150.0, |v| v.2), skipped: Vec::new(), made: 0, budget: MOST.max(all.len() * 2), using: Vec::new() };
    let st = Style::default();
    let size = |name: &str, axis: char, default: f64| -> f64 {
        match root.attr(name) {
            Some(v) if v.trim().ends_with('%') => default * v.trim().trim_end_matches('%').parse::<f64>().unwrap_or(100.0) / 100.0,
            Some(v) => r.length(v, axis, st.font_size).filter(|x| *x > 0.0).unwrap_or(default),
            None => default,
        }
    };
    // without a width or height: the viewBox's (one given: the other kept in proportion)
    let (w, h) = match vb {
        Some((_, vw, vh)) => match (root.attr("width"), root.attr("height")) {
            (Some(_), None) => {
                let w = size("width", 'x', vw);
                (w, w * vh / vw)
            }
            (None, Some(_)) => {
                let h = size("height", 'y', vh);
                (h * vw / vh, h)
            }
            _ => (size("width", 'x', vw), size("height", 'y', vh)),
        },
        None => (size("width", 'x', 300.0), size("height", 'y', 150.0)),
    };
    let usable = |v: f64, d: f64| if v.is_finite() && v > 0.0 { v } else { d };
    let (w, h) = (usable(w, 300.0), usable(h, 150.0));
    if vb.is_none() {
        (r.vw, r.vh) = (w, h);
    }
    let mut top = st.clone();
    let (opacity, _) = r.cascade(0, &mut top);
    let mut children = Vec::new();
    for c in xml.root().children.clone() {
        r.node(c, &top, &mut children, 1);
    }
    if r.made > r.budget {
        return Err(format!("its <use> references make more than {} elements", r.budget));
    }
    let mut skipped = r.skipped;
    skipped.sort();
    skipped.dedup();
    Ok(Doc { width: w, height: h, root: Group::new("svg", viewbox_fit(xml.root(), w, h), opacity, children), skipped })
}

#[cfg(test)]
mod tests;

//! A page's content run, operator by operator, into the nodes of a vector picture: the graphics
//! state (saved and restored), paths filled and stroked in their colours or patterns, cut to
//! clipping paths, soft masks, blend modes and opacity; text shown glyph by glyph in its fonts;
//! form XObjects (transparency groups as groups), pictures, shadings; optional content (Illustrator's
//! layers) as named groups, hidden ones left out.
//!
//! Each node keeps its own transform to the page's space (shapes their paths in user space, as
//! they were drawn); the clipping paths, masks and layers a node is drawn through become the
//! groups round it, shared by the nodes after it while they stay the same.

use std::collections::HashMap;
use std::sync::Arc;

use veymelo_geom::{Mat3, vec2};
use veymelo_path::{Cap, Join, Sub, Vertex};
use veymelo_svg::{BlendMode, Clip, Geom, Group, Mask, Mesh, Node, Paint, Picture, Shape, Stroke};

use crate::colour::Space;
use crate::content::{Op, ops};
use crate::file::{File, rect};
use crate::function::Function;
use crate::object::{Dict, Obj};
use crate::shading::Shading;
use crate::text::{Outlines, PdfFont};

/// A PDF matrix `[a b c d e f]` as ours.
pub fn mat(a: [f64; 6]) -> Mat3 {
    Mat3([[a[0], a[2], a[4]], [a[1], a[3], a[5]], [0.0, 0.0, 1.0]])
}

fn mat_of(o: Option<Obj>) -> Option<Mat3> {
    let v: Vec<f64> = o?.as_array()?.iter().filter_map(Obj::as_f64).collect();
    (v.len() == 6).then(|| mat([v[0], v[1], v[2], v[3], v[4], v[5]]))
}

/// What nodes are drawn through: a clipping path or a mask (in the page's space), or a layer.
enum Level {
    Clip(Clip),
    Mask(Mask),
    Layer(String),
}

#[derive(Clone)]
struct Ink {
    space: Space,
    comps: Vec<f64>,
    pattern: Option<Obj>,
}

impl Ink {
    fn of(space: Space) -> Ink {
        Ink { comps: space.initial(), space, pattern: None }
    }

    fn rgb(&self) -> Option<[f64; 3]> {
        match &self.space {
            Space::Pattern(Some(under)) => under.rgb(&self.comps),
            s => s.rgb(&self.comps),
        }
    }
}

#[derive(Clone)]
struct Gs {
    ctm: Mat3,
    fill: Ink,
    stroke: Ink,
    fill_alpha: f64,
    stroke_alpha: f64,
    width: f64,
    cap: Cap,
    join: Join,
    miter: f64,
    dash: Vec<f64>,
    dash_phase: f64,
    blend: BlendMode,
    smask: Option<Arc<Level>>,
    clips: Vec<Arc<Level>>,
    font: Option<Arc<PdfFont>>,
    size: f64,
    char_space: f64,
    word_space: f64,
    hscale: f64,
    leading: f64,
    rise: f64,
    render: i64,
    /// A Type 3 glyph that takes its colour from the text: its own colours passed over.
    colour_locked: bool,
}

impl Gs {
    fn new(ctm: Mat3) -> Gs {
        Gs {
            ctm,
            fill: Ink::of(Space::Gray),
            stroke: Ink::of(Space::Gray),
            fill_alpha: 1.0,
            stroke_alpha: 1.0,
            width: 1.0,
            cap: Cap::Butt,
            join: Join::Miter,
            miter: 10.0,
            dash: Vec::new(),
            dash_phase: 0.0,
            blend: BlendMode::Normal,
            smask: None,
            clips: Vec::new(),
            font: None,
            size: 0.0,
            char_space: 0.0,
            word_space: 0.0,
            hscale: 1.0,
            leading: 0.0,
            rise: 0.0,
            render: 0,
            colour_locked: false,
        }
    }
}

/// The nodes made so far, inside the levels they are drawn through.
struct Out {
    stack: Vec<(Option<Arc<Level>>, Vec<Node>)>,
}

impl Out {
    fn new() -> Out {
        Out { stack: vec![(None, Vec::new())] }
    }

    fn emit(&mut self, node: Node, chain: &[Arc<Level>]) {
        let open = self.stack.len() - 1;
        let mut common = 0;
        while common < open && common < chain.len() && self.stack[common + 1].0.as_ref().is_some_and(|l| Arc::ptr_eq(l, &chain[common])) {
            common += 1;
        }
        while self.stack.len() - 1 > common {
            self.close_one();
        }
        for l in &chain[common..] {
            self.stack.push((Some(l.clone()), Vec::new()));
        }
        if let Some(top) = self.stack.last_mut() {
            top.1.push(node);
        }
    }

    fn close_one(&mut self) {
        let Some((level, children)) = self.stack.pop() else { return };
        if children.is_empty() {
            return;
        }
        let g = match level.as_deref() {
            Some(Level::Clip(c)) => Group { clip: vec![c.clone()], ..Group::new("Clip", Mat3::IDENTITY, 1.0, children) },
            Some(Level::Mask(m)) => Group { mask: Some(Box::new(m.clone())), ..Group::new("Mask", Mat3::IDENTITY, 1.0, children) },
            Some(Level::Layer(n)) => Group::new(n.clone(), Mat3::IDENTITY, 1.0, children),
            None => Group::new("Group", Mat3::IDENTITY, 1.0, children),
        };
        if let Some(top) = self.stack.last_mut() {
            top.1.push(Node::Group(g));
        }
    }

    fn finish(mut self) -> Vec<Node> {
        while self.stack.len() > 1 {
            self.close_one();
        }
        self.stack.pop().map(|s| s.1).unwrap_or_default()
    }
}

/// A path being built, in user space.
#[derive(Default)]
struct PathBuilder {
    subs: Vec<Sub>,
    cur: Option<Sub>,
    start: [f64; 2],
    at: [f64; 2],
}

impl PathBuilder {
    fn finish_cur(&mut self) {
        if let Some(c) = self.cur.take()
            && c.v.len() > 1
        {
            self.subs.push(c);
        }
    }

    fn move_to(&mut self, p: [f64; 2]) {
        self.finish_cur();
        self.cur = Some(Sub { v: vec![Vertex::at(p[0], p[1])], closed: false });
        self.start = p;
        self.at = p;
    }

    fn ensure(&mut self) -> &mut Sub {
        if self.cur.is_none() {
            let at = self.at;
            self.cur = Some(Sub { v: vec![Vertex::at(at[0], at[1])], closed: false });
            self.start = at;
        }
        self.cur.as_mut().expect("just made")
    }

    fn line_to(&mut self, p: [f64; 2]) {
        self.ensure().v.push(Vertex::at(p[0], p[1]));
        self.at = p;
    }

    fn curve_to(&mut self, a: [f64; 2], b: [f64; 2], p: [f64; 2]) {
        let s = self.ensure();
        if let Some(last) = s.v.last_mut() {
            last.o = [a[0] - last.p[0], a[1] - last.p[1]];
        }
        s.v.push(Vertex { p, i: [b[0] - p[0], b[1] - p[1]], o: [0.0, 0.0] });
        self.at = p;
    }

    fn close(&mut self) {
        if let Some(mut c) = self.cur.take() {
            c.closed = true;
            if c.v.len() > 1 {
                self.subs.push(c);
            }
        }
        self.at = self.start;
    }

    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.finish_cur();
        self.subs.push(Sub::polygon(&[[x, y], [x + w, y], [x + w, y + h], [x, y + h]], true));
        self.at = [x, y];
        self.start = [x, y];
    }

    fn take(&mut self) -> Vec<Sub> {
        self.finish_cur();
        std::mem::take(&mut self.subs)
    }
}

/// A pen that keeps glyph outlines as runs of points.
struct SubPen<'a> {
    out: &'a mut Vec<Sub>,
    cur: Option<Sub>,
}

impl veymelo_font::Pen for SubPen<'_> {
    fn move_to(&mut self, x: f64, y: f64) {
        if let Some(c) = self.cur.take()
            && c.v.len() > 1
        {
            self.out.push(c);
        }
        self.cur = Some(Sub { v: vec![Vertex::at(x, y)], closed: true });
    }
    fn line_to(&mut self, x: f64, y: f64) {
        if let Some(c) = &mut self.cur {
            c.v.push(Vertex::at(x, y));
        }
    }
    fn quad_to(&mut self, cx: f64, cy: f64, x: f64, y: f64) {
        let Some(c) = &mut self.cur else { return };
        let Some(last) = c.v.last_mut() else { return };
        let p0 = last.p;
        last.o = [(cx - p0[0]) * 2.0 / 3.0, (cy - p0[1]) * 2.0 / 3.0];
        c.v.push(Vertex { p: [x, y], i: [(cx - x) * 2.0 / 3.0, (cy - y) * 2.0 / 3.0], o: [0.0, 0.0] });
    }
    fn cubic_to(&mut self, ax: f64, ay: f64, bx: f64, by: f64, x: f64, y: f64) {
        let Some(c) = &mut self.cur else { return };
        let Some(last) = c.v.last_mut() else { return };
        last.o = [ax - last.p[0], ay - last.p[1]];
        c.v.push(Vertex { p: [x, y], i: [bx - x, by - y], o: [0.0, 0.0] });
    }
    fn close(&mut self) {
        if let Some(c) = self.cur.take()
            && c.v.len() > 1
        {
            self.out.push(c);
        }
    }
}

/// Marked content: a layer (optional content with a name), content hidden, or anything else.
enum Marked {
    Layer(Arc<Level>),
    Hidden,
    Other,
}

/// One content stream being run.
struct Frame {
    resources: Dict,
    gs: Gs,
    saved: Vec<Gs>,
    out: Out,
    path: PathBuilder,
    clip_next: Option<bool>,
    tm: Mat3,
    tlm: Mat3,
    text_clip: Vec<Sub>,
    /// Marked content open, and how deep inside hidden content.
    marked: Vec<Marked>,
    hidden: usize,
    /// The space this stream's patterns are placed from.
    base: Mat3,
}

pub struct Runner<'a> {
    pub file: &'a File,
    pub notes: Vec<String>,
    fonts: HashMap<u32, Arc<PdfFont>>,
    off: Vec<u32>,
    ops_left: usize,
    depth: usize,
    /// The page's area in its space (what `sh` with no clip covers).
    pub area: [f64; 4],
}

fn num(o: Option<&Obj>) -> f64 {
    o.and_then(Obj::as_f64).unwrap_or(0.0)
}

impl<'a> Runner<'a> {
    pub fn new(file: &'a File, area: [f64; 4]) -> Runner<'a> {
        // the optional content hidden by default
        let mut off = Vec::new();
        let root = file.trailer.get("Root").map(|r| file.resolve(r)).unwrap_or(Obj::Null);
        if let Some(props) = root.as_dict().and_then(|d| file.at(d, "OCProperties"))
            && let Some(d) = props.as_dict().and_then(|p| file.at(p, "D"))
            && let Some(d) = d.as_dict()
        {
            let refs = |k: &str| -> Vec<u32> {
                file.at(d, k).and_then(|a| a.as_array().map(|a| a.iter().filter_map(|o| if let Obj::Ref(n, _) = o { Some(*n) } else { None }).collect())).unwrap_or_default()
            };
            if file.at(d, "BaseState").and_then(|b| b.as_name().map(str::to_string)).as_deref() == Some("OFF") {
                let on = refs("ON");
                let all: Vec<u32> = file.at(props.as_dict().expect("a dict"), "OCGs").and_then(|a| a.as_array().map(|a| a.iter().filter_map(|o| if let Obj::Ref(n, _) = o { Some(*n) } else { None }).collect())).unwrap_or_default();
                off = all.into_iter().filter(|n| !on.contains(n)).collect();
            } else {
                off = refs("OFF");
            }
        }
        Runner { file, notes: Vec::new(), fonts: HashMap::new(), off, ops_left: 20_000_000, depth: 0, area }
    }

    /// Whether optional content `o` (a group or membership dictionary, or a reference) shows.
    fn visible(&self, o: &Obj) -> bool {
        let n = if let Obj::Ref(n, _) = o { Some(*n) } else { None };
        if n.is_some_and(|n| self.off.contains(&n)) {
            return false;
        }
        let d = self.file.resolve(o);
        let Some(d) = d.as_dict() else { return true };
        if d.get("Type").and_then(Obj::as_name) == Some("OCMD") {
            let groups: Vec<Obj> = match d.get("OCGs").map(|g| (g.clone(), self.file.resolve(g))) {
                Some((_, Obj::Array(a))) => a,
                Some((r, _)) => vec![r],
                None => return true,
            };
            let on: Vec<bool> = groups.iter().map(|g| !matches!(g, Obj::Ref(n, _) if self.off.contains(n))).collect();
            return match self.file.at(d, "P").and_then(|p| p.as_name().map(str::to_string)).as_deref() {
                Some("AllOn") => on.iter().all(|&v| v),
                Some("AnyOff") => on.iter().any(|&v| !v),
                Some("AllOff") => on.iter().all(|&v| !v),
                _ => on.iter().any(|&v| v),
            };
        }
        true
    }

    /// The nodes of content `data` run with `resources`, from state `gs`; patterns placed from
    /// `base`.
    fn run(&mut self, data: &[u8], resources: &Dict, gs: Gs, base: Mat3) -> Vec<Node> {
        if self.depth > 24 {
            self.notes.push("content nested too deep".into());
            return Vec::new();
        }
        self.depth += 1;
        let mut f = Frame { resources: resources.clone(), gs, saved: Vec::new(), out: Out::new(), path: PathBuilder::default(), clip_next: None, tm: Mat3::IDENTITY, tlm: Mat3::IDENTITY, text_clip: Vec::new(), marked: Vec::new(), hidden: 0, base };
        for op in ops(data) {
            if self.ops_left == 0 {
                self.notes.push("more content than is read".into());
                break;
            }
            self.ops_left -= 1;
            self.op(&mut f, &op);
        }
        self.depth -= 1;
        f.out.finish()
    }

    /// The page's nodes.
    pub fn page(&mut self, data: &[u8], resources: &Dict) -> Vec<Node> {
        self.run(data, resources, Gs::new(Mat3::IDENTITY), Mat3::IDENTITY)
    }

    fn chain(f: &Frame) -> Vec<Arc<Level>> {
        let mut c: Vec<Arc<Level>> = f.marked.iter().filter_map(|m| if let Marked::Layer(l) = m { Some(l.clone()) } else { None }).collect();
        c.extend(f.gs.clips.iter().cloned());
        if let Some(m) = &f.gs.smask {
            c.push(m.clone());
        }
        c
    }

    fn emit(f: &mut Frame, node: Node) {
        if f.hidden > 0 {
            return;
        }
        let chain = Runner::chain(f);
        f.out.emit(node, &chain);
    }

    fn resource(&self, f: &Frame, kind: &str, name: &str) -> Option<Obj> {
        let group = self.file.at(&f.resources, kind)?;
        group.as_dict()?.get(name).cloned()
    }

    fn op(&mut self, f: &mut Frame, op: &Op) {
        let a = &op.args;
        let n = |i: usize| num(a.get(i));
        match op.name.as_str() {
            "q" => {
                if f.saved.len() < 256 {
                    f.saved.push(f.gs.clone());
                }
            }
            "Q" => {
                if let Some(g) = f.saved.pop() {
                    f.gs = g;
                }
            }
            "cm" if a.len() >= 6 => f.gs.ctm = f.gs.ctm * mat([n(0), n(1), n(2), n(3), n(4), n(5)]),
            "w" => f.gs.width = n(0),
            "J" => f.gs.cap = [Cap::Butt, Cap::Round, Cap::Square][(n(0) as usize).min(2)],
            "j" => f.gs.join = [Join::Miter, Join::Round, Join::Bevel][(n(0) as usize).min(2)],
            "M" => f.gs.miter = n(0),
            "d" => {
                f.gs.dash = a.first().and_then(Obj::as_array).map(|d| d.iter().filter_map(Obj::as_f64).collect()).unwrap_or_default();
                f.gs.dash_phase = n(1);
            }
            "gs" => {
                if let Some(name) = a.first().and_then(Obj::as_name)
                    && let Some(eg) = self.resource(f, "ExtGState", name).map(|o| self.file.resolve(&o))
                    && let Some(eg) = eg.as_dict()
                {
                    self.ext_gstate(f, eg);
                }
            }
            // paths
            "m" => f.path.move_to([n(0), n(1)]),
            "l" => f.path.line_to([n(0), n(1)]),
            "c" => f.path.curve_to([n(0), n(1)], [n(2), n(3)], [n(4), n(5)]),
            "v" => {
                let at = f.path.at;
                f.path.curve_to(at, [n(0), n(1)], [n(2), n(3)]);
            }
            "y" => f.path.curve_to([n(0), n(1)], [n(2), n(3)], [n(2), n(3)]),
            "h" => f.path.close(),
            "re" => f.path.rect(n(0), n(1), n(2), n(3)),
            "W" => f.clip_next = Some(false),
            "W*" => f.clip_next = Some(true),
            "S" => self.paint(f, false, true, false, false),
            "s" => self.paint(f, false, true, false, true),
            "f" | "F" => self.paint(f, true, false, false, false),
            "f*" => self.paint(f, true, false, true, false),
            "B" => self.paint(f, true, true, false, false),
            "B*" => self.paint(f, true, true, true, false),
            "b" => self.paint(f, true, true, false, true),
            "b*" => self.paint(f, true, true, true, true),
            "n" => self.paint(f, false, false, false, false),
            // colours
            "CS" | "cs" => {
                if f.gs.colour_locked {
                    return;
                }
                let space = Space::read(self.file, a.first().unwrap_or(&Obj::Null), &f.resources);
                let ink = Ink::of(space);
                if op.name == "CS" { f.gs.stroke = ink } else { f.gs.fill = ink }
            }
            "SC" | "SCN" | "sc" | "scn" => {
                if f.gs.colour_locked {
                    return;
                }
                let ink = if op.name.starts_with('S') { &mut f.gs.stroke } else { &mut f.gs.fill };
                ink.comps = a.iter().filter_map(Obj::as_f64).collect();
                ink.pattern = match a.last() {
                    Some(Obj::Name(p)) => self.file.at(&f.resources, "Pattern").and_then(|ps| ps.as_dict().and_then(|d| d.get(p).cloned())),
                    _ => None,
                };
            }
            "G" | "g" | "RG" | "rg" | "K" | "k" => {
                if f.gs.colour_locked {
                    return;
                }
                let space = match op.name.as_str() {
                    "G" | "g" => Space::Gray,
                    "RG" | "rg" => Space::Rgb,
                    _ => Space::Cmyk,
                };
                let ink = Ink { space, comps: a.iter().filter_map(Obj::as_f64).collect(), pattern: None };
                if op.name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) { f.gs.stroke = ink } else { f.gs.fill = ink }
            }
            // text
            "BT" => {
                f.tm = Mat3::IDENTITY;
                f.tlm = Mat3::IDENTITY;
                f.text_clip.clear();
            }
            "ET" => {
                if f.gs.render >= 4 {
                    let subs = std::mem::take(&mut f.text_clip);
                    f.gs.clips.push(Arc::new(Level::Clip(Clip { subs, even_odd: false })));
                }
            }
            "Tc" => f.gs.char_space = n(0),
            "Tw" => f.gs.word_space = n(0),
            "Tz" => f.gs.hscale = n(0) / 100.0,
            "TL" => f.gs.leading = n(0),
            "Ts" => f.gs.rise = n(0),
            "Tr" => f.gs.render = n(0) as i64,
            "Tf" => {
                f.gs.size = n(1);
                if let Some(name) = a.first().and_then(Obj::as_name) {
                    f.gs.font = self.font(f, name);
                }
            }
            "Td" => {
                f.tlm = f.tlm * Mat3::translate(vec2(n(0), n(1)));
                f.tm = f.tlm;
            }
            "TD" => {
                f.gs.leading = -n(1);
                f.tlm = f.tlm * Mat3::translate(vec2(n(0), n(1)));
                f.tm = f.tlm;
            }
            "Tm" if a.len() >= 6 => {
                f.tlm = mat([n(0), n(1), n(2), n(3), n(4), n(5)]);
                f.tm = f.tlm;
            }
            "T*" => {
                f.tlm = f.tlm * Mat3::translate(vec2(0.0, -f.gs.leading));
                f.tm = f.tlm;
            }
            "Tj" => {
                if let Some(s) = a.first().and_then(Obj::as_str) {
                    self.show(f, &[Obj::Str(s.to_vec())]);
                }
            }
            "'" => {
                f.tlm = f.tlm * Mat3::translate(vec2(0.0, -f.gs.leading));
                f.tm = f.tlm;
                if let Some(s) = a.first().and_then(Obj::as_str) {
                    self.show(f, &[Obj::Str(s.to_vec())]);
                }
            }
            "\"" => {
                f.gs.word_space = n(0);
                f.gs.char_space = n(1);
                f.tlm = f.tlm * Mat3::translate(vec2(0.0, -f.gs.leading));
                f.tm = f.tlm;
                if let Some(s) = a.get(2).and_then(Obj::as_str) {
                    self.show(f, &[Obj::Str(s.to_vec())]);
                }
            }
            "TJ" => {
                if let Some(items) = a.first().and_then(Obj::as_array) {
                    self.show(f, items);
                }
            }
            // a Type 3 glyph that takes the text's colour
            "d1" => f.gs.colour_locked = true,
            "Do" => {
                if let Some(name) = a.first().and_then(Obj::as_name)
                    && let Some(x) = self.resource(f, "XObject", name)
                {
                    self.xobject(f, &x);
                }
            }
            "BI" => {
                if let Some((d, bytes)) = &op.inline {
                    self.image(f, d, bytes, true);
                }
            }
            "sh" => {
                if let Some(name) = a.first().and_then(Obj::as_name)
                    && let Some(sh) = self.resource(f, "Shading", name)
                {
                    self.shade(f, &sh);
                }
            }
            "BMC" => f.marked.push(Marked::Other),
            "BDC" => {
                let mut level = Marked::Other;
                if a.first().and_then(Obj::as_name) == Some("OC") {
                    let props = match a.get(1) {
                        Some(Obj::Name(p)) => self.resource(f, "Properties", p),
                        Some(o) => Some(o.clone()),
                        None => None,
                    };
                    if let Some(p) = props {
                        if !self.visible(&p) {
                            f.hidden += 1;
                            f.marked.push(Marked::Hidden);
                            return;
                        }
                        let name = self.file.resolve(&p).as_dict().and_then(|d| self.file.at(d, "Name")).and_then(|n| n.as_str().map(text_string));
                        if let Some(n) = name {
                            level = Marked::Layer(Arc::new(Level::Layer(n)));
                        }
                    }
                }
                f.marked.push(level);
            }
            "EMC" => {
                if let Some(Marked::Hidden) = f.marked.pop() {
                    f.hidden = f.hidden.saturating_sub(1);
                }
            }
            _ => {}
        }
    }

    fn ext_gstate(&mut self, f: &mut Frame, eg: &Dict) {
        let file = self.file;
        for (k, v) in &eg.0 {
            let v = file.resolve(v);
            match k.as_str() {
                "LW" => f.gs.width = v.as_f64().unwrap_or(f.gs.width),
                "LC" => f.gs.cap = [Cap::Butt, Cap::Round, Cap::Square][(v.as_i64().unwrap_or(0).clamp(0, 2)) as usize],
                "LJ" => f.gs.join = [Join::Miter, Join::Round, Join::Bevel][(v.as_i64().unwrap_or(0).clamp(0, 2)) as usize],
                "ML" => f.gs.miter = v.as_f64().unwrap_or(f.gs.miter),
                "D" => {
                    if let Some(d) = v.as_array() {
                        f.gs.dash = d.first().and_then(Obj::as_array).map(|x| x.iter().filter_map(Obj::as_f64).collect()).unwrap_or_default();
                        f.gs.dash_phase = num(d.get(1));
                    }
                }
                "CA" => f.gs.stroke_alpha = v.as_f64().unwrap_or(1.0).clamp(0.0, 1.0),
                "ca" => f.gs.fill_alpha = v.as_f64().unwrap_or(1.0).clamp(0.0, 1.0),
                "BM" => {
                    let name = match &v {
                        Obj::Name(n) => n.clone(),
                        Obj::Array(a) => a.first().and_then(Obj::as_name).unwrap_or("Normal").to_string(),
                        _ => "Normal".into(),
                    };
                    f.gs.blend = blend_of(&name);
                }
                "Font" => {
                    if let Some(arr) = v.as_array() {
                        f.gs.size = num(arr.get(1));
                        if let Some(Obj::Ref(n, _)) = arr.first() {
                            let d = file.get(*n);
                            if let Some(d) = d.as_dict() {
                                let font = self.fonts.entry(*n).or_insert_with(|| Arc::new(PdfFont::read(file, d, &mut self.notes))).clone();
                                f.gs.font = Some(font);
                            }
                        }
                    }
                }
                "SMask" => {
                    f.gs.smask = match &v {
                        Obj::Dict(d) => self.soft_mask(f, d).map(|m| Arc::new(Level::Mask(m))),
                        _ => None,
                    };
                }
                _ => {}
            }
        }
    }

    /// A soft mask: its group drawn (where the state places it now), measured by its alpha or
    /// brightness over its backdrop, through its transfer function.
    fn soft_mask(&mut self, f: &Frame, d: &Dict) -> Option<Mask> {
        let file = self.file;
        let luminosity = file.at(d, "S").and_then(|s| s.as_name().map(str::to_string)).as_deref() != Some("Alpha");
        let Obj::Stream(g) = file.at(d, "G")? else { return None };
        let space = g.dict.get("Group").map(|gr| file.resolve(gr)).and_then(|gr| gr.as_dict().and_then(|gd| gd.get("CS").cloned())).map(|cs| Space::read(file, &cs, &f.resources)).unwrap_or(Space::Gray);
        let bc: Vec<f64> = file.at(d, "BC").and_then(|b| b.as_array().map(|a| a.iter().filter_map(Obj::as_f64).collect())).unwrap_or_else(|| vec![0.0; space.components()]);
        let backdrop = space.rgb(&bc).unwrap_or([0.0; 3]);
        let transfer = file.at(d, "TR").filter(|t| t.as_name() != Some("Identity")).and_then(|t| Function::read(file, &t)).map(|func| (0..256).map(|i| func.eval(&[i as f64 / 255.0]).first().copied().unwrap_or(0.0)).collect());
        let nodes = self.form_nodes(f, &g, Gs::new(f.gs.ctm));
        Some(Mask { content: Group::new("Mask", Mat3::IDENTITY, 1.0, nodes), luminosity, backdrop, transfer })
    }

    /// A form's content as nodes, from state `gs` (its matrix added, cut to its box).
    fn form_nodes(&mut self, f: &Frame, s: &crate::object::Stream, mut gs: Gs) -> Vec<Node> {
        let file = self.file;
        let Ok(dec) = file.decoded(s) else { return Vec::new() };
        if let Some(m) = mat_of(file.at(&s.dict, "Matrix")) {
            gs.ctm = gs.ctm * m;
        }
        let resources = file.at(&s.dict, "Resources").and_then(|r| r.as_dict().cloned()).unwrap_or_else(|| f.resources.clone());
        let ctm = gs.ctm;
        let nodes = self.run(&dec.data, &resources, gs, ctm);
        match rect(file.at(&s.dict, "BBox").as_ref()) {
            Some(b) => {
                let sub = Sub::polygon(&[[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]], true).map(&|p| {
                    let q = ctm.apply(vec2(p[0], p[1]));
                    [q.x, q.y]
                });
                vec![Node::Group(Group { clip: vec![Clip { subs: vec![sub], even_odd: false }], ..Group::new("Form", Mat3::IDENTITY, 1.0, nodes) })]
            }
            None => nodes,
        }
    }

    fn xobject(&mut self, f: &mut Frame, o: &Obj) {
        let file = self.file;
        let Obj::Stream(s) = file.resolve(o) else { return };
        if let Some(oc) = s.dict.get("OC")
            && !self.visible(oc)
        {
            return;
        }
        match s.dict.get("Subtype").and_then(Obj::as_name) {
            Some("Image") => {
                let Some(raw) = file.unlocked(&s) else { return };
                let d = s.dict.clone();
                self.image(f, &d, &raw, false);
            }
            Some("Form") => {
                let group = s.dict.get("Group").map(|g| file.resolve(g)).and_then(|g| g.as_dict().cloned()).filter(|g| g.get("S").and_then(Obj::as_name) == Some("Transparency"));
                if group.is_some() {
                    // a transparency group: drawn on its own, then laid on with the state's
                    // opacity, blend mode and mask
                    let mut gs = f.gs.clone();
                    (gs.fill_alpha, gs.stroke_alpha, gs.blend, gs.smask, gs.clips) = (1.0, 1.0, BlendMode::Normal, None, Vec::new());
                    let nodes = self.form_nodes(f, &s, gs);
                    let g = Group { blend: f.gs.blend, ..Group::new("Group", Mat3::IDENTITY, f.gs.fill_alpha, nodes) };
                    Runner::emit(f, Node::Group(g));
                } else {
                    let mut gs = f.gs.clone();
                    (gs.smask, gs.clips) = (None, Vec::new());
                    let nodes = self.form_nodes(f, &s, gs);
                    let g = Group::new("Form", Mat3::IDENTITY, 1.0, nodes);
                    Runner::emit(f, Node::Group(g));
                }
            }
            _ => {}
        }
    }

    fn image(&mut self, f: &mut Frame, d: &Dict, raw: &[u8], inline: bool) {
        if let Some(oc) = d.get("OC")
            && !self.visible(oc)
        {
            return;
        }
        let fill = f.gs.fill.rgb().map_or([0.0, 0.0, 0.0, 0.0], |c| [c[0], c[1], c[2], 1.0]);
        let Some((img, smooth)) = crate::image::picture(self.file, d, raw, &f.resources, inline, fill, &mut self.notes) else { return };
        let (w, h) = (img.width as f64, img.height as f64);
        // its pixels (from its top left) to the unit square, then to the page
        let to_unit = Mat3([[1.0 / w, 0.0, 0.0], [0.0, -1.0 / h, 1.0], [0.0, 0.0, 1.0]]);
        let p = Picture { name: "Picture".into(), image: img, transform: f.gs.ctm * to_unit, opacity: f.gs.fill_alpha, smooth, blend: f.gs.blend };
        Runner::emit(f, Node::Picture(Box::new(p)));
    }

    /// The page's area in user space (a shading with no clip fills it).
    fn area_in_user(&self, f: &Frame) -> Vec<Sub> {
        let a = self.area;
        let Some(inv) = f.gs.ctm.inverse() else { return Vec::new() };
        let corners = [[a[0], a[1]], [a[2], a[1]], [a[2], a[3]], [a[0], a[3]]].map(|p| {
            let q = inv.apply(vec2(p[0], p[1]));
            [q.x, q.y]
        });
        vec![Sub::polygon(&corners, true)]
    }

    fn shade(&mut self, f: &mut Frame, o: &Obj) {
        let file = self.file;
        let Some(sh) = Shading::read(file, o, &f.resources) else { return };
        let node = match sh.kind {
            2 | 3 => {
                let Some(g) = sh.gradient(file) else { return };
                let shape = Shape { name: "Shading".into(), geom: Geom::Path(self.area_in_user(f)), transform: f.gs.ctm, fill: Some(Paint::Gradient(g)), fill_opacity: f.gs.fill_alpha, even_odd: false, stroke: None, opacity: 1.0, blend: f.gs.blend };
                Node::Shape(Box::new(shape))
            }
            _ => {
                let tris = sh.triangles(file);
                if tris.is_empty() {
                    return;
                }
                Node::Mesh(Box::new(Mesh { name: "Mesh".into(), triangles: tris, transform: f.gs.ctm, opacity: f.gs.fill_alpha, blend: f.gs.blend }))
            }
        };
        let node = match sh.bbox {
            Some(b) => {
                let ctm = f.gs.ctm;
                let sub = Sub::polygon(&[[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]], true).map(&|p| {
                    let q = ctm.apply(vec2(p[0], p[1]));
                    [q.x, q.y]
                });
                Node::Group(Group { clip: vec![Clip { subs: vec![sub], even_odd: false }], ..Group::new("Shading", Mat3::IDENTITY, 1.0, vec![node]) })
            }
            None => node,
        };
        Runner::emit(f, node);
    }

    /// A path painted (filled, stroked, both, or neither) and, when asked, made a clipping path.
    fn paint(&mut self, f: &mut Frame, fill: bool, stroke: bool, even_odd: bool, close: bool) {
        if close {
            f.path.close();
        }
        let subs = f.path.take();
        let clip = f.clip_next.take();
        if !subs.is_empty() && (fill || stroke) {
            self.draw_path(f, &subs, fill, stroke, even_odd, "Path");
        }
        if let Some(eo) = clip {
            let ctm = f.gs.ctm;
            let page: Vec<Sub> = subs.iter().map(|s| s.map(&|p| {
                let q = ctm.apply(vec2(p[0], p[1]));
                [q.x, q.y]
            })).collect();
            f.gs.clips.push(Arc::new(Level::Clip(Clip { subs: page, even_odd: eo })));
        }
    }

    /// A paint for an ink: a colour, or a gradient (an axial or radial shading pattern, placed
    /// in the shape's units); none for nothing, or for a pattern drawn as a region instead.
    fn paint_of(&mut self, f: &Frame, ink: &Ink) -> (Option<Paint>, Option<Obj>) {
        if let Some(p) = &ink.pattern {
            let pat = self.file.resolve(p);
            let Some(pd) = pat.as_dict() else { return (None, None) };
            if self.file.at(pd, "PatternType").and_then(|t| t.as_i64()) == Some(2)
                && let Some(sh) = pd.get("Shading").and_then(|s| Shading::read(self.file, s, &f.resources))
                && matches!(sh.kind, 2 | 3)
                && let Some(mut g) = sh.gradient(self.file)
            {
                let pm = mat_of(self.file.at(pd, "Matrix")).unwrap_or(Mat3::IDENTITY);
                g.transform = f.gs.ctm.inverse().unwrap_or(Mat3::IDENTITY) * f.base * pm;
                return (Some(Paint::Gradient(g)), None);
            }
            return (None, Some(pat));
        }
        if matches!(ink.space, Space::Pattern(_)) {
            return (None, None);
        }
        (ink.rgb().map(Paint::Color), None)
    }

    fn draw_path(&mut self, f: &mut Frame, subs: &[Sub], fill: bool, stroke: bool, even_odd: bool, name: &str) {
        let (fill_paint, fill_pattern) = if fill { self.paint_of(f, &f.gs.fill.clone()) } else { (None, None) };
        let (stroke_paint, stroke_pattern) = if stroke { self.paint_of(f, &f.gs.stroke.clone()) } else { (None, None) };
        let ctm = f.gs.ctm;
        // a stroke as thin as can be drawn: a point of the page
        let width = if f.gs.width <= 0.0 { 1.0 / ctm_scale(&ctm).max(1e-9) } else { f.gs.width };
        let dashes = if f.gs.dash.iter().all(|&d| d <= 0.0) { Vec::new() } else { f.gs.dash.clone() };
        let stroke_of = |paint: Paint, f: &Frame| Stroke { paint, opacity: f.gs.stroke_alpha, width, cap: f.gs.cap, join: f.gs.join, miter: f.gs.miter, dashes: dashes.clone(), dash_offset: f.gs.dash_phase };
        // a pattern of tiles or a mesh: drawn inside the path (or the stroke's outline)
        if let Some(p) = fill_pattern {
            let region: Vec<Sub> = subs.iter().map(|s| s.map(&|q| {
                let r = ctm.apply(vec2(q[0], q[1]));
                [r.x, r.y]
            })).collect();
            self.pattern_region(f, &p, region, even_odd, f.gs.fill.clone(), f.gs.fill_alpha);
        }
        if let Some(p) = stroke_pattern {
            let k = ctm_scale(&ctm).max(1e-9);
            let scaled: Vec<Sub> = subs.iter().map(|s| s.map(&|q| [q[0] * k, q[1] * k])).collect();
            let outline = veymelo_path::stroke(&scaled, &veymelo_path::Stroke { width: width * k, cap: f.gs.cap, join: f.gs.join, miter_limit: f.gs.miter, dashes: dashes.iter().map(|d| d * k).collect(), dash_offset: f.gs.dash_phase * k, ..veymelo_path::Stroke::default() });
            let region: Vec<Sub> = outline
                .contours()
                .iter()
                .map(|c| {
                    let pts: Vec<[f64; 2]> = c.iter().map(|&(x, y)| {
                        let r = ctm.apply(vec2(x / k, y / k));
                        [r.x, r.y]
                    }).collect();
                    Sub::polygon(&pts, true)
                })
                .collect();
            self.pattern_region(f, &p, region, false, f.gs.stroke.clone(), f.gs.stroke_alpha);
        }
        if fill_paint.is_none() && stroke_paint.is_none() {
            return;
        }
        let shape = Shape {
            name: name.to_string(),
            geom: Geom::Path(subs.to_vec()),
            transform: ctm,
            fill: fill_paint,
            fill_opacity: f.gs.fill_alpha,
            even_odd,
            stroke: stroke_paint.map(|p| stroke_of(p, f)),
            opacity: 1.0,
            blend: f.gs.blend,
        };
        Runner::emit(f, Node::Shape(Box::new(shape)));
    }

    /// A tiling or shading pattern drawn inside `region` (in the page's space).
    fn pattern_region(&mut self, f: &mut Frame, pat: &Obj, region: Vec<Sub>, even_odd: bool, ink: Ink, alpha: f64) {
        let file = self.file;
        let Some(pd) = pat.as_dict().cloned() else { return };
        let pm = f.base * mat_of(file.at(&pd, "Matrix")).unwrap_or(Mat3::IDENTITY);
        let mut inside: Vec<Node> = Vec::new();
        match file.at(&pd, "PatternType").and_then(|t| t.as_i64()) {
            Some(2) => {
                let Some(sh) = pd.get("Shading").and_then(|s| Shading::read(file, s, &f.resources)) else { return };
                let tris = sh.triangles(file);
                inside.push(Node::Mesh(Box::new(Mesh { name: "Mesh".into(), triangles: tris, transform: pm, opacity: alpha, blend: f.gs.blend })));
            }
            Some(1) => {
                let Obj::Stream(s) = pat.clone() else { return };
                let (xs, ys) = (file.at(&pd, "XStep").and_then(|v| v.as_f64()).unwrap_or(1.0), file.at(&pd, "YStep").and_then(|v| v.as_f64()).unwrap_or(1.0));
                let Some(bbox) = rect(file.at(&pd, "BBox").as_ref()) else { return };
                if xs.abs() < 1e-9 || ys.abs() < 1e-9 {
                    return;
                }
                // the cell's content once, in pattern space
                let mut gs = Gs::new(pm);
                let uncoloured = file.at(&pd, "PaintType").and_then(|t| t.as_i64()) == Some(2);
                if uncoloured {
                    gs.fill = Ink { space: match &ink.space {
                        Space::Pattern(Some(u)) => (**u).clone(),
                        _ => Space::Gray,
                    }, comps: ink.comps.clone(), pattern: None };
                    gs.stroke = gs.fill.clone();
                    gs.colour_locked = true;
                }
                let Ok(dec) = file.decoded(&s) else { return };
                let resources = file.at(&pd, "Resources").and_then(|r| r.as_dict().cloned()).unwrap_or_default();
                let cell = self.run(&dec.data, &resources, gs, pm);
                if cell.is_empty() {
                    return;
                }
                // the tiles over the region's box
                let Some(inv) = pm.inverse() else { return };
                let mut lo = [f64::INFINITY; 2];
                let mut hi = [f64::NEG_INFINITY; 2];
                for s in &region {
                    for v in &s.v {
                        let q = inv.apply(vec2(v.p[0], v.p[1]));
                        lo = [lo[0].min(q.x), lo[1].min(q.y)];
                        hi = [hi[0].max(q.x), hi[1].max(q.y)];
                    }
                }
                let (i0, i1) = (((lo[0] - bbox[2]) / xs.abs()).floor() as i64, ((hi[0] - bbox[0]) / xs.abs()).ceil() as i64);
                let (j0, j1) = (((lo[1] - bbox[3]) / ys.abs()).floor() as i64, ((hi[1] - bbox[1]) / ys.abs()).ceil() as i64);
                let count = (i1 - i0 + 1).max(0) * (j1 - j0 + 1).max(0);
                if count > 4000 {
                    self.notes.push("a tiling pattern of too many tiles (left out)".into());
                    return;
                }
                let cell_box = Sub::polygon(&[[bbox[0], bbox[1]], [bbox[2], bbox[1]], [bbox[2], bbox[3]], [bbox[0], bbox[3]]], true).map(&|p| {
                    let q = pm.apply(vec2(p[0], p[1]));
                    [q.x, q.y]
                });
                for j in j0..=j1 {
                    for i in i0..=i1 {
                        let shift = pm * Mat3::translate(vec2(i as f64 * xs.abs(), j as f64 * ys.abs())) * inv;
                        let g = Group { clip: vec![Clip { subs: vec![cell_box.clone()], even_odd: false }], ..Group::new("Tile", shift, 1.0, cell.clone()) };
                        inside.push(Node::Group(g));
                    }
                }
            }
            _ => return,
        }
        let g = Group { clip: vec![Clip { subs: region, even_odd }], blend: f.gs.blend, ..Group::new("Pattern", Mat3::IDENTITY, alpha, inside) };
        Runner::emit(f, Node::Group(g));
    }

    fn font(&mut self, f: &Frame, name: &str) -> Option<Arc<PdfFont>> {
        let o = self.resource(f, "Font", name)?;
        let key = match &o {
            Obj::Ref(n, _) => Some(*n),
            _ => None,
        };
        if let Some(k) = key
            && let Some(font) = self.fonts.get(&k)
        {
            return Some(font.clone());
        }
        let d = self.file.resolve(&o);
        let font = Arc::new(PdfFont::read(self.file, d.as_dict()?, &mut self.notes));
        if let Some(k) = key {
            self.fonts.insert(k, font.clone());
        }
        Some(font)
    }

    /// Strings (and spacing numbers) shown in the current font, each glyph placed by the text
    /// matrix, which moves on by each glyph's width.
    fn show(&mut self, f: &mut Frame, items: &[Obj]) {
        let Some(font) = f.gs.font.clone() else { return };
        let gs = f.gs.clone();
        let (size, h) = (gs.size, gs.hscale);
        let mut subs: Vec<Sub> = Vec::new();
        let mut text = String::new();
        for item in items {
            match item {
                Obj::Str(s) => {
                    for (code, len) in font.codes(s) {
                        let w = font.width(code);
                        // glyph space (an em of 1) to user space
                        let (advance_v, origin) = if font.vertical { font.vertical(code) } else { (0.0, [0.0, 0.0]) };
                        let g = f.tm * Mat3([[size * h, 0.0, -origin[0] * size * h], [0.0, size, gs.rise - origin[1] * size], [0.0, 0.0, 1.0]]);
                        if let Outlines::Type3 { procs, matrix, resources } = &font.outlines {
                            if let Some(proc_name) = font.type3_proc(code)
                                && let Some(Obj::Stream(p)) = procs.get(proc_name).map(|o| self.file.resolve(o))
                                && let Ok(dec) = self.file.decoded(&p)
                            {
                                let mut g3 = gs.clone();
                                g3.ctm = gs.ctm * g * mat(*matrix);
                                (g3.smask, g3.clips) = (None, Vec::new());
                                let res = resources.clone().unwrap_or_else(|| f.resources.clone());
                                let base = g3.ctm;
                                let nodes = self.run(&dec.data, &res, g3, base);
                                if !nodes.is_empty() && gs.render != 3 {
                                    Runner::emit(f, Node::Group(Group::new("Glyph", Mat3::IDENTITY, 1.0, nodes)));
                                }
                            }
                        } else if gs.render != 3 {
                            let mut pen = SubPen { out: &mut subs, cur: None };
                            font.outline(code, &mut pen, &|x, y| {
                                let q = g.apply(vec2(x, y));
                                (q.x, q.y)
                            });
                            if let Some(c) = pen.cur.take()
                                && c.v.len() > 1
                            {
                                subs.push(c);
                            }
                        }
                        text.push_str(&font.text(code));
                        let spacing = gs.char_space + if len == 1 && code == 32 { gs.word_space } else { 0.0 };
                        if font.vertical {
                            f.tm = f.tm * Mat3::translate(vec2(0.0, advance_v * size + spacing));
                        } else {
                            f.tm = f.tm * Mat3::translate(vec2((w * size + spacing) * h, 0.0));
                        }
                    }
                }
                other => {
                    let adj = other.as_f64().unwrap_or(0.0) / 1000.0 * size;
                    if font.vertical {
                        f.tm = f.tm * Mat3::translate(vec2(0.0, -adj));
                    } else {
                        f.tm = f.tm * Mat3::translate(vec2(-adj * h, 0.0));
                    }
                }
            }
        }
        if subs.is_empty() {
            return;
        }
        let mode = gs.render;
        if mode >= 4 {
            let ctm = gs.ctm;
            f.text_clip.extend(subs.iter().map(|s| s.map(&|p| {
                let q = ctm.apply(vec2(p[0], p[1]));
                [q.x, q.y]
            })));
        }
        let (fill, stroke) = match mode % 4 {
            0 => (true, false),
            1 => (false, true),
            2 => (true, true),
            _ => (false, false),
        };
        if !fill && !stroke {
            return;
        }
        // the text it shows names it
        let name: String = text.chars().filter(|c| !c.is_control()).take(60).collect();
        let name = if name.trim().is_empty() { "Text".to_string() } else { name };
        self.draw_path(f, &subs, fill, stroke, false, &name);
    }
}

/// How much a matrix scales lengths, on average.
fn ctm_scale(m: &Mat3) -> f64 {
    (m.0[0][0] * m.0[1][1] - m.0[0][1] * m.0[1][0]).abs().sqrt()
}

/// A PDF text string: UTF-16 with its mark, UTF-8 with its mark, else PDFDocEncoding (Latin-1 near
/// enough).
pub fn text_string(s: &[u8]) -> String {
    if s.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = s[2..].chunks(2).map(|c| (c[0] as u16) << 8 | *c.get(1).unwrap_or(&0) as u16).collect();
        return String::from_utf16_lossy(&units);
    }
    if s.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&s[3..]).into_owned();
    }
    s.iter().map(|&b| b as char).collect()
}

fn blend_of(name: &str) -> BlendMode {
    match name {
        "Multiply" => BlendMode::Multiply,
        "Screen" => BlendMode::Screen,
        "Overlay" => BlendMode::Overlay,
        "Darken" => BlendMode::Darken,
        "Lighten" => BlendMode::Lighten,
        "ColorDodge" => BlendMode::ColorDodge,
        "ColorBurn" => BlendMode::ColorBurn,
        "HardLight" => BlendMode::HardLight,
        "SoftLight" => BlendMode::SoftLight,
        "Difference" => BlendMode::Difference,
        "Exclusion" => BlendMode::Exclusion,
        "Hue" => BlendMode::Hue,
        "Saturation" => BlendMode::Saturation,
        "Color" => BlendMode::Color,
        "Luminosity" => BlendMode::Luminosity,
        _ => BlendMode::Normal,
    }
}

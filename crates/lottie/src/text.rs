//! Text layers and Lottie's text (`t`: the documents `d` keyed in time, animators `a`, more
//! options `m`) both ways, and text as the outlines of its letters.

use veymelo_font::Pen;
use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::{Caps, Justify, Keyframe, ShapePath, TextDoc, Value};
use veymelo_project::{Group, Layer, Node};

use crate::values::{self, Ctx, color_of, num, nums, path_json, prop_from, prop_json, same, vec2_of};

/// Lottie's justifications, in its order.
const JUSTIFY: [Justify; 7] = [Justify::Left, Justify::Right, Justify::Center, Justify::JustifyLeft, Justify::JustifyRight, Justify::JustifyCenter, Justify::JustifyAll];

const WEIGHTS: [(u32, &str); 9] = [(100, "Thin"), (200, "ExtraLight"), (300, "Light"), (400, "Regular"), (500, "Medium"), (600, "SemiBold"), (700, "Bold"), (800, "ExtraBold"), (900, "Black")];

/// A font's style name for its weight and slant ("Bold Italic").
fn style_of(weight: f64, italic: bool) -> String {
    let w = WEIGHTS.iter().min_by_key(|(n, _)| (*n as f64 - weight).abs() as u32).map_or("Regular", |(_, s)| s);
    match (w, italic) {
        ("Regular", true) => "Italic".into(),
        (w, true) => format!("{w} Italic"),
        (w, false) => w.into(),
    }
}

/// A style name's weight and slant.
fn weight_of(style: &str) -> (f64, bool) {
    let flat: String = style.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
    let italic = flat.contains("italic") || flat.contains("oblique");
    // the longest name found wins ("extrabold" over "bold")
    let w = WEIGHTS.iter().filter(|(_, s)| flat.contains(&s.to_lowercase())).max_by_key(|(_, s)| s.len()).map_or(400, |(n, _)| *n);
    let w = if flat.contains("semi") || flat.contains("demi") { 600 } else if flat.contains("heavy") { 800 } else { w };
    (w as f64, italic)
}

/// The family a document's font names: itself, or for a font file, the family inside it.
fn family_of(d: &TextDoc) -> String {
    let lower = d.font.to_lowercase();
    if (d.font.contains('/') || lower.ends_with(".ttf") || lower.ends_with(".otf") || lower.ends_with(".ttc"))
        && let Some(f) = veymelo_text::resolve(&d.font, d.weight, d.italic)
        && let Some(name) = f.name(1)
    {
        return name;
    }
    d.font.clone()
}

/// The name a document's font goes by in the document (`fName`).
fn font_key(d: &TextDoc) -> String {
    format!("{}-{}", family_of(d).replace(' ', ""), style_of(d.weight, d.italic).replace(' ', ""))
}

fn rgb(c: veymelo_color::Rgba) -> Json {
    nums(&[c.r, c.g, c.b])
}

fn doc_json(d: &TextDoc) -> Json {
    let mut o = Map::new();
    o.insert("t", d.text.replace("\r\n", "\r").replace('\n', "\r"));
    o.insert("f", font_key(d));
    o.insert("s", num(d.size));
    o.insert("j", JUSTIFY.iter().position(|j| *j == d.justify).unwrap_or(0) as f64);
    o.insert("tr", num(d.tracking));
    o.insert("lh", num(d.line_height()));
    o.insert("ls", num(d.baseline_shift));
    if let Some(c) = d.fill {
        o.insert("fc", rgb(c));
    }
    if let Some(c) = d.stroke.filter(|_| d.stroke_width > 0.0) {
        o.insert("sc", rgb(c));
        o.insert("sw", num(d.stroke_width));
        o.insert("of", d.stroke_over);
    }
    match d.caps {
        Caps::AllCaps => o.insert("ca", 1.0),
        Caps::SmallCaps => o.insert("ca", 2.0),
        Caps::Normal => None,
    };
    if let Some(b) = d.box_size {
        o.insert("sz", nums(&b));
        o.insert("ps", nums(&[0.0, 0.0]));
    }
    Json::Object(o)
}

fn doc_from(j: &Json, fonts: &[(String, String, String)]) -> TextDoc {
    let mut d = TextDoc::default();
    let f = |k: &str| j.get(k).and_then(Json::as_f64);
    if let Some(t) = j.get("t").and_then(Json::as_str) {
        d.text = t.replace("\r\n", "\n").replace(['\r', '\u{3}'], "\n");
    }
    if let Some(name) = j.get("f").and_then(Json::as_str) {
        let (family, style) = match fonts.iter().find(|(n, _, _)| n == name) {
            Some((_, family, style)) => (family.clone(), style.clone()),
            None => {
                let (family, style) = name.split_once('-').unwrap_or((name, "Regular"));
                (family.to_string(), style.to_string())
            }
        };
        let (weight, italic) = weight_of(&style);
        (d.font, d.weight, d.italic) = (family, weight, italic);
    }
    if let Some(s) = f("s") {
        d.size = s.max(0.0);
    }
    d.fill = j.get("fc").and_then(color_of).map(|v| v.as_color());
    if let Some(c) = j.get("sc").and_then(color_of) {
        d.stroke = Some(c.as_color());
        d.stroke_width = f("sw").unwrap_or(1.0).max(0.0);
        d.stroke_over = j.get("of").and_then(Json::as_bool).unwrap_or(false);
    }
    if let Some(i) = f("j") {
        d.justify = JUSTIFY.get(i.max(0.0) as usize).copied().unwrap_or_default();
    }
    d.tracking = f("tr").unwrap_or(0.0);
    if let Some(lh) = f("lh")
        && (lh - d.size * 1.2).abs() > 1e-6
    {
        d.leading = Some(lh);
    }
    d.baseline_shift = f("ls").unwrap_or(0.0);
    d.caps = match f("ca") {
        Some(1.0) => Caps::AllCaps,
        Some(2.0) => Caps::SmallCaps,
        _ => Caps::Normal,
    };
    if let Some(sz) = j.get("sz").map(values::numbers).filter(|v| v.len() >= 2) {
        d.box_size = Some([sz[0].max(0.0), sz[1].max(0.0)]);
    }
    d
}

/// Our animator properties and their Lottie names.
const ANIMATED: [(&str, &str); 11] = [
    ("a", "anchor"),
    ("p", "position"),
    ("s", "scale"),
    ("sk", "skew"),
    ("sa", "skewAxis"),
    ("r", "rotation"),
    ("o", "opacity"),
    ("fc", "fillColor"),
    ("sc", "strokeColor"),
    ("sw", "strokeWidth"),
    ("t", "tracking"),
];

/// A range selector, or one that picks everything (an animator without selectors).
fn selector_out(cx: &mut Ctx, s: Option<&Group>, label: &str) -> Json {
    let mut o = Map::new();
    o.insert("t", 0.0);
    let still = |v: f64| Json::object([("a", Json::from(0.0)), ("k", num(v))]);
    let Some(s) = s else {
        for (k, v) in [("s", 0.0), ("e", 100.0), ("o", 0.0), ("a", 100.0), ("m", 1.0)] {
            o.insert(k, still(v));
        }
        for (k, v) in [("r", 1.0), ("b", 1.0), ("sh", 1.0), ("rn", 0.0)] {
            o.insert(k, v);
        }
        return Json::Object(o);
    };
    for (k, id) in [("s", "start"), ("e", "end"), ("o", "offset"), ("a", "amount"), ("xe", "easeHigh"), ("ne", "easeLow"), ("m", "mode")] {
        if let Ok(p) = s.prop(id) {
            let conv: &dyn Fn(&Value) -> Value = if k == "m" { &values::from_one } else { &same };
            let v = prop_json(cx, p, conv, &format!("{label} › {}", p.def.name));
            o.insert(k, v);
        }
    }
    let choice = |id: &str| s.prop(id).map_or(0, |p| p.value.as_choice()) as f64 + 1.0;
    o.insert("r", choice("units"));
    o.insert("b", choice("basedOn"));
    o.insert("sh", choice("shape"));
    let random = s.prop("randomize").is_ok_and(|p| p.value.as_bool());
    o.insert("rn", random as u8 as f64);
    if random {
        cx.warn(format!("{label}: Lottie players pick their own random order"));
    }
    Json::Object(o)
}

fn animator_out(cx: &mut Ctx, a: &Group, label: &str) -> Json {
    let mut props = Map::new();
    for (key, id) in ANIMATED {
        let Ok(p) = a.prop(id) else { continue };
        if p.is_default() {
            continue;
        }
        let v = prop_json(cx, p, &same, &format!("{label} › {}", p.def.name));
        props.insert(key, v);
    }
    let selectors: Vec<&Group> = a.group("selectors").map(|s| s.children.iter().filter_map(Node::as_group).collect()).unwrap_or_default();
    if selectors.len() > 1 {
        cx.warn(format!("{label}: Lottie animators have one selector (the first is kept)"));
    }
    let first = selectors.first().copied();
    if let Some(s) = first.filter(|s| s.def.id != "range") {
        cx.warn(format!("{label}: Lottie has no {} (it picks all the text)", s.name()));
    }
    let sel = selector_out(cx, first.filter(|s| s.def.id == "range"), label);
    Json::object([("nm", Json::from(a.name())), ("s", sel), ("a", Json::Object(props))])
}

/// A text layer's Lottie `t`; the fonts it uses are added to `fonts`.
pub fn text_out(cx: &mut Ctx, l: &Layer, fonts: &mut Vec<Json>) -> Json {
    let Ok(tg) = l.props.group("text") else { return Json::object([("d", Json::object([("k", Json::Array(vec![]))]))]) };
    let label = l.name.as_str();
    let mut add_font = |d: &TextDoc| {
        let key = font_key(d);
        if !fonts.iter().any(|f| f.get("fName").and_then(Json::as_str) == Some(key.as_str())) {
            let ascent = veymelo_text::resolve(&d.font, d.weight, d.italic).map_or(75.0, |f| f.ascent / f.units * 100.0);
            fonts.push(Json::object([("fName", Json::from(key)), ("fFamily", Json::from(family_of(d))), ("fStyle", Json::from(style_of(d.weight, d.italic))), ("ascent", num(ascent))]));
        }
    };
    let mut docs = Vec::new();
    if let Ok(st) = tg.prop("sourceText") {
        if st.keys.is_empty() {
            let d = st.value.doc().unwrap_or_default();
            add_font(&d);
            docs.push(Json::object([("t", Json::from(0.0)), ("s", doc_json(&d))]));
        } else {
            for k in &st.keys {
                let d = k.value.doc().unwrap_or_default();
                add_font(&d);
                docs.push(Json::object([("t", num(cx.frame(k.time))), ("s", doc_json(&d))]));
            }
        }
        if st.expression.as_ref().is_some_and(|e| e.enabled) {
            cx.warn(format!("{label} › Source Text: its expression is left out"));
        }
    }
    let animators: Vec<Json> = tg.group("animators").map(|list| list.children.iter().filter_map(Node::as_group).map(|a| animator_out(cx, a, &format!("{label} › {}", a.name()))).collect()).unwrap_or_default();
    let mut more = Map::new();
    if let Ok(m) = tg.group("moreOptions") {
        more.insert("g", m.prop("anchorGrouping").map_or(0, |p| p.value.as_choice()) as f64 + 1.0);
        if let Ok(p) = m.prop("groupingAlignment") {
            let v = prop_json(cx, p, &same, &format!("{label} › Grouping Alignment"));
            more.insert("a", v);
        }
    }
    Json::object([("d", Json::object([("k", Json::Array(docs))])), ("a", Json::Array(animators)), ("m", Json::Object(more)), ("p", Json::Object(Map::new()))])
}

// ---------------------------------------------------------------- in

/// Lottie's `t` into a text layer's text group.
pub fn text_in(cx: &mut Ctx, t: &Json, tg: &mut Group, fonts: &[(String, String, String)], ids: &mut dyn FnMut() -> u64, label: &str) {
    let docs = t.get("d").and_then(|d| d.get("k")).and_then(Json::as_array).map(Vec::as_slice).unwrap_or(&[]);
    if let Ok(st) = tg.prop_mut("sourceText") {
        let keys: Vec<Keyframe> = docs.iter().filter_map(|k| Some(Keyframe::new(cx.tick(k.get("t").and_then(Json::as_f64).unwrap_or(0.0)), Value::Doc(Box::new(doc_from(k.get("s")?, fonts)))))).collect();
        let result = match keys.len() {
            0 => Ok(()),
            1 => st.set_value(keys[0].value.clone()),
            _ => st.set_keys(keys),
        };
        if result.is_err() {
            cx.warn(format!("{label}: its words could not be read"));
        }
    }
    if let Some(m) = t.get("m")
        && let Ok(more) = tg.group_mut("moreOptions")
    {
        if let Some(g) = m.get("g").and_then(Json::as_f64)
            && let Ok(p) = more.prop_mut("anchorGrouping")
        {
            let _ = p.set_value(Value::Choice((g.round() as u32).clamp(1, 4) - 1));
        }
        if let (Some(a), Ok(p)) = (m.get("a"), more.prop_mut("groupingAlignment")) {
            prop_from(cx, a, p, &vec2_of, label);
        }
    }
    if t.get("p").and_then(Json::as_object).is_some_and(|p| p.contains("m")) {
        cx.warn(format!("{label}: text on a path is not read"));
    }
    let list = t.get("a").and_then(Json::as_array).map(Vec::as_slice).unwrap_or(&[]);
    let Ok(animators) = tg.group_mut("animators") else { return };
    for (n, a) in list.iter().enumerate() {
        let name = a.get("nm").and_then(Json::as_str).filter(|s| !s.is_empty()).map_or_else(|| format!("Animator {}", n + 1), str::to_string);
        let mut g = Group::instance(&veymelo_project::text::ANIMATOR, ids(), &name);
        let label = format!("{label} › {name}");
        if let Some(props) = a.get("a").and_then(Json::as_object) {
            for (key, v) in props.iter() {
                let Some((_, id)) = ANIMATED.iter().find(|(k, _)| *k == key) else {
                    cx.warn(format!("{label}: the animated \"{key}\" is not read"));
                    continue;
                };
                let Ok(p) = g.prop_mut(id) else { continue };
                let conv: &dyn Fn(&Json) -> Option<Value> = match p.value {
                    Value::Vec2(_) => &vec2_of,
                    Value::Color(_) => &color_of,
                    _ => &values::number,
                };
                prop_from(cx, v, p, conv, &label);
            }
        }
        if let Some(s) = a.get("s") {
            if s.get("t").and_then(Json::as_f64).unwrap_or(0.0) != 0.0 {
                cx.warn(format!("{label}: an expression selector is not read (it picks all the text)"));
            } else {
                let mut sel = Group::instance(&veymelo_project::text::RANGE, ids(), "Range Selector 1");
                for (k, id) in [("s", "start"), ("e", "end"), ("o", "offset"), ("a", "amount"), ("xe", "easeHigh"), ("ne", "easeLow")] {
                    if let (Some(v), Ok(p)) = (s.get(k), sel.prop_mut(id)) {
                        prop_from(cx, v, p, &values::number, &label);
                    }
                }
                if let (Some(v), Ok(p)) = (s.get("m"), sel.prop_mut("mode")) {
                    prop_from(cx, v, p, &values::choice_of, &label);
                }
                for (k, id, most) in [("r", "units", 1), ("b", "basedOn", 3), ("sh", "shape", 5)] {
                    if let (Some(v), Ok(p)) = (s.get(k).and_then(Json::as_f64), sel.prop_mut(id)) {
                        let _ = p.set_value(Value::Choice(((v.round() - 1.0).max(0.0) as u32).min(most)));
                    }
                }
                if let (Some(v), Ok(p)) = (s.get("rn").and_then(Json::as_f64), sel.prop_mut("randomize")) {
                    let _ = p.set_value(Value::Bool(v != 0.0));
                }
                if let Ok(list) = g.group_mut("selectors") {
                    list.children.push(Node::Group(sel));
                }
            }
        }
        animators.children.push(Node::Group(g));
    }
}

// ---------------------------------------------------------------- as outlines

/// Outlines kept as Bezier paths, each quadratic curve as the cubic it is.
#[derive(Default)]
struct Curves {
    done: Vec<ShapePath>,
    open: Option<ShapePath>,
}

impl Curves {
    fn finish(&mut self) {
        if let Some(p) = self.open.take().filter(|p| p.points.len() > 1) {
            self.done.push(p);
        }
    }
}

impl Pen for Curves {
    fn move_to(&mut self, x: f64, y: f64) {
        self.finish();
        self.open = Some(ShapePath { points: vec![[x, y]], in_tangents: vec![[0.0; 2]], out_tangents: vec![[0.0; 2]], closed: false, feather: veymelo_keyframe::Feathers::NONE });
    }

    fn line_to(&mut self, x: f64, y: f64) {
        let Some(p) = self.open.as_mut() else { return self.move_to(x, y) };
        p.points.push([x, y]);
        p.in_tangents.push([0.0; 2]);
        p.out_tangents.push([0.0; 2]);
    }

    fn quad_to(&mut self, cx: f64, cy: f64, x: f64, y: f64) {
        let Some(p) = self.open.as_mut() else { return self.move_to(x, y) };
        let (Some(&[x0, y0]), Some(out)) = (p.points.last(), p.out_tangents.last_mut()) else { return };
        // a quadratic's control point, two thirds of the way from each end
        *out = [(cx - x0) * 2.0 / 3.0, (cy - y0) * 2.0 / 3.0];
        p.points.push([x, y]);
        p.in_tangents.push([(cx - x) * 2.0 / 3.0, (cy - y) * 2.0 / 3.0]);
        p.out_tangents.push([0.0; 2]);
    }

    fn cubic_to(&mut self, ax: f64, ay: f64, bx: f64, by: f64, x: f64, y: f64) {
        let Some(p) = self.open.as_mut() else { return self.move_to(x, y) };
        let (Some(&[x0, y0]), Some(out)) = (p.points.last(), p.out_tangents.last_mut()) else { return };
        *out = [ax - x0, ay - y0];
        p.points.push([x, y]);
        p.in_tangents.push([bx - x, by - y]);
        p.out_tangents.push([0.0; 2]);
    }

    fn close(&mut self) {
        if let Some(p) = self.open.as_mut() {
            let n = p.points.len();
            if n > 1 && (p.points[0][0] - p.points[n - 1][0]).abs() < 1e-9 && (p.points[0][1] - p.points[n - 1][1]).abs() < 1e-9 {
                // the last point is the first come round again
                p.in_tangents[0] = p.in_tangents[n - 1];
                p.points.pop();
                p.in_tangents.pop();
                p.out_tangents.pop();
            }
            p.closed = true;
        }
        self.finish();
    }
}

/// A text layer's words, as they are at its start, as shape items: a group of paths for each
/// letter, filled and stroked as the text is.
pub fn outlines(cx: &mut Ctx, l: &Layer) -> Vec<Json> {
    let Ok(tg) = l.props.group("text") else { return Vec::new() };
    let Ok(st) = tg.prop("sourceText") else { return Vec::new() };
    let d = st.value_at(veymelo_time::Tick::ZERO).doc().unwrap_or_default();
    let moving = !st.keys.is_empty() || tg.group("animators").is_ok_and(|a| !a.children.is_empty());
    if moving {
        cx.warn(format!("{}: its letters are drawn as they are at its start (keys of its words and animators are left out)", l.name));
    }
    let laid = veymelo_text::lay_out(&d);
    if laid.no_font {
        cx.warn(format!("{}: the font \"{}\" is not on this computer (another is drawn)", l.name, d.font));
    }
    let chars: Vec<char> = d.text.chars().collect();
    let still = |k: Json| Json::object([("a", Json::from(0.0)), ("k", k)]);
    let mut items = Vec::new();
    // with more than one look, each letter carries its own fill and stroke
    let several = laid.styles.len() > 1;
    let paint_of = |fill: Option<veymelo_color::Rgba>, stroke: Option<veymelo_color::Rgba>, width: f64, over: bool| -> Vec<Json> {
        let fill = fill.map(|c| Json::object([("ty", Json::from("fl")), ("nm", Json::from("Fill")), ("c", still(rgb(c))), ("o", still(num(100.0 * c.a))), ("r", Json::from(1.0))]));
        let stroke = stroke.filter(|_| width > 0.0).map(|c| {
            Json::object([("ty", Json::from("st")), ("nm", Json::from("Stroke")), ("c", still(rgb(c))), ("o", still(num(100.0 * c.a))), ("w", still(num(width))), ("lc", Json::from(1.0)), ("lj", Json::from(1.0)), ("ml", Json::from(4.0))])
        });
        if over { [stroke, fill] } else { [fill, stroke] }.into_iter().flatten().collect()
    };
    for g in &laid.glyphs {
        let mut pen = Curves::default();
        g.font.outline(g.glyph, &mut pen, &|fx, fy| g.map(fx, fy));
        pen.finish();
        if pen.done.is_empty() {
            continue;
        }
        let name = chars.get(g.char_index).map_or_else(String::new, |c| c.to_string());
        let mut it: Vec<Json> = pen.done.iter().map(|p| Json::object([("ty", Json::from("sh")), ("ks", still(path_json(p)))])).collect();
        if several {
            let st = &laid.styles[g.style];
            it.extend(paint_of(st.fill, st.stroke, st.stroke_width, d.stroke_over));
        }
        it.push(Json::object([("ty", Json::from("tr")), ("p", still(nums(&[0.0, 0.0]))), ("a", still(nums(&[0.0, 0.0]))), ("s", still(nums(&[100.0, 100.0]))), ("r", still(num(0.0))), ("o", still(num(100.0)))]));
        items.push(Json::object([("ty", Json::from("gr")), ("nm", Json::from(name)), ("np", Json::from(it.len() as f64)), ("it", Json::Array(it))]));
    }
    if several {
        return items;
    }
    let fill = d.fill.map(|c| Json::object([("ty", Json::from("fl")), ("nm", Json::from("Fill")), ("c", still(rgb(c))), ("o", still(num(100.0 * c.a))), ("r", Json::from(1.0))]));
    let stroke = d.stroke.filter(|_| d.stroke_width > 0.0).map(|c| {
        Json::object([("ty", Json::from("st")), ("nm", Json::from("Stroke")), ("c", still(rgb(c))), ("o", still(num(100.0 * c.a))), ("w", still(num(d.stroke_width))), ("lc", Json::from(1.0)), ("lj", Json::from(1.0)), ("ml", Json::from(4.0))])
    });
    // items listed first are drawn on top
    let paints: Vec<Json> = if d.stroke_over { [stroke, fill] } else { [fill, stroke] }.into_iter().flatten().collect();
    items.extend(paints);
    items
}

// ---------------------------------------------------------------- glyphs carried in the document

/// Lottie's `chars`: each character the documents show, in each font, its outline at 100 units
/// an em (y down, the baseline at 0) and how far it moves the pen, so players need no font.
pub fn chars_out(cx: &mut Ctx, docs: &[TextDoc]) -> Vec<Json> {
    let still = |k: Json| Json::object([("a", Json::from(0.0)), ("k", k)]);
    let mut seen: Vec<(String, char)> = Vec::new();
    let mut out = Vec::new();
    for d in docs {
        let Some(font) = veymelo_text::resolve(&d.font, d.weight, d.italic) else {
            cx.warn(format!("the font \"{}\" is not on this computer: its letters are not carried", d.font));
            continue;
        };
        let key = font_key(d);
        let k = 100.0 / font.units;
        for c in d.text.chars().filter(|c| !matches!(c, '\r' | '\n' | '\u{3}')) {
            if seen.iter().any(|(f, s)| *f == key && *s == c) {
                continue;
            }
            seen.push((key.clone(), c));
            let g = font.glyph(c);
            let mut pen = Curves::default();
            font.outline(g, &mut pen, &|x, y| (x * k, -y * k));
            pen.finish();
            let mut it: Vec<Json> = pen.done.iter().map(|p| Json::object([("ty", Json::from("sh")), ("ks", still(path_json(p)))])).collect();
            it.push(Json::object([("ty", Json::from("tr")), ("p", still(nums(&[0.0, 0.0]))), ("a", still(nums(&[0.0, 0.0]))), ("s", still(nums(&[100.0, 100.0]))), ("r", still(num(0.0))), ("o", still(num(100.0)))]));
            let shape = Json::object([("ty", Json::from("gr")), ("nm", Json::from(c.to_string())), ("np", Json::from(it.len() as f64)), ("it", Json::Array(it))]);
            out.push(Json::object([
                ("ch", Json::from(c.to_string())),
                ("size", num(100.0)),
                ("style", Json::from(style_of(d.weight, d.italic))),
                ("fFamily", Json::from(family_of(d))),
                ("w", num(font.advance(g) * k)),
                ("data", Json::object([("shapes", Json::Array(vec![shape]))])),
            ]));
        }
    }
    out
}

/// A text document's words (as they are at its start) laid out from the glyphs a document
/// carries, as shape items: a group for each letter, moved to its place, then the text's fill
/// and stroke. None when a character it shows has no glyph there.
pub fn letters_from_chars(d: &TextDoc, chars: &[Json]) -> Option<Vec<Json>> {
    let family = family_of(d);
    let style = style_of(d.weight, d.italic);
    let find = |c: char| -> Option<&Json> {
        let mine = |j: &&Json| j.get("ch").and_then(Json::as_str).and_then(|s| s.chars().next()) == Some(c);
        chars.iter().filter(mine).find(|j| j.get("fFamily").and_then(Json::as_str) == Some(family.as_str()) && j.get("style").and_then(Json::as_str) == Some(style.as_str())).or_else(|| chars.iter().find(mine))
    };
    let still = |k: Json| Json::object([("a", Json::from(0.0)), ("k", k)]);
    let leading = d.leading.unwrap_or(d.size * 1.2);
    let lines: Vec<&str> = d.text.split('\n').collect();
    let mut items = Vec::new();
    for (li, line) in lines.iter().enumerate() {
        // the line's letters and their places, then the line moved for its justification
        let mut x = 0.0;
        let mut placed = Vec::new();
        for c in line.chars() {
            let j = find(c)?;
            let k = d.size / j.get("size").and_then(Json::as_f64).unwrap_or(100.0).max(1e-9);
            placed.push((c, j, x, k));
            x += j.get("w").and_then(Json::as_f64).unwrap_or(0.0) * k + d.tracking / 1000.0 * d.size;
        }
        let width = x - if placed.is_empty() { 0.0 } else { d.tracking / 1000.0 * d.size };
        let shift = match d.justify {
            Justify::Right | Justify::JustifyRight => -width,
            Justify::Center | Justify::JustifyCenter => -width / 2.0,
            _ => 0.0,
        };
        for (c, j, x, k) in placed {
            let shapes = j.get("data").and_then(|v| v.get("shapes")).and_then(Json::as_array).cloned().unwrap_or_default();
            if shapes.is_empty() {
                continue;
            }
            let mut it = shapes;
            it.push(Json::object([("ty", Json::from("tr")), ("p", still(nums(&[x + shift, li as f64 * leading - d.baseline_shift]))), ("a", still(nums(&[0.0, 0.0]))), ("s", still(nums(&[100.0 * k, 100.0 * k]))), ("r", still(num(0.0))), ("o", still(num(100.0)))]));
            items.push(Json::object([("ty", Json::from("gr")), ("nm", Json::from(c.to_string())), ("np", Json::from(it.len() as f64)), ("it", Json::Array(it))]));
        }
    }
    let fill = d.fill.map(|c| Json::object([("ty", Json::from("fl")), ("nm", Json::from("Fill")), ("c", still(rgb(c))), ("o", still(num(100.0 * c.a))), ("r", Json::from(1.0))]));
    let stroke = d.stroke.filter(|_| d.stroke_width > 0.0).map(|c| {
        Json::object([("ty", Json::from("st")), ("nm", Json::from("Stroke")), ("c", still(rgb(c))), ("o", still(num(100.0 * c.a))), ("w", still(num(d.stroke_width))), ("lc", Json::from(1.0)), ("lj", Json::from(1.0)), ("ml", Json::from(4.0))])
    });
    items.extend(if d.stroke_over { [stroke, fill] } else { [fill, stroke] }.into_iter().flatten());
    Some(items)
}

/// The document a layer's Lottie `t` shows first.
pub fn first_doc(t: &Json, fonts: &[(String, String, String)]) -> Option<TextDoc> {
    let first = t.get("d")?.get("k")?.as_array()?.first()?;
    Some(doc_from(first.get("s")?, fonts))
}

//! A shape layer's contents and Lottie's shape items, both ways: groups (`gr`, their transform
//! `tr` last), paths (`rc`, `el`, `sr`, `sh`), paints (`fl`, `st`, `gf`, `gs`) and operators
//! (`tm`, `rd`, `op`, `pb`, `tw`, `zz`, `mm`, `rp`). Both list items top first.

use veymelo_color::BlendMode;
use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::Value;
use veymelo_project::shapes as defs;
use veymelo_project::{Group, GroupDef, Node};

use crate::values::{self, Ctx, color_of, from_one, num, path_value, prop_from, prop_json, same, vec2_of};

/// Builds one Lottie item from one of our groups.
struct Out<'a> {
    cx: &'a mut Ctx,
    g: &'a Group,
    o: Map,
    label: String,
}

impl Out<'_> {
    fn put(&mut self, key: &str, id: &str, conv: &dyn Fn(&Value) -> Value) {
        if let Ok(p) = self.g.prop(id) {
            let v = prop_json(self.cx, p, conv, &format!("{} › {}", self.label, p.def.name));
            self.o.insert(key, v);
        }
    }

    fn raw(&mut self, key: &str, v: impl Into<Json>) {
        self.o.insert(key, v);
    }

    fn choice(&self, id: &str) -> u32 {
        self.g.prop(id).map_or(0, |p| p.value.as_choice())
    }

    fn number(&self, id: &str) -> f64 {
        self.g.prop(id).map_or(0.0, |p| p.value.as_f64())
    }

    fn moves(&self, id: &str) -> bool {
        self.g.prop(id).is_ok_and(|p| p.is_animated() || p.expression.is_some())
    }

    fn blend(&mut self) {
        let m = BlendMode::ALL.get(self.choice("blend") as usize).copied().unwrap_or(BlendMode::Normal);
        let bm = crate::blend_out(m).unwrap_or_else(|| {
            self.cx.warn(format!("{}: Lottie has no {} blend mode (normal is used)", self.label, m.id()));
            0
        });
        self.raw("bm", bm as f64);
    }

    fn above(&mut self) {
        if self.choice("composite") == 1 {
            self.cx.warn(format!("{}: drawn above the paints above it, which Lottie cannot say (drawn below)", self.label));
        }
    }

    /// A colour and opacity: Lottie keeps no alpha in the colour, so a still colour's alpha
    /// goes into a still opacity.
    fn paint(&mut self) {
        let color = self.g.prop("color").ok();
        let alpha = color.filter(|c| c.keys.is_empty()).map_or(1.0, |c| c.value.as_color().a);
        self.put("c", "color", &same);
        if alpha < 1.0 {
            if self.moves("opacity") {
                self.cx.warn(format!("{}: its colour's alpha is left out (its opacity is animated)", self.label));
                self.put("o", "opacity", &same);
            } else {
                self.raw("o", Json::object([("a", Json::from(0.0)), ("k", num(self.number("opacity") * alpha))]));
            }
        } else {
            self.put("o", "opacity", &same);
        }
    }

    fn stroke(&mut self) {
        self.put("w", "width", &same);
        self.raw("lc", self.choice("cap") as f64 + 1.0);
        self.raw("lj", self.choice("join") as f64 + 1.0);
        self.raw("ml", self.number("miter"));
        if self.moves("miter") {
            self.put("ml2", "miter", &same);
        }
        // Lottie strokes keep one width
        let thins = self.g.group("taper").is_ok_and(|t| ["startLength", "endLength"].iter().any(|k| t.prop(k).is_ok_and(|p| p.is_animated() || p.value.as_f64() > 0.0)));
        let waves = self.g.group("wave").is_ok_and(|w| w.prop("amount").is_ok_and(|p| p.is_animated() || p.value.as_f64() > 0.0));
        if thins || waves {
            self.cx.warn(format!("{}: Lottie strokes have no taper or wave (one width all along)", self.label));
        }
        let Ok(d) = self.g.group("dashes") else { return };
        let get = |id: &str| d.prop(id).ok();
        let used = |id: &str| get(id).is_some_and(|p| p.is_animated() || p.value.as_f64() > 0.0);
        if !used("dash") {
            return;
        }
        let mut list = Vec::new();
        for (dash, gap) in [("dash", "gap"), ("dash2", "gap2"), ("dash3", "gap3")] {
            if !used(dash) {
                break;
            }
            for (n, id) in [("d", dash), ("g", gap)] {
                let Some(p) = get(id) else { continue };
                let label = format!("{} › {}", self.label, p.def.name);
                // a gap of 0 is as long as its dash
                let v = if n == "g" && !used(gap) { prop_json(self.cx, get(dash).unwrap_or(p), &same, &label) } else { prop_json(self.cx, p, &same, &label) };
                list.push(Json::object([("n", Json::from(n)), ("nm", Json::from(p.def.name)), ("v", v)]));
            }
        }
        if let Some(p) = get("offset") {
            let v = prop_json(self.cx, p, &same, &format!("{} › Offset", self.label));
            list.push(Json::object([("n", Json::from("o")), ("nm", Json::from("Offset")), ("v", v)]));
        }
        self.raw("d", Json::Array(list));
    }

    fn gradient(&mut self) {
        self.put("o", "opacity", &same);
        self.put("s", "start", &same);
        self.put("e", "end", &same);
        self.raw("t", self.choice("type") as f64 + 1.0);
        self.put("h", "highlightLength", &same);
        self.put("a", "highlightAngle", &same);
        let Ok(p) = self.g.prop("colors") else { return };
        let count = |v: &Value| v.as_gradient().map_or(2, |g| if g.colors.is_empty() { 2 } else { g.colors.len() });
        let n = count(&p.value);
        if p.keys.iter().any(|k| count(&k.value) != n) {
            self.cx.warn(format!("{}: its keys have different numbers of colours, which Lottie cannot animate", self.label));
        }
        let k = prop_json(self.cx, p, &same, &format!("{} › Colors", self.label));
        self.raw("g", Json::object([("p", Json::from(n as f64)), ("k", k)]));
    }

    fn done(self) -> Json {
        Json::Object(self.o)
    }
}

/// A group's or repeater's transform as Lottie's `tr`.
fn transform_out(cx: &mut Ctx, tr: &Group, label: &str) -> Json {
    let mut o = Out { cx, g: tr, o: Map::new(), label: format!("{label} › Transform") };
    o.raw("ty", "tr");
    for (key, id) in [("a", "anchor"), ("p", "position"), ("s", "scale"), ("r", "rotation"), ("o", "opacity"), ("sk", "skew"), ("sa", "skewAxis"), ("so", "startOpacity"), ("eo", "endOpacity")] {
        o.put(key, id, &same);
    }
    o.raw("nm", "Transform");
    o.done()
}

/// The Lottie items of a contents list, top first.
pub fn items_out(cx: &mut Ctx, contents: &Group, label: &str) -> Vec<Json> {
    let mut out = Vec::new();
    for node in &contents.children {
        let Node::Group(g) = node else { continue };
        let label = format!("{label} › {}", g.name());
        let ty = match g.def.id {
            "group" => "gr",
            "rect" => "rc",
            "ellipse" => "el",
            "star" => "sr",
            "path" => "sh",
            "fill" => "fl",
            "stroke" => "st",
            "gradientFill" => "gf",
            "gradientStroke" => "gs",
            "trim" => "tm",
            "round" => "rd",
            "offset" => "op",
            "pucker" => "pb",
            "twist" => "tw",
            "zigzag" => "zz",
            "merge" => "mm",
            "repeater" => "rp",
            other => {
                cx.warn(format!("{label}: Lottie has no {other} (left out)"));
                continue;
            }
        };
        let mut o = Out { cx, g, o: Map::new(), label: label.clone() };
        o.raw("ty", ty);
        o.raw("nm", g.name());
        if !g.enabled {
            o.raw("hd", true);
        }
        let direction = |o: &mut Out| {
            let d = if o.choice("direction") == 1 { 3.0 } else { 1.0 };
            o.raw("d", d);
        };
        match ty {
            "gr" => {
                let mut items = g.group("contents").map(|c| items_out(o.cx, c, &label)).unwrap_or_default();
                if let Ok(tr) = g.group("transform") {
                    items.push(transform_out(o.cx, tr, &label));
                }
                o.raw("np", items.len() as f64);
                o.raw("it", Json::Array(items));
                o.blend();
            }
            "rc" => {
                direction(&mut o);
                o.put("s", "size", &same);
                o.put("p", "position", &same);
                o.put("r", "roundness", &same);
            }
            "el" => {
                direction(&mut o);
                o.put("s", "size", &same);
                o.put("p", "position", &same);
            }
            "sr" => {
                direction(&mut o);
                o.raw("sy", o.choice("type") as f64 + 1.0);
                for (key, id) in [("pt", "points"), ("p", "position"), ("r", "rotation"), ("ir", "innerRadius"), ("or", "outerRadius"), ("is", "innerRoundness"), ("os", "outerRoundness")] {
                    o.put(key, id, &same);
                }
            }
            "sh" => {
                direction(&mut o);
                o.put("ks", "path", &same);
            }
            "fl" => {
                o.paint();
                o.raw("r", o.choice("rule") as f64 + 1.0);
                o.blend();
                o.above();
            }
            "st" => {
                o.paint();
                o.stroke();
                o.blend();
                o.above();
            }
            "gf" => {
                o.gradient();
                o.raw("r", o.choice("rule") as f64 + 1.0);
                o.blend();
                o.above();
            }
            "gs" => {
                o.gradient();
                o.stroke();
                o.blend();
                o.above();
            }
            "tm" => {
                o.put("s", "start", &same);
                o.put("e", "end", &same);
                o.put("o", "offset", &same);
                o.raw("m", o.choice("mode") as f64 + 1.0);
            }
            "rd" => o.put("r", "radius", &same),
            "op" => {
                o.put("a", "amount", &same);
                o.raw("lj", o.choice("join") as f64 + 1.0);
                o.put("ml", "miter", &same);
                if o.number("copies") != 1.0 || o.moves("copies") {
                    o.cx.warn(format!("{label}: Lottie offsets one copy only"));
                }
            }
            "pb" => o.put("a", "amount", &same),
            "tw" => {
                o.put("a", "angle", &same);
                o.put("c", "center", &same);
            }
            "zz" => {
                o.put("s", "size", &same);
                o.put("r", "ridges", &same);
                o.put("pt", "points", &from_one);
            }
            "mm" => o.raw("mm", o.choice("mode") as f64 + 1.0),
            "rp" => {
                o.put("c", "copies", &same);
                o.put("o", "offset", &same);
                // Lottie: 1 above, 2 below
                o.raw("m", if o.choice("composite") == 1 { 1.0 } else { 2.0 });
                if let Ok(tr) = g.group("transform") {
                    let t = transform_out(o.cx, tr, &label);
                    o.raw("tr", t);
                }
            }
            _ => {}
        }
        out.push(o.done());
    }
    out
}

// ---------------------------------------------------------------- in

/// Sets our group's properties from a Lottie item.
struct In<'a> {
    cx: &'a mut Ctx,
    j: &'a Json,
    g: Group,
    label: String,
}

impl In<'_> {
    fn set(&mut self, key: &str, id: &str, conv: &dyn Fn(&Json) -> Option<Value>) {
        let (Some(j), Ok(p)) = (self.j.get(key), self.g.prop_mut(id)) else { return };
        let label = format!("{} › {}", self.label, p.def.name);
        prop_from(self.cx, j, p, conv, &label);
    }

    fn number(&mut self, key: &str, id: &str) {
        self.set(key, id, &values::number);
    }

    /// A raw Lottie number (from 1) as a choice, through `map` when its order differs.
    fn choice(&mut self, key: &str, id: &str, map: &dyn Fn(u32) -> u32) {
        let Some(n) = self.j.get(key).and_then(|v| v.as_f64().or_else(|| v.get("k").and_then(Json::as_f64))) else { return };
        if let Ok(p) = self.g.prop_mut(id) {
            let _ = p.set_value(Value::Choice(map((n.round() - 1.0).max(0.0) as u32)));
        }
    }

    fn blend(&mut self) {
        if let Some(bm) = self.j.get("bm").and_then(Json::as_f64) {
            let m = crate::blend_in(bm);
            let i = BlendMode::ALL.iter().position(|b| *b == m).unwrap_or(0);
            if let Ok(p) = self.g.prop_mut("blend") {
                let _ = p.set_value(Value::Choice(i as u32));
            }
        }
    }

    fn direction(&mut self) {
        if self.j.get("d").and_then(Json::as_f64) == Some(3.0)
            && let Ok(p) = self.g.prop_mut("direction")
        {
            let _ = p.set_value(Value::Choice(1));
        }
    }

    fn stroke(&mut self) {
        self.number("w", "width");
        self.choice("lc", "cap", &|c| c.min(2));
        self.choice("lj", "join", &|c| c.min(2));
        if self.j.get("ml2").is_some() {
            self.number("ml2", "miter");
        } else if let Some(ml) = self.j.get("ml").and_then(Json::as_f64)
            && let Ok(p) = self.g.prop_mut("miter")
        {
            let _ = p.set_value(Value::Number(ml.max(1.0)));
        }
        let Some(list) = self.j.get("d").and_then(Json::as_array) else { return };
        let (mut dashes, mut gaps) = (0, 0);
        for d in list {
            let id = match d.get("n").and_then(Json::as_str) {
                Some("d") if dashes < 3 => {
                    dashes += 1;
                    ["dash", "dash2", "dash3"][dashes - 1]
                }
                Some("g") if gaps < 3 => {
                    gaps += 1;
                    ["gap", "gap2", "gap3"][gaps - 1]
                }
                Some("o") => "offset",
                _ => {
                    self.cx.warn(format!("{}: only three dashes and gaps are kept", self.label));
                    continue;
                }
            };
            let (Some(v), Ok(dg)) = (d.get("v"), self.g.group_mut("dashes")) else { continue };
            if let Ok(p) = dg.prop_mut(id) {
                prop_from(self.cx, v, p, &values::number, &self.label);
            }
        }
    }

    fn gradient(&mut self) {
        self.number("o", "opacity");
        self.set("s", "start", &vec2_of);
        self.set("e", "end", &vec2_of);
        self.choice("t", "type", &|c| c.min(1));
        self.number("h", "highlightLength");
        self.number("a", "highlightAngle");
        let Some(g) = self.j.get("g") else { return };
        let count = g.get("p").and_then(Json::as_f64).unwrap_or(2.0).max(1.0) as usize;
        let (Some(k), Ok(p)) = (g.get("k"), self.g.prop_mut("colors")) else { return };
        let label = format!("{} › Colors", self.label);
        let conv = move |j: &Json| -> Option<Value> {
            let n = values::numbers(j);
            (n.len() >= count * 4).then(|| Value::Gradient(values::gradient_of(&n, count)))
        };
        prop_from(self.cx, k, p, &conv, &label);
    }
}

/// The default name of the `n`th item of a type: "Rectangle 1".
fn name_of(j: &Json, def: &GroupDef, n: usize) -> String {
    j.get("nm").and_then(Json::as_str).filter(|s| !s.is_empty()).map_or_else(|| format!("{} {n}", def.name), str::to_string)
}

/// Our contents from Lottie items (top first), each given an id by `ids`.
pub fn items_in(cx: &mut Ctx, items: &[Json], ids: &mut dyn FnMut() -> u64, label: &str) -> Vec<Node> {
    let mut out = Vec::new();
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    for j in items {
        let ty = j.get("ty").and_then(Json::as_str).unwrap_or("");
        let def: &'static GroupDef = match ty {
            "gr" => &defs::GROUP,
            "rc" => &defs::RECT,
            "el" => &defs::ELLIPSE,
            "sr" => &defs::STAR,
            "sh" => &defs::PATH,
            "fl" => &defs::FILL,
            "st" => &defs::STROKE,
            "gf" => &defs::GRADIENT_FILL,
            "gs" => &defs::GRADIENT_STROKE,
            "tm" => &defs::TRIM,
            "rd" => &defs::ROUND,
            "op" => &defs::OFFSET,
            "pb" => &defs::PUCKER,
            "tw" => &defs::TWIST,
            "zz" => &defs::ZIGZAG,
            "mm" => &defs::MERGE,
            "rp" => &defs::REPEATER,
            // a group's transform is read with its group; "no style" items draw nothing
            "tr" | "no" => continue,
            other => {
                cx.warn(format!("{label}: a shape item of type \"{other}\" is not read"));
                continue;
            }
        };
        let n = match counts.iter_mut().find(|(t, _)| *t == def.id) {
            Some((_, n)) => {
                *n += 1;
                *n
            }
            None => {
                counts.push((def.id, 1));
                1
            }
        };
        let name = name_of(j, def, n);
        let mut g = Group::instance(def, ids(), &name);
        g.enabled = j.get("hd").and_then(Json::as_bool) != Some(true);
        let mut i = In { cx, j, g, label: format!("{label} › {name}") };
        match ty {
            "gr" => {
                let list = j.get("it").and_then(Json::as_array).map(Vec::as_slice).unwrap_or(&[]);
                let inner = items_in(i.cx, list, ids, &i.label);
                if let Ok(c) = i.g.group_mut("contents") {
                    c.children = inner;
                }
                if let Some(tr) = list.iter().find(|t| t.get("ty").and_then(Json::as_str) == Some("tr")) {
                    transform_in(i.cx, tr, &mut i.g, &i.label);
                }
                i.blend();
            }
            "rc" | "el" => {
                i.direction();
                i.set("s", "size", &vec2_of);
                i.set("p", "position", &vec2_of);
                if ty == "rc" {
                    i.number("r", "roundness");
                }
            }
            "sr" => {
                i.direction();
                i.choice("sy", "type", &|c| c.min(1));
                for (key, id) in [("pt", "points"), ("r", "rotation"), ("ir", "innerRadius"), ("or", "outerRadius"), ("is", "innerRoundness"), ("os", "outerRoundness")] {
                    i.number(key, id);
                }
                i.set("p", "position", &vec2_of);
            }
            "sh" => {
                i.direction();
                i.set("ks", "path", &path_value);
            }
            "fl" | "st" => {
                i.set("c", "color", &color_of);
                i.number("o", "opacity");
                if ty == "fl" {
                    i.choice("r", "rule", &|c| c.min(1));
                } else {
                    i.stroke();
                }
                i.blend();
            }
            "gf" | "gs" => {
                i.gradient();
                if ty == "gf" {
                    i.choice("r", "rule", &|c| c.min(1));
                } else {
                    i.stroke();
                }
                i.blend();
            }
            "tm" => {
                i.number("s", "start");
                i.number("e", "end");
                i.number("o", "offset");
                i.choice("m", "mode", &|c| c.min(1));
            }
            "rd" => i.number("r", "radius"),
            "op" => {
                i.number("a", "amount");
                i.choice("lj", "join", &|c| c.min(2));
                i.number("ml", "miter");
            }
            "pb" => i.number("a", "amount"),
            "tw" => {
                i.number("a", "angle");
                i.set("c", "center", &vec2_of);
            }
            "zz" => {
                i.number("s", "size");
                i.number("r", "ridges");
                i.set("pt", "points", &values::choice_of);
            }
            "mm" => i.choice("mm", "mode", &|c| c.min(4)),
            "rp" => {
                i.number("c", "copies");
                i.number("o", "offset");
                // Lottie: 1 above, 2 below
                i.choice("m", "composite", &|c| if c == 0 { 1 } else { 0 });
                if let Some(tr) = j.get("tr") {
                    transform_in(i.cx, tr, &mut i.g, &i.label);
                }
            }
            _ => {}
        }
        out.push(Node::Group(i.g));
    }
    out
}

/// A Lottie `tr` into a group's or repeater's transform.
fn transform_in(cx: &mut Ctx, tr: &Json, g: &mut Group, label: &str) {
    let Ok(t) = g.group_mut("transform") else { return };
    let label = format!("{label} › Transform");
    for (key, id) in [("a", "anchor"), ("p", "position"), ("s", "scale"), ("r", "rotation"), ("o", "opacity"), ("sk", "skew"), ("sa", "skewAxis"), ("so", "startOpacity"), ("eo", "endOpacity")] {
        let (Some(j), Ok(p)) = (tr.get(key), t.prop_mut(id)) else { continue };
        let conv: &dyn Fn(&Json) -> Option<Value> = if matches!(p.value, Value::Vec2(_)) { &vec2_of } else { &values::number };
        prop_from(cx, j, p, conv, &label);
    }
}

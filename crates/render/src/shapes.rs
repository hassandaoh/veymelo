//! Drawing a shape layer's contents.
//!
//! The contents are read top to bottom. Paths go into slots; an operator (trim, round corners,
//! offset, pucker and bloat, twist, zig zag, wiggle) changes every slot above it in its group,
//! groups inside included; a paint (fill, stroke, gradient) remembers the slots above it, so it
//! paints them as every operator in the group leaves them, even ones below it. A group's
//! transform moves its slots and paints when the group is done. Paints draw from the bottom up,
//! each new one below those before it unless it is composited above.

use veymelo_color::{BlendMode, Rgba, blend};
use veymelo_geom::{Mat3, Rect, Vec2, vec2};
use veymelo_keyframe::{Gradient, Value};
use veymelo_path::shape::{Polystar, Sub, Vertex};
use veymelo_path::{Cap, Join, Merge, Path, Rule, Stroke};
use veymelo_project::{Group, Node};
use veymelo_raster::{Image, Px, noise};
use veymelo_time::Tick;

use crate::Read;

/// Where on a gradient each point is: along the line from start to end, or out from the
/// start (from its highlight, when moved) to the circle through the end.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ramp {
    radial: bool,
    start: [f64; 2],
    d: [f64; 2],
    len2: f64,
    focal: Option<[f64; 2]>,
}

impl Ramp {
    fn new(radial: bool, start: [f64; 2], end: [f64; 2], highlight: f64, angle: f64) -> Ramp {
        let d = [end[0] - start[0], end[1] - start[1]];
        let len2 = (d[0] * d[0] + d[1] * d[1]).max(1e-12);
        let focal = (radial && highlight.abs() > 1e-6).then(|| {
            let r = len2.sqrt() * (highlight / 100.0).clamp(-0.99, 0.99);
            let a = d[1].atan2(d[0]) + angle.to_radians();
            [start[0] + r * a.cos(), start[1] + r * a.sin()]
        });
        Ramp { radial, start, d, len2, focal }
    }

    fn at(&self, p: [f64; 2]) -> f64 {
        let v = [p[0] - self.start[0], p[1] - self.start[1]];
        if !self.radial {
            return (v[0] * self.d[0] + v[1] * self.d[1]) / self.len2;
        }
        let Some(f) = self.focal else { return ((v[0] * v[0] + v[1] * v[1]) / self.len2).sqrt() };
        // from the highlight through p out to the circle: how far along that p is
        let q = [p[0] - f[0], p[1] - f[1]];
        let dist = q[0].hypot(q[1]);
        if dist < 1e-12 {
            return 0.0;
        }
        let u = [q[0] / dist, q[1] / dist];
        let c = [f[0] - self.start[0], f[1] - self.start[1]];
        let b = u[0] * c[0] + u[1] * c[1];
        let reach = -b + (b * b - (c[0] * c[0] + c[1] * c[1] - self.len2)).max(0.0).sqrt();
        if reach <= 1e-12 { 1.0 } else { dist / reach }
    }
}

#[derive(Clone)]
enum Look {
    Color(Rgba),
    Gradient(Gradient, Ramp),
}

/// Something to paint.
#[derive(Clone)]
struct Paint {
    look: Look,
    stroke: Option<Stroke>,
    rule: Rule,
    /// The slots it paints.
    slots: Vec<usize>,
    /// From its group's space to the layer's.
    xf: Mat3,
    opacity: f64,
    blend: BlendMode,
    /// Drawn above the paints before it in its group.
    above: bool,
}

/// An item's properties at the layer's time.
struct At<'a> {
    g: &'a Group,
    read: Read<'a>,
}

impl At<'_> {
    fn value(&self, path: &str) -> Option<Value> {
        self.g.prop(path).ok().map(self.read)
    }

    fn num(&self, path: &str) -> f64 {
        self.value(path).map_or(0.0, |v| v.as_f64())
    }

    fn v2(&self, path: &str) -> [f64; 2] {
        self.value(path).map_or([0.0; 2], |v| v.as_vec2())
    }

    fn choice(&self, path: &str) -> u32 {
        self.value(path).map_or(0, |v| v.as_choice())
    }

    fn color(&self, path: &str) -> Rgba {
        self.value(path).map_or(Rgba::WHITE, |v| v.as_color())
    }

    fn blend(&self) -> BlendMode {
        BlendMode::ALL[self.choice("blend") as usize % BlendMode::ALL.len()]
    }
}

fn map_sub(s: &Sub, m: &Mat3) -> Sub {
    s.map(&|p| {
        let q = m.apply(vec2(p[0], p[1]));
        [q.x, q.y]
    })
}

fn stroke_style(a: &At) -> Stroke {
    let d = |k: &str| a.num(&format!("dashes/{k}")).max(0.0);
    let mut dashes = Vec::new();
    for (dash, gap) in [("dash", "gap"), ("dash2", "gap2"), ("dash3", "gap3")] {
        let (len, space) = (d(dash), d(gap));
        if len > 0.0 {
            dashes.push(len);
            dashes.push(if space > 0.0 { space } else { len });
        }
    }
    Stroke {
        width: a.num("width").max(0.0),
        cap: [Cap::Butt, Cap::Round, Cap::Square][a.choice("cap") as usize % 3],
        join: [Join::Miter, Join::Round, Join::Bevel][a.choice("join") as usize % 3],
        miter_limit: a.num("miter").max(1.0),
        dashes,
        dash_offset: a.num("dashes/offset"),
        taper: Some(veymelo_path::Taper {
            in_parts: a.choice("taper/lengthUnits") == 0,
            start_length: a.num("taper/startLength") / if a.choice("taper/lengthUnits") == 0 { 100.0 } else { 1.0 },
            end_length: a.num("taper/endLength") / if a.choice("taper/lengthUnits") == 0 { 100.0 } else { 1.0 },
            start_width: a.num("taper/startWidth") / 100.0,
            end_width: a.num("taper/endWidth") / 100.0,
            start_ease: a.num("taper/startEase") / 100.0,
            end_ease: a.num("taper/endEase") / 100.0,
        }),
        wave: Some(veymelo_path::Wave {
            amount: a.num("wave/amount") / 100.0,
            cycles: (a.choice("wave/units") == 1).then(|| a.num("wave/cycles")),
            wavelength: a.num("wave/wavelength"),
            phase: a.num("wave/phase"),
        }),
    }
}

fn ramp(a: &At) -> Ramp {
    Ramp::new(a.choice("type") == 1, a.v2("start"), a.v2("end"), a.num("highlightLength"), a.num("highlightAngle"))
}

fn gradient(a: &At) -> Gradient {
    a.value("colors").and_then(|v| v.as_gradient().cloned()).unwrap_or_default()
}

/// A group's transform: position, rotation, skew, scale, about its anchor.
fn group_matrix(a: &At) -> Mat3 {
    let (anchor, pos, scale) = (a.v2("transform/anchor"), a.v2("transform/position"), a.v2("transform/scale"));
    Mat3::translate(vec2(pos[0], pos[1]))
        * Mat3::rotate_deg(a.num("transform/rotation"))
        * Mat3::skew_deg(-a.num("transform/skew"), a.num("transform/skewAxis"))
        * Mat3::scale(vec2(scale[0] / 100.0, scale[1] / 100.0))
        * Mat3::translate(vec2(-anchor[0], -anchor[1]))
}

/// A drawn path from a path property.
fn drawn(p: &veymelo_keyframe::ShapePath) -> Sub {
    let v = (0..p.points.len())
        .map(|k| Vertex { p: p.points[k], i: p.in_tangents.get(k).copied().unwrap_or([0.0; 2]), o: p.out_tangents.get(k).copied().unwrap_or([0.0; 2]) })
        .collect();
    Sub { v, closed: p.closed }
}

/// Read a group's contents: its paints, bottom first, and the slots of its paths.
fn collect(contents: &Group, t: Tick, read: Read, arena: &mut Vec<Vec<Sub>>) -> (Vec<Paint>, Vec<usize>) {
    let mut order: Vec<Paint> = Vec::new();
    let mut live: Vec<usize> = Vec::new();
    let place = |order: &mut Vec<Paint>, p: Paint| {
        if p.above {
            order.push(p);
        } else {
            order.insert(0, p);
        }
    };
    for node in &contents.children {
        let Node::Group(g) = node else { continue };
        if !g.enabled {
            continue;
        }
        let a = At { g, read };
        let reversed = a.choice("direction") == 1;
        let push_path = |s: Sub, arena: &mut Vec<Vec<Sub>>, live: &mut Vec<usize>| {
            arena.push(vec![if reversed { s.reversed() } else { s }]);
            live.push(arena.len() - 1);
        };
        match g.def.id {
            "rect" => push_path(veymelo_path::shape::rect(a.v2("size"), a.v2("position"), a.num("roundness")), arena, &mut live),
            "ellipse" => push_path(veymelo_path::shape::ellipse(a.v2("size"), a.v2("position")), arena, &mut live),
            "star" => push_path(
                veymelo_path::shape::polystar(&Polystar {
                    star: a.choice("type") == 0,
                    points: a.num("points"),
                    centre: a.v2("position"),
                    rotation: a.num("rotation"),
                    inner_radius: a.num("innerRadius"),
                    outer_radius: a.num("outerRadius"),
                    inner_roundness: a.num("innerRoundness"),
                    outer_roundness: a.num("outerRoundness"),
                }),
                arena,
                &mut live,
            ),
            "path" => {
                if let Some(Value::Path(p)) = a.value("path") {
                    push_path(drawn(&p), arena, &mut live);
                }
            }
            "fill" | "stroke" | "gradientFill" | "gradientStroke" => {
                let look = if g.def.id.starts_with("gradient") { Look::Gradient(gradient(&a), ramp(&a)) } else { Look::Color(a.color("color")) };
                let stroke = g.def.id.ends_with("troke").then(|| stroke_style(&a));
                let rule = if a.choice("rule") == 1 { Rule::EvenOdd } else { Rule::NonZero };
                let p = Paint { look, stroke, rule, slots: live.clone(), xf: Mat3::default(), opacity: a.num("opacity") / 100.0, blend: a.blend(), above: a.choice("composite") == 1 };
                place(&mut order, p);
            }
            "trim" => {
                let (s, e, o, each) = (a.num("start"), a.num("end"), a.num("offset"), a.choice("mode") == 1);
                // the slots above, in order, trimmed as one list; the pieces go back to their slots
                let mut owner = Vec::new();
                let mut all = Vec::new();
                for &slot in &live {
                    for sub in &arena[slot] {
                        owner.push(slot);
                        all.push(sub.clone());
                    }
                }
                for &slot in &live {
                    arena[slot].clear();
                }
                for (k, piece) in veymelo_path::shape::trim_indexed(&all, s, e, o, each) {
                    arena[owner[k]].push(piece);
                }
            }
            "round" => {
                let r = a.num("radius");
                for &slot in &live {
                    arena[slot] = veymelo_path::ops::round_corners(&arena[slot], r);
                }
            }
            "offset" => {
                let join = [Join::Miter, Join::Round, Join::Bevel][a.choice("join") as usize % 3];
                let (amount, miter, copies, copy_offset) = (a.num("amount"), a.num("miter"), a.num("copies"), a.num("copyOffset"));
                for &slot in &live {
                    arena[slot] = veymelo_path::ops::offset(&arena[slot], amount, join, miter, copies, copy_offset);
                }
            }
            "pucker" => {
                let k = a.num("amount");
                for &slot in &live {
                    arena[slot] = veymelo_path::ops::pucker_bloat(&arena[slot], k);
                }
            }
            "twist" => {
                let (angle, centre) = (a.num("angle"), a.v2("center"));
                for &slot in &live {
                    arena[slot] = veymelo_path::ops::twist(&arena[slot], angle, centre);
                }
            }
            "zigzag" => {
                let (size, ridges, smooth) = (a.num("size"), a.num("ridges"), a.choice("points") == 1);
                for &slot in &live {
                    arena[slot] = veymelo_path::ops::zigzag(&arena[slot], size, ridges, smooth);
                }
            }
            "wiggle" => {
                let w = veymelo_path::ops::Wiggle {
                    size: a.num("size"),
                    detail: a.num("detail"),
                    smooth: a.choice("points") == 1,
                    speed: a.num("speed"),
                    correlation: a.num("correlation"),
                    phase: a.num("phase"),
                    seed: a.num("seed"),
                };
                for &slot in &live {
                    arena[slot] = veymelo_path::ops::wiggle(&arena[slot], &w, t.seconds());
                }
            }
            "merge" => {
                let mode = [Merge::Merge, Merge::Add, Merge::Subtract, Merge::Intersect, Merge::Exclude][a.choice("mode") as usize % 5];
                let inputs: Vec<Vec<Sub>> = live.iter().map(|&s| arena[s].clone()).collect();
                arena.push(veymelo_path::merge(&inputs, mode));
                live = vec![arena.len() - 1];
                // the paints above a merge in its group are not drawn: those below paint the result
                order.clear();
            }
            "group" => {
                let Ok(inner) = g.group("contents") else { continue };
                let (inner_order, inner_live) = collect(inner, t, read, arena);
                let m = group_matrix(&a);
                let fade = a.num("transform/opacity") / 100.0;
                let mut moved: Vec<usize> = inner_live.iter().chain(inner_order.iter().flat_map(|p| p.slots.iter())).copied().collect();
                moved.sort();
                moved.dedup();
                for slot in moved {
                    arena[slot] = arena[slot].iter().map(|s| map_sub(s, &m)).collect();
                }
                let group_blend = a.blend();
                let block: Vec<Paint> = inner_order
                    .into_iter()
                    .map(|mut p| {
                        p.xf = m * p.xf;
                        p.opacity *= fade;
                        if p.blend == BlendMode::Normal {
                            p.blend = group_blend;
                        }
                        p
                    })
                    .collect();
                order.splice(0..0, block);
                live.extend(inner_live);
            }
            "repeater" => {
                let copies = a.num("copies").max(0.0);
                let n = copies.ceil() as usize;
                let offset = a.num("offset");
                let (anchor, pos, scale) = (a.v2("transform/anchor"), a.v2("transform/position"), a.v2("transform/scale"));
                let (rot, so, eo) = (a.num("transform/rotation"), a.num("transform/startOpacity") / 100.0, a.num("transform/endOpacity") / 100.0);
                let step = |k: f64| {
                    Mat3::translate(vec2(pos[0] * k, pos[1] * k))
                        * Mat3::translate(vec2(anchor[0], anchor[1]))
                        * Mat3::rotate_deg(rot * k)
                        * Mat3::scale(vec2((scale[0] / 100.0).powf(k), (scale[1] / 100.0).powf(k)))
                        * Mat3::translate(vec2(-anchor[0], -anchor[1]))
                };
                let base_order = std::mem::take(&mut order);
                let base_live = std::mem::take(&mut live);
                let mut base_slots: Vec<usize> = base_live.iter().chain(base_order.iter().flat_map(|p| p.slots.iter())).copied().collect();
                base_slots.sort();
                base_slots.dedup();
                let mut blocks: Vec<Vec<Paint>> = Vec::with_capacity(n);
                for i in 0..n {
                    let m = step(i as f64 + offset);
                    let mut remap = std::collections::HashMap::new();
                    for &s in &base_slots {
                        arena.push(arena[s].iter().map(|x| map_sub(x, &m)).collect());
                        remap.insert(s, arena.len() - 1);
                    }
                    live.extend(base_live.iter().map(|s| remap[s]));
                    let fade = if n > 1 { so + (eo - so) * i as f64 / (n - 1) as f64 } else { so };
                    // a fraction of a copy draws the last one faded by that much
                    let part = if i + 1 == n && copies.fract() > 0.0 { copies.fract() } else { 1.0 };
                    blocks.push(
                        base_order
                            .iter()
                            .map(|p| {
                                let mut p = p.clone();
                                p.slots = p.slots.iter().map(|s| remap[s]).collect();
                                p.xf = m * p.xf;
                                p.opacity *= fade * part;
                                p
                            })
                            .collect(),
                    );
                }
                // below: each copy under the one before, the first on top
                if a.choice("composite") == 0 {
                    blocks.reverse();
                }
                order = blocks.into_iter().flatten().collect();
            }
            _ => {}
        }
    }
    (order, live)
}

/// What a paint covers, in the layer's space.
fn geometry(p: &Paint, arena: &[Vec<Sub>]) -> Path {
    let shapes: Vec<Sub> = p.slots.iter().flat_map(|&s| arena[s].iter().cloned()).collect();
    match &p.stroke {
        None => veymelo_path::shape::flatten(&shapes),
        Some(style) => {
            // the stroke is as wide as its own group says: drawn there, then moved out
            let Some(inv) = p.xf.inverse() else { return Path::default() };
            let local: Vec<Sub> = shapes.iter().map(|s| map_sub(s, &inv)).collect();
            let outline = veymelo_path::stroke(&local, style);
            let mut out = Path::default();
            for c in outline.contours() {
                for (k, q) in c.iter().enumerate() {
                    let m = p.xf.apply(vec2(q.0, q.1));
                    if k == 0 {
                        out.move_to(m.x, m.y);
                    } else {
                        out.line_to(m.x, m.y);
                    }
                }
            }
            out
        }
    }
}

/// The fewest pixels a shape layer's picture has for its shapes to be shared among the cores
/// (fewer cost less than starting the threads).
const BIG: usize = 1 << 18;

/// A shape layer's contents worked out at a time: each paint with its outline, and where they
/// all reach (in the layer's own units), ready to be drawn over any part of it.
pub(crate) struct Drawing {
    shapes: Vec<(Paint, Path)>,
    pub bounds: Rect,
}

impl Drawing {
    pub(crate) fn of(contents: &Group, t: Tick, read: Read) -> Drawing {
        let mut arena = Vec::new();
        let (order, _) = collect(contents, t, read, &mut arena);
        let shapes: Vec<(Paint, Path)> = order.into_iter().filter(|p| p.opacity > 0.0).map(|p| {
            let g = geometry(&p, &arena);
            (p, g)
        }).collect();
        let mut bounds = Rect::EMPTY;
        for (_, g) in &shapes {
            if let Some((x0, y0, x1, y1)) = g.extent() {
                bounds = bounds.union(&Rect::new(x0, y0, x1, y1));
            }
        }
        Drawing { shapes, bounds }
    }

    /// The part `area` of it (in its own units) drawn at `s` pixels to a unit (fewer if it is
    /// huge): the picture, where its first pixel starts, and the pixels to a unit it was drawn at.
    pub(crate) fn paint(&self, area: Rect, s: f64, px: &dyn Fn(Rgba, f64) -> Px) -> Option<(Image, Vec2, f64)> {
        let shapes = &self.shapes;
        if area.is_empty() || !area.width().is_finite() {
            return None;
        }
    let s = s.min(8192.0 / area.width().max(area.height()).max(1e-9));
    let (i0, j0) = ((area.x0 * s).floor(), (area.y0 * s).floor());
    let (w, h) = (((area.x1 * s).ceil() - i0).max(1.0) as usize, ((area.y1 * s).ceil() - j0).max(1.0) as usize);
    let mut img = Image::new(w, h);
    let origin = vec2(i0 / s, j0 / s);
    let to_buf = Mat3::translate(vec2(-i0, -j0)) * Mat3::scale(vec2(s, s));
    // each shape's cover (on the cores at once, for a big picture), then the shapes put on in
    // their order, the rows shared among the cores
    let big = w * h >= BIG;
    let cover_of = |k: usize| {
        let (p, g) = &shapes[k];
        let mut mapped = Path::default();
        for c in g.contours() {
            for (n, q) in c.iter().enumerate() {
                let m = to_buf.apply(vec2(q.0, q.1));
                if n == 0 {
                    mapped.move_to(m.x, m.y);
                } else {
                    mapped.line_to(m.x, m.y);
                }
            }
        }
        let rule = if p.stroke.is_some() { Rule::NonZero } else { p.rule };
        veymelo_path::fill(&mapped, rule, w, h)
    };
    let covers: Vec<_> = if big { veymelo_raster::par::map(shapes.len(), cover_of) } else { (0..shapes.len()).map(cover_of).collect() };
    let looks: Vec<(Option<Px>, Option<Mat3>)> = shapes
        .iter()
        .map(|(p, _)| {
            let solid = match &p.look {
                Look::Color(c) => Some(px(c.with_alpha(1.0), p.opacity)),
                Look::Gradient(..) => None,
            };
            (solid, (to_buf * p.xf).inverse())
        })
        .collect();
    // a row of the picture with the shapes put on it in their order; `src` the colour of shape k at
    // pixel x of row y
    let put = |y: usize, row: &mut [Px], src: &dyn Fn(usize, usize, usize) -> Option<Px>| {
        for (k, ((p, _), cover)) in shapes.iter().zip(&covers).enumerate() {
            let Some(cover) = cover else { continue };
            if y < cover.top || y >= cover.top + cover.height {
                continue;
            }
            for x in cover.left..cover.left + cover.width {
                let c = cover.at(x, y);
                if c <= 0.0 {
                    continue;
                }
                let Some(src) = src(k, x, y) else { continue };
                let d = &mut row[x];
                *d = blend(p.blend, *d, src.map(|v| v * c), noise(x, y, k as u32));
            }
        }
    };
    if big && looks.iter().all(|(solid, _)| solid.is_some()) {
        // solid colours: the rows on the cores
        let solid: Vec<Px> = looks.iter().map(|(sp, _)| sp.unwrap_or([0.0; 4])).collect();
        veymelo_raster::par::rows(&mut img.px, w, |y, row| put(y, row, &|k, _, _| Some(solid[k])));
    } else {
        // a small picture, or gradients (they ask for their colours pixel by pixel): here
        for (y, row) in img.px.chunks_mut(w).enumerate() {
            put(y, row, &|k, x, y| match (&looks[k].0, &shapes[k].0.look, looks[k].1) {
                (Some(sp), _, _) => Some(*sp),
                (None, Look::Gradient(gr, r), Some(inv)) => {
                    let q = inv.apply(vec2(x as f64 + 0.5, y as f64 + 0.5));
                    let [cr, cg, cb, ca] = gr.at(r.at([q.x, q.y]).clamp(0.0, 1.0));
                    Some(px(Rgba { r: cr, g: cg, b: cb, a: ca }, shapes[k].0.opacity))
                }
                _ => None,
            });
        }
    }
    Some((img, origin, s))
    }
}

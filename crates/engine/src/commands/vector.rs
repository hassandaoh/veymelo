//! Vector pictures made shape layers: an SVG footage layer's shapes, groups, fills, strokes and
//! gradients as a shape layer's own items, standing where the footage layer stands.

use veymelo_color::Rgba;
use veymelo_geom::{Mat3, vec2};
use veymelo_json::Value as Json;
use veymelo_keyframe::{Gradient, ShapePath, Value};
use veymelo_project::{Group, GroupDef, ItemKind, Layer, Node, Source, shapes};
use veymelo_svg::{GradientKind, Paint};

use super::{COMP, Command, LAYER, Run, layer_ids};
use crate::{Args, Ctx, Error, Result};

pub fn commands() -> Vec<Command> {
    vec![Command {
        id: "layer.shapesFromVector",
        label: "Create Shapes from Vector Layer",
        summary: "Make a vector picture's layer (an imported SVG) a shape layer: each of its shapes a group of paths with its fill and stroke (gradients too), its groups kept as groups with their opacity, standing where the picture stands. The picture's layer is switched off.",
        params: const { &[COMP, LAYER] },
        returns: "{layer, name, shapes}",
        run: Run::Edit(shapes_from_vector),
    }]
}

/// A new shape item of `def`.
fn item(cx: &mut Ctx, def: &'static GroupDef, name: &str) -> Group {
    Group::instance(def, cx.project.alloc(), name)
}

fn set(g: &mut Group, path: &str, v: Value) {
    if let Ok(p) = g.prop_mut(path) {
        // values out of a property's range kept to it
        let _ = p.set_value(v);
    }
}

fn color(c: [f64; 3]) -> Value {
    Value::Color(Rgba { r: c[0], g: c[1], b: c[2], a: 1.0 })
}

fn put(g: &mut Group, child: Group) {
    if let Ok(list) = g.group_mut("contents") {
        list.children.push(Node::Group(child));
    }
}

/// A gradient's stops as a gradient value.
fn stops(g: &veymelo_svg::Gradient) -> Value {
    Value::Gradient(Gradient { colors: g.stops.iter().map(|(t, c)| (*t, [c[0], c[1], c[2]])).collect(), opacities: g.stops.iter().map(|(t, c)| (*t, c[3])).collect() })
}

/// A gradient fill or stroke's type, points and colours, its coordinates through `m` (the
/// shape's units to the layer's) and its box `bounds` when they are parts of it.
fn gradient(g: &mut Group, grad: &veymelo_svg::Gradient, m: &Mat3, bounds: [f64; 4]) {
    let to_shape = |x: f64, y: f64| {
        let p = grad.transform.apply(vec2(x, y));
        let p = if grad.box_units { vec2(bounds[0] + p.x * (bounds[2] - bounds[0]), bounds[1] + p.y * (bounds[3] - bounds[1])) } else { p };
        let q = m.apply(p);
        [q.x, q.y]
    };
    match grad.kind {
        GradientKind::Linear { x1, y1, x2, y2 } => {
            set(g, "type", Value::Choice(0));
            set(g, "start", Value::Vec2(to_shape(x1, y1)));
            set(g, "end", Value::Vec2(to_shape(x2, y2)));
        }
        GradientKind::Radial { cx, cy, r, fx, fy } => {
            let (c, e, f) = (to_shape(cx, cy), to_shape(cx + r, cy), to_shape(fx, fy));
            set(g, "type", Value::Choice(1));
            set(g, "start", Value::Vec2(c));
            set(g, "end", Value::Vec2(e));
            // the focal point as how far toward the edge, and which way from the line to the end
            let radius = (e[0] - c[0]).hypot(e[1] - c[1]).max(1e-9);
            let (dx, dy) = (f[0] - c[0], f[1] - c[1]);
            if dx.hypot(dy) > 1e-9 {
                set(g, "highlightLength", Value::Number((dx.hypot(dy) / radius * 100.0).min(100.0)));
                let angle = dy.atan2(dx) - (e[1] - c[1]).atan2(e[0] - c[0]);
                set(g, "highlightAngle", Value::Number(angle.to_degrees()));
            }
        }
    }
    set(g, "colors", stops(grad));
}

/// An SVG shape as a shape group: its paths (through `m`), stroke and fill.
fn shape_group(cx: &mut Ctx, s: &veymelo_svg::Shape, m: Mat3, opacity: f64) -> Group {
    let mut g = item(cx, &shapes::GROUP, &s.name);
    for (k, sub) in s.geom.subs().iter().enumerate() {
        let laid = sub.map(&|p| {
            let q = m.apply(vec2(p[0], p[1]));
            [q.x, q.y]
        });
        let mut p = item(cx, &shapes::PATH, &format!("Path {}", k + 1));
        set(&mut p, "path", Value::Path(ShapePath { points: laid.v.iter().map(|v| v.p).collect(), in_tangents: laid.v.iter().map(|v| v.i).collect(), out_tangents: laid.v.iter().map(|v| v.o).collect(), closed: laid.closed, feather: veymelo_keyframe::Feathers::NONE }));
        put(&mut g, p);
    }
    let bounds = s.geom.bounds().unwrap_or([0.0, 0.0, 1.0, 1.0]);
    // the stroke above the fill, as an SVG shape draws them
    if let Some(st) = &s.stroke {
        let k = m.mean_scale();
        let mut item = match &st.paint {
            Paint::Color(c) => {
                let mut i = item(cx, &shapes::STROKE, "Stroke 1");
                set(&mut i, "color", color(*c));
                i
            }
            Paint::Gradient(grad) => {
                let mut i = item(cx, &shapes::GRADIENT_STROKE, "Gradient Stroke 1");
                gradient(&mut i, grad, &m, bounds);
                i
            }
        };
        set(&mut item, "opacity", Value::Number(st.opacity * 100.0));
        set(&mut item, "width", Value::Number(st.width * k));
        set(&mut item, "cap", Value::Choice(st.cap as u32));
        set(&mut item, "join", Value::Choice(st.join as u32));
        set(&mut item, "miter", Value::Number(st.miter));
        for (n, pair) in st.dashes.chunks(2).take(3).enumerate() {
            let suffix = if n == 0 { String::new() } else { (n + 1).to_string() };
            set(&mut item, &format!("dashes/dash{suffix}"), Value::Number(pair[0] * k));
            set(&mut item, &format!("dashes/gap{suffix}"), Value::Number(pair.get(1).copied().unwrap_or(pair[0]) * k));
        }
        if !st.dashes.is_empty() {
            set(&mut item, "dashes/offset", Value::Number(st.dash_offset * k));
        }
        put(&mut g, item);
    }
    if let Some(fill) = &s.fill {
        let mut item = match fill {
            Paint::Color(c) => {
                let mut i = item(cx, &shapes::FILL, "Fill 1");
                set(&mut i, "color", color(*c));
                i
            }
            Paint::Gradient(grad) => {
                let mut i = item(cx, &shapes::GRADIENT_FILL, "Gradient Fill 1");
                gradient(&mut i, grad, &m, bounds);
                i
            }
        };
        set(&mut item, "opacity", Value::Number(s.fill_opacity * 100.0));
        set(&mut item, "rule", Value::Choice(s.even_odd as u32));
        put(&mut g, item);
    }
    set(&mut g, "transform/opacity", Value::Number(opacity * 100.0));
    g
}

/// An SVG group's children as shape items, top first; how many shapes.
fn items(cx: &mut Ctx, group: &veymelo_svg::Group, m: Mat3, out: &mut Vec<Group>) -> usize {
    let mut count = 0;
    for child in group.children.iter().rev() {
        match child {
            veymelo_svg::Node::Shape(s) => {
                out.push(shape_group(cx, s, m * s.transform, s.opacity));
                count += 1;
            }
            veymelo_svg::Node::Group(sub) => {
                let mut inner = Vec::new();
                count += items(cx, sub, m * sub.transform, &mut inner);
                if inner.is_empty() {
                    continue;
                }
                let mut g = item(cx, &shapes::GROUP, &sub.name);
                for i in inner {
                    put(&mut g, i);
                }
                set(&mut g, "transform/opacity", Value::Number(sub.opacity * 100.0));
                out.push(g);
            }
        }
    }
    count
}

fn shapes_from_vector(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let source = comp.layer(lid).expect("found").clone();
    let path = match source.source {
        Source::Footage(item) => match cx.project.item(item).map(|i| &i.kind) {
            Some(ItemKind::Footage(f)) => f.path.clone(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    if !veymelo_media::is_vector(&path) {
        return Err(Error::params(format!("\"{}\" does not show a vector picture (an SVG)", source.name)));
    }
    let doc = veymelo_media::vector(&path).map_err(Error::not_found)?;
    let base = format!("{} Outlines", source.name);
    let name = if comp.layers.iter().any(|l| l.name == base) { super::fresh_name(&base, comp.layers.iter().map(|l| l.name.as_str())) } else { base };
    let index = comp.layers.iter().position(|l| l.id == lid).unwrap_or(0);
    let mut out = Vec::new();
    let count = items(cx, &doc.root, doc.root.transform, &mut out);
    let id = cx.project.alloc();
    let mut layer = Layer::new(id, &name, Source::Shape, source.out_point);
    (layer.in_point, layer.out_point, layer.start, layer.stretch) = (source.in_point, source.out_point, source.start, source.stretch);
    (layer.parent, layer.blend, layer.label) = (source.parent, source.blend, source.label);
    layer.switches = source.switches.clone();
    // where the picture stood (its corner at the layer's origin, as the picture's was)
    if let (Ok(to), Ok(from)) = (layer.props.group_mut("transform"), source.props.group("transform")) {
        *to = from.clone();
    }
    if let Ok(list) = layer.props.group_mut("contents") {
        list.children = out.into_iter().map(Node::Group).collect();
    }
    let comp = cx.project.comp_mut(cid).expect("found");
    comp.layer_mut(lid).expect("found").switches.video = false;
    comp.layers.insert(index, layer);
    cx.label = Some("Create Shapes from Vector Layer".into());
    let mut result = vec![("layer", Json::Number(id as f64)), ("name", Json::from(name.as_str())), ("shapes", Json::Number(count as f64))];
    if !doc.skipped.is_empty() {
        result.push(("skipped", Json::array(doc.skipped.iter().map(|s| Json::from(s.as_str())))));
    }
    Ok(Json::object(result))
}

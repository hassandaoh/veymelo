//! Masks: shapes in a layer's own space that say which of its pixels are seen. They are
//! combined top to bottom, each by its mode (add, subtract, intersect, lighten, darken,
//! difference), grown or shrunk by their expansion, softened by their feather (by different
//! amounts along the outline where it has softening points; smoothly or in a straight ramp),
//! inverted and faded as set; a moving outline is blurred over the shutter if asked, and
//! RotoBezier outlines make their own curves.

use veymelo_geom::{Mat3, vec2};
use veymelo_keyframe::{ShapePath, Value};
use veymelo_path::shape::{Sub, Vertex};
use veymelo_path::{Join, Rule};
use veymelo_project::{Group, Layer, Node, Property};
use veymelo_raster::Image;
use veymelo_time::Tick;

/// What a mask is read with: its values at a time, the time, and, where motion blur may be
/// drawn, the moments the shutter is open (whether a mask uses them is its own setting, or the
/// layer's switch).
pub(crate) struct Reading<'a> {
    pub read: &'a dyn Fn(&Property, Tick) -> Value,
    pub t: Tick,
    pub shutter: Option<Vec<Tick>>,
}

/// How much of each pixel a mask covers at time `t`, before its mode, inversion and opacity.
fn cover(g: &Group, r: &Reading, t: Tick, w: usize, h: usize, to_buf: Mat3, s: f64) -> Option<Vec<f32>> {
    let get = |p: &str| g.prop(p).ok().map(|q| (r.read)(q, t));
    let Some(Value::Path(mut path)) = get("path") else { return None };
    if get("rotoBezier").is_some_and(|v| v.as_bool()) {
        path = path.roto_smoothed();
    }
    let linear = get("featherFalloff").map_or(0, |v| v.as_choice()) == 1;
    let place = |p: [f64; 2]| {
        let q = to_buf.apply(vec2(p[0], p[1]));
        [q.x, q.y]
    };
    let fill = |path: &ShapePath, grow: f64| -> Vec<f32> {
        let sub = Sub {
            v: (0..path.points.len())
                .map(|k| Vertex { p: path.points[k], i: path.in_tangents.get(k).copied().unwrap_or([0.0; 2]), o: path.out_tangents.get(k).copied().unwrap_or([0.0; 2]) })
                .collect(),
            closed: true,
        };
        let mut shapes = vec![sub.map(&place)];
        if grow != 0.0 {
            shapes = veymelo_path::ops::offset(&shapes, grow, Join::Round, 4.0, 1.0, 0.0);
        }
        let mut cover = vec![0f32; w * h];
        if let Some(m) = veymelo_path::fill(&veymelo_path::shape::flatten(&shapes), Rule::NonZero, w, h) {
            for y in m.top..m.top + m.height {
                for x in m.left..m.left + m.width {
                    cover[y * w + x] = m.at(x, y);
                }
            }
        }
        cover
    };
    let grow = get("expansion").map_or(0.0, |v| v.as_f64()) * s;
    // softened by different amounts along it: grown and softened together
    let mut cover = if path.feather.is_empty() { fill(&path, grow) } else { veymelo_path::feather::coverage(&path, &place, s, w, h, grow, linear, &fill(&path, 0.0)) };
    let feather = get("feather").map_or([0.0; 2], |v| v.as_vec2());
    if feather[0] > 0.0 || feather[1] > 0.0 {
        if linear {
            veymelo_raster::blur::ramp(&mut cover, w, h, feather[0] * s, feather[1] * s);
        } else {
            veymelo_raster::blur::soften(&mut cover, w, h, feather[0] * s, feather[1] * s);
        }
    }
    Some(cover)
}

/// Cut `img` (whose pixel (0, 0) starts at `origin` in the layer's units, `s` pixels to a unit)
/// by the layer's masks, read as `r` says. Whether it had any.
pub(crate) fn apply(layer: &Layer, r: &Reading, img: &mut Image, origin: veymelo_geom::Vec2, s: f64) -> bool {
    let Ok(list) = layer.props.group("masks") else { return false };
    let masks: Vec<_> = list.children.iter().filter_map(|n| if let Node::Group(g) = n { Some(g) } else { None }).filter(|g| g.enabled).collect();
    let (w, h) = (img.width, img.height);
    let to_buf = Mat3::translate(vec2(-origin.x * s, -origin.y * s)) * Mat3::scale(vec2(s, s));
    let mut result: Option<Vec<f32>> = None;
    for g in masks {
        let get = |p: &str| g.prop(p).ok().map(|q| (r.read)(q, r.t));
        let mode = get("mode").map_or(1, |v| v.as_choice());
        if mode == 0 {
            continue;
        }
        // a moving outline blurred over the shutter: as the layer's switch says, always, or never
        let blurred = match get("motionBlur").map_or(0, |v| v.as_choice()) {
            0 => layer.switches.motion_blur,
            1 => true,
            _ => false,
        };
        let moves = g.prop("path").is_ok_and(|p| p.keys.len() > 1 || p.expression.as_ref().is_some_and(|x| x.enabled));
        let mut cover = match (&r.shutter, blurred && moves) {
            (Some(times), true) if times.len() > 1 => {
                let mut sum = vec![0f32; w * h];
                let mut n = 0;
                for &t in times {
                    if let Some(c) = cover(g, r, t, w, h, to_buf, s) {
                        sum.iter_mut().zip(&c).for_each(|(a, b)| *a += b);
                        n += 1;
                    }
                }
                if n == 0 {
                    continue;
                }
                sum.iter_mut().for_each(|v| *v /= n as f32);
                sum
            }
            _ => match cover(g, r, r.t, w, h, to_buf, s) {
                Some(c) => c,
                None => continue,
            },
        };
        let inverted = get("inverted").is_some_and(|v| v.as_bool());
        let fade = (get("opacity").map_or(100.0, |v| v.as_f64()) / 100.0) as f32;
        for c in &mut cover {
            if inverted {
                *c = 1.0 - *c;
            }
            *c *= fade;
        }
        // a first mask that subtracts takes away from all of the layer; any other starts from none
        let res = result.get_or_insert_with(|| vec![if mode == 2 { 1.0 } else { 0.0 }; w * h]);
        for (r, m) in res.iter_mut().zip(&cover) {
            *r = match mode {
                1 => *r + m - *r * m,
                2 => *r * (1.0 - m),
                3 => *r * m,
                4 => r.max(*m),
                5 => r.min(*m),
                _ => (*r - m).abs(),
            };
        }
    }
    let Some(res) = result else { return false };
    for (p, k) in img.px.iter_mut().zip(&res) {
        *p = p.map(|v| v * k);
    }
    true
}

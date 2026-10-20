//! The picture of a file whose channels are many layers (Blender's `ViewLayer.Combined.R`, its
//! compositor's `Image.R`, Nuke's `beauty.R`…): its unnamed layer with colour, else its beauty
//! pass by name, else the first layer with colour that is not a data pass (never Cryptomatte,
//! depth or normals), else a layer's first channel as grey. Luminance and chroma (`Y`, `RY`,
//! `BY`) are made red, green and blue with the weights of the file's primaries.

use crate::{Exr, Plane};

/// The layer a channel is of: its name before the last dot (none: the unnamed layer, "").
fn layer_of(name: &str) -> &str {
    name.rfind('.').map(|i| &name[..i]).unwrap_or("")
}

fn last_of(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

/// The layers of channels (by their names), in the order they first come.
pub fn layers<'a>(names: &[&'a str]) -> Vec<&'a str> {
    let mut v: Vec<&str> = Vec::new();
    for n in names {
        let l = layer_of(n);
        if !v.contains(&l) {
            v.push(l);
        }
    }
    v
}

/// A layer's red, green, blue and alpha channels, as places in `names` (`R`, `G`, `B`, `A` in
/// any case, or `red`, `green`, `blue`, `alpha`).
pub fn layer_rgba(names: &[&str], layer: &str) -> [Option<usize>; 4] {
    let colours = [["r", "red"], ["g", "green"], ["b", "blue"], ["a", "alpha"]];
    colours.map(|c| names.iter().position(|n| layer_of(n) == layer && c.iter().any(|c| last_of(n).eq_ignore_ascii_case(c))))
}

/// A channel of a layer by the last part of its name, in any case.
fn named(names: &[&str], layer: &str, name: &str) -> Option<usize> {
    names.iter().position(|n| layer_of(n) == layer && last_of(n).eq_ignore_ascii_case(name))
}

/// Names of the beauty pass, most liked first, as the last part of a layer's name: Blender's
/// Combined and its compositor's Image, the beauty, rgba, rgb and colour of Nuke, Arnold and
/// V-Ray.
const BEAUTY: [&str; 7] = ["combined", "beauty", "image", "rgba", "rgb", "color", "colour"];

/// A data pass, never the picture: Cryptomatte, depth, mist, normals, motion vectors,
/// positions, texture coordinates, object and material numbers.
fn is_data(layer: &str) -> bool {
    let last = last_of(layer).to_ascii_lowercase();
    last.starts_with("crypto")
        || ["z", "uv", "id", "mist", "indexob", "indexma"].contains(&last.as_str())
        || ["depth", "normal", "vector", "position", "velocity", "motion", "objectid", "materialid", "index"].iter().any(|k| last.contains(k))
}

/// The layer a file's picture is, of its channels' names (see the module's notes).
pub fn picture_layer(names: &[&str]) -> Option<String> {
    let all = layers(names);
    let colour: Vec<&str> = all.iter().copied().filter(|l| layer_rgba(names, l)[..3].iter().all(|c| c.is_some())).collect();
    let last = |l: &str| last_of(l).to_ascii_lowercase();
    let pictures = || colour.iter().filter(|l| !is_data(l));
    colour
        .iter()
        .find(|l| l.is_empty())
        .or_else(|| BEAUTY.iter().find_map(|k| colour.iter().find(|l| last(l) == *k)))
        .or_else(|| pictures().find(|l| ["combined", "beauty"].iter().any(|k| last(l).contains(k))))
        .or_else(|| pictures().next())
        .or_else(|| all.iter().find(|l| !is_data(l)))
        .or_else(|| colour.first())
        .or_else(|| all.first())
        .map(|l| l.to_string())
}

/// A file's picture: linear light, alpha as the file keeps it (OpenEXR's colours are
/// premultiplied), over the display window.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub px: Vec<[f32; 4]>,
    /// It has alpha (an alpha channel, or pixels outside its data window).
    pub alpha: bool,
    /// The layer it is ("" the unnamed one).
    pub layer: String,
    /// The red, green, blue and white its colours are made of (x, y each), when it says.
    pub chromaticities: Option<[[f32; 2]; 4]>,
}

/// How much red, green and blue make luminance, for these primaries and white.
fn luma_weights(c: [[f32; 2]; 4]) -> [f32; 3] {
    let xyz = |p: [f32; 2]| [p[0] as f64 / p[1] as f64, 1.0, (1.0 - p[0] as f64 - p[1] as f64) / p[1] as f64];
    let (r, g, b, w) = (xyz(c[0]), xyz(c[1]), xyz(c[2]), xyz(c[3]));
    let m = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
    let det = |m: [[f64; 3]; 3]| m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let d = det(m);
    // how much of each primary makes the white (Cramer's rule): each primary's luminance
    let s: [f64; 3] = std::array::from_fn(|k| {
        let mut mk = m;
        (0..3).for_each(|i| mk[i][k] = w[i]);
        det(mk) / d
    });
    let sum = s[0] + s[1] + s[2];
    s.map(|v| (v / sum) as f32)
}

/// The Rec. 709 primaries and D65 white.
const REC709: [[f32; 2]; 4] = [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06], [0.3127, 0.3290]];

/// The picture of a file read (see the module's notes); none when it has no channels.
pub fn picture(exr: &Exr, planes: &[Plane]) -> Option<Picture> {
    let names: Vec<&str> = planes.iter().map(|p| p.name.as_str()).collect();
    let layer = picture_layer(&names)?;
    let frame = exr.display();
    let (w, h) = (frame.width(), frame.height());
    let [r, g, b, a] = layer_rgba(&names, &layer);
    let first = names.iter().position(|n| layer_of(n) == layer)?;
    let part = planes[first].part;
    let chromaticities = exr.parts.get(part).and_then(|p| p.chromaticities());
    let get = |i: usize| planes[i].placed(frame, 0.0);
    let (r, g, b) = match (r, g, b) {
        (Some(r), Some(g), Some(b)) => (get(r), get(g), get(b)),
        _ => {
            let y = get(named(&names, &layer, "Y").unwrap_or(first));
            match (named(&names, &layer, "RY"), named(&names, &layer, "BY")) {
                (Some(ry), Some(by)) => {
                    let [wr, wg, wb] = luma_weights(chromaticities.unwrap_or(REC709));
                    let (ry, by) = (get(ry), get(by));
                    let (mut r, mut g, mut b) = (vec![0.0; w * h], vec![0.0; w * h], vec![0.0; w * h]);
                    for i in 0..w * h {
                        if ry[i] == 0.0 && by[i] == 0.0 {
                            (r[i], g[i], b[i]) = (y[i], y[i], y[i]);
                        } else {
                            r[i] = (ry[i] + 1.0) * y[i];
                            b[i] = (by[i] + 1.0) * y[i];
                            g[i] = (y[i] - r[i] * wr - b[i] * wb) / wg;
                        }
                    }
                    (r, g, b)
                }
                _ => (y.clone(), y.clone(), y),
            }
        }
    };
    // without an alpha channel: solid where the part has pixels
    let window = planes[first].window;
    let covers = window.x0 <= frame.x0 && window.y0 <= frame.y0 && window.x1 >= frame.x1 && window.y1 >= frame.y1;
    let alpha = a.is_some() || !covers;
    let a = match a {
        Some(a) => get(a),
        None => Plane { name: String::new(), part, window, samples: vec![1.0; window.width() * window.height()] }.placed(frame, 0.0),
    };
    let px = (0..w * h).map(|i| [r[i], g[i], b[i], a[i]]).collect();
    Some(Picture { width: w, height: h, px, alpha, layer, chromaticities })
}

/// A file's picture, decoding only the parts it is in.
pub fn read_picture(bytes: &[u8]) -> Result<Picture, String> {
    let exr = crate::parse(bytes)?;
    let names = exr.names();
    let refs: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
    let layer = picture_layer(&refs).ok_or("a file with no channels")?;
    let parts: Vec<usize> = names.iter().filter(|(n, _)| layer_of(n) == layer).map(|(_, k)| *k).collect();
    let mut planes = Vec::new();
    for k in 0..exr.parts.len() {
        if parts.contains(&k) {
            planes.extend(exr.named_planes(bytes, k)?.into_iter().filter(|p| layer_of(&p.name) == layer));
        }
    }
    picture(&exr, &planes).ok_or_else(|| "a file with no picture".into())
}

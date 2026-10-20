//! OpenEXR footage read by our own decoder (`veymelo-exr`), every compression there is but the
//! newest (the system reads neither DWAA nor DWAB): its picture — of a file of many layers, its
//! beauty pass — made sRGB as the system makes other pictures (extended past white,
//! premultiplied, its primaries made Rec. 709's), and its render passes (depth, object and
//! material numbers, normals, Cryptomatte…) for the 3D Channel effects.

use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use veymelo_project::AlphaMode;
use veymelo_raster::passes::{FAR, Passes};

/// Whether footage's file is an OpenEXR file.
pub fn is_exr(path: &str) -> bool {
    matches!(crate::ext_of(path).as_str(), "exr" | "sxr" | "mxr")
}

fn bytes(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))
}

/// What a file is, as the system says what other pictures are.
pub(crate) fn info(path: &str) -> Result<veymelo_os::PictureInfo, String> {
    let b = bytes(path)?;
    let exr = veymelo_exr::parse(&b).map_err(|e| format!("{path}: {e}"))?;
    let names = exr.names();
    let refs: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
    let layer = veymelo_exr::picture_layer(&refs).ok_or_else(|| format!("{path}: an OpenEXR file with no channels"))?;
    let d = exr.display();
    let alpha = veymelo_exr::layer_rgba(&refs, &layer)[3].is_some() || exr.parts.iter().any(|p| p.data.x0 > d.x0 || p.data.y0 > d.y0 || p.data.x1 < d.x1 || p.data.y1 < d.y1);
    // halves are 16 bits a channel, the rest 32
    let halves = names.iter().zip(exr.parts.iter().flat_map(|p| &p.channels)).filter(|((n, _), _)| n.rsplit_once('.').map_or("", |(l, _)| l) == layer).all(|(_, c)| c.sample == veymelo_exr::Sample::Half);
    let (width, height) = (d.width() as u32, d.height() as u32);
    Ok(veymelo_os::PictureInfo { width, height, frames: 1, delays: Vec::new(), alpha, orientation: 1, depth: if halves { 16 } else { 32 }, kind: "com.ilm.openexr-image".into() })
}

/// A linear value as sRGB, the same either side of nothing (as the system's extended sRGB).
fn encode(v: f32) -> f32 {
    let a = v.abs() as f64;
    let s = if a <= 0.003_130_8 { a * 12.92 } else { 1.055 * a.powf(1.0 / 2.4) - 0.055 };
    (s as f32).copysign(v)
}

/// The Rec. 709 primaries and D65 white, as `[[x, y]; 4]`.
const REC709: [[f64; 2]; 4] = [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06], [0.3127, 0.3290]];

/// From a file's primaries to Rec. 709's (none when they are those already).
fn to_rec709(c: [[f32; 2]; 4]) -> Option<veymelo_color::space::M3> {
    use veymelo_color::space::{bradford, invert, mul, rgb_to_xyz};
    let c = c.map(|p| [p[0] as f64, p[1] as f64]);
    if c.iter().zip(&REC709).all(|(a, b)| (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3) {
        return None;
    }
    let from = rgb_to_xyz([c[0], c[1], c[2]], c[3]);
    let to = invert(&rgb_to_xyz([REC709[0], REC709[1], REC709[2]], REC709[3]));
    Some(mul(&to, &mul(&bradford(c[3], REC709[3]), &from)))
}

/// The picture of the file at `path` as the system gives pictures: sRGB (extended),
/// premultiplied. Its alpha as `alpha` says: OpenEXR's colours are premultiplied (in linear
/// light: undone, made sRGB, done again); straight, they are multiplied once; left out, they
/// are as they are, solid.
pub(crate) fn picture(path: &str, alpha: AlphaMode) -> Result<veymelo_os::Picture, String> {
    let pic = veymelo_exr::read_picture(&bytes(path)?).map_err(|e| format!("{path}: {e}"))?;
    let m = pic.chromaticities.and_then(to_rec709);
    let px = pic
        .px
        .into_iter()
        .map(|[r, g, b, a]| {
            let a = if alpha == AlphaMode::Ignore { 1.0 } else { a.clamp(0.0, 1.0) };
            let k = if alpha == AlphaMode::Premultiplied && a > 0.0 { 1.0 / a } else { 1.0 };
            let mut c = [r * k, g * k, b * k];
            if let Some(m) = &m {
                let v = veymelo_color::space::apply(m, c.map(|v| v as f64));
                c = v.map(|v| v as f32);
            }
            let s = c.map(encode);
            [s[0] * a, s[1] * a, s[2] * a, a]
        })
        .collect();
    Ok(veymelo_os::Picture { width: pic.width, height: pic.height, px, alpha: pic.alpha })
}

/// Whether a channel is a depth (its depth where there is nothing is far away).
fn is_depth(name: &str) -> bool {
    let last = name.rsplit('.').next().unwrap_or(name);
    last.eq_ignore_ascii_case("z") || name.to_ascii_lowercase().contains("depth")
}

/// A Cryptomatte manifest (`{"name": "hex ID", …}`): its names and IDs, by name.
fn manifest(json: &str) -> Vec<(String, u32)> {
    let Ok(veymelo_json::Value::Object(m)) = veymelo_json::Value::parse(json) else { return Vec::new() };
    let mut v: Vec<(String, u32)> = m.iter().filter_map(|(k, h)| Some((k.to_string(), u32::from_str_radix(h.as_str()?.trim(), 16).ok()?))).collect();
    v.sort();
    v
}

/// The render passes of an OpenEXR file's bytes: every channel of every part named as layers,
/// over its display window (nothing outside a part's data window; far away, for a depth), and
/// the Cryptomatte manifests its headers carry (`cryptomatte/<key>/name` and `…/manifest`, as
/// Cryptomatte's specification has them).
pub fn passes_of(bytes: &[u8]) -> Result<Passes, String> {
    let (exr, planes) = veymelo_exr::read(bytes)?;
    let d = exr.display();
    let mut p = Passes::new(d.width(), d.height(), 1.0);
    for plane in planes {
        let fill = if is_depth(&plane.name) { FAR } else { 0.0 };
        let samples = plane.placed(d, fill);
        p.channels.push((plane.name, samples));
    }
    let mut kinds: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    for a in exr.parts.iter().flat_map(|p| &p.attributes) {
        let (Some(rest), veymelo_exr::Value::Text(t)) = (a.name.strip_prefix("cryptomatte/"), &a.value) else { continue };
        let Some((key, field)) = rest.split_once('/') else { continue };
        let at = match kinds.iter().position(|k| k.0 == key) {
            Some(at) => at,
            None => {
                kinds.push((key.to_string(), None, None));
                kinds.len() - 1
            }
        };
        match field {
            "name" => kinds[at].1 = Some(t.clone()),
            "manifest" => kinds[at].2 = Some(t.clone()),
            _ => {}
        }
    }
    for (_, name, m) in kinds {
        if let (Some(n), Some(m)) = (name, m) {
            p.manifests.push((n, manifest(&m)));
        }
    }
    p.manifests.sort();
    Ok(p)
}

type Kept = Vec<(String, Option<SystemTime>, Arc<Passes>)>;

/// The render passes of the OpenEXR file at `path`, read when first asked and kept (a few
/// files) while the file stays as it is.
pub fn passes(path: &str) -> Result<Arc<Passes>, String> {
    static KEPT: Mutex<Kept> = Mutex::new(Vec::new());
    let when = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if when.is_none() && !std::path::Path::new(path).exists() {
        return Err(format!("there is no file {path}"));
    }
    if let Some((_, _, p)) = KEPT.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(p, t, _)| p == path && *t == when) {
        return Ok(p.clone());
    }
    let p = Arc::new(passes_of(&bytes(path)?).map_err(|e| format!("{path}: {e}"))?);
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    kept.retain(|(q, _, _)| q != path);
    if kept.len() >= 4 {
        kept.remove(0);
    }
    kept.push((path.to_string(), when, p.clone()));
    Ok(p)
}

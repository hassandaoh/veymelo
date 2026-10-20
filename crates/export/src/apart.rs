//! A frame with its layers kept apart: each layer that shows drawn alone (as it is in the
//! composition, but at full opacity in Normal mode), written as a Photoshop file with a layer
//! for each, or as one multi-layer EXR.

use veymelo_color::{BlendMode, linear_to_srgb, srgb_to_linear};
use veymelo_project::{Project, Source};
use veymelo_raster::Image;
use veymelo_render::Options;
use veymelo_time::Tick;

use crate::{exr, psd};

/// One layer drawn alone: its name, blend mode, opacity (0 to 1), and its pixels as they are in
/// the frame (premultiplied, the frame's size).
pub struct Apart {
    pub name: String,
    pub blend: BlendMode,
    pub opacity: f64,
    pub image: Image,
}

/// The Photoshop key for a blend mode (Normal for those Photoshop has not).
pub fn blend_key(m: BlendMode) -> [u8; 4] {
    *match m {
        BlendMode::Dissolve | BlendMode::DancingDissolve => b"diss",
        BlendMode::Darken => b"dark",
        BlendMode::Multiply => b"mul ",
        BlendMode::ColorBurn | BlendMode::ClassicColorBurn => b"idiv",
        BlendMode::LinearBurn => b"lbrn",
        BlendMode::DarkerColor => b"dkCl",
        BlendMode::Lighten => b"lite",
        BlendMode::Screen => b"scrn",
        BlendMode::ColorDodge | BlendMode::ClassicColorDodge => b"div ",
        BlendMode::Add | BlendMode::LinearDodge => b"lddg",
        BlendMode::LighterColor => b"lgCl",
        BlendMode::Overlay => b"over",
        BlendMode::SoftLight => b"sLit",
        BlendMode::HardLight => b"hLit",
        BlendMode::VividLight => b"vLit",
        BlendMode::LinearLight => b"lLit",
        BlendMode::PinLight => b"pLit",
        BlendMode::HardMix => b"hMix",
        BlendMode::Difference | BlendMode::ClassicDifference => b"diff",
        BlendMode::Exclusion => b"smud",
        BlendMode::Subtract => b"fsub",
        BlendMode::Divide => b"fdiv",
        BlendMode::Hue => b"hue ",
        BlendMode::Saturation => b"sat ",
        BlendMode::Color => b"colr",
        BlendMode::Luminosity => b"lum ",
        _ => b"norm",
    }
}

/// The frame at composition time `t` drawn whole, and each layer that shows in it drawn alone,
/// from the top; and notes (what cannot be kept apart).
pub fn layers_apart(project: &Project, comp: u64, t: Tick, options: &Options) -> Result<(Image, Vec<Apart>, Vec<String>), String> {
    let c = project.comp(comp).ok_or_else(|| format!("there is no composition {comp}"))?;
    let whole = veymelo_render::render(project, comp, t, options)?;
    let mut notes = whole.notes;
    let solo = c.layers.iter().any(|l| l.switches.solo && l.switches.video);
    let mut out = Vec::new();
    for l in &c.layers {
        let shows = l.switches.video && l.is_active_at(t) && !l.switches.guide && (!solo || l.switches.solo) && !matches!(l.source, Source::Camera | Source::Light(_) | Source::Null);
        if !shows {
            continue;
        }
        if l.switches.adjustment {
            notes.push(format!("\"{}\" is an adjustment layer: it changes the layers under it and is not kept apart", l.name));
            continue;
        }
        let mut alone = project.clone();
        let Some(ac) = alone.comp_mut(comp) else { continue };
        let lt = l.layer_time(t);
        let opacity = l.props.prop("transform/opacity").map_or(100.0, |p| p.value_at(lt).as_f64()) / 100.0;
        for m in ac.layers.iter_mut() {
            m.switches.solo = m.id == l.id;
            if m.id == l.id {
                m.blend = BlendMode::Normal;
                if let Ok(p) = m.props.prop_mut("transform/opacity") {
                    p.keys.clear();
                    p.expression = None;
                    p.value = veymelo_project::Value::Number(100.0);
                }
            }
        }
        let drawn = veymelo_render::render(&alone, comp, t, &Options { background: false, solo: true, ..options.clone() })?;
        out.push(Apart { name: l.name.clone(), blend: l.blend, opacity: opacity.clamp(0.0, 1.0), image: drawn.image });
    }
    Ok((whole.image, out, notes))
}

/// A premultiplied pixel as straight colour seen on screen (from linear light when `linear`).
fn seen(p: [f32; 4], linear: bool) -> [f32; 4] {
    if p[3] <= 0.0 {
        return [0.0; 4];
    }
    let c = |v: f32| {
        let s = (v / p[3]).clamp(0.0, 1.0) as f64;
        (if linear { linear_to_srgb(s) } else { s }) as f32
    };
    [c(p[0]), c(p[1]), c(p[2]), p[3].min(1.0)]
}

/// Where a picture has pixels: [left, top, width, height] (none: nowhere).
fn inked(img: &Image) -> Option<[usize; 4]> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..img.height {
        for x in 0..img.width {
            if img.px[y * img.width + x][3] > 0.5 / 255.0 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    (x1 > x0).then_some([x0, y0, x1 - x0, y1 - y0])
}

/// A Photoshop file of the frame: a layer for each layer that shows, the whole frame merged.
pub fn psd_file(project: &Project, comp: u64, t: Tick, options: &Options) -> Result<(Vec<u8>, usize, Vec<String>), String> {
    let (whole, layers, notes) = layers_apart(project, comp, t, options)?;
    let linear = project.settings.linear_blending;
    let n = layers.len();
    // the file has them from the bottom
    let made: Vec<psd::Layer> = layers
        .into_iter()
        .rev()
        .map(|l| {
            let (rect, pixels) = match inked(&l.image) {
                Some([x, y, w, h]) => ([x as i32, y as i32, w as i32, h as i32], (y..y + h).flat_map(|r| (x..x + w).map(move |c| (r, c))).map(|(r, c)| seen(l.image.px[r * l.image.width + c], linear)).collect()),
                None => ([0; 4], Vec::new()),
            };
            psd::Layer { name: l.name, rect, pixels, blend: blend_key(l.blend), opacity: l.opacity, visible: true }
        })
        .collect();
    let merged: Vec<[f32; 4]> = whole.px.iter().map(|p| seen(*p, linear)).collect();
    Ok((psd::encode(whole.width, whole.height, &made, &merged), n, notes))
}

/// A premultiplied pixel in linear light (EXR's own), from a frame in linear light or not.
fn light(p: [f32; 4], linear: bool) -> [f32; 4] {
    if linear || p[3] <= 0.0 {
        return p;
    }
    let c = |v: f32| (srgb_to_linear((v / p[3]).clamp(0.0, 1.0) as f64) as f32) * p[3];
    [c(p[0]), c(p[1]), c(p[2]), p[3]]
}

/// The channels of a picture, named with `prefix` ("" for the whole picture's R, G, B, A).
fn channels_of(img: &Image, prefix: &str, linear: bool) -> Vec<exr::Channel> {
    let px: Vec<[f32; 4]> = img.px.iter().map(|p| light(*p, linear)).collect();
    ["R", "G", "B", "A"].iter().enumerate().map(|(k, c)| exr::Channel { name: format!("{prefix}{c}"), samples: px.iter().map(|p| p[k]).collect() }).collect()
}

/// An EXR file of a picture (premultiplied, in linear light).
pub fn exr_file(img: &Image, linear: bool, halves: bool) -> Vec<u8> {
    exr::encode(img.width, img.height, &channels_of(img, "", linear), halves)
}

/// A multi-layer EXR of the frame: the whole frame as R, G, B, A and each layer that shows as
/// its own "name.R" … (names made unique).
pub fn layered_exr(project: &Project, comp: u64, t: Tick, options: &Options, halves: bool) -> Result<(Vec<u8>, usize, Vec<String>), String> {
    let (whole, layers, notes) = layers_apart(project, comp, t, options)?;
    let linear = project.settings.linear_blending;
    let mut channels = channels_of(&whole, "", linear);
    let mut taken: Vec<String> = Vec::new();
    for l in &layers {
        // channel names keep away from the dot and from each other
        let base: String = l.name.chars().map(|c| if c == '.' || c.is_control() { '_' } else { c }).collect();
        let name = (1..).map(|k| if k == 1 { base.clone() } else { format!("{base} {k}") }).find(|n| !taken.contains(n)).expect("a free name");
        taken.push(name.clone());
        channels.extend(channels_of(&l.image, &format!("{name}."), linear));
    }
    Ok((exr::encode(whole.width, whole.height, &channels, halves), layers.len(), notes))
}

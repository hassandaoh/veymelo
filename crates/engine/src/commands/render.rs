//! Drawing frames.

use veymelo_json::Value as Json;
use veymelo_render::{Options, Rendered};

use super::{COMP, Command, Run, comp_id};
use crate::{Args, Error, Look, Param, Result, Ty};

pub fn commands() -> Vec<Command> {
    vec![Command {
        id: "render.frame",
        label: "Render Frame",
        summary: "Draw one frame of a composition and write it as a picture, of the kind its file name ends in: .png, .jpg, .tif, .exr (floating point, in linear light; with `layers`, each layer its own channels too) or .psd (a Photoshop file with a layer for each layer that shows). Notes say what could not be drawn yet.",
        params: const {
            &[
                COMP,
                Param::opt("time", Ty::Time, "when (default: the view's time)"),
                Param::opt("scale", Ty::Number, "the size to draw at, 1 for the composition's own (default 1)"),
                Param::opt("background", Ty::Bool, "draw the background colour (default true; false leaves it clear)"),
                Param::opt("motionBlur", Ty::Bool, "blur moving layers that have motion blur on (default true)"),
                Param::opt("guides", Ty::Bool, "draw guide layers too (default false: renders leave them out)"),
                Param::need("path", Ty::Text, "the file to write: .png, .jpg, .tif, .exr or .psd"),
                Param::opt("layers", Ty::Bool, ".exr: each layer that shows as its own channels too (\"Title.R\" …)"),
                Param::opt("half", Ty::Bool, ".exr: 16-bit halves instead of 32-bit floats"),
            ]
        },
        returns: "{path, width, height, time, layers?, notes}",
        run: Run::Look(render_frame),
    }]
}

/// Draw the frame the parameters ask for.
fn draw(l: &Look, a: &Args) -> Result<(Rendered, f64)> {
    let cid = comp_id(l.project, l.view, a)?;
    let t = a.time("time").unwrap_or(l.view.time);
    let options = Options {
        scale: a.num("scale").unwrap_or(1.0),
        background: a.bool("background").unwrap_or(true),
        motion_blur: a.bool("motionBlur").unwrap_or(true),
        guides: a.bool("guides").unwrap_or(false),
        ..Options::default()
    };
    let r = veymelo_render::render(l.project, cid, t, &options).map_err(Error::params)?;
    Ok((r, t.seconds()))
}

fn render_frame(l: &Look, a: &Args) -> Result<Json> {
    let path = a.text("path").unwrap_or("").to_string();
    let ending = std::path::Path::new(&path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let write = |bytes: Vec<u8>| std::fs::write(&path, bytes).map_err(|e| Error::not_found(format!("cannot write {path}: {e}")));
    let (size, t, layers, notes) = match ending.as_str() {
        "psd" | "exr" if ending == "psd" || a.bool("layers") == Some(true) => {
            let cid = comp_id(l.project, l.view, a)?;
            let t = a.time("time").unwrap_or(l.view.time);
            let options = Options { scale: a.num("scale").unwrap_or(1.0), background: a.bool("background").unwrap_or(true), motion_blur: a.bool("motionBlur").unwrap_or(true), guides: a.bool("guides").unwrap_or(false), ..Options::default() };
            let comp = l.project.comp(cid).expect("found");
            let size = [((comp.width as f64 * options.scale).round() as usize).max(1), ((comp.height as f64 * options.scale).round() as usize).max(1)];
            let (bytes, n, notes) = if ending == "psd" {
                veymelo_export::apart::psd_file(l.project, cid, t, &options)
            } else {
                veymelo_export::apart::layered_exr(l.project, cid, t, &options, a.bool("half") == Some(true))
            }
            .map_err(Error::params)?;
            write(bytes)?;
            (size, t.seconds(), Some(n), notes)
        }
        _ => {
            let (r, t) = draw(l, a)?;
            let img = &r.image;
            let bytes = match ending.as_str() {
                "png" => veymelo_render::to_png(img),
                "exr" => veymelo_export::apart::exr_file(img, l.project.settings.linear_blending, a.bool("half") == Some(true)),
                "jpg" | "jpeg" | "tif" | "tiff" => {
                    veymelo_export::write_picture(img, &path, ending.starts_with('t')).map_err(Error::refused)?;
                    Vec::new()
                }
                _ => return Err(Error::params(format!("{path}: a frame is written as .png, .jpg, .tif, .exr or .psd"))),
            };
            if !bytes.is_empty() {
                write(bytes)?;
            }
            ([img.width, img.height], t, None, r.notes)
        }
    };
    let mut o = veymelo_json::Map::new();
    o.insert("path", path.as_str());
    o.insert("width", size[0] as f64);
    o.insert("height", size[1] as f64);
    o.insert("time", t);
    if let Some(n) = layers {
        o.insert("layers", n as f64);
    }
    o.insert("notes", Json::array(notes));
    Ok(Json::Object(o))
}

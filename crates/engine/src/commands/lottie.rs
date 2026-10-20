//! Commands for Lottie, the animation format web, phone and app players draw: a composition
//! out as a `.json` document or a `.lottie` archive, and one in as new compositions.

use std::path::{Path, PathBuf};

use veymelo_json::{Map, Value as Json};
use veymelo_lottie::Options;
use veymelo_time::Tick;

use super::{COMP, Command, Run, comp_id};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "lottie.export",
            label: "Export Lottie",
            summary: "Write a composition as Lottie: a .json document, or a .lottie archive (by the path's ending). Compositions inside go along, pictures inside it (or beside it in images/), sounds inside it as sound layers (their levels as Lottie's %), text as text or as the outlines of its letters. What Lottie cannot hold comes back as warnings.",
            params: const {
                &[
                    COMP,
                    Param::need("path", Ty::Text, "where it goes: something.json or something.lottie"),
                    Param::opt("expressions", Ty::Bool, "send expressions along (players that run expressions run them; default off)"),
                    Param::opt("textAsShapes", Ty::Bool, "text as the outlines of its letters: exact, no font needed, but its words as they are at the start (default off)"),
                    Param::opt("embedImages", Ty::Bool, "pictures inside the document (default on); off: copied beside it into images/"),
                    Param::opt("workArea", Ty::Bool, "only the work area plays (default off: all of it)"),
                    Param::opt("background", Ty::Bool, "the composition's background colour as a solid under everything (Lottie has none; default off)"),
                ]
            },
            returns: "{path, bytes, layers, warnings, images?}",
            run: Run::Look(lottie_export),
        },
        Command {
            id: "lottie.import",
            label: "Import Lottie",
            summary: "Read a Lottie file (.json, or every animation of a .lottie archive) as new compositions, its precomps and pictures in a folder of their own; the view goes to the first. What could not be read comes back as warnings.",
            params: const {
                &[
                    Param::need("path", Ty::Text, "the Lottie file"),
                    Param::opt("name", Ty::Text, "the composition's name when the file does not give one"),
                    Param::opt("pictures", Ty::Text, "the folder to keep the pictures the file carries in (default: \"<its name> pictures\" beside it)"),
                ]
            },
            returns: "{comp, comps, items, warnings}",
            run: Run::Edit(lottie_import),
        },
    ]
}

fn lottie_export(l: &Look, a: &Args) -> Result<Json> {
    let comp = comp_id(l.project, l.view, a)?;
    let path = PathBuf::from(a.text("path").unwrap_or(""));
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let packed = match ext.as_str() {
        "lottie" => true,
        "json" => false,
        _ => return Err(Error::params(format!("a Lottie file ends in .json or .lottie, not \"{}\"", path.display()))),
    };
    let flag = |n: &str, d: bool| a.bool(n).unwrap_or(d);
    let options = Options { expressions: flag("expressions", false), text_as_shapes: flag("textAsShapes", false), embed_images: flag("embedImages", true), work_area: flag("workArea", false), background: flag("background", false) };
    let e = veymelo_lottie::export(l.project, comp, &options, &|p| std::fs::read(p).ok()).map_err(Error::refused)?;
    let mut warnings = e.warnings.clone();
    // the pictures it names beside it
    let mut pictures = Vec::new();
    for (name, from) in &e.images {
        match std::fs::read(from) {
            Ok(bytes) => pictures.push((name.clone(), bytes)),
            Err(err) => warnings.push(format!("the picture {from} could not be read ({err}); the document names it but it is not beside it")),
        }
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|err| Error::refused(format!("{}: {err}", dir.display())))?;
    }
    let bytes = if packed {
        let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("animation");
        veymelo_lottie::dotlottie(&e, id, &pictures)
    } else {
        if !pictures.is_empty() {
            let dir = path.parent().unwrap_or(Path::new("")).join("images");
            std::fs::create_dir_all(&dir).map_err(|err| Error::refused(format!("{}: {err}", dir.display())))?;
            for (name, bytes) in &pictures {
                std::fs::write(dir.join(name), bytes).map_err(|err| Error::refused(format!("{name}: {err}")))?;
            }
        }
        e.json.to_json().into_bytes()
    };
    std::fs::write(&path, &bytes).map_err(|err| Error::refused(format!("{}: {err}", path.display())))?;
    let mut o = Map::new();
    o.insert("path", path.to_string_lossy().as_ref());
    o.insert("bytes", bytes.len() as f64);
    o.insert("layers", e.json.get("layers").and_then(Json::as_array).map_or(0, Vec::len) as f64);
    o.insert("warnings", Json::array(warnings.iter().map(|w| Json::from(w.as_str()))));
    if !e.images.is_empty() {
        o.insert("images", Json::array(e.images.iter().map(|(n, _)| Json::from(n.as_str()))));
    }
    Ok(Json::Object(o))
}

fn lottie_import(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let path = PathBuf::from(a.text("path").unwrap_or(""));
    let bytes = std::fs::read(&path).map_err(|e| Error::not_found(format!("{}: {e}", path.display())))?;
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Lottie").to_string();
    let name = a.text("name").map_or_else(|| stem.clone(), str::to_string);
    let beside = path.parent().map(Path::to_path_buf);
    let pictures = a.text("pictures").map(PathBuf::from).unwrap_or_else(|| beside.clone().unwrap_or_default().join(format!("{stem} pictures")));
    let mut failed: Vec<String> = Vec::new();
    let mut store = |pic: &veymelo_lottie::Picture| -> Option<String> {
        let safe: String = pic.id.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        let file = pictures.join(format!("{safe}.{}", pic.ext));
        let kept = std::fs::create_dir_all(&pictures).and_then(|_| std::fs::write(&file, &pic.bytes));
        match kept {
            Ok(()) => Some(file.to_string_lossy().into_owned()),
            Err(e) => {
                failed.push(format!("{}: {e}", file.display()));
                None
            }
        }
    };
    let im = veymelo_lottie::import(cx.project, &bytes, &name, beside.as_deref(), &mut store).map_err(Error::refused)?;
    let mut warnings = im.warnings;
    warnings.extend(failed);
    cx.view.comp = Some(im.comp);
    cx.view.time = Tick::ZERO;
    cx.label = Some(format!("Import Lottie {stem}"));
    let ids = |v: &[u64]| Json::array(v.iter().map(|i| Json::Number(*i as f64)));
    Ok(Json::object([("comp", Json::Number(im.comp as f64)), ("comps", ids(&im.comps)), ("items", ids(&im.items)), ("warnings", Json::array(warnings.iter().map(|w| Json::from(w.as_str()))))]))
}

//! A composition made into a file and used in its place (pre-rendered), and proxies: files drawn
//! in place of a composition or footage while they are switched on: a smaller stand-in to work
//! with, or a render of what is slow to draw.

use veymelo_json::{Map, Value as Json};
use veymelo_project::queue::{After, Format, RenderItem, Span};
use veymelo_project::{Footage, FootageKind, ItemKind, Project, Proxy, Source};
use veymelo_time::Tick;

use super::items::find_item;
use super::{COMP, Command, Run, comp_id};
use crate::{Args, Ctx, Error, Param, Result, Ty};

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "comp.preRender",
            label: "Pre-render",
            summary: "Make a composition into a file now and use it in its place: by default the file comes in as footage and takes the composition's place in every composition that uses it (lined up in time), so what was slow to draw is played back. Or it only comes in (import), or becomes the composition's proxy (setProxy). With `queue`, it waits in the render queue and this happens when queue.render makes it.",
            params: const {
                &[
                    COMP,
                    Param::need("path", Ty::Text, "where the file goes (a ProRes .mov keeps its transparency; frames/shot_####.png makes a sequence)"),
                    Param::opt("format", Ty::Choice(Format::IDS), "the kind of file (default prores, 4444 with alpha)"),
                    Param::opt("after", Ty::Choice(&["importReplace", "import", "setProxy"]), "what the file does once made (default importReplace)"),
                    Param::opt("span", Ty::Choice(&["comp", "workArea"]), "all of it (the default) or its work area"),
                    Param::opt("queue", Ty::Bool, "put it in the render queue instead of making it now"),
                    Param::opt("settings", Ty::Object, "how it is drawn, as queue.add takes it"),
                    Param::opt("output", Ty::Object, "what file it becomes, as queue.add takes it"),
                ]
            },
            returns: "{files, frames, size, footage, replaced?, proxy?, notes?} or {item} with queue",
            run: Run::Edit(pre_render),
        },
        Command {
            id: "project.proxy.set",
            label: "Set Proxy",
            summary: "Give a composition or footage a proxy: a file drawn in its place (fitted to its size) while the proxy is switched on. The view uses proxies that are on; renders use them as their settings say (proxies: none, current, all, comps). Without `path`, switch the proxy on or off; with `path` null, take it away.",
            params: const {
                &[
                    Param::need("item", Ty::Any, "the composition or footage (its id or name)"),
                    Param::opt("path", Ty::Any, "the proxy's file, or null for none"),
                    Param::opt("sequence", Ty::Bool, "the file is one of a numbered sequence of pictures"),
                    Param::opt("on", Ty::Bool, "drawn in its place (default on when a file is given)"),
                ]
            },
            returns: "{item, proxy}",
            run: Run::Edit(proxy_set),
        },
        Command {
            id: "project.proxy.create",
            label: "Create Proxy",
            summary: "Make a smaller stand-in of a composition or footage and set it as its proxy, switched on: a movie (ProRes, keeping transparency) or one still picture (PNG) at a time.",
            params: const {
                &[
                    Param::need("item", Ty::Any, "the composition or footage (its id or name)"),
                    Param::need("path", Ty::Text, "where the proxy goes (.mov for a movie, .png for a still)"),
                    Param::opt("resolution", Ty::Number, "its size against the original: 0.5 half (the default), 0.25 a quarter"),
                    Param::opt("time", Ty::Time, "for a still: the moment it shows (default 0)"),
                ]
            },
            returns: "{item, proxy, files, frames, size}",
            run: Run::Edit(proxy_create),
        },
    ]
}

fn proxy_json(p: &Proxy) -> Json {
    Json::object([
        ("path", Json::from(p.footage.path.as_str())),
        ("kind", Json::from(p.footage.kind.id())),
        ("size", Json::array([Json::Number(p.footage.width as f64), Json::Number(p.footage.height as f64)])),
        ("duration", Json::Number(p.footage.duration.seconds())),
        ("on", Json::from(p.on)),
    ])
}

/// The footage a made file is, at the rate it was made at.
fn footage_of(item: &RenderItem, files: &[String], rate: veymelo_time::FrameRate) -> Result<Footage> {
    let first = files.first().ok_or_else(|| Error::refused("no file was made"))?;
    let sequence = item.output.format.is_sequence();
    let mut f = veymelo_media::probe(first, sequence).map_err(Error::refused)?;
    if f.kind == FootageKind::Sequence {
        let frames = files.len() as i64;
        f.frame_rate = rate;
        f.duration = rate.tick_of(frames);
    }
    Ok(f)
}

/// Where the made part starts in the composition.
fn span_start(project: &Project, item: &RenderItem) -> Tick {
    let comp = project.comp(item.comp).expect("found");
    match item.settings.span {
        Span::Comp => Tick::ZERO,
        Span::WorkArea => comp.work_area.start,
        Span::Custom => item.settings.start,
    }
}

/// What a made file does next (`files` from making `item`): comes in as footage, takes the
/// composition's place, or becomes its proxy. None when it does nothing.
pub(super) fn after_made(project: &mut Project, item: &RenderItem, files: &[String]) -> Result<Option<Json>> {
    if item.after == After::Nothing {
        return Ok(None);
    }
    let Some(comp) = project.comp(item.comp) else { return Err(Error::not_found(format!("there is no composition {}", item.comp))) };
    let rate = match item.settings.frame_rate.and_then(veymelo_time::FrameRate::from_f64) {
        Some(r) => r,
        None => comp.frame_rate,
    };
    let f = footage_of(item, files, rate)?;
    let offset = span_start(project, item);
    let mut out = Map::new();
    let mut notes = Vec::new();
    if item.after == After::SetProxy {
        if offset != Tick::ZERO {
            notes.push(format!("the render starts {} s in, but a proxy is drawn from the composition's start", offset.seconds()));
        }
        let p = Proxy { footage: f, on: true };
        out.insert("proxy", proxy_json(&p));
        project.item_mut(item.comp).expect("found").proxy = Some(p);
    } else {
        let (name, folder) = {
            let it = project.item(item.comp).expect("found");
            let file = std::path::Path::new(&f.path).file_name().map_or_else(|| f.path.clone(), |n| n.to_string_lossy().replace('#', ""));
            (file, it.folder)
        };
        let id = project.add(&name, folder, ItemKind::Footage(f));
        out.insert("footage", id as f64);
        if item.after == After::ImportReplace {
            let mut replaced = 0usize;
            let comps: Vec<u64> = project.items.iter().filter(|i| i.as_comp().is_some_and(|c| c.layers.iter().any(|l| l.source == Source::Comp(item.comp)))).map(|i| i.id).collect();
            for cid in comps {
                for l in &mut project.comp_mut(cid).expect("found").layers {
                    if l.source != Source::Comp(item.comp) {
                        continue;
                    }
                    l.source = Source::Footage(id);
                    if offset != Tick::ZERO {
                        // the file's first frame is the composition at `offset`
                        if l.time_remap {
                            if let Ok(p) = l.props.prop_mut("timeRemap") {
                                let shift = |v: &veymelo_project::Value| match v {
                                    veymelo_project::Value::Number(x) => veymelo_project::Value::Number(x - offset.seconds()),
                                    other => other.clone(),
                                };
                                p.value = shift(&p.value);
                                p.keys.iter_mut().for_each(|k| k.value = shift(&k.value));
                            }
                        } else {
                            l.start = l.comp_time(offset);
                        }
                    }
                    replaced += 1;
                }
            }
            out.insert("replaced", replaced as f64);
        }
    }
    if !notes.is_empty() {
        out.insert("notes", Json::array(notes));
    }
    Ok(Some(Json::Object(out)))
}

fn pre_render(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let comp = comp_id(cx.project, cx.view, a)?;
    let mut item = RenderItem::new(cx.project.alloc(), comp);
    // what keeps it as it was: every pixel, its transparency, its sound
    item.output.format = Format::ProRes;
    item.output.prores = veymelo_project::queue::ProRes::P4444;
    item.output.channels = veymelo_project::queue::Channels::Rgba;
    item.settings.span = if a.text("span") == Some("workArea") { Span::WorkArea } else { Span::Comp };
    item.after = match a.text("after") {
        Some("import") => After::Import,
        Some("setProxy") => After::SetProxy,
        _ => After::ImportReplace,
    };
    let item = super::queue::item_from(a, &item)?;
    let name = cx.project.item(comp).expect("found").name.clone();
    if a.bool("queue").unwrap_or(false) {
        let id = item.id;
        cx.project.queue.push(item);
        cx.label = Some(format!("Pre-render {name}"));
        return Ok(Json::object([("item", Json::Number(id as f64))]));
    }
    let m = veymelo_export::make(cx.project, &item, &mut |_, _| true).map_err(Error::refused)?;
    let mut o = super::queue::made_json(&m);
    if let Some(Json::Object(next)) = after_made(cx.project, &item, &m.files)? {
        for (k, v) in next.iter() {
            o.insert(k, v.clone());
        }
    }
    cx.label = Some(format!("Pre-render {name}"));
    Ok(Json::Object(o))
}

/// The composition or footage `item` names.
fn proxied(project: &Project, a: &Args) -> Result<u64> {
    let id = find_item(project, a.need("item"))?;
    match &project.item(id).expect("found").kind {
        ItemKind::Comp(_) => Ok(id),
        ItemKind::Footage(f) if f.kind != FootageKind::Audio => Ok(id),
        ItemKind::Footage(_) => Err(Error::refused("sound is not drawn, so it takes no proxy")),
        ItemKind::Folder => Err(Error::refused("a folder takes no proxy")),
    }
}

fn proxy_set(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let id = proxied(cx.project, a)?;
    let item = cx.project.item_mut(id).expect("found");
    let name = item.name.clone();
    match a.json("path") {
        Some(Json::Null) => {
            item.proxy = None;
            cx.label = Some(format!("Remove Proxy of {name}"));
            return Ok(Json::object([("item", Json::Number(id as f64)), ("proxy", Json::Null)]));
        }
        Some(Json::String(path)) => {
            if !veymelo_media::importable(path) {
                return Err(Error::params(format!("{path}: not a picture or video file")));
            }
            let f = veymelo_media::probe(path, a.bool("sequence").unwrap_or(false)).map_err(Error::not_found)?;
            if f.kind == FootageKind::Audio {
                return Err(Error::params("a proxy is something to see, not sound"));
            }
            item.proxy = Some(Proxy { footage: f, on: a.bool("on").unwrap_or(true) });
        }
        Some(other) => return Err(Error::params(format!("`path` is a file's path or null, not {}", other.to_json()))),
        None => {
            let p = item.proxy.as_mut().ok_or_else(|| Error::refused(format!("\"{name}\" has no proxy (give `path`)")))?;
            if let Some(on) = a.bool("on") {
                p.on = on;
            }
        }
    }
    let p = item.proxy.as_ref().expect("set");
    let out = Json::object([("item", Json::Number(id as f64)), ("proxy", proxy_json(p))]);
    cx.label = Some(format!("Set Proxy of {name}"));
    Ok(out)
}

fn proxy_create(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let id = proxied(cx.project, a)?;
    let res = a.num("resolution").unwrap_or(0.5);
    if !(res > 0.0 && res <= 1.0) {
        return Err(Error::params(format!("`resolution` is above 0 and at most 1, not {res}")));
    }
    let path = a.text("path").unwrap_or("").to_string();
    let still = path.to_ascii_lowercase().ends_with(".png");
    if !still && !path.to_ascii_lowercase().ends_with(".mov") {
        return Err(Error::params("a proxy is a .mov movie or a .png still"));
    }
    // footage is drawn through a composition of its own, made for the moment
    let mut work = cx.project.clone();
    let comp = match &work.item(id).expect("found").kind {
        ItemKind::Comp(_) => id,
        ItemKind::Footage(f) => {
            let f = f.clone();
            let (w, h) = ((f.width as f64 * f.pixel_aspect).round().max(1.0) as u32, f.height.max(1));
            let mut c = veymelo_project::Comp::new(w, h, f.frame_rate, if f.duration > Tick::ZERO { f.duration } else { f.frame_rate.frame_duration() });
            c.background = veymelo_color::Rgba::CLEAR;
            let cid = work.add("proxy", None, ItemKind::Comp(std::sync::Arc::new(c)));
            let lid = work.alloc();
            let length = work.comp(cid).expect("made").duration;
            let mut l = veymelo_project::Layer::new(lid, "footage", Source::Footage(id), length);
            l.out_point = length;
            if let Ok(p) = l.props.prop_mut("transform/position") {
                p.value = veymelo_project::Value::Vec3([w as f64 / 2.0, h as f64 / 2.0, 0.0]);
            }
            if let Ok(p) = l.props.prop_mut("transform/anchor") {
                p.value = veymelo_project::Value::Vec3([w as f64 / 2.0, h as f64 / 2.0, 0.0]);
            }
            work.comp_mut(cid).expect("made").layers.push(l);
            cid
        }
        ItemKind::Folder => unreachable!("not proxied"),
    };
    // the original drawn, not a proxy it has now
    work.item_mut(id).expect("found").proxy = None;
    let mut item = RenderItem::new(0, comp);
    item.settings.resolution = res;
    item.settings.span = if still { Span::Custom } else { Span::Comp };
    if still {
        let t = a.time("time").unwrap_or(Tick::ZERO);
        let frame = work.comp(comp).expect("found").frame_rate.frame_duration();
        (item.settings.start, item.settings.end) = (t, t + frame);
    }
    item.output.format = if still { Format::PngSequence } else { Format::ProRes };
    item.output.prores = veymelo_project::queue::ProRes::P4444;
    item.output.channels = veymelo_project::queue::Channels::Rgba;
    item.output.audio = false;
    item.output.path = if still { path.replace(".png", "_#.png").replace(".PNG", "_#.PNG") } else { path.clone() };
    let m = veymelo_export::make(&work, &item, &mut |_, _| true).map_err(Error::refused)?;
    let file = if still {
        // one picture, under the name asked for
        let made = m.files.first().cloned().ok_or_else(|| Error::refused("no picture was made"))?;
        std::fs::rename(&made, &path).map_err(|e| Error::refused(format!("{path}: {e}")))?;
        path.clone()
    } else {
        m.files.first().cloned().ok_or_else(|| Error::refused("no file was made"))?
    };
    let f = veymelo_media::probe(&file, false).map_err(Error::refused)?;
    let p = Proxy { footage: f, on: true };
    let mut o = super::queue::made_json(&m);
    o.insert("files", Json::array([Json::from(file.as_str())]));
    o.insert("item", id as f64);
    o.insert("proxy", proxy_json(&p));
    let it = cx.project.item_mut(id).expect("found");
    let name = it.name.clone();
    it.proxy = Some(p);
    cx.label = Some(format!("Create Proxy of {name}"));
    Ok(Json::Object(o))
}

//! Commands about text layers: their words and look, and animators with their selectors.

use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::{Kind, Value};
use veymelo_project::{Group, Layer, Node, Source};

use super::{COMP, Command, LAYER, Run, fresh_name, layer_ids};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "text.set",
            label: "Set Text",
            summary: "Change a text layer's words, look or paragraphs; what is not given stays as it is. With `range` [start, end] (characters), the look changes only there and the paragraph settings only for the paragraphs it touches; `text` with a range replaces those characters. To set the whole document, or a keyframe of it, use prop.set on text/sourceText.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::opt("range", Ty::List, "[start, end]: the characters it is for (default: all of them)"),
                    Param::opt("text", Ty::Text, "the words (a new line starts a new paragraph)"),
                    Param::opt("font", Ty::Text, "a font's family (\"Georgia\"), full or PostScript name, or a font file's path; Thai the font lacks comes from a Thai font"),
                    Param::opt("weight", Ty::Number, "100 (thin) to 900 (black); a variable font takes it exactly"),
                    Param::opt("italic", Ty::Bool, "slanted"),
                    Param::opt("size", Ty::Number, "in pixels"),
                    Param::opt("fill", Ty::Any, "the colour, or null for no fill"),
                    Param::opt("stroke", Ty::Any, "the stroke colour, or null for none"),
                    Param::opt("strokeWidth", Ty::Number, "in pixels"),
                    Param::opt("tracking", Ty::Number, "room between letters, in thousandths of the size"),
                    Param::opt("leading", Ty::Any, "baseline to baseline in pixels, or \"auto\" (1.2 times the size)"),
                    Param::opt("baselineShift", Ty::Number, "raised (or below zero, lowered), in pixels"),
                    Param::opt("caps", Ty::Choice(veymelo_keyframe::Caps::IDS), "capitals"),
                    Param::opt("hScale", Ty::Number, "letters stretched across, in percent"),
                    Param::opt("vScale", Ty::Number, "letters stretched up, in percent"),
                    Param::opt("tsume", Ty::Number, "the room either side of each letter taken in, 0 to 100 (for CJK)"),
                    Param::opt("fauxBold", Ty::Bool, "made heavier by drawing"),
                    Param::opt("fauxItalic", Ty::Bool, "slanted by drawing"),
                    Param::opt("position", Ty::Choice(veymelo_keyframe::Position::IDS), "superscript or subscript: small and raised or lowered"),
                    Param::opt("kerning", Ty::Any, "\"metrics\" (the font's), \"optical\" (by the letters' shapes) or a number (thousandths of the size, before each letter)"),
                    Param::opt("ligatures", Ty::Bool, "letters the font joins, joined (default on)"),
                    Param::opt("features", Ty::List, "OpenType features asked for: [\"onum\", \"smcp\", \"ss01\"…] (text.fontFeatures lists a font's)"),
                    Param::opt("axes", Ty::Object, "a variable font's axes beyond weight: {\"wdth\": 80, \"opsz\": 24}"),
                    Param::opt("underline", Ty::Bool, "a line under the letters"),
                    Param::opt("strikethrough", Ty::Bool, "a line through the letters"),
                    Param::opt("justify", Ty::Choice(veymelo_keyframe::Justify::IDS), "how lines sit"),
                    Param::opt("indentLeft", Ty::Number, "room kept at the left of each line, pixels"),
                    Param::opt("indentRight", Ty::Number, "room kept at the right, pixels"),
                    Param::opt("indentFirst", Ty::Number, "the first line's extra indent, pixels"),
                    Param::opt("spaceBefore", Ty::Number, "room before a paragraph, pixels"),
                    Param::opt("spaceAfter", Ty::Number, "room after a paragraph, pixels"),
                    Param::opt("direction", Ty::Choice(veymelo_keyframe::Direction::IDS), "which way paragraphs go: auto (as their first letter), ltr or rtl"),
                    Param::opt("composer", Ty::Choice(veymelo_keyframe::Composer::IDS), "lines broken for the whole paragraph's evenness (everyLine) or each as full as it goes (singleLine)"),
                    Param::opt("hanging", Ty::Bool, "punctuation at the ends of lines hangs past the edges"),
                    Param::opt("strokeOver", Ty::Bool, "the stroke over the fill (else under it)"),
                    Param::opt("box", Ty::Any, "[width, height]: paragraph text wrapping in a box whose top left is the layer's origin; null: point text"),
                    Param::opt("vertical", Ty::Bool, "written down in columns, the columns right to left"),
                ]
            },
            returns: "{text}",
            run: Run::Edit(text_set),
        },
        Command {
            id: "text.insert",
            label: "Type Text",
            summary: "Put words into a text layer at a place (or in place of a range): they take the look of the characters they go among, and paragraphs they make take the setting of the one they are in.",
            params: const { &[COMP, LAYER, Param::need("text", Ty::Text, "the words"), Param::opt("at", Ty::Whole, "the character they go before (default: the end)"), Param::opt("range", Ty::List, "[start, end]: the characters they replace")] },
            returns: "{text, range}",
            run: Run::Edit(text_insert),
        },
        Command {
            id: "text.delete",
            label: "Delete Text",
            summary: "Take characters out of a text layer.",
            params: const { &[COMP, LAYER, Param::need("range", Ty::List, "[start, end]: the characters to take out")] },
            returns: "{text}",
            run: Run::Edit(text_delete),
        },
        Command {
            id: "text.fonts",
            label: "List Fonts",
            summary: "The font families on this computer: each with its styles, and whether it is variable.",
            params: const { &[Param::opt("query", Ty::Text, "only families with this in their name"), Param::opt("variable", Ty::Bool, "only variable fonts")] },
            returns: "{count, families: [{family, styles, variable}]}",
            run: Run::Look(fonts),
        },
        Command {
            id: "text.fontFeatures",
            label: "OpenType Features",
            summary: "What a font has to ask for: its OpenType features (for text.set features) and a variable font's axes (for weight and text.set axes).",
            params: const { &[Param::need("font", Ty::Text, "a family, full or PostScript name, or a font file's path"), Param::opt("weight", Ty::Number, "the weight to look at (default 400)"), Param::opt("italic", Ty::Bool, "the italic")] },
            returns: "{font, features, axes: [{tag, min, default, max}]}",
            run: Run::Look(font_features),
        },
        Command {
            id: "text.animator.add",
            label: "Add Text Animator",
            summary: "Add an animator to a text layer: it changes the characters its selectors pick (by default one range selector over all of them). Animate the selector's start, end or offset to sweep the change through the text.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::opt("props", Ty::Object, "what it changes: {\"position\": [0, 50], \"opacity\": 0, \"scale\": [150, 150], \"rotation\": 20, \"fillColor\": \"#ff0000\", \"fillHue\": 90, \"tracking\": 200, \"lineAnchor\": 50, \"blur\": [10, 10], \"characterOffset\": 5…}"),
                    Param::opt("selector", Ty::Choice(&["range", "wiggly", "expression", "none"]), "the selector it starts with (default range)"),
                    Param::opt("selectorProps", Ty::Object, "the selector's values: {\"start\": 0, \"end\": 50, \"shape\": \"rampUp\", \"basedOn\": \"words\"…}"),
                    Param::opt("name", Ty::Text, "what to call it (default: Animator 1…)"),
                ]
            },
            returns: "{path, selector?}",
            run: Run::Edit(animator_add),
        },
        Command {
            id: "text.selector.add",
            label: "Add Text Selector",
            summary: "Add a selector to a text animator: a range of the text, a wiggly one that picks at random over time, or one an expression decides (its amount's expression reads textIndex, textTotal and selectorValue).",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::need("animator", Ty::Text, "the animator, like \"Animator 1\""),
                    Param::need("type", Ty::Choice(&["range", "wiggly", "expression"]), "which kind"),
                    Param::opt("props", Ty::Object, "its values"),
                    Param::opt("name", Ty::Text, "what to call it"),
                ]
            },
            returns: "{path}",
            run: Run::Edit(selector_add),
        },
        Command {
            id: "text.removeAllAnimators",
            label: "Remove All Text Animators",
            summary: "Take every animator off a text layer.",
            params: const { &[COMP, LAYER] },
            returns: "{removed}",
            run: Run::Edit(remove_animators),
        },
    ]
}

/// The text layer the parameters name, to change.
fn text_layer<'a>(cx: &'a mut Ctx, a: &Args) -> Result<&'a mut Layer> {
    let (cid, id) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(id).expect("found");
    if layer.source != Source::Text {
        return Err(Error::refused(format!("\"{}\" is a {} layer, not a text layer", layer.name, layer.source.id())));
    }
    Ok(layer)
}

/// The characters a `range` parameter names, held inside the text.
fn range_of(a: &Args, key: &str, n: usize) -> Result<Option<std::ops::Range<usize>>> {
    let Some(j) = a.json(key) else { return Ok(None) };
    let pair = j.as_array().filter(|v| v.len() == 2).and_then(|v| Some((v[0].as_f64()?, v[1].as_f64()?)));
    match pair {
        Some((s, e)) if s >= 0.0 && e >= s => Ok(Some((s as usize).min(n)..(e as usize).min(n))),
        _ => Err(Error::params(format!("`{key}` is [start, end] in characters, start not past end"))),
    }
}

fn text_set(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let layer = text_layer(cx, a)?;
    let name = layer.name.clone();
    let p = layer.props.prop_mut("text/sourceText").expect("defined");
    if p.is_animated() {
        return Err(Error::refused("its text has keys; set one with prop.set and `time`, or change them all with prop.keys"));
    }
    let mut doc = p.value.doc().unwrap_or_default();
    let n = doc.char_count();
    let given = range_of(a, "range", n)?;
    let mut range = given.clone().unwrap_or(0..n);
    if let Some(t) = a.text("text") {
        doc.replace(range.clone(), t);
        range = range.start..range.start + t.chars().count();
        if given.is_none() {
            range = 0..doc.char_count();
        }
    }
    let pick = |fields: &[&str]| -> Map {
        let mut m = Map::new();
        for f in fields {
            if let Some(v) = a.json(f) {
                m.insert(*f, v.clone());
            }
        }
        m
    };
    let look = pick(veymelo_keyframe::CharStyle::FIELDS);
    if !look.is_empty() {
        let mut check = doc.base_style();
        check.apply_json(&look).map_err(Error::params)?;
        if doc.char_count() == 0 || given.is_none() {
            // all of it, and its own look (what new words take)
            doc.style_range(0..doc.char_count(), &mut |s| {
                let _ = s.apply_json(&look);
            });
            let mut b = doc.base_style();
            let _ = b.apply_json(&look);
            if doc.runs.is_empty() {
                doc.set_base_style(&b);
            }
        } else {
            doc.style_range(range.clone(), &mut |s| {
                let _ = s.apply_json(&look);
            });
        }
    }
    let para = pick(veymelo_keyframe::ParaStyle::FIELDS);
    if !para.is_empty() {
        let mut check = doc.base_para();
        check.apply_json(&para).map_err(Error::params)?;
        let paras = if given.is_some() { doc.paragraph_of(range.start)..doc.paragraph_of(range.end.saturating_sub(1).max(range.start)) + 1 } else { 0..doc.paragraph_ranges().len() };
        doc.para_range(paras, &mut |p| {
            let _ = p.apply_json(&para);
        });
    }
    if let Some(v) = a.bool("strokeOver") {
        doc.stroke_over = v;
    }
    if let Some(v) = a.bool("vertical") {
        doc.vertical = v;
    }
    if let Some(v) = a.json("box") {
        doc.box_size = match v {
            Json::Null => None,
            _ => {
                let b: Option<Vec<f64>> = v.as_array().map(|x| x.iter().filter_map(Json::as_f64).collect());
                match b {
                    Some(b) if b.len() == 2 && b.iter().all(|x| *x > 0.0) => Some([b[0], b[1]]),
                    _ => return Err(Error::params("`box` is [width, height], both above zero (or null for point text)")),
                }
            }
        };
    }
    p.set_value(Value::Doc(Box::new(doc))).map_err(Error::params)?;
    let out = Json::object([("text", p.value.to_json(Kind::Doc))]);
    cx.label = Some(format!("Set text of {name}"));
    Ok(out)
}

fn edit_words(cx: &mut Ctx, a: &Args, f: &mut dyn FnMut(&mut veymelo_keyframe::TextDoc) -> Result<Json>) -> Result<Json> {
    let layer = text_layer(cx, a)?;
    let name = layer.name.clone();
    let p = layer.props.prop_mut("text/sourceText").expect("defined");
    if p.is_animated() {
        return Err(Error::refused("its text has keys; change them with prop.keys"));
    }
    let mut doc = p.value.doc().unwrap_or_default();
    let out = f(&mut doc)?;
    p.set_value(Value::Doc(Box::new(doc))).map_err(Error::params)?;
    cx.label = Some(format!("Edit text of {name}"));
    Ok(out)
}

fn text_insert(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let words = a.text("text").unwrap_or("").to_string();
    let at = a.whole("at");
    let given = a.json("range").cloned();
    edit_words(cx, a, &mut |doc| {
        let n = doc.char_count();
        let range = match &given {
            Some(_) => range_of(a, "range", n)?.expect("given"),
            None => {
                let k = at.map_or(n, |k| (k as usize).min(n));
                k..k
            }
        };
        doc.replace(range.clone(), &words);
        let end = range.start + words.chars().count();
        Ok(Json::object([("text", Json::from(doc.text.as_str())), ("range", Json::array([Json::Number(range.start as f64), Json::Number(end as f64)]))]))
    })
}

fn text_delete(cx: &mut Ctx, a: &Args) -> Result<Json> {
    edit_words(cx, a, &mut |doc| {
        let range = range_of(a, "range", doc.char_count())?.expect("needed");
        doc.replace(range, "");
        Ok(Json::object([("text", Json::from(doc.text.as_str()))]))
    })
}

fn fonts(_: &Look, a: &Args) -> Result<Json> {
    let q = a.text("query").map(str::to_lowercase);
    let only_variable = a.bool("variable").unwrap_or(false);
    let mut by_family: Vec<(String, Vec<String>, bool)> = Vec::new();
    for f in veymelo_font::system::faces() {
        if f.family.starts_with('.') || q.as_ref().is_some_and(|q| !f.family.to_lowercase().contains(q.as_str())) || (only_variable && !f.variable) {
            continue;
        }
        match by_family.iter_mut().find(|x| x.0 == f.family) {
            Some(x) => {
                if !x.1.contains(&f.style) {
                    x.1.push(f.style.clone());
                }
                x.2 |= f.variable;
            }
            None => by_family.push((f.family.clone(), vec![f.style.clone()], f.variable)),
        }
    }
    by_family.sort_by_key(|x| x.0.to_lowercase());
    let count = by_family.len();
    let list = by_family.into_iter().take(500).map(|(family, styles, variable)| Json::object([("family", Json::from(family)), ("styles", Json::array(styles)), ("variable", Json::from(variable))]));
    Ok(Json::object([("count", Json::Number(count as f64)), ("families", Json::array(list))]))
}

fn font_features(_: &Look, a: &Args) -> Result<Json> {
    let name = a.text("font").unwrap_or("");
    let font = veymelo_text::resolve(name, a.num("weight").unwrap_or(400.0), a.bool("italic").unwrap_or(false)).ok_or_else(|| Error::not_found(format!("there is no font \"{name}\" (text.fonts lists them)")))?;
    let axes = font.axes().into_iter().map(|(tag, min, def, max)| Json::object([("tag", Json::from(tag)), ("min", Json::Number(min)), ("default", Json::Number(def)), ("max", Json::Number(max))]));
    Ok(Json::object([
        ("font", Json::from(font.name(4).unwrap_or_else(|| name.to_string()))),
        ("features", Json::array(veymelo_font::shape::features(&font))),
        ("axes", Json::array(axes)),
    ]))
}

fn remove_animators(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let layer = text_layer(cx, a)?;
    let list = layer.props.group_mut("text/animators").expect("defined");
    let n = list.children.len();
    list.children.clear();
    cx.label = Some("Remove All Text Animators".into());
    Ok(Json::object([("removed", Json::Number(n as f64))]))
}

/// Give a new instance the values in `props`.
fn start_with(g: &mut Group, props: Option<&Json>) -> Result<()> {
    let Some(Json::Object(m)) = props else { return Ok(()) };
    for (path, v) in m.iter() {
        let p = g.prop_mut(path).map_err(|e| Error::params(format!("`props`: {e}")))?;
        let value = Value::from_json(p.def.kind, v).map_err(|e| Error::params(format!("`props.{path}`: {e}")))?;
        p.set_value(value).map_err(|e| Error::params(format!("`props.{path}`: {e}")))?;
    }
    Ok(())
}

fn animator_add(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (animator_def, selector) = (cx.catalog.find("animators", "animator").expect("in the catalog"), a.text("selector").unwrap_or("range"));
    let sel_def = (selector != "none").then(|| cx.catalog.find("selectors", selector).expect("in the catalog"));
    let (aid, sid) = (cx.project.alloc(), cx.project.alloc());
    let layer = text_layer(cx, a)?;
    let list = layer.props.group_mut("text/animators").expect("defined");
    let name = a.text("name").map(str::to_string).unwrap_or_else(|| fresh_name("Animator", list.children.iter().map(Node::name)));
    let mut g = Group::instance(animator_def, aid, &name);
    start_with(&mut g, a.json("props"))?;
    let mut out = Map::new();
    out.insert("path", format!("text/animators/{name}"));
    if let Some(def) = sel_def {
        let sname = format!("{} 1", def.name);
        let mut s = Group::instance(def, sid, &sname);
        start_with(&mut s, a.json("selectorProps"))?;
        g.group_mut("selectors").expect("defined").children.push(Node::Group(s));
        out.insert("selector", format!("text/animators/{name}/selectors/{sname}"));
    } else if a.has("selectorProps") {
        return Err(Error::params("`selectorProps` needs a selector"));
    }
    list.children.push(Node::Group(g));
    cx.label = Some(format!("Add {name}"));
    Ok(Json::Object(out))
}

fn selector_add(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let ty = a.text("type").unwrap_or("range");
    let def = cx.catalog.find("selectors", ty).expect("in the catalog");
    let id = cx.project.alloc();
    let layer = text_layer(cx, a)?;
    let animator = a.text("animator").unwrap_or("");
    let list = layer.props.group_mut(&format!("text/animators/{animator}/selectors")).map_err(Error::not_found)?;
    let name = a.text("name").map(str::to_string).unwrap_or_else(|| fresh_name(def.name, list.children.iter().map(Node::name)));
    let mut g = Group::instance(def, id, &name);
    start_with(&mut g, a.json("props"))?;
    list.children.push(Node::Group(g));
    cx.label = Some(format!("Add {name}"));
    Ok(Json::object([("path", format!("text/animators/{animator}/selectors/{name}"))]))
}

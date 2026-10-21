//! OpenColorIO configurations (`.ocio`, read from the published format and the v2
//! documentation of its transforms): colour spaces and display colour spaces with their
//! transforms to and from the reference, roles, displays and their views (v1: a colour space;
//! v2: a view transform and a display colour space), view transforms, looks, and the search path
//! files are found on. A conversion is built as one list of [`Step`]s.
//!
//! Transforms read: Matrix, File, Exponent, ExponentWithLinear, Log, LogAffine, LogCamera, CDL,
//! Range, Allocation, ExposureContrast (linear), GradingPrimary, GradingTone, GradingRGBCurve,
//! FixedFunction (ACES gamut compression, RGB to
//! HSV, XYZ to xyY), Builtin (the camera, ACES (output transforms too: ACES 1.x SDR and HDR) and
//! display ones), Group, ColorSpace, Look and
//! DisplayView, each either way. Others pass colours through and are listed
//! ([`Config::skipped`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use veymelo_color::space::{self, Curve, M3, Space};

use super::yaml::{self, Yaml};
use crate::colorfile::{self, Cdl, Step, Transform};

/// The longest chain of named transforms one conversion follows (a config naming itself in a
/// loop stops here).
pub const DEEPEST: usize = 16;
/// The most steps one conversion has.
pub const MOST: usize = 4096;

/// A transform as a configuration writes it.
#[derive(Clone, Debug)]
pub enum Xf {
    /// Ready steps.
    Steps(Vec<Step>),
    Group(Vec<Xf>),
    /// Run the other way.
    Back(Box<Xf>),
    /// From one of the config's colour spaces to another.
    Space { src: String, dst: String },
    /// Through looks, from `src` to `dst`.
    Look { src: String, dst: String, looks: String },
    /// From `src` through a display's view.
    DisplayView { src: String, display: String, view: String, looks_bypass: bool },
    /// Not done (its kind, or what went wrong): passes colours through.
    Skipped(String),
}

fn m33_of(v: &[f64]) -> M3 {
    // a 4 × 4 row-major matrix, its colour part
    if v.len() >= 16 { [[v[0], v[1], v[2]], [v[4], v[5], v[6]], [v[8], v[9], v[10]]] } else if v.len() >= 9 { [[v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]]] } else { space::IDENTITY }
}

/// Three numbers at `key` (one number for all three; `d` when it is not there).
fn three(y: &Yaml, key: &str, d: f64) -> [f64; 3] {
    let v = y.get(key).map(Yaml::nums).unwrap_or_default();
    match v.len() {
        0 => [d; 3],
        1 | 2 => [v[0]; 3],
        _ => [v[0], v[1], v[2]],
    }
}

fn num(y: &Yaml, key: &str, d: f64) -> f64 {
    y.get(key).map(Yaml::nums).and_then(|v| v.first().copied()).unwrap_or(d)
}

/// A named camera or display transform built in to OpenColorIO, from our colour spaces.
pub fn builtin(style: &str) -> Option<Vec<Step>> {
    if let Some(out) = veymelo_color::aces::Output::named(style) {
        return Some(vec![Step::Aces(out)]);
    }
    let named = |n: &str| space::find(n).copied();
    let aces = named("ACES2065-1")?;
    let xyz = named("CIE XYZ D65")?;
    let to = |a: &Space, b: &Space| -> Vec<Step> {
        let mut s = Vec::new();
        if a.curve != Curve::Linear {
            s.push(Step::Curve { curve: a.curve, encode: false });
        }
        s.push(Step::Matrix { m: space::matrix(a, b), offset: [0.0; 3] });
        if b.curve != Curve::Linear {
            s.push(Step::Curve { curve: b.curve, encode: true });
        }
        s
    };
    let camera = |n: &str| named(n).map(|a| to(&a, &aces));
    let display = |n: &str| named(n).map(|b| to(&xyz, &b));
    let lin = |n: &str| named(n).map(|s| Space { curve: Curve::Linear, ..s });
    Some(match style.trim() {
        "IDENTITY" => Vec::new(),
        "UTILITY - ACES-AP0_to_CIE-XYZ-D65_BFD" => to(&aces, &xyz),
        "UTILITY - ACES-AP1_to_CIE-XYZ-D65_BFD" => to(&named("ACEScg")?, &xyz),
        "UTILITY - ACES-AP1_to_LINEAR-REC709_BFD" => to(&named("ACEScg")?, &named("Linear Rec.709 (sRGB)")?),
        "ACES-AP1_to_AP0" | "ACEScg_to_ACES2065-1" => to(&named("ACEScg")?, &aces),
        "ACES-AP0_to_AP1" => to(&aces, &named("ACEScg")?),
        "ACEScct_to_ACES2065-1" => camera("ACEScct")?,
        "ACEScc_to_ACES2065-1" => camera("ACEScc")?,
        "ARRI_ALEXA-LOGC-EI800-AWG_to_ACES2065-1" => camera("ARRI LogC3 (EI800) - AWG3")?,
        "ARRI_LOGC4_to_ACES2065-1" => camera("ARRI LogC4 - AWG4")?,
        "CANON_CLOG2-CGAMUT_to_ACES2065-1" => camera("Canon Log 2 CinemaGamut D55")?,
        "CANON_CLOG3-CGAMUT_to_ACES2065-1" => camera("Canon Log 3 CinemaGamut D55")?,
        "PANASONIC_VLOG-VGAMUT_to_ACES2065-1" => camera("V-Log V-Gamut")?,
        "RED_LOG3G10-RWG_to_ACES2065-1" => camera("Log3G10 REDWideGamutRGB")?,
        "SONY_SLOG3-SGAMUT3_to_ACES2065-1" => camera("S-Log3 S-Gamut3")?,
        "SONY_SLOG3-SGAMUT3.CINE_to_ACES2065-1" => camera("S-Log3 S-Gamut3.Cine")?,
        "DISPLAY - CIE-XYZ-D65_to_sRGB" => display("sRGB - Display")?,
        "DISPLAY - CIE-XYZ-D65_to_REC.1886-REC.709" => display("Rec.1886 Rec.709 - Display")?,
        "DISPLAY - CIE-XYZ-D65_to_REC.1886-REC.2020" => display("Rec.1886 Rec.2020 - Display")?,
        "DISPLAY - CIE-XYZ-D65_to_G2.2-REC.709" => display("Gamma 2.2 Rec.709 - Display")?,
        "DISPLAY - CIE-XYZ-D65_to_sRGB-P3-D65" => display("Display P3 - Display")?,
        "DISPLAY - CIE-XYZ-D65_to_G2.6-P3-D65" => display("P3-D65 - Display")?,
        "DISPLAY - CIE-XYZ-D65_to_G2.6-P3-DCI-BFD" => display("P3-DCI - Display")?,
        "DISPLAY - CIE-XYZ-D65_to_REC.2100-PQ" => display("Rec.2100-PQ - Display")?,
        "DISPLAY - CIE-XYZ-D65_to_ST2084-P3-D65" => to(&xyz, &Space { curve: Curve::Pq, ..lin("Linear P3-D65")? }),
        "CURVE - ST-2084_to_LINEAR" => vec![Step::Curve { curve: Curve::Pq, encode: false }],
        "CURVE - LINEAR_to_ST-2084" => vec![Step::Curve { curve: Curve::Pq, encode: true }],
        _ => return None,
    })
}

impl Xf {
    /// A transform from its YAML (FileTransform files found on `search`, from `dir`).
    pub fn of(y: &Yaml, dir: &Path, search: &[String]) -> Xf {
        let x = Xf::kind(y, dir, search);
        let back = y.text("direction").eq_ignore_ascii_case("inverse");
        if back { Xf::Back(Box::new(x)) } else { x }
    }

    fn kind(y: &Yaml, dir: &Path, search: &[String]) -> Xf {
        let steps = |s: Vec<Step>| Xf::Steps(s);
        match y.tag() {
            "MatrixTransform" => {
                let m = y.get("matrix").map(Yaml::nums).unwrap_or_default();
                let off = y.get("offset").map(Yaml::nums).unwrap_or_default();
                let offset = [0, 1, 2].map(|i| off.get(i).copied().unwrap_or(0.0));
                steps(vec![Step::Matrix { m: if m.is_empty() { space::IDENTITY } else { m33_of(&m) }, offset }])
            }
            "FileTransform" => {
                let src = y.text("src");
                let which = if y.text("cccid").is_empty() { y.text("ccc_id") } else { y.text("cccid") };
                match find_file(src, dir, search) {
                    Some(path) => match colorfile::load(&path.to_string_lossy(), which) {
                        Ok(t) => steps(t.steps.clone()),
                        Err(e) => Xf::Skipped(format!("FileTransform {src}: {e}")),
                    },
                    None => Xf::Skipped(format!("FileTransform {src}: not found on the search path")),
                }
            }
            "ExponentTransform" => {
                let v = y.get("value").map(Yaml::nums).unwrap_or_default();
                let e = match v.len() {
                    0 => [1.0; 3],
                    1 | 2 => [v[0]; 3],
                    _ => [v[0], v[1], v[2]],
                };
                let style = format!("{}{}", y.text("style"), y.text("negative_style")).to_ascii_lowercase();
                steps(vec![Step::Power { e, mirror: style.contains("mirror") || style.contains("pass") }])
            }
            "ExponentWithLinearTransform" => {
                let style = y.text("negative_style").to_ascii_lowercase();
                steps(vec![Step::MonCurve { gamma: three(y, "gamma", 1.0), offset: three(y, "offset", 0.0), forward: true, mirror: style == "mirror" }])
            }
            "LogTransform" => steps(vec![Step::Log { base: num(y, "base", 2.0), log_slope: [1.0; 3], log_offset: [0.0; 3], lin_slope: [1.0; 3], lin_offset: [0.0; 3], brk: None, to_log: true }]),
            tag @ ("LogAffineTransform" | "LogCameraTransform") => steps(vec![Step::Log {
                base: num(y, "base", 2.0),
                log_slope: three(y, "logSideSlope", 1.0),
                log_offset: three(y, "logSideOffset", 0.0),
                lin_slope: three(y, "linSideSlope", 1.0),
                lin_offset: three(y, "linSideOffset", 0.0),
                brk: (tag == "LogCameraTransform").then(|| three(y, "linSideBreak", 0.0)),
                to_log: true,
            }]),
            "CDLTransform" => {
                let sat = y.get("sat").or_else(|| y.get("saturation")).map(Yaml::nums).and_then(|v| v.first().copied()).unwrap_or(1.0);
                let cdl = Cdl { slope: three(y, "slope", 1.0), offset: three(y, "offset", 0.0), power: three(y, "power", 1.0), sat };
                steps(vec![Step::Cdl { cdl, clamp: !y.text("style").to_ascii_lowercase().contains("noclamp") }])
            }
            "RangeTransform" => {
                let get = |k: &str| y.get(k).map(Yaml::nums).and_then(|v| v.first().copied());
                let clamp = !y.text("style").to_ascii_lowercase().contains("noclamp");
                match (get("min_in_value"), get("max_in_value"), get("min_out_value"), get("max_out_value")) {
                    (Some(a), Some(b), Some(p), Some(q)) => steps(vec![Step::Range { lo_in: [a; 3], hi_in: [b; 3], lo_out: [p; 3], hi_out: [q; 3], clamp }]),
                    (a, b, p, q) => {
                        let shift = a.zip(p).map(|(a, p)| p - a).or_else(|| b.zip(q).map(|(b, q)| q - b)).unwrap_or(0.0);
                        let (lo, hi) = if clamp { (p.filter(|_| a.is_some()).unwrap_or(f64::NEG_INFINITY), q.filter(|_| b.is_some()).unwrap_or(f64::INFINITY)) } else { (f64::NEG_INFINITY, f64::INFINITY) };
                        steps(vec![Step::Clamp { shift: [shift; 3], lo: [lo; 3], hi: [hi; 3] }])
                    }
                }
            }
            "AllocationTransform" => {
                let v = y.get("vars").map(Yaml::nums).unwrap_or_default();
                if y.text("allocation").eq_ignore_ascii_case("lg2") {
                    let (lo, hi) = (v.first().copied().unwrap_or(-10.0), v.get(1).copied().unwrap_or(6.0));
                    let k = 1.0 / (hi - lo).max(1e-9);
                    steps(vec![Step::Log { base: 2.0, log_slope: [k; 3], log_offset: [-lo * k; 3], lin_slope: [1.0; 3], lin_offset: [v.get(2).copied().unwrap_or(0.0); 3], brk: None, to_log: true }])
                } else {
                    let (lo, hi) = (v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(1.0));
                    steps(vec![Step::Range { lo_in: [lo; 3], hi_in: [hi; 3], lo_out: [0.0; 3], hi_out: [1.0; 3], clamp: false }])
                }
            }
            "ExposureContrastTransform" => {
                if !matches!(y.text("style").to_ascii_lowercase().as_str(), "" | "linear") {
                    return Xf::Skipped(format!("ExposureContrastTransform style {}", y.text("style")));
                }
                let (exposure, contrast, gamma, pivot) = (num(y, "exposure", 0.0), num(y, "contrast", 1.0), num(y, "gamma", 1.0), num(y, "pivot", 0.18).max(1e-9));
                let scale = |k: f64| Step::Matrix { m: [[k, 0.0, 0.0], [0.0, k, 0.0], [0.0, 0.0, k]], offset: [0.0; 3] };
                steps(vec![scale(2f64.powf(exposure) / pivot), Step::Power { e: [contrast * gamma; 3], mirror: true }, scale(pivot)])
            }
            "FixedFunctionTransform" => {
                let p = y.get("params").map(Yaml::nums).unwrap_or_default();
                match y.text("style") {
                    "ACES_GamutComp13" | "ACES_GAMUT_COMP13" if p.len() >= 7 => steps(vec![Step::GamutCompress { limit: [p[0], p[1], p[2]], threshold: [p[3], p[4], p[5]], power: p[6] }]),
                    "RGB_TO_HSV" => steps(vec![Step::Hsv { to: true }]),
                    "HSV_TO_RGB" => steps(vec![Step::Hsv { to: false }]),
                    "XYZ_TO_xyY" => steps(vec![Step::XyY { to: true }]),
                    "xyY_TO_XYZ" => steps(vec![Step::XyY { to: false }]),
                    other => Xf::Skipped(format!("FixedFunctionTransform {other}")),
                }
            }
            "BuiltinTransform" => builtin(y.text("style")).map_or_else(|| Xf::Skipped(format!("BuiltinTransform {}", y.text("style"))), Xf::Steps),
            "GradingPrimaryTransform" => steps(vec![Step::GradingPrimary(Box::new(super::grading::Primary::read(y)))]),
            "GradingToneTransform" => steps(vec![Step::GradingTone(Box::new(super::grading::Tone::read(y)))]),
            "GradingRGBCurveTransform" => steps(vec![Step::GradingCurves(Box::new(super::grading::Curves::read(y)))]),
            "GroupTransform" => Xf::Group(y.get("children").map(Yaml::seq).unwrap_or(&[]).iter().map(|c| Xf::of(c, dir, search)).collect()),
            "ColorSpaceTransform" => Xf::Space { src: y.text("src").to_string(), dst: y.text("dst").to_string() },
            "LookTransform" => Xf::Look { src: y.text("src").to_string(), dst: y.text("dst").to_string(), looks: y.text("looks").to_string() },
            "DisplayViewTransform" => Xf::DisplayView { src: y.text("src").to_string(), display: y.text("display").to_string(), view: y.text("view").to_string(), looks_bypass: y.flag("looks_bypass", false) },
            "" => Xf::Steps(Vec::new()),
            other => Xf::Skipped(other.to_string()),
        }
    }
}

/// A FileTransform's file: as written if it is absolute, else on the search path (from the
/// config's folder), else beside the config.
fn find_file(src: &str, dir: &Path, search: &[String]) -> Option<PathBuf> {
    let p = Path::new(src.trim());
    if src.trim().is_empty() {
        return None;
    }
    if p.is_absolute() {
        return p.exists().then(|| p.to_path_buf());
    }
    search.iter().map(|s| dir.join(s).join(p)).chain([dir.join(p)]).find(|f| f.exists())
}

/// A colour space of a config.
#[derive(Clone, Debug)]
pub struct ConfigSpace {
    pub name: String,
    pub aliases: Vec<String>,
    pub family: String,
    /// Not colour (masks, normals): left as it is.
    pub data: bool,
    /// Referred to the display reference (a v2 display colour space).
    pub display: bool,
    pub to_ref: Option<Xf>,
    pub from_ref: Option<Xf>,
}

/// A display's view: a colour space (v1), or a view transform and a display colour space (v2);
/// either with looks.
#[derive(Clone, Debug, Default)]
pub struct View {
    pub name: String,
    pub colorspace: String,
    pub view_transform: String,
    pub display_colorspace: String,
    pub looks: String,
    /// (Our built-in views) the display's light rolled under 1 before it is written.
    pub tone_map: bool,
}

/// A view transform: from the scene reference to the display reference (or display to display).
#[derive(Clone, Debug)]
pub struct ViewTransform {
    pub name: String,
    pub from_scene: Option<Xf>,
    pub to_scene: Option<Xf>,
    pub from_display: Option<Xf>,
    pub to_display: Option<Xf>,
}

impl ViewTransform {
    fn scene(&self) -> bool {
        self.from_scene.is_some() || self.to_scene.is_some()
    }
}

/// A look: a transform done in its process space.
#[derive(Clone, Debug)]
pub struct Look {
    pub name: String,
    pub process_space: String,
    pub transform: Option<Xf>,
    pub inverse_transform: Option<Xf>,
}

/// A configuration.
#[derive(Clone, Debug, Default)]
pub struct Config {
    pub name: String,
    pub roles: Vec<(String, String)>,
    pub spaces: Vec<ConfigSpace>,
    /// Each display and its views.
    pub displays: Vec<(String, Vec<View>)>,
    pub view_transforms: Vec<ViewTransform>,
    pub default_view_transform: String,
    pub looks: Vec<Look>,
}

impl Config {
    /// A configuration from its text (its files found from `dir`).
    pub fn parse(text: &str, dir: &Path) -> Result<Config, String> {
        let y = yaml::parse(text);
        if !matches!(y, Yaml::Map(..)) {
            return Err("not an OCIO configuration (no YAML mapping)".into());
        }
        let search: Vec<String> = match y.get("search_path") {
            Some(Yaml::Seq(v)) => v.iter().filter_map(|s| s.str().map(str::to_string)).collect(),
            Some(s) => s.str().map(|s| s.split(':').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()).unwrap_or_default(),
            None => Vec::new(),
        };
        let xf = |entry: &Yaml, keys: &[&str]| keys.iter().find_map(|k| entry.get(k)).filter(|v| !matches!(v, Yaml::Null)).map(|v| Xf::of(v, dir, &search));
        let mut spaces = Vec::new();
        for (list, display) in [("colorspaces", false), ("display_colorspaces", true)] {
            for cs in y.get(list).map(Yaml::seq).unwrap_or(&[]) {
                let name = cs.text("name");
                if name.is_empty() {
                    continue;
                }
                spaces.push(ConfigSpace {
                    name: name.to_string(),
                    aliases: cs.get("aliases").map(|a| a.seq().iter().filter_map(|s| s.str().map(str::to_string)).collect()).unwrap_or_default(),
                    family: cs.text("family").to_string(),
                    data: cs.flag("isdata", false),
                    display,
                    to_ref: xf(cs, &["to_reference", "to_scene_reference", "to_display_reference"]),
                    from_ref: xf(cs, &["from_reference", "from_scene_reference", "from_display_reference"]),
                });
            }
        }
        if spaces.is_empty() {
            return Err("the configuration has no colour spaces".into());
        }
        let text = |v: &Yaml, k: &str| v.text(k).to_string();
        let displays = y
            .get("displays")
            .map(Yaml::map)
            .unwrap_or(&[])
            .iter()
            .map(|(d, views)| {
                let vs = views
                    .seq()
                    .iter()
                    .filter(|v| !v.text("name").is_empty())
                    .map(|v| View { name: text(v, "name"), colorspace: text(v, "colorspace"), view_transform: text(v, "view_transform"), display_colorspace: text(v, "display_colorspace"), looks: text(v, "looks"), tone_map: false })
                    .collect();
                (d.clone(), vs)
            })
            .collect();
        let view_transforms = y
            .get("view_transforms")
            .map(Yaml::seq)
            .unwrap_or(&[])
            .iter()
            .filter(|v| !v.text("name").is_empty())
            .map(|v| ViewTransform {
                name: text(v, "name"),
                from_scene: xf(v, &["from_scene_reference", "from_reference"]),
                to_scene: xf(v, &["to_scene_reference", "to_reference"]),
                from_display: xf(v, &["from_display_reference"]),
                to_display: xf(v, &["to_display_reference"]),
            })
            .collect();
        let looks = y
            .get("looks")
            .map(Yaml::seq)
            .unwrap_or(&[])
            .iter()
            .filter(|l| !l.text("name").is_empty())
            .map(|l| Look { name: text(l, "name"), process_space: text(l, "process_space"), transform: xf(l, &["transform"]), inverse_transform: xf(l, &["inverse_transform"]) })
            .collect();
        let roles = y.get("roles").map(Yaml::map).unwrap_or(&[]).iter().filter_map(|(k, v)| v.str().map(|s| (k.clone(), s.to_string()))).collect();
        Ok(Config { name: y.text("name").to_string(), roles, spaces, displays, view_transforms, default_view_transform: y.text("default_view_transform").to_string(), looks })
    }

    /// Our own configuration: every colour space we know (the reference CIE XYZ D65), each
    /// display space a display with a plain view, a tone-mapped one and a raw one.
    pub fn built_in() -> Config {
        let to_xyz = |s: &Space| {
            let mut steps = Vec::new();
            if s.curve != Curve::Linear {
                steps.push(Step::Curve { curve: s.curve, encode: false });
            }
            steps.push(Step::Matrix { m: s.to_xyz(true), offset: [0.0; 3] });
            Xf::Steps(steps)
        };
        let mut spaces: Vec<ConfigSpace> = space::SPACES
            .iter()
            .map(|s| ConfigSpace { name: s.name.to_string(), aliases: Vec::new(), family: if s.display { "Display".into() } else { "Scene".into() }, data: false, display: false, to_ref: Some(to_xyz(s)), from_ref: None })
            .collect();
        spaces.push(ConfigSpace { name: "Raw".into(), aliases: vec!["Utility - Raw".into()], family: "Utility".into(), data: true, display: false, to_ref: None, from_ref: None });
        let displays = space::SPACES
            .iter()
            .filter(|s| s.display)
            .map(|s| {
                let name = s.name.trim_end_matches(" - Display").to_string();
                let view = |n: &str, cs: &str, tone: bool| View { name: n.into(), colorspace: cs.into(), tone_map: tone, ..View::default() };
                (name, vec![view("Standard", s.name, false), view("Tone Mapped", s.name, true), view("Raw", "Raw", false)])
            })
            .collect();
        let roles = vec![("scene_linear".into(), "ACEScg".into()), ("color_timing".into(), "ACEScct".into()), ("compositing_log".into(), "ACEScct".into()), ("data".into(), "Raw".into())];
        Config { name: "built-in".into(), roles, spaces, displays, view_transforms: Vec::new(), default_view_transform: String::new(), looks: Vec::new() }
    }

    /// A colour space by name, other name or role (case not minded).
    pub fn space(&self, name: &str) -> Option<&ConfigSpace> {
        let n = name.trim();
        let by = |n: &str| self.spaces.iter().find(|s| s.name.eq_ignore_ascii_case(n) || s.aliases.iter().any(|a| a.eq_ignore_ascii_case(n)));
        by(n).or_else(|| self.roles.iter().find(|(r, _)| r.eq_ignore_ascii_case(n)).and_then(|(_, s)| by(s)))
    }

    /// A display (named, else the first) and its view (named, else its first).
    pub fn view(&self, display: &str, view: &str) -> Option<(&str, &View)> {
        let (d, views) = self.displays.iter().find(|(d, _)| d.eq_ignore_ascii_case(display.trim())).or(self.displays.first())?;
        let v = views.iter().find(|v| v.name.eq_ignore_ascii_case(view.trim())).or(views.first())?;
        Some((d.as_str(), v))
    }

    fn look(&self, name: &str) -> Option<&Look> {
        self.looks.iter().find(|l| l.name.eq_ignore_ascii_case(name.trim()))
    }

    fn view_transform(&self, name: &str) -> Option<&ViewTransform> {
        self.view_transforms.iter().find(|v| v.name.eq_ignore_ascii_case(name.trim()))
    }

    /// The view transform between the scene and display references: the default, else the
    /// first that starts from the scene.
    fn scene_to_display(&self) -> Option<&ViewTransform> {
        self.view_transform(&self.default_view_transform).filter(|v| v.scene()).or_else(|| self.view_transforms.iter().find(|v| v.scene()))
    }

    /// From space `a` to space `b`.
    pub fn convert(&self, a: &str, b: &str) -> Result<Transform, String> {
        let (sa, sb) = (self.space(a).ok_or_else(|| format!("no colour space \"{a}\""))?, self.space(b).ok_or_else(|| format!("no colour space \"{b}\""))?);
        let mut k = Build::new(self);
        k.path(sa, sb, 0);
        Ok(k.done())
    }

    /// Space `input` on `display` through `view` (each by name; empty: the first).
    pub fn display(&self, input: &str, display: &str, view: &str, looks_bypass: bool) -> Result<Transform, String> {
        let s = self.space(input).ok_or_else(|| format!("no colour space \"{input}\""))?;
        let mut k = Build::new(self);
        k.display_view(s, display, view, looks_bypass, 0);
        Ok(k.done())
    }

    /// From `src` through `looks` (`+A, -B`; `A | B` the first the configuration has) to `dst`.
    pub fn looks(&self, src: &str, dst: &str, looks: &str) -> Result<Transform, String> {
        let (sa, sb) = (self.space(src).ok_or_else(|| format!("no colour space \"{src}\""))?, self.space(dst).ok_or_else(|| format!("no colour space \"{dst}\""))?);
        let mut k = Build::new(self);
        let at = k.apply_looks(sa, looks, 0);
        k.path(at, sb, 0);
        Ok(k.done())
    }
}

/// One conversion being put together, step by step.
struct Build<'c> {
    cfg: &'c Config,
    out: Vec<Step>,
    skipped: Vec<String>,
}

impl<'c> Build<'c> {
    fn new(cfg: &'c Config) -> Build<'c> {
        Build { cfg, out: Vec::new(), skipped: Vec::new() }
    }

    fn done(mut self) -> Transform {
        self.skipped.sort();
        self.skipped.dedup();
        Transform { steps: self.out, skipped: self.skipped }
    }

    /// A configuration transform's steps, run forwards or `back`.
    fn xf(&mut self, x: &Xf, back: bool, depth: usize) {
        if depth > DEEPEST || self.out.len() > MOST {
            self.skipped.push("a chain of named transforms too long (a loop?)".into());
            return;
        }
        match x {
            Xf::Steps(s) => {
                if back {
                    self.out.extend(s.iter().rev().map(|s| Step::Back(Box::new(s.clone()))));
                } else {
                    self.out.extend(s.iter().cloned());
                }
            }
            Xf::Group(children) => {
                if back {
                    children.iter().rev().for_each(|c| self.xf(c, true, depth + 1));
                } else {
                    children.iter().for_each(|c| self.xf(c, false, depth + 1));
                }
            }
            Xf::Back(inner) => self.xf(inner, !back, depth + 1),
            Xf::Space { src, dst } => {
                let (a, b) = if back { (dst, src) } else { (src, dst) };
                match (self.cfg.space(a), self.cfg.space(b)) {
                    (Some(a), Some(b)) => self.path(a, b, depth + 1),
                    _ => self.skipped.push(format!("ColorSpaceTransform between unknown spaces {src}, {dst}")),
                }
            }
            Xf::Look { src, dst, looks } => match (self.cfg.space(src), self.cfg.space(dst)) {
                (Some(a), Some(b)) if !back => {
                    let at = self.apply_looks(a, looks, depth + 1);
                    self.path(at, b, depth + 1);
                }
                (Some(a), Some(b)) => {
                    // undone: back from dst into the last look's space, the looks undone in turn
                    let inverted: Vec<String> = looks.split(',').rev().map(|l| l.trim()).filter(|l| !l.is_empty()).map(|l| if let Some(r) = l.strip_prefix('-') { r.to_string() } else { format!("-{}", l.trim_start_matches('+')) }).collect();
                    let at = self.apply_looks(b, &inverted.join(","), depth + 1);
                    self.path(at, a, depth + 1);
                }
                _ => self.skipped.push(format!("LookTransform between unknown spaces {src}, {dst}")),
            },
            Xf::DisplayView { src, display, view, looks_bypass } => match self.cfg.space(src) {
                Some(s) if !back => self.display_view(s, display, view, *looks_bypass, depth + 1),
                Some(_) => self.skipped.push("a DisplayViewTransform undone".into()),
                None => self.skipped.push(format!("DisplayViewTransform from unknown space {src}")),
            },
            Xf::Skipped(what) => self.skipped.push(what.clone()),
        }
    }

    /// From space `a` to its reference.
    fn space_to_ref(&mut self, a: &ConfigSpace, depth: usize) {
        match (&a.to_ref, &a.from_ref) {
            (Some(x), _) => self.xf(x, false, depth),
            (None, Some(x)) => self.xf(x, true, depth),
            _ => {}
        }
    }

    fn ref_to_space(&mut self, b: &ConfigSpace, depth: usize) {
        match (&b.from_ref, &b.to_ref) {
            (Some(x), _) => self.xf(x, false, depth),
            (None, Some(x)) => self.xf(x, true, depth),
            _ => {}
        }
    }

    /// From space `a` to space `b` through the reference (data passes through; between the
    /// scene and display references, the default view transform).
    fn path(&mut self, a: &ConfigSpace, b: &ConfigSpace, depth: usize) {
        if a.data || b.data || a.name == b.name {
            return;
        }
        self.space_to_ref(a, depth);
        if a.display != b.display {
            match self.cfg.scene_to_display() {
                Some(vt) => {
                    let (x, back) = match (a.display, &vt.from_scene, &vt.to_scene) {
                        (false, Some(x), _) => (x.clone(), false),
                        (false, None, Some(x)) => (x.clone(), true),
                        (true, _, Some(x)) => (x.clone(), false),
                        (true, Some(x), None) => (x.clone(), true),
                        _ => (Xf::Steps(Vec::new()), false),
                    };
                    self.xf(&x, back, depth + 1);
                }
                None => self.skipped.push("no view transform between the scene and display references".into()),
            }
        }
        self.ref_to_space(b, depth);
    }

    /// Looks done in turn from space `at`; the space the colour ends in.
    fn apply_looks(&mut self, at: &'c ConfigSpace, looks: &str, depth: usize) -> &'c ConfigSpace {
        let mut at = at;
        for part in looks.split(',').map(str::trim).filter(|l| !l.is_empty()) {
            // `A | B`: the first the configuration has
            let Some(choice) = part.split('|').map(str::trim).find(|l| self.cfg.look(l.trim_start_matches(['+', '-'])).is_some()) else {
                self.skipped.push(format!("look {part}"));
                continue;
            };
            let back = choice.starts_with('-');
            let Some(look) = self.cfg.look(choice.trim_start_matches(['+', '-'])) else { continue };
            if let Some(ps) = self.cfg.space(&look.process_space) {
                self.path(at, ps, depth + 1);
                at = ps;
            }
            match (back, &look.transform, &look.inverse_transform) {
                (false, Some(x), _) => self.xf(x, false, depth + 1),
                (false, None, Some(x)) => self.xf(x, true, depth + 1),
                (true, _, Some(x)) => self.xf(x, false, depth + 1),
                (true, Some(x), None) => self.xf(x, true, depth + 1),
                _ => {}
            }
        }
        at
    }

    /// Space `input` through a display's view.
    fn display_view(&mut self, input: &'c ConfigSpace, display: &str, view: &str, looks_bypass: bool, depth: usize) {
        let Some((dname, v)) = self.cfg.view(display, view) else {
            self.skipped.push(format!("display {display}"));
            return;
        };
        let v = v.clone();
        let at = if looks_bypass || v.looks.is_empty() { input } else { self.apply_looks(input, &v.looks, depth + 1) };
        if !v.colorspace.is_empty() {
            let Some(cs) = self.cfg.space(&v.colorspace) else {
                self.skipped.push(format!("view {} names unknown space {}", v.name, v.colorspace));
                return;
            };
            if v.tone_map && !cs.data {
                // our own: to the display's light, rolled under 1, then written
                match space::find(&cs.name).copied() {
                    Some(d) => {
                        let lin = Space { curve: Curve::Linear, ..d };
                        self.space_to_ref(at, depth);
                        self.out.push(Step::Matrix { m: space::invert(&lin.to_xyz(true)), offset: [0.0; 3] });
                        self.out.push(Step::ToneMap);
                        if d.curve != Curve::Linear {
                            self.out.push(Step::Curve { curve: d.curve, encode: true });
                        }
                    }
                    None => self.path(at, cs, depth + 1),
                }
                return;
            }
            self.path(at, cs, depth + 1);
            return;
        }
        // v2: to the reference, the view transform, then the display colour space
        let dcs_name = if v.display_colorspace == "<USE_DISPLAY_NAME>" { dname.to_string() } else { v.display_colorspace.clone() };
        let (Some(vt), Some(dcs)) = (self.cfg.view_transform(&v.view_transform), self.cfg.space(&dcs_name)) else {
            self.skipped.push(format!("view {} of {dname}", v.name));
            return;
        };
        self.space_to_ref(at, depth);
        let x = if at.display {
            vt.from_display.clone().map(|x| (x, false)).or_else(|| vt.to_display.clone().map(|x| (x, true)))
        } else {
            vt.from_scene.clone().map(|x| (x, false)).or_else(|| vt.to_scene.clone().map(|x| (x, true)))
        };
        if let Some((x, back)) = x {
            self.xf(&x, back, depth + 1);
        }
        self.ref_to_space(dcs, depth);
    }
}

/// A configuration from a file (kept while the file is unchanged), or our own when no file is
/// named.
pub fn load(path: &str) -> Result<Arc<Config>, String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use std::time::SystemTime;
    type Kept = HashMap<String, (Option<SystemTime>, Result<Arc<Config>, String>)>;
    static OURS: OnceLock<Arc<Config>> = OnceLock::new();
    static KEPT: Mutex<Option<Kept>> = Mutex::new(None);
    let path = path.trim();
    if path.is_empty() {
        return Ok(OURS.get_or_init(|| Arc::new(Config::built_in())).clone());
    }
    if path.contains('\n') {
        return Config::parse(path, Path::new(".")).map(Arc::new);
    }
    let changed = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let Ok(kept) = KEPT.lock()
        && let Some((when, c)) = kept.as_ref().and_then(|k| k.get(path))
        && *when == changed
    {
        return c.clone();
    }
    let dir = Path::new(path).parent().unwrap_or(Path::new(".")).to_path_buf();
    let c = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}")).and_then(|t| Config::parse(&t, &dir).map_err(|e| format!("{path}: {e}"))).map(Arc::new);
    if let Ok(mut kept) = KEPT.lock() {
        kept.get_or_insert_with(HashMap::new).insert(path.to_string(), (changed, c.clone()));
    }
    c
}

use super::config::Config;
use super::yaml::{self, Yaml};
use super::*;
use crate::testing::{buf, instance, n, run};
use veymelo_geom::Rect;
use veymelo_raster::Image;
use std::path::Path;

const CONFIG: &str = r#"ocio_profile_version: 2.1   # a comment

environment: {}
search_path: luts
roles:
  scene_linear: ACEScg
  color_timing: ACEScct
  data: Raw

file_rules:
  - !<Rule> {name: Default, colorspace: ACEScg}

displays:
  sRGB:
    - !<View> {name: Standard, view_transform: Un-tone-mapped, display_colorspace: sRGB - Display}
    - !<View> {name: Raw, colorspace: Raw}

view_transforms:
  - !<ViewTransform>
    name: Un-tone-mapped
    from_scene_reference: !<BuiltinTransform> {style: "UTILITY - ACES-AP0_to_CIE-XYZ-D65_BFD"}

looks:
  - !<Look>
    name: Warm
    process_space: ACEScct
    transform: !<CDLTransform> {slope: [1.1, 1, 0.9]}

colorspaces:
  - !<ColorSpace>
    name: ACES2065-1
    aliases: [aces2065_1, "ACES - ACES2065-1"]
    family: ACES
    isdata: false
    description: |
      The Academy Color Encoding System reference color space
      # not a comment in a block

  - !<ColorSpace>
    name: ACEScg
    to_scene_reference: !<BuiltinTransform> {style: ACEScg_to_ACES2065-1}

  - !<ColorSpace>
    name: ACEScct
    to_scene_reference: !<BuiltinTransform> {style: ACEScct_to_ACES2065-1}

  - !<ColorSpace>
    name: Raw
    isdata: true

  - !<ColorSpace>
    name: Linear Rec.709
    from_scene_reference: !<GroupTransform>
      children:
        - !<MatrixTransform> {matrix: [ 2.52168618674388, -1.13413098823972, -0.387555198504164, 0,
            -0.276479914229922, 1.37271908766826, -0.096239173438334, 0,
            -0.0153780649660342, -0.152975335867399, 1.16835340083344, 0, 0, 0, 0, 1 ]}

display_colorspaces:
  - !<ColorSpace>
    name: sRGB - Display
    from_display_reference: !<BuiltinTransform> {style: DISPLAY - CIE-XYZ-D65_to_sRGB}
"#;

fn near3(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < tol)
}

#[test]
fn reads_yaml() {
    let y = yaml::parse(CONFIG);
    assert_eq!(y.text("ocio_profile_version"), "2.1");
    assert_eq!(y.text("search_path"), "luts");
    assert_eq!(y.get("roles").map(|r| r.text("data")), Some("Raw"));
    let spaces = y.get("colorspaces").map(Yaml::seq).unwrap();
    assert_eq!(spaces.len(), 5);
    assert_eq!(spaces[0].tag(), "ColorSpace");
    assert_eq!(spaces[0].get("aliases").map(|a| a.seq().len()), Some(2));
    assert!(spaces[0].text("description").contains("# not a comment"), "{:?}", spaces[0].get("description"));
    let m = spaces[4].get("from_scene_reference").and_then(|g| g.get("children")).map(|c| c.seq()[0].clone()).unwrap();
    assert_eq!(m.tag(), "MatrixTransform");
    assert_eq!(m.get("matrix").map(Yaml::nums).map(|v| v.len()), Some(16), "a list over several lines");
    let views = y.get("displays").and_then(|d| d.get("sRGB")).map(Yaml::seq).unwrap();
    assert_eq!(views[0].text("display_colorspace"), "sRGB - Display");
    let quoted = yaml::parse("a: 'it''s # here'\nb: \"tab\\there\"\nc: [1, [2, 3], {d: e}]\n");
    assert_eq!(quoted.text("a"), "it's # here");
    assert_eq!(quoted.text("b"), "tab\there");
    assert_eq!(quoted.get("c").map(|c| c.seq().len()), Some(3));
}

#[test]
fn a_configuration_converts() {
    let cfg = Config::parse(CONFIG, Path::new(".")).unwrap();
    assert_eq!(cfg.space("aces2065_1").map(|s| s.name.as_str()), Some("ACES2065-1"), "by alias");
    assert_eq!(cfg.space("scene_linear").map(|s| s.name.as_str()), Some("ACEScg"), "by role");
    // ACEScg to linear Rec.709 through the reference: as our own maths says
    let t = cfg.convert("ACEScg", "Linear Rec.709").unwrap();
    assert!(t.skipped.is_empty(), "{:?}", t.skipped);
    let ours = space::matrix(space::find("ACEScg").unwrap(), space::find("Linear Rec.709 (sRGB)").unwrap());
    for c in [[0.18f32, 0.18, 0.18], [0.5, 0.2, 0.05], [0.02, 0.3, 0.9]] {
        let want = space::apply(&ours, c.map(|v| v as f64)).map(|v| v as f32);
        assert!(near3(t.run(c, Between::Tetrahedral, false), want, 2e-5), "{c:?}");
        assert!(near3(t.run(want, Between::Tetrahedral, true), c, 2e-5), "and back");
    }
    // shown on sRGB: through the view transform to the display reference, then written
    let shown = cfg.display("ACEScg", "sRGB", "Standard", false).unwrap();
    let direct = Config::built_in().convert("ACEScg", "sRGB - Display").unwrap();
    for c in [[0.18f32, 0.18, 0.18], [0.5, 0.2, 0.05]] {
        assert!(near3(shown.run(c, Between::Tetrahedral, false), direct.run(c, Between::Tetrahedral, false), 2e-5), "{c:?}");
    }
    // data passes through; a look is done in its own space
    assert!(near3(cfg.display("ACEScg", "sRGB", "Raw", false).unwrap().run([0.3, 2.0, -1.0], Between::Tetrahedral, false), [0.3, 2.0, -1.0], 1e-9));
    let warm = cfg.looks("ACEScg", "ACEScg", "+Warm").unwrap();
    let grey = warm.run([0.18; 3], Between::Tetrahedral, false);
    assert!(grey[0] > 0.18 && grey[2] < 0.18, "{grey:?}");
    let undone = cfg.looks("ACEScg", "ACEScg", "-Warm").unwrap();
    assert!(near3(undone.run(grey, Between::Tetrahedral, false), [0.18; 3], 1e-5));
    assert!(cfg.looks("ACEScg", "ACEScg", "Nothing | Warm").unwrap().skipped.is_empty(), "the first it has");
    assert!(Config::parse("just words", Path::new(".")).is_err());
}

#[test]
fn our_own_configuration() {
    let cfg = Config::built_in();
    let t = cfg.convert("Linear Rec.709 (sRGB)", "sRGB - Display").unwrap();
    let v = t.run([0.18; 3], Between::Tetrahedral, false);
    assert!((v[0] - 0.4613).abs() < 1e-3, "{v:?}");
    let tone = cfg.display("Linear Rec.709 (sRGB)", "sRGB", "Tone Mapped", false).unwrap();
    let (lo, hi) = (tone.run([0.18; 3], Between::Tetrahedral, false), tone.run([50.0; 3], Between::Tetrahedral, false));
    assert!(lo[0] > 0.2 && lo[0] < hi[0] && hi[0] <= 1.0, "{lo:?} {hi:?}");
    assert!(near3(tone.run(lo, Between::Tetrahedral, true), [0.18; 3], 1e-4), "and back");
    assert!(cfg.view("Rec.2100-PQ", "").is_some());
    let s_log = cfg.convert("S-Log3 S-Gamut3", "ACEScg").unwrap();
    let mid = s_log.run([420.0 / 1023.0; 3], Between::Tetrahedral, false);
    assert!(near3(mid, [0.18; 3], 2e-3), "grey stays grey: {mid:?}");
}

#[test]
fn effects_run() {
    let grey = Image::filled(4, 4, [0.18, 0.18, 0.18, 1.0]);
    let srgb = run(&CST, &[("source", Value::Text("Linear Rec.709 (sRGB)".into())), ("destination", Value::Text("sRGB - Texture".into()))], grey.clone());
    assert!((srgb.img.px[0][0] - 0.4613).abs() < 1e-3);
    let back = run(&CST, &[("source", Value::Text("Linear Rec.709 (sRGB)".into())), ("destination", Value::Text("sRGB - Texture".into())), ("direction", Value::Choice(1))], srgb.img.clone());
    assert!((back.img.px[0][0] - 0.18).abs() < 1e-5);
    let cdl = run(&CDL, &[("slopeRed", n(2.0)), ("saturation", n(1.0))], grey.clone());
    assert!((cdl.img.px[0][0] - 0.36).abs() < 1e-6 && (cdl.img.px[0][1] - 0.18).abs() < 1e-6);
    let file = run(&FILE, &[("file", Value::Text("LUT_1D_SIZE 2\n0 0 0\n0.5 0.5 0.5\n".into()))], grey.clone());
    assert!((file.img.px[0][0] - 0.09).abs() < 1e-6);
    let missing = run(&FILE, &[("file", Value::Text("/no/such/file.cube".into()))], grey.clone());
    assert_eq!(missing.img.px, grey.px, "nothing to do: as it was");
    let shown = run(&DISPLAY, &[("source", Value::Text("Linear Rec.709 (sRGB)".into())), ("display", Value::Text("sRGB".into()))], grey.clone());
    assert!((shown.img.px[0][0] - 0.4613).abs() < 1e-3);
    let no_looks = run(&LOOK, &[], grey.clone());
    assert_eq!(no_looks.img.px, grey.px, "our configuration has no looks");
}

#[test]
fn profiles() {
    let img = Image::filled(2, 2, [0.6, 0.3, 0.2, 1.0]);
    let same = run(&PROFILE, &[("outputProfile", Value::Choice(0))], img.clone());
    assert_eq!(same.img.px, img.px, "the working space is sRGB: nothing to do");
    let wide = run(&PROFILE, &[("outputProfile", Value::Choice(3))], img.clone());
    assert!(wide.img.px[0][0] < 0.6, "the same red is less of Rec. 2020's: {:?}", wide.img.px[0]);
    let rel = run(&PROFILE, &[("outputProfile", Value::Choice(8))], img.clone());
    let abs = run(&PROFILE, &[("outputProfile", Value::Choice(8)), ("intent", Value::Choice(3))], img.clone());
    assert!((rel.img.px[0][2] - abs.img.px[0][2]).abs() > 1e-3, "D50 against D65 whites");
    let white = run(&PROFILE, &[("outputProfile", Value::Choice(8))], Image::filled(1, 1, [1.0; 4]));
    assert!(white.img.px[0].iter().take(3).all(|v| (v - 1.0).abs() < 1e-5), "white stays white, its white matched");
    // a saturated green out of sRGB's gamut, from Rec. 2020: clipped or drawn in
    let green = Image::filled(1, 1, [0.1, 0.9, 0.1, 1.0]);
    let clip = run(&PROFILE, &[("inputProfile", Value::Choice(3)), ("intent", Value::Choice(2))], green.clone());
    let soft = run(&PROFILE, &[("inputProfile", Value::Choice(3)), ("intent", Value::Choice(0))], green.clone());
    assert!(clip.img.px[0][0] >= 0.0 && soft.img.px[0].iter().all(|v| *v >= -1e-6), "{:?} {:?}", clip.img.px[0], soft.img.px[0]);
    assert!(soft.img.px[0][0] > clip.img.px[0][0], "drawn in, not cut: {:?} {:?}", soft.img.px[0], clip.img.px[0]);
    // a colour well inside both gamuts: much as it is
    let mild = run(&PROFILE, &[("inputProfile", Value::Choice(3)), ("intent", Value::Choice(0))], Image::filled(1, 1, [0.4, 0.4, 0.38, 1.0]));
    let mild_rel = run(&PROFILE, &[("inputProfile", Value::Choice(3))], Image::filled(1, 1, [0.4, 0.4, 0.38, 1.0]));
    assert!(near3([mild.img.px[0][0], mild.img.px[0][1], mild.img.px[0][2]], [mild_rel.img.px[0][0], mild_rel.img.px[0][1], mild_rel.img.px[0][2]], 1e-6));
}

/// A layer that is darker now than at its reference frame.
struct Dimmer;

impl crate::Host for Dimmer {
    fn layer(&self, _: u64, _: crate::Take) -> Option<(Buf, Rect)> {
        None
    }

    fn this_at(&self, _: Tick, _: usize) -> Option<Buf> {
        Some(buf(ramp(1.0)))
    }
}

/// Grey from 0.1 to 0.9 across, times `k`.
fn ramp(k: f32) -> Image {
    let mut img = Image::new(40, 40);
    for y in 0..40 {
        for x in 0..40 {
            let v = (0.1 + 0.8 * x as f32 / 39.0) * k;
            img.px[y * 40 + x] = [v, v, v, 1.0];
        }
    }
    img
}

#[test]
fn stabilizes() {
    let mut g = instance(&STABILIZER, &[]);
    STABILIZER.start(&mut g, Rect::from_size(40.0, 40.0));
    let go = |mode: u32| {
        let mut g = g.clone();
        g.prop_mut("stabilize").unwrap().set_value(Value::Choice(mode)).unwrap();
        g.prop_mut("sampleSize").unwrap().set_value(n(1.0)).unwrap();
        crate::testing::run_when(&STABILIZER, &g, ramp(0.8), Tick::from_seconds(1.0), &[], Some(&Dimmer))
    };
    let want = ramp(1.0);
    for mode in 0..3 {
        let out = go(mode);
        for x in [10usize, 20, 30] {
            let (got, w) = (out.img.px[20 * 40 + x][0], want.px[20 * 40 + x][0]);
            assert!((got - w).abs() < 0.01, "mode {mode} at {x}: {got} for {w}");
        }
    }
    let _ = &mut g;
    let alone = crate::testing::run_when(&STABILIZER, &instance(&STABILIZER, &[]), ramp(0.8), Tick::ZERO, &[], None);
    assert_eq!(alone.img.px, ramp(0.8).px, "with no renderer: as it was");
}

#[test]
fn every_one_is_registered() {
    for spec in ALL {
        assert!(crate::find(spec.def.id).is_some(), "{}", spec.def.id);
    }
}

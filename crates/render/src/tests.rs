use super::*;
use veymelo_keyframe::{Keyframe, Value};
use veymelo_project::{Matte, MatteKind};
use veymelo_time::FrameRate;

const TWO: Tick = Tick(2 * veymelo_time::TICKS_PER_SECOND);

fn rgba(hex: &str) -> Rgba {
    Rgba::from_hex(hex).unwrap()
}

/// A layer showing a solid of `w` × `h` in `hex`, its anchor in its middle, at (`x`, `y`).
fn solid(p: &mut Project, name: &str, hex: &str, w: u32, h: u32, x: f64, y: f64) -> Layer {
    let mut l = Layer::new(p.alloc(), name, Source::Solid { color: rgba(hex), width: w, height: h }, TWO);
    l.props.prop_mut("transform/position").unwrap().set_value(Value::Vec3([x, y, 0.0])).unwrap();
    l.props.prop_mut("transform/anchor").unwrap().set_value(Value::Vec3([w as f64 / 2.0, h as f64 / 2.0, 0.0])).unwrap();
    l
}

/// A 200 × 100 grey composition with a red 50 × 20 solid in its middle.
fn scene() -> (Project, u64, u64) {
    let mut p = Project::new();
    let mut c = Comp::new(200, 100, FrameRate::FPS_30, TWO);
    c.background = rgba("#808080");
    let id = p.add_comp("Main", c);
    let red = solid(&mut p, "Red", "#ff0000", 50, 20, 100.0, 50.0);
    let lid = red.id;
    p.comp_mut(id).unwrap().layers.push(red);
    (p, id, lid)
}

fn draw(p: &Project, c: u64, t: f64) -> Rendered {
    render(p, c, Tick::from_seconds(t), &Options::default()).unwrap()
}

fn hex(img: &Image, x: usize, y: usize) -> String {
    img.color_at(x, y).to_hex()
}

fn layer(p: &mut Project, c: u64, id: u64) -> &mut Layer {
    p.comp_mut(c).unwrap().layer_mut(id).unwrap()
}

fn set(l: &mut Layer, path: &str, v: Value) {
    l.props.prop_mut(path).unwrap().set_value(v).unwrap();
}

#[test]
fn a_solid_in_the_middle() {
    let (p, c, _) = scene();
    let r = draw(&p, c, 0.0);
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    let img = &r.image;
    assert_eq!((img.width, img.height), (200, 100));
    assert_eq!(hex(img, 0, 0), "#808080");
    assert_eq!(hex(img, 100, 50), "#ff0000");
    assert_eq!(hex(img, 75, 40), "#ff0000", "its corner pixel is inside");
    assert_eq!(hex(img, 74, 40), "#808080", "and the one before it outside");
    assert_eq!(hex(img, 124, 59), "#ff0000");
    assert_eq!(hex(img, 125, 60), "#808080");
}

#[test]
fn opacity_rotation_and_parents() {
    let (mut p, c, lid) = scene();
    p.settings.bit_depth = 32;
    layer(&mut p, c, lid).props.prop_mut("transform/opacity").unwrap().set_keys(vec![Keyframe::new(Tick::ZERO, Value::Number(0.0)), Keyframe::new(Tick::SECOND, Value::Number(100.0))]).unwrap();
    let px = draw(&p, c, 0.5).image.color_at(100, 50);
    let grey = 128.0 / 255.0;
    assert!((px.r - (0.5 + 0.5 * grey)).abs() < 1e-6 && (px.g - 0.5 * grey).abs() < 1e-6, "half red over grey: {px:?}");
    let l = layer(&mut p, c, lid);
    set(l, "transform/opacity", Value::Number(100.0));
    set(l, "transform/rotation", Value::Number(90.0));
    let turned = draw(&p, c, 0.0).image;
    assert_eq!(hex(&turned, 100, 28), "#ff0000", "turned a quarter: 20 wide, 50 tall");
    assert_eq!(hex(&turned, 80, 50), "#808080");
    // a parent null moves it 50 right and makes it twice as wide
    let mut null = Layer::new(p.alloc(), "Rig", Source::Null, TWO);
    set(&mut null, "transform/position", Value::Vec3([50.0, 0.0, 0.0]));
    set(&mut null, "transform/scale", Value::Vec3([200.0, 100.0, 100.0]));
    let nid = null.id;
    p.comp_mut(c).unwrap().layers.push(null);
    let l = layer(&mut p, c, lid);
    set(l, "transform/rotation", Value::Number(0.0));
    l.parent = Some(nid);
    let comp = p.comp(c).unwrap();
    let e = Eval::new(&p);
    assert_eq!(e.matrix(e.place(comp, comp.layer(lid).unwrap()), Tick::ZERO).apply(vec2(25.0, 10.0)), vec2(250.0, 50.0));
}

#[test]
fn edges_between_pixels_cover_exactly_their_area() {
    let (mut p, c, lid) = scene();
    p.settings.bit_depth = 32;
    set(layer(&mut p, c, lid), "transform/position", Value::Vec3([100.5, 50.0, 0.0]));
    let img = render(&p, c, Tick::ZERO, &Options { background: false, ..Options::default() }).unwrap().image;
    assert!((img.px[50 * 200 + 75][3] - 0.5).abs() < 1e-4, "{:?}", img.px[50 * 200 + 75]);
    assert_eq!(img.px[50 * 200 + 76][3], 1.0);
    assert_eq!(img.px[0], [0.0; 4], "no background asked for");
    let total: f32 = img.px.iter().map(|p| p[3]).sum();
    assert!((total - 1000.0).abs() < 0.05, "exactly its area: {total}");
}

#[test]
fn a_small_layer_scaled_up_stays_sharp() {
    let (mut p, c, lid) = scene();
    let l = layer(&mut p, c, lid);
    l.source = Source::Solid { color: rgba("#ff0000"), width: 5, height: 2 };
    set(l, "transform/anchor", Value::Vec3([2.5, 1.0, 0.0]));
    set(l, "transform/scale", Value::Vec3([1000.0, 1000.0, 100.0]));
    let img = draw(&p, c, 0.0).image;
    // 5 × 2 at ten times: 50 × 20 again, its edge as sharp as the unscaled one
    assert_eq!(hex(&img, 75, 50), "#ff0000");
    assert_eq!(hex(&img, 74, 50), "#808080");
}

#[test]
fn blend_modes_mix_with_what_is_under() {
    let (mut p, c, lid) = scene();
    layer(&mut p, c, lid).blend = BlendMode::Multiply;
    assert_eq!(hex(&draw(&p, c, 0.0).image, 100, 50), "#ff0000", "the background is not a layer: nothing to mix with");
    let grey = solid(&mut p, "Grey", "#808080", 200, 100, 100.0, 50.0);
    p.comp_mut(c).unwrap().layers.push(grey);
    assert_eq!(hex(&draw(&p, c, 0.0).image, 100, 50), "#800000", "red times grey");
    layer(&mut p, c, lid).blend = BlendMode::Screen;
    assert_eq!(hex(&draw(&p, c, 0.0).image, 100, 50), "#ff8080");
    layer(&mut p, c, lid).blend = BlendMode::Difference;
    assert_eq!(hex(&draw(&p, c, 0.0).image, 100, 50), "#7f8080");
    // a stencil keeps what is under it only where it is
    layer(&mut p, c, lid).blend = BlendMode::StencilAlpha;
    let img = render(&p, c, Tick::ZERO, &Options { background: false, ..Options::default() }).unwrap().image;
    assert_eq!(img.px[0], [0.0; 4]);
    let img = draw(&p, c, 0.0).image;
    assert_eq!(hex(&img, 100, 50), "#808080", "the grey layer shows through the stencil");
    let cut = render(&p, c, Tick::ZERO, &Options { background: false, ..Options::default() }).unwrap().image;
    assert_eq!(cut.px[10 * 200 + 10], [0.0; 4], "and outside it the layers under it are gone");
}

#[test]
fn dissolve_is_half_and_half_at_half_opacity() {
    let (mut p, c, lid) = scene();
    let l = layer(&mut p, c, lid);
    l.blend = BlendMode::Dissolve;
    set(l, "transform/opacity", Value::Number(50.0));
    let img = draw(&p, c, 0.0).image;
    let reds = (40..60).flat_map(|y| (76..124).map(move |x| (x, y))).filter(|&(x, y)| hex(&img, x, y) == "#ff0000").count();
    let all = 20 * 48;
    assert!((reds as f64 / all as f64 - 0.5).abs() < 0.06, "{reds} of {all}");
}

#[test]
fn track_mattes_cut_by_alpha_and_brightness() {
    let (mut p, c, lid) = scene();
    // a matte half the solid's width, over its left half
    let mut m = solid(&mut p, "Matte", "#ffffff", 25, 20, 87.5, 50.0);
    m.switches.video = false;
    let mid = m.id;
    p.comp_mut(c).unwrap().layers.insert(0, m);
    layer(&mut p, c, lid).matte = Some(Matte { layer: mid, kind: MatteKind::Alpha });
    let img = draw(&p, c, 0.0).image;
    assert_eq!(hex(&img, 80, 50), "#ff0000", "seen through the matte");
    assert_eq!(hex(&img, 120, 50), "#808080", "cut where the matte is not");
    layer(&mut p, c, lid).matte = Some(Matte { layer: mid, kind: MatteKind::AlphaInverted });
    let img = draw(&p, c, 0.0).image;
    assert_eq!((hex(&img, 80, 50).as_str(), hex(&img, 120, 50).as_str()), ("#808080", "#ff0000"));
    // a black matte lets nothing through by brightness, everything by alpha
    let ml = layer(&mut p, c, mid);
    ml.source = Source::Solid { color: rgba("#000000"), width: 25, height: 20 };
    layer(&mut p, c, lid).matte = Some(Matte { layer: mid, kind: MatteKind::Luma });
    assert_eq!(hex(&draw(&p, c, 0.0).image, 80, 50), "#808080");
    layer(&mut p, c, lid).matte = Some(Matte { layer: mid, kind: MatteKind::LumaInverted });
    assert_eq!(hex(&draw(&p, c, 0.0).image, 80, 50), "#ff0000");
}

#[test]
fn preserve_transparency_draws_only_on_what_is_there() {
    let mut p = Project::new();
    let c = p.add_comp("Clear", Comp::new(100, 100, FrameRate::FPS_30, TWO));
    let under = solid(&mut p, "Card", "#0000ff", 40, 40, 30.0, 50.0);
    let mut over = solid(&mut p, "Paint", "#ffff00", 100, 20, 50.0, 50.0);
    over.preserve_transparency = true;
    p.comp_mut(c).unwrap().layers = vec![over, under];
    let img = render(&p, c, Tick::ZERO, &Options { background: false, ..Options::default() }).unwrap().image;
    assert_eq!(hex(&img, 30, 50), "#ffff00", "on the card");
    assert_eq!(img.px[50 * 100 + 80], [0.0; 4], "off the card, nothing");
}

#[test]
fn compositions_inside_compositions() {
    let (mut p, c, lid) = scene();
    p.comp_mut(c).unwrap().layers.clear();
    // a 40 × 40 composition with a green square in its top-left quarter, lasting 1 s
    let mut inner = Comp::new(40, 40, FrameRate::FPS_30, Tick::SECOND);
    inner.background = rgba("#ff00ff");
    let iid = p.add_comp("Inner", inner);
    let g = solid(&mut p, "Green", "#00ff00", 20, 20, 10.0, 10.0);
    p.comp_mut(iid).unwrap().layers.push(g);
    let mut nest = Layer::new(p.alloc(), "Nest", Source::Comp(iid), Tick::SECOND);
    set(&mut nest, "transform/position", Value::Vec3([100.0, 50.0, 0.0]));
    set(&mut nest, "transform/anchor", Value::Vec3([20.0, 20.0, 0.0]));
    set(&mut nest, "transform/scale", Value::Vec3([200.0, 200.0, 100.0]));
    let nid = nest.id;
    p.comp_mut(c).unwrap().layers.push(nest);
    let _ = lid;
    let img = draw(&p, c, 0.0).image;
    // drawn twice as big around the middle: the green covers (60…100, 10…50)
    assert_eq!(hex(&img, 61, 11), "#00ff00");
    assert_eq!(hex(&img, 99, 49), "#00ff00");
    assert_eq!(hex(&img, 101, 51), "#808080", "the inner background is not drawn");
    assert_eq!(hex(&img, 59, 11), "#808080");
    // the layer starts a second later: its composition's time starts with it
    layer(&mut p, c, nid).start = Tick::SECOND;
    layer(&mut p, c, nid).out_point = TWO;
    assert_eq!(hex(&draw(&p, c, 0.5).image, 80, 30), "#808080", "not yet");
    assert_eq!(hex(&draw(&p, c, 1.5).image, 80, 30), "#00ff00");
    // collapsed: the same picture, the inner layers drawn straight in
    layer(&mut p, c, nid).switches.collapse = true;
    let img = draw(&p, c, 1.5).image;
    assert_eq!(hex(&img, 61, 11), "#00ff00");
    assert_eq!(hex(&img, 101, 51), "#808080");
    // and an inner layer's blend mode then mixes with this composition's layers
    let grey = solid(&mut p, "Grey", "#808080", 200, 100, 100.0, 50.0);
    p.comp_mut(c).unwrap().layers.push(grey);
    p.comp_mut(iid).unwrap().layers[0].blend = BlendMode::Multiply;
    assert_eq!(hex(&draw(&p, c, 1.5).image, 80, 30), "#008000", "green times grey");
    layer(&mut p, c, nid).switches.collapse = false;
    assert_eq!(hex(&draw(&p, c, 1.5).image, 80, 30), "#00ff00", "not collapsed, it multiplies only the empty inside");
}

#[test]
fn motion_blur_spreads_a_moving_layer_along_its_path() {
    let (mut p, c, lid) = scene();
    p.settings.bit_depth = 32;
    let l = layer(&mut p, c, lid);
    // 20 × 20, moving 300 px a second to the right: 10 px a frame, 5 px while a 180° shutter is open
    l.source = Source::Solid { color: rgba("#ffffff"), width: 20, height: 20 };
    set(l, "transform/anchor", Value::Vec3([10.0, 10.0, 0.0]));
    l.props.prop_mut("transform/position").unwrap().set_keys(vec![Keyframe::new(Tick::ZERO, Value::Vec3([50.0, 50.0, 0.0])), Keyframe::new(Tick::SECOND, Value::Vec3([350.0, 50.0, 0.0]))]).unwrap();
    l.switches.motion_blur = true;
    let clear = Options { background: false, ..Options::default() };
    let img = render(&p, c, Tick::from_seconds(0.25), &clear).unwrap().image;
    let row: Vec<f32> = (0..200).map(|x| img.px[50 * 200 + x][3]).collect();
    let total: f32 = img.px.iter().map(|p| p[3]).sum();
    assert!((total - 400.0).abs() < 1.0, "the blur keeps the layer's area: {total}");
    // at 0.25 s the layer is centred at 125; the shutter opens 90° before (2.5 px back) for 5 px
    assert!((row[114] - 0.4).abs() < 0.1, "a soft left edge, 40 % of the shutter covered: {}", row[114]);
    assert!((row[125] - 1.0).abs() < 1e-3, "solid in the middle");
    assert!((row[135] - 0.4).abs() < 0.1, "a soft right edge, the same: {}", row[135]);
    assert_eq!(row[111], 0.0);
    // with the composition's motion blur off, the edges are hard again
    p.comp_mut(c).unwrap().enable_motion_blur = false;
    let img = render(&p, c, Tick::from_seconds(0.25), &clear).unwrap().image;
    assert_eq!(img.px[50 * 200 + 115][3], 1.0);
    assert_eq!(img.px[50 * 200 + 114][3], 0.0);
}

#[test]
fn eight_bits_round_every_step_and_thirty_two_do_not() {
    let (mut p, c, lid) = scene();
    set(layer(&mut p, c, lid), "transform/opacity", Value::Number(33.3));
    let eight = draw(&p, c, 0.0).image.px[50 * 200 + 100];
    for v in eight {
        assert!((v * 255.0 - (v * 255.0).round()).abs() < 1e-4, "{eight:?}");
    }
    p.settings.bit_depth = 32;
    let float = draw(&p, c, 0.0).image.px[50 * 200 + 100];
    assert!((float[1] * 255.0 - (float[1] * 255.0).round()).abs() > 1e-3, "{float:?}");
}

#[test]
fn linear_light_mixes_brighter() {
    let mut p = Project::new();
    let mut comp = Comp::new(10, 10, FrameRate::FPS_30, TWO);
    comp.background = rgba("#000000");
    let c = p.add_comp("Light", comp);
    let mut white = solid(&mut p, "White", "#ffffff", 10, 10, 5.0, 5.0);
    set(&mut white, "transform/opacity", Value::Number(50.0));
    p.comp_mut(c).unwrap().layers.push(white);
    p.settings.bit_depth = 32;
    let stored = draw(&p, c, 0.0).image.color_at(5, 5).r;
    assert!((stored - 0.5).abs() < 1e-6);
    p.settings.linear_blending = true;
    let light = draw(&p, c, 0.0).image.color_at(5, 5).r;
    assert!((light - veymelo_color::linear_to_srgb(0.5)).abs() < 1e-5, "{light}");
}

#[test]
fn solo_and_guide_layers() {
    let (mut p, c, lid) = scene();
    let blue = solid(&mut p, "Blue", "#0000ff", 20, 20, 20.0, 20.0);
    let bid = blue.id;
    p.comp_mut(c).unwrap().layers.push(blue);
    layer(&mut p, c, lid).switches.solo = true;
    let img = draw(&p, c, 0.0).image;
    assert_eq!(hex(&img, 20, 20), "#808080", "only the solo layer is seen");
    assert_eq!(hex(&img, 100, 50), "#ff0000");
    layer(&mut p, c, lid).switches.solo = false;
    layer(&mut p, c, bid).switches.guide = true;
    assert_eq!(hex(&draw(&p, c, 0.0).image, 20, 20), "#808080", "guides are left out of renders");
    let shown = render(&p, c, Tick::ZERO, &Options { guides: true, ..Options::default() }).unwrap().image;
    assert_eq!(hex(&shown, 20, 20), "#0000ff", "but shown when asked for");
}

#[test]
fn draft_and_wireframe() {
    let (mut p, c, lid) = scene();
    set(layer(&mut p, c, lid), "transform/position", Value::Vec3([100.5, 50.0, 0.0]));
    layer(&mut p, c, lid).switches.quality = Quality::Draft;
    let img = draw(&p, c, 0.0).image;
    assert!(["#ff0000", "#808080"].contains(&hex(&img, 75, 50).as_str()), "draft has no in-between pixels");
    layer(&mut p, c, lid).switches.quality = Quality::Wireframe;
    layer(&mut p, c, lid).label = veymelo_project::Label::Blue;
    let img = draw(&p, c, 0.0).image;
    assert_eq!(hex(&img, 100, 50), "#808080", "a wireframe is only an outline");
    assert_ne!(hex(&img, 100, 40), "#808080");
}

#[test]
fn what_is_left_out_is_said() {
    let (mut p, c, lid) = scene();
    layer(&mut p, c, lid).switches.three_d = true;
    let t = Layer::new(p.alloc(), "Clip", Source::Footage(999), TWO);
    p.comp_mut(c).unwrap().layers.insert(0, t);
    let r = draw(&p, c, 0.0);
    assert_eq!(r.notes, ["\"Clip\": its footage (item 999) is not in the project"]);
    assert!(render(&p, 99, Tick::ZERO, &Options::default()).is_err());
    let small = render(&p, c, Tick::ZERO, &Options { scale: 0.25, ..Options::default() }).unwrap().image;
    assert_eq!((small.width, small.height), (50, 25));
}

/// A frame with most of what stage 1 draws, written out to look at.
#[test]
fn a_frame_to_look_at() {
    let mut p = Project::new();
    let mut comp = Comp::new(480, 270, FrameRate::FPS_30, TWO);
    comp.background = rgba("#f4f1e8");
    let c = p.add_comp("Look", comp);
    let mut layers = Vec::new();
    // a card turned and scaled under a parent, a multiply stripe, a luma-matted block, a blurred mover
    let mut rig = Layer::new(p.alloc(), "Rig", Source::Null, TWO);
    set(&mut rig, "transform/position", Value::Vec3([140.0, 135.0, 0.0]));
    set(&mut rig, "transform/rotation", Value::Number(-12.0));
    let mut card = solid(&mut p, "Card", "#2f4b3a", 160, 110, 0.0, 0.0);
    card.parent = Some(rig.id);
    let mut stripe = solid(&mut p, "Stripe", "#f2a33a", 480, 40, 240.0, 150.0);
    stripe.blend = BlendMode::Multiply;
    let mut gradient = solid(&mut p, "Matte", "#ffffff", 120, 120, 380.0, 90.0);
    set(&mut gradient, "transform/rotation", Value::Number(45.0));
    gradient.switches.video = false;
    let mut block = solid(&mut p, "Block", "#3a6cf2", 140, 140, 380.0, 90.0);
    block.matte = Some(Matte { layer: gradient.id, kind: MatteKind::Alpha });
    let mut mover = solid(&mut p, "Mover", "#c0392b", 40, 40, 0.0, 0.0);
    mover.props.prop_mut("transform/position").unwrap().set_keys(vec![Keyframe::new(Tick::ZERO, Value::Vec3([300.0, 220.0, 0.0])), Keyframe::new(Tick::SECOND, Value::Vec3([620.0, 220.0, 0.0]))]).unwrap();
    mover.props.prop_mut("transform/rotation").unwrap().set_keys(vec![Keyframe::new(Tick::ZERO, Value::Number(0.0)), Keyframe::new(Tick::SECOND, Value::Number(180.0))]).unwrap();
    mover.switches.motion_blur = true;
    layers.extend([mover, gradient, block, stripe, card, rig]);
    p.comp_mut(c).unwrap().layers = layers;
    let r = render(&p, c, Tick::from_seconds(0.2), &Options::default()).unwrap();
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    if let Some(dir) = std::env::var_os("VEYMELO_FRAMES") {
        std::fs::write(std::path::Path::new(&dir).join("stage1.png"), to_png(&r.image)).unwrap();
    }
}

use veymelo_project::shapes as sh;
use veymelo_project::{Group, Node};

/// A shape layer in a clear 200 × 200 composition, its contents what `items` makes.
fn shape_scene(items: Vec<Group>) -> (Project, u64) {
    let mut p = Project::new();
    let c = p.add_comp("Shapes", Comp::new(200, 200, FrameRate::FPS_30, TWO));
    let mut l = Layer::new(p.alloc(), "Shape", Source::Shape, TWO);
    set(&mut l, "transform/position", Value::Vec3([100.0, 100.0, 0.0]));
    l.props.group_mut("contents").unwrap().children = items.into_iter().map(Node::Group).collect();
    p.comp_mut(c).unwrap().layers.push(l);
    p.settings.bit_depth = 32;
    (p, c)
}

fn item(def: &'static veymelo_project::GroupDef, props: &[(&str, Value)]) -> Group {
    let mut g = Group::instance(def, 0, def.name);
    for (path, v) in props {
        g.prop_mut(path).unwrap().set_value(v.clone()).unwrap();
    }
    g
}

fn clear(p: &Project, c: u64, t: f64) -> Image {
    render(p, c, Tick::from_seconds(t), &Options { background: false, ..Options::default() }).unwrap().image
}

fn alpha_sum(img: &Image) -> f32 {
    img.px.iter().map(|p| p[3]).sum()
}

#[test]
fn a_rectangle_filled_and_stroked() {
    let red = Value::Color(rgba("#ff0000"));
    let (p, c) = shape_scene(vec![item(&sh::RECT, &[("size", Value::Vec2([80.0, 40.0]))]), item(&sh::FILL, &[("color", red)])]);
    let img = clear(&p, c, 0.0);
    assert!((alpha_sum(&img) - 3200.0).abs() < 0.5, "{}", alpha_sum(&img));
    assert_eq!(hex(&img, 100, 100), "#ff0000");
    assert_eq!(img.px[100 * 200 + 59][3], 0.0);
    // a 10 px stroke round it (listed first, so drawn on top), half inside, half out
    let (p, c) = shape_scene(vec![
        item(&sh::RECT, &[("size", Value::Vec2([80.0, 40.0]))]),
        item(&sh::STROKE, &[("width", Value::Number(10.0)), ("color", Value::Color(rgba("#0000ff")))]),
        item(&sh::FILL, &[("color", Value::Color(rgba("#ff0000")))]),
    ]);
    let img = clear(&p, c, 0.0);
    assert!((alpha_sum(&img) - 90.0 * 50.0).abs() < 1.0, "{}", alpha_sum(&img));
    assert_eq!(hex(&img, 61, 100), "#0000ff", "the stroke is over the fill");
    assert_eq!(hex(&img, 100, 100), "#ff0000");
}

#[test]
fn groups_move_and_scale_their_contents_and_strokes() {
    let mut g = item(&sh::GROUP, &[("transform/scale", Value::Vec2([200.0, 200.0])), ("transform/position", Value::Vec2([20.0, 0.0]))]);
    g.group_mut("contents").unwrap().children = vec![Node::Group(item(&sh::RECT, &[("size", Value::Vec2([20.0, 20.0]))])), Node::Group(item(&sh::STROKE, &[("width", Value::Number(2.0))]))];
    let (p, c) = shape_scene(vec![g]);
    let img = clear(&p, c, 0.0);
    // a 40 × 40 square around (120, 100) with a 4 px stroke (scaled with it)
    let ring = 44.0 * 44.0 - 36.0 * 36.0;
    assert!((alpha_sum(&img) - ring).abs() < 1.5, "{} vs {ring}", alpha_sum(&img));
    assert!(img.px[100 * 200 + 99][3] > 0.99, "the left side of the square, at x 100");
    assert_eq!(img.px[100 * 200 + 120][3], 0.0, "the middle is not filled");
}

#[test]
fn trim_draws_part_of_a_stroke() {
    let (p, c) = shape_scene(vec![
        item(&sh::RECT, &[("size", Value::Vec2([100.0, 100.0]))]),
        item(&sh::STROKE, &[("width", Value::Number(2.0)), ("cap", Value::Choice(0))]),
        item(&sh::TRIM, &[("end", Value::Number(50.0))]),
    ]);
    let half = alpha_sum(&clear(&p, c, 0.0));
    assert!((half - 400.0).abs() < 3.0, "half of the outline: {half}");
}

#[test]
fn gradients_go_from_start_to_end() {
    let colors = Value::from_json(veymelo_keyframe::Kind::Gradient, &veymelo_json::Value::parse(r##"["#ff0000", "#0000ff"]"##).unwrap()).unwrap();
    let (p, c) = shape_scene(vec![
        item(&sh::RECT, &[("size", Value::Vec2([100.0, 100.0]))]),
        item(&sh::GRADIENT_FILL, &[("start", Value::Vec2([-50.0, 0.0])), ("end", Value::Vec2([50.0, 0.0])), ("colors", colors.clone())]),
    ]);
    let img = clear(&p, c, 0.0);
    let (l, m, r) = (img.color_at(51, 100), img.color_at(100, 100), img.color_at(148, 100));
    assert!(l.r > 0.95 && l.b < 0.05 && r.b > 0.95 && r.r < 0.05, "{l:?} {r:?}");
    assert!((m.r - 0.5).abs() < 0.02 && (m.b - 0.5).abs() < 0.02, "{m:?}");
    // radial: red at the centre, blue at the radius
    let (p, c) = shape_scene(vec![
        item(&sh::ELLIPSE, &[("size", Value::Vec2([100.0, 100.0]))]),
        item(&sh::GRADIENT_FILL, &[("type", Value::Choice(1)), ("start", Value::Vec2([0.0, 0.0])), ("end", Value::Vec2([50.0, 0.0])), ("colors", colors)]),
    ]);
    let img = clear(&p, c, 0.0);
    assert!(img.color_at(100, 100).r > 0.95 && img.color_at(146, 100).b > 0.9);
}

#[test]
fn repeaters_merges_and_paint_order() {
    // three 10 × 10 squares, 20 apart
    let (p, c) = shape_scene(vec![
        item(&sh::RECT, &[("size", Value::Vec2([10.0, 10.0]))]),
        item(&sh::FILL, &[]),
        item(&sh::REPEATER, &[("transform/position", Value::Vec2([20.0, 0.0]))]),
    ]);
    let img = clear(&p, c, 0.0);
    assert!((alpha_sum(&img) - 300.0).abs() < 0.5, "{}", alpha_sum(&img));
    assert!(img.px[100 * 200 + 140][3] > 0.99, "the third copy at 140");
    // a ring: a circle less a smaller one
    let (p, c) = shape_scene(vec![
        item(&sh::ELLIPSE, &[("size", Value::Vec2([100.0, 100.0]))]),
        item(&sh::ELLIPSE, &[("size", Value::Vec2([50.0, 50.0]))]),
        item(&sh::MERGE, &[("mode", Value::Choice(2))]),
        item(&sh::FILL, &[]),
    ]);
    let ring = alpha_sum(&clear(&p, c, 0.0));
    let want = std::f32::consts::PI * (2500.0 - 625.0);
    assert!((ring - want).abs() / want < 0.01, "{ring} vs {want}");
    // even-odd makes the same ring without merging
    let (p, c) = shape_scene(vec![
        item(&sh::ELLIPSE, &[("size", Value::Vec2([100.0, 100.0]))]),
        item(&sh::ELLIPSE, &[("size", Value::Vec2([50.0, 50.0]))]),
        item(&sh::FILL, &[("rule", Value::Choice(1))]),
    ]);
    assert!((alpha_sum(&clear(&p, c, 0.0)) - want).abs() / want < 0.01);
}

#[test]
fn masks_cut_layers() {
    let (mut p, c, lid) = scene();
    p.settings.bit_depth = 32;
    let mask = |mode: u32, inverted: bool| {
        let mut m = Group::instance(&veymelo_project::MASK, 0, "Mask 1");
        let sq = veymelo_keyframe::ShapePath::polygon(&[[0.0, 0.0], [25.0, 0.0], [25.0, 20.0], [0.0, 20.0]], true);
        m.prop_mut("path").unwrap().set_value(Value::Path(sq)).unwrap();
        m.prop_mut("mode").unwrap().set_value(Value::Choice(mode)).unwrap();
        m.prop_mut("inverted").unwrap().set_value(Value::Bool(inverted)).unwrap();
        m
    };
    let with = |p: &mut Project, masks: Vec<Group>| {
        layer(p, c, lid).props.group_mut("masks").unwrap().children = masks.into_iter().map(Node::Group).collect();
        alpha_sum(&clear(p, c, 0.0))
    };
    // the left half of the 50 × 20 solid
    assert!((with(&mut p, vec![mask(1, false)]) - 500.0).abs() < 0.5);
    assert!((with(&mut p, vec![mask(2, false)]) - 500.0).abs() < 0.5, "subtracted: the right half");
    assert!((with(&mut p, vec![mask(1, true)]) - 500.0).abs() < 0.5, "inverted: the right half");
    assert!((with(&mut p, vec![mask(1, false), mask(2, false)]) - 0.0).abs() < 0.5, "added then taken away");
    // grown by 5 px into the solid's other half
    let mut grown = mask(1, false);
    grown.prop_mut("expansion").unwrap().set_value(Value::Number(5.0)).unwrap();
    assert!((with(&mut p, vec![grown]) - 600.0).abs() < 1.5, "{}", with(&mut p, vec![]));
    // a feathered edge spreads but keeps about the same amount
    let mut soft = mask(1, false);
    soft.prop_mut("feather").unwrap().set_value(Value::Vec2([10.0, 0.0])).unwrap();
    layer(&mut p, c, lid).props.group_mut("masks").unwrap().children = vec![Node::Group(soft)];
    let img = clear(&p, c, 0.0);
    let edge = img.px[50 * 200 + 100][3];
    assert!(edge > 0.3 && edge < 0.7, "soft at the edge: {edge}");
}

/// Shapes and masks, written out to look at.
#[test]
fn shapes_to_look_at() {
    let colors = Value::from_json(veymelo_keyframe::Kind::Gradient, &veymelo_json::Value::parse(r##"["#f2a33a", "#c0392b"]"##).unwrap()).unwrap();
    let mut star = item(&sh::GROUP, &[("transform/position", Value::Vec2([-50.0, 0.0]))]);
    star.group_mut("contents").unwrap().children = vec![
        Node::Group(item(&sh::STAR, &[("outerRadius", Value::Number(60.0)), ("innerRadius", Value::Number(28.0)), ("outerRoundness", Value::Number(20.0))])),
        Node::Group(item(&sh::STROKE, &[("width", Value::Number(4.0)), ("join", Value::Choice(1)), ("color", Value::Color(rgba("#2f4b3a")))])),
        Node::Group(item(&sh::GRADIENT_FILL, &[("type", Value::Choice(1)), ("end", Value::Vec2([60.0, 0.0])), ("colors", colors)])),
    ];
    let mut dots = item(&sh::GROUP, &[("transform/position", Value::Vec2([20.0, 60.0]))]);
    dots.group_mut("contents").unwrap().children = vec![
        Node::Group(item(&sh::ELLIPSE, &[("size", Value::Vec2([12.0, 12.0]))])),
        Node::Group(item(&sh::FILL, &[("color", Value::Color(rgba("#3a6cf2")))])),
        Node::Group(item(&sh::REPEATER, &[("copies", Value::Number(5.0)), ("transform/position", Value::Vec2([18.0, 0.0])), ("transform/endOpacity", Value::Number(20.0))])),
    ];
    let mut wave = item(&sh::GROUP, &[("transform/position", Value::Vec2([50.0, -40.0]))]);
    wave.group_mut("contents").unwrap().children = vec![
        Node::Group(item(&sh::RECT, &[("size", Value::Vec2([70.0, 50.0]))])),
        Node::Group(item(&sh::ZIGZAG, &[("size", Value::Number(4.0)), ("ridges", Value::Number(6.0)), ("points", Value::Choice(1))])),
        Node::Group(item(&sh::STROKE, &[("width", Value::Number(3.0)), ("color", Value::Color(rgba("#2f4b3a"))), ("dashes/dash", Value::Number(6.0))])),
    ];
    let (mut p, c) = shape_scene(vec![star, dots, wave]);
    p.comp_mut(c).unwrap().background = rgba("#f4f1e8");
    let r = render(&p, c, Tick::ZERO, &Options::default()).unwrap();
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    if let Some(dir) = std::env::var_os("VEYMELO_FRAMES") {
        std::fs::write(std::path::Path::new(&dir).join("shapes.png"), to_png(&r.image)).unwrap();
    }
}

/// A text layer in a dark 600 × 200 composition, its origin at (20, 120).
fn text_scene(doc: &str) -> (Project, u64, u64) {
    let mut p = Project::new();
    let mut comp = Comp::new(600, 200, FrameRate::FPS_30, TWO);
    comp.background = rgba("#101010");
    let c = p.add_comp("Words", comp);
    let mut l = Layer::new(p.alloc(), "Words", Source::Text, TWO);
    set(&mut l, "transform/position", Value::Vec3([20.0, 120.0, 0.0]));
    let v = Value::from_json(veymelo_keyframe::Kind::Doc, &veymelo_json::Value::parse(doc).unwrap()).unwrap();
    l.props.prop_mut("text/sourceText").unwrap().set_value(v).unwrap();
    let id = l.id;
    p.comp_mut(c).unwrap().layers.push(l);
    p.settings.bit_depth = 32;
    (p, c, id)
}

#[cfg(target_os = "macos")]
#[test]
fn text_is_drawn_in_its_font_and_colour() {
    let (p, c, _) = text_scene(r##"{"text": "Hello สวัสดี", "font": "Tahoma", "size": 64, "fill": "#ffcc00"}"##);
    let r = render(&p, c, Tick::ZERO, &Options { background: false, ..Options::default() }).unwrap();
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    let img = &r.image;
    let ink = alpha_sum(img);
    assert!(ink > 1000.0, "there are letters: {ink}");
    // all of it above the baseline but the descenders, and starting at the origin
    let rows: Vec<f32> = (0..200).map(|y| (0..600).map(|x| img.px[y * 600 + x][3]).sum()).collect();
    assert!(rows[..60].iter().all(|&v| v == 0.0), "nothing far above the letters");
    assert!(rows[100] > 10.0, "letters just above the baseline");
    let first_col = (0..600).find(|&x| (0..200).any(|y| img.px[y * 600 + x][3] > 0.5)).unwrap();
    assert!((20..30).contains(&first_col), "the H starts at the origin: {first_col}");
    let yellow = (0..600 * 200).map(|i| img.px[i]).find(|p| p[3] > 0.99).unwrap();
    assert!((yellow[0] - 1.0).abs() < 1e-4 && (yellow[1] - 0.8).abs() < 1e-3, "{yellow:?}");
}

#[cfg(target_os = "macos")]
#[test]
fn animators_change_what_their_selectors_pick() {
    let (mut p, c, id) = text_scene(r##"{"text": "ABCD", "font": "Tahoma", "size": 64}"##);
    let plain = clear(&p, c, 0.0);
    // an animator that fades out the first half
    let mut a = Group::instance(&veymelo_project::text::ANIMATOR, 0, "Animator 1");
    a.prop_mut("opacity").unwrap().set_value(Value::Number(0.0)).unwrap();
    let mut r = Group::instance(&veymelo_project::text::RANGE, 0, "Range Selector 1");
    r.prop_mut("end").unwrap().set_value(Value::Number(50.0)).unwrap();
    a.group_mut("selectors").unwrap().children.push(Node::Group(r));
    layer(&mut p, c, id).props.group_mut("text/animators").unwrap().children.push(Node::Group(a));
    let faded = clear(&p, c, 0.0);
    let half = |img: &Image, from: usize, to: usize| (0..200).flat_map(|y| (from..to).map(move |x| (x, y))).map(|(x, y)| img.px[y * 600 + x][3]).sum::<f32>();
    // where C starts: the first half (A, B) is picked, the rest is not
    let lay = veymelo_text::lay_out(&veymelo_keyframe::TextDoc { font: "Tahoma".into(), size: 64.0, ..veymelo_keyframe::TextDoc::of("ABCD") });
    let split = (20.0 + lay.glyphs[2].x).round() as usize;
    assert!(half(&faded, 0, split) < half(&plain, 0, split) * 0.5, "the first letters faded");
    assert!((half(&faded, split, 600) - half(&plain, split, 600)).abs() < 1.0, "the rest as they were");
    // a position offset moves the picked letters
    let a = layer(&mut p, c, id).props.group_mut("text/animators/Animator 1").unwrap();
    a.prop_mut("opacity").unwrap().set_value(Value::Number(100.0)).unwrap();
    a.prop_mut("position").unwrap().set_value(Value::Vec3([0.0, -40.0, 0.0])).unwrap();
    let moved = clear(&p, c, 0.0);
    let top = |img: &Image, from: usize, to: usize| (0..200).find(|&y| (from..to).any(|x| img.px[y * 600 + x][3] > 0.5)).unwrap_or(200);
    assert!(top(&moved, 0, split) + 30 < top(&plain, 0, split), "lifted 40 px");
    assert_eq!(top(&moved, split, 600), top(&plain, split, 600));
}

/// Thai and Latin text with an animator, written out to look at.
#[cfg(target_os = "macos")]
#[test]
fn text_to_look_at() {
    let (mut p, c, id) = text_scene(r##"{"text": "สวัสดีครับ Sunday", "font": ".ThonburiUI", "weight": 700, "size": 72, "fill": "#f4f1e8", "stroke": "#c0392b", "strokeWidth": 3}"##);
    let mut a = Group::instance(&veymelo_project::text::ANIMATOR, 0, "Animator 1");
    a.prop_mut("rotation").unwrap().set_value(Value::Number(-12.0)).unwrap();
    a.prop_mut("fillColor").unwrap().set_value(Value::Color(rgba("#f2a33a"))).unwrap();
    let mut r = Group::instance(&veymelo_project::text::RANGE, 0, "Range Selector 1");
    r.prop_mut("shape").unwrap().set_value(Value::Choice(3)).unwrap();
    a.group_mut("selectors").unwrap().children.push(Node::Group(r));
    layer(&mut p, c, id).props.group_mut("text/animators").unwrap().children.push(Node::Group(a));
    let r = render(&p, c, Tick::ZERO, &Options::default()).unwrap();
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    if let Some(dir) = std::env::var_os("VEYMELO_FRAMES") {
        std::fs::write(std::path::Path::new(&dir).join("text.png"), to_png(&r.image)).unwrap();
    }
}

/// Emoji drawn in their own colours, a family joined into one picture.
#[cfg(target_os = "macos")]
#[test]
fn emoji_are_drawn_in_colour() {
    let doc = r##"{"text": "A😀👨‍👩‍👧", "font": "Tahoma", "size": 64, "fill": "#ffffff"}"##;
    let lay = veymelo_text::lay_out(&veymelo_keyframe::TextDoc { font: "Tahoma".into(), size: 64.0, ..veymelo_keyframe::TextDoc::of("A😀👨‍👩‍👧") });
    let emoji: Vec<_> = lay.glyphs.iter().filter(|g| g.font.has_color()).collect();
    if emoji.is_empty() {
        // no colour emoji font here
        return;
    }
    assert_eq!(emoji.len(), 2, "the face and the family as one: {:?}", lay.glyphs.iter().map(|g| (g.glyph, g.char_index)).collect::<Vec<_>>());
    let (p, c, _) = text_scene(doc);
    let r = render(&p, c, Tick::ZERO, &Options { background: false, ..Options::default() }).unwrap();
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    let img = &r.image;
    let straight = |q: [f32; 4]| [q[0] / q[3], q[1] / q[3], q[2] / q[3]];
    // the A white; the face yellow, beside it
    let face_x = (20.0 + emoji[0].x) as usize;
    let yellow = (60..130).flat_map(|y| (face_x..face_x + 64).map(move |x| (x, y))).filter(|&(x, y)| {
        let q = img.px[y * 600 + x];
        q[3] > 0.95 && { let [r, g, b] = straight(q); r > 0.8 && g > 0.6 && b < 0.4 }
    }).count();
    assert!(yellow > 300, "a yellow face: {yellow} pixels");
    let a_white = (60..120).flat_map(|y| (20..60).map(move |x| (x, y))).any(|(x, y)| { let q = img.px[y * 600 + x]; q[3] > 0.99 && straight(q).iter().all(|v| *v > 0.99) });
    assert!(a_white, "the A in the text's colour");
    // to look at: Thai with emoji, skin tones, a flag, a keycap, a heart, turned by an animator
    if let Some(dir) = std::env::var_os("VEYMELO_FRAMES") {
        let (mut p, c, id) = text_scene(r##"{"text": "สวัสดี 😀👍🏽🇹🇭 1️⃣❤️✨", "font": ".ThonburiUI", "weight": 700, "size": 56, "fill": "#f4f1e8", "stroke": "#c0392b", "strokeWidth": 2}"##);
        let mut a = Group::instance(&veymelo_project::text::ANIMATOR, 0, "Animator 1");
        a.prop_mut("rotation").unwrap().set_value(Value::Number(-15.0)).unwrap();
        let mut r = Group::instance(&veymelo_project::text::RANGE, 0, "Range Selector 1");
        r.prop_mut("shape").unwrap().set_value(Value::Choice(3)).unwrap();
        a.group_mut("selectors").unwrap().children.push(Node::Group(r));
        layer(&mut p, c, id).props.group_mut("text/animators").unwrap().children.push(Node::Group(a));
        let r = render(&p, c, Tick::ZERO, &Options::default()).unwrap();
        assert!(r.notes.is_empty(), "{:?}", r.notes);
        std::fs::write(std::path::Path::new(&dir).join("emoji.png"), to_png(&r.image)).unwrap();
    }
}

/// A small font file whose "A" is a colour glyph (`COLR` and `CPAL`): its left half red from
/// the palette, its right half in the text's colour; its plain outline a 600-unit square.
fn colour_layer_font() -> Vec<u8> {
    let be16 = |v: &mut Vec<u8>, x: u16| v.extend(x.to_be_bytes());
    let be32 = |v: &mut Vec<u8>, x: u32| v.extend(x.to_be_bytes());
    let mut colr = Vec::new();
    be16(&mut colr, 0);
    be16(&mut colr, 1);
    be32(&mut colr, 14);
    be32(&mut colr, 20);
    be16(&mut colr, 2);
    for v in [1u16, 0, 2, 2, 0, 3, 0xFFFF] {
        be16(&mut colr, v);
    }
    font_file(vec![(*b"COLR", colr), (*b"CPAL", palette_table())])
}

/// A palette of red, green and blue (`CPAL`).
fn palette_table() -> Vec<u8> {
    let mut c = Vec::new();
    for v in [0u16, 3, 1, 3, 0, 14, 0] {
        c.extend(v.to_be_bytes());
    }
    c.extend([0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255]);
    c
}

/// A small font file, 1000 units an em, whose glyph 1 ("A") is a 600-unit square and glyphs 2
/// and 3 its left and right halves, with `more` tables.
fn font_file(more: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    let be16 = |v: &mut Vec<u8>, x: u16| v.extend(x.to_be_bytes());
    let be32 = |v: &mut Vec<u8>, x: u32| v.extend(x.to_be_bytes());
    // a rectangle glyph: one contour of four corners on the curve, coordinates as moves
    let rect = |x0: i16, y0: i16, x1: i16, y1: i16| {
        let mut g = Vec::new();
        for v in [1, x0, y0, x1, y1] {
            g.extend(v.to_be_bytes());
        }
        g.extend(3u16.to_be_bytes());
        g.extend(0u16.to_be_bytes());
        g.extend([1u8; 4]);
        for (a, b) in [(0, x0), (x0, x0), (x0, x1), (x1, x1)] {
            g.extend((b - a).to_be_bytes());
        }
        for (a, b) in [(0, y0), (y0, y1), (y1, y1), (y1, y0)] {
            g.extend((b - a).to_be_bytes());
        }
        g
    };
    let glyphs = [Vec::new(), rect(0, 0, 600, 600), rect(0, 0, 300, 600), rect(300, 0, 600, 600)];
    let (mut glyf, mut loca) = (Vec::new(), Vec::new());
    for g in &glyphs {
        be16(&mut loca, (glyf.len() / 2) as u16);
        glyf.extend(g);
        if glyf.len() % 2 == 1 {
            glyf.push(0);
        }
    }
    be16(&mut loca, (glyf.len() / 2) as u16);
    let mut head = vec![0u8; 54];
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut hhea = vec![0u8; 36];
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[34..36].copy_from_slice(&4u16.to_be_bytes());
    let mut maxp = vec![0, 0, 0x50, 0];
    be16(&mut maxp, 4);
    let mut hmtx = Vec::new();
    for _ in 0..4 {
        be16(&mut hmtx, 700);
        be16(&mut hmtx, 0);
    }
    // "A" to glyph 1
    let mut cmap = Vec::new();
    for v in [0u16, 1, 3, 1] {
        be16(&mut cmap, v);
    }
    be32(&mut cmap, 12);
    for v in [4u16, 32, 0, 4, 4, 1, 0, 0x41, 0xFFFF, 0, 0x41, 0xFFFF, 1u16.wrapping_sub(0x41), 1, 0, 0] {
        be16(&mut cmap, v);
    }
    let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![(*b"cmap", cmap), (*b"glyf", glyf), (*b"head", head), (*b"hhea", hhea), (*b"hmtx", hmtx), (*b"loca", loca), (*b"maxp", maxp)];
    tables.extend(more);
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    for v in [tables.len() as u16, 128, 3, 16] {
        be16(&mut out, v);
    }
    let mut at = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, t) in &tables {
        out.extend(tag);
        be32(&mut out, 0);
        be32(&mut out, at as u32);
        be32(&mut out, t.len() as u32);
        body.extend(t);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        at = 12 + 16 * tables.len() + body.len();
    }
    out.extend(body);
    out
}

#[test]
fn colour_layers_take_their_palette_colours() {
    let path = std::env::temp_dir().join(format!("veymelo-colr-{}.ttf", std::process::id()));
    std::fs::write(&path, colour_layer_font()).unwrap();
    let font = veymelo_text::resolve(&path.to_string_lossy(), 400.0, false).unwrap();
    assert!(font.has_color());
    assert_eq!(font.color_layers(font.glyph('A')), Some(vec![(2, Some([1.0, 0.0, 0.0, 1.0])), (3, None)]));
    assert_eq!(font.color_layers(0), None);
    // drawn at 100 px: the square 60 px across from (20, 60), red on the left, green on the right
    let (p, c, _) = text_scene(&format!(r##"{{"text": "A", "font": "{}", "size": 100, "fill": "#00ff00"}}"##, path.to_string_lossy()));
    let r = render(&p, c, Tick::ZERO, &Options { background: false, ..Options::default() }).unwrap();
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    let at = |x: usize, y: usize| r.image.px[y * 600 + x];
    assert_eq!(at(35, 90), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(at(65, 90), [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(at(90, 90)[3], 0.0);
}

/// A paint table `head` with `kid` after it, the three-byte offset at `at` in it set to where.
fn paint_with(mut head: Vec<u8>, at: usize, kid: Vec<u8>) -> Vec<u8> {
    let off = head.len() as u32;
    head[at..at + 3].copy_from_slice(&off.to_be_bytes()[1..]);
    head.extend(kid);
    head
}

/// A paint of `format` with a child (its offset first) and then `values`.
fn paint_of(format: u8, values: &[u16], kid: Vec<u8>) -> Vec<u8> {
    let mut p = vec![format, 0, 0, 0];
    for v in values {
        p.extend(v.to_be_bytes());
    }
    paint_with(p, 1, kid)
}

/// A colour line: how it carries on, and its stops (offset, palette colour, alpha, as 2.14).
fn colour_line(extend: u8, stops: &[(u16, u16, u16)]) -> Vec<u8> {
    let mut l = vec![extend];
    l.extend((stops.len() as u16).to_be_bytes());
    for (o, i, a) in stops {
        for v in [o, i, a] {
            l.extend(v.to_be_bytes());
        }
    }
    l
}

fn paint_glyph(g: u16, kid: Vec<u8>) -> Vec<u8> {
    paint_with(vec![10, 0, 0, 0, (g >> 8) as u8, g as u8], 1, kid)
}

/// A `COLR` version 1 table whose glyph 1 is `layers`, bottom first.
fn layered_paints(layers: Vec<Vec<u8>>) -> Vec<u8> {
    let be16 = |v: &mut Vec<u8>, x: u16| v.extend(x.to_be_bytes());
    let be32 = |v: &mut Vec<u8>, x: u32| v.extend(x.to_be_bytes());
    // the base glyph list (one record) and the layer list
    let mut bases = Vec::new();
    be32(&mut bases, 1);
    be16(&mut bases, 1);
    be32(&mut bases, 10);
    bases.extend([1, layers.len() as u8, 0, 0, 0, 0]);
    let mut list = Vec::new();
    be32(&mut list, layers.len() as u32);
    let mut at = 4 + 4 * layers.len();
    for l in &layers {
        be32(&mut list, at as u32);
        at += l.len();
    }
    list.extend(layers.concat());
    let mut colr = Vec::new();
    for v in [1u16, 0, 0, 0, 0, 0, 0] {
        be16(&mut colr, v);
    }
    be32(&mut colr, 34);
    be32(&mut colr, 34 + bases.len() as u32);
    for _ in 0..3 {
        be32(&mut colr, 0);
    }
    colr.extend(bases);
    colr.extend(list);
    colr
}

/// A `COLR` version 1 table: glyph 1 two layers, a linear gradient red to blue across glyph 2
/// (the left half) and the text's own colour half seen over glyph 3 (the right half).
fn paint_table() -> Vec<u8> {
    let linear = paint_of(4, &[0, 0, 300, 0, 0, 300], colour_line(0, &[(0, 0, 0x4000), (0x4000, 2, 0x4000)]));
    let solid = vec![2, 0xFF, 0xFF, 0x20, 0x00];
    layered_paints(vec![paint_glyph(2, linear), paint_glyph(3, solid)])
}

#[test]
fn colour_glyphs_of_paints_are_drawn_with_their_gradients() {
    let path = std::env::temp_dir().join(format!("veymelo-colr1-{}.ttf", std::process::id()));
    std::fs::write(&path, font_file(vec![(*b"COLR", paint_table()), (*b"CPAL", palette_table())])).unwrap();
    let (p, c, _) = text_scene(&format!(r##"{{"text": "A", "font": "{}", "size": 100, "fill": "#00ff00"}}"##, path.to_string_lossy()));
    let r = render(&p, c, Tick::ZERO, &Options { background: false, ..Options::default() }).unwrap();
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    let at = |x: usize, y: usize| r.image.px[y * 600 + x];
    // halfway across the gradient: red and blue mixed in linear light, half of each
    let mid = veymelo_color::linear_to_srgb(0.5) as f32;
    let m = at(35, 90);
    assert!((m[0] - mid).abs() < 0.02 && m[1] < 0.01 && (m[2] - mid).abs() < 0.02 && m[3] > 0.99, "{m:?}");
    // near its start mostly red, near its end mostly blue
    let (a, b) = (at(21, 90), at(48, 90));
    assert!(a[0] > 0.95 && a[2] < 0.4 && b[2] > 0.95 && b[0] < 0.4, "{a:?} {b:?}");
    // the right half the text's green, half seen
    let g = at(65, 90);
    assert!(g[0] < 0.01 && (g[1] - 0.5).abs() < 0.01 && (g[3] - 0.5).abs() < 0.01, "{g:?}");
    assert_eq!(at(90, 90)[3], 0.0);
    // to look at: rings reflected out from the middle, a colour wheel turned 30° in the left
    // half, and the text's colour laid over a gradient down the right half
    if let Some(dir) = std::env::var_os("VEYMELO_FRAMES") {
        let rings = paint_glyph(1, paint_of(6, &[300, 300, 0, 300, 300, 120], colour_line(2, &[(0, 0, 0x4000), (0x4000, 2, 0x4000)])));
        let wheel = paint_of(8, &[300, 300, 0xC000, 0x4000], colour_line(0, &[(0, 0, 0x4000), (0x1555, 1, 0x4000), (0x2AAA, 2, 0x4000), (0x4000, 0, 0x4000)]));
        let turned = paint_of(26, &[0x0AAB, 300, 300], paint_glyph(2, wheel));
        let down = paint_glyph(3, paint_of(4, &[0, 0, 0, 600, 600, 0], colour_line(0, &[(0, 2, 0x4000), (0x4000, 0, 0x4000)])));
        let over = paint_glyph(3, vec![2, 0xFF, 0xFF, 0x40, 0x00]);
        let mixed = paint_with(paint_with(vec![32, 0, 0, 0, 14, 0, 0, 0], 1, over), 5, down);
        let path = std::env::temp_dir().join(format!("veymelo-colr1-show-{}.ttf", std::process::id()));
        std::fs::write(&path, font_file(vec![(*b"COLR", layered_paints(vec![rings, turned, mixed])), (*b"CPAL", palette_table())])).unwrap();
        let (mut p, c, id) = text_scene(&format!(r##"{{"text": "AAA", "font": "{}", "size": 160, "fill": "#ffd23f"}}"##, path.to_string_lossy()));
        set(layer(&mut p, c, id), "transform/position", Value::Vec3([20.0, 150.0, 0.0]));
        set(layer(&mut p, c, id), "transform/rotation", Value::Number(-8.0));
        let r = render(&p, c, Tick::ZERO, &Options::default()).unwrap();
        assert!(r.notes.is_empty(), "{:?}", r.notes);
        std::fs::write(std::path::Path::new(&dir).join("colr1.png"), to_png(&r.image)).unwrap();
    }
}

#[cfg(target_os = "macos")]
#[test]
fn bitmap_colour_glyphs_are_drawn_from_their_pngs() {
    let be16 = |v: &mut Vec<u8>, x: u16| v.extend(x.to_be_bytes());
    let be32 = |v: &mut Vec<u8>, x: u32| v.extend(x.to_be_bytes());
    // a 12 × 16 picture of one blue, 100 pixels an em, its top left 10 right of the origin and
    // 50 up
    let mut img = Image::new(12, 16);
    img.px.iter_mut().for_each(|q| *q = [0.2, 0.4, 0.8, 1.0]);
    let png = to_png(&img);
    let mut cbdt = vec![0, 3, 0, 0, 16, 12, 10, 50, 14];
    be32(&mut cbdt, png.len() as u32);
    cbdt.extend(&png);
    let mut cblc = vec![0, 3, 0, 0];
    be32(&mut cblc, 1);
    for v in [56u32, 0, 1, 0] {
        be32(&mut cblc, v);
    }
    cblc.extend([0u8; 24]);
    for v in [1u16, 1] {
        be16(&mut cblc, v);
    }
    cblc.extend([100, 100, 32, 1]);
    for v in [1u16, 1] {
        be16(&mut cblc, v);
    }
    be32(&mut cblc, 8);
    for v in [1u16, 17] {
        be16(&mut cblc, v);
    }
    for v in [4u32, 0, cbdt.len() as u32 - 4] {
        be32(&mut cblc, v);
    }
    let path = std::env::temp_dir().join(format!("veymelo-cbdt-{}.ttf", std::process::id()));
    std::fs::write(&path, font_file(vec![(*b"CBDT", cbdt), (*b"CBLC", cblc)])).unwrap();
    let font = veymelo_text::resolve(&path.to_string_lossy(), 400.0, false).unwrap();
    assert!(font.has_color() && font.picture(1, 100.0).is_some());
    let (p, c, _) = text_scene(&format!(r##"{{"text": "A", "font": "{}", "size": 100, "fill": "#ffffff"}}"##, path.to_string_lossy()));
    let r = render(&p, c, Tick::ZERO, &Options { background: false, ..Options::default() }).unwrap();
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    let at = |x: usize, y: usize| r.image.px[y * 600 + x];
    // the picture over x 30 to 42 and y 70 to 86 (the baseline at 120)
    let q = at(35, 78);
    assert!((q[0] - 0.2).abs() < 0.01 && (q[1] - 0.4).abs() < 0.01 && (q[2] - 0.8).abs() < 0.01 && q[3] > 0.99, "{q:?}");
    assert_eq!(at(45, 78)[3], 0.0);
    assert_eq!(at(35, 90)[3], 0.0);
}

/// The scripts of India, Khmer and Myanmar set by our own shaping, each in the font that has it.
#[cfg(target_os = "macos")]
#[test]
fn indic_text_is_shaped() {
    let lines = ["हिन्दी: नमस्ते, क्षत्रिय, धर्म", "বাংলা: রবীন্দ্রনাথ ঠাকুর", "தமிழ்: வணக்கம், கொ", "తెలుగు: శ్రీ, స్త్రీ", "മലയാളം: ക്രി, ന്റെ", "ខ្មែរ: ប្រទេសកម្ពុជា", "မြန်မာ: မင်္ဂလာပါ"];
    let text = lines.join("\\r");
    let (mut p, c, id) = text_scene(&format!(r##"{{"text": "{text}", "font": "Helvetica", "size": 40, "leading": 60, "fill": "#f4f1e8"}}"##));
    let comp = p.comp_mut(c).unwrap();
    comp.width = 900;
    comp.height = 460;
    set(layer(&mut p, c, id), "transform/position", Value::Vec3([30.0, 60.0, 0.0]));
    let r = render(&p, c, Tick::ZERO, &Options::default()).unwrap();
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    // the Hindi conjunct क्ष is one glyph from the Devanagari font
    let lay = veymelo_text::lay_out(&veymelo_keyframe::TextDoc { font: "Helvetica".into(), size: 40.0, ..veymelo_keyframe::TextDoc::of("क्ष") });
    assert_eq!(lay.glyphs.len(), 1, "{:?}", lay.glyphs.iter().map(|g| g.glyph).collect::<Vec<_>>());
    if let Some(dir) = std::env::var_os("VEYMELO_FRAMES") {
        std::fs::write(std::path::Path::new(&dir).join("indic.png"), to_png(&r.image)).unwrap();
    }
}

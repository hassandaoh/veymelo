//! What every effect makes, kept to hold it while its code is written anew: each effect run on one
//! picture with its defaults, then with each setting moved on its own (every option of a choice,
//! a switch turned, a number moved a step, a colour or point changed), at the start and a little
//! later. Run by hand:
//!
//!   VEYMELO_GOLDENS=/tmp/g cargo test -p veymelo-effects --release goldens -- --ignored
//!
//! The first run writes what each effect makes (`/tmp/g/<effect>.bin`); later runs compare with
//! it and list what changed, with a picture of each change (before, after, and the difference
//! made bright) in `/tmp/g/changed/`. `VEYMELO_GOLDENS_ONLY=word` runs only the effects whose id
//! holds the word; `VEYMELO_GOLDENS_WRITE=1` writes anew. The effects run on the processor:
//! `VEYMELO_GOLDENS_GPU=1` (with `VEYMELO_GPU_FROM=0`, so even these small pictures go) runs them
//! with the GPU, to see that what it filters is the processor's (some results move by more than
//! a float's last bits where an effect makes much of a small change: a threshold, a seam).

use std::path::{Path, PathBuf};

use veymelo_color::Rgba;
use veymelo_keyframe::{Kind, Value};
use veymelo_project::Group;
use veymelo_raster::Image;
use veymelo_time::Tick;

use crate::testing::{buf, instance};
use crate::{Ctx, Spec};

const W: usize = 64;
const H: usize = 48;

/// The picture every effect is run on: a colour ramp, a bright disc, a dark square, a half
/// clear band and a clear corner, so edges, colours, darks, lights and alpha are all there.
fn picture() -> Image {
    let mut img = Image::new(W, H);
    for y in 0..H {
        for x in 0..W {
            let (fx, fy) = (x as f32 / (W - 1) as f32, y as f32 / (H - 1) as f32);
            let mut p = [0.15 + 0.7 * fx, 0.2 + 0.6 * fy, 0.8 - 0.5 * fx, 1.0];
            let (dx, dy) = (x as f32 - 40.0, y as f32 - 18.0);
            if dx * dx + dy * dy < 100.0 {
                p = [0.95, 0.9, 0.75, 1.0];
            }
            if (10..22).contains(&x) && (26..38).contains(&y) {
                p = [0.05, 0.07, 0.1, 1.0];
            }
            if (40..48).contains(&y) {
                p[3] = 0.5;
                p = [p[0] * 0.5, p[1] * 0.5, p[2] * 0.5, 0.5];
            }
            if x < 6 && y < 6 {
                p = [0.0; 4];
            }
            img.px[y * W + x] = p;
        }
    }
    img
}

/// One run of an effect: its name, and the settings moved from the defaults.
struct Case {
    name: String,
    values: Vec<(String, Value)>,
    time: f64,
}

/// The runs for `spec`.
fn cases(spec: &Spec) -> Vec<Case> {
    let g = instance(spec, &[]);
    let mut out = vec![Case { name: "default".into(), values: vec![], time: 0.0 }, Case { name: "later".into(), values: vec![], time: 1.25 }];
    let mut props = vec![];
    g.walk(&mut |path, p| props.push((path.to_string(), p.def, p.value.clone())));
    for (path, def, v) in props {
        let mut alts = vec![];
        match (&v, def.kind) {
            (Value::Number(x), Kind::Number) => {
                let span = def.max - def.min;
                let step = if span.is_finite() && span <= 400.0 { 0.2 * span } else { (x.abs() * 0.5).max(1.0) };
                let mut y = (x + step).clamp(def.min, def.max);
                if (y - x).abs() < 1e-9 {
                    y = (x - step).clamp(def.min, def.max);
                }
                if (y - x).abs() > 1e-9 {
                    alts.push(Value::Number(y));
                }
            }
            (Value::Bool(b), _) => alts.push(Value::Bool(!b)),
            (Value::Choice(i), Kind::Choice(options)) => {
                for k in 0..options.len() as u32 {
                    if k != *i {
                        alts.push(Value::Choice(k));
                    }
                }
            }
            (Value::Color(_), _) => alts.push(Value::Color(Rgba { r: 0.2, g: 0.6, b: 0.9, a: 1.0 })),
            (Value::Vec2(p), _) => alts.push(Value::Vec2([p[0] + 7.0, p[1] - 5.0])),
            (Value::Vec3(p), _) => alts.push(Value::Vec3([p[0] + 7.0, p[1] - 5.0, p[2] + 3.0])),
            _ => {}
        }
        for a in alts {
            if def.check(&a).is_err() {
                continue;
            }
            let label = match &a {
                Value::Choice(k) => match def.kind {
                    Kind::Choice(options) => options[*k as usize].to_string(),
                    _ => k.to_string(),
                },
                Value::Number(y) => format!("{y}"),
                Value::Bool(b) => b.to_string(),
                _ => "moved".into(),
            };
            out.push(Case { name: format!("{path}={label}"), values: vec![(path.clone(), a)], time: 0.0 });
        }
    }
    out
}

/// What `spec` makes in `case`: where its picture lies (origin and scale) and its pixels.
fn run(spec: &Spec, case: &Case) -> (f64, f64, f64, Image) {
    let mut g: Group = instance(spec, &[]);
    let bounds = veymelo_geom::Rect::from_size(W as f64, H as f64);
    spec.start(&mut g, bounds);
    for (path, v) in &case.values {
        if let Ok(p) = g.prop_mut(path) {
            let _ = p.set_value(v.clone());
        }
    }
    let t = Tick::from_seconds(case.time);
    let read = |p: &veymelo_project::Property| p.value_at(t);
    let ctx = Ctx::new(&g, &read, t, bounds, 7);
    let b = (spec.run)(&ctx, buf(picture()));
    (b.origin.x, b.origin.y, b.scale, b.img)
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_f64(out: &mut Vec<u8>, v: f64) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Every case's result, one after another.
fn write(results: &[(String, (f64, f64, f64, Image))]) -> Vec<u8> {
    let mut out = vec![];
    put_u32(&mut out, results.len() as u32);
    for (name, (x, y, s, img)) in results {
        put_u32(&mut out, name.len() as u32);
        out.extend_from_slice(name.as_bytes());
        put_f64(&mut out, *x);
        put_f64(&mut out, *y);
        put_f64(&mut out, *s);
        put_u32(&mut out, img.width as u32);
        put_u32(&mut out, img.height as u32);
        for p in &img.px {
            for c in p {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
    }
    veymelo_zip::deflate(&out)
}

fn read(bytes: &[u8]) -> Vec<(String, (f64, f64, f64, Image))> {
    let data = veymelo_zip::inflate(bytes).expect("a goldens file");
    let mut at = 0;
    let u32_ = |at: &mut usize| {
        let v = u32::from_le_bytes(data[*at..*at + 4].try_into().unwrap());
        *at += 4;
        v
    };
    let n = u32_(&mut at);
    let mut out = vec![];
    for _ in 0..n {
        let len = u32_(&mut at) as usize;
        let name = String::from_utf8(data[at..at + len].to_vec()).unwrap();
        at += len;
        let mut f = [0f64; 3];
        for v in &mut f {
            *v = f64::from_le_bytes(data[at..at + 8].try_into().unwrap());
            at += 8;
        }
        let (w, h) = (u32_(&mut at) as usize, u32_(&mut at) as usize);
        let mut img = Image::new(w, h);
        for p in &mut img.px {
            for c in p.iter_mut() {
                *c = f32::from_le_bytes(data[at..at + 4].try_into().unwrap());
                at += 4;
            }
        }
        out.push((name, (f[0], f[1], f[2], img)));
    }
    out
}

/// How far apart two results are: the largest and the mean difference of a channel (alpha
/// included, colours as they show over black), or none when they do not even lie alike.
fn apart(a: &(f64, f64, f64, Image), b: &(f64, f64, f64, Image)) -> Option<(f32, f32)> {
    if a.3.width != b.3.width || a.3.height != b.3.height || (a.0 - b.0).abs() > 1e-6 || (a.1 - b.1).abs() > 1e-6 || (a.2 - b.2).abs() > 1e-9 {
        return None;
    }
    let (mut most, mut sum) = (0f32, 0f64);
    for (p, q) in a.3.px.iter().zip(&b.3.px) {
        for c in 0..4 {
            let d = if p[c].is_nan() != q[c].is_nan() { f32::INFINITY } else if p[c].is_nan() { 0.0 } else { (p[c] - q[c]).abs() };
            most = most.max(d);
            sum += d as f64;
        }
    }
    Some((most, (sum / (a.3.px.len() * 4).max(1) as f64) as f32))
}

/// A PNG of `img` (straight colours, clamped), for looking at.
fn png(img: &Image) -> Vec<u8> {
    let mut raw = vec![];
    for y in 0..img.height {
        raw.push(0u8);
        for x in 0..img.width {
            let p = img.px[y * img.width + x];
            let a = p[3].clamp(0.0, 1.0);
            let un = |v: f32| if a > 0.0 { (v / a).clamp(0.0, 1.0) } else { 0.0 };
            for v in [un(p[0]), un(p[1]), un(p[2]), a] {
                raw.push((v * 255.0).round() as u8);
            }
        }
    }
    let chunk = |out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&veymelo_zip::crc32(&c).to_be_bytes());
    };
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut head = vec![];
    head.extend_from_slice(&(img.width as u32).to_be_bytes());
    head.extend_from_slice(&(img.height as u32).to_be_bytes());
    head.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &head);
    chunk(&mut out, b"IDAT", &veymelo_zip::zlib(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Before, after and the difference (times 20) side by side, each on a checkerboard.
fn side_by_side(a: &Image, b: &Image) -> Image {
    let (w, h) = (a.width.max(b.width), a.height.max(b.height));
    let mut out = Image::new(w * 3 + 8, h);
    let board = |x: usize, y: usize| if (x / 4 + y / 4) % 2 == 0 { 0.8 } else { 0.6 };
    for y in 0..h {
        for x in 0..out.width {
            out.px[y * out.width + x] = [board(x, y), board(x, y), board(x, y), 1.0];
        }
    }
    let over = |out: &mut Image, img: &Image, x0: usize| {
        for y in 0..img.height {
            for x in 0..img.width {
                let p = img.px[y * img.width + x];
                let a = p[3].clamp(0.0, 1.0);
                let o = &mut out.px[y * out.width + x0 + x];
                for c in 0..3 {
                    o[c] = p[c].clamp(0.0, 1.0) + o[c] * (1.0 - a);
                }
            }
        }
    };
    over(&mut out, a, 0);
    over(&mut out, b, w + 4);
    for y in 0..a.height.min(b.height) {
        for x in 0..a.width.min(b.width) {
            let (p, q) = (a.px[y * a.width + x], b.px[y * b.width + x]);
            let d = |c: usize| ((p[c] - q[c]).abs() * 20.0).min(1.0);
            out.px[y * out.width + 2 * (w + 4) + x] = [d(0).max(d(3)), d(1).max(d(3)), d(2).max(d(3)), 1.0];
        }
    }
    out
}

/// A picture `scale` times bigger, pixel for pixel, to look at.
fn bigger(img: &Image, scale: usize) -> Image {
    let mut out = Image::new(img.width * scale, img.height * scale);
    for y in 0..out.height {
        for x in 0..out.width {
            out.px[y * out.width + x] = img.px[(y / scale) * img.width + x / scale];
        }
    }
    out
}

fn file(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{}.bin", id.replace(['/', ' '], "_")))
}

#[test]
#[ignore]
fn goldens() {
    let Some(dir) = std::env::var_os("VEYMELO_GOLDENS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(dir.join("changed")).unwrap();
    let only = std::env::var("VEYMELO_GOLDENS_ONLY").ok();
    let rewrite = std::env::var_os("VEYMELO_GOLDENS_WRITE").is_some();
    let (mut written, mut same, mut changed, mut runs) = (0, 0, vec![], 0);
    let gpu = std::env::var_os("VEYMELO_GOLDENS_GPU").is_some();
    for spec in crate::all() {
        if only.as_deref().is_some_and(|w| !spec.def.id.contains(w)) {
            continue;
        }
        let started = std::time::Instant::now();
        let results: Vec<_> = veymelo_raster::gpu::with_mode(!gpu, || cases(spec).into_iter().map(|c| (c.name.clone(), run(spec, &c))).collect());
        runs += results.len();
        let took = started.elapsed().as_secs_f64();
        if took > 20.0 {
            eprintln!("slow: {} took {took:.1} s for {} runs", spec.def.id, results.len());
        }
        let path = file(&dir, spec.def.id);
        if rewrite || !path.exists() {
            std::fs::write(&path, write(&results)).unwrap();
            written += 1;
            continue;
        }
        let before = read(&std::fs::read(&path).unwrap());
        for (name, now) in &results {
            let Some((_, was)) = before.iter().find(|(n, _)| n == name) else {
                changed.push(format!("{} {name}: new", spec.def.id));
                continue;
            };
            match apart(was, now) {
                Some((most, _)) if most <= 1e-5 => same += 1,
                d => {
                    let what = d.map_or("lies elsewhere or is another size".to_string(), |(most, mean)| format!("most {most:.5}, mean {mean:.6}"));
                    changed.push(format!("{} {name}: {what}", spec.def.id));
                    let picture = bigger(&side_by_side(&was.3, &now.3), 4);
                    let name = format!("{}__{}", spec.def.id, name).replace(['/', ' ', '='], "_");
                    std::fs::write(dir.join("changed").join(format!("{name}.png")), png(&picture)).unwrap();
                }
            }
        }
        for (name, _) in &before {
            if !results.iter().any(|(n, _)| n == name) {
                changed.push(format!("{} {name}: gone", spec.def.id));
            }
        }
    }
    eprintln!("{runs} runs; {written} effects written, {same} the same, {} changed", changed.len());
    for c in &changed {
        eprintln!("  {c}");
    }
    assert!(changed.is_empty(), "{} runs changed (see {})", changed.len(), dir.join("changed").display());
}

/// How long each effect takes with its defaults on a 1080 × 1920 picture, the slowest first
/// (run by hand: cargo test -p veymelo-effects --release effects_take -- --ignored --nocapture;
/// VEYMELO_GPU=0 for the processor alone).
#[test]
#[ignore]
fn effects_take() {
    let (w, h) = (1080, 1920);
    let small = picture();
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            img.px[y * w + x] = small.px[(y * H / h) * W + x * W / w];
        }
    }
    let bounds = veymelo_geom::Rect::from_size(w as f64, h as f64);
    let mut took = vec![];
    for spec in crate::all() {
        let mut g: Group = instance(spec, &[]);
        spec.start(&mut g, bounds);
        let t = Tick::from_seconds(0.5);
        let read = |p: &veymelo_project::Property| p.value_at(t);
        let ctx = Ctx::new(&g, &read, t, bounds, 7);
        let started = std::time::Instant::now();
        std::hint::black_box((spec.run)(&ctx, buf(img.clone())));
        took.push((started.elapsed().as_secs_f64() * 1000.0, spec.def.id));
    }
    took.sort_by(|a, b| b.0.total_cmp(&a.0));
    let all: f64 = took.iter().map(|t| t.0).sum();
    eprintln!("{} effects, {all:.0} ms in all", took.len());
    for (ms, id) in took.iter().take(60) {
        eprintln!("{ms:9.1} ms  {id}");
    }
}

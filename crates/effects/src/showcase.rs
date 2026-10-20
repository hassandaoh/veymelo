//! Effects drawn at a size to look at, for comparing a change by eye. Run by hand:
//!   VEYMELO_SHOW=/tmp/s cargo test -p veymelo-effects --release showcase -- --ignored
//! Each picture is written to `/tmp/s/<effect>__<setting>.raw` (width, height, then RGBA as
//! 32-bit floats, premultiplied).

use veymelo_keyframe::Value;
use veymelo_raster::Image;

use crate::testing::run;

const W: usize = 480;
const H: usize = 270;

/// A scene to look at: sky over hills, a low sun, a grey ramp along the bottom, and patches of
/// skin, foliage, a red and a blue.
fn scene() -> Image {
    let mut img = Image::new(W, H);
    for y in 0..H {
        for x in 0..W {
            let (fx, fy) = (x as f32 / W as f32, y as f32 / H as f32);
            let hill = 0.62 + 0.06 * (fx * 7.0).sin() + 0.04 * (fx * 13.0 + 1.0).sin();
            let mut p = if fy < hill {
                let t = fy / hill;
                [0.25 + 0.5 * t, 0.45 + 0.35 * t, 0.85 - 0.1 * t]
            } else {
                let t = (fy - hill) / (1.0 - hill);
                [0.18 + 0.1 * t, 0.32 - 0.1 * t, 0.12]
            };
            let (dx, dy) = (fx - 0.72, fy - 0.42);
            if dx * dx * 3.2 + dy * dy < 0.004 {
                p = [1.0, 0.92, 0.7];
            }
            if fy > 0.9 {
                let g = (fx * 8.0).floor() / 7.0;
                p = [g, g, g];
            }
            let patch = |x0: f32, c: [f32; 3], p: &mut [f32; 3]| {
                if fx > x0 && fx < x0 + 0.07 && fy > 0.74 && fy < 0.86 {
                    *p = c;
                }
            };
            patch(0.08, [0.85, 0.62, 0.5], &mut p);
            patch(0.18, [0.2, 0.5, 0.15], &mut p);
            patch(0.28, [0.8, 0.12, 0.1], &mut p);
            patch(0.38, [0.1, 0.2, 0.75], &mut p);
            img.px[y * W + x] = [p[0], p[1], p[2], 1.0];
        }
    }
    img
}

fn write(dir: &std::path::Path, name: &str, img: &Image) {
    let mut out = vec![];
    out.extend_from_slice(&(img.width as u32).to_le_bytes());
    out.extend_from_slice(&(img.height as u32).to_le_bytes());
    for p in &img.px {
        for c in p {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    std::fs::write(dir.join(format!("{name}.raw")), out).unwrap();
}

#[test]
#[ignore]
fn showcase() {
    let Some(dir) = std::env::var_os("VEYMELO_SHOW").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "source", &scene());
    let c = |i: u32| Value::Choice(i);
    let cases: Vec<(&str, &str, Vec<(&str, Value)>)> = vec![
        ("colorGrade", "warmFilm", vec![("look", c(2))]),
        ("colorGrade", "coolNight", vec![("look", c(3))]),
        ("colorGrade", "bleachBypass", vec![("look", c(4))]),
        ("colorGrade", "tealOrange", vec![("look", c(5))]),
        ("colorGrade", "fadedMatte", vec![("look", c(6))]),
        ("lensFlare", "zoom", vec![("flareCenter", Value::Vec2([140.0, 70.0]))]),
        ("lensFlare", "prime35", vec![("flareCenter", Value::Vec2([140.0, 70.0])), ("lensType", c(1))]),
        ("lensFlare", "prime105", vec![("flareCenter", Value::Vec2([140.0, 70.0])), ("lensType", c(2))]),
        ("addGrain", "fine35mm", vec![("preset", c(1)), ("viewingMode", c(2))]),
        ("addGrain", "coarse16mm", vec![("preset", c(2)), ("viewingMode", c(2))]),
        ("addGrain", "vintage8mm", vec![("preset", c(3)), ("viewingMode", c(2))]),
        ("addGrain", "monochromeHighSpeed", vec![("preset", c(5)), ("viewingMode", c(2))]),
        ("brightnessContrast", "bright60", vec![("brightness", Value::Number(60.0))]),
        ("brightnessContrast", "dark-60contrast40", vec![("brightness", Value::Number(-60.0)), ("contrast", Value::Number(40.0))]),
    ];
    for (id, name, values) in cases {
        let Some(spec) = crate::find(id) else { continue };
        let values: Vec<(&str, Value)> = values.into_iter().collect();
        let out = run(spec, &values, scene());
        write(&dir, &format!("{id}__{name}"), &out.img);
    }
}

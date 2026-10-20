use super::*;

fn near_m(a: &M3, b: &M3, tol: f64) -> bool {
    (0..3).all(|i| (0..3).all(|j| (a[i][j] - b[i][j]).abs() < tol))
}

fn named(n: &str) -> &'static Space {
    find(n).unwrap_or_else(|| panic!("{n}"))
}

#[test]
fn matrices() {
    let m = rgb_to_xyz(REC709, D65);
    assert!(near_m(&m, &[[0.4124, 0.3576, 0.1805], [0.2126, 0.7152, 0.0722], [0.0193, 0.1192, 0.9505]], 2e-4), "{m:?}");
    // ACEScg to ACES2065-1 (one white: no adapting)
    let m = matrix(named("ACEScg"), named("ACES2065-1"));
    assert!(near_m(&m, &[[0.695_452_2, 0.140_678_7, 0.163_869_1], [0.044_794_6, 0.859_671_1, 0.095_534_3], [-0.005_525_9, 0.004_025_2, 1.001_500_7]], 2e-6), "{m:?}");
    // linear sRGB to ACEScg, Bradford from D65 to the ACES white
    let m = matrix(named("Linear Rec.709 (sRGB)"), named("ACEScg"));
    assert!(near_m(&m, &[[0.613_097, 0.339_523, 0.047_379], [0.070_194, 0.916_356, 0.013_453], [0.020_616, 0.109_570, 0.869_815]], 2e-5), "{m:?}");
    // white stays white, through any space and back
    for s in SPACES {
        let w = apply(&matrix(named("Linear Rec.709 (sRGB)"), s), [1.0; 3]);
        let back = apply(&matrix(s, named("Linear Rec.709 (sRGB)")), w);
        assert!(back.iter().all(|v| (v - 1.0).abs() < 1e-9), "{}: {back:?}", s.name);
    }
    assert!(near_m(&mul(&invert(&m), &m), &IDENTITY, 1e-12));
}

#[test]
fn grey_on_each_curve() {
    let grey = [
        (Curve::SLog3, 420.0 / 1023.0),
        (Curve::LogC3, 0.391_007),
        (Curve::LogC4, 0.278_396),
        (Curve::VLog, 0.423_311),
        (Curve::AcesCct, 0.413_588),
        (Curve::Log3G10, 1.0 / 3.0),
        (Curve::CanonLog3, 0.343_4),
        (Curve::BmdFilm5, 0.383_56),
        (Curve::DLog, 0.398_76),
    ];
    for (c, want) in grey {
        assert!((c.encode(0.18) - want).abs() < 2e-4, "{c:?}: {}", c.encode(0.18));
    }
    assert!((Curve::Pq.encode(1.0) - 0.508_078).abs() < 1e-5, "100 cd/m²");
    assert!((Curve::Pq.encode(100.0) - 1.0).abs() < 1e-9, "10,000 cd/m²");
    assert!((Curve::Hlg.encode(1.0 / 12.0) - 0.5).abs() < 1e-9 && (Curve::Hlg.encode(1.0) - 1.0).abs() < 1e-5);
    assert!((Curve::Srgb.encode(0.5) - 0.735_357).abs() < 1e-5);
}

#[test]
fn curves_undo_themselves() {
    let curves = [
        Curve::Linear,
        Curve::Srgb,
        Curve::Gamma(2.4),
        Curve::Rec709,
        Curve::Romm,
        Curve::AcesCct,
        Curve::AcesCc,
        Curve::Pq,
        Curve::Hlg,
        Curve::LogC3,
        Curve::LogC4,
        Curve::SLog3,
        Curve::VLog,
        Curve::CanonLog2,
        Curve::CanonLog3,
        Curve::Log3G10,
        Curve::BmdFilm5,
        Curve::DLog,
    ];
    for c in curves {
        for i in 0..200 {
            let x = 1e-4 + i as f64 * 0.04;
            let x = if c == Curve::Hlg { x.min(1.0) } else { x };
            let back = c.decode(c.encode(x));
            assert!((back - x).abs() < 1e-7 * x.max(1.0), "{c:?} {x}: {back}");
        }
        // smooth through the joins: no step bigger than its neighbours'
        let steps: Vec<f64> = (0..400).map(|i| c.encode(i as f64 * 0.0005 + 0.0005) - c.encode(i as f64 * 0.0005)).collect();
        assert!(steps.iter().all(|s| *s >= 0.0), "{c:?} rises");
    }
}

#[test]
fn compresses_only_past_the_edge() {
    let inside = [0.5, 0.3, 0.2];
    assert_eq!(compress_gamut(inside), inside);
    let past = compress_gamut([0.8, 0.4, -0.3]);
    assert!(past[2] > -0.3 && past[2] < 0.4 && past[0] == 0.8, "{past:?}");
    // the limit lands on the edge
    let at = compress_gamut([1.0, 1.0, 1.0 - 1.312]);
    assert!(at[2].abs() < 1e-6, "{at:?}");
}

#[test]
fn names() {
    assert_eq!(find("srgb").map(|s| s.name), Some("sRGB - Texture"));
    assert_eq!(find("ACES cg").map(|s| s.name), Some("ACEScg"));
    assert!(find("nothing").is_none());
    assert!(find("Rec.2100-PQ - Display").is_some_and(|s| s.display));
}

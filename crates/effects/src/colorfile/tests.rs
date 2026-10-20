use super::*;

fn near3(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < tol)
}

const T: Between = Between::Tetrahedral;

#[test]
fn cdl_both_ways() {
    let cdl = Cdl { slope: [1.2, 1.0, 0.9], offset: [-0.05, 0.02, 0.0], power: [1.1, 0.9, 1.0], sat: 0.8 };
    // (none of these leave 0 to 1 on the way, where clamping would lose them)
    for c in [[0.2, 0.4, 0.6], [0.7, 0.1, 0.5], [0.5, 0.5, 0.5]] {
        for clamp in [true, false] {
            let back = cdl.undo(cdl.apply(c, clamp), clamp);
            assert!((0..3).all(|i| (back[i] - c[i]).abs() < 1e-9), "{c:?} {back:?}");
        }
    }
    // grey keeps grey through saturation alone
    let sat = Cdl { sat: 0.0, ..Cdl::default() };
    let g = sat.apply([0.2, 0.5, 0.8], true);
    assert!((g[0] - g[1]).abs() < 1e-12 && (g[1] - g[2]).abs() < 1e-12);
}

#[test]
fn moncurve_is_srgb_above_its_foot() {
    for v in [0.1, 0.5, 0.9] {
        assert!((moncurve(v, 2.4, 0.055, true, false) - Curve::Srgb.decode(v)).abs() < 1e-12);
    }
    for v in [-0.3, 0.001, 0.02, 0.3, 1.5] {
        let back = moncurve(moncurve(v, 2.4, 0.055, true, true), 2.4, 0.055, false, true);
        assert!((back - v).abs() < 1e-12, "{v}");
    }
}

#[test]
fn reads_xml() {
    let x = parse_xml("<?xml version=\"1.0\"?><!-- c --><A k='1' j=\"a&amp;b\"><B>hi</B><C x=\"2\"/>text<![CDATA[<raw>]]></A>").unwrap();
    assert_eq!(x.name, "A");
    assert_eq!(x.attr("j"), Some("a&b"));
    assert_eq!(x.child("B").map(|b| b.text.as_str()), Some("hi"));
    assert_eq!(x.child("C").and_then(|c| c.attr("x")), Some("2"));
    assert!(x.text.contains("text<raw>"));
    assert!(parse_xml("<A><B></A>").is_none(), "tags must close in order");
}

const CCC: &str = r#"<ColorCorrectionCollection xmlns="urn:ASC:CDL:v1.2">
  <ColorCorrection id="warm"><SOPNode><Slope>1.1 1.0 0.9</Slope><Offset>0 0 0</Offset><Power>1 1 1</Power></SOPNode><SatNode><Saturation>1.0</Saturation></SatNode></ColorCorrection>
  <ColorCorrection id="flat"><SOPNode><Slope>1 1 1</Slope><Offset>0 0 0</Offset><Power>1 1 1</Power></SOPNode><SatNode><Saturation>0</Saturation></SatNode></ColorCorrection>
</ColorCorrectionCollection>"#;

#[test]
fn cdl_files() {
    let list = parse_cdl(CCC).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].0, "warm");
    let warm = parse(CCC, "ccc", "").unwrap();
    assert!(near3(warm.run([0.5; 3], T, false), [0.55, 0.5, 0.45], 1e-6), "the first when none is asked for");
    let flat = parse(CCC, "ccc", "flat").unwrap();
    let g = flat.run([0.2, 0.5, 0.8], T, false);
    assert!((g[0] - g[2]).abs() < 1e-6, "{g:?}");
}

#[test]
fn imageworks_files() {
    let m = parse_spimtx("2 0 0 0\n0 1 0 32767.5\n0 0 1 0\n").unwrap();
    assert!(near3(m.run([0.25, 0.25, 0.25], T, false), [0.5, 0.75, 0.25], 1e-6));
    assert!(near3(m.run([0.5, 0.75, 0.25], T, true), [0.25; 3], 1e-6), "and back");
    let one = parse_spi1d("Version 1\nFrom 0.0 1.0\nLength 3\nComponents 1\n{\n 0.0\n 0.25\n 1.0\n}\n").unwrap();
    assert!(near3(one.run([0.5; 3], T, false), [0.25; 3], 1e-6));
    assert!(near3(one.run([0.25; 3], T, true), [0.5; 3], 1e-6));
    // a lattice that swaps red and blue
    let mut s = String::from("SPILUT 1.0\n3 3\n2 2 2\n");
    for r in 0..2 {
        for g in 0..2 {
            for b in 0..2 {
                s += &format!("{r} {g} {b} {b} {g} {r}\n");
            }
        }
    }
    let swap = parse_spi3d(&s).unwrap();
    assert!(near3(swap.run([0.8, 0.5, 0.1], Between::Trilinear, false), [0.1, 0.5, 0.8], 1e-5));
}

#[test]
fn lustre_and_cinespace() {
    // 10-bit, blue fastest: the swap of red and blue again
    let mut s = String::from("0 1023\n");
    for r in 0..2 {
        for g in 0..2 {
            for b in 0..2 {
                s += &format!("{} {} {}\n", b * 1023, g * 1023, r * 1023);
            }
        }
    }
    let swap = parse_3dl(&s).unwrap();
    assert!(near3(swap.run([0.8, 0.5, 0.1], Between::Trilinear, false), [0.1, 0.5, 0.8], 1e-5));
    let csp = "CSPLUTV100\n3D\n\n2\n0 1\n0 1\n2\n0 1\n0 1\n3\n0 0.5 1\n0 0.25 1\n\n2 2 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
    let t = parse_csp(csp).unwrap();
    // blue through its pre-LUT curve (0.5 to 0.25), the lattice as it is (red fastest)
    assert!(near3(t.run([0.3, 0.6, 0.5], Between::Trilinear, false), [0.3, 0.6, 0.25], 1e-5));
    assert!(near3(t.run([0.3, 0.6, 0.25], Between::Trilinear, true), [0.3, 0.6, 0.5], 1e-4), "and back");
}

const CLF: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ProcessList id="test" compCLFversion="3.0">
  <Description>a test</Description>
  <Matrix inBitDepth="32f" outBitDepth="32f">
    <Array dim="3 4">
      0.5 0 0 0.1
      0 1 0 0
      0 0 1 0
    </Array>
  </Matrix>
  <LUT1D inBitDepth="32f" outBitDepth="10i">
    <Array dim="2 1">0 1023</Array>
  </LUT1D>
  <Range inBitDepth="10i" outBitDepth="32f">
    <minInValue>0</minInValue><maxInValue>1023</maxInValue><minOutValue>0</minOutValue><maxOutValue>1</maxOutValue>
  </Range>
  <Exponent inBitDepth="32f" outBitDepth="32f" style="monCurveRev">
    <ExponentParams exponent="2.4" offset="0.055"/>
  </Exponent>
  <ASC_CDL inBitDepth="32f" outBitDepth="32f" style="FwdNoClamp">
    <SOPNode><Slope>1 1 1</Slope><Offset>0 0 0</Offset><Power>1 1 1</Power></SOPNode>
  </ASC_CDL>
  <Log inBitDepth="32f" outBitDepth="32f" style="antiLog2"/>
</ProcessList>"#;

#[test]
fn common_lut_format() {
    let t = parse_clf(CLF).unwrap();
    assert!(t.skipped.is_empty(), "{:?}", t.skipped);
    assert_eq!(t.steps.len(), 6);
    let c = [0.4, 0.3, 0.2];
    // by hand: matrix, the identity table and range, sRGB written, then 2 to the power
    let hand = [0.4 * 0.5 + 0.1, 0.3, 0.2].map(|v: f64| 2f64.powf(Curve::Srgb.encode(v)) as f32);
    let got = t.run(c, T, false);
    assert!(near3(got, hand, 1e-4), "{got:?} {hand:?}");
    assert!(near3(t.run(got, T, true), c, 1e-4), "and back");
    // logs with a camera break meet the line smoothly
    let cam = parse_clf(r#"<ProcessList id="x"><Log style="cameraLinToLog"><LogParams base="2" logSideSlope="0.05" logSideOffset="0.6" linSideSlope="1" linSideOffset="0.0001" linSideBreak="0.01"/></Log></ProcessList>"#).unwrap();
    let f = |x: f32| cam.run([x; 3], T, false)[0];
    let (a, b, c2) = (f(0.0099), f(0.01), f(0.0101));
    assert!(((b - a) - (c2 - b)).abs() < 1e-5, "{a} {b} {c2}");
    for x in [-0.01, 0.005, 0.2, 4.0] {
        assert!((cam.run(cam.run([x; 3], T, false), T, true)[0] - x).abs() < 1e-5, "{x}");
    }
    assert!(parse_clf("<Something/>").is_err());
}

#[test]
fn lattices_run_backwards() {
    // a lattice bending each channel a little and mixing them: undone by searching
    let n = 17;
    let mut three = Vec::new();
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                let c = [r, g, b].map(|v| v as f32 / (n - 1) as f32);
                three.push([c[0].powf(1.2) * 0.9 + c[1] * 0.1, c[1].powf(0.9), c[2] * 0.8 + c[0] * 0.1 + 0.05]);
            }
        }
    }
    let t = Transform::of(vec![Step::Lut(Arc::new(Lut { size1: 0, one: Vec::new(), size3: n, three, lo: [0.0; 3], hi: [1.0; 3] }))]);
    for c in [[0.2, 0.4, 0.6], [0.7, 0.3, 0.1], [0.5, 0.9, 0.4]] {
        let back = t.run(t.run(c, T, false), T, true);
        assert!(near3(back, c, 1e-4), "{c:?} {back:?}");
    }
}

#[test]
fn reads_by_kind() {
    let cube = "LUT_1D_SIZE 2\n0 0 0\n1 1 1\n";
    assert!(matches!(parse(cube, "", "").unwrap().steps[0], Step::Lut(_)));
    assert!(load("", "").is_err());
    let inline = load("LUT_1D_SIZE 2\n0 0 0\n0.5 0.5 0.5\n", "").unwrap();
    assert!(near3(inline.run([1.0; 3], T, false), [0.5; 3], 1e-6));
    assert!(load("/nowhere/at/all.cube", "").is_err());
}

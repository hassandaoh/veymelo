//! 16-bit floats ("halves", IEEE 754 binary16): one sign bit, five of exponent, ten of
//! fraction.

/// A half as a float (exactly: every half is a float).
pub fn to_f32(h: u16) -> f32 {
    let sign = (h as u32 & 0x8000) << 16;
    let exp = (h >> 10 & 0x1f) as u32;
    let man = (h & 0x3ff) as u32;
    let bits = match exp {
        0 if man == 0 => sign,
        // too small for a normal half: made a normal float
        0 => {
            let shift = man.leading_zeros() - 21;
            sign | (113 - shift) << 23 | (man << shift & 0x3ff) << 13
        }
        // infinity, or not a number
        31 => sign | 0x7f80_0000 | man << 13,
        _ => sign | (exp + 112) << 23 | man << 13,
    };
    f32::from_bits(bits)
}

/// A float as a half, rounded to the nearest (ties to even).
pub fn from_f32(v: f32) -> u16 {
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32;
    let man = b & 0x7f_ffff;
    if exp == 0xff {
        // infinity, or not a number
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        // too small for a normal half: a subnormal one, or nothing
        if e < -10 {
            return sign;
        }
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let half_m = m >> shift;
        let rest = m & ((1 << shift) - 1);
        let mid = 1 << (shift - 1);
        let up = rest > mid || (rest == mid && half_m & 1 == 1);
        return sign | (half_m + up as u32) as u16;
    }
    let half_m = man >> 13;
    let rest = man & 0x1fff;
    let up = rest > 0x1000 || (rest == 0x1000 && half_m & 1 == 1);
    // rounding up may carry into the exponent, as it should
    (sign as u32 | ((e as u32) << 10) | half_m) as u16 + up as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_both_ways() {
        assert_eq!(from_f32(1.0), 0x3c00);
        assert_eq!(from_f32(-2.0), 0xc000);
        assert_eq!(from_f32(0.5), 0x3800);
        assert_eq!(from_f32(65504.0), 0x7bff);
        assert_eq!(from_f32(1e6), 0x7c00, "too big: infinity");
        assert_eq!(from_f32(5.960_464_5e-8), 0x0001, "the smallest");
        assert_eq!(from_f32(0.333_333), 0x3555);
        // every half there is comes back as itself
        for h in 0..=u16::MAX {
            let f = to_f32(h);
            if f.is_nan() {
                assert!(h & 0x7c00 == 0x7c00 && h & 0x3ff != 0);
                continue;
            }
            assert_eq!(from_f32(f), h, "{h:#06x} {f}");
        }
        assert_eq!(to_f32(0x0001), 5.960_464_5e-8);
        assert_eq!(to_f32(0x8400), -6.103_515_6e-5);
    }
}

//! Time as whole numbers of ticks, so every frame and every audio sample falls exactly on a
//! tick: a tick is a flick, 1/705,600,000 of a second, a number every common frame rate (24,
//! 25, 30, 48, 50, 60, 90, 100, 120, and their NTSC kin like 30000/1001) and audio rate (8 to
//! 192 kHz) divides. An `i64` of ticks spans thousands of years.
//!
//! Frame rates are fractions; timecode (with drop-frame for 29.97 and 59.94) is only a way to
//! show and read a frame number.

use std::fmt;
use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

/// Ticks in a second.
pub const TICKS_PER_SECOND: i64 = 705_600_000;

/// A moment, or a length of time, in ticks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(pub i64);

/// `a * b / c` rounded to the nearest whole number (halves away from zero), without overflow.
fn mul_div_round(a: i64, b: i64, c: i64) -> i64 {
    let (n, d) = (a as i128 * b as i128, c as i128);
    let q = n / d;
    let r = n % d;
    let up = 2 * r.abs() >= d.abs();
    let away = if (n < 0) != (d < 0) { -1 } else { 1 };
    (if up { q + away } else { q }) as i64
}

/// `a * b / c` rounded down, without overflow (`c` is above zero).
fn mul_div_floor(a: i64, b: i64, c: i64) -> i64 {
    (a as i128 * b as i128).div_euclid(c as i128) as i64
}

impl Tick {
    pub const ZERO: Tick = Tick(0);
    /// A second.
    pub const SECOND: Tick = Tick(TICKS_PER_SECOND);

    /// The nearest tick to `s` seconds.
    pub fn from_seconds(s: f64) -> Tick {
        Tick((s * TICKS_PER_SECOND as f64).round() as i64)
    }

    pub fn seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }

    /// `units` of `per_second` a second (samples, milliseconds…), to the nearest tick.
    pub fn from_units(units: i64, per_second: i64) -> Tick {
        Tick(mul_div_round(units, TICKS_PER_SECOND, per_second))
    }

    /// How many whole units of `per_second` a second have passed (rounded down).
    pub fn to_units_floor(self, per_second: i64) -> i64 {
        mul_div_floor(self.0, per_second, TICKS_PER_SECOND)
    }

    /// The nearest number of units of `per_second` a second.
    pub fn to_units_round(self, per_second: i64) -> i64 {
        mul_div_round(self.0, per_second, TICKS_PER_SECOND)
    }

    /// This length times `num / den`, to the nearest tick.
    pub fn mul_ratio(self, num: i64, den: i64) -> Tick {
        Tick(mul_div_round(self.0, num, den))
    }

    pub fn abs(self) -> Tick {
        Tick(self.0.abs())
    }

    pub fn min(self, o: Tick) -> Tick {
        Tick(self.0.min(o.0))
    }

    pub fn max(self, o: Tick) -> Tick {
        Tick(self.0.max(o.0))
    }

    pub fn clamp(self, lo: Tick, hi: Tick) -> Tick {
        Tick(self.0.clamp(lo.0, hi.0))
    }
}

impl Add for Tick {
    type Output = Tick;
    fn add(self, o: Tick) -> Tick {
        Tick(self.0 + o.0)
    }
}

impl Sub for Tick {
    type Output = Tick;
    fn sub(self, o: Tick) -> Tick {
        Tick(self.0 - o.0)
    }
}

impl Neg for Tick {
    type Output = Tick;
    fn neg(self) -> Tick {
        Tick(-self.0)
    }
}

impl AddAssign for Tick {
    fn add_assign(&mut self, o: Tick) {
        self.0 += o.0;
    }
}

impl SubAssign for Tick {
    fn sub_assign(&mut self, o: Tick) {
        self.0 -= o.0;
    }
}

/// A stretch of time: where it starts and how long it lasts (the end is not in it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TimeRange {
    pub start: Tick,
    pub duration: Tick,
}

impl TimeRange {
    pub fn new(start: Tick, duration: Tick) -> TimeRange {
        TimeRange { start, duration: duration.max(Tick::ZERO) }
    }

    pub fn from_bounds(start: Tick, end: Tick) -> TimeRange {
        TimeRange::new(start, end - start)
    }

    pub fn end(&self) -> Tick {
        self.start + self.duration
    }

    pub fn is_empty(&self) -> bool {
        self.duration.0 <= 0
    }

    pub fn contains(&self, t: Tick) -> bool {
        t >= self.start && t < self.end()
    }

    pub fn overlaps(&self, o: &TimeRange) -> bool {
        self.start < o.end() && o.start < self.end()
    }

    pub fn intersect(&self, o: &TimeRange) -> Option<TimeRange> {
        let (s, e) = (self.start.max(o.start), self.end().min(o.end()));
        (s < e).then(|| TimeRange::from_bounds(s, e))
    }
}

/// Frames a second, as a fraction (30000/1001 for 29.97), kept in lowest terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameRate {
    pub num: u32,
    pub den: u32,
}

impl Default for FrameRate {
    fn default() -> FrameRate {
        FrameRate::FPS_30
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

impl FrameRate {
    pub const FPS_23_976: FrameRate = FrameRate { num: 24000, den: 1001 };
    pub const FPS_24: FrameRate = FrameRate { num: 24, den: 1 };
    pub const FPS_25: FrameRate = FrameRate { num: 25, den: 1 };
    pub const FPS_29_97: FrameRate = FrameRate { num: 30000, den: 1001 };
    pub const FPS_30: FrameRate = FrameRate { num: 30, den: 1 };
    pub const FPS_47_952: FrameRate = FrameRate { num: 48000, den: 1001 };
    pub const FPS_48: FrameRate = FrameRate { num: 48, den: 1 };
    pub const FPS_50: FrameRate = FrameRate { num: 50, den: 1 };
    pub const FPS_59_94: FrameRate = FrameRate { num: 60000, den: 1001 };
    pub const FPS_60: FrameRate = FrameRate { num: 60, den: 1 };
    pub const FPS_119_88: FrameRate = FrameRate { num: 120000, den: 1001 };
    pub const FPS_120: FrameRate = FrameRate { num: 120, den: 1 };

    /// The usual rates.
    pub const COMMON: [FrameRate; 12] = [
        FrameRate::FPS_23_976,
        FrameRate::FPS_24,
        FrameRate::FPS_25,
        FrameRate::FPS_29_97,
        FrameRate::FPS_30,
        FrameRate::FPS_47_952,
        FrameRate::FPS_48,
        FrameRate::FPS_50,
        FrameRate::FPS_59_94,
        FrameRate::FPS_60,
        FrameRate::FPS_119_88,
        FrameRate::FPS_120,
    ];

    /// `num / den` frames a second, in lowest terms (None for zero or more than 1000 fps).
    pub fn new(num: u32, den: u32) -> Option<FrameRate> {
        if num == 0 || den == 0 || num as u64 > 1000 * den as u64 {
            return None;
        }
        let g = gcd(num as u64, den as u64) as u32;
        Some(FrameRate { num: num / g, den: den / g })
    }

    /// The rate `fps` stands for: an NTSC rate when it is one written short (29.97 → 30000/1001),
    /// else the fraction nearest it with a denominator up to 1001.
    pub fn from_f64(fps: f64) -> Option<FrameRate> {
        if !(fps > 0.0 && fps <= 1000.0) {
            return None;
        }
        if let Some(r) = FrameRate::COMMON.iter().find(|r| (r.as_f64() - fps).abs() < 0.0006) {
            return Some(*r);
        }
        if (fps - fps.round()).abs() < 1e-9 {
            return FrameRate::new(fps.round() as u32, 1);
        }
        // the closest fraction with a small denominator
        let mut best = (f64::MAX, 1u32, 1u32);
        for den in 1..=1001u32 {
            let num = (fps * den as f64).round();
            let err = (num / den as f64 - fps).abs();
            if num >= 1.0 && err < best.0 - 1e-12 {
                best = (err, num as u32, den);
            }
        }
        FrameRate::new(best.1, best.2)
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// How long a frame lasts (exact for the usual rates, else to the nearest tick).
    pub fn frame_duration(self) -> Tick {
        Tick(mul_div_round(TICKS_PER_SECOND, self.den as i64, self.num as i64))
    }

    /// The frame showing at `t` (the one that started last).
    pub fn frame_at(self, t: Tick) -> i64 {
        mul_div_floor(t.0, self.num as i64, TICKS_PER_SECOND * self.den as i64)
    }

    /// When frame `f` starts.
    pub fn tick_of(self, f: i64) -> Tick {
        Tick(mul_div_round(f, TICKS_PER_SECOND * self.den as i64, self.num as i64))
    }

    /// The start of the frame showing at `t`.
    pub fn snap(self, t: Tick) -> Tick {
        self.tick_of(self.frame_at(t))
    }

    /// The start of the frame nearest `t`.
    pub fn snap_nearest(self, t: Tick) -> Tick {
        let f = mul_div_round(t.0, self.num as i64, TICKS_PER_SECOND * self.den as i64);
        self.tick_of(f)
    }

    /// The frames a second timecode counts (30 for 29.97).
    pub fn timecode_base(self) -> i64 {
        (self.as_f64().round() as i64).max(1)
    }

    /// An NTSC rate (a whole rate times 1000/1001).
    pub fn is_ntsc(self) -> bool {
        self.den == 1001
    }

    /// Frames timecode drops each minute (but every tenth) to keep up with the clock: 2 for
    /// 29.97, 4 for 59.94, none for other rates.
    pub fn drop_frames(self) -> i64 {
        match (self.num, self.den) {
            (30000, 1001) => 2,
            (60000, 1001) => 4,
            _ => 0,
        }
    }

    /// Frame `frame` as timecode: `HH:MM:SS:FF`, or `HH;MM;SS;FF` drop-frame (for 29.97 and
    /// 59.94 when `drop`). A frame before zero is shown with a minus sign.
    pub fn timecode(self, frame: i64, drop: bool) -> String {
        let base = self.timecode_base();
        let d = if drop { self.drop_frames() } else { 0 };
        let sign = if frame < 0 { "-" } else { "" };
        let mut n = frame.abs();
        if d > 0 {
            // put back the numbers drop-frame skips, so the count reads as a clock
            let per_minute = base * 60 - d;
            let per_ten = per_minute * 10 + d;
            let (tens, rest) = (n / per_ten, n % per_ten);
            n += 9 * d * tens + if rest > d { d * ((rest - d) / per_minute) } else { 0 };
        }
        let (ff, s) = (n % base, n / base);
        let sep = if d > 0 { ';' } else { ':' };
        format!("{sign}{:02}{sep}{:02}{sep}{:02}{sep}{:02}", s / 3600, (s / 60) % 60, s % 60, ff)
    }

    /// The frame timecode stands for (`HH:MM:SS:FF`; with `;` it is read as drop-frame).
    pub fn parse_timecode(self, text: &str) -> Option<i64> {
        let text = text.trim();
        let (neg, text) = match text.strip_prefix('-') {
            Some(t) => (true, t),
            None => (false, text),
        };
        let drop = text.contains(';');
        let parts: Vec<i64> = text.split([':', ';', '.']).map(|p| p.parse().ok()).collect::<Option<_>>()?;
        let [h, m, s, f] = parts[..] else { return None };
        let base = self.timecode_base();
        if m >= 60 || s >= 60 || f >= base || h < 0 || m < 0 || s < 0 || f < 0 {
            return None;
        }
        let d = if drop { self.drop_frames() } else { 0 };
        let minutes = h * 60 + m;
        // drop-frame numbers that do not exist (the first frames of most minutes)
        if d > 0 && s == 0 && f < d && m % 10 != 0 {
            return None;
        }
        let n = (minutes * 60 + s) * base + f - d * (minutes - minutes / 10);
        Some(if neg { -n } else { n })
    }

    /// The rate in words: "29.97", "30", "23.976".
    pub fn label(self) -> String {
        if self.den == 1 {
            return self.num.to_string();
        }
        let s = format!("{:.3}", self.as_f64());
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

impl fmt::Display for FrameRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_common_rate_and_audio_rate_falls_on_whole_ticks() {
        for r in FrameRate::COMMON {
            assert_eq!((TICKS_PER_SECOND as i128 * r.den as i128) % r.num as i128, 0, "{r}");
        }
        for rate in [8000, 11025, 16000, 22050, 32000, 44100, 48000, 88200, 96000, 176400, 192000] {
            assert_eq!(TICKS_PER_SECOND % rate, 0, "{rate}");
        }
    }

    #[test]
    fn frames_and_ticks_go_both_ways_exactly() {
        for r in FrameRate::COMMON {
            for f in [-5, 0, 1, 2, 29, 30, 1799, 1800, 17982, 107892, 10_000_000] {
                let t = r.tick_of(f);
                assert_eq!(r.frame_at(t), f, "{r} frame {f}");
                // just before the frame starts is still the frame before
                assert_eq!(r.frame_at(t - Tick(1)), f - 1, "{r} frame {f}");
                assert_eq!(r.snap(t + Tick(1)), t);
            }
            assert_eq!(r.tick_of(1) - r.tick_of(0), r.frame_duration());
        }
        let r = FrameRate::FPS_30;
        assert_eq!(r.snap_nearest(r.tick_of(4) + r.frame_duration().mul_ratio(6, 10)), r.tick_of(5));
    }

    #[test]
    fn seconds_and_units() {
        assert_eq!(Tick::from_seconds(1.5), Tick(1_058_400_000));
        assert_eq!(Tick::from_seconds(2.0).seconds(), 2.0);
        assert_eq!(Tick::from_units(48000, 48000), Tick::SECOND);
        assert_eq!(Tick::SECOND.to_units_floor(44100), 44100);
        assert_eq!(Tick(-1).to_units_floor(1000), -1, "rounds down below zero too");
        assert_eq!(Tick(10).mul_ratio(1, 4), Tick(3));
        assert_eq!(Tick(-10).mul_ratio(1, 4), Tick(-3));
    }

    #[test]
    fn rates_from_numbers() {
        assert_eq!(FrameRate::from_f64(29.97), Some(FrameRate::FPS_29_97));
        assert_eq!(FrameRate::from_f64(23.976), Some(FrameRate::FPS_23_976));
        assert_eq!(FrameRate::from_f64(30.0), Some(FrameRate::FPS_30));
        assert_eq!(FrameRate::from_f64(12.5), FrameRate::new(25, 2));
        assert_eq!(FrameRate::from_f64(0.0), None);
        assert_eq!(FrameRate::new(60, 2), Some(FrameRate::FPS_30));
        assert_eq!(FrameRate::FPS_29_97.label(), "29.97");
        assert_eq!(FrameRate::FPS_23_976.label(), "23.976");
        assert_eq!(FrameRate::FPS_25.label(), "25");
    }

    #[test]
    fn timecode_reads_as_a_clock_drop_frame_too() {
        let r = FrameRate::FPS_29_97;
        assert_eq!(r.timecode(0, true), "00;00;00;00");
        assert_eq!(r.timecode(1799, true), "00;00;59;29");
        // the first two numbers of minute one are skipped
        assert_eq!(r.timecode(1800, true), "00;01;00;02");
        // but not at minute ten
        assert_eq!(r.timecode(17982, true), "00;10;00;00");
        assert_eq!(r.timecode(107892, true), "01;00;00;00");
        assert_eq!(r.timecode(1800, false), "00:01:00:00");
        assert_eq!(FrameRate::FPS_25.timecode(90061, true), "01:00:02:11", "no drop-frame at 25");
        assert_eq!(FrameRate::FPS_59_94.timecode(3600, true), "00;01;00;04");
        // and back
        for r in [FrameRate::FPS_29_97, FrameRate::FPS_59_94, FrameRate::FPS_24] {
            for f in (0..200_000).step_by(997) {
                for drop in [true, false] {
                    assert_eq!(r.parse_timecode(&r.timecode(f, drop)), Some(f), "{r} {f} {drop}");
                }
            }
        }
        assert_eq!(r.parse_timecode("00;01;00;00"), None, "a skipped number");
        assert_eq!(r.parse_timecode("00:00:00:30"), None);
        assert_eq!(r.timecode(-30, false), "-00:00:01:00");
        assert_eq!(r.parse_timecode("-00:00:01:00"), Some(-30));
    }

    #[test]
    fn ranges() {
        let a = TimeRange::new(Tick(10), Tick(10));
        let b = TimeRange::from_bounds(Tick(15), Tick(30));
        assert!(a.contains(Tick(10)) && !a.contains(Tick(20)));
        assert!(a.overlaps(&b));
        assert_eq!(a.intersect(&b), Some(TimeRange::new(Tick(15), Tick(5))));
        assert_eq!(a.intersect(&TimeRange::new(Tick(20), Tick(5))), None);
    }
}

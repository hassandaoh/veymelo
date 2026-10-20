//! WAV files (RIFF WAVE), our own: PCM of 16 or 24 bits, or 32-bit floating point; one or two
//! channels.

/// `samples` (stereo: left, right, left…) as a WAV file at `rate`, in `channels` (1: the two
/// averaged) and `bits` (16, 24, or 32 for floating point; whole samples are dithered by
/// rounding, kept within full scale).
pub fn encode(samples: &[f32], rate: u32, channels: u16, bits: u16) -> Vec<u8> {
    let channels = channels.clamp(1, 2);
    let bits = if matches!(bits, 16 | 24 | 32) { bits } else { 24 };
    let frames: Vec<[f32; 2]> = samples.as_chunks::<2>().0.to_vec();
    let bytes = (bits / 8) as usize;
    let data_len = frames.len() * channels as usize * bytes;
    let mut out = Vec::with_capacity(44 + data_len);
    let u32le = |out: &mut Vec<u8>, v: u32| out.extend(v.to_le_bytes());
    let u16le = |out: &mut Vec<u8>, v: u16| out.extend(v.to_le_bytes());
    out.extend(b"RIFF");
    u32le(&mut out, (36 + data_len) as u32);
    out.extend(b"WAVEfmt ");
    u32le(&mut out, 16);
    // 1: whole numbers, 3: floating point
    u16le(&mut out, if bits == 32 { 3 } else { 1 });
    u16le(&mut out, channels);
    u32le(&mut out, rate);
    u32le(&mut out, rate * channels as u32 * bytes as u32);
    u16le(&mut out, channels * bytes as u16);
    u16le(&mut out, bits);
    out.extend(b"data");
    u32le(&mut out, data_len as u32);
    for f in frames {
        let these = if channels == 1 { vec![(f[0] + f[1]) * 0.5] } else { vec![f[0], f[1]] };
        for v in these {
            match bits {
                32 => out.extend(v.to_le_bytes()),
                16 => out.extend(((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes()),
                _ => out.extend(&((v.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32).to_le_bytes()[..3]),
            }
        }
    }
    out
}

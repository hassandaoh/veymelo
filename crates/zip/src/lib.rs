//! Our own deflate and inflate (RFC 1951), zlib streams (RFC 1950), CRC-32 and zip archives:
//! what PNG pictures and packed animation files (`.lottie`) are made of.

mod archive;
mod deflate;
mod inflate;

pub use archive::{unzip, zip};
pub use deflate::{adler32, deflate, zlib};
pub use inflate::{inflate, inflate_up_to, unzlib};

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
}

static CRC: [u32; 256] = crc_table();

/// The CRC-32 (the one zip, PNG and gzip use) of `data`.
pub fn crc32(data: &[u8]) -> u32 {
    !data.iter().fold(!0u32, |c, &b| CRC[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8))
}

#[cfg(test)]
mod tests;

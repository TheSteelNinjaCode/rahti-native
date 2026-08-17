//! Placeholder launcher icons.
//!
//! `init` has to produce a project that builds, and both bundlers refuse to
//! run without icons: Tauri's Windows bundler embeds an `.ico` into the
//! executable, and `tauri android init` generates the launcher densities from
//! the PNGs. A scaffold that stopped at "now add some icons" would be a
//! scaffold nobody could run.
//!
//! So a plain one is written, in Rahti's own colour, and the generated
//! `native/README.md` says to replace it. It is deliberately not pretty:
//! nobody should ship it by accident.
//!
//! ## Why the encoders are here
//!
//! An image crate would be several hundred kilobytes of dependency to produce
//! six squares of one colour. A PNG whose pixel data is a *stored* deflate
//! block is legal PNG — deflate's stored mode is uncompressed — and an ICO is
//! a header, one directory entry and a bottom-up BGRA bitmap. Both are short
//! enough to write and to test.

/// The squares Tauri's bundlers look for.
pub const SIZES: &[(&str, u32)] = &[
    ("32x32.png", 32),
    ("128x128.png", 128),
    ("128x128@2x.png", 256),
    ("icon.png", 512),
];

/// The file the Windows bundler embeds into the executable.
pub const ICO: &str = "icon.ico";

/// Rahti's blue, as the placeholder colour. Opaque.
const COLOR: [u8; 4] = [0x2c, 0x5f, 0xd8, 0xff];

/// A square PNG of one colour.
pub fn png(size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);

    // IHDR: width, height, 8-bit, colour type 6 (RGBA), no interlace.
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);

    // Every row is a filter byte followed by its pixels. Filter 0 is "none",
    // which is what makes the rows predictable enough to store rather than
    // compress.
    let mut raw = Vec::with_capacity((size as usize + 1) * size as usize * 4);
    for _ in 0..size {
        raw.push(0);
        for _ in 0..size {
            raw.extend_from_slice(&COLOR);
        }
    }
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));

    chunk(&mut out, b"IEND", &[]);
    out
}

/// A 32×32 ICO holding one bottom-up BGRA bitmap.
///
/// A bitmap rather than an embedded PNG: PNG-in-ICO needs Vista or later to
/// read, which every target here is, and a bitmap needs nothing at all to
/// read. When the file's whole job is to not be the reason a build failed, the
/// older format is the better one.
pub fn ico() -> Vec<u8> {
    const SIZE: u32 = 32;

    // BITMAPINFOHEADER. The height is doubled because an ICO's bitmap carries
    // the colour rows and then the AND-mask rows, and the header describes
    // both.
    let mut dib = Vec::new();
    dib.extend_from_slice(&40u32.to_le_bytes());
    dib.extend_from_slice(&SIZE.to_le_bytes());
    dib.extend_from_slice(&(SIZE * 2).to_le_bytes());
    dib.extend_from_slice(&1u16.to_le_bytes()); // planes
    dib.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    dib.extend_from_slice(&[0u8; 24]); // no compression, no palette

    // Bottom-up BGRA.
    for _ in 0..SIZE {
        for _ in 0..SIZE {
            dib.extend_from_slice(&[COLOR[2], COLOR[1], COLOR[0], COLOR[3]]);
        }
    }
    // The AND mask, all zero: nothing is transparent. One bit per pixel, rows
    // padded to four bytes — at 32 pixels wide that is exactly four.
    dib.extend_from_slice(&vec![0u8; (SIZE as usize / 8) * SIZE as usize]);

    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    out.extend_from_slice(&1u16.to_le_bytes()); // one image
    out.push(SIZE as u8);
    out.push(SIZE as u8);
    out.push(0); // palette size: none
    out.push(0); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&(dib.len() as u32).to_le_bytes());
    out.extend_from_slice(&22u32.to_le_bytes()); // where the bitmap starts
    out.extend_from_slice(&dib);
    out
}

/// Length, type, payload, CRC — the shape of every PNG chunk.
fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);

    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// A zlib stream whose deflate blocks are all *stored*.
///
/// Stored mode is deflate saying "the next n bytes are literal". It compresses
/// nothing, which for a square of one colour costs a few hundred kilobytes on
/// the largest icon and saves a compression library.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01]; // deflate, 32K window, no preset dictionary

    let mut rest = data;
    loop {
        // A stored block's length is a `u16`, so a long stream is several
        // blocks and only the last one is marked final.
        let take = rest.len().min(u16::MAX as usize);
        let (block, remaining) = rest.split_at(take);
        let last = remaining.is_empty();

        out.push(if last { 1 } else { 0 });
        out.extend_from_slice(&(take as u16).to_le_bytes());
        out.extend_from_slice(&(!(take as u16)).to_le_bytes());
        out.extend_from_slice(block);

        if last {
            break;
        }
        rest = remaining;
    }

    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in data {
        a = (a + *byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

#[cfg(test)]
#[path = "tests/icons.rs"]
mod tests;

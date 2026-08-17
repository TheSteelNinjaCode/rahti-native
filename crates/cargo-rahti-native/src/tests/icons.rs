//! The placeholder icons. Both bundlers refuse to run without them, so what
//! matters is that they are structurally valid files rather than that they
//! look like anything.

use super::*;

#[test]
fn a_png_is_a_png() {
    let png = png(32);
    assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    assert_eq!(&png[12..16], b"IHDR");
    assert_eq!(&png[16..20], &32u32.to_be_bytes(), "width");
    assert_eq!(&png[20..24], &32u32.to_be_bytes(), "height");
    assert_eq!(&png[24..29], &[8, 6, 0, 0, 0], "8-bit RGBA, not interlaced");
    assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
}

#[test]
fn every_chunk_carries_a_correct_crc() {
    // The check a decoder makes first. A wrong one is a file the bundler
    // rejects with a message about nothing.
    let png = png(32);
    let mut at = 8;
    let mut chunks = 0;

    while at + 12 <= png.len() {
        let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let end = at + 8 + length;
        let expected = u32::from_be_bytes(png[end..end + 4].try_into().unwrap());
        assert_eq!(crc32(&png[at + 4..end]), expected, "chunk {chunks}");
        at = end + 4;
        chunks += 1;
    }

    assert_eq!(chunks, 3, "IHDR, IDAT, IEND");
    assert_eq!(at, png.len(), "there is nothing after IEND");
}

#[test]
fn the_pixel_data_is_a_well_formed_zlib_stream() {
    // Stored deflate blocks: each is a flag byte, a length, and that length
    // complemented — which is the check a decoder makes.
    let data = vec![7u8; 200_000];
    assert!(
        data.len() > u16::MAX as usize,
        "the case worth testing is a stream of several blocks"
    );

    let stream = zlib_stored(&data);
    assert_eq!(&stream[..2], &[0x78, 0x01]);

    let mut at = 2;
    let mut total = 0usize;
    loop {
        let last = stream[at] == 1;
        let length = u16::from_le_bytes(stream[at + 1..at + 3].try_into().unwrap());
        let complement = u16::from_le_bytes(stream[at + 3..at + 5].try_into().unwrap());
        assert_eq!(complement, !length, "the length check would fail");
        assert_eq!(
            &stream[at + 5..at + 5 + length as usize],
            &data[total..total + length as usize]
        );

        total += length as usize;
        at += 5 + length as usize;
        if last {
            break;
        }
    }

    assert_eq!(total, data.len(), "not every byte was stored");
    assert_eq!(&stream[at..], &adler32(&data).to_be_bytes());
    assert_eq!(at + 4, stream.len());
}

#[test]
fn every_size_a_bundler_looks_for_is_produced() {
    for (name, size) in SIZES {
        let png = png(*size);
        assert_eq!(&png[16..20], &size.to_be_bytes(), "{name}");
        assert!(png.len() > 100, "{name} is too small to be an image");
    }
}

#[test]
fn the_ico_is_one_thirty_two_pixel_bitmap() {
    let ico = ico();
    assert_eq!(&ico[..4], &[0, 0, 1, 0], "reserved, then type 1 (icon)");
    assert_eq!(
        u16::from_le_bytes(ico[4..6].try_into().unwrap()),
        1,
        "one image"
    );
    assert_eq!(ico[6], 32, "width");
    assert_eq!(ico[7], 32, "height");
    assert_eq!(
        u16::from_le_bytes(ico[12..14].try_into().unwrap()),
        32,
        "bits per pixel"
    );

    let size = u32::from_le_bytes(ico[14..18].try_into().unwrap()) as usize;
    let offset = u32::from_le_bytes(ico[18..22].try_into().unwrap()) as usize;
    assert_eq!(
        offset, 22,
        "the bitmap starts after the header and one entry"
    );
    assert_eq!(
        offset + size,
        ico.len(),
        "the entry describes the whole file"
    );

    // BITMAPINFOHEADER: 40 bytes, and a height of twice the width because the
    // bitmap carries the colour rows and then the AND mask.
    assert_eq!(u32::from_le_bytes(ico[22..26].try_into().unwrap()), 40);
    assert_eq!(u32::from_le_bytes(ico[26..30].try_into().unwrap()), 32);
    assert_eq!(u32::from_le_bytes(ico[30..34].try_into().unwrap()), 64);
}

#[test]
fn the_checksums_match_values_that_are_not_this_codes_opinion_of_itself() {
    // The standard check vectors for both algorithms.
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(adler32(b"123456789"), 0x091E_01DE);
}

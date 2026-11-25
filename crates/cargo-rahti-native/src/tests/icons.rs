//! The launcher icons a new shell starts with.
//!
//! They are embedded rather than generated, so what matters is that the set is
//! complete and that every file in it is the image format its name claims —
//! a bundler handed something else fails with a message about nothing.

use super::*;

/// The eight-byte signature every PNG starts with.
const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

fn dimensions(png: &[u8]) -> (u32, u32) {
    // IHDR is always the first chunk, and its payload starts at byte 16.
    assert_eq!(&png[..8], PNG_MAGIC, "not a PNG");
    assert_eq!(&png[12..16], b"IHDR", "IHDR is not the first chunk");
    (
        u32::from_be_bytes(png[16..20].try_into().unwrap()),
        u32::from_be_bytes(png[20..24].try_into().unwrap()),
    )
}

#[test]
fn every_size_a_bundler_looks_for_is_present_and_the_right_size() {
    // `bundle.icon` in the generated tauri.conf.json names the first three;
    // the Windows executable carries the `.ico`.
    for (name, expected) in [
        ("32x32.png", 32),
        ("128x128.png", 128),
        ("128x128@2x.png", 256),
        ("icon.png", 512),
    ] {
        let icon = DESKTOP
            .iter()
            .find(|i| i.path == name)
            .unwrap_or_else(|| panic!("{name} is missing"));
        let (width, height) = dimensions(icon.bytes);
        assert_eq!((width, height), (expected, expected), "{name}");
    }
}

#[test]
fn the_windows_executable_icon_is_an_ico() {
    let ico = DESKTOP
        .iter()
        .find(|i| i.path == "icon.ico")
        .expect("icon.ico is missing");
    // Reserved zero, then type 1 (icon rather than cursor).
    assert_eq!(&ico.bytes[..4], &[0, 0, 1, 0], "not an ICO");
    let images = u16::from_le_bytes(ico.bytes[4..6].try_into().unwrap());
    assert!(images > 0, "an ICO with no images in it");
}

#[test]
fn every_android_density_has_its_three_icons() {
    // Miss one and that density falls back to the project template's own,
    // which is the failure the whole embedded set exists to prevent.
    for density in ["mdpi", "hdpi", "xhdpi", "xxhdpi", "xxxhdpi"] {
        for name in ["ic_launcher", "ic_launcher_round", "ic_launcher_foreground"] {
            let path = format!("android/mipmap-{density}/{name}.png");
            let icon = ANDROID
                .iter()
                .find(|i| i.path == path)
                .unwrap_or_else(|| panic!("{path} is missing"));
            let (width, height) = dimensions(icon.bytes);
            assert_eq!(width, height, "{path} is not square");
            assert!(width >= 48, "{path} is {width}px, too small for a launcher");
        }
    }
}

#[test]
fn the_adaptive_icon_and_its_background_are_present() {
    // Android 8 and later compose `ic_launcher_foreground` over the colour
    // named by this XML pair; without them an adaptive launcher shows the
    // legacy square instead.
    for path in [
        "android/mipmap-anydpi-v26/ic_launcher.xml",
        "android/values/ic_launcher_background.xml",
    ] {
        let icon = ANDROID
            .iter()
            .find(|i| i.path == path)
            .unwrap_or_else(|| panic!("{path} is missing"));
        let text = String::from_utf8_lossy(icon.bytes);
        assert!(text.contains("<?xml"), "{path} is not XML");
    }
}

#[test]
fn a_density_climbs_with_its_name() {
    // mdpi through xxxhdpi is 1x to 4x. Out-of-order files would install and
    // look wrong only on some screens, which is the worst way to be wrong.
    let mut last = 0;
    for density in ["mdpi", "hdpi", "xhdpi", "xxhdpi", "xxxhdpi"] {
        let path = format!("android/mipmap-{density}/ic_launcher.png");
        let icon = ANDROID.iter().find(|i| i.path == path).unwrap();
        let (width, _) = dimensions(icon.bytes);
        assert!(
            width > last,
            "{density} is not larger than the density below"
        );
        last = width;
    }
}

#[test]
fn every_embedded_icon_has_bytes_and_a_relative_path() {
    for icon in all() {
        assert!(!icon.bytes.is_empty(), "{} is empty", icon.path);
        assert!(
            !icon.path.starts_with('/') && !icon.path.contains(".."),
            "{} is not a path inside native/icons/",
            icon.path
        );
        // `/` separators: these become directories on Windows too.
        assert!(!icon.path.contains('\\'), "{} uses a backslash", icon.path);
    }
}

#[test]
fn the_desktop_and_android_sets_do_not_overlap() {
    for desktop in DESKTOP {
        assert!(
            !ANDROID.iter().any(|a| a.path == desktop.path),
            "{} is in both sets",
            desktop.path
        );
    }
    assert_eq!(all().count(), DESKTOP.len() + ANDROID.len());
}

// ----------------------------------------------------------------- `icon`

#[test]
fn base64_is_the_standard_alphabet_with_padding() {
    // RFC 4648's own test vectors.
    for (input, expected) in [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(base64(input.as_bytes()), expected, "{input}");
    }
}

#[test]
fn the_adaptive_artwork_stays_inside_what_a_launcher_mask_shows() {
    let svg = adaptive_svg(b"foo", "image/svg+xml");

    assert!(
        svg.contains("href=\"data:image/svg+xml;base64,Zm9v\""),
        "{svg}"
    );
    // Centred, and small enough that the square's corners stay inside the
    // 66dp circle of 108dp that every mask, round ones included, leaves visible.
    assert!(svg.contains("width=\"430\""), "{svg}");
    assert!(svg.contains("x=\"297\" y=\"297\""), "{svg}");
    const { assert!(ADAPTIVE_ART_PERCENT * 108 * 1_415 <= 66 * 100 * 1_000) };
}

#[test]
fn only_the_two_formats_tauri_draws_from_are_taken() {
    assert_eq!(media_type(Path::new("a.SVG")).unwrap(), "image/svg+xml");
    assert_eq!(media_type(Path::new("a.png")).unwrap(), "image/png");
    assert!(media_type(Path::new("a.jpg")).is_err());
    assert!(media_type(Path::new("logo")).is_err());
}

fn project_with_icon(label: &str, bytes: &[u8]) -> Project {
    let root =
        std::env::temp_dir().join(format!("rahti-native-icons-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("native/icons")).unwrap();
    std::fs::write(root.join("native/icons/icon.png"), bytes).unwrap();
    Project {
        root,
        package: "fixture".to_string(),
        lib: "fixture".to_string(),
        version: "0.1.0".to_string(),
    }
}

#[test]
fn the_framework_mark_is_recognised_and_a_replacement_is_not() {
    let placeholder = DESKTOP.iter().find(|i| i.path == "icon.png").unwrap();
    let project = project_with_icon("placeholder", placeholder.bytes);
    assert!(is_placeholder(&project));

    let project = project_with_icon("replaced", b"somebody's logo");
    assert!(!is_placeholder(&project));
}

#[test]
fn the_default_adaptive_layer_keeps_the_mark_inside_the_safe_zone() {
    // The bug this exists for: the embedded `ic_launcher_foreground` was drawn
    // edge to edge, and a launcher shows only the middle of the 108dp layer
    // through its mask — so the installed application's icon was cropped and
    // looked off-centre. Then it was inset to the central 66dp *square*, and a
    // round launcher still cut the mark's corners: the safe zone is a circle.
    // Everything outside the 66dp circle has to be empty.
    let layers: Vec<_> = ANDROID
        .iter()
        .filter(|icon| icon.path.ends_with("ic_launcher_foreground.png"))
        .collect();
    assert_eq!(layers.len(), 5, "one adaptive layer per density");

    for icon in layers {
        let decoder = png::Decoder::new(std::io::Cursor::new(icon.bytes));
        let mut reader = decoder.read_info().expect(icon.path);
        let mut pixels = vec![0; reader.output_buffer_size().expect(icon.path)];
        let frame = reader.next_frame(&mut pixels).expect(icon.path);
        assert_eq!(
            frame.color_type,
            png::ColorType::Rgba,
            "{} has no alpha",
            icon.path
        );

        let (width, height) = (frame.width as usize, frame.height as usize);
        // Outside the 66dp circle every mask keeps, in this density's pixels.
        let centre = width as f64 / 2.0;
        let radius = width as f64 * 33.0 / 108.0;
        for y in 0..height {
            for x in 0..width {
                let (dx, dy) = (x as f64 + 0.5 - centre, y as f64 + 0.5 - centre);
                if dx * dx + dy * dy <= radius * radius {
                    continue;
                }
                let alpha = pixels[(y * width + x) * 4 + 3];
                assert!(
                    alpha < 8,
                    "{} draws at ({x}, {y}), outside the circle a round launcher keeps",
                    icon.path
                );
            }
        }
    }
}

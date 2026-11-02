//! The launcher icons a new native shell starts with.
//!
//! `init` has to produce a project that builds and installs looking like
//! something. Both bundlers refuse to run without icons — Tauri's Windows
//! bundler embeds an `.ico` into the executable — and Android needs a launcher
//! icon per screen density or it falls back to whatever the project template
//! shipped.
//!
//! ## Why these are embedded rather than generated
//!
//! They used to be generated: a flat square of one colour, written by a small
//! PNG encoder in this file. That produced valid files and a bad outcome. A
//! flat square is not recognisable as anything, and — worse — it did nothing
//! at all about the case below, so an Android build shipped the Tauri robot
//! and looked like a Tauri application rather than a Rahti one.
//!
//! So the real set is embedded: the Rahti mark, at every size the two
//! platforms ask for, produced by `cargo tauri icon` from the framework's own
//! logo. Around 550 KB in the binary, which is the price of a scaffold that
//! looks right the first time.
//!
//! ## Still a default, not a brand
//!
//! It is Rahti's mark, not the application's. The generated `native/README.md`
//! says to replace it, `cargo rahti native icon path/to/logo.svg` rewrites the
//! whole set from one image, and `doctor` warns until it has. Shipping somebody
//! else's logo is a smaller mistake than shipping a blank square, and it is
//! still a mistake.
//!
//! ## The Android half is not optional
//!
//! `cargo tauri android init` writes a project whose `res/mipmap-*` already
//! contains the template's own `ic_launcher.webp`. Nothing copies `icons/`
//! over it, so the icons in this directory reach a Windows package and are
//! ignored entirely by an Android one. [`ANDROID`] is what
//! `crate::run::install_android_icons` copies into the generated project on
//! every Android build, and the `.webp` it replaces is deleted rather than
//! left beside it — two files with one resource name is a build error.

use std::path::{Path, PathBuf};

use rahti_native::NativeError;

use crate::args;
use crate::project::Project;

/// One embedded file: where it goes under `native/icons/`, and its bytes.
pub struct Icon {
    /// Relative to `native/icons/`, with `/` separators.
    pub path: &'static str,
    pub bytes: &'static [u8],
}

/// What `bundle.icon` names, plus the `.ico` the Windows executable carries.
pub const DESKTOP: &[Icon] = &[
    Icon {
        path: "32x32.png",
        bytes: include_bytes!("../assets/icons/32x32.png"),
    },
    Icon {
        path: "128x128.png",
        bytes: include_bytes!("../assets/icons/128x128.png"),
    },
    Icon {
        path: "128x128@2x.png",
        bytes: include_bytes!("../assets/icons/128x128@2x.png"),
    },
    Icon {
        path: "icon.png",
        bytes: include_bytes!("../assets/icons/icon.png"),
    },
    Icon {
        path: "icon.ico",
        bytes: include_bytes!("../assets/icons/icon.ico"),
    },
];

/// The Android launcher set, one entry per screen density.
///
/// `ic_launcher` is the square icon, `ic_launcher_round` the round one older
/// launchers ask for, and `ic_launcher_foreground` the adaptive-icon layer that
/// `mipmap-anydpi-v26/ic_launcher.xml` composes over the background colour in
/// `values/ic_launcher_background.xml`.
pub const ANDROID: &[Icon] = &[
    Icon {
        path: "android/mipmap-anydpi-v26/ic_launcher.xml",
        bytes: include_bytes!("../assets/icons/android/mipmap-anydpi-v26/ic_launcher.xml"),
    },
    Icon {
        path: "android/values/ic_launcher_background.xml",
        bytes: include_bytes!("../assets/icons/android/values/ic_launcher_background.xml"),
    },
    Icon {
        path: "android/mipmap-mdpi/ic_launcher.png",
        bytes: include_bytes!("../assets/icons/android/mipmap-mdpi/ic_launcher.png"),
    },
    Icon {
        path: "android/mipmap-mdpi/ic_launcher_round.png",
        bytes: include_bytes!("../assets/icons/android/mipmap-mdpi/ic_launcher_round.png"),
    },
    Icon {
        path: "android/mipmap-mdpi/ic_launcher_foreground.png",
        bytes: include_bytes!("../assets/icons/android/mipmap-mdpi/ic_launcher_foreground.png"),
    },
    Icon {
        path: "android/mipmap-hdpi/ic_launcher.png",
        bytes: include_bytes!("../assets/icons/android/mipmap-hdpi/ic_launcher.png"),
    },
    Icon {
        path: "android/mipmap-hdpi/ic_launcher_round.png",
        bytes: include_bytes!("../assets/icons/android/mipmap-hdpi/ic_launcher_round.png"),
    },
    Icon {
        path: "android/mipmap-hdpi/ic_launcher_foreground.png",
        bytes: include_bytes!("../assets/icons/android/mipmap-hdpi/ic_launcher_foreground.png"),
    },
    Icon {
        path: "android/mipmap-xhdpi/ic_launcher.png",
        bytes: include_bytes!("../assets/icons/android/mipmap-xhdpi/ic_launcher.png"),
    },
    Icon {
        path: "android/mipmap-xhdpi/ic_launcher_round.png",
        bytes: include_bytes!("../assets/icons/android/mipmap-xhdpi/ic_launcher_round.png"),
    },
    Icon {
        path: "android/mipmap-xhdpi/ic_launcher_foreground.png",
        bytes: include_bytes!("../assets/icons/android/mipmap-xhdpi/ic_launcher_foreground.png"),
    },
    Icon {
        path: "android/mipmap-xxhdpi/ic_launcher.png",
        bytes: include_bytes!("../assets/icons/android/mipmap-xxhdpi/ic_launcher.png"),
    },
    Icon {
        path: "android/mipmap-xxhdpi/ic_launcher_round.png",
        bytes: include_bytes!("../assets/icons/android/mipmap-xxhdpi/ic_launcher_round.png"),
    },
    Icon {
        path: "android/mipmap-xxhdpi/ic_launcher_foreground.png",
        bytes: include_bytes!("../assets/icons/android/mipmap-xxhdpi/ic_launcher_foreground.png"),
    },
    Icon {
        path: "android/mipmap-xxxhdpi/ic_launcher.png",
        bytes: include_bytes!("../assets/icons/android/mipmap-xxxhdpi/ic_launcher.png"),
    },
    Icon {
        path: "android/mipmap-xxxhdpi/ic_launcher_round.png",
        bytes: include_bytes!("../assets/icons/android/mipmap-xxxhdpi/ic_launcher_round.png"),
    },
    Icon {
        path: "android/mipmap-xxxhdpi/ic_launcher_foreground.png",
        bytes: include_bytes!("../assets/icons/android/mipmap-xxxhdpi/ic_launcher_foreground.png"),
    },
];

/// Everything `init` writes under `native/icons/`.
pub fn all() -> impl Iterator<Item = &'static Icon> {
    DESKTOP.iter().chain(ANDROID.iter())
}

/// Whether the project still wears the framework's mark.
pub fn is_placeholder(project: &Project) -> bool {
    let path = project.native_dir().join("icons/icon.png");
    let placeholder = DESKTOP.iter().find(|icon| icon.path == "icon.png");
    match (std::fs::read(path), placeholder) {
        (Ok(current), Some(placeholder)) => current == placeholder.bytes,
        _ => false,
    }
}

/// How much of the 108dp adaptive layer the artwork's square covers.
///
/// A launcher shows the central 72dp through a mask, and only the central 66dp
/// is promised to survive every mask. That is 61% for artwork inside the circle
/// its square encloses, and logos are rarely centred in their own square, so a
/// little more margin keeps an off-centre tip inside a round mask too.
const ADAPTIVE_ART_PERCENT: u32 = 56;

/// `cargo rahti native icon` — every launcher icon, from one image.
///
/// ## Why not `cargo tauri icon` alone
///
/// It draws `ic_launcher_foreground` edge to edge, so a launcher crops the
/// logo, and its `android_fg_scale` manifest field leaves that layer unscaled.
/// It also writes iOS and Microsoft Store sets a Rahti shell has no use for.
///
/// It still does all the drawing: this tool has no image dependency, and an SVG
/// that places the source inside a transparent margin is one Tauri can rasterize
/// for the adaptive layer. Everything lands in a working directory first, so a
/// failure leaves `native/icons/` as it was.
pub fn generate(project: &Project, args: &args::Icon) -> Result<(), NativeError> {
    let native = project.native_dir();
    if !native.join("tauri.conf.json").is_file() {
        return Err(NativeError::at(
            "icon",
            &native,
            "there is no native shell here yet.\n  \
             Create one with: cargo rahti native init",
        ));
    }

    let source = std::path::absolute(&args.source)
        .map_err(|e| NativeError::new("icon", format!("cannot read that path: {e}")))?;
    let media_type = media_type(&source)?;
    let bytes = std::fs::read(&source).map_err(|e| NativeError::io("icon", &source, e))?;

    let work = native.join(".rahti/icons");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| NativeError::io("icon", &work, e))?;

    let manifest = work.join("manifest.json");
    let mut fields = serde_json::json!({ "default": source });
    if let Some(background) = &args.background {
        fields["bg_color"] = background.as_str().into();
    }
    write(&manifest, &fields.to_string())?;

    let adaptive = work.join("adaptive.svg");
    write(&adaptive, &adaptive_svg(&bytes, media_type))?;

    tauri_icon(&native, &manifest, &work.join("set"))?;
    tauri_icon(&native, &adaptive, &work.join("adaptive"))?;

    let copies: Vec<(PathBuf, PathBuf)> = all()
        .map(|icon| {
            let set = if icon.path.ends_with("ic_launcher_foreground.png") {
                "adaptive"
            } else {
                "set"
            };
            (
                work.join(set).join(icon.path),
                native.join("icons").join(icon.path),
            )
        })
        .collect();

    if let Some((missing, _)) = copies.iter().find(|(from, _)| !from.is_file()) {
        return Err(NativeError::at(
            "icon",
            missing,
            "`cargo tauri icon` did not write this file, so native/icons/ was left as it was.",
        ));
    }

    for (from, to) in &copies {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| NativeError::io("icon", parent, e))?;
        }
        std::fs::copy(from, to).map_err(|e| NativeError::io("icon", to, e))?;
    }
    let _ = std::fs::remove_dir_all(&work);

    println!();
    println!("  icons: wrote {} files to native/icons/", copies.len());
    println!("         from {}", source.display());
    Ok(())
}

fn tauri_icon(native: &Path, input: &Path, output: &Path) -> Result<(), NativeError> {
    let mut command = crate::doctor::cargo_tauri(&["icon"]);
    command.arg(input).arg("--output").arg(output);
    command.current_dir(native);
    crate::run::execute(command, "icon")
}

fn write(path: &Path, contents: &str) -> Result<(), NativeError> {
    std::fs::write(path, contents).map_err(|e| NativeError::io("icon", path, e))
}

fn media_type(source: &Path) -> Result<&'static str, NativeError> {
    let extension = source.extension().and_then(|e| e.to_str());
    match extension.map(str::to_ascii_lowercase).as_deref() {
        Some("svg") => Ok("image/svg+xml"),
        Some("png") => Ok("image/png"),
        _ => Err(NativeError::at(
            "icon",
            source,
            "`icon` takes a square PNG or SVG. Those are the two formats Tauri draws from.",
        )),
    }
}

/// An SVG drawing `image` inside the adaptive layer's safe zone.
pub(crate) fn adaptive_svg(image: &[u8], media_type: &str) -> String {
    const SIDE: u32 = 1024;
    let art = SIDE * ADAPTIVE_ART_PERCENT / 100;
    let inset = (SIDE - art) / 2;
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{SIDE}\" height=\"{SIDE}\" \
         viewBox=\"0 0 {SIDE} {SIDE}\"><image x=\"{inset}\" y=\"{inset}\" width=\"{art}\" \
         height=\"{art}\" href=\"data:{media_type};base64,{}\"/></svg>",
        base64(image)
    )
}

pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= chunk.len() {
                ALPHABET[(n >> shift & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

#[cfg(test)]
#[path = "tests/icons.rs"]
mod tests;

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
//! says to replace it, and `cargo tauri icon path/to/icon.png` rewrites the
//! whole set from one square PNG. Shipping somebody else's logo is a smaller
//! mistake than shipping a blank square, and it is still a mistake.
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

#[cfg(test)]
#[path = "tests/icons.rs"]
mod tests;

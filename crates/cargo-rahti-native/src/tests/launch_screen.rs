//! The Android launch screen patch.

use super::*;

/// Tauri 2's generated manifest, in the shape this patch anchors on.
const MANIFEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <application
        android:icon="@mipmap/ic_launcher"
        android:theme="@style/Theme.demo_native"
        android:usesCleartextTraffic="${usesCleartextTraffic}">
        <activity
            android:launchMode="singleTask"
            android:name=".MainActivity"
            android:exported="true">
        </activity>
    </application>
</manifest>
"#;

const ACTIVITY: &str = "package com.example.app

import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }
}
";

/// A throwaway `gen/android` with a manifest, an activity and a launcher icon.
fn generated(label: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let root = std::env::temp_dir().join(format!(
        "rahti-native-launch-{label}-{}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let java = root.join("app/src/main/java/com/example/app");
    let mipmap = root.join("app/src/main/res/drawable-nodpi");
    std::fs::create_dir_all(&java).unwrap();
    std::fs::create_dir_all(&mipmap).unwrap();
    std::fs::write(root.join("app/src/main/AndroidManifest.xml"), MANIFEST).unwrap();
    std::fs::write(java.join("MainActivity.kt"), ACTIVITY).unwrap();
    std::fs::write(mipmap.join("rahti_launch_icon.png"), b"png").unwrap();
    root
}

fn read(root: &std::path::Path, file: &str) -> String {
    std::fs::read_to_string(root.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"))
}

const ACTIVITY_FILE: &str = "app/src/main/java/com/example/app/MainActivity.kt";
const CLASS: &str = "app/src/main/java/com/example/app/RahtiLaunchScreen.kt";
const MANIFEST_FILE: &str = "app/src/main/AndroidManifest.xml";

#[test]
fn a_launch_shows_the_icon_on_the_configured_colours() {
    // The bug this exists for: a launch was a white window, then a blank
    // WebView, then the page, and every application fixed it by hand.
    let root = generated("install");
    let splash = SplashConfig {
        enabled: true,
        background: "#0a2540".to_string(),
        background_dark: "#000000".to_string(),
    };
    install(&root, &splash).expect("the patch applies");

    let light = read(&root, "app/src/main/res/values/rahti_launch.xml");
    assert!(light.contains(">#0a2540</color>"), "{light}");
    let dark = read(&root, "app/src/main/res/values-night/rahti_launch.xml");
    assert!(dark.contains(">#000000</color>"), "{dark}");

    // Android 12+: the system splash, with the icon's artwork.
    let v31 = read(&root, "app/src/main/res/values-v31/rahti_launch.xml");
    assert!(
        v31.contains("android:windowSplashScreenBackground"),
        "{v31}"
    );
    assert!(v31.contains("@drawable/rahti_launch_icon"), "{v31}");

    // Before it: the window background.
    let background = read(
        &root,
        "app/src/main/res/drawable/rahti_launch_background.xml",
    );
    assert!(background.contains("@drawable/rahti_launch_icon"));

    let manifest = read(&root, MANIFEST_FILE);
    assert!(
        manifest.contains(r#"android:theme="@style/Theme.Rahti.Launch""#),
        "{manifest}"
    );
    // The application's own theme is still there, and is what ours extends.
    assert!(manifest.contains("@style/Theme.demo_native"));
    assert!(light.contains(r#"parent="Theme.demo_native""#), "{light}");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_splash_is_held_until_the_page_has_painted() {
    let root = generated("held");
    install(&root, &SplashConfig::default()).unwrap();

    let activity = read(&root, ACTIVITY_FILE);
    let super_at = activity.find("super.onCreate(savedInstanceState)").unwrap();
    let install_at = activity.find("RahtiLaunchScreen.install(this)").unwrap();
    assert!(install_at > super_at, "{activity}");

    let class = read(&root, CLASS);
    assert!(class.starts_with("package com.example.app\n"), "{class}");
    // Claimed rather than left to go at the first frame, and never held
    // forever.
    assert!(class.contains("setOnExitAnimationListener"));
    assert!(class.contains("postVisualStateCallback"));
    assert!(class.contains("MAX_HOLD_MS"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn applying_it_twice_changes_nothing() {
    let root = generated("twice");
    install(&root, &SplashConfig::default()).unwrap();
    let once = (read(&root, MANIFEST_FILE), read(&root, ACTIVITY_FILE));
    install(&root, &SplashConfig::default()).unwrap();
    let twice = (read(&root, MANIFEST_FILE), read(&root, ACTIVITY_FILE));

    assert_eq!(once, twice);
    assert_eq!(twice.0.matches("Theme.Rahti.Launch").count(), 1);
    assert_eq!(twice.1.matches("RahtiLaunchScreen.install").count(), 1);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn it_sits_beside_the_insets_patch_in_either_order() {
    let insets = crate::run::with_insets(ACTIVITY).unwrap();
    let both = with_launch_screen(&insets, true).expect("after the insets");
    assert!(both.contains("setOnApplyWindowInsetsListener"));
    assert!(both.contains("RahtiLaunchScreen.install(this)"));

    let launch = with_launch_screen(ACTIVITY, true).unwrap();
    let both = crate::run::with_insets(&launch).expect("before the insets");
    assert!(both.contains("RahtiLaunchScreen.install(this)"));
}

#[test]
fn turning_it_off_takes_it_out_again() {
    // A shell with its own launch screen must not be left with two, and a
    // theme the manifest still named would not compile without its resource.
    let root = generated("off");
    install(&root, &SplashConfig::default()).unwrap();

    let off = SplashConfig {
        enabled: false,
        ..SplashConfig::default()
    };
    install(&root, &off).expect("removal");

    assert_eq!(read(&root, MANIFEST_FILE), MANIFEST);
    assert_eq!(read(&root, ACTIVITY_FILE), ACTIVITY);
    assert!(!root.join(CLASS).exists());
    for file in RESOURCES {
        assert!(!root.join("app/src/main/res").join(file).exists(), "{file}");
    }

    // And off from the start touches nothing.
    install(&root, &off).unwrap();
    assert_eq!(read(&root, MANIFEST_FILE), MANIFEST);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_activity_with_a_theme_of_its_own_is_reported_not_edited() {
    let themed = MANIFEST.replace(
        "android:launchMode=\"singleTask\"",
        "android:theme=\"@style/Theme.Mine\"",
    );
    assert!(with_launch_theme(&themed).is_none());

    let root = generated("themed");
    std::fs::write(root.join(MANIFEST_FILE), &themed).unwrap();
    let error = install(&root, &SplashConfig::default()).expect_err("unrecognised");
    assert!(
        error.to_string().contains("android.splash.enabled"),
        "{error}"
    );

    // Nothing was half-applied.
    assert_eq!(read(&root, MANIFEST_FILE), themed);
    assert_eq!(read(&root, ACTIVITY_FILE), ACTIVITY);
    assert!(!root.join(CLASS).exists());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_activity_it_does_not_recognise_is_reported_not_edited() {
    let custom = "package com.example.app\n\nclass MainActivity : TauriActivity() {\n  override fun onCreate(b: Bundle?) {\n    super.onCreate(b)\n  }\n}\n";
    assert!(with_launch_screen(custom, true).is_none());

    let root = generated("custom");
    std::fs::write(root.join(ACTIVITY_FILE), custom).unwrap();
    assert!(install(&root, &SplashConfig::default()).is_err());
    assert_eq!(read(&root, MANIFEST_FILE), MANIFEST);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_activity_with_windows_line_endings_keeps_them() {
    let crlf = ACTIVITY.replace('\n', "\r\n");
    let patched = with_launch_screen(&crlf, true).unwrap();
    assert!(patched.contains("RahtiLaunchScreen.install(this)"));
    assert!(
        !patched.replace("\r\n", "").contains('\n'),
        "line endings were mixed"
    );
    assert_eq!(with_launch_screen(&patched, false).unwrap(), crlf);
}

#[test]
fn a_project_without_a_foreground_icon_gets_the_colour_alone() {
    let files = resources(&SplashConfig::default(), "Theme.demo", None);
    for (file, content) in &files {
        assert!(!content.contains("rahti_launch_icon"), "{file}");
    }
    // Still a splash: the colour is what replaces the white window.
    assert!(files[2].1.contains("android:windowSplashScreenBackground"));
}

#[test]
fn the_theme_extends_materials_when_the_manifest_names_none() {
    let bare = MANIFEST.replace("        android:theme=\"@style/Theme.demo_native\"\n", "");
    assert_eq!(
        application_theme(&bare),
        "Theme.MaterialComponents.DayNight.NoActionBar"
    );
    assert_eq!(application_theme(MANIFEST), "Theme.demo_native");
}

#[test]
fn the_splash_stays_over_the_start_page_of_a_hosted_application() {
    // Otherwise a launch is the splash, then a spinner, then the application:
    // two fades. The start page says `#offline` when it has nowhere to go.
    let class = class("com.example.app");
    let start = class
        .find("START_PAGE_HOST ->")
        .expect("the start page case");
    let painted = class.find("view.progress >= 100").unwrap();
    assert!(start < painted, "the start page is revealed as a page");
    assert!(
        class.contains("fragment == \"offline\") reveal()"),
        "{class}"
    );
}

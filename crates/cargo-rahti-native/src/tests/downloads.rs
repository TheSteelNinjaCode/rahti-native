//! The Android downloads patch.

use super::*;

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

/// A throwaway `gen/android` with an activity in it.
fn generated(label: &str, activity: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let root = std::env::temp_dir().join(format!(
        "rahti-native-downloads-{label}-{}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let java = root.join("app/src/main/java/com/example/app");
    std::fs::create_dir_all(&java).unwrap();
    std::fs::write(java.join("MainActivity.kt"), activity).unwrap();
    root
}

const ACTIVITY_FILE: &str = "app/src/main/java/com/example/app/MainActivity.kt";
const CLASS: &str = "app/src/main/java/com/example/app/RahtiDownloads.kt";

#[test]
fn a_download_in_a_page_has_somewhere_to_go() {
    // The bug this exists for: a WebView with no download listener drops the
    // download, so `<a download>` was a link that did nothing on a phone.
    let root = generated("install", ACTIVITY);
    install(&root).expect("the patch applies");

    let activity = std::fs::read_to_string(root.join(ACTIVITY_FILE)).unwrap();
    let super_at = activity.find("super.onCreate(savedInstanceState)").unwrap();
    let install_at = activity.find("RahtiDownloads.install(this)").unwrap();
    assert!(install_at > super_at, "{activity}");

    let class = std::fs::read_to_string(root.join(CLASS)).unwrap();
    assert!(class.starts_with("package com.example.app\n"), "{class}");
    assert!(class.contains("setDownloadListener"));
    assert!(class.contains("MediaStore.Downloads"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_cookies_go_to_the_applications_own_server_only() {
    // The fetch carries the launch cookie and the session cookie. Fetching in
    // this process, from this origin, is what keeps them here.
    let class = class("com.example.app");
    assert!(class.contains("sameOrigin(url, page)"), "{class}");
    assert!(
        !class.contains("DownloadManager"),
        "the system's downloader runs in another process"
    );
}

#[test]
fn applying_it_twice_adds_one_line() {
    let root = generated("twice", ACTIVITY);
    install(&root).unwrap();
    install(&root).unwrap();

    let activity = std::fs::read_to_string(root.join(ACTIVITY_FILE)).unwrap();
    assert_eq!(activity.matches("RahtiDownloads.install").count(), 1);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn it_sits_beside_the_other_activity_patches() {
    let patched = crate::run::with_insets(ACTIVITY).unwrap();
    let patched = crate::launch_screen::with_launch_screen(&patched, true).unwrap();
    let patched = with_downloads(&patched).expect("after the others");

    for line in [
        "setOnApplyWindowInsetsListener",
        "RahtiLaunchScreen.install(this)",
        "RahtiDownloads.install(this)",
    ] {
        assert_eq!(patched.matches(line).count(), 1, "{line}: {patched}");
    }
}

#[test]
fn an_activity_it_does_not_recognise_is_reported_not_edited() {
    let custom = "package com.example.app\n\nclass MainActivity : TauriActivity() {\n  override fun onCreate(b: Bundle?) {\n    super.onCreate(b)\n  }\n}\n";
    let root = generated("custom", custom);

    let error = install(&root).expect_err("unrecognised");
    assert!(error.to_string().contains("download"), "{error}");
    assert_eq!(
        std::fs::read_to_string(root.join(ACTIVITY_FILE)).unwrap(),
        custom
    );
    assert!(!root.join(CLASS).exists());

    let _ = std::fs::remove_dir_all(&root);
}

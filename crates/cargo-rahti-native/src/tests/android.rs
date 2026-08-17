//! The Android project patches `cargo rahti native` applies after Tauri has
//! generated it.

use super::*;

/// Tauri 2's generated manifest, in the shape this patch anchors on.
const TAURI_MANIFEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <uses-permission android:name="android.permission.INTERNET" />

    <application
        android:icon="@mipmap/ic_launcher"
        android:label="@string/app_name"
        android:theme="@style/Theme.demo"
        android:usesCleartextTraffic="${usesCleartextTraffic}">
        <activity android:name=".MainActivity" android:exported="true" />
    </application>
</manifest>
"#;

/// A throwaway `gen/android` with a manifest in it.
fn generated(label: &str, manifest: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let root = std::env::temp_dir().join(format!(
        "rahti-native-android-{label}-{}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("app/src/main")).unwrap();
    std::fs::write(root.join("app/src/main/AndroidManifest.xml"), manifest).unwrap();
    root
}

fn manifest_of(root: &std::path::Path) -> String {
    std::fs::read_to_string(root.join("app/src/main/AndroidManifest.xml")).unwrap()
}

#[test]
fn the_loopback_server_is_reachable_from_the_webview() {
    // The bug this exists for: Tauri sets `usesCleartextTraffic=false` on a
    // release build — correct for Tauri, wrong for Rahti, whose whole design
    // is the real router on `http://127.0.0.1`. Without this the release APK
    // installs, launches, and shows a blank screen while the server runs.
    let root = generated("patch", TAURI_MANIFEST);
    crate::run::allow_loopback_cleartext(&root).expect("the patch applies");

    let manifest = manifest_of(&root);
    assert!(
        manifest.contains(r#"android:networkSecurityConfig="@xml/rahti_network_security_config""#),
        "{manifest}"
    );

    let config = std::fs::read_to_string(
        root.join("app/src/main/res/xml/rahti_network_security_config.xml"),
    )
    .expect("the network security config");
    assert!(config.contains("<domain includeSubdomains=\"false\">127.0.0.1</domain>"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn cleartext_stays_refused_for_everything_else() {
    // Setting `usesCleartextTraffic=true` would also have fixed the blank
    // screen, and would have permitted cleartext to every host — a real
    // downgrade for an application that talks to an API over TLS.
    let root = generated("scope", TAURI_MANIFEST);
    crate::run::allow_loopback_cleartext(&root).expect("the patch applies");

    let config = std::fs::read_to_string(
        root.join("app/src/main/res/xml/rahti_network_security_config.xml"),
    )
    .unwrap();
    assert!(
        config.contains(r#"<base-config cleartextTrafficPermitted="false" />"#),
        "{config}"
    );
    // And the manifest's own flag is left as Tauri wrote it — the config takes
    // precedence, so the two cannot disagree.
    assert!(
        manifest_of(&root).contains(r#"android:usesCleartextTraffic="${usesCleartextTraffic}""#)
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn applying_it_twice_adds_one_attribute() {
    // It runs on every Android build, because `gen/android` is regenerated
    // rather than reviewed and a patch that only ran once would be gone the
    // first time somebody deleted it.
    let root = generated("twice", TAURI_MANIFEST);
    crate::run::allow_loopback_cleartext(&root).expect("a first run");
    let once = manifest_of(&root);

    crate::run::allow_loopback_cleartext(&root).expect("a second run");
    assert_eq!(
        manifest_of(&root),
        once,
        "the second run changed the manifest"
    );
    assert_eq!(once.matches("networkSecurityConfig").count(), 1);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_manifest_this_tool_does_not_recognise_is_reported_rather_than_guessed_at() {
    // A future Tauri that stops writing the anchor attribute must not be
    // silently half-patched: the symptom would be a blank screen in release
    // and nothing to explain it.
    let root = generated(
        "unknown",
        "<?xml version=\"1.0\"?>\n<manifest><application /></manifest>\n",
    );

    let error = crate::run::allow_loopback_cleartext(&root).expect_err("no anchor");
    assert!(
        error.to_string().contains("networkSecurityConfig"),
        "the error does not say what to add by hand: {error}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

// ------------------------------------------------------- release signing

/// Tauri 2's generated Gradle file, in the shape the signing patch anchors on.
const TAURI_GRADLE: &str = r#"import java.util.Properties

plugins {
    id("com.android.application")
}

android {
    compileSdk = 36
    buildTypes {
        getByName("debug") {
            isDebuggable = true
        }
        getByName("release") {
            isMinifyEnabled = true
        }
    }
}
"#;

/// The four signing variables, set for one test and removed after.
///
/// This owns the lock rather than handing it back beside the value, and that
/// is not a style choice. `let (_signing, _guard) = …` drops in *reverse*
/// declaration order — the guard first — so the lock was released while the
/// variables were still set, and whichever test took it next read them. That
/// made the suite fail about one run in three, in a different test each time.
/// One value owning both makes the order impossible to get wrong.
struct Signing(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

static SIGNING_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const SIGNING_VARS: [&str; 4] = [
    "RAHTI_NATIVE_ANDROID_KEYSTORE",
    "RAHTI_NATIVE_ANDROID_KEYSTORE_PASSWORD",
    "RAHTI_NATIVE_ANDROID_KEY_ALIAS",
    "RAHTI_NATIVE_ANDROID_KEY_PASSWORD",
];

impl Signing {
    /// All four set, for a fully configured key.
    fn set(keystore: &str) -> Self {
        let held = Signing::none();
        // SAFETY: `held` owns the lock, which is this binary's whole access
        // to these four, and holds it until it drops.
        unsafe {
            std::env::set_var("RAHTI_NATIVE_ANDROID_KEYSTORE", keystore);
            std::env::set_var("RAHTI_NATIVE_ANDROID_KEYSTORE_PASSWORD", "storepass");
            std::env::set_var("RAHTI_NATIVE_ANDROID_KEY_ALIAS", "release");
            std::env::set_var("RAHTI_NATIVE_ANDROID_KEY_PASSWORD", "keypass");
        }
        held
    }

    /// The lock, with none of the four set.
    fn none() -> Self {
        let guard = SIGNING_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: as above.
        unsafe {
            for name in SIGNING_VARS {
                std::env::remove_var(name);
            }
        }
        Signing(guard)
    }
}

impl Drop for Signing {
    fn drop(&mut self) {
        // SAFETY: this value still holds the lock; it is released after.
        unsafe {
            for name in SIGNING_VARS {
                std::env::remove_var(name);
            }
        }
    }
}

fn with_gradle(label: &str) -> std::path::PathBuf {
    let root = generated(label, TAURI_MANIFEST);
    std::fs::write(root.join("app/build.gradle.kts"), TAURI_GRADLE).unwrap();
    root
}

#[test]
fn a_release_key_reaches_the_gradle_project() {
    // The bug this exists for: Tauri reads no signing key from the environment
    // on Android, so passing `TAURI_ANDROID_KEYSTORE` does nothing and the
    // build succeeds with an *unsigned* release — which Google Play refuses
    // and which will not install.
    let root = with_gradle("signing");
    let _signing = Signing::set("C:\\keys\\release.jks");

    crate::run::configure_signing(&root).expect("the signing patch applies");

    let gradle = std::fs::read_to_string(root.join("app/build.gradle.kts")).unwrap();
    assert!(gradle.contains("signingConfigs {"), "{gradle}");
    assert!(
        gradle.contains("signingConfig = signingConfigs.getByName(\"release\")"),
        "{gradle}"
    );
    // On the release build type, and not on debug — a debug build is signed
    // with Android's own key and must not need this one.
    let release_at = gradle.find("getByName(\"release\")").unwrap();
    let applied_at = gradle.find("signingConfig = signingConfigs").unwrap();
    assert!(applied_at > release_at, "{gradle}");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_key_details_go_only_to_the_ignored_properties_file() {
    let root = with_gradle("signing-secrets");
    let _signing = Signing::set("C:\\keys\\release.jks");

    crate::run::configure_signing(&root).expect("the signing patch applies");

    let properties =
        std::fs::read_to_string(root.join("keystore.properties")).expect("the properties file");
    assert!(properties.contains("keyAlias=release"), "{properties}");
    assert!(
        properties.contains("storePassword=storepass"),
        "{properties}"
    );
    // A Windows path is written with forward slashes: Gradle reads this as a
    // Java properties file, where a backslash escapes the next character.
    assert!(
        properties.contains("storeFile=C:/keys/release.jks"),
        "{properties}"
    );

    // And nowhere else.
    let gradle = std::fs::read_to_string(root.join("app/build.gradle.kts")).unwrap();
    for secret in ["storepass", "keypass", "release.jks"] {
        assert!(
            !gradle.contains(secret),
            "the Gradle file carries `{secret}`, which is committed"
        );
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn applying_the_signing_patch_twice_wires_it_once() {
    let root = with_gradle("signing-twice");
    let _signing = Signing::set("C:\\keys\\release.jks");

    crate::run::configure_signing(&root).expect("a first run");
    let once = std::fs::read_to_string(root.join("app/build.gradle.kts")).unwrap();
    crate::run::configure_signing(&root).expect("a second run");
    let twice = std::fs::read_to_string(root.join("app/build.gradle.kts")).unwrap();

    assert_eq!(once, twice);
    assert_eq!(once.matches("signingConfigs {").count(), 1);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn no_keystore_configured_changes_nothing() {
    // A debug build is signed with Android's debug key and needs none of this.
    let root = with_gradle("signing-absent");
    let _signing = Signing::none();

    crate::run::configure_signing(&root).expect("nothing to do");

    assert_eq!(
        std::fs::read_to_string(root.join("app/build.gradle.kts")).unwrap(),
        TAURI_GRADLE
    );
    assert!(!root.join("keystore.properties").exists());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_half_configured_key_is_refused_rather_than_producing_an_unsigned_release() {
    let root = with_gradle("signing-partial");
    // Nothing set, then one of the four — a keystore with no password and no
    // alias, which is the shape that would otherwise produce a release nobody
    // can install.
    let _signing = Signing::none();
    // SAFETY: `_signing` holds the lock for the whole test.
    unsafe {
        std::env::set_var("RAHTI_NATIVE_ANDROID_KEYSTORE", "C:\\keys\\release.jks");
    }

    let error = crate::run::configure_signing(&root).expect_err("a half-configured key");
    assert!(error.to_string().contains("unsigned"), "{error}");

    let _ = std::fs::remove_dir_all(&root);
}

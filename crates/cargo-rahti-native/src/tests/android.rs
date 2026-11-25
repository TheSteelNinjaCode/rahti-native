//! The Android project patches `cargo rahti native` applies after Tauri has
//! generated it.

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
    /// All four set, for a fully configured key with these passwords.
    fn with(keystore: &std::path::Path, store_password: &str, key_password: &str) -> Self {
        let held = Signing::none();
        // SAFETY: `held` owns the lock, which is this binary's whole access
        // to these four, and holds it until it drops.
        unsafe {
            std::env::set_var("RAHTI_NATIVE_ANDROID_KEYSTORE", keystore);
            std::env::set_var("RAHTI_NATIVE_ANDROID_KEYSTORE_PASSWORD", store_password);
            std::env::set_var("RAHTI_NATIVE_ANDROID_KEY_ALIAS", "release");
            std::env::set_var("RAHTI_NATIVE_ANDROID_KEY_PASSWORD", key_password);
        }
        held
    }

    /// A key that exists, at `root/keys/release.jks`.
    fn set(root: &std::path::Path) -> Self {
        Signing::with(&keystore_in(root), "storepass", "keypass")
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

/// A keystore file that exists. Its contents are Gradle's business, not ours.
fn keystore_in(root: &std::path::Path) -> std::path::PathBuf {
    let keystore = root.join("keys").join("release.jks");
    std::fs::create_dir_all(keystore.parent().unwrap()).unwrap();
    std::fs::write(&keystore, b"a keystore").unwrap();
    keystore
}

fn with_gradle(label: &str) -> std::path::PathBuf {
    let root = generated(label, TAURI_MANIFEST);
    std::fs::write(root.join("app/build.gradle.kts"), TAURI_GRADLE).unwrap();
    root
}

fn gradle_of(root: &std::path::Path) -> String {
    std::fs::read_to_string(root.join("app/build.gradle.kts")).unwrap()
}

/// A Java properties file read the way `java.util.Properties.load` reads one,
/// for the escapes this tool writes: enough to prove a value survives.
fn read_properties(text: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for line in text.lines() {
        let mut key = String::new();
        let mut value = String::new();
        let mut in_value = false;
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            let decoded = if c == '\\' {
                match chars.next() {
                    Some('t') => '\t',
                    Some('n') => '\n',
                    Some('r') => '\r',
                    Some('f') => '\u{0c}',
                    Some('u') => {
                        let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                        let unit = u16::from_str_radix(&hex, 16).unwrap();
                        // A surrogate pair is two `\u` escapes in a row.
                        if (0xD800..0xDC00).contains(&unit) {
                            assert_eq!(chars.next(), Some('\\'));
                            assert_eq!(chars.next(), Some('u'));
                            let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                            let low = u16::from_str_radix(&hex, 16).unwrap();
                            char::decode_utf16([unit, low]).next().unwrap().unwrap()
                        } else {
                            char::from_u32(unit as u32).unwrap()
                        }
                    }
                    Some(other) => other,
                    None => break,
                }
            } else if c == '=' && !in_value {
                in_value = true;
                continue;
            } else {
                c
            };
            if in_value {
                value.push(decoded);
            } else {
                key.push(decoded);
            }
        }
        map.insert(key, value);
    }
    map
}

#[test]
fn a_release_key_reaches_the_gradle_project() {
    // The bug this exists for: Tauri reads no signing key from the environment
    // on Android, so passing `TAURI_ANDROID_KEYSTORE` does nothing and the
    // build succeeds with an *unsigned* release — which Google Play refuses
    // and which will not install.
    let root = with_gradle("signing");
    let _signing = Signing::set(&root);

    crate::run::configure_signing(&root).expect("the signing patch applies");

    let gradle = gradle_of(&root);
    assert!(gradle.contains("signingConfigs {"), "{gradle}");
    assert!(
        gradle.contains("signingConfigs.findByName(\"release\")?.let { signingConfig = it }"),
        "{gradle}"
    );
    // On the release build type, and not on debug — a debug build is signed
    // with Android's own key and must not need this one.
    let release_at = gradle.find("getByName(\"release\")").unwrap();
    let applied_at = gradle.find("signingConfig = it").unwrap();
    assert!(applied_at > release_at, "{gradle}");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_key_details_go_only_to_the_ignored_properties_file() {
    let root = with_gradle("signing-secrets");
    let _signing = Signing::set(&root);

    crate::run::configure_signing(&root).expect("the signing patch applies");

    let properties =
        std::fs::read_to_string(root.join("keystore.properties")).expect("the properties file");
    assert!(properties.contains("keyAlias=release"), "{properties}");
    assert!(
        properties.contains("storePassword=storepass"),
        "{properties}"
    );
    // A Windows path is written with forward slashes and its drive colon
    // escaped: Gradle reads this as a Java properties file.
    let store_file = read_properties(&properties)["storeFile"].clone();
    assert!(!store_file.contains('\\'), "{properties}");
    assert!(store_file.ends_with("keys/release.jks"), "{properties}");

    // And nowhere else.
    let gradle = gradle_of(&root);
    for secret in ["storepass", "keypass", "release.jks"] {
        assert!(
            !gradle.contains(secret),
            "the Gradle file carries `{secret}`, which is committed"
        );
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_password_with_properties_syntax_in_it_reaches_gradle_unchanged() {
    // The bug this exists for: the passwords were written into
    // keystore.properties as they were, and that file is a Java properties
    // file — a backslash escapes, `=` and `:` end a key, `#` and `!` start a
    // comment, and it is read as ISO-8859-1. A password such as `Test#pass\1=x`
    // reached Gradle as `Test#pass1=x`, and the build stopped at packaging with
    // "keystore password was incorrect".
    let root = with_gradle("signing-escapes");
    let store_password = r"Test#pass\1=x:!";
    let key_password = " lead and ñ and 🔑\ttab";
    let _signing = Signing::with(&keystore_in(&root), store_password, key_password);

    crate::run::configure_signing(&root).expect("the signing patch applies");

    let text = std::fs::read_to_string(root.join("keystore.properties")).unwrap();
    assert!(
        text.is_ascii(),
        "read as ISO-8859-1, so it has to be ASCII: {text}"
    );
    let read = read_properties(&text);
    assert_eq!(read["storePassword"], store_password, "{text}");
    assert_eq!(read["password"], key_password, "{text}");
    assert_eq!(read["keyAlias"], "release", "{text}");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn applying_the_signing_patch_twice_wires_it_once() {
    let root = with_gradle("signing-twice");
    let _signing = Signing::set(&root);

    crate::run::configure_signing(&root).expect("a first run");
    let once = gradle_of(&root);
    crate::run::configure_signing(&root).expect("a second run");
    let twice = gradle_of(&root);

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

    assert_eq!(gradle_of(&root), TAURI_GRADLE);
    assert!(!root.join("keystore.properties").exists());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_key_from_an_earlier_build_does_not_outlive_its_variables() {
    // The bug this exists for: keystore.properties stayed after the variables
    // were gone, so the next release was signed with whatever key an earlier
    // build had used, and that key's passwords stayed on disk indefinitely.
    let root = with_gradle("signing-stale");
    {
        let _signing = Signing::set(&root);
        crate::run::configure_signing(&root).expect("a signed build");
    }
    assert!(root.join("keystore.properties").exists());

    let _signing = Signing::none();
    crate::run::configure_signing(&root).expect("an unsigned build");
    assert!(!root.join("keystore.properties").exists());

    // And a Gradle file without the properties file still configures: the
    // signing configuration is conditional, so a debug build is not broken by
    // a release key that is not there.
    let gradle = gradle_of(&root);
    assert!(
        gradle.contains("if (rahtiKeystore.getProperty(\"storeFile\") != null)"),
        "{gradle}"
    );
    assert!(!gradle.contains("as String"), "{gradle}");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_project_patched_by_an_earlier_version_is_upgraded() {
    // A Gradle file the unconditional patch wrote: `rahtiKeystore["keyAlias"]
    // as String` throws at configuration time when the file is absent, before
    // any task, so every build of that project failed — debug included.
    let legacy = TAURI_GRADLE
        .replacen(
            "    buildTypes {",
            r#"    val rahtiKeystore = Properties().apply {
        val f = rootProject.file("keystore.properties")
        if (f.exists()) { f.inputStream().use { load(it) } }
    }
    signingConfigs {
        create("release") {
            keyAlias = rahtiKeystore["keyAlias"] as String
            keyPassword = rahtiKeystore["password"] as String
            storeFile = file(rahtiKeystore["storeFile"] as String)
            storePassword = rahtiKeystore["storePassword"] as String
        }
    }
    buildTypes {"#,
            1,
        )
        .replacen(
            "        getByName(\"release\") {",
            "        getByName(\"release\") {\n            signingConfig = signingConfigs.getByName(\"release\")",
            1,
        );

    let upgraded = crate::run::with_signing(&legacy).expect("a recognised shape");
    assert!(!upgraded.contains("as String"), "{upgraded}");
    assert!(
        !upgraded.contains("signingConfig = signingConfigs.getByName"),
        "{upgraded}"
    );
    assert_eq!(upgraded.matches("signingConfigs {").count(), 1);
    assert_eq!(
        crate::run::with_signing(&upgraded).as_deref(),
        Some(upgraded.as_str())
    );
}

#[test]
fn a_gradle_file_with_windows_line_endings_is_still_recognised() {
    let crlf = TAURI_GRADLE.replace('\n', "\r\n");
    let patched = crate::run::with_signing(&crlf).expect("a recognised shape");
    assert!(patched.contains("signingConfigs {"));
    assert!(
        !patched.replace("\r\n", "").contains('\n'),
        "line endings were mixed"
    );
}

#[test]
fn a_keystore_that_is_not_there_is_refused_before_the_build() {
    // Otherwise every ABI compiles first and Gradle says so at the very end.
    let root = with_gradle("signing-missing-file");
    let _signing = Signing::with(&root.join("nowhere.jks"), "storepass", "keypass");

    let error = crate::run::configure_signing(&root).expect_err("no keystore file");
    assert!(error.to_string().contains("names no keystore"), "{error}");

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
        std::env::set_var("RAHTI_NATIVE_ANDROID_KEYSTORE", keystore_in(&root));
    }

    let error = crate::run::configure_signing(&root).expect_err("a half-configured key");
    assert!(error.to_string().contains("unsigned"), "{error}");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_unsigned_release_package_is_reported() {
    // Gradle names it `…-unsigned.apk` and reports success.
    let build = |debug| crate::args::Build {
        target: rahti_native::Platform::Android,
        format: None,
        debug,
        abi: None,
        device: None,
    };
    let unsigned = [std::path::PathBuf::from(
        "out/app-universal-release-unsigned.apk",
    )];
    let signed = [std::path::PathBuf::from("out/app-universal-release.apk")];

    assert!(crate::run::unsigned_release(&build(false), &unsigned).is_some());
    assert!(crate::run::unsigned_release(&build(false), &signed).is_none());
    assert!(crate::run::unsigned_release(&build(true), &unsigned).is_none());
}

// -------------------------------------------------------- launcher icons

/// A generated Android project with the template's own launcher icons in it,
/// which is the state `cargo tauri android init` leaves behind.
fn with_template_icons(label: &str) -> (std::path::PathBuf, crate::project::Project) {
    let root = generated(label, TAURI_MANIFEST);

    for density in ["mdpi", "hdpi", "xhdpi", "xxhdpi", "xxxhdpi"] {
        let dir = root.join(format!("app/src/main/res/mipmap-{density}"));
        std::fs::create_dir_all(&dir).unwrap();
        // WebP, which is what cargo-mobile2's template actually ships.
        std::fs::write(dir.join("ic_launcher.webp"), b"the template's robot").unwrap();
    }

    // The project whose `native/icons/android` the icons are copied from.
    let project_root = root.join("project");
    std::fs::create_dir_all(project_root.join("src")).unwrap();
    std::fs::write(project_root.join("rahti.config.json"), "{}").unwrap();
    std::fs::write(
        project_root.join("Cargo.toml"),
        "[package]
name = \"app\"
version = \"1.0.0\"
",
    )
    .unwrap();

    let icons = project_root.join("native/icons");
    for icon in crate::icons::ANDROID {
        let target = icons.join(icon.path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, icon.bytes).unwrap();
    }

    let project = crate::project::Project::at(&project_root).expect("a project");
    (root, project)
}

#[test]
fn the_applications_launcher_icons_replace_the_templates() {
    // The bug this exists for: nothing in `tauri android init` copies
    // `native/icons/` into the generated project, so an Android build wore the
    // template's robot however many times you replaced `icons/icon.png`.
    let (root, project) = with_template_icons("icons");
    crate::run::install_android_icons(&project, &root).expect("the icons install");

    let res = root.join("app/src/main/res");
    for density in ["mdpi", "hdpi", "xhdpi", "xxhdpi", "xxxhdpi"] {
        let png = res.join(format!("mipmap-{density}/ic_launcher.png"));
        assert!(png.is_file(), "mipmap-{density} did not get an icon");
        assert_ne!(
            std::fs::read(&png).unwrap(),
            b"the template's robot".to_vec()
        );
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_templates_file_for_the_same_resource_name_is_removed() {
    // Android names a resource without its extension, so `ic_launcher.webp`
    // beside `ic_launcher.png` is two files claiming one name — `aapt2` fails
    // the build on it. Writing ours without removing theirs would trade a
    // wrong icon for no build at all.
    let (root, project) = with_template_icons("icons-clash");
    crate::run::install_android_icons(&project, &root).expect("the icons install");

    let res = root.join("app/src/main/res");
    for density in ["mdpi", "hdpi", "xhdpi", "xxhdpi", "xxxhdpi"] {
        assert!(
            !res.join(format!("mipmap-{density}/ic_launcher.webp"))
                .exists(),
            "mipmap-{density} still has the template's .webp beside ours"
        );
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_adaptive_icon_xml_and_its_background_are_installed() {
    let (root, project) = with_template_icons("icons-adaptive");
    crate::run::install_android_icons(&project, &root).expect("the icons install");

    let res = root.join("app/src/main/res");
    assert!(res.join("mipmap-anydpi-v26/ic_launcher.xml").is_file());
    assert!(res.join("values/ic_launcher_background.xml").is_file());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn installing_the_icons_twice_writes_them_once() {
    // It runs on every Android build; a second run must not rewrite files and
    // make Gradle think every resource changed.
    let (root, project) = with_template_icons("icons-twice");
    crate::run::install_android_icons(&project, &root).expect("a first run");

    let png = root.join("app/src/main/res/mipmap-mdpi/ic_launcher.png");
    let stamp = std::fs::metadata(&png).unwrap().modified().unwrap();

    crate::run::install_android_icons(&project, &root).expect("a second run");
    assert_eq!(
        std::fs::metadata(&png).unwrap().modified().unwrap(),
        stamp,
        "the second run rewrote an unchanged icon"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_project_with_no_icon_tree_is_left_alone() {
    // A project scaffolded before the icons shipped. The build still works; it
    // wears the template's icon, which is what it did before.
    let root = generated("icons-absent", TAURI_MANIFEST);
    let project_root = root.join("project");
    std::fs::create_dir_all(&project_root).unwrap();
    std::fs::write(project_root.join("rahti.config.json"), "{}").unwrap();
    std::fs::write(
        project_root.join("Cargo.toml"),
        "[package]
name = \"app\"
version = \"1.0.0\"
",
    )
    .unwrap();
    let project = crate::project::Project::at(&project_root).expect("a project");

    crate::run::install_android_icons(&project, &root).expect("nothing to do");

    let _ = std::fs::remove_dir_all(&root);
}

// ------------------------------------------------------------ product name

const TAURI_STRINGS: &str = r#"<resources>
    <string name="app_name">"Old Name"</string>
    <string name="main_activity_title">"Old Name"</string>
</resources>
"#;

fn with_strings(label: &str, strings: &str) -> std::path::PathBuf {
    let root = generated(label, TAURI_MANIFEST);
    let values = root.join("app/src/main/res/values");
    std::fs::create_dir_all(&values).unwrap();
    std::fs::write(values.join("strings.xml"), strings).unwrap();
    root
}

fn strings_of(root: &std::path::Path) -> String {
    std::fs::read_to_string(root.join("app/src/main/res/values/strings.xml")).unwrap()
}

#[test]
fn a_rename_reaches_the_android_launcher() {
    // Tauri writes this file once, so without this the Windows package takes
    // the new name and the phone keeps the old one.
    let root = with_strings("rename", TAURI_STRINGS);
    crate::run::sync_android_name(&root, "ilnaya").expect("the name applies");

    let strings = strings_of(&root);
    assert!(strings.contains(r#"<string name="app_name">"ilnaya"</string>"#));
    assert!(strings.contains(r#"<string name="main_activity_title">"ilnaya"</string>"#));
    assert!(!strings.contains("Old Name"));
}

#[test]
fn a_name_android_would_misread_is_escaped() {
    let root = with_strings("escape", TAURI_STRINGS);
    crate::run::sync_android_name(&root, r#"Tom & "Jerry's" <App>"#).expect("the name applies");

    let strings = strings_of(&root);
    assert!(
        strings.contains(r#""Tom &amp; \"Jerry's\" &lt;App>""#),
        "{strings}"
    );
}

#[test]
fn a_strings_file_this_tool_does_not_recognise_is_reported() {
    let root = with_strings("unknown", "<resources></resources>");
    let error = crate::run::sync_android_name(&root, "ilnaya").expect_err("no anchor");
    assert!(error.to_string().contains("app_name"), "{error}");
}

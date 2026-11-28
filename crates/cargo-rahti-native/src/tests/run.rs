use std::path::PathBuf;

use rahti_native::Platform;

use crate::project::Project;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "cargo-rahti-native-watch-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["src", "public"] {
            std::fs::create_dir_all(root.join(dir)).expect("a fixture directory");
        }
        for file in ["Cargo.toml", "Cargo.lock", "rahti.config.json", ".env"] {
            std::fs::write(root.join(file), "").expect("a fixture file");
        }
        Fixture(root)
    }

    fn project(&self) -> Project {
        Project {
            root: self.0.clone(),
            package: "fixture".to_string(),
            lib: "fixture".to_string(),
            version: "0.1.0".to_string(),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn windows_watches_application_code_but_leaves_public_to_rahti() {
    let fixture = Fixture::new();
    let watched = crate::run::application_watch_folders(&fixture.project(), Platform::Windows);

    assert!(watched.contains(&fixture.0.join("src")));
    assert!(watched.contains(&fixture.0.join("Cargo.toml")));
    assert!(watched.contains(&fixture.0.join("Cargo.lock")));
    assert!(watched.contains(&fixture.0.join("rahti.config.json")));
    assert!(watched.contains(&fixture.0.join(".env")));
    assert!(!watched.contains(&fixture.0.join("public")));
    assert!(
        !watched.contains(&fixture.0.join("build.rs")),
        "missing paths are ignored"
    );
}

#[test]
fn android_rebuilds_when_an_embedded_public_asset_changes() {
    let fixture = Fixture::new();
    let watched = crate::run::application_watch_folders(&fixture.project(), Platform::Android);

    assert!(watched.contains(&fixture.0.join("src")));
    assert!(watched.contains(&fixture.0.join("public")));
}

#[test]
fn artifact_discovery_uses_cargos_configured_output_directory() {
    let fixture = Fixture::new();
    let native = fixture.0.join("native");
    std::fs::create_dir_all(native.join("src")).unwrap();
    std::fs::create_dir_all(native.join(".cargo")).unwrap();
    std::fs::write(native.join("Cargo.toml"),
        "[package]\nname = \"artifact-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[workspace]\n").unwrap();
    std::fs::write(native.join("src/main.rs"), "fn main() {}\n").unwrap();
    let output = fixture.0.join("custom-output");
    let configured = output.to_string_lossy().replace('\\', "/");
    std::fs::write(
        native.join(".cargo/config.toml"),
        format!("[build]\ntarget-dir = \"{configured}\"\n"),
    )
    .unwrap();
    assert_eq!(crate::run::target_directory(&native, None), output);
    let overridden = fixture.0.join("env-output");
    assert_eq!(
        crate::run::target_directory(&native, Some(overridden.as_os_str())),
        overridden
    );
}

#[test]
fn only_a_device_that_can_take_a_command_is_forwarded_to() {
    let output = "List of devices attached\n\
                  emulator-5554\tdevice\n\
                  R58M123\tdevice product:x model:y\n\
                  ZX1G22\tunauthorized\n\
                  0123\toffline\n\n";
    assert_eq!(
        crate::run::attached_devices(output),
        ["emulator-5554", "R58M123"]
    );
    assert!(crate::run::attached_devices("List of devices attached\n\n").is_empty());
}

// --------------------------------------------------------------- artifacts

#[test]
fn an_android_build_looks_only_where_it_wrote() {
    use crate::args::{AndroidAbi, AndroidFormat};
    use crate::run::android_output_dirs;

    let outputs = std::path::Path::new("outputs");

    // The real bug: a universal build listed an older x86_64 APK beside its own.
    assert_eq!(
        android_output_dirs(outputs, None, Some(AndroidFormat::Apk), true),
        [(outputs.join("apk/universal/debug"), "apk")]
    );
    assert_eq!(
        android_output_dirs(
            outputs,
            Some(AndroidAbi::X86_64),
            Some(AndroidFormat::Apk),
            true
        ),
        [(outputs.join("apk/x86_64/debug"), "apk")]
    );
    assert_eq!(
        android_output_dirs(
            outputs,
            Some(AndroidAbi::Arm64),
            Some(AndroidFormat::Aab),
            false
        ),
        [(outputs.join("bundle/arm64Release"), "aab")]
    );
    assert_eq!(
        android_output_dirs(outputs, None, None, false),
        [
            (outputs.join("apk/universal/release"), "apk"),
            (outputs.join("bundle/universalRelease"), "aab"),
        ]
    );
}

#[test]
fn an_installer_from_before_this_build_is_not_this_builds() {
    use std::time::{Duration, SystemTime};

    let fixture = Fixture::new();
    let installer = fixture.0.join("app_0.1.0_x64.msi");
    std::fs::write(&installer, "").unwrap();

    let before = SystemTime::now() - Duration::from_secs(60);
    let after = SystemTime::now() + Duration::from_secs(60);

    assert!(crate::run::written_since(&installer, before));
    assert!(!crate::run::written_since(&installer, after));
    assert!(!crate::run::written_since(
        &fixture.0.join("absent"),
        before
    ));
}

#[test]
fn the_build_report_says_what_an_agent_would_otherwise_scrape() {
    use crate::args::{AndroidAbi, Build};

    let args = Build {
        target: Platform::Android,
        format: None,
        debug: true,
        abi: None,
        device: Some("emulator-5554".to_string()),
    };
    let warning = crate::doctor::Finding::warning("debug package", "Do not\n   pass it on.");
    let report = crate::run::build_report(
        &args,
        Some(AndroidAbi::X86_64),
        &[PathBuf::from("app-x86_64-debug.apk")],
        &[warning],
        true,
    );

    assert_eq!(report["target"], "android");
    assert_eq!(report["profile"], "debug");
    assert_eq!(report["abi"], "x86_64");
    assert_eq!(report["device"], "emulator-5554");
    assert_eq!(report["installed"], true);
    assert_eq!(report["artifacts"][0], "app-x86_64-debug.apk");
    assert_eq!(report["warnings"][0]["state"], "warning");
    assert_eq!(report["warnings"][0]["advice"], "Do not pass it on.");
}

// -------------------------------------------------------------- packages

#[test]
fn every_package_lands_in_one_directory_per_target_and_profile() {
    let fixture = Fixture::new();
    let project = fixture.project();
    assert_eq!(
        crate::run::packages_dir(&project, Platform::Android, true),
        fixture.0.join("native/packages/android/debug")
    );
    assert_eq!(
        crate::run::packages_dir(&project, Platform::Windows, false),
        fixture.0.join("native/packages/windows/release")
    );
}

#[test]
fn a_package_is_named_for_the_application_its_version_and_its_variant() {
    use crate::run::package_name;
    let android =
        |debug, file| package_name("ilnaya", "ilnaya", "0.1.0", Platform::Android, debug, file);
    assert_eq!(
        android(false, "app-universal-release.apk"),
        "ilnaya-0.1.0-universal.apk"
    );
    assert_eq!(
        android(false, "app-universal-release.aab"),
        "ilnaya-0.1.0-universal.aab"
    );
    assert_eq!(
        android(false, "app-arm64-release.apk"),
        "ilnaya-0.1.0-arm64.apk"
    );
    assert_eq!(
        android(true, "app-x86_64-debug.apk"),
        "ilnaya-0.1.0-x86_64-debug.apk"
    );
    assert_eq!(
        android(false, "app-universal-release-unsigned.apk"),
        "ilnaya-0.1.0-universal-unsigned.apk",
        "an unsigned package keeps saying so"
    );

    let windows = |debug, file| {
        package_name(
            "my-shop",
            "My Shop",
            "2.3.0",
            Platform::Windows,
            debug,
            file,
        )
    };
    assert_eq!(
        windows(false, "My Shop_2.3.0_x64-setup.exe"),
        "my-shop-2.3.0-x64-setup.exe"
    );
    assert_eq!(
        windows(false, "My Shop_2.3.0_x64_en-US.msi"),
        "my-shop-2.3.0-x64-en-US.msi"
    );
    assert_eq!(
        windows(true, "My Shop_2.3.0_x64-setup.exe"),
        "my-shop-2.3.0-x64-setup-debug.exe"
    );
    assert_eq!(
        windows(false, "something-else.msi"),
        "my-shop-2.3.0-something-else.msi",
        "an unfamiliar name is kept, not guessed at"
    );
}

#[test]
fn the_product_name_becomes_a_file_name() {
    assert_eq!(crate::run::package_slug("ilnaya", "x"), "ilnaya");
    assert_eq!(crate::run::package_slug("My  Shop!", "x"), "my-shop");
    assert_eq!(crate::run::package_slug("Ñandú", "fallback"), "and");
    assert_eq!(crate::run::package_slug("—", "fallback"), "fallback");
}

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

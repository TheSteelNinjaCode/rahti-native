//! `init`: that it produces a shell, that running it twice changes nothing,
//! and that it never takes back a file somebody edited.

use std::path::{Path, PathBuf};

use rahti_native::NativeConfig;

use crate::args::Init;
use crate::project::Project;

/// A throwaway project that looks enough like a scaffolded one.
struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        // A counter rather than randomness: these tests run in parallel
        // threads of one process, and two fixtures that landed on the same
        // directory would have one deleting the other's tree on drop. A
        // counter cannot collide; a small random name eventually does, and
        // the failure it produces is a test that fails once in fifty runs
        // with a message about the wrong thing.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let root =
            std::env::temp_dir().join(format!("cargo-rahti-native-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("a project");

        std::fs::write(root.join("rahti.config.json"), "{\"schema\":1}").unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"my-app\"\nversion = \"0.3.0\"\nedition = \"2024\"\n\n\
             [lib]\nname = \"my_app\"\npath = \"src/lib.rs\"\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/lib.rs"),
            "pub async fn initialize_application() {}\n",
        )
        .unwrap();
        std::fs::write(
            root.join(".env"),
            "# a comment\nAUTH_COOKIE_NAME=rahti_session_9f2c\nAUTH_SECRET=not-read-by-init\n",
        )
        .unwrap();

        Fixture(root)
    }

    fn project(&self) -> Project {
        Project::discover(&self.0).expect("a project")
    }

    fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.0.join(path))
            .unwrap_or_else(|e| panic!("{path}: {e}"))
            .replace("\r\n", "\n")
    }

    fn write(&self, path: &str, contents: &str) {
        std::fs::write(self.0.join(path), contents).unwrap();
    }

    fn exists(&self, path: &str) -> bool {
        self.0.join(path).exists()
    }

    fn config(&self) -> NativeConfig {
        NativeConfig::load(&self.0.join("rahti.native.json")).expect("a native configuration")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn first_run() -> Init {
    Init {
        identifier: Some("com.example.myapp".to_string()),
        targets: vec!["windows", "android"],
        ..Init::default()
    }
}

// ------------------------------------------------------------- the tree

#[test]
fn a_first_run_writes_a_shell_that_has_everything_a_build_needs() {
    let fixture = Fixture::new();
    crate::init::run(&fixture.project(), &first_run()).expect("a first run");

    for path in [
        "rahti.native.json",
        "rahti.native.schema.json",
        "native/Cargo.toml",
        "native/build.rs",
        "native/tauri.conf.json",
        "native/capabilities/default.json",
        "native/src/lib.rs",
        "native/src/main.rs",
        "native/dist/index.html",
        "native/.gitignore",
        "native/README.md",
        // Both bundlers refuse to run without these.
        "native/icons/32x32.png",
        "native/icons/128x128.png",
        "native/icons/icon.ico",
    ] {
        assert!(fixture.exists(path), "{path} was not written");
    }
}

#[test]
fn nothing_outside_native_and_the_two_config_files_is_touched() {
    let fixture = Fixture::new();
    let before = fixture.read("Cargo.toml");
    let env_before = fixture.read(".env");
    let lib_before = fixture.read("src/lib.rs");

    crate::init::run(&fixture.project(), &first_run()).expect("a first run");

    assert_eq!(fixture.read("Cargo.toml"), before);
    assert_eq!(fixture.read(".env"), env_before);
    assert_eq!(fixture.read("src/lib.rs"), lib_before);
}

#[test]
fn the_defaults_come_from_the_project() {
    let fixture = Fixture::new();
    crate::init::run(&fixture.project(), &first_run()).expect("a first run");

    let config = fixture.config();
    assert_eq!(config.product_name, "My App", "from the package name");
    assert_eq!(config.version, "0.3.0", "from the package version");
    assert_eq!(
        config.auth.cookie_name.as_deref(),
        Some("rahti_session_9f2c"),
        "the project's cookie name, so the package and the web app agree"
    );
}

#[test]
fn the_session_key_never_reaches_a_committed_file() {
    let fixture = Fixture::new();
    crate::init::run(&fixture.project(), &first_run()).expect("a first run");

    // The `.env` has one. Nothing `init` writes may carry it.
    for path in [
        "rahti.native.json",
        "native/tauri.conf.json",
        "native/src/lib.rs",
        "native/Cargo.toml",
    ] {
        let contents = fixture.read(path);
        assert!(
            !contents.contains("not-read-by-init"),
            "{path} carries AUTH_SECRET"
        );
        assert!(
            !contents.contains("AUTH_SECRET"),
            "{path} names AUTH_SECRET as a value to set"
        );
    }
}

#[test]
fn a_first_run_with_no_target_flags_means_both() {
    let fixture = Fixture::new();
    let args = Init {
        identifier: Some("com.example.myapp".to_string()),
        ..Init::default()
    };
    crate::init::run(&fixture.project(), &args).expect("a first run");

    assert_eq!(fixture.config().targets, vec!["windows", "android"]);
}

#[test]
fn a_first_run_without_an_identifier_says_what_is_missing() {
    let fixture = Fixture::new();
    let error = crate::init::run(&fixture.project(), &Init::default()).expect_err("no identifier");
    assert!(error.to_string().contains("--identifier"), "{error}");
    assert!(!fixture.exists("native/Cargo.toml"));
}

#[test]
fn an_identifier_android_would_refuse_is_refused_before_anything_is_written() {
    let fixture = Fixture::new();
    let args = Init {
        identifier: Some("com.example.my-app".to_string()),
        ..Init::default()
    };
    let error = crate::init::run(&fixture.project(), &args).expect_err("a hyphen");
    assert!(error.to_string().contains("hyphen"), "{error}");
    assert!(
        !fixture.exists("native"),
        "a shell was written for a configuration that cannot build"
    );
}

// ------------------------------------------------------------ idempotent

#[test]
fn running_it_twice_changes_nothing() {
    let fixture = Fixture::new();
    crate::init::run(&fixture.project(), &first_run()).expect("a first run");

    let before: Vec<(String, String)> = tree(&fixture.0);

    crate::init::run(&fixture.project(), &Init::default()).expect("a second run");

    assert_eq!(before, tree(&fixture.0), "a second run changed the tree");
}

#[test]
fn a_second_run_does_not_revert_the_configuration() {
    let fixture = Fixture::new();
    crate::init::run(&fixture.project(), &first_run()).expect("a first run");

    // Somebody edits the configuration, which is the file that is theirs to
    // edit, and re-runs to bring the shell into agreement with it.
    let edited = fixture.read("rahti.native.json").replace("1200", "900");
    fixture.write("rahti.native.json", &edited);

    crate::init::run(&fixture.project(), &Init::default()).expect("a second run");

    assert_eq!(fixture.config().window.width, 900);
    assert!(
        fixture.read("native/src/lib.rs").contains("900.0"),
        "the shell did not pick up the new window size"
    );
}

#[test]
fn a_target_can_be_added_later() {
    let fixture = Fixture::new();
    let args = Init {
        identifier: Some("com.example.myapp".to_string()),
        targets: vec!["windows"],
        ..Init::default()
    };
    crate::init::run(&fixture.project(), &args).expect("a first run");
    assert_eq!(fixture.config().targets, vec!["windows"]);

    let args = Init {
        targets: vec!["android"],
        ..Init::default()
    };
    crate::init::run(&fixture.project(), &args).expect("a second run");
    assert_eq!(fixture.config().targets, vec!["windows", "android"]);
}

// -------------------------------------------------------- non-destructive

#[test]
fn an_edited_file_is_left_alone_and_reported() {
    let fixture = Fixture::new();
    crate::init::run(&fixture.project(), &first_run()).expect("a first run");

    let mine = "// I added a command here.\n";
    fixture.write("native/src/lib.rs", mine);

    crate::init::run(&fixture.project(), &Init::default()).expect("a second run");

    assert_eq!(
        fixture.read("native/src/lib.rs"),
        mine,
        "somebody's work was overwritten"
    );
}

#[test]
fn an_edited_file_stays_edited_over_several_runs() {
    // The hash of a kept file is not updated, so the second, third and tenth
    // run all reach the same conclusion.
    let fixture = Fixture::new();
    crate::init::run(&fixture.project(), &first_run()).expect("a first run");

    let mine = "{ \"my\": \"configuration\" }\n";
    fixture.write("native/tauri.conf.json", mine);

    for _ in 0..3 {
        crate::init::run(&fixture.project(), &Init::default()).expect("a repeat run");
        assert_eq!(fixture.read("native/tauri.conf.json"), mine);
    }
}

#[test]
fn force_takes_an_edited_file_back() {
    let fixture = Fixture::new();
    crate::init::run(&fixture.project(), &first_run()).expect("a first run");

    fixture.write("native/src/lib.rs", "// mine\n");

    let args = Init {
        force: true,
        ..Init::default()
    };
    crate::init::run(&fixture.project(), &args).expect("a forced run");

    let restored = fixture.read("native/src/lib.rs");
    assert!(restored.contains("initialize_application"), "{restored}");
    assert!(!restored.contains("// mine"));
}

#[test]
fn an_icon_is_written_once_and_never_replaced() {
    // An icon is the first thing a project changes, and a hash check on a
    // file a designer exported would always say "edited".
    let fixture = Fixture::new();
    crate::init::run(&fixture.project(), &first_run()).expect("a first run");

    fixture.write("native/icons/32x32.png", "my icon");
    let args = Init {
        force: true,
        ..Init::default()
    };
    crate::init::run(&fixture.project(), &args).expect("a forced run");

    assert_eq!(fixture.read("native/icons/32x32.png"), "my icon");
}

// -------------------------------------------------------------- content

#[test]
fn the_shell_links_the_projects_own_library() {
    let fixture = Fixture::new();
    crate::init::run(&fixture.project(), &first_run()).expect("a first run");

    let manifest = fixture.read("native/Cargo.toml");
    assert!(
        manifest.contains("my-app = { path = \"..\" }"),
        "{manifest}"
    );

    let lib = fixture.read("native/src/lib.rs");
    assert!(lib.contains("my_app::initialize_application()"), "{lib}");

    let main = fixture.read("native/src/main.rs");
    assert!(main.contains("my_app_native_lib::run()"), "{main}");
}

#[test]
fn the_shell_binds_and_serves_before_it_opens_a_window() {
    // The startup race, asserted on the generated source: a WebView told to
    // load a port nothing is listening on shows a blank window and no error.
    let lib = {
        let fixture = Fixture::new();
        crate::init::run(&fixture.project(), &first_run()).expect("a first run");
        fixture.read("native/src/lib.rs")
    };

    // Inside `start`, which is what returns the running server.
    let start = lib.find("fn start(").expect("a start function");
    let start_body = &lib[start..];
    let bind = start_body.find("EmbeddedServer::bind").expect("a bind");
    let serve = start_body.find("server.serve(router)").expect("a serve");
    let ready = start_body
        .find("wait_until_ready")
        .expect("a readiness check");
    assert!(bind < serve, "the server served before it bound");
    assert!(serve < ready, "readiness was checked before serving");

    // And in the setup closure, `start` runs before the window is created.
    let setup = lib.find(".setup(").expect("a setup");
    let setup_body = &lib[setup..start];
    let started = setup_body
        .find("start(app.handle())")
        .expect("a start call");
    let window = setup_body.find("open_window(").expect("a window");
    assert!(
        started < window,
        "the window opened before the server started"
    );
}

#[test]
fn shutdown_signals_the_applications_rahti_runtime_first() {
    let fixture = Fixture::new();
    crate::init::run(&fixture.project(), &first_run()).expect("a first run");
    let lib = fixture.read("native/src/lib.rs");

    let stop = lib.find("fn stop(").expect("a stop function");
    let stop_body = &lib[stop..];
    let framework = stop_body
        .find("my_app::rahti::begin_shutdown()")
        .expect("the application's shutdown broadcast");
    let server = stop_body
        .find("server.shutdown(")
        .expect("the server shutdown");

    assert!(
        framework < server,
        "the server stopped before the application's long-lived responses"
    );
}

#[test]
fn the_loopback_gate_is_wired_when_the_configuration_asks_for_it() {
    let fixture = Fixture::new();
    crate::init::run(&fixture.project(), &first_run()).expect("a first run");
    let lib = fixture.read("native/src/lib.rs");
    assert!(lib.contains("const LOOPBACK_TOKEN: bool = true;"), "{lib}");
    // One call, so the shell names no axum type and cannot get the layer
    // order backwards.
    assert!(lib.contains("rahti_native::secure(application.router, LOOPBACK_TOKEN)"));

    // Turned off, and the layer is not in the router at all rather than
    // present and deciding it does not apply — `secure` reads the constant.
    let off = fixture
        .read("rahti.native.json")
        .replace("\"loopbackToken\": true", "\"loopbackToken\": false");
    fixture.write("rahti.native.json", &off);

    let args = Init {
        force: true,
        ..Init::default()
    };
    crate::init::run(&fixture.project(), &args).expect("a forced run");
    assert!(
        fixture
            .read("native/src/lib.rs")
            .contains("const LOOPBACK_TOKEN: bool = false;")
    );
}

#[test]
fn the_generated_capabilities_grant_only_what_the_commands_need() {
    let fixture = Fixture::new();
    crate::init::run(&fixture.project(), &first_run()).expect("a first run");

    let capabilities = fixture.read("native/capabilities/default.json");
    // The page is on a loopback origin, which Tauri treats as remote.
    assert!(
        capabilities.contains("http://127.0.0.1:*"),
        "{capabilities}"
    );

    for forbidden in ["shell:", "fs:allow", "fs:default", "process:"] {
        assert!(
            !capabilities.contains(forbidden),
            "the shell was granted `{forbidden}`"
        );
    }
}

#[test]
fn the_generated_gitignore_keeps_credentials_out_of_the_repository() {
    let fixture = Fixture::new();
    crate::init::run(&fixture.project(), &first_run()).expect("a first run");

    let ignored = fixture.read("native/.gitignore");
    for pattern in ["*.keystore", "*.jks", "*.pfx", "key.properties", "/gen"] {
        assert!(ignored.contains(pattern), "{pattern} is not ignored");
    }

    // And the development log. A `dev` run has native/ as its working
    // directory, so the application writes `.rahti/dev.log` there — where the
    // project's own root-anchored `/.rahti` rule does not reach it.
    assert!(
        ignored.contains("/.rahti"),
        "the development log would be committed: {ignored}"
    );
}

/// Every file under `root`, with its contents, for comparing two runs.
fn tree(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, out);
        } else {
            let name = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string()
                .replace('\\', "/");
            let contents = std::fs::read(&path)
                .map(|bytes| format!("{:x}", bytes.len()))
                .unwrap_or_default();
            out.push((name, contents));
        }
    }
}

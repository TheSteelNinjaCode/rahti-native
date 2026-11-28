use std::path::PathBuf;

use crate::progress::{self, Progress};
use crate::project::Project;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "cargo-rahti-native-progress-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("native")).expect("a fixture directory");
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

    fn report(&self) -> serde_json::Value {
        let text = std::fs::read_to_string(progress::report_path(&self.project()))
            .expect("a build report");
        serde_json::from_str(&text).expect("JSON")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A process that stays alive until its stdin closes.
fn idle_child() -> std::process::Child {
    let program = if cfg!(windows) { "cmd" } else { "cat" };
    std::process::Command::new(program)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("an idle process")
}

#[test]
fn a_build_says_it_is_running_before_it_has_done_anything() {
    let fixture = Fixture::new();
    let mut progress = Progress::start(&fixture.project(), "build --target android").unwrap();
    progress.phase("rust");

    let report = fixture.report();
    assert_eq!(report["state"], "running");
    assert_eq!(report["pid"], std::process::id());
    assert_eq!(report["phase"], "rust");
    assert_eq!(report["phases"][0]["running"], true);
    assert_eq!(report["command"], "build --target android");
    assert!(report["log"].as_str().unwrap().ends_with("build.log"));
}

#[test]
fn a_finished_build_records_its_phases_and_its_verdict() {
    let fixture = Fixture::new();
    let mut progress = Progress::start(&fixture.project(), "build").unwrap();
    progress.phase("prepare");
    progress.phase("rust");
    progress.merge(serde_json::json!({ "artifacts": ["app.apk"] }));
    let summary = progress.finish(None);

    let report = fixture.report();
    assert_eq!(report["state"], "succeeded");
    assert!(report.get("phase").is_none(), "no phase is current");
    assert_eq!(report["phases"][0]["name"], "prepare");
    assert_eq!(report["phases"][1]["name"], "rust");
    assert!(report["phases"][1].get("running").is_none());
    assert_eq!(report["artifacts"][0], "app.apk");
    assert!(report["seconds"].is_u64());
    assert!(summary.starts_with("Finished in "), "{summary}");
    assert!(summary.contains("prepare 0s, rust 0s"), "{summary}");

    let mut failing = Progress::start(&fixture.project(), "build").unwrap();
    let error = rahti_native::NativeError::new("tauri", "it broke");
    assert!(failing.finish(Some(&error)).starts_with("Failed in "));
    let report = fixture.report();
    assert_eq!(report["state"], "failed");
    assert!(report["error"].as_str().unwrap().contains("it broke"));
}

#[test]
fn a_second_build_is_refused_while_the_first_one_runs() {
    let fixture = Fixture::new();
    let mut child = idle_child();
    std::fs::create_dir_all(fixture.0.join("native/.rahti")).unwrap();
    std::fs::write(
        progress::report_path(&fixture.project()),
        serde_json::json!({
            "state": "running",
            "pid": child.id(),
            "started": progress::now(),
            "command": "build --target android --debug",
        })
        .to_string(),
    )
    .unwrap();

    let refused = Progress::start(&fixture.project(), "build")
        .err()
        .expect("refused");
    assert!(refused.to_string().contains("already running"), "{refused}");
    assert!(refused.to_string().contains("status --wait"), "{refused}");

    let _ = child.kill();
    let _ = child.wait();

    // Its process is gone, so it was interrupted, and a build may start.
    let current = progress::current(&progress::report_path(&fixture.project())).unwrap();
    assert_eq!(current["state"], "interrupted");
    assert!(Progress::start(&fixture.project(), "build").is_ok());
}

#[test]
fn durations_read_as_a_person_would_say_them() {
    assert_eq!(progress::human(0), "0s");
    assert_eq!(progress::human(48), "48s");
    assert_eq!(progress::human(161), "2m41s");
    assert_eq!(progress::human(3720), "1h02m");
}

#[test]
fn the_log_has_no_terminal_colour_codes() {
    let coloured = b"\x1b[1m\x1b[92m   Compiling\x1b[0m ilnaya v0.1.0\n";
    assert_eq!(
        progress::strip_ansi(coloured),
        b"   Compiling ilnaya v0.1.0\n".to_vec()
    );
}

#[test]
fn a_logged_command_is_shown_kept_and_read_line_by_line() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.0.join("native/.rahti")).unwrap();
    let log = progress::log_path(&fixture.project());

    let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = lines.clone();
    let mut command = std::process::Command::new("cargo");
    command.arg("--version");
    let status = progress::run_logged(command, &log, move |line| {
        seen.lock().unwrap().push(line.to_string());
    })
    .expect("cargo runs");

    assert!(status.success());
    let kept = std::fs::read_to_string(&log).unwrap();
    assert!(kept.starts_with("cargo "), "{kept}");
    assert!(
        lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.starts_with("cargo "))
    );
    assert_eq!(progress::tail(&log, 1).len(), 1);
}

// --------------------------------------------------------------- devices

#[test]
fn attached_devices_are_read_with_their_state_and_model() {
    let output = "* daemon started successfully\n\
                  List of devices attached\n\
                  emulator-5554          device product:sdk_gphone64 model:sdk_gphone64_x86_64 device:emu64xa transport_id:3\n\
                  R58M123ABC             unauthorized usb:1-1 transport_id:4\n\
                  \n";
    let devices = crate::devices::parse_attached(output);

    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0].serial, "emulator-5554");
    assert!(devices[0].ready() && devices[0].emulator());
    assert_eq!(devices[0].model.as_deref(), Some("sdk_gphone64_x86_64"));
    assert_eq!(devices[1].state, "unauthorized");
    assert!(!devices[1].ready() && !devices[1].emulator());
}

#[test]
fn avd_names_are_read_without_the_emulators_own_chatter() {
    let output =
        "INFO    | Storing crashdata in: C:\\tmp\\crashes\r\nMedium_Phone\r\nPixel_9_API_36\r\n";
    assert_eq!(
        crate::devices::parse_avds(output),
        vec!["Medium_Phone".to_string(), "Pixel_9_API_36".to_string()]
    );
}

//! `dev` and `build` — running the shell, and packaging it.
//!
//! Both delegate to Tauri's own CLI. That decision is argued in
//! [`crate::doctor`]; the short version is that the CLI has to match the
//! `tauri` version in `native/Cargo.toml`, which is application-owned, and two
//! versions that must agree and are released by two people is worse than one
//! prerequisite that is checked by name.
//!
//! What this module adds around it:
//!
//! - the prerequisites are checked *first*, so a missing NDK is a sentence
//!   rather than a Gradle stack trace;
//! - the working directory is `native/`, so the command works from anywhere in
//!   the project;
//! - signing is taken from the `RAHTI_NATIVE_*` variables and never from a
//!   committed file — on Windows by handing Tauri the variables it reads, and
//!   on Android by wiring the key into the generated Gradle project, because
//!   Tauri reads no signing key from the environment there;
//! - the artifacts are found, copied to `native/packages/<target>/<profile>/`
//!   under names that say what they are, and those paths printed, because
//!   "finished in 4m12s" is not an answer to "where is the installer";
//! - the build is described in `native/.rahti/build.json` from the moment it
//!   starts, with its full output in `build.log` (see [`crate::progress`]);
//! - `--device` takes a serial, an AVD or `emulator`, and boots an AVD that is
//!   not running (see [`crate::devices`]).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use rahti_native::{DatabaseMode, NativeConfig, NativeError, Platform};

use crate::args::{AndroidAbi, AndroidFormat, Build, Dev};
use crate::doctor;
use crate::progress::Progress;
use crate::project::Project;

pub fn dev(project: &Project, config: &NativeConfig, args: &Dev) -> Result<(), NativeError> {
    prepare(project, config, args.target, false)?;

    let mut command = doctor::cargo_tauri(match args.target {
        Platform::Android => &["android", "dev"],
        _ => &["dev"],
    });
    command.current_dir(project.native_dir());
    add_application_watch_folders(&mut command, project, args.target);

    if let Some(device) = &args.device {
        command.arg(device);
    }

    // In this order: the JDK has to suit the Gradle version the generated
    // project pins, so that project has to exist first.
    if args.target == Platform::Android {
        ensure_android_project(project, config, false)?;
        android_environment(&mut command, project)?;
        forward_database_port(project, config);
    }

    println!("  running {}", describe(&command));
    println!();
    execute(command, "dev")
}

/// Make a database on this machine reachable from every attached device.
///
/// `localhost` on a device is the device. `adb reverse` is the one route that
/// works on an emulator and on a phone alike. Best effort, because no device
/// may be attached yet, and an emulator has its own alias for this machine to
/// fall back on.
fn forward_database_port(project: &Project, config: &NativeConfig) {
    if config.database.mode != DatabaseMode::Remote {
        return;
    }
    let url = project.env_value("DATABASE_URL").unwrap_or_default();
    let Some(port) = rahti_native::loopback_database_port(&url) else {
        return;
    };
    let Ok(devices) = Command::new(doctor::adb()).arg("devices").output() else {
        return;
    };

    let forward = format!("tcp:{port}");
    for serial in attached_devices(&String::from_utf8_lossy(&devices.stdout)) {
        let forwarded = Command::new(doctor::adb())
            .args(["-s", serial, "reverse", &forward, &forward])
            .output()
            .is_ok_and(|output| output.status.success());
        if forwarded {
            println!("  database: forwarded port {port} to {serial}");
        } else {
            println!("  database: could not forward port {port} to {serial}");
        }
    }
}

/// The serials in `adb devices` output that are ready to take a command.
///
/// `offline` and `unauthorized` devices are listed too, and refuse one.
pub(crate) fn attached_devices(output: &str) -> Vec<&str> {
    output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let serial = columns.next()?;
            (columns.next() == Some("device")).then_some(serial)
        })
        .collect()
}

/// Teach Tauri about the application outside the shell's own workspace.
///
/// `native/` is deliberately its own Cargo workspace, so Tauri discovers no
/// path dependency to watch above it. Without these explicit paths, editing a
/// page or component leaves the running native application unchanged.
///
/// Windows does not watch `public/` here: the embedded Rahti server serves the
/// checkout directly and swaps CSS or reloads the WebView itself. Android
/// cannot read a host path, so a public-asset change has to rebuild, reinstall
/// and restart the debug application with a new embedded asset tree.
fn add_application_watch_folders(command: &mut Command, project: &Project, target: Platform) {
    for path in application_watch_folders(project, target) {
        command.arg("--additional-watch-folders").arg(path);
    }
}

pub(crate) fn application_watch_folders(project: &Project, target: Platform) -> Vec<PathBuf> {
    let mut candidates = vec![
        project.root.join("src"),
        project.root.join("Cargo.toml"),
        project.root.join("Cargo.lock"),
        project.root.join("build.rs"),
        project.root.join("rahti.config.json"),
        project.root.join(".env"),
    ];

    if target == Platform::Android {
        candidates.push(project.root.join("public"));
    }

    candidates.retain(|path| path.exists());
    candidates
}

pub fn build(project: &Project, config: &NativeConfig, args: &Build) -> Result<(), NativeError> {
    let progress = Progress::start(project, &invocation(args))?;
    let progress = Arc::new(Mutex::new(progress));
    let log = crate::progress::log_path(project);

    let result = build_phases(project, config, args, &progress, &log);

    let mut progress = progress.lock().unwrap_or_else(|e| e.into_inner());
    let summary = progress.finish(result.as_ref().err());
    // The last lines are the ones an agent keeps, so the failure's own words
    // and the verdict go there — not 4,000 lines up, inside Gradle's output.
    if result.is_err() {
        let tail = crate::progress::tail(&log, 25);
        if !tail.is_empty() {
            println!();
            println!("  Last lines of {}:", log.display());
            for line in tail {
                println!("    {line}");
            }
        }
    }
    println!();
    println!(
        "  Report: {}",
        crate::progress::report_path(project).display()
    );
    println!("  {summary}");
    result
}

/// The command as typed, for `status` and for a refused second build.
fn invocation(args: &Build) -> String {
    let mut line = format!("build --target {}", args.target.name());
    if let Some(format) = args.format {
        line.push_str(&format!(" --format {}", format.name()));
    }
    if args.debug {
        line.push_str(" --debug");
    }
    if let Some(abi) = args.abi {
        line.push_str(&format!(" --abi {}", abi.flavor()));
    }
    if let Some(device) = &args.device {
        line.push_str(&format!(" --device {device}"));
    }
    line
}

fn set_phase(progress: &Mutex<Progress>, name: &str) {
    if let Ok(mut progress) = progress.lock() {
        progress.phase(name);
    }
}

fn build_phases(
    project: &Project,
    config: &NativeConfig,
    args: &Build,
    progress: &Arc<Mutex<Progress>>,
    log: &Path,
) -> Result<(), NativeError> {
    let started = SystemTime::now();
    set_phase(progress, "prepare");
    let mut warnings = prepare(project, config, args.target, !args.debug)?;
    warnings.extend(debug_credentials(project, config, args));

    // A build that succeeds says nothing about what it could not do, so what
    // `doctor` warned about is said here, ahead of Tauri's own output.
    if !warnings.is_empty() {
        doctor::report(&warnings);
        println!();
    }

    // Before the build: a serial that is not attached should cost a sentence,
    // not several minutes of compiling. An AVD named here is booted now, while
    // nothing else needs the machine.
    let serial = match &args.device {
        Some(device) => Some(crate::devices::resolve(project, device)?),
        None => None,
    };
    let abi = match (args.abi, &serial) {
        (Some(abi), _) => Some(abi),
        (None, Some(serial)) => Some(device_abi(serial)?),
        (None, None) => None,
    };
    // `--device` has to install something, and only an APK installs.
    let format = match &args.device {
        Some(_) => Some(AndroidFormat::Apk),
        None => args.format,
    };

    let mut signer = None;
    let mut command = match args.target {
        Platform::Android => {
            signer = Some(ensure_android_project(project, config, !args.debug)?);
            let mut command = doctor::cargo_tauri(&["android", "build"]);
            // `apk` installs on a device; `aab` is what Google Play takes and
            // is not installable as it is. Naming neither builds Tauri's
            // default, which is both.
            match format {
                Some(AndroidFormat::Apk) => {
                    command.args(["--apk"]);
                }
                Some(AndroidFormat::Aab) => {
                    command.args(["--aab"]);
                }
                None => {}
            }
            if args.debug {
                command.arg("--debug");
            }
            if let Some(abi) = abi {
                command.args(["--target", abi.tauri_target(), "--split-per-abi"]);
            }
            android_environment(&mut command, project)?;
            command
        }
        _ => {
            let mut command = doctor::cargo_tauri(&["build"]);
            if args.debug {
                command.arg("--debug");
            }
            windows_environment(&mut command);
            command
        }
    };
    command.current_dir(project.native_dir());

    if let Some(format) = format {
        println!("  format: {}", format.name());
    }
    println!("  running {}", describe(&command));
    println!("  log: {}", log.display());
    println!();

    // `rust` until Cargo's first `Finished`, then the bundler: on Android,
    // Gradle, which also calls Cargo again (a no-op by then) per ABI.
    set_phase(progress, "rust");
    let bundler = if args.target == Platform::Android {
        "gradle"
    } else {
        "bundle"
    };
    let watcher = progress.clone();
    let mut compiled = false;
    let status = crate::progress::run_logged(command, log, move |line| {
        if !compiled && line.contains("Finished `") {
            compiled = true;
            set_phase(&watcher, bundler);
        }
    })
    .map_err(|e| {
        NativeError::new(
            "tauri",
            format!(
                "could not run `cargo tauri`: {e}
                   Install it with: cargo install tauri-cli --version \"^2.8\" --locked"
            ),
        )
    })?;
    if !status.success() {
        return Err(NativeError::new(
            "tauri",
            format!(
                "`cargo tauri build` failed ({status}).
                   The output above is Tauri's own; all of it is in {}.",
                log.display()
            ),
        ));
    }

    let (built, searched) = find_artifacts(project, args, abi, format, started);
    let artifacts = publish_packages(project, config, args, &built, &searched)?;
    if let Some(finding) = unsigned_release(args, &artifacts) {
        println!();
        doctor::report(std::slice::from_ref(&finding));
        warnings.push(finding);
    }

    let mut report = build_report(args, abi, &artifacts, &warnings, false);
    report["device"] = serial.clone().into();
    report["packages"] = serde_json::json!(packages_dir(project, args.target, args.debug));
    if let Some(signer) = &signer {
        report["signing"] = signer.to_json();
    }
    if let Ok(mut progress) = progress.lock() {
        progress.merge(report);
    }

    if let Some(serial) = &serial {
        set_phase(progress, "install");
        install_on_device(serial, &artifacts, &config.identifier)?;
        if let Ok(mut progress) = progress.lock() {
            progress.merge(serde_json::json!({ "installed": true }));
        }
    }
    Ok(())
}

/// The one thing a successful debug build can do badly: be passed on.
fn debug_credentials(
    project: &Project,
    config: &NativeConfig,
    args: &Build,
) -> Option<doctor::Finding> {
    if !args.debug {
        return None;
    }
    let names: Vec<String> = config
        .development_env()
        .into_iter()
        .filter(|name| project.env_value(name).is_some())
        .collect();
    (!names.is_empty()).then(|| {
        doctor::Finding::warning(
            "debug package",
            format!(
                "This package carries {} from `.env`, and anything in a package can be read\n   \
                 back out of it. Use it on your own devices and do not pass it on.",
                names.join(", ")
            ),
        )
    })
}

// --------------------------------------------------------------- device

/// The ABI of an attached device.
fn device_abi(device: &str) -> Result<AndroidAbi, NativeError> {
    let attached = Command::new(doctor::adb())
        .arg("devices")
        .output()
        .map_err(|e| NativeError::new("android", format!("could not run adb: {e}")))?;
    let attached = String::from_utf8_lossy(&attached.stdout);
    let serials = attached_devices(&attached);

    if !serials.contains(&device) {
        return Err(NativeError::new(
            "android",
            format!(
                "`{device}` is not an attached device. `adb devices` lists: {}.",
                if serials.is_empty() {
                    "nothing ready".to_string()
                } else {
                    serials.join(", ")
                }
            ),
        ));
    }

    let output = Command::new(doctor::adb())
        .args(["-s", device, "shell", "getprop", "ro.product.cpu.abi"])
        .output()
        .map_err(|e| NativeError::new("android", format!("could not run adb: {e}")))?;
    let reported = String::from_utf8_lossy(&output.stdout);

    AndroidAbi::parse(&reported).ok_or_else(|| {
        NativeError::new(
            "android",
            format!(
                "`{device}` reports the ABI `{}`, which this tool does not know.\n  \
                 Name one with `--abi`.",
                reported.trim()
            ),
        )
    })
}

/// Install the package this build produced, and start it.
fn install_on_device(
    device: &str,
    artifacts: &[PathBuf],
    identifier: &str,
) -> Result<(), NativeError> {
    let package = artifacts
        .iter()
        .find(|path| path.extension().is_some_and(|e| e == "apk"))
        .ok_or_else(|| NativeError::new("android", "this build produced no APK to install."))?;

    println!();
    println!("  installing on {device}");
    adb_on(
        device,
        "install",
        &["install", "-r", &package.display().to_string()],
    )
    .map_err(|error| {
        if !error.to_string().contains("INSUFFICIENT_STORAGE") {
            return error;
        }
        NativeError::new(
            "android",
            format!(
                "{error}
                   A reinstall needs room for the old copy and the new one at once. Uninstalling                  first frees it, and deletes the application's data on {device}:
                     adb -s {device} uninstall {identifier}"
            ),
        )
    })?;

    let activity = format!("{identifier}/.MainActivity");
    adb_on(device, "start", &["shell", "am", "start", "-n", &activity])?;
    println!("  started {activity} on {device}");
    Ok(())
}

/// One `adb` command on one device, with its own words if it fails.
fn adb_on(device: &str, what: &str, args: &[&str]) -> Result<(), NativeError> {
    let output = Command::new(doctor::adb())
        .args(["-s", device])
        .args(args)
        .output()
        .map_err(|e| NativeError::new("android", format!("could not run adb: {e}")))?;

    if output.status.success() {
        return Ok(());
    }
    Err(NativeError::new(
        "android",
        format!(
            "`adb {what}` failed on {device}:\n  {}{}",
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    ))
}

// --------------------------------------------------------------- checks

/// Everything that has to be true before an expensive command starts.
///
/// Returns what did not stop the build but should be said.
fn prepare(
    project: &Project,
    config: &NativeConfig,
    target: Platform,
    release: bool,
) -> Result<Vec<doctor::Finding>, NativeError> {
    if !config.builds(target.name()) {
        return Err(NativeError::new(
            "config",
            format!(
                "this project does not build a {target} package.\n  \
                 `targets` in rahti.native.json is [{}].\n  \
                 Add it with: cargo rahti native init --{target}",
                config.targets.join(", ")
            ),
        ));
    }

    if !project.native_dir().join("Cargo.toml").is_file() {
        return Err(NativeError::at(
            "init",
            project.native_dir(),
            "there is no native shell here yet.\n  \
             Create one with: cargo rahti native init",
        ));
    }

    let findings = doctor::examine(project, config, &[target], release);
    if doctor::blocked(&findings) {
        println!("  Something needed for a {target} package is missing:");
        println!();
        doctor::report(&findings);
        println!();
        return Err(NativeError::new(
            "doctor",
            "stopped before building, because the build would have failed later and said \
             less.\n  \
             Run `cargo rahti native doctor` after fixing it.",
        ));
    }
    Ok(findings
        .into_iter()
        .filter(|finding| finding.state == doctor::State::Warning)
        .collect())
}

/// `cargo tauri android init` has not been run yet.
///
/// Generated rather than committed: the Android project it writes carries
/// absolute paths from the machine that created it, which is why the shell's
/// `.gitignore` excludes it.
fn ensure_android_project(
    project: &Project,
    config: &NativeConfig,
    release: bool,
) -> Result<Signer, NativeError> {
    let generated = project.native_dir().join("gen/android");

    if !generated.is_dir() {
        println!("  generating the Android project (once per checkout)…");
        let mut command = doctor::cargo_tauri(&["android", "init"]);
        command.current_dir(project.native_dir());
        android_environment(&mut command, project)?;
        execute(command, "android init")?;
    }

    // Every time, not only after `init`: the directory is regenerated rather
    // than reviewed, and a patch that only ran once would be gone the first
    // time somebody deleted it.
    allow_loopback_cleartext(&generated)?;
    install_android_icons(project, &generated)?;
    keep_clear_of_system_bars(&generated)?;
    // After the insets: an older activity has no `onCreate` until they add it.
    crate::launch_screen::install(&generated, &config.android.splash)?;
    crate::downloads::install(&generated)?;

    // The configured key wins. A release without one signs with this
    // machine's key rather than coming out unsigned, which Android refuses to
    // install. A debug build is signed with Android's debug key.
    let (key, signer) = match signing_key()? {
        Some(key) => (Some(key), Signer::Environment),
        None if release => {
            let key = crate::release_key::machine_key(project, config)?;
            let dir = crate::release_key::key_dir(&config.identifier).unwrap_or_default();
            (Some(key), Signer::Machine(dir))
        }
        None => (None, Signer::Debug),
    };
    configure_signing_with(&generated, key.as_ref())?;

    sync_android_name(&generated, &config.product_name)?;

    Ok(signer)
}

/// What signed an Android package, for the build report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Signer {
    /// The key `RAHTI_NATIVE_ANDROID_*` named.
    Environment,
    /// This machine's key for the application, in this directory.
    Machine(PathBuf),
    /// Android's debug key.
    Debug,
}

impl Signer {
    pub(crate) fn to_json(&self) -> serde_json::Value {
        match self {
            Signer::Environment => serde_json::json!({ "key": "environment" }),
            Signer::Machine(dir) => serde_json::json!({ "key": "machine", "dir": dir }),
            Signer::Debug => serde_json::json!({ "key": "debug" }),
        }
    }
}

/// Give the generated Android project the current product name.
///
/// `cargo tauri android init` writes the name into `res/values/strings.xml`
/// once, and nothing reads `tauri.conf.json` for it afterwards. So after a
/// rename the Windows package has the new name and the Android launcher keeps
/// the old one, with nothing in the build to say so.
pub(crate) fn sync_android_name(generated: &Path, name: &str) -> Result<(), NativeError> {
    let path = generated.join("app/src/main/res/values/strings.xml");
    let strings =
        std::fs::read_to_string(&path).map_err(|e| NativeError::io("android", &path, e))?;

    let mut patched = strings.clone();
    for resource in ["app_name", "main_activity_title"] {
        patched = with_android_string(&patched, resource, name).ok_or_else(|| {
            NativeError::at(
                "android",
                &path,
                format!(
                    "this file has no `{resource}` string, so `cargo rahti native` cannot give \
                     the application its name.\n  \
                     Add it by hand, or delete native/gen/android to generate it again."
                ),
            )
        })?;
    }

    if patched != strings {
        std::fs::write(&path, patched).map_err(|e| NativeError::io("android", &path, e))?;
        println!("  android: application name is now {name}");
    }
    Ok(())
}

/// `strings` with one resource's value replaced, or `None` if it is not there.
fn with_android_string(strings: &str, resource: &str, value: &str) -> Option<String> {
    let open = format!("<string name=\"{resource}\">");
    let start = strings.find(&open)? + open.len();
    let end = start + strings[start..].find("</string>")?;

    // Quoted as Tauri quotes it, which is what makes an apostrophe legal.
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('&', "&amp;")
        .replace('<', "&lt;");
    Some(format!(
        "{}\"{escaped}\"{}",
        &strings[..start],
        &strings[end..]
    ))
}

/// Put the application's launcher icons into the generated Android project.
///
/// ## The bug this exists for
///
/// `cargo tauri android init` writes a project from `cargo-mobile2`'s template,
/// and that template ships its own `ic_launcher.webp` in every
/// `res/mipmap-*`. Nothing then copies `native/icons/` over them — so the icons
/// beside `tauri.conf.json` reach a Windows package and are ignored completely
/// by an Android one, and the installed application wears the template's robot.
/// It looks like a Tauri sample rather than the application somebody built,
/// and no amount of replacing `icons/icon.png` changes it.
///
/// So the icons are copied in, on every Android build, for the same reason the
/// other two patches are: `gen/android` is regenerated rather than reviewed.
///
/// ## Why the `.webp` has to go
///
/// Android resources are named without their extension, so
/// `ic_launcher.webp` and `ic_launcher.png` in one directory are two files
/// claiming one name — `aapt2` fails the build on the duplicate. Writing the
/// PNG without removing the template's WebP would trade a wrong icon for no
/// build at all.
pub(crate) fn install_android_icons(
    project: &Project,
    generated: &Path,
) -> Result<(), NativeError> {
    let source = project.native_dir().join("icons/android");
    if !source.is_dir() {
        // A project scaffolded before the icons were shipped. The build still
        // works; it wears the template's icon, which is what it did before.
        return Ok(());
    }

    let res = generated.join("app/src/main/res");
    let mut copied = 0usize;

    for entry in walk(&source)? {
        let relative = entry
            .strip_prefix(&source)
            .map_err(|_| NativeError::at("android", &entry, "an icon outside the icon tree"))?;
        let target = res.join(relative);

        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| NativeError::io("android", parent, e))?;
        }

        // The template's file for the same resource name, whatever it is
        // called. Removed before ours is written, not after.
        for extension in ["webp", "png", "xml"] {
            let rival = target.with_extension(extension);
            if rival != target && rival.exists() {
                std::fs::remove_file(&rival).map_err(|e| NativeError::io("android", &rival, e))?;
            }
        }

        let bytes = std::fs::read(&entry).map_err(|e| NativeError::io("android", &entry, e))?;
        if std::fs::read(&target).is_ok_and(|current| current == bytes) {
            continue;
        }
        std::fs::write(&target, bytes).map_err(|e| NativeError::io("android", &target, e))?;
        copied += 1;
    }

    if copied > 0 {
        println!("  android: installed {copied} launcher icon(s)");
    }
    Ok(())
}

/// Every file under `dir`, recursively.
fn walk(dir: &Path) -> Result<Vec<PathBuf>, NativeError> {
    let mut found = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| NativeError::io("android", dir, e))?;

    for entry in entries {
        let entry = entry.map_err(|e| NativeError::io("android", dir, e))?;
        let path = entry.path();
        if path.is_dir() {
            found.extend(walk(&path)?);
        } else {
            found.push(path);
        }
    }
    Ok(found)
}

/// Wire the release signing key into the generated Gradle project.
///
/// ## Why this is not just environment variables
///
/// Tauri does not read a signing key from the environment on Android. Its
/// documented flow is manual: you add a `signingConfigs` block to the generated
/// `app/build.gradle.kts` yourself and put the key's details in a
/// `keystore.properties` beside it. Passing `TAURI_ANDROID_KEYSTORE` and
/// hoping does nothing at all — the build succeeds and produces an *unsigned*
/// release, which Google Play refuses and which will not install.
///
/// So `cargo rahti native` does the wiring, from the `RAHTI_NATIVE_ANDROID_*`
/// variables, on every Android build. The generated Android project is
/// regenerated rather than reviewed, so this has to be reapplied the same way
/// the network-security configuration is.
///
/// ## Where the secrets go
///
/// Into `gen/android/keystore.properties`, which is inside the generated
/// directory the shell's `.gitignore` excludes, and which is additionally
/// named there. They are never written to `rahti.native.json`, never to
/// `tauri.conf.json`, and never printed.
///
/// With nothing configured, a `--debug` build is signed with Android's debug
/// key as usual and a release build is unsigned — and a `keystore.properties`
/// an earlier signed build left behind is removed, so that neither a stale key
/// nor its passwords outlive the variables that supplied them.
#[cfg(test)]
pub(crate) fn configure_signing(generated: &Path) -> Result<(), NativeError> {
    configure_signing_with(generated, signing_key()?.as_ref())
}

/// `configure_signing` with the key already chosen: the configured one, this
/// machine's (see `release_key`), or none for a debug build.
pub(crate) fn configure_signing_with(
    generated: &Path,
    key: Option<&SigningKey>,
) -> Result<(), NativeError> {
    let properties_path = generated.join("keystore.properties");

    let Some(key) = key else {
        if properties_path.exists() {
            std::fs::remove_file(&properties_path)
                .map_err(|e| NativeError::io("android", &properties_path, e))?;
        }
        return Ok(());
    };

    std::fs::write(&properties_path, key.properties())
        .map_err(|e| NativeError::io("android", &properties_path, e))?;

    let gradle_path = generated.join("app/build.gradle.kts");
    let gradle = std::fs::read_to_string(&gradle_path)
        .map_err(|e| NativeError::io("android", &gradle_path, e))?;

    let Some(patched) = with_signing(&gradle) else {
        return Err(NativeError::at(
            "android",
            &gradle_path,
            "this Gradle file is not one `cargo rahti native` recognises, so release signing \
             was not wired in.\n  \
             Add a `signingConfigs` block reading keystore.properties by hand, or delete \
             native/gen/android to generate it again; otherwise the release package will be \
             unsigned.",
        ));
    };

    if patched != gradle {
        std::fs::write(&gradle_path, patched)
            .map_err(|e| NativeError::io("android", &gradle_path, e))?;
        println!("  android: release signing wired into the Gradle project");
    }
    Ok(())
}

/// Where the patch puts the signing configuration, and where it applies it.
const CONFIG_ANCHOR: &str = "    buildTypes {";
const RELEASE_ANCHOR: &str = "        getByName(\"release\") {";

/// The signing configuration the patch writes.
///
/// Created only when `keystore.properties` names a key. The file is removed
/// whenever the variables are not set, and a Gradle file that demanded it —
/// `rahtiKeystore["keyAlias"] as String` — failed at configuration time, which
/// is before *any* task, so a debug build broke too.
const SIGNING_CONFIG: &str = r#"    val rahtiKeystore = Properties().apply {
        val f = rootProject.file("keystore.properties")
        if (f.exists()) { f.inputStream().use { load(it) } }
    }
    signingConfigs {
        if (rahtiKeystore.getProperty("storeFile") != null) {
            create("release") {
                keyAlias = rahtiKeystore.getProperty("keyAlias")
                keyPassword = rahtiKeystore.getProperty("password")
                storeFile = file(rahtiKeystore.getProperty("storeFile"))
                storePassword = rahtiKeystore.getProperty("storePassword")
            }
        }
    }
"#;
const APPLY_SIGNING: &str =
    "            signingConfigs.findByName(\"release\")?.let { signingConfig = it }";

/// What versions before the conditional configuration wrote, replaced in place
/// so that a project patched by one of them is upgraded rather than skipped.
const LEGACY_SIGNING_CONFIG: &str = r#"    val rahtiKeystore = Properties().apply {
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
"#;
const LEGACY_APPLY_SIGNING: &str =
    "            signingConfig = signingConfigs.getByName(\"release\")";

/// `gradle` with release signing wired in, or `None` if its shape is not one
/// this tool recognises. Idempotent.
pub(crate) fn with_signing(gradle: &str) -> Option<String> {
    // Gradle files written on Windows may carry CRLF line endings, and the
    // anchors are written with LF.
    let crlf = gradle.contains("\r\n");
    let text = gradle.replace("\r\n", "\n");

    let text = if text.contains(LEGACY_SIGNING_CONFIG) {
        text.replacen(LEGACY_SIGNING_CONFIG, SIGNING_CONFIG, 1)
            .replacen(LEGACY_APPLY_SIGNING, APPLY_SIGNING, 1)
    } else if text.contains(SIGNING_CONFIG) {
        text
    } else if text.contains("rahtiKeystore") {
        // Somebody's own edit of an earlier patch: theirs to keep.
        return None;
    } else {
        if !text.contains(CONFIG_ANCHOR) || !text.contains(RELEASE_ANCHOR) {
            return None;
        }
        text.replacen(
            CONFIG_ANCHOR,
            &format!("{SIGNING_CONFIG}{CONFIG_ANCHOR}"),
            1,
        )
        .replacen(
            RELEASE_ANCHOR,
            &format!("{RELEASE_ANCHOR}\n{APPLY_SIGNING}"),
            1,
        )
    };

    Some(if crlf {
        text.replace('\n', "\r\n")
    } else {
        text
    })
}

/// A release signing key, from the `RAHTI_NATIVE_ANDROID_*` variables.
pub(crate) struct SigningKey {
    keystore: PathBuf,
    store_password: String,
    alias: String,
    key_password: String,
}

/// Written by hand so that no password reaches a log or a panic message.
impl std::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SigningKey")
            .field("keystore", &self.keystore)
            .field("alias", &self.alias)
            .finish_non_exhaustive()
    }
}

impl SigningKey {
    pub(crate) fn new(
        keystore: PathBuf,
        store_password: String,
        alias: String,
        key_password: String,
    ) -> Self {
        SigningKey {
            keystore,
            store_password,
            alias,
            key_password,
        }
    }

    /// `keystore.properties`, which Gradle reads as a Java properties file.
    ///
    /// Every value is escaped: in that format a backslash starts an escape,
    /// `#` and `!` start a comment at the start of a line, `=` and `:` end a
    /// key, and the file is read as ISO-8859-1. A password with any of those in
    /// it, written as it was, reached Gradle as a different password, and the
    /// build failed with "keystore password was incorrect" for a password that
    /// was not.
    pub(crate) fn properties(&self) -> String {
        // Forward slashes, which Gradle accepts on Windows, keep the path
        // readable; the escaping would handle backslashes as well.
        let store_file = self.keystore.display().to_string().replace('\\', "/");
        format!(
            "storeFile={}\nstorePassword={}\nkeyAlias={}\npassword={}\n",
            escape_property(&store_file),
            escape_property(&self.store_password),
            escape_property(&self.alias),
            escape_property(&self.key_password),
        )
    }
}

/// The configured release key, `None` when none is configured, or why the
/// configuration cannot sign anything.
///
/// Checked here, before the build, rather than left to Gradle: a keystore that
/// does not exist or a missing alias otherwise costs a full compile of every
/// ABI before Gradle says so, and a relative path was resolved by Gradle
/// against `gen/android/app` rather than the directory the build ran from.
pub(crate) fn signing_key() -> Result<Option<SigningKey>, NativeError> {
    let Some(keystore) = env_value("RAHTI_NATIVE_ANDROID_KEYSTORE") else {
        return Ok(None);
    };

    // Passwords as they were given: a space is a character a password may
    // have, so only an entirely blank one counts as unset.
    let store_password = env_secret("RAHTI_NATIVE_ANDROID_KEYSTORE_PASSWORD");
    let alias = env_value("RAHTI_NATIVE_ANDROID_KEY_ALIAS");
    let (Some(store_password), Some(alias)) = (store_password, alias) else {
        return Err(NativeError::new(
            "android",
            "RAHTI_NATIVE_ANDROID_KEYSTORE is set, but the keystore password or the key alias \
             is not.\n  \
             RAHTI_NATIVE_ANDROID_KEYSTORE, RAHTI_NATIVE_ANDROID_KEYSTORE_PASSWORD and \
             RAHTI_NATIVE_ANDROID_KEY_ALIAS are all needed to sign a release \
             (RAHTI_NATIVE_ANDROID_KEY_PASSWORD too, when the key's password differs from the \
             keystore's); a half-configured key produces an unsigned package that does not \
             install.",
        ));
    };
    let key_password =
        env_secret("RAHTI_NATIVE_ANDROID_KEY_PASSWORD").unwrap_or_else(|| store_password.clone());

    let keystore = PathBuf::from(keystore);
    let keystore = if keystore.is_absolute() {
        keystore
    } else {
        std::env::current_dir()
            .map(|dir| dir.join(&keystore))
            .unwrap_or(keystore)
    };
    if !keystore.is_file() {
        return Err(NativeError::at(
            "android",
            &keystore,
            "RAHTI_NATIVE_ANDROID_KEYSTORE names no keystore file.\n  \
             Point it at the .jks or .keystore file that holds the release key.",
        ));
    }

    Ok(Some(SigningKey {
        keystore,
        store_password,
        alias,
        key_password,
    }))
}

/// One value escaped for a Java properties file. See [`SigningKey::properties`].
pub(crate) fn escape_property(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for (index, c) in value.chars().enumerate() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '=' | ':' | '#' | '!' => {
                escaped.push('\\');
                escaped.push(c);
            }
            // Leading whitespace is skipped by the reader unless escaped.
            ' ' if index == 0 => escaped.push_str("\\ "),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\u{0c}' => escaped.push_str("\\f"),
            ' '..='~' => escaped.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    escaped.push_str(&format!("\\u{unit:04X}"));
                }
            }
        }
    }
    escaped
}

/// A variable's value, trimmed, with blank counted as unset.
fn env_value(name: &str) -> Option<String> {
    let value = std::env::var(name).ok()?;
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// A secret's value exactly as given, with blank counted as unset.
fn env_secret(name: &str) -> Option<String> {
    let value = std::env::var(name).ok()?;
    (!value.trim().is_empty()).then_some(value)
}

/// A release that came out unsigned, said rather than left in a file name.
///
/// Gradle names an APK it could not sign `…-unsigned.apk` and reports success,
/// and Android refuses to install one. A build that produced only that has
/// succeeded badly, and the report says so.
pub(crate) fn unsigned_release(args: &Build, artifacts: &[PathBuf]) -> Option<doctor::Finding> {
    if args.debug || args.target != Platform::Android {
        return None;
    }
    let unsigned: Vec<String> = artifacts
        .iter()
        .filter_map(|path| path.file_name()?.to_str())
        .filter(|name| name.contains("-unsigned"))
        .map(str::to_string)
        .collect();
    (!unsigned.is_empty()).then(|| {
        doctor::Finding::warning(
            "release signing",
            format!(
                "{} is unsigned, and Android will not install it.\n   \
                 Set RAHTI_NATIVE_ANDROID_KEYSTORE, RAHTI_NATIVE_ANDROID_KEYSTORE_PASSWORD and\n   \
                 RAHTI_NATIVE_ANDROID_KEY_ALIAS in the environment of the build, or build with\n   \
                 `--debug` for a package that installs on your own devices.",
                unsigned.join(", ")
            ),
        )
    })
}

/// Marks the insets code this tool adds to `MainActivity.kt`.
const INSETS_MARKER: &str = "rahti-native: keep the page clear of the system bars";

/// What goes into `onCreate`, after `super.onCreate`.
const INSETS_CODE: &str = r#"    // rahti-native: keep the page clear of the system bars.
    // Written by `cargo rahti native` on every Android build.
    val content = findViewById<android.view.View>(android.R.id.content)
    androidx.core.view.ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(
        androidx.core.view.WindowInsetsCompat.Type.systemBars() or
          androidx.core.view.WindowInsetsCompat.Type.displayCutout() or
          androidx.core.view.WindowInsetsCompat.Type.ime()
      )
      view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      androidx.core.view.WindowInsetsCompat.CONSUMED
    }
"#;

/// Keep the WebView out from under the status bar, the navigation bar, a
/// display cutout and the keyboard.
///
/// ## The bug this exists for
///
/// Tauri's generated `MainActivity` calls `enableEdgeToEdge()`, and Android 15
/// enforces edge-to-edge for every application targeting API 35 or later
/// anyway. So the WebView is laid out across the whole screen, and a page's
/// header is drawn under the clock, the battery and the signal icons — where
/// a tap reaches the status bar, not the page. A button in a sticky header
/// looks fine and cannot be pressed.
///
/// A web page cannot fix it from CSS: Android System WebView does not report
/// the status bar through `env(safe-area-inset-top)` to an ordinary page. So
/// the activity pads its content view by the system bars' insets instead —
/// the documented way to handle edge-to-edge — and the page gets exactly the
/// area it can use, the keyboard included, which edge-to-edge also stopped
/// `adjustResize` from doing.
///
/// Idempotent, and it refuses an activity it does not recognise rather than
/// editing it by guesswork.
pub(crate) fn keep_clear_of_system_bars(generated: &Path) -> Result<(), NativeError> {
    let Some(path) = find_main_activity(&generated.join("app/src/main/java")) else {
        return Err(NativeError::at(
            "android",
            generated,
            "this Android project has no MainActivity.kt, so the page could not be kept \
             clear of the status bar.\n  \
             Delete native/gen/android to generate it again.",
        ));
    };
    let source =
        std::fs::read_to_string(&path).map_err(|e| NativeError::io("android", &path, e))?;

    let Some(patched) = with_insets(&source) else {
        return Err(NativeError::at(
            "android",
            &path,
            format!(
                "this MainActivity is not one `cargo rahti native` recognises, so the page was \
                 not kept clear of the status bar and a header under it cannot be tapped.\n  \
                 Add this to `onCreate`, after `super.onCreate(savedInstanceState)`:\n{INSETS_CODE}"
            ),
        ));
    };

    if patched != source {
        std::fs::write(&path, patched).map_err(|e| NativeError::io("android", &path, e))?;
        println!("  android: kept the page clear of the status and navigation bars");
    }
    Ok(())
}

/// `MainActivity.kt` with the insets code in it, or `None` for a shape this
/// tool does not recognise. Idempotent.
pub(crate) fn with_insets(source: &str) -> Option<String> {
    if source.contains(INSETS_MARKER) {
        return Some(source.to_string());
    }
    let crlf = source.contains("\r\n");
    let text = source.replace("\r\n", "\n");

    const SUPER: &str = "super.onCreate(savedInstanceState)\n";
    let patched = if text.matches(SUPER).count() == 1 {
        text.replacen(SUPER, &format!("{SUPER}{INSETS_CODE}"), 1)
    } else if !text.contains("fun onCreate") {
        // Older templates: a bare `class MainActivity : TauriActivity()`.
        let class = text.lines().find(|line| {
            line.trim_start()
                .starts_with("class MainActivity : TauriActivity()")
        })?;
        let rest = class
            .trim()
            .trim_start_matches("class MainActivity : TauriActivity()")
            .trim();
        if !(rest.is_empty() || rest == "{}") {
            return None;
        }
        let replacement = format!(
            "class MainActivity : TauriActivity() {{\n  \
             override fun onCreate(savedInstanceState: android.os.Bundle?) {{\n    \
             {SUPER}{INSETS_CODE}  }}\n}}"
        );
        text.replacen(class, &replacement, 1)
    } else {
        return None;
    };

    Some(if crlf {
        patched.replace('\n', "\r\n")
    } else {
        patched
    })
}

/// The one `MainActivity.kt` under `java/`, wherever the identifier put it.
pub(crate) fn find_main_activity(java: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = walk(java)
        .ok()?
        .into_iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name == "MainActivity.kt")
        })
        // Tauri's own generated sources live under `generated/` and are not
        // the application's activity.
        .filter(|path| !path.components().any(|c| c.as_os_str() == "generated"))
        .collect();
    (found.len() == 1).then(|| found.remove(0))
}

/// The file Android reads to decide whether cleartext HTTP is allowed.
const NETWORK_CONFIG: &str = "app/src/main/res/xml/rahti_network_security_config.xml";

/// Let the WebView reach the embedded server, and nothing else in cleartext.
///
/// ## The bug this exists for
///
/// Tauri's generated Android project sets
/// `android:usesCleartextTraffic="false"` on a release build — correct for a
/// Tauri application, which serves its pages through an asset protocol and
/// never speaks HTTP to itself.
///
/// A Rahti application does. Its whole design is the real Axum router on a
/// loopback socket, and the WebView reaches it over `http://127.0.0.1:<port>`.
/// With cleartext refused, a release APK installs, launches, and shows a blank
/// screen — the server is running, the WebView asked for the page, and Android
/// refused the request. A debug build works, because Tauri sets the flag `true`
/// there, so the failure appears only in the package that ships.
///
/// ## Why a network security config rather than the flag
///
/// Setting `usesCleartextTraffic="true"` would fix it and would also permit
/// cleartext to *every* host, which is a real downgrade in an application that
/// may well talk to an API over TLS. A network security config is per-domain:
/// the base configuration keeps cleartext refused, and one `domain-config`
/// permits it for the loopback addresses the embedded server binds. It also
/// takes precedence over the manifest flag, so the two cannot disagree.
///
/// Idempotent: the file is rewritten (it is this tool's, not the
/// application's) and the manifest attribute is added only when absent.
pub(crate) fn allow_loopback_cleartext(generated: &Path) -> Result<(), NativeError> {
    const CONFIG: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!-- Written by `cargo rahti native`. Do not edit: it is rewritten on every
     Android build.

     A Rahti application serves itself on http://127.0.0.1, so the WebView has
     to be allowed to reach it. Everything else stays as Android's default:
     cleartext refused. -->
<network-security-config>
    <base-config cleartextTrafficPermitted="false" />
    <domain-config cleartextTrafficPermitted="true">
        <domain includeSubdomains="false">127.0.0.1</domain>
        <domain includeSubdomains="false">localhost</domain>
    </domain-config>
</network-security-config>
"#;

    let config_path = generated.join(NETWORK_CONFIG);
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| NativeError::io("android", parent, e))?;
    }
    std::fs::write(&config_path, CONFIG)
        .map_err(|e| NativeError::io("android", &config_path, e))?;

    let manifest_path = generated.join("app/src/main/AndroidManifest.xml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .map_err(|e| NativeError::io("android", &manifest_path, e))?;

    if manifest.contains("android:networkSecurityConfig") {
        return Ok(());
    }

    // Anchored on the attribute Tauri writes, so a manifest this tool does not
    // recognise is reported rather than edited by guesswork.
    const ANCHOR: &str = "android:usesCleartextTraffic=";
    let Some(at) = manifest.find(ANCHOR) else {
        return Err(NativeError::at(
            "android",
            &manifest_path,
            "this Android manifest is not one `cargo rahti native` recognises, so the \
             loopback network-security configuration was not applied.\n  \
             Add this to its `<application>` element by hand, or the release package will \
             show a blank screen:\n    \
             android:networkSecurityConfig=\"@xml/rahti_network_security_config\"",
        ));
    };

    let patched = format!(
        "{}android:networkSecurityConfig=\"@xml/rahti_network_security_config\"\n        {}",
        &manifest[..at],
        &manifest[at..]
    );
    std::fs::write(&manifest_path, patched)
        .map_err(|e| NativeError::io("android", &manifest_path, e))?;

    println!("  android: allowed cleartext to the loopback server only");
    Ok(())
}

// ---------------------------------------------------------- environment

/// Point the Android tooling at the SDK, NDK and JDK that `doctor` found.
///
/// Set for the child process only. Changing the machine's environment is not
/// this tool's to do, and a person who has deliberately pointed `ANDROID_HOME`
/// somewhere keeps it — [`doctor::android_sdk`] reads the variable first and
/// only falls back to the default location. A `JAVA_HOME` that cannot run the
/// build is the one exception, and the build says when it passes one over.
fn android_environment(command: &mut Command, project: &Project) -> Result<(), NativeError> {
    let sdk = doctor::android_sdk().ok_or_else(|| {
        NativeError::new(
            "android",
            "no Android SDK — run `cargo rahti native doctor`.",
        )
    })?;
    let ndk = doctor::android_ndk(Some(&sdk)).ok_or_else(|| {
        NativeError::new(
            "android",
            "no Android NDK — run `cargo rahti native doctor`.",
        )
    })?;

    command.env("ANDROID_HOME", &sdk);
    command.env("ANDROID_SDK_ROOT", &sdk);
    command.env("NDK_HOME", &ndk);
    debug_library_size(command);

    let java = doctor::java(project);
    let home = java.home.ok_or_else(|| {
        NativeError::new(
            "android",
            format!(
                "no JDK can run this build, which needs {} — run `cargo rahti native doctor`.",
                java.wanted
            ),
        )
    })?;
    if let Some(problem) = java.problem {
        println!("  java: {problem}");
        println!("  java: using {} for this build", home.display());
    }
    command.env("JAVA_HOME", home);

    // Signing is *not* passed through the environment to Tauri: it does not
    // read one on Android. `configure_signing` writes the key's details into
    // the generated Gradle project instead — see the note there.

    Ok(())
}

/// The debug-info level of an Android debug library.
///
/// Cargo's `dev` profile carries full debug info, and Tauri's Gradle project
/// keeps debug symbols in a debug package. For an application linking a whole
/// web backend that is a library of half a gigabyte, stored uncompressed: the
/// package takes tens of seconds to write and to push, and an emulator with a
/// few gigabytes free refuses to *reinstall* it, because it holds the old and
/// the new copy at once. So the second debug build in a row failed with
/// `INSTALL_FAILED_INSUFFICIENT_STORAGE`.
///
/// Line tables keep what a debug build on a device is used for — a panic's
/// backtrace with files and lines — and drop the type and variable
/// information only a native debugger reads. Set for the child only, and not
/// at all when the variable is already set: whoever set it decides.
pub(crate) const DEV_DEBUG: (&str, &str) = ("CARGO_PROFILE_DEV_DEBUG", "line-tables-only");

fn debug_library_size(command: &mut Command) {
    let (name, value) = DEV_DEBUG;
    if std::env::var_os(name).is_none() {
        command.env(name, value);
    }
}

/// The same, for a Windows code-signing certificate.
fn windows_environment(command: &mut Command) {
    for (ours, theirs) in [
        (
            "RAHTI_NATIVE_WINDOWS_CERTIFICATE",
            "TAURI_SIGNING_WINDOWS_CERTIFICATE",
        ),
        (
            "RAHTI_NATIVE_WINDOWS_CERTIFICATE_PASSWORD",
            "TAURI_SIGNING_WINDOWS_CERTIFICATE_PASSWORD",
        ),
    ] {
        if let Ok(value) = std::env::var(ours)
            && !value.trim().is_empty()
        {
            command.env(theirs, value);
        }
    }
}

// ------------------------------------------------------------ artifacts

/// Resolve the build directory through Cargo, including local configuration.
pub(crate) fn target_directory(native: &Path, override_dir: Option<&std::ffi::OsStr>) -> PathBuf {
    let mut command = Command::new("cargo");
    command.env_remove("CARGO_TARGET_DIR");
    if let Some(directory) = override_dir {
        command.env("CARGO_TARGET_DIR", directory);
    }
    command
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(native)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok())
        .and_then(|metadata| metadata["target_directory"].as_str().map(PathBuf::from))
        .unwrap_or_else(|| native.join("target"))
}

/// The directories this Android build writes to, and no other.
///
/// Gradle keeps one directory per flavor and profile and never clears the
/// others, so walking `outputs/` finds every package any earlier build left
/// behind. An earlier single-ABI APK then sits in the list beside the one just
/// built, under the same name, and installing it puts an old application on the
/// device with nothing to say so.
///
/// Named here rather than filtered by date because Gradle leaves a package
/// that is up to date alone, and an unchanged package is still this build's.
pub(crate) fn android_output_dirs(
    outputs: &Path,
    abi: Option<AndroidAbi>,
    format: Option<AndroidFormat>,
    debug: bool,
) -> Vec<(PathBuf, &'static str)> {
    let flavor = abi.map_or("universal", AndroidAbi::flavor);
    let (profile, capitalized) = if debug {
        ("debug", "Debug")
    } else {
        ("release", "Release")
    };
    let apk = outputs.join("apk").join(flavor).join(profile);
    let aab = outputs
        .join("bundle")
        .join(format!("{flavor}{capitalized}"));

    match format {
        Some(AndroidFormat::Apk) => vec![(apk, "apk")],
        Some(AndroidFormat::Aab) => vec![(aab, "aab")],
        None => vec![(apk, "apk"), (aab, "aab")],
    }
}

/// Whether a file was written by a build that started at `started`.
///
/// Two seconds of grace, because FAT and some network drives keep modification
/// times that coarsely.
pub(crate) fn written_since(path: &Path, started: SystemTime) -> bool {
    let earliest = started - std::time::Duration::from_secs(2);
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| modified >= earliest)
}

/// Everything this build produced, where the bundler left it.
///
/// Found by walking the output directories rather than by predicting the
/// filenames, because a bundler's naming is its own and a printed path that
/// does not exist is worse than none.
fn find_artifacts(
    project: &Project,
    args: &Build,
    abi: Option<AndroidAbi>,
    format: Option<AndroidFormat>,
    started: SystemTime,
) -> (Vec<PathBuf>, PathBuf) {
    let native = project.native_dir();
    let profile = if args.debug { "debug" } else { "release" };
    let mut found: Vec<PathBuf> = Vec::new();

    let searched = match args.target {
        Platform::Android => {
            let outputs = native.join("gen/android/app/build/outputs");
            for (dir, extension) in android_output_dirs(&outputs, abi, format, args.debug) {
                collect(&dir, &[extension], &mut found);
            }
            outputs
        }
        _ => {
            let override_dir = std::env::var_os("CARGO_TARGET_DIR");
            let bundle = target_directory(&native, override_dir.as_deref())
                .join(profile)
                .join("bundle");
            // The bundlers rewrite their installers on every run, so one older
            // than this build is from before a rename or a version bump.
            collect(&bundle, &["exe", "msi"], &mut found);
            found.retain(|path| written_since(path, started));
            bundle
        }
    };
    found.sort();
    (found, searched)
}

/// Where every finished package is copied: `native/packages/<target>/<profile>/`.
pub(crate) fn packages_dir(project: &Project, target: Platform, debug: bool) -> PathBuf {
    project
        .native_dir()
        .join("packages")
        .join(target.name())
        .join(if debug { "debug" } else { "release" })
}

/// Copy what this build produced into `native/packages/`, under names that
/// say what each file is, print those paths and return them.
///
/// ## Why a copy, in one place
///
/// Each bundler keeps its output in its own tree (NSIS and WiX under Cargo's
/// target directory, Gradle under `gen/android/app/build/outputs/<kind>/
/// <flavor>/<profile>`) and names it for itself: `app-universal-release.apk`
/// says neither which application nor which version. So whoever needed the
/// file had to know four paths, and agents copied packages into directories of
/// their own invention. Every package now lands in one directory, named
/// `<product>-<version>-<variant>[-debug].<ext>`.
///
/// Copied, not moved or linked: Gradle and the bundlers compare against their
/// own output on the next build, and Gradle rewrites an APK in place, which
/// would rewrite the other name of a hard link too. A rebuild of the same
/// version replaces its file; an older version's stays until it is deleted.
fn publish_packages(
    project: &Project,
    config: &NativeConfig,
    args: &Build,
    built: &[PathBuf],
    searched: &Path,
) -> Result<Vec<PathBuf>, NativeError> {
    println!();
    if built.is_empty() {
        println!("  The build reported success, but no package was found where one was");
        println!("  expected. Look under:");
        println!("    {}", searched.display());
        return Ok(Vec::new());
    }

    let dir = packages_dir(project, args.target, args.debug);
    std::fs::create_dir_all(&dir).map_err(|e| NativeError::io("build", &dir, e))?;
    // Ignored by its own file, so no project's .gitignore has to know about it.
    let ignore = project.native_dir().join("packages/.gitignore");
    if !ignore.is_file() {
        std::fs::write(
            &ignore,
            "# Packages `cargo rahti native build` produced. Not source.\n*\n",
        )
        .map_err(|e| NativeError::io("build", &ignore, e))?;
    }

    let slug = package_slug(&config.product_name, &project.package);
    let mut published = Vec::new();
    for source in built {
        let Some(file) = source.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let name = package_name(
            &slug,
            &config.product_name,
            &config.version,
            args.target,
            args.debug,
            file,
        );
        let destination = dir.join(name);
        std::fs::copy(source, &destination)
            .map_err(|e| NativeError::io("build", &destination, e))?;
        published.push(destination);
    }

    println!("  Packages in {}:", dir.display());
    for path in &published {
        let size = std::fs::metadata(path).map_or(0, |m| m.len());
        println!(
            "    {}  ({})",
            path.file_name().unwrap_or_default().to_string_lossy(),
            megabytes(size)
        );
    }
    Ok(published)
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

/// The product name as a file name: lowercase, `-` for anything else.
pub(crate) fn package_slug(product_name: &str, fallback: &str) -> String {
    let mut slug = String::new();
    for c in product_name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        fallback.to_string()
    } else {
        slug
    }
}

/// `<slug>-<version>-<variant>[-debug].<ext>` for one bundler's file.
///
/// - Gradle: `app-<flavor>-<profile>[-unsigned].<ext>`, so the variant is the
///   flavor (`universal`, `arm64`, `x86_64`), and an unsigned package keeps
///   saying so.
/// - Tauri: `<productName>_<version>_<arch>-setup.exe` and
///   `<productName>_<version>_<arch>_<language>.msi`, so the variant is what
///   follows the version: `x64-setup`, `x64-en-US`.
///
/// A name in neither shape keeps its own name after the prefix, rather than
/// being guessed at.
pub(crate) fn package_name(
    slug: &str,
    product_name: &str,
    version: &str,
    target: Platform,
    debug: bool,
    file: &str,
) -> String {
    let (stem, extension) = file.rsplit_once('.').unwrap_or((file, ""));
    let profile = if debug { "debug" } else { "release" };

    let variant = match target {
        Platform::Android => stem.strip_prefix("app-").map(|rest| {
            let (flavor, unsigned) = match rest.strip_suffix("-unsigned") {
                Some(rest) => (rest, "-unsigned"),
                None => (rest, ""),
            };
            let flavor = flavor
                .strip_suffix(&format!("-{profile}"))
                .unwrap_or(flavor);
            format!("{flavor}{unsigned}")
        }),
        _ => stem
            .strip_prefix(&format!("{product_name}_{version}_"))
            .map(|rest| rest.replace('_', "-")),
    }
    .unwrap_or_else(|| stem.to_string());

    let debug = if debug { "-debug" } else { "" };
    let dot = if extension.is_empty() { "" } else { "." };
    format!("{slug}-{version}-{variant}{debug}{dot}{extension}")
}

pub(crate) fn build_report(
    args: &Build,
    abi: Option<AndroidAbi>,
    artifacts: &[PathBuf],
    warnings: &[doctor::Finding],
    installed: bool,
) -> serde_json::Value {
    serde_json::json!({
        "target": args.target.name(),
        "profile": if args.debug { "debug" } else { "release" },
        "abi": abi.map(AndroidAbi::flavor),
        "device": args.device,
        "installed": installed,
        "artifacts": artifacts,
        "warnings": warnings.iter().map(doctor::Finding::to_json).collect::<Vec<_>>(),
    })
}

/// Every file under `dir` with one of `extensions`.
fn collect(dir: &Path, extensions: &[&str], found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, extensions, found);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| extensions.iter().any(|w| e.eq_ignore_ascii_case(w)))
        {
            found.push(path);
        }
    }
}

// --------------------------------------------------------------- running

/// Run it, inheriting the terminal so the tooling's own output is the output.
pub(crate) fn execute(mut command: Command, what: &str) -> Result<(), NativeError> {
    let status = command.status().map_err(|e| {
        NativeError::new(
            "tauri",
            format!(
                "could not run `cargo tauri`: {e}\n  \
                 Install it with: cargo install tauri-cli --version \"^2.8\" --locked"
            ),
        )
    })?;

    if status.success() {
        return Ok(());
    }

    Err(NativeError::new(
        "tauri",
        format!(
            "`cargo tauri {what}` failed ({status}).\n  \
             The output above is Tauri's own."
        ),
    ))
}

fn describe(command: &Command) -> String {
    let args: Vec<String> = command
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    format!("cargo {}", args.join(" "))
}

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
//! - the artifacts are found and their absolute paths printed, because
//!   "finished in 4m12s" is not an answer to "where is the installer", and
//!   recorded in `native/.rahti/build.json` for whatever runs next.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use rahti_native::{DatabaseMode, NativeConfig, NativeError, Platform};

use crate::args::{AndroidAbi, AndroidFormat, Build, Dev};
use crate::doctor;
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
        ensure_android_project(project, config)?;
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
    let started = SystemTime::now();
    let mut warnings = prepare(project, config, args.target, !args.debug)?;
    warnings.extend(debug_credentials(project, config, args));

    // A build that succeeds says nothing about what it could not do, so what
    // `doctor` warned about is said here, ahead of Tauri's own output.
    if !warnings.is_empty() {
        doctor::report(&warnings);
        println!();
    }

    // Before the build: a serial that is not attached should cost a sentence,
    // not several minutes of compiling.
    let abi = match (args.abi, &args.device) {
        (Some(abi), _) => Some(abi),
        (None, Some(device)) => Some(device_abi(device)?),
        (None, None) => None,
    };
    // `--device` has to install something, and only an APK installs.
    let format = match &args.device {
        Some(_) => Some(AndroidFormat::Apk),
        None => args.format,
    };

    let mut command = match args.target {
        Platform::Android => {
            ensure_android_project(project, config)?;
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
    println!();
    execute(command, "build")?;

    let artifacts = report_artifacts(project, args, abi, format, started);

    let installed = match &args.device {
        Some(device) => {
            install_on_device(device, &artifacts, &config.identifier)?;
            true
        }
        None => false,
    };

    write_build_report(project, args, abi, &artifacts, &warnings, installed)
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
    )?;

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
fn ensure_android_project(project: &Project, config: &NativeConfig) -> Result<(), NativeError> {
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
    configure_signing(&generated)?;

    sync_android_name(&generated, &config.product_name)?;

    Ok(())
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
/// With nothing configured this does nothing, and a `--debug` build is signed
/// with Android's debug key as usual.
pub(crate) fn configure_signing(generated: &Path) -> Result<(), NativeError> {
    let Some(keystore) = env_value("RAHTI_NATIVE_ANDROID_KEYSTORE") else {
        return Ok(());
    };

    let store_password = env_value("RAHTI_NATIVE_ANDROID_KEYSTORE_PASSWORD").unwrap_or_default();
    let alias = env_value("RAHTI_NATIVE_ANDROID_KEY_ALIAS").unwrap_or_default();
    let key_password =
        env_value("RAHTI_NATIVE_ANDROID_KEY_PASSWORD").unwrap_or_else(|| store_password.clone());

    if store_password.is_empty() || alias.is_empty() {
        return Err(NativeError::new(
            "android",
            "RAHTI_NATIVE_ANDROID_KEYSTORE is set, but the keystore password or the key alias              is not.
               All of RAHTI_NATIVE_ANDROID_KEYSTORE,              RAHTI_NATIVE_ANDROID_KEYSTORE_PASSWORD and RAHTI_NATIVE_ANDROID_KEY_ALIAS are              needed to sign a release; a half-configured key produces an unsigned package              that Google Play refuses.",
        ));
    }

    // Gradle reads this as a Java properties file, where a backslash escapes —
    // so a Windows path is written with forward slashes.
    let properties = format!(
        "storeFile={}\nstorePassword={}\nkeyAlias={}\npassword={}\n",
        keystore.replace('\\', "/"),
        store_password,
        alias,
        key_password
    );
    let properties_path = generated.join("keystore.properties");
    std::fs::write(&properties_path, properties)
        .map_err(|e| NativeError::io("android", &properties_path, e))?;

    let gradle_path = generated.join("app/build.gradle.kts");
    let gradle = std::fs::read_to_string(&gradle_path)
        .map_err(|e| NativeError::io("android", &gradle_path, e))?;

    if gradle.contains("rahtiKeystore") {
        return Ok(());
    }

    const CONFIG_ANCHOR: &str = "    buildTypes {";
    const RELEASE_ANCHOR: &str = "        getByName(\"release\") {";
    const APPLY_SIGNING: &str = "            signingConfig = signingConfigs.getByName(\"release\")";

    if !gradle.contains(CONFIG_ANCHOR) || !gradle.contains(RELEASE_ANCHOR) {
        return Err(NativeError::at(
            "android",
            &gradle_path,
            "this Gradle file is not one `cargo rahti native` recognises, so release signing \
             was not wired in.\n  \
             Add a `signingConfigs` block reading keystore.properties by hand, or the release \
             package will be unsigned.",
        ));
    }

    let signing = r#"    val rahtiKeystore = Properties().apply {
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

    let patched = gradle
        .replacen(CONFIG_ANCHOR, &format!("{signing}{CONFIG_ANCHOR}"), 1)
        .replacen(
            RELEASE_ANCHOR,
            &format!(
                "{RELEASE_ANCHOR}
{APPLY_SIGNING}"
            ),
            1,
        );

    std::fs::write(&gradle_path, patched)
        .map_err(|e| NativeError::io("android", &gradle_path, e))?;

    println!("  android: release signing wired in from RAHTI_NATIVE_ANDROID_*");
    Ok(())
}

/// A variable's value, with blank counted as unset.
fn env_value(name: &str) -> Option<String> {
    let value = std::env::var(name).ok()?;
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
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

/// Print the absolute path of everything this build produced, and return it.
///
/// Found by walking the output directories rather than by predicting the
/// filenames, because a bundler's naming is its own and a printed path that
/// does not exist is worse than none.
fn report_artifacts(
    project: &Project,
    args: &Build,
    abi: Option<AndroidAbi>,
    format: Option<AndroidFormat>,
    started: SystemTime,
) -> Vec<PathBuf> {
    let native = project.native_dir();
    let profile = if args.debug { "debug" } else { "release" };
    let override_dir = std::env::var_os("CARGO_TARGET_DIR");
    let target_dir = target_directory(&native, override_dir.as_deref());

    let mut found: Vec<PathBuf> = Vec::new();

    match args.target {
        Platform::Android => {
            let outputs = native.join("gen/android/app/build/outputs");
            for (dir, extension) in android_output_dirs(&outputs, abi, format, args.debug) {
                collect(&dir, &[extension], &mut found);
            }
        }
        _ => {
            let target = target_dir.join(profile);
            // The bundlers rewrite their installers on every run, so one older
            // than this build is from before a rename or a version bump.
            collect(&target.join("bundle"), &["exe", "msi"], &mut found);
            found.retain(|path| written_since(path, started));
            // Not filtered: cargo leaves a current executable alone.
            let exe = target.join(format!("{}-native.exe", project.package));
            if exe.is_file() {
                found.push(exe);
            }
        }
    }

    println!();
    found.sort();
    if found.is_empty() {
        println!("  The build reported success, but no package was found where one was");
        println!("  expected. Look under:");
        println!("    {}", target_dir.display());
        return found;
    }

    println!("  Built:");
    for path in &found {
        println!("    {}", path.display());
    }
    found
}

/// Where the last `build` is described for whatever runs next.
pub(crate) fn build_report_path(project: &Project) -> PathBuf {
    project.native_dir().join(".rahti/build.json")
}

/// Record what this build produced in `native/.rahti/build.json`.
///
/// A build's output is thousands of lines of somebody else's tooling, and an
/// agent running one usually keeps only the end of it. This is the same answer
/// as a file, so nothing has to be parsed out of a terminal.
fn write_build_report(
    project: &Project,
    args: &Build,
    abi: Option<AndroidAbi>,
    artifacts: &[PathBuf],
    warnings: &[doctor::Finding],
    installed: bool,
) -> Result<(), NativeError> {
    let report = build_report(args, abi, artifacts, warnings, installed);
    let path = build_report_path(project);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| NativeError::io("build", parent, e))?;
    }
    let text = serde_json::to_string_pretty(&report).unwrap_or_default();
    std::fs::write(&path, text).map_err(|e| NativeError::io("build", &path, e))?;

    println!();
    println!("  Report: {}", path.display());
    Ok(())
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

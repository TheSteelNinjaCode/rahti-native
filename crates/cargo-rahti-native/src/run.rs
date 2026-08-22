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
//!   "finished in 4m12s" is not an answer to "where is the installer".

use std::path::{Path, PathBuf};
use std::process::Command;

use rahti_native::{NativeConfig, NativeError, Platform};

use crate::args::{AndroidFormat, Build, Dev};
use crate::doctor;
use crate::project::Project;

pub fn dev(project: &Project, config: &NativeConfig, args: &Dev) -> Result<(), NativeError> {
    prepare(project, config, args.target, false)?;

    let mut command = doctor::cargo_tauri(match args.target {
        Platform::Android => &["android", "dev"],
        _ => &["dev"],
    });
    command.current_dir(project.native_dir());

    if let Some(device) = &args.device {
        command.arg(device);
    }

    if args.target == Platform::Android {
        android_environment(&mut command)?;
        ensure_android_project(project, config)?;
    }

    println!("  running {}", describe(&command));
    println!();
    execute(command, "dev")
}

pub fn build(project: &Project, config: &NativeConfig, args: &Build) -> Result<(), NativeError> {
    prepare(project, config, args.target, !args.debug)?;

    let mut command = match args.target {
        Platform::Android => {
            ensure_android_project(project, config)?;
            let mut command = doctor::cargo_tauri(&["android", "build"]);
            // `apk` installs on a device; `aab` is what Google Play takes and
            // is not installable as it is. Naming neither builds Tauri's
            // default, which is both.
            match args.format {
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
            android_environment(&mut command)?;
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

    if let Some(format) = args.format {
        println!("  format: {}", format.name());
    }
    println!("  running {}", describe(&command));
    println!();
    execute(command, "build")?;

    report_artifacts(project, config, args);
    Ok(())
}

// --------------------------------------------------------------- checks

/// Everything that has to be true before an expensive command starts.
fn prepare(
    project: &Project,
    config: &NativeConfig,
    target: Platform,
    release: bool,
) -> Result<(), NativeError> {
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
    Ok(())
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
        android_environment(&mut command)?;
        execute(command, "android init")?;
    }

    // Every time, not only after `init`: the directory is regenerated rather
    // than reviewed, and a patch that only ran once would be gone the first
    // time somebody deleted it.
    allow_loopback_cleartext(&generated)?;
    install_android_icons(project, &generated)?;
    configure_signing(&generated)?;

    let _ = config;
    Ok(())
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
/// only falls back to the default location.
fn android_environment(command: &mut Command) -> Result<(), NativeError> {
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

    if let Some(java) = doctor::java_home() {
        command.env("JAVA_HOME", java);
    }

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

/// Print the absolute path of everything the build produced.
///
/// Found by walking the output directories rather than by predicting the
/// filenames, because a bundler's naming is its own and a printed path that
/// does not exist is worse than none.
fn report_artifacts(project: &Project, config: &NativeConfig, args: &Build) {
    let native = project.native_dir();
    let profile = if args.debug { "debug" } else { "release" };

    let mut found: Vec<PathBuf> = Vec::new();

    match args.target {
        Platform::Android => {
            let outputs = native.join("gen/android/app/build/outputs");
            let wanted: &[&str] = match args.format {
                Some(AndroidFormat::Apk) => &["apk"],
                Some(AndroidFormat::Aab) => &["aab"],
                None => &["apk", "aab"],
            };
            collect(&outputs, wanted, &mut found);
        }
        _ => {
            let target = native.join("target").join(profile);
            // The executable, then whatever the bundlers wrote.
            let exe = target.join(format!("{}.exe", config.product_name));
            if exe.is_file() {
                found.push(exe);
            }
            collect(&target.join("bundle"), &["exe", "msi"], &mut found);
        }
    }

    println!();
    if found.is_empty() {
        println!("  The build reported success, but no package was found where one was");
        println!("  expected. Look under:");
        println!("    {}", native.join("target").display());
        return;
    }

    println!("  Built:");
    found.sort();
    for path in found {
        println!("    {}", path.display());
    }
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
fn execute(mut command: Command, what: &str) -> Result<(), NativeError> {
    let status = command.status().map_err(|e| {
        NativeError::new(
            "tauri",
            format!(
                "could not run `cargo tauri`: {e}\n  \
                 Install it with: cargo install tauri-cli --version \"^2\" --locked"
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

//! `cargo rahti native doctor` — what is missing, and what to do about it.
//!
//! Native packaging fails at the end of a long build far more often than at
//! the start of a short one. A missing Android NDK is discovered after Gradle
//! has been downloaded; a missing Rust target after Tauri has compiled. So the
//! checks are here, they run before `dev` and `build`, and every failure says
//! four things: what is missing, why it is needed, how to get it, and which
//! command to run again.
//!
//! ## What it does not do
//!
//! It does not install anything. A tool that silently downloaded an Android
//! SDK, changed a global rustup configuration or fetched a JDK would be doing
//! something the person running it did not ask for, on a machine that is
//! theirs. Every failure prints the command *they* can run.
//!
//! ## Only what the target needs
//!
//! A Windows-only project is not told about the Android NDK. A check that
//! reports problems with something you are not building teaches people to
//! ignore it.

use std::path::PathBuf;
use std::process::Command;

use rahti_native::{NativeConfig, Platform};

use crate::project::Project;

/// One check and how it went.
pub struct Finding {
    pub name: String,
    pub state: State,
    /// Why it is needed, and how to supply it. Empty when it is fine.
    pub advice: String,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum State {
    /// Present, and the build will use it.
    Ok,
    /// Absent, and the build cannot proceed without it.
    Missing,
    /// Absent, and the build will work without it — release signing, an
    /// emulator.
    Warning,
}

impl Finding {
    fn ok(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Finding {
            name: name.into(),
            state: State::Ok,
            advice: detail.into(),
        }
    }

    fn missing(name: impl Into<String>, advice: impl Into<String>) -> Self {
        Finding {
            name: name.into(),
            state: State::Missing,
            advice: advice.into(),
        }
    }

    fn warning(name: impl Into<String>, advice: impl Into<String>) -> Self {
        Finding {
            name: name.into(),
            state: State::Warning,
            advice: advice.into(),
        }
    }
}

/// Every check that applies to `targets`.
pub fn examine(
    project: &Project,
    config: &NativeConfig,
    targets: &[Platform],
    release: bool,
) -> Vec<Finding> {
    let mut findings = vec![shared_startup(project), native_shell(project), tauri_cli()];

    for target in targets {
        match target {
            Platform::Windows => findings.extend(windows(release)),
            Platform::Android => findings.extend(android(config, release)),
            Platform::Other => {}
        }
    }
    findings
}

/// Whether every check that has to pass, passed.
pub fn blocked(findings: &[Finding]) -> bool {
    findings.iter().any(|f| f.state == State::Missing)
}

// --------------------------------------------------------------- shared

fn shared_startup(project: &Project) -> Finding {
    if project.has_shared_startup() {
        return Finding::ok(
            "shared application startup",
            "src/lib.rs exposes initialize_application",
        );
    }
    Finding::missing(
        "shared application startup",
        format!(
            "The native shell starts the same application the web binary does, by calling\n     \
             `{}::initialize_application()`. This project's src/lib.rs does not define it.\n   \
             A project scaffolded before native support has its startup in src/main.rs, where a\n   \
             Tauri host cannot reach it — on Android the operating system calls into a library,\n   \
             not a `main`.\n   \
             Fix: cargo rahti upgrade --force src/main.rs src/lib.rs",
            project.lib
        ),
    )
}

fn native_shell(project: &Project) -> Finding {
    let manifest = project.native_dir().join("Cargo.toml");
    if manifest.is_file() {
        return Finding::ok(
            "native shell",
            format!("{}", project.native_dir().display()),
        );
    }
    Finding::missing(
        "native shell",
        "This project has no native/ directory yet.\n   \
         Fix: cargo rahti native init --identifier com.example.myapp --windows",
    )
}

/// Delegated to rather than embedded, and this is where that costs something.
///
/// The alternative — linking `tauri-cli` into this binary — would pin the CLI
/// to whatever Tauri version this tool was released against, while the shell's
/// `tauri` dependency is application-owned and upgraded on the application's
/// schedule. Two versions that have to match, released by two people, is worse
/// than one prerequisite that is checked and named.
fn tauri_cli() -> Finding {
    match run(&mut cargo_tauri(&["--version"])) {
        Some(version) => Finding::ok("cargo tauri", version.trim().to_string()),
        None => Finding::missing(
            "cargo tauri",
            "Rahti delegates packaging to Tauri's own CLI, which is not installed.\n   \
             It is a separate tool so that it can track the `tauri` version in native/Cargo.toml,\n   \
             which is yours to upgrade.\n   \
             Fix: cargo install tauri-cli --version \"^2\" --locked",
        ),
    }
}

// -------------------------------------------------------------- windows

fn windows(release: bool) -> Vec<Finding> {
    let mut findings = vec![rust_target(
        "x86_64-pc-windows-msvc",
        "The Rust target a Windows package is compiled for.",
    )];

    findings.push(match run(Command::new("link").arg("/?")) {
        Some(_) => Finding::ok("MSVC toolchain", "link.exe is on PATH"),
        None => Finding::warning(
            "MSVC toolchain",
            "`link.exe` is not on PATH. The MSVC Rust target needs the Microsoft C++ build\n   \
             tools to link, and cargo usually finds them without PATH — so this is only a\n   \
             problem if the build fails to link.\n   \
             Fix: install \"Desktop development with C++\" from the Visual Studio Build Tools.",
        ),
    });

    findings.push(webview2());

    if release {
        findings.push(match std::env::var("RAHTI_NATIVE_WINDOWS_CERTIFICATE") {
            Ok(value) if !value.trim().is_empty() => {
                Finding::ok("code signing", "RAHTI_NATIVE_WINDOWS_CERTIFICATE is set")
            }
            _ => Finding::warning(
                "code signing",
                "No signing certificate is configured, so the installer will be unsigned and\n   \
                 SmartScreen will warn about it.\n   \
                 Set RAHTI_NATIVE_WINDOWS_CERTIFICATE (base64 PFX) and\n   \
                 RAHTI_NATIVE_WINDOWS_CERTIFICATE_PASSWORD in the environment of the build.\n   \
                 Never commit either.",
            ),
        });
    }

    findings
}

/// WebView2 is the renderer a Windows package draws in.
///
/// Present on Windows 11 and on any Windows 10 that has taken updates; the
/// check is here because when it is absent the failure is a window that opens
/// empty, which explains nothing.
fn webview2() -> Finding {
    const KEYS: &[&str] = &[
        r"HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
        r"HKLM\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
        r"HKCU\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
    ];

    if !cfg!(windows) {
        return Finding::warning(
            "WebView2 runtime",
            "Not checked: this is not Windows. A Windows package needs the WebView2 runtime\n   \
             on the machine it runs on.",
        );
    }

    for key in KEYS {
        if let Some(output) = run(Command::new("reg").args(["query", key, "/v", "pv"])) {
            if let Some(version) = output.split_whitespace().last() {
                return Finding::ok("WebView2 runtime", version.to_string());
            }
        }
    }

    Finding::warning(
        "WebView2 runtime",
        "Not installed on this machine. It is what a Windows package renders in, so a build\n   \
         will succeed and the window will open empty.\n   \
         Present by default on Windows 11 and on an updated Windows 10.\n   \
         Fix: install the Evergreen WebView2 Runtime from Microsoft.",
    )
}

// -------------------------------------------------------------- android

fn android(config: &NativeConfig, release: bool) -> Vec<Finding> {
    let mut findings = Vec::new();

    let sdk = android_sdk();
    findings.push(match &sdk {
        Some(dir) => Finding::ok("Android SDK", dir.display().to_string()),
        None => Finding::missing(
            "Android SDK",
            "ANDROID_HOME is not set and no SDK was found in the usual place.\n   \
             The SDK supplies the platform the application is compiled against and the tools\n   \
             that package it.\n   \
             Fix: install it with Android Studio, then set ANDROID_HOME to the SDK directory\n   \
             (Windows: %LOCALAPPDATA%\\Android\\Sdk).",
        ),
    });

    findings.push(match android_ndk(sdk.as_deref()) {
        Some(dir) => Finding::ok("Android NDK", dir.display().to_string()),
        None => Finding::missing(
            "Android NDK",
            "NDK_HOME is not set and no NDK was found in the SDK.\n   \
             The NDK is the C toolchain that links the Rust library into the APK — an Android\n   \
             package is a Rust cdylib, and nothing links it without this.\n   \
             Fix: install \"NDK (Side by side)\" in Android Studio's SDK Manager, then set\n   \
             NDK_HOME to the versioned directory inside <sdk>/ndk.",
        ),
    });

    findings.push(match java_home() {
        Some(dir) => Finding::ok("Java", dir.display().to_string()),
        None => Finding::missing(
            "Java",
            "JAVA_HOME is not set to a JDK. Gradle runs on it, and Gradle is what assembles\n   \
             an APK or an AAB.\n   \
             Android Studio ships one: set JAVA_HOME to its `jbr` directory, or install a\n   \
             JDK 17 or later.",
        ),
    });

    // Four, because an APK on Google Play covers four ABIs. A device build
    // needs only its own, but a release AAB needs all of them.
    for target in [
        "aarch64-linux-android",
        "armv7-linux-androideabi",
        "i686-linux-android",
        "x86_64-linux-android",
    ] {
        findings.push(rust_target(
            target,
            "One of the four Android ABIs a Play release covers.",
        ));
    }

    findings.push(match run(Command::new(adb()).arg("version")) {
        Some(version) => Finding::ok(
            "adb",
            version
                .lines()
                .next()
                .unwrap_or("present")
                .trim()
                .to_string(),
        ),
        None => Finding::warning(
            "adb",
            "Not on PATH. It is how a build installs onto a device and how `dev` reaches one;\n   \
             a build that only produces a file does not need it.\n   \
             Fix: add <sdk>/platform-tools to PATH.",
        ),
    });

    findings.push(Finding::ok(
        "minimum API level",
        format!("android.minSdk = {}", config.android.min_sdk),
    ));

    if cfg!(windows) {
        findings.push(windows_symlinks());
    }

    if release {
        findings.push(match std::env::var("RAHTI_NATIVE_ANDROID_KEYSTORE") {
            Ok(value) if !value.trim().is_empty() => {
                Finding::ok("release signing", "RAHTI_NATIVE_ANDROID_KEYSTORE is set")
            }
            _ => Finding::warning(
                "release signing",
                "No keystore is configured. An unsigned release AAB is refused by Google Play\n   \
                 and an unsigned release APK will not install.\n   \
                 Set RAHTI_NATIVE_ANDROID_KEYSTORE, RAHTI_NATIVE_ANDROID_KEYSTORE_PASSWORD,\n   \
                 RAHTI_NATIVE_ANDROID_KEY_ALIAS and RAHTI_NATIVE_ANDROID_KEY_PASSWORD in the\n   \
                 environment of the build.\n   \
                 Never commit a keystore or any of these values. `--debug` builds a package\n   \
                 that installs without them.",
            ),
        });
    }

    findings
}

/// Whether this account may create a symbolic link.
///
/// A Windows-host-only check, and it is here because of exactly where it
/// fails without it: `cargo tauri android build` compiles every Rust target —
/// several minutes and a 20 MB shared library per ABI — and *then* links the
/// result into the Gradle project's `jniLibs` with a symlink. On Windows that
/// needs either Developer Mode or an elevated shell, and without one the build
/// stops after all the expensive work with a message about
/// `SeCreateSymbolicLinkPrivilege`.
///
/// Tested by trying it rather than by reading the registry, because the
/// privilege can come from Developer Mode, from elevation, or from group
/// policy, and only the attempt covers all three.
fn windows_symlinks() -> Finding {
    let dir = std::env::temp_dir();
    let link = dir.join(format!("rahti-native-symlink-{}", std::process::id()));
    let _ = std::fs::remove_file(&link);

    #[cfg(windows)]
    let created = std::os::windows::fs::symlink_dir(&dir, &link).is_ok();
    #[cfg(not(windows))]
    let created = true;

    let _ = std::fs::remove_dir(&link);
    let _ = std::fs::remove_file(&link);

    if created {
        return Finding::ok("symbolic links", "this account may create them");
    }

    Finding::missing(
        "symbolic links",
        "This Windows account cannot create a symbolic link, and Tauri's Android build\n   \
         links the compiled Rust library into the Gradle project with one. Without it the\n   \
         build compiles every ABI and then fails at the last step.\n   \
         Fix: turn on Settings > System > For developers > Developer Mode, or run the build\n   \
         from an elevated terminal. Then: cargo rahti native doctor --target android",
    )
}

// ---------------------------------------------------------------- parts

fn rust_target(triple: &str, why: &str) -> Finding {
    match run(Command::new("rustup").args(["target", "list", "--installed"])) {
        Some(installed) if installed.lines().any(|line| line.trim() == triple) => {
            Finding::ok(format!("rust target {triple}"), String::new())
        }
        Some(_) => Finding::missing(
            format!("rust target {triple}"),
            format!("{why}\n   Fix: rustup target add {triple}"),
        ),
        None => Finding::warning(
            format!("rust target {triple}"),
            "rustup is not on PATH, so installed targets could not be listed.".to_string(),
        ),
    }
}

/// `ANDROID_HOME`, then the two names Google has used, then the default
/// install directory.
pub fn android_sdk() -> Option<PathBuf> {
    for name in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(dir) = existing_dir_from_env(name) {
            return Some(dir);
        }
    }

    let default = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("Android/Sdk"))
    } else {
        std::env::var_os("HOME").map(|dir| PathBuf::from(dir).join("Android/Sdk"))
    };
    default.filter(|dir| dir.is_dir())
}

/// `NDK_HOME`, or the newest version inside the SDK.
pub fn android_ndk(sdk: Option<&std::path::Path>) -> Option<PathBuf> {
    for name in ["NDK_HOME", "ANDROID_NDK_HOME", "ANDROID_NDK_ROOT"] {
        if let Some(dir) = existing_dir_from_env(name) {
            return Some(dir);
        }
    }

    let versions = std::fs::read_dir(sdk?.join("ndk")).ok()?;
    let mut found: Vec<PathBuf> = versions
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    // Lexical, which orders the versions Google publishes correctly because
    // they are zero-padded.
    found.sort();
    found.pop()
}

/// A `JAVA_HOME` that is actually a JDK.
///
/// The directory existing is not enough: an Android Studio installation that
/// was moved or partly removed leaves a `jbr` with no runtime in it, and
/// `JAVA_HOME` pointing at one produces a Gradle failure that names a missing
/// `jvm.cfg`.
pub fn java_home() -> Option<PathBuf> {
    let home = existing_dir_from_env("JAVA_HOME")?;
    let launcher = if cfg!(windows) {
        home.join("bin/java.exe")
    } else {
        home.join("bin/java")
    };
    launcher.is_file().then_some(home)
}

fn adb() -> PathBuf {
    android_sdk()
        .map(|sdk| {
            sdk.join(if cfg!(windows) {
                "platform-tools/adb.exe"
            } else {
                "platform-tools/adb"
            })
        })
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("adb"))
}

fn existing_dir_from_env(name: &str) -> Option<PathBuf> {
    let value = std::env::var(name).ok()?;
    let value = value.trim();
    let path = PathBuf::from(value);
    (!value.is_empty() && path.is_dir()).then_some(path)
}

/// `cargo tauri <args>`.
pub fn cargo_tauri(args: &[&str]) -> Command {
    let mut command = Command::new("cargo");
    command.arg("tauri").args(args);
    command
}

/// Run something and take its output, or `None` if it is not there or failed.
fn run(command: &mut Command) -> Option<String> {
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stderr).to_string();
    }
    Some(text)
}

/// Print the findings the way `doctor` does.
pub fn report(findings: &[Finding]) {
    for finding in findings {
        let mark = match finding.state {
            State::Ok => "ok  ",
            State::Missing => "MISS",
            State::Warning => "warn",
        };
        if finding.state == State::Ok {
            if finding.advice.is_empty() {
                println!("  {mark}  {}", finding.name);
            } else {
                println!("  {mark}  {} — {}", finding.name, finding.advice);
            }
        } else {
            println!("  {mark}  {}", finding.name);
            for line in finding.advice.lines() {
                println!("        {line}");
            }
        }
    }
}

//! Reading the command line.
//!
//! Separated from the commands so that parsing can be tested without a project
//! on disk, and read by hand rather than with a parser crate — the same choice
//! `cargo-rahti` makes, for the same reason: the surface is a verb and a
//! handful of flags, and a dependency that read them would be larger than the
//! code that acts on them.
//!
//! ## How this binary is reached
//!
//! Three ways, and all three have to parse:
//!
//! | Typed | `args()` after the executable |
//! | --- | --- |
//! | `cargo rahti-native init` | `["rahti-native", "init"]` |
//! | `cargo rahti native init` | `["native", "init"]` — forwarded by `cargo-rahti` |
//! | `cargo-rahti-native init` | `["init"]` |
//!
//! Cargo turns `cargo <name> <args>` into `<name-binary> <name> <args>`, which
//! is where the first form's leading word comes from. The second is
//! `cargo-rahti` delegating. So the leading words are skipped, and what is
//! left is the same command in every case.

use rahti_native::Platform;

/// What was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Init(Init),
    Doctor(Doctor),
    Dev(Dev),
    Build(Build),
    Help,
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Init {
    /// Reverse-DNS. Required on a first run, and taken from the existing file
    /// on a repeat.
    pub identifier: Option<String>,
    pub product_name: Option<String>,
    pub version: Option<String>,
    /// `--windows`, `--android`. Empty means "whatever the project already
    /// asked for", which on a first run is both.
    pub targets: Vec<&'static str>,
    /// Take back a generated file that was edited. Never implied.
    pub force: bool,
    /// Depend on a Rahti checkout by path. For working on the framework.
    pub local: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Doctor {
    /// Which target's prerequisites to check. `None` checks every target the
    /// project configured.
    pub target: Option<Platform>,
    /// Also check what only a release build needs — signing, most of all.
    pub release: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dev {
    pub target: Platform,
    /// Which device or emulator, on Android.
    pub device: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    pub target: Platform,
    /// Android only.
    pub format: Option<AndroidFormat>,
    /// A debug build, which on Android is what installs without signing
    /// configured.
    pub debug: bool,
}

/// What an Android build produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AndroidFormat {
    /// Installs on a device directly. What testing uses.
    Apk,
    /// What Google Play takes. Not installable as it is — Play builds the
    /// per-device APKs from it.
    Aab,
}

impl AndroidFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "apk" => Some(AndroidFormat::Apk),
            "aab" => Some(AndroidFormat::Aab),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            AndroidFormat::Apk => "apk",
            AndroidFormat::Aab => "aab",
        }
    }
}

/// Drop the words cargo and `cargo-rahti` put in front of the real command.
pub fn strip_dispatch(args: &[String]) -> Vec<&str> {
    args.iter()
        .map(String::as_str)
        .skip_while(|a| matches!(*a, "rahti-native" | "rahti" | "native"))
        .collect()
}

/// Parse, or say what was wrong with it.
pub fn parse(args: &[&str]) -> Result<Command, String> {
    match args.first().copied() {
        Some("init") => parse_init(&args[1..]),
        Some("doctor") => parse_doctor(&args[1..]),
        Some("dev") => parse_dev(&args[1..]),
        Some("build") => parse_build(&args[1..]),
        Some("--version" | "-V") => Ok(Command::Version),
        Some("help" | "--help" | "-h") | None => Ok(Command::Help),
        Some(other) => Err(format!(
            "`{other}` is not a `cargo rahti native` command.\n  \
             Expected one of: init, doctor, dev, build."
        )),
    }
}

fn parse_init(args: &[&str]) -> Result<Command, String> {
    let mut init = Init::default();
    let mut i = 0;

    while i < args.len() {
        match args[i] {
            "--identifier" => init.identifier = Some(value(args, &mut i, "--identifier")?),
            "--name" => init.product_name = Some(value(args, &mut i, "--name")?),
            "--app-version" => init.version = Some(value(args, &mut i, "--app-version")?),
            "--windows" => push_target(&mut init.targets, "windows"),
            "--android" => push_target(&mut init.targets, "android"),
            "--local" => init.local = Some(value(args, &mut i, "--local")?),
            "--force" | "-f" => init.force = true,
            "--help" | "-h" => return Ok(Command::Help),
            other => return Err(unknown(other, "init")),
        }
        i += 1;
    }

    Ok(Command::Init(init))
}

fn parse_doctor(args: &[&str]) -> Result<Command, String> {
    let mut doctor = Doctor::default();
    let mut i = 0;

    while i < args.len() {
        match args[i] {
            "--target" => doctor.target = Some(target(args, &mut i)?),
            "--release" => doctor.release = true,
            "--help" | "-h" => return Ok(Command::Help),
            other => return Err(unknown(other, "doctor")),
        }
        i += 1;
    }

    Ok(Command::Doctor(doctor))
}

fn parse_dev(args: &[&str]) -> Result<Command, String> {
    let mut platform = None;
    let mut device = None;
    let mut i = 0;

    while i < args.len() {
        match args[i] {
            "--target" => platform = Some(target(args, &mut i)?),
            "--device" => device = Some(value(args, &mut i, "--device")?),
            "--help" | "-h" => return Ok(Command::Help),
            other => return Err(unknown(other, "dev")),
        }
        i += 1;
    }

    let target = platform.ok_or_else(|| required_target("dev"))?;
    if device.is_some() && target != Platform::Android {
        return Err(
            "`--device` names an Android device or emulator, and only Android has \
                    one.\n  Drop it, or use `--target android`."
                .to_string(),
        );
    }

    Ok(Command::Dev(Dev { target, device }))
}

fn parse_build(args: &[&str]) -> Result<Command, String> {
    let mut platform = None;
    let mut format = None;
    let mut debug = false;
    let mut i = 0;

    while i < args.len() {
        match args[i] {
            "--target" => platform = Some(target(args, &mut i)?),
            "--format" => {
                let raw = value(args, &mut i, "--format")?;
                format = Some(AndroidFormat::parse(&raw).ok_or_else(|| {
                    format!(
                        "`{raw}` is not an Android package format.\n  \
                         Expected `apk` — installs on a device — or `aab`, which is what \
                         Google Play takes."
                    )
                })?);
            }
            "--debug" => debug = true,
            "--help" | "-h" => return Ok(Command::Help),
            other => return Err(unknown(other, "build")),
        }
        i += 1;
    }

    let target = platform.ok_or_else(|| required_target("build"))?;

    // Refused rather than ignored: a Windows build that quietly accepted
    // `--format aab` would look like it had produced one.
    if format.is_some() && target != Platform::Android {
        return Err(format!(
            "`--format` chooses between an Android APK and an AAB, and `--target {target}` \
             builds neither.\n  \
             A Windows build produces an executable and an installer, and takes no format."
        ));
    }

    Ok(Command::Build(Build {
        target,
        format,
        debug,
    }))
}

fn push_target(targets: &mut Vec<&'static str>, name: &'static str) {
    if !targets.contains(&name) {
        targets.push(name);
    }
}

fn target(args: &[&str], i: &mut usize) -> Result<Platform, String> {
    let raw = value(args, i, "--target")?;
    Platform::parse_target(&raw).ok_or_else(|| {
        format!(
            "`{raw}` is not a native target.\n  \
             Rahti packages `windows` and `android`."
        )
    })
}

fn value(args: &[&str], i: &mut usize, flag: &str) -> Result<String, String> {
    *i += 1;
    let Some(value) = args.get(*i) else {
        return Err(format!("`{flag}` needs a value."));
    };
    // A flag where a value should be is a missing value, not a value that
    // happens to start with a dash.
    if value.starts_with("--") {
        return Err(format!("`{flag}` needs a value, and `{value}` is a flag."));
    }
    Ok((*value).to_string())
}

fn required_target(command: &str) -> String {
    format!(
        "`{command}` needs to know which package to work on.\n  \
         cargo rahti native {command} --target windows\n  \
         cargo rahti native {command} --target android"
    )
}

fn unknown(flag: &str, command: &str) -> String {
    format!("`{flag}` is not an option of `{command}`. Run `cargo rahti native --help`.")
}

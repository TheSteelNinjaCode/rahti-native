//! `cargo rahti native` — optional Windows and Android packaging for a Rahti
//! application.
//!
//! A separate executable from `cargo-rahti`, and that is the whole reason
//! native support can be called optional. `cargo-rahti` is what every Rahti
//! project uses and it knows nothing about Tauri; this is installed by the
//! people who want a package:
//!
//! ```text
//! cargo install cargo-rahti-native
//! ```
//!
//! `cargo rahti native …` reaches it through a delegation in `cargo-rahti`,
//! which is a `Command` spawn and adds no dependency. `cargo rahti-native …`
//! reaches it directly, through cargo's own subcommand dispatch. Both arrive
//! here — see [`args::strip_dispatch`].
//!
//! ## What it does not do
//!
//! It does not install anything. No SDK, no toolchain, no Tauri CLI, and no
//! change to a global rustup configuration. `doctor` says what is missing and
//! prints the command to run; the person whose machine it is runs it.

mod args;
mod doctor;
mod icons;
mod init;
mod project;
mod run;
mod schema;
mod shell;

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;

use std::process::ExitCode;

use rahti_native::{NativeConfig, NativeError, Platform};

use args::Command;
use project::Project;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let argv = args::strip_dispatch(&raw);

    let command = match args::parse(&argv) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };

    let result = match command {
        Command::Help => {
            print!("{}", usage());
            return ExitCode::SUCCESS;
        }
        Command::Version => {
            println!("cargo-rahti-native {VERSION}");
            return ExitCode::SUCCESS;
        }
        Command::Init(ref init_args) => in_project(|project| init::run(project, init_args)),
        Command::Doctor(ref doctor_args) => in_project(|project| {
            let config = load_config(project)?;
            let targets = match doctor_args.target {
                Some(target) => vec![target],
                // Nothing named: check what the project actually builds. A
                // report about a platform you are not shipping is a report
                // people learn to skip.
                None => config
                    .targets
                    .iter()
                    .filter_map(|t| Platform::parse_target(t))
                    .collect(),
            };

            println!();
            println!("  {} {}", config.product_name, config.identifier);
            println!(
                "  checking: {}",
                targets
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!();

            let findings = doctor::examine(project, &config, &targets, doctor_args.release);
            doctor::report(&findings);
            println!();

            if doctor::blocked(&findings) {
                return Err(NativeError::new(
                    "doctor",
                    "something a build needs is missing. Each line above says what to run.",
                ));
            }
            println!("  Ready.");
            Ok(())
        }),
        Command::Dev(ref dev_args) => in_project(|project| {
            let config = load_config(project)?;
            run::dev(project, &config, dev_args)
        }),
        Command::Build(ref build_args) => in_project(|project| {
            let config = load_config(project)?;
            run::build(project, &config, build_args)
        }),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!();
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Find the project, then do the thing.
fn in_project(work: impl FnOnce(&Project) -> Result<(), NativeError>) -> Result<(), NativeError> {
    let here = std::env::current_dir().map_err(|e| {
        NativeError::new("project", format!("cannot read the working directory: {e}"))
    })?;
    let project = Project::discover(&here)?;
    work(&project)
}

fn load_config(project: &Project) -> Result<NativeConfig, NativeError> {
    NativeConfig::load(&project.native_config())
}

fn usage() -> String {
    format!(
        "cargo-rahti-native {VERSION}
Package a Rahti application for Windows and Android.

The application is the one you already have. Its Rust backend is compiled for
the target platform and runs inside the installed program; the operating
system's WebView loads the same server-rendered pages, the same PulsePoint
runtime, the same rpcs and the same sockets. Nothing is converted, and the
controls on screen are a WebView's, not the platform's.

USAGE:
    cargo rahti native init [options]
    cargo rahti native doctor [--target <target>] [--release]
    cargo rahti native dev --target <target> [--device <name>]
    cargo rahti native build --target <target> [--format apk|aab] [--debug]

    <target> is `windows` or `android`.

INIT:
    Writes native/ — the Tauri shell — and rahti.native.json beside it.
    Additive and idempotent: run it again after editing the configuration and
    the shell is brought back into agreement with it. A generated file you
    have edited is left alone and reported.

    --identifier <id>  Reverse-DNS, for example com.example.myapp. Required
                       the first time. It is the Android package name, so
                       every segment must be a legal Java identifier and a
                       hyphen is refused. Changing it later makes a different
                       application.
    --name <name>      What the installed application is called. Defaults to
                       the cargo package name, title-cased.
    --app-version <v>  major.minor.patch, all numeric. Defaults to the cargo
                       package version.
    --windows          Build a Windows package.
    --android          Build an Android package.
                       Naming neither, the first time, means both.
    --local <path>     Depend on a Rahti checkout by path rather than by
                       version. For working on the framework itself. Recorded
                       in rahti.native.json, so a later run does not need it.
    -f, --force        Take back a generated file you have edited. What it
                       replaces is not kept anywhere.

DOCTOR:
    Checks only what the target needs, and never installs anything. Every
    failure says what is missing, why it is needed, and the command to run.

    --target <target>  Just this one. Without it, every target the project
                       configured.
    --release          Also check what only a release needs — signing, most
                       of all.

DEV:
    Builds and launches the application in a WebView, with development
    diagnostics on.

    --device <name>    Which Android device or emulator.

BUILD:
    Windows produces an executable and an installer. Android produces an APK,
    an AAB, or both. Absolute paths to everything are printed at the end.

    --format apk       Installs on a device directly. What testing wants.
    --format aab       What Google Play takes. Not installable as it is.
    --debug            A debug build. On Android this is what installs
                       without signing configured.

    -h, --help         Print this message.
    -V, --version      Print the version.

PREREQUISITES:
    Packaging is delegated to Tauri's own CLI, which is a separate install:

        cargo install tauri-cli --version \"^2\" --locked

    `cargo rahti native doctor` checks for it, and for the Android SDK, NDK,
    JDK and Rust targets when Android is a target.
"
    )
}

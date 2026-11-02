//! Reading the command line.

use crate::args::*;
use rahti_native::Platform;

fn parsed(line: &str) -> Command {
    let argv: Vec<&str> = line.split_whitespace().collect();
    parse(&argv).unwrap_or_else(|e| panic!("`{line}` did not parse: {e}"))
}

fn refused(line: &str) -> String {
    let argv: Vec<&str> = line.split_whitespace().collect();
    parse(&argv)
        .err()
        .unwrap_or_else(|| panic!("`{line}` was accepted"))
}

// ------------------------------------------------------------- dispatch

#[test]
fn all_three_ways_of_reaching_this_binary_parse_the_same() {
    // `cargo rahti-native init`  — cargo's own subcommand dispatch
    // `cargo rahti native init`  — cargo-rahti delegating
    // `cargo-rahti-native init`  — run directly, which is what tests do
    let lines = [
        vec!["rahti-native".to_string(), "init".to_string()],
        vec!["native".to_string(), "init".to_string()],
        vec![
            "rahti".to_string(),
            "native".to_string(),
            "init".to_string(),
        ],
        vec!["init".to_string()],
    ];

    for line in lines {
        assert_eq!(strip_dispatch(&line), vec!["init"], "{line:?}");
    }
}

#[test]
fn a_dispatch_word_after_the_command_is_left_alone() {
    // Only the leading words are cargo's. `--name native` is a value.
    let line = vec![
        "native".to_string(),
        "init".to_string(),
        "--name".to_string(),
        "native".to_string(),
    ];
    assert_eq!(strip_dispatch(&line), vec!["init", "--name", "native"]);
}

// ----------------------------------------------------------------- init

#[test]
fn init_reads_its_flags() {
    let Command::Init(init) = parsed("init --identifier com.example.myapp --windows --android")
    else {
        panic!("not an init");
    };
    assert_eq!(init.identifier.as_deref(), Some("com.example.myapp"));
    assert_eq!(init.targets, vec!["windows", "android"]);
    assert!(!init.force);
}

#[test]
fn init_needs_nothing_because_a_repeat_run_reads_the_file() {
    let Command::Init(init) = parsed("init") else {
        panic!("not an init");
    };
    assert!(init.identifier.is_none());
    assert!(init.targets.is_empty());
}

#[test]
fn a_repeated_target_flag_is_named_once() {
    let Command::Init(init) = parsed("init --windows --windows") else {
        panic!("not an init");
    };
    assert_eq!(init.targets, vec!["windows"]);
}

#[test]
fn force_is_never_implied() {
    let Command::Init(init) = parsed("init --identifier com.example.a") else {
        panic!("not an init");
    };
    assert!(!init.force);

    let Command::Init(init) = parsed("init --force") else {
        panic!("not an init");
    };
    assert!(init.force);
}

// --------------------------------------------------------------- doctor

#[test]
fn doctor_checks_every_configured_target_when_none_is_named() {
    let Command::Doctor(doctor) = parsed("doctor") else {
        panic!("not a doctor");
    };
    assert!(doctor.target.is_none());
    assert!(!doctor.release);
}

#[test]
fn doctor_takes_a_target_and_a_release_flag() {
    let Command::Doctor(doctor) = parsed("doctor --target android --release") else {
        panic!("not a doctor");
    };
    assert_eq!(doctor.target, Some(Platform::Android));
    assert!(doctor.release);
}

// ------------------------------------------------------------ dev/build

#[test]
fn dev_and_build_require_a_target() {
    for line in ["dev", "build"] {
        let error = refused(line);
        assert!(error.contains("--target windows"), "{error}");
    }
}

#[test]
fn build_reads_the_android_formats() {
    for (line, expected) in [
        ("build --target android --format apk", AndroidFormat::Apk),
        ("build --target android --format aab", AndroidFormat::Aab),
        ("build --target android --format AAB", AndroidFormat::Aab),
    ] {
        let Command::Build(build) = parsed(line) else {
            panic!("not a build");
        };
        assert_eq!(build.format, Some(expected), "{line}");
        assert_eq!(build.target, Platform::Android);
    }
}

#[test]
fn build_without_a_format_builds_what_tauri_builds() {
    let Command::Build(build) = parsed("build --target android") else {
        panic!("not a build");
    };
    assert!(build.format.is_none());
}

#[test]
fn a_windows_build_refuses_a_format_rather_than_ignoring_it() {
    // A build that quietly accepted `--format aab` would look like it had
    // produced one.
    let error = refused("build --target windows --format aab");
    assert!(error.contains("Android"), "{error}");
}

#[test]
fn a_format_that_is_not_a_package_is_refused() {
    let error = refused("build --target android --format ipa");
    assert!(error.contains("apk"), "{error}");
}

#[test]
fn a_device_belongs_to_android() {
    let Command::Dev(dev) = parsed("dev --target android --device Pixel_7") else {
        panic!("not a dev");
    };
    assert_eq!(dev.device.as_deref(), Some("Pixel_7"));

    let error = refused("dev --target windows --device Pixel_7");
    assert!(error.contains("Android"), "{error}");
}

// ------------------------------------------------------------- refusals

#[test]
fn a_target_that_is_not_a_platform_is_refused() {
    let error = refused("build --target ios");
    assert!(
        error.contains("windows") && error.contains("android"),
        "{error}"
    );
}

#[test]
fn a_flag_where_a_value_should_be_is_a_missing_value() {
    // `--identifier --windows` is somebody who forgot the identifier, not an
    // application called `--windows`.
    let error = refused("init --identifier --windows");
    assert!(error.contains("needs a value"), "{error}");

    let error = refused("build --target");
    assert!(error.contains("needs a value"), "{error}");
}

#[test]
fn an_unknown_command_says_what_the_commands_are() {
    let error = refused("package");
    assert!(error.contains("init"), "{error}");
    assert!(error.contains("doctor"), "{error}");
}

#[test]
fn an_unknown_flag_is_refused_rather_than_ignored() {
    let error = refused("build --target windows --sign");
    assert!(error.contains("--sign"), "{error}");
}

#[test]
fn help_and_version_are_reachable_from_everywhere() {
    assert!(matches!(parsed(""), Command::Help));
    assert!(matches!(parsed("--help"), Command::Help));
    assert!(matches!(parsed("-h"), Command::Help));
    assert!(matches!(parsed("help"), Command::Help));
    assert!(matches!(parsed("init --help"), Command::Help));
    assert!(matches!(parsed("build --help"), Command::Help));
    assert!(matches!(parsed("--version"), Command::Version));
    assert!(matches!(parsed("-V"), Command::Version));
}

// ---------------------------------------------------------- agent surface

#[test]
fn a_build_can_name_one_abi_in_androids_words_or_taurus() {
    for line in [
        "build --target android --abi x86_64",
        "build --target android --abi X86_64",
    ] {
        let Command::Build(build) = parsed(line) else {
            panic!("`{line}` is not a build");
        };
        assert_eq!(build.abi, Some(AndroidAbi::X86_64));
    }
    assert_eq!(AndroidAbi::parse("arm64-v8a\r\n"), Some(AndroidAbi::Arm64));
    assert_eq!(AndroidAbi::parse("aarch64"), Some(AndroidAbi::Arm64));
    assert_eq!(AndroidAbi::parse("armeabi-v7a"), Some(AndroidAbi::Arm));
    assert_eq!(AndroidAbi::parse("i686"), Some(AndroidAbi::X86));
    assert!(refused("build --target android --abi mips").contains("arm64-v8a"));
}

#[test]
fn a_build_can_name_a_device() {
    let Command::Build(build) = parsed("build --target android --debug --device emulator-5554")
    else {
        panic!("not a build");
    };
    assert_eq!(build.device.as_deref(), Some("emulator-5554"));
}

#[test]
fn what_only_android_has_is_refused_on_windows() {
    assert!(refused("build --target windows --abi x86_64").contains("--target android"));
    assert!(refused("build --target windows --device emulator-5554").contains("--target android"));
}

#[test]
fn an_aab_cannot_be_installed_on_a_device() {
    let message = refused("build --target android --format aab --device emulator-5554");
    assert!(message.contains("does not install"), "{message}");
}

#[test]
fn doctor_can_answer_in_json() {
    let Command::Doctor(doctor) = parsed("doctor --json") else {
        panic!("not doctor");
    };
    assert!(doctor.json);
}

#[test]
fn icon_takes_one_image_and_an_optional_background() {
    assert_eq!(
        parsed("icon public/logo.svg --background #0a2540"),
        Command::Icon(Icon {
            source: "public/logo.svg".to_string(),
            background: Some("#0a2540".to_string()),
        })
    );
    assert_eq!(
        parsed("icon logo.png"),
        Command::Icon(Icon {
            source: "logo.png".to_string(),
            background: None,
        })
    );
    assert!(refused("icon").contains("square PNG or SVG"));
    assert!(refused("icon a.svg b.svg").contains("second"));
    assert!(refused("icon a.svg --background red").contains("#rrggbb"));
    assert!(refused("icon a.svg --force").contains("not an option"));
}

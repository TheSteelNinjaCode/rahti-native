#[test]
fn tauri_2_8_is_the_first_cli_with_external_watch_folders() {
    for version in ["tauri-cli 2.8.0", "tauri-cli 2.11.4"] {
        assert!(super::supported_tauri_cli(version), "{version}");
    }

    for version in [
        "tauri-cli 2.7.1",
        "tauri-cli 1.6.3",
        "tauri-cli 3.0.0",
        "not a version",
    ] {
        assert!(!super::supported_tauri_cli(version), "{version}");
    }
}

#[test]
fn a_prerelease_suffix_does_not_hide_the_supported_minor() {
    assert!(super::supported_tauri_cli("tauri-cli 2.8.0-beta.1"));
}

#[test]
fn both_java_version_styles_give_the_major() {
    let modern = "openjdk version \"25.0.4\" 2026-07-21\nOpenJDK Runtime Environment";
    assert_eq!(super::java_major(modern), Some(25));
    assert_eq!(super::java_major("java version \"1.8.0_292\""), Some(8));
    assert_eq!(super::java_major("openjdk version \"26-ea\""), Some(26));

    let picked_up = "Picked up JAVA_TOOL_OPTIONS: -Xmx1g\nopenjdk version \"21.0.2\"";
    assert_eq!(super::java_major(picked_up), Some(21));

    assert_eq!(super::java_major("not java"), None);
}

#[test]
fn a_java_home_that_does_not_start_is_a_problem() {
    let problem = super::java_problem(std::path::Path::new("no/such/jbr"), None);
    assert!(problem.is_some_and(|text| text.contains("does not start")));
}

#[test]
fn the_gradle_version_comes_from_the_wrapper_properties() {
    let wrapper = "#Tue May 10 19:22:52 CST 2022\n\
                   distributionBase=GRADLE_USER_HOME\n\
                   distributionUrl=https\\://services.gradle.org/distributions/gradle-8.14.3-bin.zip\n";
    assert_eq!(super::gradle_version(wrapper), Some((8, 14)));

    let two_part = "distributionUrl=https\\://example.test/gradle-9.1-all.zip";
    assert_eq!(super::gradle_version(two_part), Some((9, 1)));

    assert_eq!(super::gradle_version("distributionBase=x"), None);
}

#[test]
fn gradle_8_14_stops_at_java_24_and_9_1_has_no_known_ceiling() {
    assert_eq!(super::newest_java_on_gradle((8, 14)), Some(24));
    assert_eq!(super::newest_java_on_gradle((9, 0)), Some(24));
    assert_eq!(super::newest_java_on_gradle((8, 11)), Some(23));
    assert_eq!(super::newest_java_on_gradle((8, 5)), Some(21));
    assert_eq!(super::newest_java_on_gradle((9, 1)), None);
}

#[test]
fn a_remote_release_is_told_it_has_no_database() {
    let mut config =
        rahti_native::NativeConfig::new("My App", "com.example.myapp", "0.1.0", &["android"]);
    assert!(crate::doctor::release_database(&config, true).is_none());

    config.database.mode = rahti_native::DatabaseMode::Remote;
    assert!(crate::doctor::release_database(&config, false).is_none());

    let finding = crate::doctor::release_database(&config, true).expect("a warning");
    assert!(
        finding.advice.contains("DATABASE_URL"),
        "{}",
        finding.advice
    );
}

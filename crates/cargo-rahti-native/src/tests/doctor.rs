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

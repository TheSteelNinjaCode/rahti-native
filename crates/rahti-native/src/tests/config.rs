//! `rahti.native.json`: what it accepts, and what it refuses before a build
//! has cost anything.

use crate::config::*;

fn valid() -> NativeConfig {
    NativeConfig::new("My App", "com.example.myapp", "0.1.0", &["windows"])
}

#[test]
fn a_generated_configuration_is_valid() {
    assert!(valid().validate().is_ok());
}

#[test]
fn a_configuration_survives_a_round_trip() {
    let original = valid();
    let parsed = NativeConfig::parse(&original.to_json()).expect("the file it just wrote");
    assert_eq!(original, parsed);
}

#[test]
fn omitted_sections_take_their_defaults() {
    let parsed = NativeConfig::parse(
        r#"{
            "schema": 1,
            "productName": "Minimal",
            "identifier": "com.example.minimal",
            "version": "1.0.0",
            "targets": ["android"]
        }"#,
    )
    .expect("a file with only the required fields");

    assert_eq!(parsed.window.width, 1200);
    assert_eq!(parsed.android.min_sdk, MIN_ANDROID_SDK);
    assert_eq!(parsed.database.mode, DatabaseMode::SqliteLocal);
    assert!(parsed.security.loopback_token);
    assert!(!parsed.security.csp.is_empty());
}

#[test]
fn a_misspelled_field_is_refused_rather_than_ignored() {
    // The whole reason for validating before a build: a `prodcutName` that
    // parsed and defaulted would produce an installer with the wrong name and
    // no complaint anywhere.
    let error = NativeConfig::parse(
        r#"{
            "schema": 1,
            "prodcutName": "Typo",
            "productName": "My App",
            "identifier": "com.example.myapp",
            "version": "1.0.0",
            "targets": ["windows"]
        }"#,
    )
    .expect_err("an unknown field");
    assert!(error.to_string().contains("prodcutName"), "{error}");
}

#[test]
fn a_future_schema_version_is_refused() {
    let error = NativeConfig::parse(
        r#"{
            "schema": 99,
            "productName": "My App",
            "identifier": "com.example.myapp",
            "version": "1.0.0",
            "targets": ["windows"]
        }"#,
    )
    .expect_err("a schema this build does not know");
    assert!(error.to_string().contains("schema"), "{error}");
}

#[test]
fn a_target_that_is_not_a_platform_is_refused() {
    let mut config = valid();
    config.targets = vec!["ios".to_string()];
    let error = config.validate().expect_err("an unsupported target");
    assert!(error.to_string().contains("ios"), "{error}");
}

#[test]
fn no_targets_is_refused() {
    let mut config = valid();
    config.targets.clear();
    assert!(config.validate().is_err());
}

#[test]
fn an_empty_csp_is_refused() {
    let mut config = valid();
    config.security.csp = "   ".to_string();
    let error = config.validate().expect_err("no policy at all");
    assert!(
        error.to_string().contains("Content-Security-Policy"),
        "{error}"
    );
}

#[test]
fn a_min_sdk_below_tauris_floor_is_refused() {
    let mut config = valid();
    config.android.min_sdk = 21;
    assert!(config.validate().is_err());
}

// --------------------------------------------------------- identifiers

#[test]
fn a_reverse_dns_identifier_is_accepted() {
    for good in [
        "com.example.myapp",
        "io.github.ada.notes",
        "com.example.my_app",
        "com.example.app2",
    ] {
        assert!(check_identifier(good).is_ok(), "{good} was refused");
    }
}

#[test]
fn a_hyphen_is_refused_because_android_refuses_it() {
    // The mistake worth catching early: a perfectly good Windows bundle
    // identity that stops a Gradle build several minutes in.
    let error = check_identifier("com.example.my-app").expect_err("a hyphen");
    assert!(error.contains("hyphen"), "{error}");
}

#[test]
fn an_identifier_needs_more_than_one_segment() {
    assert!(check_identifier("myapp").is_err());
}

#[test]
fn a_segment_starting_with_a_digit_is_refused() {
    assert!(check_identifier("com.example.2fast").is_err());
    assert!(check_identifier("com.1example.app").is_err());
}

#[test]
fn an_empty_segment_is_refused() {
    assert!(check_identifier("com..app").is_err());
    assert!(check_identifier("com.example.").is_err());
    assert!(check_identifier(".com.example").is_err());
}

#[test]
fn a_java_keyword_segment_is_refused() {
    // `com.example.new` compiles to a Java package called `new`.
    let error = check_identifier("com.example.new").expect_err("a keyword");
    assert!(error.contains("keyword"), "{error}");
}

#[test]
fn tauris_placeholder_identifier_is_refused() {
    let error = check_identifier("com.tauri.dev").expect_err("the placeholder");
    assert!(error.contains("placeholder"), "{error}");
}

#[test]
fn surrounding_whitespace_is_refused_rather_than_trimmed() {
    assert!(check_identifier(" com.example.myapp").is_err());
    assert!(check_identifier("com.example.myapp\n").is_err());
}

// ------------------------------------------------------- product names

#[test]
fn a_product_name_that_is_not_a_filename_is_refused() {
    for bad in ["", "  ", "My/App", "My:App", "My\"App", "My App "] {
        assert!(check_product_name(bad).is_err(), "{bad:?} was accepted");
    }
    assert!(check_product_name("My App").is_ok());
    assert!(check_product_name("Ada's Notes").is_ok());
}

// ------------------------------------------------------------ versions

#[test]
fn a_version_is_three_numbers() {
    assert!(check_version("0.1.0").is_ok());
    assert!(check_version("12.4.199").is_ok());
}

#[test]
fn a_pre_release_version_is_refused() {
    for bad in ["1.0", "1.0.0-beta", "1.0.0.1", "v1.0.0", "1..0", ""] {
        assert!(check_version(bad).is_err(), "{bad:?} was accepted");
    }
}

#[test]
fn an_android_version_code_increases_with_the_version() {
    let code = |version: &str| {
        NativeConfig::new("A", "com.example.a", version, &["android"]).android_version_code()
    };
    assert!(code("1.0.0") < code("1.0.1"));
    assert!(code("1.0.99") < code("1.1.0"));
    assert!(code("1.99.99") < code("2.0.0"));
    assert_eq!(code("1.2.3"), 10203);
}

// ------------------------------------------------------------- cookies

#[test]
fn a_cookie_name_that_could_not_be_read_back_is_refused() {
    let mut config = valid();
    for bad in ["a=b", "a;b", "a b", ""] {
        config.auth.cookie_name = Some(bad.to_string());
        assert!(config.validate().is_err(), "{bad:?} was accepted");
    }
    config.auth.cookie_name = Some("rahti_session_9f2c".to_string());
    assert!(config.validate().is_ok());
}

#[test]
fn a_configuration_never_serializes_a_secret_field() {
    // The file is committed. Nothing that is a credential may reach it, and
    // the way that stays true is that there is no field to put one in.
    let json = valid().to_json().to_ascii_lowercase();
    for forbidden in ["secret", "password", "keystore", "auth_secret"] {
        assert!(
            !json.contains(forbidden),
            "rahti.native.json serialized `{forbidden}`"
        );
    }
}

#[test]
fn builds_answers_for_the_targets_that_were_asked_for() {
    let config = NativeConfig::new("A", "com.example.a", "1.0.0", &["windows"]);
    assert!(config.builds("windows"));
    assert!(!config.builds("android"));
}

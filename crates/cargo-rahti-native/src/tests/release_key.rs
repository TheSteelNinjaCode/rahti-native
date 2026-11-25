//! The release key a machine makes when none is configured.

use crate::release_key::{DETAILS, KEYSTORE};

fn scratch(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rahti-native-release-key-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn an_empty_folder_has_no_key_yet() {
    let dir = scratch("empty");
    assert!(crate::release_key::load(&dir).unwrap().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_made_key_is_read_back_with_its_passwords() {
    let dir = scratch("made");
    std::fs::write(dir.join(KEYSTORE), b"a keystore").unwrap();
    std::fs::write(
        dir.join(DETAILS),
        r#"{ "alias": "release", "storePassword": "s3cret#\\x", "keyPassword": "s3cret#\\x" }"#,
    )
    .unwrap();

    let key = crate::release_key::load(&dir).unwrap().expect("a key");
    let properties = key.properties();
    assert!(properties.contains("keyAlias=release"), "{properties}");
    // Escaped for the properties file like any other password.
    assert!(
        properties.contains(r"storePassword=s3cret\#\\x"),
        "{properties}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_keystore_without_its_details_is_refused_not_replaced() {
    // Making a new key here would orphan every installation the old one
    // signed: an update must be signed with the key of the first install.
    let dir = scratch("half");
    std::fs::write(dir.join(KEYSTORE), b"somebody's key").unwrap();

    let error = crate::release_key::load(&dir).expect_err("half a key");
    assert!(error.to_string().contains("backup"), "{error}");
    assert_eq!(
        std::fs::read(dir.join(KEYSTORE)).unwrap(),
        b"somebody's key".to_vec()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn details_without_a_password_are_refused() {
    let dir = scratch("no-password");
    std::fs::write(dir.join(KEYSTORE), b"a keystore").unwrap();
    std::fs::write(dir.join(DETAILS), r#"{ "alias": "release" }"#).unwrap();

    let error = crate::release_key::load(&dir).expect_err("no password");
    assert!(error.to_string().contains("storePassword"), "{error}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn generated_passwords_are_long_random_and_plain() {
    let one = crate::release_key::password().unwrap();
    let two = crate::release_key::password().unwrap();
    assert_eq!(one.len(), 48);
    assert!(one.chars().all(|c| c.is_ascii_hexdigit()), "{one}");
    assert_ne!(one, two);
}

#[test]
fn the_certificate_name_escapes_x500_syntax() {
    assert_eq!(
        crate::release_key::distinguished_name("Apaknaka"),
        "CN=Apaknaka, O=Apaknaka"
    );
    assert_eq!(
        crate::release_key::distinguished_name("Acme, Inc."),
        r"CN=Acme\, Inc., O=Acme\, Inc."
    );
    assert_eq!(
        crate::release_key::distinguished_name("  "),
        "CN=Rahti application, O=Rahti application"
    );
}

#[test]
fn each_application_has_its_own_key_folder() {
    let one = crate::release_key::key_dir("com.example.one").expect("a key folder");
    let two = crate::release_key::key_dir("com.example.two").expect("a key folder");
    assert_ne!(one, two);
    assert!(one.ends_with("com.example.one"), "{}", one.display());
}

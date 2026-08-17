//! The JSON schema, held to the parser it describes.

use super::*;

fn schema() -> serde_json::Value {
    serde_json::from_str(&json()).expect("the schema is JSON")
}

#[test]
fn the_schema_describes_every_field_the_parser_writes() {
    // The failure this catches: a field added to `NativeConfig` and not to the
    // schema, so an editor reports the file's own valid content as an error.
    let schema = schema();
    let described: Vec<String> = schema["properties"]
        .as_object()
        .expect("properties")
        .keys()
        .cloned()
        .collect();

    let written =
        rahti_native::NativeConfig::new("A", "com.example.a", "1.0.0", &["windows"]).to_json();
    let written: serde_json::Value = serde_json::from_str(&written).unwrap();

    for field in written.as_object().unwrap().keys() {
        assert!(
            described.contains(field),
            "`{field}` is written but not in the schema"
        );
    }

    // And the bookkeeping `init` appends.
    assert!(described.iter().any(|f| f == "scaffold"));
}

#[test]
fn the_schema_refuses_what_the_parser_refuses() {
    let schema = schema();
    assert_eq!(schema["additionalProperties"], serde_json::json!(false));
    assert_eq!(
        schema["properties"]["schema"]["const"],
        rahti_native::SCHEMA_VERSION
    );
    assert_eq!(
        schema["properties"]["android"]["properties"]["minSdk"]["minimum"],
        rahti_native::MIN_ANDROID_SDK
    );

    let targets = schema["properties"]["targets"]["items"]["enum"]
        .as_array()
        .expect("the target enum");
    assert_eq!(targets.len(), rahti_native::TARGETS.len());
    for target in rahti_native::TARGETS {
        assert!(targets.contains(&serde_json::json!(target)), "{target}");
    }
}

#[test]
fn the_required_fields_are_the_ones_with_no_default() {
    let schema = schema();
    let required = schema["required"].as_array().expect("required");
    for field in ["schema", "productName", "identifier", "version", "targets"] {
        assert!(required.contains(&serde_json::json!(field)), "{field}");
    }
    // Everything else has a default in the parser, so requiring it here would
    // reject a file the parser reads.
    assert_eq!(required.len(), 5);
}

#[test]
fn a_configuration_the_parser_writes_satisfies_the_schemas_own_patterns() {
    // Not a full JSON Schema validator — just the three patterns that carry a
    // rule, checked against a real value.
    let schema = schema();
    let config =
        rahti_native::NativeConfig::new("My App", "com.example.myapp", "1.2.3", &["windows"]);

    // The rule the identifier pattern exists for: segments are Java
    // identifiers, so a hyphen is not in the character class.
    assert_eq!(
        schema["properties"]["identifier"]["pattern"],
        serde_json::json!("^[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z][A-Za-z0-9_]*)+$")
    );
    assert!(rahti_native::check_identifier(&config.identifier).is_ok());
    assert!(rahti_native::check_identifier("com.example.my-app").is_err());

    assert_eq!(
        schema["properties"]["version"]["pattern"],
        serde_json::json!("^[0-9]+\\.[0-9]+\\.[0-9]+$")
    );
    assert!(rahti_native::check_version(&config.version).is_ok());
}

#[test]
fn the_schema_has_no_field_for_a_credential() {
    let text = json().to_ascii_lowercase();
    for forbidden in [
        "\"password\"",
        "\"keystore\"",
        "\"auth_secret\"",
        "\"secret\"",
    ] {
        assert!(!text.contains(forbidden), "the schema declares {forbidden}");
    }
}

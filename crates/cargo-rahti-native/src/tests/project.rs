//! Finding the project, and the facts read out of its manifest.

use super::*;

#[test]
fn a_lib_name_is_read_when_one_is_declared() {
    let manifest =
        "[package]\nname = \"my-app\"\nversion = \"0.3.0\"\n\n[lib]\nname = \"custom\"\n";
    assert_eq!(
        table_value(manifest, "package", "name").as_deref(),
        Some("my-app")
    );
    assert_eq!(
        table_value(manifest, "package", "version").as_deref(),
        Some("0.3.0")
    );
    assert_eq!(
        table_value(manifest, "lib", "name").as_deref(),
        Some("custom")
    );
}

#[test]
fn a_key_from_another_table_is_not_read() {
    // `[dependencies] name` and `[package] name` are different things, and the
    // reader stops at the next header rather than taking the first match in
    // the file.
    let manifest = "[dependencies]\nname = \"wrong\"\n\n[package]\nname = \"right\"\n";
    assert_eq!(
        table_value(manifest, "package", "name").as_deref(),
        Some("right")
    );
}

#[test]
fn a_comment_is_not_a_value() {
    let manifest = "[package]\n# name = \"commented-out\"\nname = \"real\" # trailing\n";
    assert_eq!(
        table_value(manifest, "package", "name").as_deref(),
        Some("real")
    );
}

#[test]
fn an_absent_key_is_absent_rather_than_empty() {
    assert_eq!(
        table_value("[package]\nname = \"a\"\n", "lib", "name"),
        None
    );
    assert_eq!(table_value("", "package", "name"), None);
}

/// A throwaway directory holding one manifest.
fn manifest_dir(label: &str, manifest: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rahti-native-project-{label}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Cargo.toml"), manifest).unwrap();
    dir
}

#[test]
fn a_workspace_inherited_version_falls_back_rather_than_becoming_true() {
    // `version.workspace = true` is a value this reader cannot follow — it is
    // in another file — and `true` is not a version.
    let dir = manifest_dir(
        "inherit",
        "[package]\nname = \"app\"\nversion.workspace = true\n",
    );

    let project = Project::at(&dir).expect("a project");
    assert_eq!(project.version, "0.1.0");
    // Cargo's default library name: the package name with its hyphens turned
    // into underscores.
    assert_eq!(project.lib, "app");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_hyphenated_package_has_an_underscored_library() {
    let dir = manifest_dir(
        "hyphen",
        "[package]\nname = \"my-great-app\"\nversion = \"1.0.0\"\n",
    );

    let project = Project::at(&dir).expect("a project");
    assert_eq!(project.lib, "my_great_app");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_directory_that_is_not_a_rahti_project_says_so() {
    let dir = std::env::temp_dir().join("rahti-native-not-a-project");
    std::fs::create_dir_all(&dir).unwrap();

    let error = Project::discover(&dir).expect_err("no marker");
    assert!(error.to_string().contains("rahti.config.json"), "{error}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_root_is_found_from_a_subdirectory() {
    // People run this from wherever they are in the project.
    let root = manifest_dir(
        "discover",
        "[package]\nname = \"app\"\nversion = \"1.0.0\"\n",
    );
    let deep = root.join("src/app/customers");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(root.join("rahti.config.json"), "{}").unwrap();

    let project = Project::discover(&deep).expect("a project");
    assert_eq!(project.root, root);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_cookie_name_is_read_from_env_and_the_secret_is_not() {
    let root = manifest_dir("cookie", "[package]\nname = \"app\"\nversion = \"1.0.0\"\n");
    std::fs::write(
        root.join(".env"),
        "#AUTH_COOKIE_NAME=commented\nAUTH_SECRET=deadbeefdeadbeef\n\
         AUTH_COOKIE_NAME=\"rahti_9f2c\"\n",
    )
    .unwrap();

    let project = Project::at(&root).expect("a project");
    assert_eq!(project.cookie_name().as_deref(), Some("rahti_9f2c"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn shared_startup_is_detected_by_the_function_the_shell_calls() {
    let root = manifest_dir(
        "startup",
        "[package]\nname = \"app\"\nversion = \"1.0.0\"\n",
    );
    std::fs::create_dir_all(root.join("src")).unwrap();

    let project = Project::at(&root).expect("a project");
    assert!(!project.has_shared_startup(), "no src/lib.rs at all");

    std::fs::write(root.join("src/lib.rs"), "mod routes;\n").unwrap();
    assert!(!project.has_shared_startup(), "a lib without the function");

    std::fs::write(
        root.join("src/lib.rs"),
        "pub async fn initialize_application() -> Result<(), ()> { Ok(()) }\n",
    )
    .unwrap();
    assert!(project.has_shared_startup());

    let _ = std::fs::remove_dir_all(&root);
}

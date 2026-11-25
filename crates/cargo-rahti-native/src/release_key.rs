//! The Android release key this machine makes when none is configured.
//!
//! ## The bug this exists for
//!
//! A release APK built without `RAHTI_NATIVE_ANDROID_*` came out
//! `…-unsigned.apk`, and Android refuses to install an unsigned package — the
//! installer says "invalid APK" and nothing else. So the default release build
//! produced a file nobody could use, and the way out was a `keytool` command
//! and four environment variables most people had never needed before.
//!
//! Tools like PWABuilder sign by default for the same reason: they make a key,
//! sign with it, and hand the key over to keep.
//!
//! ## What happens instead
//!
//! A release build with no key configured signs with this machine's key for the
//! application's identifier, and makes that key the first time:
//!
//! ```text
//! Windows  %APPDATA%\rahti-native\keys\<identifier>\
//! macOS    ~/Library/Application Support/rahti-native/keys/<identifier>/
//! Linux    $XDG_DATA_HOME (or ~/.local/share)/rahti-native/keys/<identifier>/
//! ```
//!
//! `RAHTI_NATIVE_KEY_DIR` replaces the `…/rahti-native/keys` part, for a CI
//! runner or a key kept on other storage.
//!
//! The directory holds `release.jks` and `release-key.json`, its alias and
//! generated passwords. Outside the project, so it is never committed, and per
//! application, so two applications never share a key. It is made once and
//! never replaced: every update of an installed application has to be signed
//! with the key that signed the first install, so a key silently regenerated
//! would make every existing installation impossible to update. The build says
//! where the key is, and that it has to be backed up.
//!
//! `RAHTI_NATIVE_ANDROID_KEYSTORE` still wins whenever it is set.

use std::path::{Path, PathBuf};
use std::process::Command;

use rahti_native::{NativeConfig, NativeError};

use crate::doctor;
use crate::project::Project;
use crate::run::SigningKey;

/// The keystore's file name inside the key directory.
pub(crate) const KEYSTORE: &str = "release.jks";
/// Its alias and passwords, beside it.
pub(crate) const DETAILS: &str = "release-key.json";
/// The alias of the key the machine makes.
const ALIAS: &str = "release";

/// Where this machine keeps `identifier`'s release key, if it has a place for
/// keys at all.
pub(crate) fn key_dir(identifier: &str) -> Option<PathBuf> {
    keys_root().map(|root| root.join(identifier))
}

/// `…/rahti-native/keys`, or `RAHTI_NATIVE_KEY_DIR`.
fn keys_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("RAHTI_NATIVE_KEY_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };

    let data = if cfg!(windows) {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library/Application Support"))
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|home| home.join(".local/share")))
    }?;
    Some(data.join("rahti-native").join("keys"))
}

/// The key in `dir`, if one was made there.
///
/// A keystore without its details is refused rather than replaced: it is
/// somebody's key, and making a new one would orphan every installation it
/// signed.
pub(crate) fn load(dir: &Path) -> Result<Option<SigningKey>, NativeError> {
    let keystore = dir.join(KEYSTORE);
    let details_path = dir.join(DETAILS);

    match (keystore.is_file(), details_path.is_file()) {
        (false, false) => return Ok(None),
        (true, true) => {}
        _ => {
            return Err(NativeError::at(
                "android",
                dir,
                format!(
                    "this folder has one of {KEYSTORE} and {DETAILS} but not the other, so the \
                     release key in it cannot be used.\n  \
                     Restore the missing file from your backup. It is not made again: a new key \
                     cannot update an application the old one installed."
                ),
            ));
        }
    }

    let text = std::fs::read_to_string(&details_path)
        .map_err(|e| NativeError::io("android", &details_path, e))?;
    let details: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        NativeError::at("android", &details_path, format!("is not valid JSON: {e}"))
    })?;
    let field = |name: &str| {
        details[name]
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| NativeError::at("android", &details_path, format!("has no `{name}`.")))
    };

    Ok(Some(SigningKey::new(
        keystore,
        field("storePassword")?,
        field("alias")?,
        field("keyPassword")?,
    )))
}

/// This machine's key for the application, made now if there is none yet.
pub(crate) fn machine_key(
    project: &Project,
    config: &NativeConfig,
) -> Result<SigningKey, NativeError> {
    let dir = key_dir(&config.identifier).ok_or_else(|| {
        NativeError::new(
            "android",
            "there is no folder on this machine to keep a release key in (no APPDATA, HOME or \
             RAHTI_NATIVE_KEY_DIR).\n  \
             Set RAHTI_NATIVE_KEY_DIR, or configure your own key with \
             RAHTI_NATIVE_ANDROID_KEYSTORE.",
        )
    })?;

    if let Some(key) = load(&dir)? {
        println!(
            "  android: signed with this machine's release key in {}",
            dir.display()
        );
        return Ok(key);
    }

    let java = doctor::java(project);
    let home = java.home.ok_or_else(|| {
        NativeError::new(
            "android",
            "no JDK to make a release key with — run `cargo rahti native doctor`.",
        )
    })?;
    create(&dir, &keytool(&home), &config.product_name)?;

    let key = load(&dir)?.ok_or_else(|| {
        NativeError::at(
            "android",
            &dir,
            "keytool reported success but wrote no key.",
        )
    })?;

    println!();
    println!("  android: no RAHTI_NATIVE_ANDROID_KEYSTORE was set, so this machine made a");
    println!(
        "  release key for {} and signed with it:",
        config.identifier
    );
    println!("    {}", dir.display());
    println!();
    println!("  Back that folder up and keep it private. Every update to an installed copy");
    println!("  of this application has to be signed with this same key; without it, users");
    println!("  have to uninstall to update. On Google Play it is your upload key.");
    println!();
    Ok(key)
}

/// `keytool` in a JDK.
fn keytool(java_home: &Path) -> PathBuf {
    java_home.join(if cfg!(windows) {
        "bin/keytool.exe"
    } else {
        "bin/keytool"
    })
}

/// Make the key in `dir`. Never replaces one.
fn create(dir: &Path, keytool: &Path, product_name: &str) -> Result<(), NativeError> {
    std::fs::create_dir_all(dir).map_err(|e| NativeError::io("android", dir, e))?;
    let keystore = dir.join(KEYSTORE);
    if keystore.exists() {
        return Err(NativeError::at(
            "android",
            &keystore,
            "a keystore is already here, and is not replaced.",
        ));
    }

    // PKCS12, where the key's password is the store's.
    let password = password()?;

    // The password reaches keytool through its environment, not its
    // arguments, which any process on the machine can list.
    const PASSWORD_VAR: &str = "RAHTI_NATIVE_KEYTOOL_PASSWORD";
    let output = Command::new(keytool)
        .env(PASSWORD_VAR, &password)
        .args(["-genkeypair", "-noprompt", "-storetype", "PKCS12"])
        .arg("-keystore")
        .arg(&keystore)
        .args(["-alias", ALIAS, "-keyalg", "RSA", "-keysize", "4096"])
        // About 27 years. Google Play requires validity past 2033.
        .args(["-validity", "10000"])
        .args(["-storepass:env", PASSWORD_VAR, "-keypass:env", PASSWORD_VAR])
        .arg("-dname")
        .arg(distinguished_name(product_name))
        .output()
        .map_err(|e| {
            NativeError::at(
                "android",
                keytool,
                format!("could not run keytool to make a release key: {e}"),
            )
        })?;

    if !output.status.success() || !keystore.is_file() {
        let _ = std::fs::remove_file(&keystore);
        return Err(NativeError::new(
            "android",
            format!(
                "keytool could not make a release key:\n  {}{}",
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }

    let details = serde_json::json!({
        "about": "The release key `cargo rahti native` made for this application. Keep this \
                  folder private and backed up: every update must be signed with this key.",
        "alias": ALIAS,
        "storePassword": password,
        "keyPassword": password,
    });
    let details_path = dir.join(DETAILS);
    std::fs::write(
        &details_path,
        serde_json::to_string_pretty(&details).unwrap_or_default(),
    )
    .map_err(|e| NativeError::io("android", &details_path, e))?;
    restrict(&details_path);
    restrict(&keystore);
    Ok(())
}

/// The certificate's subject: the product name, with the characters an X.500
/// name treats as syntax escaped.
pub(crate) fn distinguished_name(product_name: &str) -> String {
    let mut name = String::new();
    for c in product_name.chars() {
        if matches!(c, ',' | '+' | '"' | '\\' | '<' | '>' | ';' | '=') {
            name.push('\\');
        }
        name.push(c);
    }
    let name = if name.trim().is_empty() {
        "Rahti application".to_string()
    } else {
        name
    };
    format!("CN={name}, O={name}")
}

/// 48 hexadecimal characters from the operating system's generator: 192 bits,
/// and nothing a properties file or a shell treats as syntax.
pub(crate) fn password() -> Result<String, NativeError> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|e| {
        NativeError::new(
            "android",
            format!("the operating system's random generator failed: {e}"),
        )
    })?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Readable by this user only, where the platform has such a thing. On
/// Windows the folder is already inside the user's own profile.
fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

//! `rahti.native.json` — what a project's native packages are called.
//!
//! Separate from `rahti.config.json` for the reason native support is opt-in
//! at all: a web project should not carry a file describing packages it does
//! not build, and `cargo rahti native init` is what puts this one there.
//!
//! ## What it holds, and what it must not
//!
//! Names, sizes and identifiers — the things every build of the application
//! produces the same values for, which is exactly what belongs in a committed
//! file.
//!
//! Not: signing passwords, keystore paths that carry credentials, the
//! `AUTH_SECRET`, or anything per-installation. Signing is configured through
//! the environment (see `native-packaging.md`), and the session key is
//! generated on the device at first launch — see [`crate::session_secret`].
//!
//! `auth.cookieName` is the one value that looks like it might be a credential
//! and is not. A cookie *name* is public: it is in every response header the
//! application sends. It is recorded so that a packaged build keeps the
//! per-project name `cargo rahti new` generated instead of falling back to the
//! framework default, which would be a different cookie and therefore a
//! different session. `AUTH_COOKIE_NAME` still comes from the environment
//! everywhere else; this is what puts it in the environment of a process that
//! has no `.env` to read.
//!
//! ## Validation before expense
//!
//! Everything here is checked before a build starts. An Android build that
//! fails on a malformed application identifier fails after Gradle has been
//! downloaded, a Rust target has been compiled and several minutes have
//! passed. The same failure costs nothing if it happens at the point the file
//! is read.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::NativeError;

/// The version of this file format that this build understands.
pub const SCHEMA_VERSION: u32 = 1;

/// The `--target` values `cargo rahti native` accepts.
pub const TARGETS: &[&str] = &["windows", "android"];

/// The lowest Android API level Tauri 2 supports.
pub const MIN_ANDROID_SDK: u32 = 24;

/// A project's native packaging configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeConfig {
    /// Editor support. Written by `init`, ignored on read.
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema_ref: Option<String>,

    /// The format version. Refused rather than guessed at when it is not one
    /// this build knows: a field that changed meaning is worse than a file
    /// that will not load.
    pub schema: u32,

    /// What the installed application is called.
    pub product_name: String,

    /// Reverse-DNS. The Windows bundle identity and the Android package name.
    pub identifier: String,

    /// `major.minor.patch`. Numeric because both platforms require it.
    pub version: String,

    /// Which packages this project builds.
    pub targets: Vec<String>,

    #[serde(default)]
    pub window: WindowConfig,

    #[serde(default)]
    pub android: AndroidConfig,

    #[serde(default)]
    pub bundle: BundleConfig,

    #[serde(default)]
    pub database: DatabaseConfig,

    #[serde(default)]
    pub auth: AuthConfig,

    /// Depend on a Rahti checkout by path rather than by version.
    ///
    /// For working on the framework itself, and the same idea as
    /// `cargo rahti new --local`. Recorded rather than passed each time
    /// because `native/Cargo.toml` is regenerated from this file, and a run
    /// that forgot the flag would quietly move the shell onto a published
    /// version of a crate that is being changed locally.
    ///
    /// The path is relative to the project root, or absolute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<String>,

    #[serde(default)]
    pub security: SecurityConfig,

    /// Content hashes of the generated files under `native/`, written by
    /// `cargo rahti native init`.
    ///
    /// How a later run knows which of them you have edited — and therefore
    /// which it must leave alone. Bookkeeping rather than configuration: it is
    /// in this file because this file is already the one a project commits,
    /// and a second file holding nine hashes would be a second file.
    ///
    /// Empty is not written out, so a hand-written configuration does not
    /// grow a section it never asked for.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub scaffold: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    /// Whether the window may be resized. Ignored on Android, which has no
    /// window to resize.
    #[serde(default = "yes")]
    pub resizable: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        WindowConfig {
            title: "Rahti".to_string(),
            width: 1200,
            height: 800,
            resizable: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidConfig {
    /// The lowest API level the package installs on.
    pub min_sdk: u32,
}

impl Default for AndroidConfig {
    fn default() -> Self {
        AndroidConfig {
            min_sdk: MIN_ANDROID_SDK,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleConfig {
    /// Where the launcher icons live, relative to the project root.
    pub icons: String,
}

impl Default for BundleConfig {
    fn default() -> Self {
        BundleConfig {
            icons: "native/icons".to_string(),
        }
    }
}

/// What a packaged application does about its database.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DatabaseMode {
    /// A SQLite file in application storage, created on first launch.
    ///
    /// The only backend that can run inside the package, because it is the
    /// only one that is a file rather than a server.
    SqliteLocal,
    /// The application's `DATABASE_URL` is left exactly as it is.
    ///
    /// What a project on PostgreSQL or MySQL gets. Its database is somewhere
    /// else and stays there; a packaged application is a client of it, and
    /// rewriting the connection string to a local SQLite file would start the
    /// application against an empty database that looked like a working one.
    Remote,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatabaseConfig {
    pub mode: DatabaseMode,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        DatabaseConfig {
            mode: DatabaseMode::SqliteLocal,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthConfig {
    /// The project's `AUTH_COOKIE_NAME`. A name, never a key — see the module
    /// note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cookie_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityConfig {
    /// Whether the embedded server refuses requests that did not come from
    /// this launch of this application. See [`crate::gate`].
    #[serde(default = "yes")]
    pub loopback_token: bool,

    /// The Content-Security-Policy the embedded server serves.
    ///
    /// The default is as narrow as PulsePoint will run under, which is one
    /// directive wider than it looks like it should be — see [`default_csp`].
    /// A project that adds an external script widens it here, and knows it
    /// did.
    #[serde(default = "default_csp")]
    pub csp: String,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        SecurityConfig {
            loopback_token: true,
            csp: default_csp(),
        }
    }
}

/// `default-src 'self'` and nothing external.
///
/// ## `'unsafe-eval'`, and why it is not optional here
///
/// PulsePoint compiles the expressions in a reactive block at runtime: it
/// parses them and builds a render function with `new Function`, which the CSP
/// counts as evaluating a string as JavaScript. That is what makes it a
/// browser runtime rather than a build step, and it is not something a
/// configuration option turns off.
///
/// Without the directive the page still renders — the document is
/// server-rendered — and every binding on it is dead, reporting
/// `EvalError: … violates the following Content Security Policy directive`
/// from inside the minified bundle. Which is a failure worth naming, because
/// the sentence "tighten the CSP" is otherwise an obvious-looking change that
/// breaks the whole browser layer of the application in a way that reads as a
/// PulsePoint bug.
///
/// It is narrower than it sounds: `script-src` still refuses every *source*
/// but this origin, so injected markup cannot load an attacker's file. What it
/// permits is the application's own runtime compiling the application's own
/// expressions.
///
/// The rest: `connect-src` covers `pp.rpc`, the streaming responses and the
/// WebSocket — `ws:` because the socket is on the loopback origin, which is
/// not TLS. `img-src data:` is what an inline SVG data URI needs;
/// `style-src 'unsafe-inline'` is what a `style` attribute needs, which
/// PulsePoint writes when a binding targets one.
pub fn default_csp() -> String {
    "default-src 'self'; \
     script-src 'self' 'unsafe-eval'; \
     style-src 'self' 'unsafe-inline'; \
     img-src 'self' data: blob:; \
     font-src 'self' data:; \
     connect-src 'self' ws: http://127.0.0.1:*; \
     frame-ancestors 'none'; \
     object-src 'none'; \
     base-uri 'self'"
        .to_string()
}

/// Policies a previous Rahti wrote as its default, and would write differently
/// now.
///
/// `security.csp` is stored, so a project keeps whatever it was created with —
/// right for a value somebody tuned, wrong for one nobody touched. Without this
/// list a framework-level correction to the default could never reach an
/// existing project, and the correction that prompted the list is not
/// cosmetic: the entry below has no `'unsafe-eval'`, so every PulsePoint
/// binding in a package built with it is dead.
///
/// An entry is matched byte for byte. A policy that differs by so much as a
/// space is one somebody edited, and is left exactly as it is.
const SUPERSEDED_CSPS: &[&str] = &[
    // Shipped before it was found that PulsePoint compiles its reactive
    // expressions with `new Function`.
    "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
     img-src 'self' data: blob:; font-src 'self' data:; \
     connect-src 'self' ws: http://127.0.0.1:*; frame-ancestors 'none'; \
     object-src 'none'; base-uri 'self'",
];

/// Whether `csp` is an old default this build would now write differently.
///
/// Used by `cargo rahti native init` to bring an untouched policy forward, the
/// same way an unedited scaffold file is regenerated.
pub fn superseded_csp(csp: &str) -> bool {
    SUPERSEDED_CSPS.contains(&csp)
}

fn yes() -> bool {
    true
}

impl NativeConfig {
    /// A configuration for a project that has just run `init`.
    pub fn new(product_name: &str, identifier: &str, version: &str, targets: &[&str]) -> Self {
        NativeConfig {
            schema_ref: Some("./rahti.native.schema.json".to_string()),
            schema: SCHEMA_VERSION,
            product_name: product_name.to_string(),
            identifier: identifier.to_string(),
            version: version.to_string(),
            targets: targets.iter().map(|t| t.to_string()).collect(),
            window: WindowConfig {
                title: product_name.to_string(),
                ..WindowConfig::default()
            },
            android: AndroidConfig::default(),
            bundle: BundleConfig::default(),
            database: DatabaseConfig::default(),
            auth: AuthConfig::default(),
            local: None,
            security: SecurityConfig::default(),
            scaffold: BTreeMap::new(),
        }
    }

    /// Read and validate the file at `path`.
    pub fn load(path: &Path) -> Result<Self, NativeError> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                NativeError::at(
                    "config",
                    path,
                    "this project has no native configuration.\n  \
                     Create one with:\n    \
                     cargo rahti native init --identifier com.example.myapp --windows",
                )
            } else {
                NativeError::io("config", path, e)
            }
        })?;

        Self::parse(&text).map_err(|mut e| {
            e.path = Some(path.to_path_buf());
            e
        })
    }

    /// Parse, migrate, and validate, without a file.
    ///
    /// Migration comes before validation on purpose. A stored value this build
    /// would now write differently is corrected here, so every caller — `init`,
    /// `doctor`, `build` — sees the corrected configuration, and a project
    /// created by an older Rahti is not refused by a rule that did not exist
    /// when its file was written. See [`superseded_csp`].
    pub fn parse(text: &str) -> Result<Self, NativeError> {
        let mut config: NativeConfig = serde_json::from_str(text).map_err(|e| {
            NativeError::new("config", format!("rahti.native.json is not valid: {e}"))
        })?;
        config.migrate();
        config.validate()?;
        Ok(config)
    }

    /// Bring stored values this build owns forward.
    ///
    /// Only values that are byte-identical to something a previous Rahti wrote
    /// as *its* default. Anything somebody edited is theirs and is untouched.
    fn migrate(&mut self) {
        if superseded_csp(&self.security.csp) {
            self.security.csp = default_csp();
        }
    }

    /// Serialize, formatted the way `init` writes it.
    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("a configuration serializes");
        text.push('\n');
        text
    }

    /// Every rule, checked in one place and before anything expensive runs.
    pub fn validate(&self) -> Result<(), NativeError> {
        let fail = |message: String| NativeError::new("config", message);

        if self.schema != SCHEMA_VERSION {
            return Err(fail(format!(
                "rahti.native.json has `schema` {}, and this tool understands {SCHEMA_VERSION}.\n  \
                 Upgrade cargo-rahti-native, or regenerate the file with `cargo rahti native init`.",
                self.schema
            )));
        }

        check_product_name(&self.product_name).map_err(fail)?;
        check_identifier(&self.identifier).map_err(fail)?;
        check_version(&self.version).map_err(fail)?;

        if self.targets.is_empty() {
            return Err(fail(
                "`targets` is empty, so there is nothing to build.\n  \
                 Add \"windows\", \"android\", or both."
                    .to_string(),
            ));
        }
        for target in &self.targets {
            if !TARGETS.contains(&target.as_str()) {
                return Err(fail(format!(
                    "`{target}` is not a native target. Rahti packages {}.",
                    TARGETS.join(" and ")
                )));
            }
        }

        if self.window.width == 0 || self.window.height == 0 {
            return Err(fail(
                "a window with a zero dimension has nothing to show.".to_string(),
            ));
        }

        if self.android.min_sdk < MIN_ANDROID_SDK {
            return Err(fail(format!(
                "`android.minSdk` is {}, and Tauri 2 needs at least {MIN_ANDROID_SDK}.",
                self.android.min_sdk
            )));
        }

        if let Some(cookie) = &self.auth.cookie_name {
            check_cookie_name(cookie).map_err(fail)?;
        }

        if self.security.csp.trim().is_empty() {
            return Err(fail(
                "`security.csp` is empty. A package that serves no Content-Security-Policy \
                 puts an XSS in reach of the native command bridge — set a policy, or remove \
                 the field to take the default."
                    .to_string(),
            ));
        }

        // A policy without `'unsafe-eval'` is one the application cannot run
        // under, and the runtime symptom is bad: the server-rendered page
        // appears, every binding on it is dead, and the only error is an
        // `EvalError` from inside a minified bundle. Cheaper to say here.
        let scripts = self
            .security
            .csp
            .split(';')
            .map(str::trim)
            .find(|directive| directive.starts_with("script-src"));
        if let Some(scripts) = scripts {
            if !scripts.contains("'unsafe-eval'") {
                return Err(fail(format!(
                    "`security.csp` has `{scripts}`, and PulsePoint cannot run under it.\n  \
                     It compiles the expressions in a reactive block at runtime, with \
                     `new Function` — which a Content-Security-Policy counts as evaluating a \
                     string as JavaScript.\n  \
                     Without `'unsafe-eval'` the page renders and every binding on it is \
                     dead, reporting an EvalError from inside the runtime bundle.\n  \
                     Add `'unsafe-eval'` to `script-src`. It stays narrow: `script-src` still \
                     refuses every *source* but this origin, so injected markup cannot load \
                     an attacker's file."
                )));
            }
        }

        Ok(())
    }

    /// Whether `target` is one this project asked for.
    pub fn builds(&self, target: &str) -> bool {
        self.targets.iter().any(|t| t == target)
    }

    /// The integer Google Play orders releases by.
    ///
    /// Derived rather than stored, so there is one version in the file and no
    /// way for the two to disagree. `1.2.3` becomes `10203`, which increases
    /// with the version for any patch or minor below 100.
    pub fn android_version_code(&self) -> u32 {
        let mut parts = self
            .version
            .split('.')
            .map(|p| p.parse::<u32>().unwrap_or(0));
        let major = parts.next().unwrap_or(0);
        let minor = parts.next().unwrap_or(0);
        let patch = parts.next().unwrap_or(0);
        major * 10_000 + minor * 100 + patch
    }
}

/// Reverse-DNS, and legal as an Android package name.
///
/// Android's rule is the strict one and is therefore the one enforced: a
/// package name is a Java package name, so every segment is a Java identifier.
/// A hyphen is the common mistake — `com.example.my-app` is a perfectly good
/// Windows bundle identity and will not compile on Android, several minutes
/// into a Gradle build that had no reason to start.
pub fn check_identifier(identifier: &str) -> Result<(), String> {
    let advice = "  An identifier is reverse-DNS and has to be a legal Android package name: \
                  at least two segments, each starting with a letter and made of letters, \
                  digits and `_`. No hyphens.\n  \
                  For example: com.example.myapp";

    if identifier.trim() != identifier || identifier.is_empty() {
        return Err(format!(
            "`{identifier}` is not an application identifier.\n{advice}"
        ));
    }

    let segments: Vec<&str> = identifier.split('.').collect();
    if segments.len() < 2 {
        return Err(format!(
            "`{identifier}` has one segment, and an identifier needs at least two.\n{advice}"
        ));
    }

    for segment in &segments {
        if segment.is_empty() {
            return Err(format!("`{identifier}` has an empty segment.\n{advice}"));
        }
        if !segment.starts_with(|c: char| c.is_ascii_alphabetic()) {
            return Err(format!(
                "`{identifier}`: the segment `{segment}` does not start with a letter.\n{advice}"
            ));
        }
        if !segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(format!(
                "`{identifier}`: the segment `{segment}` has a character that is not a letter, \
                 a digit or `_`.\n{advice}"
            ));
        }
        if JAVA_KEYWORDS.contains(segment) {
            return Err(format!(
                "`{identifier}`: `{segment}` is a Java keyword, which an Android package name \
                 cannot contain.\n{advice}"
            ));
        }
    }

    // Tauri's own placeholder. Two applications sharing it share an
    // installation on Android, and the second one to install replaces the
    // first.
    if identifier == "com.tauri.dev" {
        return Err(
            "`com.tauri.dev` is Tauri's placeholder identifier, and every application using it \
             would replace every other one on the device.\n  \
             Use your own reverse-DNS identifier, for example com.example.myapp."
                .to_string(),
        );
    }

    Ok(())
}

/// Segments an Android package name cannot use.
const JAVA_KEYWORDS: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "try",
    "void",
    "volatile",
    "while",
];

/// The installed application's name, which is also a filename.
pub fn check_product_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err(
            "`productName` is empty, and it is what the installed application is \
                    called."
                .to_string(),
        );
    }
    if name.trim() != name {
        return Err(format!(
            "`productName` is `{name}`, which has leading or trailing whitespace. It becomes a \
             filename, where that does not survive."
        ));
    }
    // It reaches an installer path, a Start-menu entry and an APK label.
    const FORBIDDEN: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
    if let Some(bad) = name
        .chars()
        .find(|c| FORBIDDEN.contains(c) || c.is_control())
    {
        return Err(format!(
            "`productName` contains `{bad}`, which cannot be in a filename — and the product \
             name becomes one."
        ));
    }
    Ok(())
}

/// `major.minor.patch`, all numeric.
///
/// Neither platform takes anything else. An MSI version is three numbers, and
/// Google Play orders releases by an integer derived from these; a `-beta.1`
/// suffix has nowhere to go in either.
pub fn check_version(version: &str) -> Result<(), String> {
    let parts: Vec<&str> = version.split('.').collect();
    let numeric = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));

    if !numeric {
        return Err(format!(
            "`version` is `{version}`, and a native package needs `major.minor.patch` with all \
             three numeric.\n  \
             A Windows installer version is three numbers, and Google Play orders releases by an \
             integer derived from them — a pre-release suffix has nowhere to go in either."
        ));
    }

    for part in parts {
        if part.parse::<u32>().is_err() {
            return Err(format!(
                "`version` is `{version}`, and `{part}` is too large."
            ));
        }
    }
    Ok(())
}

/// An RFC 6265 cookie name, checked here for the same reason `rahti::auth`
/// checks it: a name carrying `;` or `=` is written and never read back, and
/// every sign-in appears to work while nobody stays signed in.
fn check_cookie_name(name: &str) -> Result<(), String> {
    const SEPARATORS: &[char] = &[
        '(', ')', '<', '>', '@', ',', ';', ':', '\\', '"', '/', '[', ']', '?', '=', '{', '}', ' ',
    ];
    if name.is_empty() {
        return Err("`auth.cookieName` is empty.".to_string());
    }
    if name
        .chars()
        .any(|c| c.is_control() || SEPARATORS.contains(&c) || !c.is_ascii())
    {
        return Err(format!(
            "`auth.cookieName` is `{name}`, which is not a legal cookie name.\n  \
             It has to be a token: letters, digits, and `-_.~!#$%&'*+^|`."
        ));
    }
    Ok(())
}

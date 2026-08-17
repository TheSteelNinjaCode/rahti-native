//! Finding the Rahti project, and the few facts about it the shell needs.
//!
//! A native package is built from a project that already exists, so nothing
//! here creates one. What it does is answer three questions: where the project
//! root is, what its library crate is called (the shell links it), and what
//! version it is at.

use std::path::{Path, PathBuf};

use rahti_native::NativeError;

/// The file that makes a directory a Rahti project.
pub const MARKER: &str = "rahti.config.json";

/// The native configuration, beside it.
pub const NATIVE_CONFIG: &str = "rahti.native.json";

/// A Rahti project on disk.
#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    /// The cargo package name, as written in `Cargo.toml`.
    pub package: String,
    /// The library target the shell links — `[lib] name`, or the package name
    /// with its hyphens turned into underscores, which is cargo's default.
    pub lib: String,
    /// The package version, used as the default native version.
    pub version: String,
}

impl Project {
    /// Walk up from `start` looking for `rahti.config.json`.
    ///
    /// Up rather than down, because a person runs this from wherever they
    /// happen to be in the project and the root is always an ancestor.
    pub fn discover(start: &Path) -> Result<Self, NativeError> {
        let mut dir = Some(start);
        while let Some(current) = dir {
            if current.join(MARKER).is_file() {
                return Self::at(current);
            }
            dir = current.parent();
        }

        Err(NativeError::at(
            "project",
            start,
            format!(
                "this is not a Rahti project — no {MARKER} here or in any directory above it.\n  \
                 Run this from inside a project created by `cargo rahti new`."
            ),
        ))
    }

    /// Read the manifest of a known root.
    pub fn at(root: &Path) -> Result<Self, NativeError> {
        let manifest_path = root.join("Cargo.toml");
        let manifest = std::fs::read_to_string(&manifest_path)
            .map_err(|e| NativeError::io("project", &manifest_path, e))?;

        let package = table_value(&manifest, "package", "name").ok_or_else(|| {
            NativeError::at("project", &manifest_path, "this manifest names no package")
        })?;

        let version = table_value(&manifest, "package", "version")
            // `version.workspace = true` is a value this reader cannot follow
            // — it is in another file. The native version is asked for
            // explicitly in that case.
            .filter(|v| v != "true")
            .unwrap_or_else(|| "0.1.0".to_string());

        let lib =
            table_value(&manifest, "lib", "name").unwrap_or_else(|| package.replace('-', "_"));

        Ok(Project {
            root: root.to_path_buf(),
            package,
            lib,
            version,
        })
    }

    pub fn native_dir(&self) -> PathBuf {
        self.root.join("native")
    }

    pub fn native_config(&self) -> PathBuf {
        self.root.join(NATIVE_CONFIG)
    }

    pub fn native_schema(&self) -> PathBuf {
        self.root.join("rahti.native.schema.json")
    }

    /// Whether the project's library exposes the shared startup the shell
    /// calls.
    ///
    /// Checked by `doctor` rather than left to the linker, because "cannot
    /// find function `initialize_application`" arrives after Tauri has been
    /// compiled and says nothing about what to do.
    pub fn has_shared_startup(&self) -> bool {
        std::fs::read_to_string(self.root.join("src/lib.rs"))
            .is_ok_and(|source| source.contains("pub async fn initialize_application"))
    }

    /// The project's `AUTH_COOKIE_NAME`, if it has one.
    ///
    /// Read from `.env` at `init` time and recorded in `rahti.native.json` so
    /// the packaged application keeps the per-project cookie name. A name, not
    /// a key: `AUTH_SECRET` is deliberately not read here and has no field to
    /// go in.
    pub fn cookie_name(&self) -> Option<String> {
        let env = std::fs::read_to_string(self.root.join(".env")).ok()?;
        for line in env.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            if name.trim() != "AUTH_COOKIE_NAME" {
                continue;
            }
            let value = value.trim().trim_matches('"').trim_matches('\'').trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
        None
    }
}

/// One `key = "value"` from one `[table]` of a manifest.
///
/// A whole TOML parser for four strings would be a dependency doing more than
/// it is asked. This reads the first `[table]` header it finds and the keys
/// under it, stopping at the next header — enough for `[package] name`,
/// `[package] version` and `[lib] name`, and honest about being no more than
/// that: a value written with a single quote, over several lines, or inside an
/// inline table is not found, and the caller falls back.
fn table_value(manifest: &str, table: &str, key: &str) -> Option<String> {
    let mut inside = false;

    for line in manifest.lines() {
        let line = line.split('#').next().unwrap_or("").trim();

        if line.starts_with('[') {
            inside = line == format!("[{table}]");
            continue;
        }
        if !inside {
            continue;
        }

        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != key {
            continue;
        }

        let value = value.trim();
        return Some(
            match value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
                Some(quoted) => quoted.to_string(),
                None => value.to_string(),
            },
        );
    }
    None
}

#[cfg(test)]
#[path = "tests/project.rs"]
mod tests;

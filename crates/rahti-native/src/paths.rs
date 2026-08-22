//! Where an installed application's files are.
//!
//! A Rahti web project resolves everything against the working directory: the
//! public assets are `public/`, the SQLite file is whatever relative path
//! `DATABASE_URL` names, and a spilled upload goes to the system temporary
//! directory. All three are correct for a server started from its own checkout
//! and all three are wrong for an installed program, whose working directory
//! is wherever the user happened to launch it from and whose installation
//! directory is often not writable at all.
//!
//! So a packaged application resolves its paths from the operating system's
//! own application directories, once, before the router is built.
//!
//! ## The rule
//!
//! **Nothing is written to the installation directory.** Resources shipped
//! inside the package are read-only — on Android they are not even files, they
//! are entries in an APK — and a Windows installation under `Program Files` is
//! not writable by the user running it. Everything the application writes goes
//! under [`AppPaths::data`] or [`AppPaths::cache`].
//!
//! ## Where they land
//!
//! | | Windows | Android |
//! | --- | --- | --- |
//! | [`data`](AppPaths::data) | `%LOCALAPPDATA%\<identifier>` | internal files directory |
//! | [`config`](AppPaths::config) | `%APPDATA%\<identifier>` | `<files>/config` |
//! | [`cache`](AppPaths::cache) | `%LOCALAPPDATA%\<identifier>\cache` | cache directory |
//!
//! Android's two directories are handed in by the host rather than guessed:
//! only the Java side knows them, and Tauri asks it. [`AppPaths::from_host`]
//! is that constructor; [`AppPaths::resolve`] is the one that reads the
//! environment and is what a Windows package uses.
//!
//! Data against cache is the distinction the operating system acts on: Android
//! deletes a cache directory when the device is short of space, and Windows
//! roams `%APPDATA%` between machines on a domain while leaving
//! `%LOCALAPPDATA%` where it is. So the database, uploads and logs are data;
//! spilled upload parts are cache, because a spilled part exists for the
//! duration of one request and losing it costs nothing.

use std::path::{Path, PathBuf};

use crate::error::NativeError;
use crate::platform::Platform;

const PUBLIC_DIR_ENV: &str = "RAHTI_PUBLIC_DIR";
const SPILL_DIR_ENV: &str = "RAHTI_SPILL_DIR";

/// Overrides [`AppPaths::data`]. Set by an Android host, and by tests.
pub const DATA_DIR_ENV: &str = "RAHTI_NATIVE_DATA_DIR";
/// Overrides [`AppPaths::config`].
pub const CONFIG_DIR_ENV: &str = "RAHTI_NATIVE_CONFIG_DIR";
/// Overrides [`AppPaths::cache`].
pub const CACHE_DIR_ENV: &str = "RAHTI_NATIVE_CACHE_DIR";
/// Overrides [`AppPaths::resources`] — where the package's read-only files
/// were installed.
pub const RESOURCE_DIR_ENV: &str = "RAHTI_NATIVE_RESOURCE_DIR";

/// The file that records which version of the package staged the assets now in
/// internal storage.
pub const ASSET_STAMP: &str = ".rahti-assets";

/// The resolved directories of one installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    identifier: String,
    data: PathBuf,
    config: PathBuf,
    cache: PathBuf,
    resources: PathBuf,
}

impl AppPaths {
    /// The directories for `identifier`, from the environment or the platform
    /// defaults.
    ///
    /// The four `RAHTI_NATIVE_*_DIR` variables win where they are set, which
    /// is how an Android host passes in directories only the Java side knows
    /// and how a test gets a temporary tree without touching the real one.
    pub fn resolve(identifier: &str) -> Result<Self, NativeError> {
        crate::config::check_identifier(identifier)
            .map_err(|message| NativeError::new("paths", message))?;

        let resources = match env_path(RESOURCE_DIR_ENV) {
            Some(dir) => dir,
            None => default_resource_dir()?,
        };

        let data = match env_path(DATA_DIR_ENV) {
            Some(dir) => dir,
            None => default_data_dir(identifier)?,
        };

        let config = env_path(CONFIG_DIR_ENV)
            .or_else(|| default_config_dir(identifier))
            .unwrap_or_else(|| data.join("config"));

        let cache = env_path(CACHE_DIR_ENV).unwrap_or_else(|| data.join("cache"));

        Ok(AppPaths {
            identifier: identifier.to_string(),
            data,
            config,
            cache,
            resources,
        })
    }

    /// The directories the host already knows.
    ///
    /// Android's are only reachable from the Java side, so the Tauri shell
    /// asks Tauri for them and hands them over rather than this crate trying
    /// to derive something it cannot see.
    pub fn from_host(
        identifier: &str,
        data: impl Into<PathBuf>,
        cache: impl Into<PathBuf>,
        resources: impl Into<PathBuf>,
    ) -> Result<Self, NativeError> {
        crate::config::check_identifier(identifier)
            .map_err(|message| NativeError::new("paths", message))?;

        let data = data.into();
        Ok(AppPaths {
            identifier: identifier.to_string(),
            config: data.join("config"),
            cache: cache.into(),
            resources: resources.into(),
            data,
        })
    }

    /// The application identifier these paths were built for.
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// Durable application data: the database, uploads, the session key, logs.
    /// Survives until the user uninstalls or clears the application's data.
    pub fn data(&self) -> &Path {
        &self.data
    }

    /// User-visible configuration the application chooses to persist.
    pub fn config(&self) -> &Path {
        &self.config
    }

    /// Data the operating system may delete without asking.
    pub fn cache(&self) -> &Path {
        &self.cache
    }

    /// The package's read-only installed files. **Never written to.**
    pub fn resources(&self) -> &Path {
        &self.resources
    }

    /// Where the application's static assets are served from.
    ///
    /// Under `data`, not under `resources`, because Android's are not files
    /// until [`stage_public_assets`] has copied them out of the package.
    /// Windows could serve straight from the installation directory, and
    /// deliberately does not: one path that behaves the same on both platforms
    /// is worth more than one avoided copy of a few hundred kilobytes.
    pub fn public(&self) -> PathBuf {
        self.data.join("assets")
    }

    /// Where the assets are staged *from* — the copy inside the package.
    pub fn bundled_public(&self) -> PathBuf {
        self.resources.join("public")
    }

    /// The SQLite file, for an application whose database is local.
    pub fn database(&self) -> PathBuf {
        self.data.join("app.db")
    }

    /// Files the application saved on the user's behalf.
    pub fn uploads(&self) -> PathBuf {
        self.data.join("uploads")
    }

    /// Where a large upload is spooled while it arrives.
    ///
    /// Cache rather than data: the file exists for one request, and an
    /// operating system that reclaims it between requests has taken nothing.
    pub fn spill(&self) -> PathBuf {
        self.cache.join("uploads")
    }

    /// Application logs.
    pub fn logs(&self) -> PathBuf {
        self.data.join("logs")
    }

    /// Short-lived scratch files.
    pub fn temp(&self) -> PathBuf {
        self.cache.join("tmp")
    }

    /// Files the application exports for the user to keep.
    pub fn exports(&self) -> PathBuf {
        self.data.join("exports")
    }

    /// The per-installation session key.
    pub fn secret_file(&self) -> PathBuf {
        self.data.join(crate::secret::SECRET_FILE)
    }

    /// Create every directory the application writes into.
    ///
    /// Idempotent, and called before anything else reads a path — a first
    /// launch has none of them, and an SQLite file cannot be created in a
    /// directory that is not there.
    pub fn prepare(&self) -> Result<(), NativeError> {
        for dir in [
            self.data.clone(),
            self.config.clone(),
            self.cache.clone(),
            self.public(),
            self.uploads(),
            self.spill(),
            self.logs(),
            self.temp(),
            self.exports(),
        ] {
            std::fs::create_dir_all(&dir).map_err(|e| NativeError::io("paths", &dir, e))?;
        }
        Ok(())
    }

    /// A `DATABASE_URL` for the local SQLite file.
    ///
    /// Absolute, because a relative one resolves against a working directory
    /// an installed application does not control. `mode=rwc` because a first
    /// launch has no file yet.
    pub fn sqlite_url(&self) -> String {
        // Forward slashes on every platform: a backslash in a URL is not a
        // path separator, and `C:\Users\…` arrives at SQLite as one long
        // filename with no directories in it.
        let path = self.database().display().to_string().replace('\\', "/");
        format!("sqlite://{path}?mode=rwc")
    }

    /// Put the resolved paths where the application will read them.
    ///
    /// Called *before* `initialize_application`, because the generated router
    /// reads `RAHTI_PUBLIC_DIR` while it is being built and an upload reads
    /// `RAHTI_SPILL_DIR` on the first request that spills.
    ///
    /// `public` is passed rather than taken from [`Self::public`] because it
    /// is the one path that is not always the packaged one: a `cargo rahti
    /// native dev` run has no bundle to stage from and serves the project's
    /// own `public/` directly, which is also what makes a stylesheet edit
    /// visible without a rebuild. See [`resolve_public`].
    ///
    /// Set rather than defaulted: `std::env::set_var` overwrites, and that is
    /// deliberate. An installed application inherits the environment of
    /// whoever launched it, and a developer with `RAHTI_PUBLIC_DIR` exported
    /// for their own checkout would otherwise have a shipped application
    /// serving assets out of their source tree.
    pub fn apply_environment(&self, public: &Path) {
        // SAFETY: called by the native host before any task is spawned and
        // before the router is built — the same single-threaded moment `main`
        // sets anything else.
        unsafe {
            std::env::set_var(PUBLIC_DIR_ENV, public);
            std::env::set_var(SPILL_DIR_ENV, self.spill());
        }
    }

    /// Point `DATABASE_URL` at the local SQLite file.
    ///
    /// Separate from [`apply_environment`](Self::apply_environment) because it
    /// is the one path decision a native package must not make on the
    /// application's behalf. A project on PostgreSQL or MySQL has a database
    /// somewhere else, and silently rewriting its connection string to a local
    /// SQLite file would start the application against an empty database that
    /// looks like a working one. See [`crate::DatabaseMode`].
    pub fn apply_sqlite_database_url(&self) {
        // SAFETY: as above.
        unsafe {
            std::env::set_var("DATABASE_URL", self.sqlite_url());
        }
    }
}

/// One file of the application's `public/`, compiled into the binary.
pub struct EmbeddedAsset<'a> {
    /// The path below `public/`, with `/` separators — `js/main.js`.
    pub path: &'a str,
    pub bytes: &'a [u8],
}

/// Write the embedded assets into internal storage, once per asset version.
///
/// ## Why the assets are in the binary
///
/// Because on Android there is no other portable way to get at them.
///
/// Tauri's bundler does copy `bundle.resources` into an Android package — into
/// the APK's `assets/`, which is a zip entry rather than a file. And
/// `app.path().resource_dir()` on Android does not return a directory at all:
/// it returns the string `asset://localhost/`. So a `ServeDir` pointed at it
/// serves nothing, `Path::is_dir` on it is false, and an application built
/// that way starts, binds its port, opens its window, and 404s every
/// stylesheet and the entire PulsePoint runtime.
///
/// Reading them out of the APK instead would mean the Android AssetManager,
/// which means JNI, which means the platform-neutral half of this crate would
/// stop being platform-neutral.
///
/// Embedding sidesteps all of it. The bytes are in the executable on both
/// platforms, they are written to application storage on first launch, and one
/// code path serves both. It costs the size of `public/` in the binary, which
/// for a stylesheet, a runtime bundle and a favicon is a fair price for the
/// alternative being "does not work".
///
/// Returns `true` when it wrote, `false` when the staged copy was already
/// current. The version stamp and the replace-whole-tree rule are
/// [`stage_public_assets`]'s, for the same reasons.
pub fn stage_embedded_assets(
    assets: &[EmbeddedAsset<'_>],
    destination: &Path,
    version: &str,
) -> Result<bool, NativeError> {
    let stamp = destination.join(ASSET_STAMP);

    if std::fs::read_to_string(&stamp).is_ok_and(|current| current.trim() == version.trim()) {
        return Ok(false);
    }

    if assets.is_empty() {
        return Err(NativeError::at(
            "assets",
            destination,
            "this package embeds no static assets, so every stylesheet and the browser \
             runtime would 404.\n  \
             The shell embeds the project's `public/` at compile time — check that the \
             directory exists and is not empty.",
        ));
    }

    if destination.exists() {
        std::fs::remove_dir_all(destination)
            .map_err(|e| NativeError::io("assets", destination, e))?;
    }

    for asset in assets {
        // Rejected rather than sanitized: these paths come from the project's
        // own directory at compile time, so anything climbing out of the
        // destination is a bug in the shell rather than input to defend
        // against — and writing it anyway would put a file somewhere nobody
        // asked for.
        if asset.path.contains("..") || Path::new(asset.path).is_absolute() {
            return Err(NativeError::at(
                "assets",
                asset.path,
                "an embedded asset path leaves the asset directory",
            ));
        }

        let target = destination.join(asset.path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| NativeError::io("assets", parent, e))?;
        }
        std::fs::write(&target, asset.bytes).map_err(|e| NativeError::io("assets", &target, e))?;
    }

    std::fs::write(&stamp, version).map_err(|e| NativeError::io("assets", &stamp, e))?;
    Ok(true)
}

/// Copy the package's public assets into internal storage, once per asset version.
///
/// Android needs this: the files in an APK are entries in a zip, not paths a
/// `ServeDir` can open. Windows does not need it and does it anyway, so that
/// one code path serves both.
///
/// Returns `true` when it copied, `false` when the staged copy was already
/// current.
///
/// ## What an upgrade does
///
/// The staged tree is *replaced*, not merged. A framework-owned asset that was
/// renamed or deleted between versions would otherwise sit in internal storage
/// forever, and a stale `pp-reactive-v2.min.js` beside a current `main.js` is
/// a runtime that fails in ways nothing explains.
///
/// Replacing is safe because the destination is the application's asset
/// directory and nothing else — [`AppPaths::public`] is a subdirectory of
/// `data`, not `data` itself. The database, the uploads, the logs and the
/// session key are siblings of it and are never touched.
pub fn stage_public_assets(
    source: &Path,
    destination: &Path,
    version: &str,
) -> Result<bool, NativeError> {
    let stamp = destination.join(ASSET_STAMP);

    if std::fs::read_to_string(&stamp).is_ok_and(|current| current.trim() == version.trim()) {
        return Ok(false);
    }

    if !source.is_dir() {
        return Err(NativeError::at(
            "assets",
            source,
            "the package has no public assets to stage",
        ));
    }

    if destination.exists() {
        std::fs::remove_dir_all(destination)
            .map_err(|e| NativeError::io("assets", destination, e))?;
    }
    copy_tree(source, destination)?;

    std::fs::write(&stamp, version).map_err(|e| NativeError::io("assets", &stamp, e))?;
    Ok(true)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), NativeError> {
    std::fs::create_dir_all(destination).map_err(|e| NativeError::io("assets", destination, e))?;

    let entries = std::fs::read_dir(source).map_err(|e| NativeError::io("assets", source, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| NativeError::io("assets", source, e))?;
        let from = entry.path();
        let to = destination.join(entry.file_name());

        let kind = entry
            .file_type()
            .map_err(|e| NativeError::io("assets", &from, e))?;
        if kind.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| NativeError::io("assets", &from, e))?;
        }
    }
    Ok(())
}

/// A directory named by an environment variable, with blank counted as unset.
fn env_path(name: &str) -> Option<PathBuf> {
    let value = std::env::var(name).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| PathBuf::from(value))
}

/// `%LOCALAPPDATA%\<identifier>` on Windows.
///
/// Local rather than roaming: a database and a cache of uploads are not things
/// to copy across a domain at every sign-in, and `%APPDATA%` is where a
/// managed network would put them.
fn default_data_dir(identifier: &str) -> Result<PathBuf, NativeError> {
    match Platform::current() {
        Platform::Windows => Ok(required_env("LOCALAPPDATA")?.join(identifier)),
        Platform::Android => Err(NativeError::new(
            "paths",
            format!(
                "an Android package must be told where its files are: set {DATA_DIR_ENV}, \
                 or build the paths with `AppPaths::from_host`.\n  \
                 Only the Java side knows the internal files directory, so it cannot be \
                 derived here."
            ),
        )),
        // Not a packaging target. Resolved anyway so that this crate's tests
        // run on a machine that is neither.
        Platform::Other => {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            Ok(home.join(".local/share").join(identifier))
        }
    }
}

/// `%APPDATA%\<identifier>` on Windows, and nothing anywhere else — the
/// caller falls back to `data/config`.
fn default_config_dir(identifier: &str) -> Option<PathBuf> {
    match Platform::current() {
        Platform::Windows => std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|dir| dir.join(identifier)),
        _ => None,
    }
}

/// Where the package's own files were installed: the directory holding the
/// executable.
///
/// A Tauri host overrides this with its resolved resource directory, which is
/// the same place on Windows and a very different one on Android.
fn default_resource_dir() -> Result<PathBuf, NativeError> {
    let exe = std::env::current_exe()
        .map_err(|e| NativeError::new("paths", format!("cannot locate the executable: {e}")))?;
    Ok(exe
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".")))
}

fn required_env(name: &str) -> Result<PathBuf, NativeError> {
    std::env::var_os(name).map(PathBuf::from).ok_or_else(|| {
        NativeError::new(
            "paths",
            format!("{name} is not set, so there is nowhere to keep this application's data"),
        )
    })
}

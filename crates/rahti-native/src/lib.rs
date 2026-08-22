//! Running a Rahti application inside a native package.
//!
//! A native Rahti package is the application it already was. The Rust backend
//! is compiled for the target platform and runs inside the installed program;
//! the generated Axum router answers on a loopback socket; and the operating
//! system's WebView loads the same server-rendered HTML, the same PulsePoint
//! bundle, the same RPCs and the same WebSockets it would over the network.
//!
//! **This is not a compiler from HTML to native widgets.** Nothing here
//! translates markup. What the user sees is a WebView, and calling its
//! contents native controls would be untrue.
//!
//! ## What this crate is, and is not
//!
//! It is the platform-neutral half: where a packaged application's files live,
//! how its server binds, how its session key survives a restart, and which
//! native commands its JavaScript may call. It has **no Tauri dependency**, so
//! it compiles and tests in an ordinary `cargo test --workspace` run.
//!
//! The Tauri half is generated into the application's own `native/` directory
//! by `cargo rahti native init`. That shell owns the identifier, the icons, the
//! permissions and the window — application decisions, in application files.
//!
//! It is also not the application's startup. Connecting a database, applying
//! migrations and installing an auth policy are decisions a project makes in
//! its own `src/lib.rs`; this crate starts a server, it does not start *your*
//! server.
//!
//! ## The shape of a launch
//!
//! ```no_run
//! # use rahti_native::{AppPaths, EmbeddedServer, NativeError};
//! # async fn example(router: axum::Router) -> Result<(), NativeError> {
//! // 1. Where this installation keeps its files.
//! let paths = AppPaths::resolve("com.example.myapp")?;
//! paths.prepare()?;
//!
//! // 2. Write the embedded assets into internal storage, and put the paths
//! //    in the environment the application is about to read.
//! let assets = [rahti_native::EmbeddedAsset { path: "js/main.js", bytes: b"" }];
//! rahti_native::stage_embedded_assets(&assets, &paths.public(), "1.0.0")?;
//! paths.apply_environment(&paths.public());
//!
//! // 3. Bind first, so the port is real before anything is told to go there.
//! let server = EmbeddedServer::bind().await?;
//! let url = server.base_url();
//!
//! // 4. Serve, then create the window. Never the other way round.
//! let running = server.serve(router);
//! running.wait_until_ready().await?;
//! // open_webview(&url);
//!
//! # let _ = (url, running);
//! # Ok(())
//! # }
//! ```
//!
//! Step 3 before step 4 is the whole of the startup race: [`EmbeddedServer`]
//! has no constructor that produces a URL it is not already listening on.

// Framework surface: exported for native shells to use, so "nothing in this
// crate calls it yet" is not a defect.
#![allow(dead_code)]

mod bridge;
mod capabilities;
mod config;
mod error;
mod gate;
mod headers;
mod paths;
mod platform;
mod secret;
mod server;

const DEV_ENV: &str = "RAHTI_DEV";

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;

pub use bridge::{BRIDGE_PATH, bridge_route, bridge_script};
pub use capabilities::{Capability, NativeCommand, commands, is_allowed, is_external_url};
pub use config::{
    AndroidConfig, AuthConfig, BundleConfig, DatabaseConfig, DatabaseMode, MIN_ANDROID_SDK,
    NativeConfig, SCHEMA_VERSION, SecurityConfig, TARGETS, WindowConfig, check_identifier,
    check_product_name, check_version, default_csp, superseded_csp,
};
pub use error::NativeError;
pub use gate::{LAUNCH_PARAM, LaunchToken, gate, launch_token};
pub use headers::{csp, install_csp, secure, security_headers};
pub use paths::{ASSET_STAMP, AppPaths, EmbeddedAsset, stage_embedded_assets, stage_public_assets};
pub use platform::Platform;
pub use secret::{SECRET_FILE, install_session_secret, session_secret};
pub use server::{EmbeddedServer, RunningServer};

/// Turn off every development affordance, for a package that is being shipped.
///
/// Release builds already default to dev mode off, so this is belt and braces
/// — but the belt matters here in a way it does not on a server. `RAHTI_DEV`
/// is read from the process environment, an installed application inherits the
/// environment of whoever launched it, and a user with `RAHTI_DEV=1` exported
/// for their own project would otherwise start a shipped application with its
/// diagnostics endpoint, its reload stream and its `.rahti/dev.log` writer
/// live — inside a WebView with access to native commands.
///
/// So a release package states the value rather than inheriting it. A debug
/// build is left alone: that is `cargo rahti native dev`, where the
/// diagnostics are the point.
pub fn harden_release() {
    if cfg!(debug_assertions) {
        return;
    }
    // SAFETY: called from the native host before any task is spawned and
    // before the router is built, which is the same single-threaded moment
    // `main` sets anything else.
    unsafe {
        std::env::set_var(DEV_ENV, "0");
    }
}

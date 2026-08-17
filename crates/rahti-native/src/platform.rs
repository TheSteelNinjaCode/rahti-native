//! Which operating system the package is running on.
//!
//! Two named platforms and an "other", rather than a `cfg!` at every call
//! site: the path rules, the secret store and the back-button behaviour all
//! branch on this, and a value can be constructed in a test on a machine that
//! is neither.

use std::fmt;

/// A platform a Rahti application can be packaged for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    Windows,
    Android,
    /// Anything else the host compiles for. Packaging is not supported, but
    /// the embedded server and the paths still resolve, which is what lets
    /// this crate's tests run on a Linux CI machine.
    Other,
}

impl Platform {
    /// The platform this binary was compiled for.
    pub const fn current() -> Self {
        if cfg!(target_os = "windows") {
            Platform::Windows
        } else if cfg!(target_os = "android") {
            Platform::Android
        } else {
            Platform::Other
        }
    }

    /// The name `pp.native.platform` reports to the browser.
    pub const fn name(self) -> &'static str {
        match self {
            Platform::Windows => "windows",
            Platform::Android => "android",
            Platform::Other => "other",
        }
    }

    /// The name `cargo rahti native --target` accepts, for the two that have
    /// one.
    pub fn parse_target(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "windows" | "win" => Some(Platform::Windows),
            "android" => Some(Platform::Android),
            _ => None,
        }
    }

    /// Whether the application is on a battery-powered platform whose OS may
    /// stop and recreate the process at will.
    ///
    /// Android does; Windows does not. The difference decides whether a
    /// long-lived task may assume it will be allowed to finish.
    pub const fn is_mobile(self) -> bool {
        matches!(self, Platform::Android)
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

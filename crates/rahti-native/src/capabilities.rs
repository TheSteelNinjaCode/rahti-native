//! What the WebView is allowed to ask the operating system for.
//!
//! ## Why this list is short
//!
//! In a browser, an XSS is a stolen session. In a native shell, an XSS is a
//! stolen session *and* whatever the page can reach through the native bridge
//! — so the size of that bridge is the size of the damage. A shell exposing
//! `execute_process` or an unrestricted filesystem API has turned every markup
//! bug in the application into remote code execution on the user's machine.
//!
//! So the bridge is an allowlist of narrow, named operations, and it starts as
//! small as it can while still being useful. `choose_file` opens the platform
//! picker and returns what the *user* selected; there is no command that reads
//! a path the page names. That difference is the entire security model: the
//! human, not the JavaScript, decides which file the application sees.
//!
//! ## Adding one
//!
//! A command has to be in [`commands`] before the generated shell will route
//! it, and the shell's Tauri capability file has to list it too. Both, on
//! purpose: the Rust allowlist is what this crate can test, and the capability
//! file is what Tauri enforces. Neither alone.
//!
//! Every command must be safe to hand to a page that has been compromised.
//! "The application only calls it with good values" is not an argument — the
//! attacker calls it with the others.

/// What a command lets the page do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// Read-only facts about the host. No side effects.
    Inspect,
    /// Hands something to another program, or to the user.
    Handoff,
}

/// One entry in the bridge allowlist.
#[derive(Debug, Clone, Copy)]
pub struct NativeCommand {
    /// What `pp.native.invoke` names.
    pub name: &'static str,
    /// The capability string `pp.native.has(...)` answers for.
    pub capability: &'static str,
    pub kind: Capability,
    /// One line, used by the generated shell's documentation.
    pub summary: &'static str,
}

/// The whole allowlist.
///
/// Deliberately five. Each is either a fact about the host or an action the
/// user completes themselves.
pub fn commands() -> &'static [NativeCommand] {
    &[
        NativeCommand {
            name: "platform",
            capability: "platform",
            kind: Capability::Inspect,
            summary: "Which operating system the package is running on.",
        },
        NativeCommand {
            name: "app_version",
            capability: "app-version",
            kind: Capability::Inspect,
            summary: "The installed application's version.",
        },
        NativeCommand {
            name: "app_data_dir",
            capability: "app-data-dir",
            kind: Capability::Inspect,
            summary: "Where this installation keeps its data, for showing the user.",
        },
        NativeCommand {
            name: "open_external",
            capability: "open-external",
            kind: Capability::Handoff,
            summary: "Open an http, https or mailto URL in the user's own browser or mail client.",
        },
        NativeCommand {
            name: "choose_file",
            capability: "choose-file",
            kind: Capability::Handoff,
            summary: "Open the platform file picker and return what the user chose.",
        },
    ]
}

/// Whether the shell should route `name` at all.
///
/// The check the generated shell makes before dispatching, so a command that
/// was removed from the allowlist stops working even if a route for it was
/// left behind.
pub fn is_allowed(name: &str) -> bool {
    commands().iter().any(|command| command.name == name)
}

/// The capability strings, for the bridge script's `pp.native.has(...)`.
pub fn capability_names() -> Vec<&'static str> {
    commands().iter().map(|c| c.capability).collect()
}

/// Whether a URL may be handed to the operating system.
///
/// The check that keeps `open_external` from being a way to run things. A
/// `file:` URL opens whatever the shell associates with the extension, and on
/// Windows that includes executables and script hosts; `javascript:` and
/// `data:` are ways back into a privileged context. Only the three schemes
/// that mean "somewhere else, in another program" are allowed.
///
/// Scheme comparison is ASCII-case-insensitive because URL schemes are, and
/// `JavaScript:` is the form that gets past a check that forgot.
pub fn is_external_url(url: &str) -> bool {
    let url = url.trim();

    // A control character can split a URL across a line in whatever parses it
    // next, which is how a check on the first line stops describing the whole
    // value.
    if url.is_empty() || url.chars().any(|c| c.is_control()) {
        return false;
    }

    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }

    match scheme.to_ascii_lowercase().as_str() {
        // A host is required, so `http:///etc` and `https:/x` are not URLs
        // that go anywhere.
        "http" | "https" => rest.starts_with("//") && rest.len() > 2 && !rest.starts_with("///"),
        "mailto" => true,
        _ => false,
    }
}

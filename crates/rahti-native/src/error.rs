//! What a native host is told when startup cannot continue.
//!
//! Every failure in this crate is a value, never an exit. A packaged
//! application has no terminal: a process that calls `std::process::exit`
//! during startup is, from the user's side, a program that did not open. The
//! host is expected to show [`NativeError`] in a dialog and stop, which is the
//! difference between "the database could not be created" and nothing at all.

use std::fmt;
use std::path::{Path, PathBuf};

/// A native startup or packaging failure.
///
/// `step` names the part of startup that failed so a host can say *where*
/// without parsing the message, and `Display` is the sentence to show.
#[derive(Debug)]
pub struct NativeError {
    /// Which part of startup failed: `"paths"`, `"listener"`, `"secret"`,
    /// `"assets"`, `"config"`, `"server"`.
    pub step: &'static str,
    /// The failure, phrased for a person.
    pub message: String,
    /// The path involved, when there was one.
    pub path: Option<PathBuf>,
}

impl NativeError {
    pub fn new(step: &'static str, message: impl Into<String>) -> Self {
        NativeError {
            step,
            message: message.into(),
            path: None,
        }
    }

    pub fn at(step: &'static str, path: impl AsRef<Path>, message: impl Into<String>) -> Self {
        NativeError {
            step,
            message: message.into(),
            path: Some(path.as_ref().to_path_buf()),
        }
    }

    /// An `io::Error` with the path it happened to, which the standard error
    /// does not carry and which is the only part a user can act on.
    pub fn io(step: &'static str, path: impl AsRef<Path>, error: std::io::Error) -> Self {
        NativeError::at(step, path, error.to_string())
    }
}

impl fmt::Display for NativeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(path) => write!(f, "{} ({})", self.message, path.display()),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for NativeError {}

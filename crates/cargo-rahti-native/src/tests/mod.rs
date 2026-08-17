//! `cargo-rahti-native`'s tests.
//!
//! Split by the thing under test. `args` and `init` are here; `project`,
//! `icons` and `schema` are included from the modules they test, so they can
//! see those modules' private items — the arrangement the rest of the
//! workspace uses.

mod android;
mod args;
mod init;

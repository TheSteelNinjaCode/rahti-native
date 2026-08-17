//! `rahti-native`'s own tests.
//!
//! Split by the thing under test rather than by file, and included from
//! `lib.rs` so they see the crate's private items — the same arrangement
//! `rahti` uses.

mod bridge;
mod capabilities;
mod config;
mod gate;
mod headers;
mod paths;
mod secret;
mod server;

/// Serializes the tests that set process environment variables.
///
/// `std::env::set_var` is process-wide and a test binary runs its tests in
/// parallel threads, so two tests putting a different `AUTH_SECRET` in the
/// environment would read each other's. Everything that touches the real
/// environment takes this first.
pub static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A temporary directory that deletes itself.
///
/// Small enough to write, and smaller than the dependency: these tests need a
/// directory each and nothing else a temporary-file crate offers.
pub struct TempDir(std::path::PathBuf);

impl TempDir {
    pub fn new(label: &str) -> Self {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).expect("randomness for a test directory");
        let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();

        let path = std::env::temp_dir().join(format!("rahti-native-{label}-{name}"));
        std::fs::create_dir_all(&path).expect("a test directory");
        TempDir(path)
    }

    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

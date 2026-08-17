//! The per-installation session key.
//!
//! `rahti::auth` signs its session cookie with `AUTH_SECRET`, read from the
//! process environment and, in a web project, put there by an ignored `.env`.
//! A packaged application has no `.env` — and must not have one, because a
//! file shipped inside an installer is a file every copy of the application
//! shares, which for a signing key means every installation can forge every
//! other installation's sessions.
//!
//! It also cannot go without. `rahti::auth` invents a key when none is set,
//! once per process — so a packaged application with no secret signs you in,
//! and signs you out again the next time it starts.
//!
//! So the key is generated **on the device, at first launch**, and kept in
//! application storage. It is per installation: nothing in the repository,
//! nothing in the installer, and nothing shared between two users of the same
//! machine.
//!
//! ## Where it is kept
//!
//! **Windows.** Encrypted with DPAPI (`CryptProtectData`) before it is
//! written. DPAPI derives its key from the signed-in user account, so the file
//! is readable by that user on that machine and by nobody else: copied to
//! another machine, or read by another account, it decrypts to nothing. The
//! file itself is in `%LOCALAPPDATA%`, which is already per-user.
//!
//! **Android.** In the application's internal files directory, which is the
//! platform's own per-application sandbox — a directory owned by a UID that
//! only this application runs as, unreadable by every other installed app on a
//! non-rooted device. That is the protection Android provides to files; a
//! Keystore-backed wrapper on top of it needs Kotlin and a plugin, and this
//! crate does not claim to have one. See `native-packaging.md`.
//!
//! ## What is not done with it
//!
//! It is never printed, never written to a log, never returned to the WebView,
//! and never a native command's return value. [`session_secret`] is called
//! once at startup and its result goes straight into the environment.

use std::path::Path;

use crate::error::NativeError;
use crate::paths::AppPaths;

/// The file, under [`AppPaths::data`].
pub const SECRET_FILE: &str = "session.key";

/// How many random bytes the key is. 32 bytes, written as 64 hex characters —
/// comfortably past the length `rahti::auth` refuses below.
const SECRET_BYTES: usize = 32;

/// This installation's session key, generating one on first launch.
///
/// Idempotent across launches by construction: a key that already exists is
/// read, and only a missing or unreadable one is replaced. That is the whole
/// point — a key that changed per launch would sign every user out at every
/// restart, which looks exactly like a broken login.
pub fn session_secret(paths: &AppPaths) -> Result<String, NativeError> {
    let file = paths.secret_file();

    if let Some(existing) = read_secret(&file)? {
        return Ok(existing);
    }

    let secret = generate()?;
    write_secret(&file, &secret)?;
    Ok(secret)
}

/// Put the session key, and the project's cookie name, where `rahti::auth`
/// will read them.
///
/// Called before `initialize_application`, because the auth policy is built
/// from the environment as the router goes up.
///
/// `cookie_name` is `auth.cookieName` from `rahti.native.json`. Without it the
/// framework default applies, which works — a native application's cookie jar
/// belongs to that application — but means a package and its web deployment
/// disagree about the name for no reason.
pub fn install_session_secret(
    paths: &AppPaths,
    cookie_name: Option<&str>,
) -> Result<(), NativeError> {
    let secret = session_secret(paths)?;

    // SAFETY: called by the native host before any task is spawned and before
    // the router is built — the same single-threaded moment `main` sets
    // anything else.
    unsafe {
        std::env::set_var(rahti::auth::SECRET_ENV, secret);
        if let Some(name) = cookie_name {
            std::env::set_var(rahti::auth::COOKIE_ENV, name);
        }
    }
    Ok(())
}

/// A new key, from the operating system's randomness.
fn generate() -> Result<String, NativeError> {
    let mut bytes = [0u8; SECRET_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| {
        NativeError::new(
            "secret",
            format!("the operating system would not provide randomness for a session key: {e}"),
        )
    })?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The stored key, or `None` when there is not a usable one.
///
/// A file that exists and cannot be decrypted counts as absent rather than as
/// an error: the realistic cause is a Windows profile that was restored or a
/// user who was recreated, and the correct response to "this key is not
/// readable by me" is a new key and a signed-out user, not an application that
/// refuses to open.
fn read_secret(file: &Path) -> Result<Option<String>, NativeError> {
    let stored = match std::fs::read(file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(NativeError::io("secret", file, e)),
    };

    let Some(plain) = unprotect(&stored) else {
        return Ok(None);
    };

    let secret = String::from_utf8(plain).ok().filter(|s| s.len() >= 32);
    Ok(secret)
}

fn write_secret(file: &Path, secret: &str) -> Result<(), NativeError> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| NativeError::io("secret", parent, e))?;
    }

    let protected = protect(secret.as_bytes())?;
    std::fs::write(file, protected).map_err(|e| NativeError::io("secret", file, e))?;
    restrict(file)?;
    Ok(())
}

/// Owner-only, where the filesystem has a word for it.
///
/// Android's internal files directory is already per-application, so this is
/// belt and braces there. It is not on the "other" platforms this crate
/// compiles for so that a developer running the tests on Linux does not leave
/// a world-readable key behind.
#[cfg(unix)]
fn restrict(file: &Path) -> Result<(), NativeError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| NativeError::io("secret", file, e))
}

/// Windows has no mode bits. `%LOCALAPPDATA%` is per-user, and DPAPI is what
/// actually protects the contents.
#[cfg(not(unix))]
fn restrict(_file: &Path) -> Result<(), NativeError> {
    Ok(())
}

// ------------------------------------------------------------------ DPAPI

#[cfg(windows)]
mod dpapi {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData,
    };

    /// Encrypt for the signed-in user.
    ///
    /// `None` for the entropy argument deliberately: a second secret to
    /// protect the first one would have to be stored beside it, which protects
    /// nothing. The user account *is* the key.
    pub fn protect(plain: &[u8]) -> Option<Vec<u8>> {
        let mut input = blob(plain);
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };

        // SAFETY: `input` points at `plain` for the duration of the call, and
        // every optional parameter is null, which the API documents as
        // "absent". `output` is written by the call and freed below.
        let ok = unsafe {
            CryptProtectData(
                &mut input,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                &mut output,
            )
        };
        take(ok, output)
    }

    pub fn unprotect(sealed: &[u8]) -> Option<Vec<u8>> {
        let mut input = blob(sealed);
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };

        // SAFETY: as above.
        let ok = unsafe {
            CryptUnprotectData(
                &mut input,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                &mut output,
            )
        };
        take(ok, output)
    }

    fn blob(bytes: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: bytes.len() as u32,
            pbData: bytes.as_ptr() as *mut u8,
        }
    }

    /// Copy what the API allocated, and give it back.
    /// `ok` is a Win32 `BOOL`: zero is failure, anything else is success.
    fn take(ok: i32, output: CRYPT_INTEGER_BLOB) -> Option<Vec<u8>> {
        if ok == 0 || output.pbData.is_null() {
            return None;
        }
        // SAFETY: a successful call leaves `cbData` bytes at `pbData`, which
        // is a `LocalAlloc` allocation the caller owns.
        let copied =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
        unsafe {
            LocalFree(output.pbData as _);
        }
        Some(copied)
    }
}

#[cfg(windows)]
fn protect(plain: &[u8]) -> Result<Vec<u8>, NativeError> {
    dpapi::protect(plain).ok_or_else(|| {
        NativeError::new(
            "secret",
            "Windows would not encrypt the session key for this user account (DPAPI).",
        )
    })
}

#[cfg(windows)]
fn unprotect(sealed: &[u8]) -> Option<Vec<u8>> {
    dpapi::unprotect(sealed)
}

/// Everywhere else the key is stored as it is, protected by the filesystem.
///
/// On Android that is the application sandbox, which is the platform's actual
/// answer for application-private files. On a developer machine running these
/// tests it is mode `0600`. Neither is DPAPI, and neither pretends to be.
#[cfg(not(windows))]
fn protect(plain: &[u8]) -> Result<Vec<u8>, NativeError> {
    Ok(plain.to_vec())
}

#[cfg(not(windows))]
fn unprotect(sealed: &[u8]) -> Option<Vec<u8>> {
    Some(sealed.to_vec())
}

//! The per-installation session key: that it is generated once, survives a
//! restart, and never appears in plain text on Windows.

use super::TempDir;
use crate::paths::AppPaths;
use crate::secret::*;

fn packaged(root: &std::path::Path) -> AppPaths {
    AppPaths::from_host(
        "com.example.myapp",
        root.join("data"),
        root.join("cache"),
        root.join("resources"),
    )
    .expect("packaged paths")
}

#[test]
fn a_first_launch_generates_a_key_a_release_build_would_accept() {
    let temp = TempDir::new("secret");
    let paths = packaged(temp.path());
    paths.prepare().expect("a first launch");

    let secret = session_secret(&paths).expect("a key");

    // `rahti::auth` refuses a release build below its minimum, and invents one
    // per process when there is none — which signs everybody out at every
    // restart.
    assert_eq!(secret.len(), 64, "32 random bytes, as hex");
    assert!(secret.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(!secret.chars().all(|c| c == '0'));
}

#[test]
fn the_key_survives_a_restart() {
    // The whole point. A key that changed per launch would sign every user out
    // at every restart, which looks exactly like a broken login.
    let temp = TempDir::new("secret");
    let paths = packaged(temp.path());
    paths.prepare().expect("a first launch");

    let first = session_secret(&paths).expect("a key");
    let second = session_secret(&paths).expect("the same key");
    assert_eq!(first, second);
}

#[test]
fn two_installations_do_not_share_a_key() {
    // A key shipped in an installer is a key every installation can forge
    // every other installation's sessions with.
    let one = TempDir::new("secret");
    let two = TempDir::new("secret");
    let (a, b) = (packaged(one.path()), packaged(two.path()));
    a.prepare().unwrap();
    b.prepare().unwrap();

    assert_ne!(
        session_secret(&a).unwrap(),
        session_secret(&b).unwrap(),
        "two installations were given the same signing key"
    );
}

#[test]
fn the_key_is_kept_in_data_and_not_beside_the_assets() {
    let temp = TempDir::new("secret");
    let paths = packaged(temp.path());
    paths.prepare().unwrap();
    session_secret(&paths).unwrap();

    let file = paths.secret_file();
    assert!(file.exists());
    assert!(file.starts_with(paths.data()));
    assert!(!file.starts_with(paths.public()));
    assert!(!file.starts_with(paths.resources()));
    assert_eq!(file.file_name().unwrap(), SECRET_FILE);
}

#[cfg(windows)]
#[test]
fn the_stored_key_is_not_the_key_in_plain_text() {
    // DPAPI: the file is readable by this user on this machine and by nobody
    // else. Copied elsewhere, it decrypts to nothing.
    let temp = TempDir::new("secret");
    let paths = packaged(temp.path());
    paths.prepare().unwrap();

    let secret = session_secret(&paths).unwrap();
    let stored = std::fs::read(paths.secret_file()).unwrap();

    assert!(
        !stored.windows(secret.len()).any(|w| w == secret.as_bytes()),
        "the session key is on disk in plain text"
    );
    assert!(stored.len() > secret.len(), "nothing was encrypted");
}

#[test]
fn an_unreadable_key_file_is_replaced_rather_than_fatal() {
    // A restored Windows profile or a recreated user account makes the stored
    // blob undecryptable. The right answer is a new key and a signed-out user,
    // not an application that refuses to open.
    let temp = TempDir::new("secret");
    let paths = packaged(temp.path());
    paths.prepare().unwrap();

    std::fs::write(paths.secret_file(), b"not a key this user can read").unwrap();

    let secret = session_secret(&paths).expect("a replacement key");
    assert_eq!(secret.len(), 64);
    assert_eq!(session_secret(&paths).unwrap(), secret, "and it stuck");
}

#[test]
fn a_truncated_key_is_replaced() {
    let temp = TempDir::new("secret");
    let paths = packaged(temp.path());
    paths.prepare().unwrap();
    let first = session_secret(&paths).unwrap();

    // Short enough that `rahti::auth` would refuse it in a release build.
    std::fs::write(paths.secret_file(), b"abc").unwrap();
    let replacement = session_secret(&paths).expect("a replacement key");

    assert_eq!(replacement.len(), 64);
    assert_ne!(replacement, first);
}

#[test]
fn installing_the_key_puts_it_and_the_cookie_name_in_the_environment() {
    // Serialized against the other environment-reading tests in this binary:
    // `std::env::set_var` is process-wide, and two tests setting `AUTH_SECRET`
    // at once would read each other's value.
    let _guard = crate::tests::ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let temp = TempDir::new("secret");
    let paths = packaged(temp.path());
    paths.prepare().unwrap();

    install_session_secret(&paths, Some("rahti_session_9f2c")).expect("the environment");

    assert_eq!(
        std::env::var(rahti::auth::SECRET_ENV).unwrap(),
        session_secret(&paths).unwrap()
    );
    assert_eq!(
        std::env::var(rahti::auth::COOKIE_ENV).unwrap(),
        "rahti_session_9f2c"
    );

    // SAFETY: the lock above is the whole of this binary's access to these two
    // variables, and it is still held.
    unsafe {
        std::env::remove_var(rahti::auth::SECRET_ENV);
        std::env::remove_var(rahti::auth::COOKIE_ENV);
    }
}

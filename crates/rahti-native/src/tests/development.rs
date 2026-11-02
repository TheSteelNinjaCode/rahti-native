//! Development settings: what reaches a debug build, and where its database
//! connection ends up pointing.

use super::ENV_LOCK;
use crate::Platform;
use crate::development::*;

const LOCAL: &str = "postgres://shop:p%40ss@localhost:5432/shop";

fn on_android(url: &str, loopback_open: bool) -> String {
    reachable_database_url(url, Platform::Android, |_| loopback_open)
}

#[test]
fn a_dotenv_line_reads_the_way_the_framework_reads_it() {
    let pairs = parse(
        "# a comment\n\
         \n\
         PLAIN=value\n\
         QUOTED=\"postgres://localhost/shop\"\r\n\
         SINGLE='a b'\n\
         export EXPORTED=yes\n\
         not a setting\n",
    );

    let pairs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("PLAIN", "value"),
            ("QUOTED", "postgres://localhost/shop"),
            ("SINGLE", "a b"),
            ("EXPORTED", "yes"),
        ]
    );
}

#[test]
fn an_android_emulator_reaches_its_host_through_its_alias() {
    // `localhost` is the device, and the failure without this is an empty
    // catalog rather than an error.
    assert_eq!(
        on_android(LOCAL, false),
        "postgres://shop:p%40ss@10.0.2.2:5432/shop"
    );
}

#[test]
fn a_forwarded_port_keeps_the_address_it_was_given() {
    // `adb reverse` is the only route a phone has, so the probe wins.
    assert_eq!(on_android(LOCAL, true), LOCAL);
}

#[test]
fn the_probe_asks_about_the_port_the_url_names() {
    let probed = std::cell::Cell::new(0);
    reachable_database_url(
        "mysql://root@127.0.0.1:3307/shop",
        Platform::Android,
        |port| {
            probed.set(port);
            true
        },
    );
    assert_eq!(probed.get(), 3307);
}

#[test]
fn a_url_without_a_port_gets_its_backends_default() {
    assert_eq!(
        on_android("postgresql://localhost/shop?sslmode=disable", false),
        "postgresql://10.0.2.2:5432/shop?sslmode=disable"
    );
    assert_eq!(
        on_android("mysql://root:pw@localhost/shop", false),
        "mysql://root:pw@10.0.2.2:3306/shop"
    );
}

#[test]
fn only_the_host_of_a_url_is_rewritten() {
    // A password or a database called `localhost` is not an address.
    assert_eq!(
        on_android("postgres://localhost:pw@db.example.com/localhost", false),
        "postgres://localhost:pw@db.example.com/localhost"
    );
    assert_eq!(
        on_android("postgres://me:a@localhost@localhost:5432/localhost", false),
        "postgres://me:a@localhost@10.0.2.2:5432/localhost"
    );
}

#[test]
fn a_database_that_is_really_remote_is_left_alone() {
    let url = "postgres://shop:secret@db.example.com:5432/shop";
    assert_eq!(on_android(url, false), url);
    assert_eq!(loopback_database_port(url), None);
}

#[test]
fn a_desktop_shares_the_loopback_with_its_database() {
    assert_eq!(
        reachable_database_url(LOCAL, Platform::Windows, |_| panic!("no probe")),
        LOCAL
    );
}

#[test]
fn something_that_is_not_a_url_is_left_alone() {
    assert_eq!(on_android("not a url", false), "not a url");
    assert_eq!(
        on_android("sqlite://local.db?mode=rwc", false),
        "sqlite://local.db?mode=rwc"
    );
    assert_eq!(
        on_android("postgres://localhost:port/x", false),
        "postgres://localhost:port/x"
    );
}

#[test]
fn the_loopback_port_is_what_cargo_rahti_native_forwards() {
    assert_eq!(loopback_database_port(LOCAL), Some(5432));
    assert_eq!(
        loopback_database_port("postgres://[::1]:6000/shop"),
        Some(6000)
    );
    assert_eq!(loopback_database_port("mysql://127.0.0.1/shop"), Some(3306));
}

#[test]
fn the_environment_wins_over_the_embedded_value() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (kept, added) = ("RAHTI_NATIVE_TEST_KEPT", "RAHTI_NATIVE_TEST_ADDED");
    // SAFETY: ENV_LOCK serializes every test that touches the environment.
    unsafe {
        std::env::set_var(kept, "the real one");
        std::env::remove_var(added);
    }

    apply_development_env(&format!("{kept}=embedded\n{added}=embedded\n"));

    assert_eq!(std::env::var(kept).as_deref(), Ok("the real one"));
    assert_eq!(std::env::var(added).as_deref(), Ok("embedded"));
    // SAFETY: as above.
    unsafe {
        std::env::remove_var(kept);
        std::env::remove_var(added);
    }
}

#[test]
fn a_release_builds_empty_file_sets_nothing() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let before = std::env::vars().count();
    apply_development_env("");
    assert_eq!(std::env::vars().count(), before);
}

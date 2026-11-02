//! The `.env` values a development build carries onto the device.
//!
//! A packaged application reads no `.env`: Android has no project directory to
//! read one from, and a Windows `dev` run starts in `native/`. So a project on
//! PostgreSQL or MySQL starts with no `DATABASE_URL`, connects to nothing, and
//! shows whatever its pages show without data.
//!
//! The generated `build.rs` selects the names `rahti.native.json` lists and
//! leaves them in the build's `OUT_DIR`, for the debug profile only. Those
//! values are credentials, and anything in a package can be read back out of
//! it, so a release build has an empty file and a shell that never looks at
//! it. See `docs/native-packaging.md`.

use crate::Platform;

/// How an Android emulator addresses the machine it runs on.
const EMULATOR_HOST: &str = "10.0.2.2";

/// Put the values `build.rs` selected in this process's environment.
///
/// Called before `initialize_application`, which is what reads them. A
/// variable the environment already carries wins, as it does in
/// `rahti::load_env`.
pub fn apply_development_env(contents: &str) {
    for (name, value) in parse(contents) {
        if std::env::var_os(&name).is_some() {
            continue;
        }
        let value = if name == "DATABASE_URL" {
            reachable_database_url(&value, Platform::current(), loopback_listening)
        } else {
            value
        };
        // SAFETY: called by the native host before any task is spawned and
        // before the router is built — the same single-threaded moment `main`
        // sets anything else.
        unsafe {
            std::env::set_var(name, value);
        }
    }
}

/// The pairs in a `.env`, read the way `rahti::load_env` reads them.
pub(crate) fn parse(contents: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        pairs.push((name.trim().to_string(), value.to_string()));
    }

    pairs
}

/// A connection string, taken apart around its host.
struct Address<'a> {
    /// Everything up to the host: `postgres://user:password@`.
    before: &'a str,
    host: &'a str,
    port: u16,
    /// Everything after the port: `/shop?sslmode=disable`.
    after: &'a str,
}

fn address(url: &str) -> Option<Address<'_>> {
    let scheme_end = url.find("://")? + 3;
    let rest = &url[scheme_end..];
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    // The last `@`, so an unencoded one in a password stays in the password.
    let host_start = authority.rfind('@').map_or(0, |at| at + 1);
    let location = &authority[host_start..];

    let (host, port) = match location.rsplit_once(':') {
        Some((host, port)) if !port.contains(']') => (host, port.parse().ok()?),
        _ => (location, default_port(&url[..scheme_end])?),
    };

    Some(Address {
        before: &url[..scheme_end + host_start],
        host,
        port,
        after: &rest[authority_end..],
    })
}

fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "postgres://" | "postgresql://" => Some(5432),
        "mysql://" | "mariadb://" => Some(3306),
        _ => None,
    }
}

/// The port of a database on this machine's loopback, if that is where it is.
///
/// What `cargo rahti native dev` forwards to an attached Android device.
pub fn loopback_database_port(url: &str) -> Option<u16> {
    let address = address(url)?;
    matches!(address.host, "localhost" | "127.0.0.1" | "[::1]").then_some(address.port)
}

/// A connection string the device can follow to the development machine.
///
/// On Android `localhost` is the device. `adb reverse` makes it the development
/// machine, on an emulator or a phone, and is used when `loopback_open` finds
/// it. Otherwise an emulator reaches its host at `10.0.2.2`, and the URL is
/// rewritten to that: a phone with no forwarded port has no route to it either
/// way, and the application's own connection error is what names the problem.
pub(crate) fn reachable_database_url(
    url: &str,
    platform: Platform,
    loopback_open: impl Fn(u16) -> bool,
) -> String {
    if platform != Platform::Android {
        return url.to_string();
    }
    let Some(port) = loopback_database_port(url) else {
        return url.to_string();
    };
    if loopback_open(port) {
        return url.to_string();
    }
    let Some(address) = address(url) else {
        return url.to_string();
    };
    format!("{}{EMULATOR_HOST}:{port}{}", address.before, address.after)
}

fn loopback_listening(port: u16) -> bool {
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(250)).is_ok()
}

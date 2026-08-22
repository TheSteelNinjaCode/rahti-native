# rahti-native

Optional Windows and Android packaging for [Rahti](https://crates.io/crates/rahti)
applications.

Your application is the one you already have. Its Rust backend is compiled for
the target platform and runs inside the installed program; the generated Axum
router — the same one, with the same auth guard, the same CSRF layer and the
same static fallback — answers on a loopback socket; and the operating system's
WebView loads it. Pages, components, layouts, error boundaries, `html!`,
PulsePoint, rpcs, streaming rpcs, uploads, sockets and sessions all work,
because none of them are involved.

**It is not an HTML-to-native-widget compiler.** Nothing here translates
markup. What the user sees is a WebView, and the controls in it are the
browser's, not the platform's.

## Why this is a separate package

So that "optional" is a fact rather than a claim.

A Rahti project is a web application and stays one. Nothing in this repository
is compiled, downloaded or resolved by a project that has not asked for it —
not Tauri, not an Android SDK, not this crate. You install the tool when you
want a package, and a project that never does is unaffected.

## Install

```bash
cargo install cargo-rahti-native
```

Packaging itself is delegated to Tauri's own CLI, which is a separate install:

```bash
cargo install tauri-cli --version "^2" --locked
```

Two tools rather than one because the Tauri CLI has to match the `tauri`
version in the shell's `Cargo.toml`, which is yours to upgrade on your own
schedule.

## Use

```bash
cargo rahti native init --identifier com.example.myapp --windows --android
cargo rahti native doctor
cargo rahti native dev   --target windows
cargo rahti native build --target windows
cargo rahti native build --target android --format apk
cargo rahti native build --target android --format aab
```

`cargo rahti native …` is forwarded by `cargo-rahti`; `cargo rahti-native …`
reaches this tool directly through cargo's own subcommand dispatch. `build`
prints the absolute path of every artifact it produced.

`doctor` checks only what the named target needs, installs nothing, and each
failure says what is missing, why it is needed, and the command that supplies
it.

## The two crates

| Crate | Role |
| --- | --- |
| `rahti-native` | The platform-neutral runtime: packaged paths, the loopback-only embedded server, the per-installation session key, the launch gate, the security headers, and the `pp.native` bridge. **No Tauri dependency**, so all of it tests in an ordinary `cargo test`. |
| `cargo-rahti-native` | The CLI: `init`, `doctor`, `dev`, `build`. Generates the application-owned Tauri shell under `native/`. No Tauri dependency either — it delegates. |

## What your application has to expose

One function in its library:

```rust
// src/lib.rs
pub async fn initialize_application() -> Result<ApplicationRuntime, StartupError>
```

`src/main.rs` calls it and binds the public listener; the native shell calls it
and binds a loopback one. Projects scaffolded by a current `cargo rahti new`
already have it, and `doctor` says so with the command that fixes a project
that does not.

## Documentation

[`docs/native-packaging.md`](docs/native-packaging.md) — the full reference:
packaged paths, the database policy, the session key, the loopback security
model, the native command allowlist, signing, and shutdown. It is the same
document `cargo rahti new` ships into a project's `docs/conventions/`.

## Status

Early — `0.0.2`. It has its own release line rather
than tracking the framework's: the two are separate installs and you upgrade
them separately. See [`PUBLISHING.md`](PUBLISHING.md).

## License

MIT OR Apache-2.0.

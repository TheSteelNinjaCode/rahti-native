# Publishing

Two crates, released together, in this order:

```text
cargo publish -p rahti-native
cargo publish -p cargo-rahti-native
```

The order is forced by the dependency: the CLI depends on the runtime.

## Before the first release

**`rahti` 0.0.18 has to be on crates.io first.** This package requires it
exactly, because it is the first release carrying the three things this package
calls and nothing in the 0.0.17 line has them:

| Needs | Why |
| --- | --- |
| `rahti::begin_shutdown` | A packaged GUI never receives Ctrl+C and Android's lifecycle has nothing like it, so the host says when to stop. |
| the `dev-watch` cargo feature | Turned off, so `notify` leaves the Android build rather than being compiled and never called. |
| `initialize_application` in the scaffold | The shell calls it; Android has no `main` to reach. |

All three are in the `../rahti` publishing checkout, staged for its `0.0.18`
release. Until that release is live, this package builds only beside that
checkout, through the `path` in `[workspace.dependencies]`.

So the release order across the two repositories is:

1. `rahti` 0.0.18 — `rahti-macros`, `rahti-build`, `rahti`, `cargo-rahti`.
2. `rahti-native` 0.0.1 — `rahti-native`, then `cargo-rahti-native`.

## The `rahti` dependency

```toml
rahti = { version = "0.0.18", path = "../rahti/crates/rahti", default-features = false }
```

`version` is what makes it publishable — cargo refuses a path-only dependency
on publish — and `path` is what makes a side-by-side checkout build before that
version exists. Cargo uses the path locally and the version once published.

The path names `../rahti`, the publishing checkout — not `../rahti-dev`, where
framework work happens. What this builds against should be what actually ships.

A clone that does not sit beside it should delete the `path` and let cargo
resolve `0.0.18` from crates.io.

## Version

This package has its **own** release line, independent of the framework's. It
is versioned by what it changes, not by what `rahti` does: the two are separate
installs and a user upgrades them separately. It starts at `0.0.1`.

A bump is two strings in the root `Cargo.toml` — `[workspace.package] version`
and the `rahti-native` pin in `[workspace.dependencies]` beside it. Both crates
inherit it, and the generated shell records the tool's own
`CARGO_PKG_VERSION`, so there is nothing else to touch.

Do not confuse it with the `rahti` requirement in the same file. That names the
*framework* version this package builds against, and the two move on their own
schedules.

## Before publishing

```bash
cargo fmt --all -- --check
cargo clippy --workspace
cargo test --workspace
cargo publish -p rahti-native --dry-run
```

`cargo publish` refuses a dirty tree.

## What is not published from here

The generated Tauri shell. It is written into an application's own `native/`
directory by `cargo rahti native init` and belongs to that application — its
identifier, its icons, its permissions and its window are application
decisions, in application files.

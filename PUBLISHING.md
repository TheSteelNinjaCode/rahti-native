# Publishing

Two crates, released together, in this order:

```text
cargo publish -p rahti-native
cargo publish -p cargo-rahti-native
```

The order is forced by the dependency: the CLI depends on the runtime.

## Framework prerequisite

**`rahti` 0.0.37 has to be on crates.io first.** The runtime's integration
tests use that release as a dev-dependency, and the generated shell relies on
the scaffold contract it supplies:

| Needs | Why |
| --- | --- |
| the application's `rahti::begin_shutdown` | A packaged GUI never receives Ctrl+C, so the generated host signals the exact framework instance the application uses. |
| the `dev-watch` cargo feature | Disabled in the integration tests that exercise packaged behavior. |
| `initialize_application` in the scaffold | The shell calls it; Android has no `main` to reach. |

All three are in the `../rahti` publishing checkout at `0.0.37`. Until that
release is live, the tests build beside that checkout through the `path` in
`[workspace.dependencies]`.

So the release order across the two repositories is:

1. `rahti` 0.0.37 — `rahti-macros`, `rahti-build`, `rahti`, `cargo-rahti`.
2. `rahti-native` 0.0.8 — `rahti-native`, then `cargo-rahti-native`.

## The `rahti` dependency

```toml
rahti = { version = "0.0.37", path = "../rahti/crates/rahti", default-features = false }
```

This is a dev-dependency of `rahti-native`, used by its integration tests. It
must not become a normal dependency: an application already links Rahti, and a
second version would carry an independent shutdown broadcast. The generated
shell calls `begin_shutdown` through the application's framework re-export.

`version` makes the published dev-dependency resolvable, and `path` makes the
side-by-side checkout build before that version exists. Cargo uses the path
locally and the version once published.

The path names `../rahti`, the publishing checkout — not `../rahti-dev`, where
framework work happens. What this builds against should be what actually ships.

A clone that does not sit beside it should delete the `path` and let cargo
resolve `0.0.37` from crates.io.

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

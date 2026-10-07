# rahti-native Agent Guide

## Purpose

This repository is Rahti's **optional** native packaging: two crates that put
an existing Rahti web application inside a Windows or Android package. It is
deliberately not part of the framework workspace, and that separation is the
feature — a Rahti project that never installs this compiles, downloads and
resolves none of it.

Read `docs/native-packaging.md` before changing behaviour. It is the canonical
account and it also ships into scaffolded applications, so a rule stated there
is a rule users read.

## The two crates

- `crates/rahti-native/` — the platform-neutral runtime. Packaged paths, the
  loopback embedded server, the per-installation session key, the launch gate,
  the security headers, `rahti.native.json`, the capability allowlist, the
  `pp.native` bridge script.
- `crates/cargo-rahti-native/` — the CLI: `init`, `doctor`, `dev`, `build`, and
  the generated Tauri shell templates.

## Non-Negotiable Conventions

1. **`rahti-native` has no Tauri dependency, and must not gain one.** That is
   what lets every one of its behaviours run in an ordinary `cargo test`. Tauri
   appears only in the shell this repository *generates*, never in what it
   compiles.
2. **`cargo-rahti-native` has no Tauri dependency either.** Packaging is
   delegated to `cargo tauri`, because the CLI has to match the `tauri` version
   in the application's own `native/Cargo.toml`, which the application owns and
   upgrades on its own schedule.
3. Nothing here installs anything. `doctor` names what is missing, why it is
   needed, and the command that supplies it; the person whose machine it is
   runs that command. No SDK download, no global toolchain change.
4. The embedded server binds `127.0.0.1` on an OS-assigned port and there is no
   API to bind anything else. A packaged application on `0.0.0.0` is a
   signed-in session offered to the user's network.
5. Nothing writes into the installation directory. Data, config, cache, the
   database, uploads, logs and the session key resolve through `AppPaths`.
6. The session key is generated on the device at first launch and never
   bundled, never hardcoded, never regenerated per launch, and never written to
   a committed file. DPAPI-encrypted on Windows.
7. The native command allowlist stays short, and nothing on it runs a program
   or reads a path the page names. An XSS in a native shell reaches everything
   the bridge reaches, so a command has to be safe for a page that has been
   compromised. Adding one means the Rust allowlist *and* the generated Tauri
   capability file.
8. `native/` in a user's project is **theirs**. `init` records a hash per
   generated file, regenerates one whose hash still matches, and leaves an
   edited one alone and reports it. `--force` is the only way to take one back,
   and it never keeps what it replaced.
9. Never write a credential to `rahti.native.json`, to `tauri.conf.json`, or to
   any file this tool generates outside the ignored `gen/`. Signing comes from
   `RAHTI_NATIVE_*` environment variables. `AUTH_SECRET` is not read at all.
10. The default Content-Security-Policy must keep `'unsafe-eval'` in
    `script-src`. PulsePoint compiles reactive expressions with `new Function`;
    without it the server-rendered page appears and every binding on it is
    dead, reporting an `EvalError` from inside a minified bundle. Narrowing it
    is not a tightening, it is turning the browser layer off.
11. An application's `public/` is compiled into the shell's binary and written
    to internal storage on first launch. Android's package resources are zip
    entries and Tauri reports their directory as the URI `asset://localhost/`,
    which no file server can open — serving them from `resource_dir()` 404s
    every asset on every device.
12. Android needs two patches applied to the Tauri-generated project on **every**
    build, because `gen/android` is regenerated rather than reviewed: the
    loopback network-security configuration, and the release signing wiring.
    Both are idempotent and both refuse to guess at a project shape they do not
    recognise.

## What lives in the framework, not here

`initialize_application` and its `StartupError`, `rahti::begin_shutdown`, the
`dev-watch` feature, and the `cargo rahti native` delegation in `cargo-rahti`
are all in the `rahti-dev` workspace. The generated shell calls them through
the application's `pub use rahti` re-export; `rahti-native` must not link its
own production copy, because two framework versions have two shutdown globals.
A change needed here that belongs there is made there and released with the
framework.

## Commands

```text
cargo fmt --all -- --check
cargo test --workspace
```

The parts that need a platform — the shell compiling, a WebView loading a page,
a bundler producing an installer or an APK — are not reachable from
`cargo test`. Verify those by generating a shell in a real project and building
it, and report an unrun target check as unrun. A generated shell that compiles
is not a package that launches, and a package that launches is not one whose
installer was produced.

## Building against the framework

`[workspace.dependencies] rahti` carries both a `version` and a `path` for the
runtime's integration tests. It is a dev-dependency, not something linked into
a packaged application. The path is what makes those tests build today; the
version is what makes the published dev-dependency resolvable.

It requires `0.0.47`, matching the current publishing checkout. The path names
`../rahti`, not `../rahti-dev` where framework work happens: what these tests
run against should be what actually ships. That release has to land before
this package can. See `PUBLISHING.md`.

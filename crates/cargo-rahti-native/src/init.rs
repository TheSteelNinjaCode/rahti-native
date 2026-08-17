//! `cargo rahti native init` — add native packaging to a project that has
//! none, and keep it current afterwards.
//!
//! ## Additive, idempotent, non-destructive
//!
//! Three properties, and the third is the one that costs something to get
//! right.
//!
//! - **Additive.** Nothing outside `native/` and the two `rahti.native.*`
//!   files is touched. The application's `src/`, its `Cargo.toml` and its
//!   `.env` are not read for anything but two facts and are never written.
//! - **Idempotent.** Running it twice produces the same tree. The second run
//!   reports "unchanged" rather than rewriting files with identical contents,
//!   because a diff of nothing is what tells somebody the run did nothing.
//! - **Non-destructive.** A generated file that has been *edited* is left
//!   alone. Which files those are is decided by hashes recorded in
//!   `rahti.native.json` — the same mechanism `cargo rahti upgrade` uses — and
//!   not by guessing.
//!
//! `--force` takes an edited file back. It is never implied, and what it
//! replaces is not kept anywhere.
//!
//! ## What a second run is for
//!
//! Changing `rahti.native.json` and re-running is how the shell learns a new
//! window size, a new version, or a new target. The generated `src/lib.rs`
//! carries those values as constants, so the file and the configuration are
//! brought back into agreement by regenerating it — which works precisely
//! because it is one of the files nobody is expected to edit.

use std::collections::BTreeMap;
use std::path::Path;

use rahti_native::{NativeConfig, NativeError, check_identifier};

use crate::args::Init;
use crate::project::Project;
use crate::{icons, schema, shell};

/// What happened to one file.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Outcome {
    Created,
    Updated,
    Unchanged,
    /// Edited since it was generated, and left as it is.
    Kept,
}

pub fn run(project: &Project, args: &Init) -> Result<(), NativeError> {
    let existing = read_existing(project)?;
    let mut config = build_config(project, args, existing.as_ref())?;

    // Before a single file is written. An identifier that Android will refuse
    // is worth catching here rather than several minutes into a Gradle build.
    config.validate()?;

    let mut hashes = config.scaffold.clone();

    let files = shell::files(project, &config);
    let mut outcomes: Vec<(String, Outcome)> = Vec::new();

    for file in &files {
        let outcome = write_file(project, file, &mut hashes, args.force)?;
        outcomes.push((file.path.clone(), outcome));
    }

    outcomes.extend(write_icons(project)?);

    // The schema is written beside the configuration so an editor can offer
    // the fields without a network round trip, and is always overwritten: it
    // describes this tool's format and is not somebody's to edit.
    let schema_path = project.native_schema();
    let schema = schema::json();
    outcomes.push((
        relative(project, &schema_path),
        put(&schema_path, &schema, true)?,
    ));

    // Last, because it carries the hashes of everything above.
    let config_path = project.native_config();
    config.scaffold = hashes;
    let config_outcome = put(&config_path, &config.to_json(), true)?;
    outcomes.push((relative(project, &config_path), config_outcome));

    report(&outcomes, &config, project);
    Ok(())
}

// ------------------------------------------------------------- decisions

/// The configuration the run will write.
///
/// A first run builds one from the flags. A repeat starts from the file on
/// disk, so re-running `init` to regenerate the shell does not silently revert
/// a window size somebody changed.
fn build_config(
    project: &Project,
    args: &Init,
    existing: Option<&NativeConfig>,
) -> Result<NativeConfig, NativeError> {
    let mut config = match existing {
        Some(config) => config.clone(),
        None => {
            let identifier = args.identifier.as_deref().ok_or_else(|| {
                NativeError::new(
                    "config",
                    "this project has no native configuration yet, so `init` needs an \
                     identifier.\n  \
                     It is reverse-DNS, it is the Android package name, and changing it later \
                     makes a different application:\n    \
                     cargo rahti native init --identifier com.example.myapp --windows --android",
                )
            })?;
            check_identifier(identifier).map_err(|m| NativeError::new("config", m))?;

            // A first run with no `--windows`/`--android` means both: the
            // person asked for native packaging, and naming neither platform
            // is not a way of asking for no platforms.
            let targets: Vec<&str> = if args.targets.is_empty() {
                rahti_native::TARGETS.to_vec()
            } else {
                args.targets.clone()
            };

            let name = args
                .product_name
                .clone()
                .unwrap_or_else(|| title_case(&project.package));

            let mut config = NativeConfig::new(
                &name,
                identifier,
                args.version.as_deref().unwrap_or(&project.version),
                &targets,
            );
            // A name, not a key. Kept so the package and the web deployment
            // agree about which cookie the session is in.
            config.auth.cookie_name = project.cookie_name();
            config
        }
    };

    // A flag on a repeat run adds; it never removes. Dropping a target is an
    // edit to the file, where it is visible.
    if let Some(identifier) = &args.identifier {
        check_identifier(identifier).map_err(|m| NativeError::new("config", m))?;
        config.identifier = identifier.clone();
    }
    if let Some(name) = &args.product_name {
        config.product_name = name.clone();
        config.window.title = name.clone();
    }
    if let Some(version) = &args.version {
        config.version = version.clone();
    }
    if let Some(local) = &args.local {
        config.local = Some(local.replace('\\', "/"));
    }

    for target in &args.targets {
        if !config.targets.iter().any(|t| t == target) {
            config.targets.push((*target).to_string());
        }
    }

    Ok(config)
}

/// The configuration on disk, hashes and all.
fn read_existing(project: &Project) -> Result<Option<NativeConfig>, NativeError> {
    let path = project.native_config();
    if !path.is_file() {
        return Ok(None);
    }

    let text = std::fs::read_to_string(&path).map_err(|e| NativeError::io("config", &path, e))?;

    // `NativeConfig::parse` brings a superseded default forward; this is only
    // so the run says it happened. A security-relevant value changing under
    // somebody is worth one line of output.
    if raw_csp(&text).is_some_and(|csp| rahti_native::superseded_csp(&csp)) {
        println!("  updated `security.csp` — the previous default could not run PulsePoint");
    }

    NativeConfig::parse(&text).map(Some).map_err(|mut e| {
        e.path = Some(path);
        e
    })
}

/// `security.csp` exactly as the file spells it, before any migration.
fn raw_csp(text: &str) -> Option<String> {
    let raw: serde_json::Value = serde_json::from_str(text).ok()?;
    raw.get("security")?
        .get("csp")?
        .as_str()
        .map(str::to_string)
}

// ---------------------------------------------------------------- files

fn write_file(
    project: &Project,
    file: &shell::File,
    hashes: &mut BTreeMap<String, String>,
    force: bool,
) -> Result<Outcome, NativeError> {
    let path = project.root.join(&file.path);
    let recorded = hashes.get(&file.path).cloned();

    let outcome = if path.is_file() {
        let current = std::fs::read_to_string(&path)
            .map_err(|e| NativeError::io("init", &path, e))?
            .replace("\r\n", "\n");
        let current_hash = hash(&current);

        let edited = recorded.as_deref() != Some(current_hash.as_str());

        if edited && !force {
            // Left alone, and said so. Somebody's work is not a diff to
            // resolve.
            Outcome::Kept
        } else if current_hash == hash(&file.contents) {
            Outcome::Unchanged
        } else {
            put(&path, &file.contents, true)?;
            Outcome::Updated
        }
    } else {
        put(&path, &file.contents, true)?;
        Outcome::Created
    };

    // A file that was kept keeps its recorded hash, so the next run reaches
    // the same conclusion.
    if outcome != Outcome::Kept {
        hashes.insert(file.path.clone(), hash(&file.contents));
    }
    Ok(outcome)
}

/// The placeholder icons.
///
/// Written once and never replaced: an icon is the first thing a project
/// changes, and a hash check on a binary file that a designer exported would
/// be a check that always says "edited".
fn write_icons(project: &Project) -> Result<Vec<(String, Outcome)>, NativeError> {
    let dir = project.native_dir().join("icons");
    let mut outcomes = Vec::new();

    for (name, size) in icons::SIZES {
        let path = dir.join(name);
        outcomes.push((
            relative(project, &path),
            put_bytes(&path, &icons::png(*size))?,
        ));
    }

    let path = dir.join(icons::ICO);
    outcomes.push((relative(project, &path), put_bytes(&path, &icons::ico())?));
    Ok(outcomes)
}

fn put(path: &Path, contents: &str, overwrite: bool) -> Result<Outcome, NativeError> {
    if path.is_file() {
        let current = std::fs::read_to_string(path)
            .map_err(|e| NativeError::io("init", path, e))?
            .replace("\r\n", "\n");
        if current == contents {
            return Ok(Outcome::Unchanged);
        }
        if !overwrite {
            return Ok(Outcome::Kept);
        }
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| NativeError::io("init", parent, e))?;
    }
    let created = !path.exists();
    std::fs::write(path, contents).map_err(|e| NativeError::io("init", path, e))?;

    Ok(if created {
        Outcome::Created
    } else {
        Outcome::Updated
    })
}

fn put_bytes(path: &Path, contents: &[u8]) -> Result<Outcome, NativeError> {
    if path.is_file() {
        return Ok(Outcome::Unchanged);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| NativeError::io("init", parent, e))?;
    }
    std::fs::write(path, contents).map_err(|e| NativeError::io("init", path, e))?;
    Ok(Outcome::Created)
}

/// A stable content hash.
///
/// Not a cryptographic one and not asked to be: it answers "has this file
/// changed since we wrote it", where the adversary is a text editor. FNV-1a
/// over the newline-normalized text, so a checkout with CRLF line endings does
/// not read as edited on every file.
fn hash(contents: &str) -> String {
    let mut value = 0xcbf2_9ce4_8422_2325u64;
    for byte in contents.replace("\r\n", "\n").as_bytes() {
        value ^= *byte as u64;
        value = value.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{value:016x}")
}

fn relative(project: &Project, path: &Path) -> String {
    path.strip_prefix(&project.root)
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/")
}

fn title_case(package: &str) -> String {
    package
        .split(['-', '_'])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn report(outcomes: &[(String, Outcome)], config: &NativeConfig, project: &Project) {
    let mut kept = Vec::new();
    let mut changed = 0;

    for (path, outcome) in outcomes {
        match outcome {
            Outcome::Created => {
                println!("  created   {path}");
                changed += 1;
            }
            Outcome::Updated => {
                println!("  updated   {path}");
                changed += 1;
            }
            Outcome::Kept => kept.push(path),
            Outcome::Unchanged => {}
        }
    }

    if changed == 0 && kept.is_empty() {
        println!("  nothing to do — the native shell is current.");
    }

    if !kept.is_empty() {
        println!();
        println!("  These were edited since they were generated, and were left alone:");
        for path in &kept {
            println!("    {path}");
        }
        println!("  Take them back with `cargo rahti native init --force`, which does not");
        println!("  keep what it replaces.");
    }

    println!();
    println!(
        "  {} {} — targets: {}",
        config.product_name,
        config.identifier,
        config.targets.join(", ")
    );
    println!();

    if !project.has_shared_startup() {
        println!("  This project's startup is still in src/main.rs, where the native shell");
        println!("  cannot reach it. Run `cargo rahti native doctor` for what to do.");
        println!();
    }

    println!("  Next:");
    println!("    cargo rahti native doctor");
    for target in &config.targets {
        println!("    cargo rahti native dev --target {target}");
    }
}

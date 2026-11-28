//! Android devices and emulators: which are attached, which could be started,
//! and starting one.
//!
//! An emulator is the one device an agent can bring up on its own, so the
//! whole route — find an AVD, boot it, wait until it can take an install — is
//! a command that ends when the device is ready, and is reachable from
//! `build --device` with nothing else running.
//!
//! Nothing here creates an AVD or downloads a system image. Those are
//! installs, and `doctor` names the command for them.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rahti_native::NativeError;

use crate::doctor;
use crate::project::Project;

/// One `adb devices -l` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Attached {
    pub serial: String,
    /// `device` when it takes commands; `offline`, `unauthorized` otherwise.
    pub state: String,
    pub model: Option<String>,
}

impl Attached {
    pub(crate) fn ready(&self) -> bool {
        self.state == "device"
    }

    pub(crate) fn emulator(&self) -> bool {
        self.serial.starts_with("emulator-")
    }
}

/// `adb devices -l`, read.
pub(crate) fn parse_attached(output: &str) -> Vec<Attached> {
    output
        .lines()
        .skip_while(|line| !line.starts_with("List of devices"))
        .skip(1)
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let serial = columns.next()?.to_string();
            let state = columns.next()?.to_string();
            let model = columns
                .find_map(|c| c.strip_prefix("model:"))
                .map(str::to_string);
            Some(Attached {
                serial,
                state,
                model,
            })
        })
        .collect()
}

pub(crate) fn attached() -> Vec<Attached> {
    adb(&["devices", "-l"])
        .map(|text| parse_attached(&text))
        .unwrap_or_default()
}

/// One `adb` command's stdout, or `None` if it could not run or failed.
fn adb(args: &[&str]) -> Option<String> {
    let output = Command::new(doctor::adb())
        .args(args)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).to_string())
}

/// The emulator executable in the SDK.
pub(crate) fn emulator_binary() -> Option<PathBuf> {
    let sdk = doctor::android_sdk()?;
    let path = sdk.join(if cfg!(windows) {
        "emulator/emulator.exe"
    } else {
        "emulator/emulator"
    });
    path.is_file().then_some(path)
}

/// The AVDs this machine has, by name.
pub(crate) fn avds() -> Vec<String> {
    let Some(emulator) = emulator_binary() else {
        return Vec::new();
    };
    Command::new(emulator)
        .arg("-list-avds")
        .stdin(Stdio::null())
        .output()
        .map(|output| parse_avds(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
}

/// `emulator -list-avds`, without the `INFO |` lines newer emulators mix in.
pub(crate) fn parse_avds(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.contains('|') && !line.contains(' '))
        .map(str::to_string)
        .collect()
}

/// Which AVD a running emulator is.
fn avd_of(serial: &str) -> Option<String> {
    let text = adb(&["-s", serial, "emu", "avd", "name"])?;
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && *line != "OK")
        .map(str::to_string)
}

fn booted(serial: &str) -> bool {
    adb(&["-s", serial, "shell", "getprop", "sys.boot_completed"]).is_some_and(|v| v.trim() == "1")
}

// ------------------------------------------------------------ resolving

/// Turn what `--device` named into a serial that is ready for an install.
///
/// In this order:
///
/// 1. a serial `adb devices` lists as ready;
/// 2. the name of an AVD — its serial if it is running, otherwise it is
///    booted and waited for;
/// 3. `emulator` — the one running emulator, or, with none running, the only
///    AVD (or the first, when there are several, which is said).
pub(crate) fn resolve(project: &Project, name: &str) -> Result<String, NativeError> {
    let attached = attached();
    if attached.iter().any(|d| d.serial == name && d.ready()) {
        return Ok(name.to_string());
    }
    if let Some(device) = attached.iter().find(|d| d.serial == name) {
        return Err(NativeError::new(
            "android",
            format!(
                "`{name}` is attached but {}: {}.",
                device.state,
                if device.state == "unauthorized" {
                    "accept the debugging prompt on the device"
                } else {
                    "wait for it, or restart it"
                }
            ),
        ));
    }

    let avds = avds();
    let running_emulators: Vec<&Attached> = attached
        .iter()
        .filter(|d| d.emulator() && d.ready())
        .collect();

    let avd = if avds.iter().any(|a| a == name) {
        name.to_string()
    } else if name == "emulator" {
        if let [only] = running_emulators.as_slice() {
            return Ok(only.serial.clone());
        }
        if running_emulators.len() > 1 {
            return Err(NativeError::new(
                "android",
                format!(
                    "`--device emulator` is ambiguous: {} are running. Name one.",
                    running_emulators
                        .iter()
                        .map(|d| d.serial.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
        let Some(first) = avds.first() else {
            return Err(no_device(name, &attached, &avds));
        };
        if avds.len() > 1 {
            println!(
                "  emulator: {} AVDs, starting {first}. Name another with --device <avd>.",
                avds.len()
            );
        }
        first.clone()
    } else {
        return Err(no_device(name, &attached, &avds));
    };

    // The AVD may already be up under some serial.
    for device in &running_emulators {
        if avd_of(&device.serial).as_deref() == Some(avd.as_str()) {
            return Ok(device.serial.clone());
        }
    }
    boot(project, &avd, Duration::from_secs(300))
}

fn no_device(name: &str, attached: &[Attached], avds: &[String]) -> NativeError {
    let ready: Vec<&str> = attached
        .iter()
        .filter(|d| d.ready())
        .map(|d| d.serial.as_str())
        .collect();
    NativeError::new(
        "android",
        format!(
            "`{name}` is neither an attached device nor an AVD.\n  \
             attached: {}\n  \
             AVDs: {}\n  \
             `cargo rahti native devices` lists both; `--device emulator` takes the running \
             emulator, or starts one.",
            if ready.is_empty() {
                "none ready".to_string()
            } else {
                ready.join(", ")
            },
            if avds.is_empty() {
                "none — create one in Android Studio's Device Manager".to_string()
            } else {
                avds.join(", ")
            }
        ),
    )
}

/// Start an AVD and wait until it can take an install. Returns its serial.
///
/// The emulator is started detached, with its output in
/// `native/.rahti/emulator.log`, so it outlives this command and holds no
/// pipe of the caller's open.
pub(crate) fn boot(project: &Project, avd: &str, wait: Duration) -> Result<String, NativeError> {
    let emulator = emulator_binary().ok_or_else(|| {
        NativeError::new(
            "android",
            "the Android SDK has no emulator. Install it with:\n    \
             sdkmanager \"emulator\"",
        )
    })?;

    let before: Vec<String> = attached().into_iter().map(|d| d.serial).collect();

    let log_path = project.native_dir().join(".rahti/emulator.log");
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| NativeError::io("android", parent, e))?;
    }
    let log =
        std::fs::File::create(&log_path).map_err(|e| NativeError::io("android", &log_path, e))?;
    let log_err = log
        .try_clone()
        .map_err(|e| NativeError::io("android", &log_path, e))?;

    let mut command = Command::new(&emulator);
    command
        .args(["-avd", avd, "-no-boot-anim", "-no-snapshot-save"])
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(log_err);
    detach(&mut command);
    let mut child = command.spawn().map_err(|e| {
        NativeError::new(
            "android",
            format!("could not start the emulator {}: {e}", emulator.display()),
        )
    })?;

    println!("  emulator: starting {avd}…");
    let started = Instant::now();
    let mut serial: Option<String> = None;
    while started.elapsed() < wait {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(NativeError::at(
                "android",
                &log_path,
                format!(
                    "the emulator for {avd} exited ({status}) before it booted. Its output is here."
                ),
            ));
        }
        if serial.is_none() {
            serial = attached()
                .into_iter()
                .find(|d| d.emulator() && !before.contains(&d.serial))
                .map(|d| d.serial);
        }
        if let Some(serial) = &serial
            && booted(serial)
        {
            println!(
                "  emulator: {avd} is ready as {serial} ({}s)",
                started.elapsed().as_secs()
            );
            return Ok(serial.clone());
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    Err(NativeError::at(
        "android",
        &log_path,
        format!(
            "{avd} did not finish booting in {}s. It is still starting; run the command again \
             once `cargo rahti native devices` lists it as ready.",
            wait.as_secs()
        ),
    ))
}

/// Leave the emulator running after this command, without a console of its
/// own on Windows.
#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(windows))]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // Its own process group, so a Ctrl-C at this terminal does not reach it.
    command.process_group(0);
}

// --------------------------------------------------------------- command

/// `cargo rahti native devices`: what is attached, and what could be started.
pub fn list(args: &crate::args::Devices) -> Result<(), NativeError> {
    let attached = attached();
    let avds = avds();
    let running: Vec<(String, Option<String>)> = attached
        .iter()
        .filter(|d| d.emulator() && d.ready())
        .map(|d| (d.serial.clone(), avd_of(&d.serial)))
        .collect();
    let abi = |serial: &str| {
        adb(&["-s", serial, "shell", "getprop", "ro.product.cpu.abi"]).map(|v| v.trim().to_string())
    };

    if args.json {
        let devices: Vec<serde_json::Value> = attached
            .iter()
            .map(|d| {
                serde_json::json!({
                    "serial": d.serial,
                    "state": d.state,
                    "ready": d.ready(),
                    "model": d.model,
                    "emulator": d.emulator(),
                    "avd": running.iter().find(|(s, _)| *s == d.serial).and_then(|(_, a)| a.clone()),
                    "abi": if d.ready() { abi(&d.serial) } else { None },
                })
            })
            .collect();
        let avd_list: Vec<serde_json::Value> = avds
            .iter()
            .map(|name| {
                let serial = running
                    .iter()
                    .find(|(_, a)| a.as_deref() == Some(name.as_str()))
                    .map(|(s, _)| s.clone());
                serde_json::json!({ "name": name, "running": serial.is_some(), "serial": serial })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "devices": devices,
                "avds": avd_list,
            }))
            .unwrap_or_default()
        );
        return Ok(());
    }

    println!();
    println!("  Attached:");
    if attached.is_empty() {
        println!("    none");
    }
    for device in &attached {
        let avd = running
            .iter()
            .find(|(s, _)| *s == device.serial)
            .and_then(|(_, a)| a.clone());
        println!(
            "    {:<18} {:<12} {}{}{}",
            device.serial,
            device.state,
            device.model.as_deref().unwrap_or(""),
            avd.map(|a| format!("  avd {a}")).unwrap_or_default(),
            if device.ready() {
                abi(&device.serial)
                    .map(|a| format!("  {a}"))
                    .unwrap_or_default()
            } else {
                String::new()
            }
        );
    }
    println!();
    println!("  AVDs:");
    if avds.is_empty() {
        println!("    none — create one in Android Studio's Device Manager");
    }
    for name in &avds {
        let up = running
            .iter()
            .find(|(_, a)| a.as_deref() == Some(name.as_str()));
        println!(
            "    {name}{}",
            up.map(|(s, _)| format!("  (running as {s})"))
                .unwrap_or_default()
        );
    }
    println!();
    println!(
        "  Install and start on one: cargo rahti native build --target android --debug --device <serial|avd|emulator>"
    );
    Ok(())
}

/// `cargo rahti native emulator [<avd>]`: have an emulator ready, and say its
/// serial.
pub fn emulator(project: &Project, args: &crate::args::Emulator) -> Result<(), NativeError> {
    let name = args.avd.as_deref().unwrap_or("emulator");
    let serial = resolve(project, name)?;
    println!("  ready: {serial}");
    Ok(())
}

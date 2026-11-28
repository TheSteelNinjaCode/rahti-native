//! What a build is doing, for whoever is not watching it.
//!
//! An Android build takes minutes, and agents run it in the background and come
//! back later. Coming back has to work without the terminal: an agent's
//! background task can outlive its notification, a second build started "to
//! check" waits on the first one's Cargo lock and looks hung, and the one line
//! kept from thousands of lines of Gradle is rarely the error.
//!
//! So a build:
//!
//! - writes `native/.rahti/build.json` when it *starts*, with `state:
//!   "running"`, its process id and the phase it is in, and rewrites it at
//!   every phase and at the end, with how long each phase took;
//! - copies every line of the tooling's output into `native/.rahti/build.log`,
//!   and on failure prints the end of it again last, where a kept tail sees it;
//! - refuses to start while another build of the same project is running;
//! - and does not wait for a daemon that holds its output open after the
//!   build itself has ended (see [`run_logged`]).
//!
//! `cargo rahti native status --wait` reads the file and waits for the build
//! it describes, which is the one command an agent needs after losing track.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rahti_native::NativeError;

use crate::project::Project;

/// Where the current or last build is described.
pub(crate) fn report_path(project: &Project) -> PathBuf {
    project.native_dir().join(".rahti/build.json")
}

/// Where the current or last build's full output is.
pub(crate) fn log_path(project: &Project) -> PathBuf {
    project.native_dir().join(".rahti/build.log")
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `2m41s`, `48s`, `1h02m`.
pub(crate) fn human(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m{:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h{:02}m", seconds / 3600, (seconds % 3600) / 60),
    }
}

// ---------------------------------------------------------------- state

/// The build in progress, mirrored into `build.json` as it goes.
pub(crate) struct Progress {
    path: PathBuf,
    started: Instant,
    report: serde_json::Value,
    phase: Option<(String, Instant)>,
    phases: Vec<(String, u64)>,
}

impl Progress {
    /// Claim the project for a build, or say which build already has it.
    pub(crate) fn start(project: &Project, command: &str) -> Result<Self, NativeError> {
        let path = report_path(project);
        if let Some(running) = running(&path) {
            return Err(NativeError::new(
                "build",
                format!(
                    "a build of this project is already running (process {}, started {} ago: \
                     `{}`).\n  \
                     A second one would wait on its Cargo lock and look hung. Wait for it with:\n    \
                     cargo rahti native status --wait",
                    running.pid,
                    human(now().saturating_sub(running.started)),
                    running.command,
                ),
            ));
        }

        let log = log_path(project);
        let mut progress = Progress {
            path,
            started: Instant::now(),
            report: serde_json::json!({
                "state": "running",
                "pid": std::process::id(),
                "command": command,
                "started": now(),
                "log": log,
            }),
            phase: None,
            phases: Vec::new(),
        };
        if let Some(parent) = progress.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| NativeError::io("build", parent, e))?;
        }
        // A fresh log per build: what is in it is this build's, and nothing older.
        std::fs::write(&log, "").map_err(|e| NativeError::io("build", &log, e))?;
        progress.save();
        Ok(progress)
    }

    /// End the current phase, if any, and begin `name`.
    pub(crate) fn phase(&mut self, name: &str) {
        self.end_phase();
        self.phase = Some((name.to_string(), Instant::now()));
        println!("  [{}] {name}", human(self.elapsed()));
        self.save();
    }

    fn end_phase(&mut self) {
        if let Some((name, began)) = self.phase.take() {
            self.phases.push((name, began.elapsed().as_secs()));
        }
    }

    pub(crate) fn elapsed(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    /// Fields the finished report carries: artifacts, warnings, signing.
    pub(crate) fn merge(&mut self, fields: serde_json::Value) {
        if let (Some(report), serde_json::Value::Object(fields)) =
            (self.report.as_object_mut(), fields)
        {
            report.extend(fields);
        }
    }

    /// The build ended. Returns the one line that says how it went.
    pub(crate) fn finish(&mut self, error: Option<&NativeError>) -> String {
        self.end_phase();
        let seconds = self.elapsed();
        self.report["state"] = if error.is_some() {
            "failed"
        } else {
            "succeeded"
        }
        .into();
        self.report["finished"] = now().into();
        self.report["seconds"] = seconds.into();
        if let Some(error) = error {
            self.report["error"] = error.to_string().into();
        }
        self.save();

        let breakdown = self
            .phases
            .iter()
            .map(|(name, seconds)| format!("{name} {}", human(*seconds)))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{} in {} ({breakdown}).",
            if error.is_some() {
                "Failed"
            } else {
                "Finished"
            },
            human(seconds)
        )
    }

    fn save(&mut self) {
        let mut phases: Vec<serde_json::Value> = self
            .phases
            .iter()
            .map(|(name, seconds)| serde_json::json!({ "name": name, "seconds": seconds }))
            .collect();
        if let Some((name, began)) = &self.phase {
            let seconds = began.elapsed().as_secs();
            phases.push(serde_json::json!({
                "name": name,
                "seconds": seconds,
                "since": now().saturating_sub(seconds),
                "running": true,
            }));
            self.report["phase"] = name.as_str().into();
        } else if let Some(report) = self.report.as_object_mut() {
            report.remove("phase");
        }
        self.report["phases"] = phases.into();
        self.report["updated"] = now().into();

        let text = serde_json::to_string_pretty(&self.report).unwrap_or_default();
        // Best effort: a report that cannot be written must not fail a build
        // that worked. The terminal still has the answer.
        let _ = std::fs::write(&self.path, text);
    }
}

/// A build that `build.json` says is running, and whose process still is.
pub(crate) struct Running {
    pub pid: u32,
    pub started: u64,
    pub command: String,
}

pub(crate) fn running(path: &Path) -> Option<Running> {
    let report = read(path)?;
    if report["state"] != "running" {
        return None;
    }
    let pid = u32::try_from(report["pid"].as_u64()?).ok()?;
    (pid != std::process::id() && process_alive(pid)).then(|| Running {
        pid,
        started: report["started"].as_u64().unwrap_or(0),
        command: report["command"].as_str().unwrap_or_default().to_string(),
    })
}

fn read(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Whether a process with this id exists. No dependency: the platform's own
/// tool answers it.
pub(crate) fn process_alive(pid: u32) -> bool {
    if cfg!(windows) {
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .stdin(Stdio::null())
            .output()
            .is_ok_and(|output| {
                String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
            })
    } else {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

/// What `build.json` says, with a `running` build whose process is gone
/// called what it is: interrupted.
pub(crate) fn current(path: &Path) -> Option<serde_json::Value> {
    let mut report = read(path)?;
    if report["state"] == "running" {
        let alive = report["pid"]
            .as_u64()
            .and_then(|pid| u32::try_from(pid).ok())
            .is_some_and(process_alive);
        if alive {
            // The file is written at phase changes; the clock keeps going.
            report["seconds"] = now()
                .saturating_sub(report["started"].as_u64().unwrap_or(0))
                .into();
            if let Some(phases) = report["phases"].as_array_mut() {
                for phase in phases.iter_mut().filter(|p| p["running"] == true) {
                    if let Some(since) = phase["since"].as_u64() {
                        phase["seconds"] = now().saturating_sub(since).into();
                    }
                }
            }
        } else {
            report["state"] = "interrupted".into();
        }
    }
    Some(report)
}

// --------------------------------------------------------------- status

/// `cargo rahti native status`: what the current or last build is doing or
/// did, optionally after waiting for it.
///
/// Exit status says it without parsing: success for a build that succeeded,
/// failure for one that failed or was interrupted, and failure with "still
/// running" when `--wait` ran out of time.
pub fn status(project: &Project, args: &crate::args::Status) -> Result<(), NativeError> {
    let path = report_path(project);
    let deadline = args
        .timeout
        .map(|s| Instant::now() + Duration::from_secs(s));
    let mut announced = false;

    let report = loop {
        let Some(report) = current(&path) else {
            return Err(NativeError::at(
                "status",
                &path,
                "no build has been recorded for this project yet.",
            ));
        };
        let waiting = args.wait
            && report["state"] == "running"
            && deadline.is_none_or(|deadline| Instant::now() < deadline);
        if !waiting {
            break report;
        }
        if !announced && !args.json {
            println!(
                "  waiting for process {} ({}), running for {}…",
                report["pid"],
                report["command"].as_str().unwrap_or("a build"),
                human(report["seconds"].as_u64().unwrap_or(0))
            );
            announced = true;
        }
        std::thread::sleep(Duration::from_secs(2));
    };

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_default()
        );
    } else {
        print_status(&report);
    }

    match report["state"].as_str() {
        Some("succeeded") => Ok(()),
        Some("running") => Err(NativeError::new(
            "status",
            "the build is still running. `--wait` waits for it.",
        )),
        Some("interrupted") => Err(NativeError::new(
            "status",
            "the build was interrupted: its process ended without finishing the report. \
             Run it again.",
        )),
        _ => Err(NativeError::new("status", "the last build failed.")),
    }
}

fn print_status(report: &serde_json::Value) {
    let state = report["state"].as_str().unwrap_or("unknown");
    let seconds = report["seconds"].as_u64().unwrap_or(0);
    println!();
    println!(
        "  {state}: {} ({})",
        report["command"].as_str().unwrap_or("build"),
        human(seconds)
    );
    if let Some(phase) = report["phase"].as_str() {
        println!("  phase: {phase}");
    }
    if let Some(phases) = report["phases"].as_array() {
        for phase in phases {
            println!(
                "    {:<10} {}",
                phase["name"].as_str().unwrap_or(""),
                human(phase["seconds"].as_u64().unwrap_or(0))
            );
        }
    }
    if let Some(artifacts) = report["artifacts"].as_array() {
        for artifact in artifacts.iter().filter_map(|a| a.as_str()) {
            println!("  built: {artifact}");
        }
    }
    if report["installed"] == true {
        println!(
            "  installed and started on {}",
            report["device"].as_str().unwrap_or("the device")
        );
    }
    if let Some(error) = report["error"].as_str() {
        println!("  error: {error}");
    }
    if let Some(log) = report["log"].as_str() {
        println!("  log: {log}");
    }
}

// -------------------------------------------------------------- running

/// Run a command with its output shown, copied into `log`, and handed line by
/// line to `on_line`.
///
/// ## Why the output is piped rather than inherited
///
/// Gradle starts a daemon on its first build, and the Kotlin compiler starts
/// another. On Windows a process inherits every inheritable handle its parent
/// has, so a daemon started under an inherited terminal keeps that terminal's
/// pipe open for as long as it lives — hours. Whatever is reading the pipe (an
/// agent's background task, a CI step, `cmd | tee`) then never sees it close,
/// and reports the build as running long after it ended.
///
/// Piped, the daemon holds *this* process's pipe instead. So the readers are
/// not waited for once the child has exited: they get a moment to drain, and
/// are then left behind, and this process exits as the build did.
pub(crate) fn run_logged(
    mut command: Command,
    log: &Path,
    mut on_line: impl FnMut(&str) + Send + 'static,
) -> std::io::Result<std::process::ExitStatus> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .ok();
    let shared = Arc::new(Mutex::new(Sink {
        file,
        on_line: Box::new(move |line: &str| on_line(line)),
    }));

    let (done, finished) = mpsc::channel();
    let mut readers = 0;
    if let Some(stdout) = child.stdout.take() {
        spawn_reader(stdout, false, shared.clone(), done.clone());
        readers += 1;
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_reader(stderr, true, shared.clone(), done.clone());
        readers += 1;
    }
    drop(done);

    let status = child.wait()?;

    // Normally both close at once with the child. A daemon holding one open is
    // the case above; three seconds drains what the child wrote last.
    let grace = Instant::now() + Duration::from_secs(3);
    for _ in 0..readers {
        let left = grace.saturating_duration_since(Instant::now());
        if finished.recv_timeout(left).is_err() {
            break;
        }
    }
    Ok(status)
}

struct Sink {
    file: Option<std::fs::File>,
    on_line: Box<dyn FnMut(&str) + Send>,
}

fn spawn_reader(
    stream: impl Read + Send + 'static,
    to_stderr: bool,
    sink: Arc<Mutex<Sink>>,
    done: mpsc::Sender<()>,
) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut line = Vec::new();
        while reader.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
            if to_stderr {
                let _ = std::io::stderr().write_all(&line);
            } else {
                let _ = std::io::stdout().write_all(&line);
            }
            if let Ok(mut sink) = sink.lock() {
                let plain = strip_ansi(&line);
                if let Some(file) = sink.file.as_mut() {
                    let _ = file.write_all(&plain);
                }
                let text = String::from_utf8_lossy(&plain);
                (sink.on_line)(text.trim_end());
            }
            line.clear();
        }
        let _ = done.send(());
    });
}

/// The bytes without terminal colour codes, which make a log unreadable to
/// anything but a terminal.
pub(crate) fn strip_ansi(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'[') {
            i += 2;
            while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                i += 1;
            }
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// The last `count` lines of the log, for a failure's last word.
pub(crate) fn tail(log: &Path, count: usize) -> Vec<String> {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(count)..]
        .iter()
        .map(|l| l.to_string())
        .collect()
}

/// Stop the children this process starts from inheriting its own standard
/// handles.
///
/// The other half of [`run_logged`]: a child given a pipe still inherits every
/// *other* inheritable handle, and this process's stdout — the agent's or the
/// terminal's pipe — is one. Rust duplicates a handle it is asked to pass on
/// explicitly (`Stdio::inherit`), so marking the originals does not stop a
/// command that inherits the terminal on purpose.
#[cfg(windows)]
pub(crate) fn keep_std_handles_to_ourselves() {
    type Handle = *mut core::ffi::c_void;
    unsafe extern "system" {
        fn GetStdHandle(which: u32) -> Handle;
        fn SetHandleInformation(handle: Handle, mask: u32, flags: u32) -> i32;
    }
    const HANDLE_FLAG_INHERIT: u32 = 1;
    // STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE: -10, -11, -12.
    for which in [-10i32, -11, -12] {
        // SAFETY: both are plain kernel32 calls on this process's own standard
        // handles. A null or invalid handle makes SetHandleInformation fail,
        // which is ignored: nothing is changed in that case.
        unsafe {
            let handle = GetStdHandle(which as u32);
            if !handle.is_null() && handle as isize != -1 {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn keep_std_handles_to_ourselves() {}

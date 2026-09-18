//! Target validation and the two injection strategies.
//!
//! * `Attach` mirrors the default path of `tools/inject.sh`: HotSpot's Attach
//!   API loads the JVMTI agent.
//! * `Force` mirrors `tools/inject.sh --force`: the native `ptrace` injector
//!   is used instead, which also works when Attach is disabled.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::runtime::{CancelToken, Job, LogBuffer, LogKind, Msg};
use crate::targets::Target;
use crate::util::{injection_state_dir, JavaRuntime};

pub const VERIFIED_MARKER: &str = "NativeBridge.start completed; Linux injection is active";
pub const FORCE_FAILED_MARKER: &str = "FORCE_BOOTSTRAP_FAILED";
pub const THREAD_TIMEOUT_MARKER: &str = "Minecraft client/render thread was not found";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InjectMode {
    Attach,
    Force,
}

impl InjectMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Attach => "Attach",
            Self::Force => "Force (ptrace)",
        }
    }
}

/// The files produced by `./gradlew prepareInjectionBundle`.
#[derive(Clone, Debug)]
pub struct Artifacts {
    pub dir: PathBuf,
    pub agent: PathBuf,
    pub payload: PathBuf,
    pub attach_jar: PathBuf,
    pub force_binary: PathBuf,
}

impl Artifacts {
    pub fn detect(dir: &Path) -> Option<Self> {
        let artifacts = Self {
            dir: dir.to_path_buf(),
            agent: dir.join("libVape421Native.so"),
            payload: dir.join("Vape421Payload.jar"),
            attach_jar: dir.join("Vape421LinuxInjector.jar"),
            force_binary: dir.join("Vape421LinuxNativeInjector"),
        };
        Some(artifacts)
    }

    /// Names of the files that are missing or unreadable.
    pub fn missing(&self) -> Vec<String> {
        let checks = [
            ("libVape421Native.so", &self.agent),
            ("Vape421Payload.jar", &self.payload),
            ("Vape421LinuxInjector.jar", &self.attach_jar),
            ("Vape421LinuxNativeInjector", &self.force_binary),
        ];
        checks
            .into_iter()
            .filter(|(_, path)| !is_readable(path))
            .map(|(name, _)| name.to_string())
            .collect()
    }

    pub fn ready(&self) -> bool {
        self.missing().is_empty()
    }

    /// Directories that may hold the native bootstrap log for a PID.
    pub fn log_candidates(&self, pid: u32) -> Vec<PathBuf> {
        let name = format!("vape421-native-{pid}.log");
        vec![injection_state_dir().join(&name), self.dir.join(name)]
    }
}

fn is_readable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.is_file())
        .unwrap_or(false)
}

/// Locates `build/injection` next to the repository or the executable.
pub fn find_artifacts(repo_root: Option<&Path>) -> Option<Artifacts> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(root) = repo_root {
        candidates.push(root.join("build/injection"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("injection"));
            candidates.push(parent.to_path_buf());
        }
    }
    candidates
        .into_iter()
        .find(|dir| dir.join("Vape421Payload.jar").is_file())
        .and_then(|dir| Artifacts::detect(&dir))
}

pub fn runtime_agent_path(agent: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(agent)
        .map_err(|error| format!("Cannot read {}: {error}", agent.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let short: String = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(injection_state_dir().join(format!("libVape421Native-{short}.so")))
}

/// Replicates the immutable, content-addressed copy the shell helper performs
/// before a force injection.
pub fn prepare_force_agent(
    agent: &Path,
    target_uid: u32,
    target_gid: u32,
    log: &LogBuffer,
) -> Result<PathBuf, String> {
    let runtime_agent = runtime_agent_path(agent)?;
    let dir = runtime_agent
        .parent()
        .ok_or_else(|| "Invalid runtime agent path".to_string())?;

    std::fs::create_dir_all(dir)
        .map_err(|error| format!("Cannot create {}: {error}", dir.display()))?;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));

    if !runtime_agent.exists() {
        std::fs::copy(agent, &runtime_agent).map_err(|error| {
            format!(
                "Cannot copy the agent to {}: {error}",
                runtime_agent.display()
            )
        })?;
        let _ = std::fs::set_permissions(&runtime_agent, std::fs::Permissions::from_mode(0o400));
        if crate::util::effective_uid() == 0 {
            let _ = std::os::unix::fs::chown(&runtime_agent, Some(target_uid), Some(target_gid));
        }
        log.push(
            "inject",
            LogKind::Dim,
            format!("Published immutable agent: {}", runtime_agent.display()),
        );
    }

    // `cmp -s` in the shell helper: never load an artifact that was tampered with.
    let original = std::fs::read(agent).map_err(|error| error.to_string())?;
    let installed = std::fs::read(&runtime_agent).map_err(|error| error.to_string())?;
    if original != installed {
        return Err(format!(
            "Immutable runtime agent does not match the build artifact: {}",
            runtime_agent.display()
        ));
    }
    Ok(runtime_agent)
}

fn target_gid(pid: u32) -> u32 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("Gid:")
                    .and_then(|rest| rest.split_whitespace().nth(1))
                    .and_then(|value| value.parse().ok())
            })
        })
        .unwrap_or(0)
}

/// Everything that must hold before the target is touched.
pub fn preflight(
    target: &Target,
    artifacts: &Artifacts,
    mode: InjectMode,
    java: &JavaRuntime,
) -> Result<(), String> {
    let missing = artifacts.missing();
    if !missing.is_empty() {
        return Err(format!("Missing build output: {}", missing.join(", ")));
    }
    if mode == InjectMode::Attach {
        if !is_readable(&java.path) {
            return Err(format!("Java runtime not found: {}", java.path.display()));
        }
        if target.attach_disabled {
            return Err(
                "This JVM was started with -XX:+DisableAttachMechanism. Use force mode, or \
                 enable attach in the launcher and restart the game."
                    .to_string(),
            );
        }
    }
    if mode == InjectMode::Force && !is_executable(&artifacts.force_binary) {
        return Err(format!(
            "The native injector is not executable: {}",
            artifacts.force_binary.display()
        ));
    }
    Ok(())
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Spawns the injector for `target` and returns the running job.
pub fn launch(
    target: &Target,
    artifacts: &Artifacts,
    mode: InjectMode,
    java: &JavaRuntime,
    owner_uid: u32,
    log: &LogBuffer,
) -> Result<Job, String> {
    preflight(target, artifacts, mode, java)?;

    match mode {
        InjectMode::Attach => {
            log.push(
                "inject",
                LogKind::Warn,
                format!(
                    "The agent and payload inherit every permission of PID {}",
                    target.pid
                ),
            );
            let mut command = Command::new(&java.path);
            command
                .arg("--add-modules")
                .arg("jdk.attach")
                .arg("-jar")
                .arg(&artifacts.attach_jar)
                .arg(target.pid.to_string())
                .arg(&artifacts.agent)
                .arg(&artifacts.payload);
            Job::spawn("inject", &mut command, log, &[], false)
                .map_err(|error| format!("Cannot start the injector: {error}"))
        }
        InjectMode::Force => {
            let agent =
                prepare_force_agent(&artifacts.agent, owner_uid, target_gid(target.pid), log)?;
            log.push(
                "inject",
                LogKind::Warn,
                format!(
                    "Force mode uses ptrace to execute native code inside PID {}",
                    target.pid
                ),
            );

            // Yama normally only lets a process trace its own descendants.
            let elevate = crate::util::ptrace_needs_root(target.pid);
            let mut command = if elevate {
                let prompt = crate::util::pkexec_path().ok_or_else(|| {
                    "ptrace needs root here but pkexec is not installed.                      Start OpenVape with sudo, or lower kernel.yama.ptrace_scope."
                        .to_string()
                })?;
                log.push(
                    "inject",
                    LogKind::Warn,
                    "ptrace is restricted by Yama; authorising through pkexec",
                );
                let mut command = Command::new(prompt);
                command.arg(&artifacts.force_binary);
                command
            } else {
                Command::new(&artifacts.force_binary)
            };
            command
                .arg(target.pid.to_string())
                .arg(&agent)
                .arg(&artifacts.payload);
            Job::spawn("inject", &mut command, log, &[], false)
                .map_err(|error| format!("Cannot start the native injector: {error}"))
        }
    }
}

/// Exit-code plus stdout interpretation, matching the shell helper's contract.
pub fn interpret(mode: InjectMode, job: &Job) -> Result<String, String> {
    match job.exit_code() {
        Some(0) => {
            let confirmed = match mode {
                InjectMode::Attach => job.stdout_contains("agent loaded into PID"),
                InjectMode::Force => {
                    job.stdout_contains("started its native bootstrap in PID")
                        || job.stdout_contains("native bootstrap")
                }
            };
            if confirmed {
                Ok("The agent was loaded into the target JVM".to_string())
            } else {
                Ok("The injector exited successfully".to_string())
            }
        }
        Some(code) => Err(format!("The injector exited with status {code}")),
        None => Err("The injector stopped without reporting a status".to_string()),
    }
}

/// Watches the native bootstrap log until the payload reports that it is live.
///
/// This is the only reliable confirmation: the injector returns as soon as the
/// library is loaded, while the payload still has to attach to the render thread.
pub fn watch_bootstrap(pid: u32, artifacts: &Artifacts, tx: Sender<Msg>, cancel: CancelToken) {
    let candidates = artifacts.log_candidates(pid);
    let deadline = Instant::now() + Duration::from_secs(95);
    std::thread::spawn(move || {
        let mut last_seen: Option<String> = None;
        loop {
            if cancel.stopped() {
                return;
            }
            for path in &candidates {
                let Ok(text) = std::fs::read_to_string(path) else {
                    continue;
                };
                if text.contains(VERIFIED_MARKER) {
                    let _ = tx.send(Msg::Bootstrap(Ok(VERIFIED_MARKER.to_string())));
                    return;
                }
                if text.contains(FORCE_FAILED_MARKER) {
                    let _ = tx.send(Msg::Bootstrap(Err(
                        "The native bootstrap reported FORCE_BOOTSTRAP_FAILED".to_string(),
                    )));
                    return;
                }
                if let Some(line) = text
                    .lines()
                    .rev()
                    .find(|line| !line.trim().is_empty())
                    .map(|line| line.to_string())
                {
                    last_seen = Some(line);
                }
            }
            if Instant::now() >= deadline {
                let detail = last_seen
                    .filter(|line| !line.contains("Loading Linux payload"))
                    .or_else(|| {
                        std::fs::read_to_string(candidates.first()?)
                            .ok()?
                            .lines()
                            .rev()
                            .find(|line| !line.trim().is_empty())
                            .map(|line| line.to_string())
                    });
                let reason = match detail {
                    Some(line) if line.contains(THREAD_TIMEOUT_MARKER) => {
                        "The payload could not find the Minecraft client thread".to_string()
                    }
                    Some(line) => format!("No confirmation after 95s · last log line: {line}"),
                    None => "No native bootstrap log was written".to_string(),
                };
                let _ = tx.send(Msg::Bootstrap(Err(reason)));
                return;
            }
            std::thread::sleep(Duration::from_millis(600));
        }
    });
}

/// Reads the tail of the bootstrap log for display.
pub fn bootstrap_tail(pid: u32, artifacts: &Artifacts, lines: usize) -> Option<String> {
    for path in artifacts.log_candidates(pid) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let collected: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            let start = collected.len().saturating_sub(lines);
            return Some(collected[start..].join("\n"));
        }
    }
    None
}

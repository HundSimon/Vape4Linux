//! Filesystem locations, Java discovery and small text helpers.

use std::path::{Path, PathBuf};
use std::process::Command;

/// `$HOME`, falling back to the passwd-independent default.
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn xdg_dir(variable: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(variable) {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => home_dir().join(fallback),
    }
}

/// Mirrors the helper scripts: `~/.local/state/vape4linux`.
pub fn state_dir() -> PathBuf {
    xdg_dir("XDG_STATE_HOME", ".local/state").join("vape4linux")
}

/// Where the injection helper keeps immutable content-addressed native libraries.
pub fn injection_state_dir() -> PathBuf {
    state_dir().join("injection")
}

/// Where the local service keeps its JSON store.
pub fn service_state_dir() -> PathBuf {
    state_dir().join("service")
}

/// Checkout of the service source used when no prebuilt jar exists.
pub fn service_source_dir() -> PathBuf {
    xdg_dir("XDG_DATA_HOME", ".local/share").join("vape4linux/VapeService")
}

pub fn config_path() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config")
        .join("openvape")
        .join("config.json")
}

/// The real user id, used to refuse touching JVMs owned by somebody else.
pub fn real_uid() -> u32 {
    uid_from_status().0
}

/// The effective user id; differs from the real one when running under sudo.
pub fn effective_uid() -> u32 {
    uid_from_status().1
}

/// Strictest owner check: when the GUI runs as root it still only manages the
/// JVMs of the user that launched it.
pub fn target_owner_uid() -> u32 {
    if effective_uid() == 0 && real_uid() != 0 {
        real_uid()
    } else {
        effective_uid()
    }
}

fn uid_from_status() -> (u32, u32) {
    let parse = |path: &str| -> Option<(u32, u32)> {
        let status = std::fs::read_to_string(path).ok()?;
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("Uid:") {
                let mut fields = rest.split_whitespace();
                let real = fields.next()?.parse().ok()?;
                let effective = fields.next().unwrap_or("").parse().unwrap_or(real);
                return Some((real, effective));
            }
        }
        None
    };
    parse("/proc/self/status").unwrap_or((0, 0))
}

/// Walks upwards looking for the Vape4Linux checkout that owns this build.
pub fn repo_root() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("OPENVAPE_ROOT") {
        let path = PathBuf::from(explicit);
        if looks_like_repo(&path) {
            return Some(path);
        }
    }

    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }

    for start in roots {
        for candidate in start.ancestors() {
            if looks_like_repo(candidate) {
                return Some(candidate.to_path_buf());
            }
        }
    }
    None
}

fn looks_like_repo(path: &Path) -> bool {
    path.join("tools/inject.sh").is_file()
        || path.join("build/injection/Vape421Payload.jar").is_file()
}

/// Looks for a runnable service jar, preferring an explicit override.
pub fn find_service_jar(root: Option<&Path>) -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("OPENVAPE_SERVICE_JAR") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Some(path);
        }
    }

    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(root) = root {
        dirs.push(root.join("build/service"));
        dirs.push(root.join("build/libs"));
    }
    dirs.push(service_source_dir().join("build/libs"));
    dirs.push(state_dir().join("service"));

    for dir in dirs {
        if let Some(found) = newest_jar(&dir, "vape421-experimental-service") {
            return Some(found);
        }
    }
    None
}

fn newest_jar(dir: &Path, prefix: &str) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(prefix) || !name.ends_with(".jar") {
            continue;
        }
        if name.ends_with("-sources.jar") || name.ends_with("-javadoc.jar") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(time, _)| modified > *time) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, path)| path)
}

/// A Java runtime, with the major version we detected for it.
#[derive(Clone, Debug)]
pub struct JavaRuntime {
    pub path: PathBuf,
    pub major: u32,
    pub origin: String,
}

impl JavaRuntime {
    pub fn display(&self) -> String {
        format!("Java {} · {}", self.major, self.origin)
    }
}

/// Runs `java -version` and extracts the feature release.
pub fn probe_java(path: &Path) -> Option<u32> {
    let output = Command::new(path).arg("-version").output().ok()?;
    let mut text = String::from_utf8_lossy(&output.stderr).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stdout));

    let start = text.find('"')? + 1;
    let rest = &text[start..];
    let end = rest.find('"').unwrap_or(rest.len());
    let version = &rest[..end];

    // "17.0.9", "21", "1.8.0_301"
    let mut parts = version.split(['.', '_', '-']);
    let first = parts.next()?;
    let major: u32 = first.parse().ok()?;
    if major == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(major)
    }
}

fn jvm_binaries() -> Vec<(PathBuf, String)> {
    let mut found: Vec<(PathBuf, String)> = Vec::new();

    if let Some(home) = std::env::var_os("JAVA_HOME") {
        let candidate = PathBuf::from(home).join("bin/java");
        if candidate.is_file() {
            found.push((candidate, "JAVA_HOME".to_string()));
        }
    }

    let mut system: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/usr/lib/jvm") {
        for entry in entries.flatten() {
            let candidate = entry.path().join("bin/java");
            if candidate.is_file() {
                system.push(candidate);
            }
        }
    }
    // Prefer the JDKs Gradle 8.8 can actually run on.
    system.sort_by_key(|path| {
        let text = path.to_string_lossy().to_lowercase();
        if text.contains("17") {
            0
        } else if text.contains("21") {
            1
        } else if text.contains("19") || text.contains("20") {
            2
        } else {
            3
        }
    });
    for path in system {
        let origin = path
            .parent()
            .and_then(|p| p.parent())
            .map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.display().to_string())
            })
            .unwrap_or_else(|| path.display().to_string());
        found.push((path, format!("/usr/lib/jvm/{origin}")));
    }

    // Launcher-provided runtimes are a good fallback for the attach injector.
    let launcher_roots = [
        home_dir().join(".local/share/FjordLauncher/java"),
        home_dir().join(".local/share/PrismLauncher/java"),
    ];
    for root in launcher_roots {
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let candidate = entry.path().join("bin/java");
                if candidate.is_file() {
                    found.push((candidate, "launcher runtime".to_string()));
                }
            }
        }
    }

    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let candidate = Path::new(dir).join("java");
            if candidate.is_file() {
                found.push((candidate, "PATH".to_string()));
                break;
            }
        }
    }

    found
}

/// Picks the best available JVM.
///
/// `min` is required, `max` exists because Gradle 8.8 refuses to run on newer
/// releases even though the service itself is happy on them.
pub fn discover_java(min: u32, max: Option<u32>) -> Option<JavaRuntime> {
    let mut best: Option<JavaRuntime> = None;
    for (path, origin) in jvm_binaries() {
        let Some(major) = probe_java(&path) else {
            continue;
        };
        if major < min {
            continue;
        }
        let in_range = max.is_none_or(|max| major <= max);
        let runtime = JavaRuntime {
            path,
            major,
            origin,
        };
        if in_range {
            return Some(runtime);
        }
        // Remember an out-of-range runtime so the caller can still try it.
        if best
            .as_ref()
            .is_none_or(|current| runtime.major < current.major)
        {
            best = Some(runtime);
        }
    }
    best
}

pub fn probe_java_path(path: &Path) -> Option<JavaRuntime> {
    let major = probe_java(path)?;
    Some(JavaRuntime {
        path: path.to_path_buf(),
        major,
        origin: "configured".to_string(),
    })
}

/// Removes SGR escape sequences and carriage returns from child output.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => {
                if chars.peek() == Some(&'[') {
                    chars.next();
                }
                for control in chars.by_ref() {
                    if control.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            '\r' => {}
            '\t' => out.push_str("  "),
            _ => out.push(ch),
        }
    }
    out
}

/// Gives a safe, human label for a Minecraft JVM without echoing its command
/// line, which routinely contains launcher access tokens.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// `CAP_SYS_PTRACE` is bit 19 of the effective capability set.
fn has_cap_sys_ptrace() -> bool {
    effective_capabilities()
        .map(|bits| bits & (1 << 19) != 0)
        .unwrap_or(false)
}

fn effective_capabilities() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("CapEff:") {
            return u64::from_str_radix(rest.trim(), 16).ok();
        }
    }
    None
}

/// Yama's `ptrace_scope`: 0 is permissive, 1 restricts tracing to descendants,
/// 2 restricts it to `CAP_SYS_PTRACE`, 3 forbids it entirely.
pub fn ptrace_scope() -> Option<u8> {
    std::fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope")
        .ok()
        .and_then(|value| value.trim().parse().ok())
}

fn parent_pid(pid: u32) -> Option<u32> {
    // `/proc/<pid>/stat` starts with "pid (comm) state ppid", and `comm` may
    // contain anything, so only the tail after the final ')' is parsed.
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let tail = stat.rsplit_once(')')?.1;
    tail.split_whitespace().nth(1)?.parse().ok()
}

fn is_descendant_of_self(pid: u32) -> bool {
    let me = std::process::id();
    let mut current = pid;
    for _ in 0..128 {
        if current == me {
            return true;
        }
        if current <= 1 {
            return false;
        }
        match parent_pid(current) {
            Some(parent) => current = parent,
            None => return false,
        }
    }
    false
}

/// True when `ptrace` into `pid` needs root: Yama's scope is restrictive, the
/// GUI has no `CAP_SYS_PTRACE`, and the target is not one of our children.
pub fn ptrace_needs_root(pid: u32) -> bool {
    if effective_uid() == 0 || has_cap_sys_ptrace() {
        return false;
    }
    match ptrace_scope() {
        Some(0) | None => false,
        Some(_) => !is_descendant_of_self(pid),
    }
}

/// Path of the graphical privilege prompt, when one is installed.
pub fn pkexec_path() -> Option<PathBuf> {
    ["/usr/bin/pkexec", "/bin/pkexec"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

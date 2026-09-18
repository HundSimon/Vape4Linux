//! Minecraft JVM discovery, mirroring the heuristics of `tools/inject.sh`.
//!
//! Command lines are inspected in-process but never stored or shown: launcher
//! arguments frequently contain account tokens.

use std::path::{Path, PathBuf};

/// A Java process that looks like a Minecraft client.
#[derive(Clone, Debug)]
pub struct Target {
    pub pid: u32,
    pub exe: PathBuf,
    pub kind: &'static str,
    pub attach_disabled: bool,
}

/// Markers taken verbatim from `tools/inject.sh`.
const MARKERS: &[&str] = &[
    "net.minecraft.client",
    "knotclient",
    "clientlaunchhandler",
    "forgeclient",
    "neoforgeclient",
    "lunarclient",
    "org.prismlauncher.entrypoint",
    "newlaunch.jar",
    "--gamedir",
    "--assetsdir",
];

/// `*minecraft-*client.jar*` is a glob in the shell script.
fn matches_minecraft_client_jar(lowercase: &str) -> bool {
    let mut search = lowercase;
    while let Some(index) = search.find("minecraft-") {
        let rest = &search[index + "minecraft-".len()..];
        if rest.contains("client.jar") {
            return true;
        }
        search = rest;
    }
    false
}

fn classify(lowercase: &str) -> Option<&'static str> {
    if lowercase.contains("lunarclient") {
        Some("Lunar Client")
    } else if lowercase.contains("org.prismlauncher.entrypoint") {
        Some("Prism Launcher")
    } else if lowercase.contains("neoforgeclient") {
        Some("NeoForge")
    } else if lowercase.contains("forgeclient") {
        Some("Forge")
    } else if lowercase.contains("knotclient") {
        Some("Fabric")
    } else if lowercase.contains("clientlaunchhandler") {
        Some("Modded")
    } else if lowercase.contains("net.minecraft.client") {
        Some("Minecraft")
    } else if lowercase.contains("newlaunch.jar") {
        Some("Legacy launcher")
    } else if matches_minecraft_client_jar(lowercase) {
        Some("Minecraft")
    } else if lowercase.contains("--gamedir") || lowercase.contains("--assetsdir") {
        Some("Minecraft")
    } else {
        None
    }
}

fn read_cmdline(pid: u32) -> Option<String> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    if raw.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    for chunk in raw.split(|byte| *byte == 0) {
        if chunk.is_empty() {
            continue;
        }
        parts.push(String::from_utf8_lossy(chunk).to_string());
    }
    Some(parts.join(" "))
}

fn read_uid(pid: u32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            return rest.split_whitespace().nth(1)?.parse().ok();
        }
    }
    None
}

/// Everything the shell script verifies before it touches a PID.
pub fn inspect(pid: u32, owner_uid: u32) -> Result<Target, String> {
    if pid <= 1 {
        return Err("Refusing an invalid target PID".to_string());
    }
    let status_path = format!("/proc/{pid}/status");
    if !Path::new(&status_path).exists() {
        return Err(format!("PID {pid} does not exist"));
    }
    let uid = read_uid(pid).ok_or_else(|| format!("Cannot read the owner of PID {pid}"))?;
    if uid != owner_uid {
        return Err(format!("Refusing a JVM owned by another user (UID {uid})"));
    }
    let exe = std::fs::read_link(format!("/proc/{pid}/exe"))
        .map_err(|_| format!("Cannot resolve the executable of PID {pid}"))?;
    let exe = exe.canonicalize().unwrap_or(exe);
    if exe.file_name().map(|n| n != "java").unwrap_or(true) {
        return Err(format!("Refusing non-Java target: {}", exe.display()));
    }

    let cmdline =
        read_cmdline(pid).ok_or_else(|| format!("Cannot read the command line of PID {pid}"))?;
    let lower = cmdline.to_lowercase();
    let kind = classify(&lower).unwrap_or("Java process");
    let attach_disabled = lower.contains("-xx:+disableattachmechanism");

    Ok(Target {
        pid,
        exe,
        kind,
        attach_disabled,
    })
}

/// Scans `/proc` for every Minecraft JVM owned by `owner_uid`.
pub fn scan(owner_uid: u32) -> Vec<Target> {
    let mut found: Vec<Target> = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return found;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };

        if read_uid(pid) != Some(owner_uid) {
            continue;
        }
        let exe = match std::fs::read_link(format!("/proc/{pid}/exe")) {
            Ok(path) => path,
            Err(_) => continue,
        };
        if exe.file_name().map(|n| n != "java").unwrap_or(true) {
            continue;
        }
        let Some(cmdline) = read_cmdline(pid) else {
            continue;
        };
        let lower = cmdline.to_lowercase();
        let is_target = MARKERS.iter().any(|marker| lower.contains(marker))
            || matches_minecraft_client_jar(&lower);
        if !is_target {
            continue;
        }

        found.push(Target {
            pid,
            exe,
            kind: classify(&lower).unwrap_or("Minecraft"),
            attach_disabled: lower.contains("-xx:+disableattachmechanism"),
        });
    }

    found.sort_by_key(|target| target.pid);
    found
}

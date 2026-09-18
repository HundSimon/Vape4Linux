//! Headless mode: `--list` and `--inject <pid>`.
//!
//! It drives exactly the same code as the window does, which keeps the two
//! entry points honest and makes the injection path scriptable.

use std::time::{Duration, Instant};

use crate::inject::{self, Artifacts, InjectMode};
use crate::runtime::{LogBuffer, LogKind};
use crate::targets::{self, Target};
use crate::util;

const HELP: &str = "\
OpenVape — native Linux launcher for the Vape 4.21 agent

USAGE:
    openvape                 start the graphical client
    openvape --list          list the Minecraft JVMs this user owns
    openvape --inject <pid>  inject into a PID (add --force for ptrace)

OPTIONS:
    -l, --list        list candidate Minecraft processes
    -i, --inject PID  inject into the given PID
    -f, --force       use the native ptrace injector instead of Attach
    -q, --no-wait     return as soon as the injector exits
        --provision-service [FILE]
                      create the account for the loader-stub access token in
                      the service data file (defaults to the GUI's store)
    -h, --help        show this message
";

/// Returns `Some(exit_code)` when the command line was handled headlessly.
pub fn dispatch(args: &[String]) -> Option<i32> {
    if args.is_empty() {
        return None;
    }

    let mut list = false;
    let mut inject_pid: Option<u32> = None;
    let mut force = false;
    let mut wait = true;
    let mut provision: Option<Option<String>> = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return Some(0);
            }
            "-l" | "--list" => list = true,
            "-f" | "--force" => force = true,
            "-q" | "--no-wait" => wait = false,
            "--provision-service" => {
                // The path is optional: default to the store the GUI manages.
                let next = args.get(index + 1).filter(|value| !value.starts_with('-'));
                if next.is_some() {
                    index += 1;
                }
                provision = Some(next.cloned());
            }
            "-i" | "--inject" => {
                index += 1;
                match args.get(index).and_then(|value| value.parse().ok()) {
                    Some(pid) => inject_pid = Some(pid),
                    None => {
                        eprintln!("openvape: --inject needs a numeric PID");
                        return Some(2);
                    }
                }
            }
            other => {
                eprintln!("openvape: unknown argument '{other}'");
                eprint!("{HELP}");
                return Some(2);
            }
        }
        index += 1;
    }

    if let Some(path) = provision {
        let data_file = path
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| util::service_state_dir().join("vape-service.json"));
        return Some(provision_service(&data_file));
    }
    if let Some(pid) = inject_pid {
        return Some(inject(pid, force, wait));
    }
    if list {
        return Some(list_targets());
    }

    eprint!("{HELP}");
    Some(2)
}

/// Repairs the service store so the injected payload can authenticate.
fn provision_service(data_file: &std::path::Path) -> i32 {
    let log = LogBuffer::new();
    match crate::service::provision_bridge_account(data_file, &log) {
        Ok(true) => {
            println!(
                "Provisioned access token \"{}\" in {}",
                crate::service::BRIDGE_TOKEN,
                data_file.display()
            );
            println!("Restart the service for the change to take effect.");
            0
        }
        Ok(false) => {
            println!(
                "{} already has an account for access token \"{}\"",
                data_file.display(),
                crate::service::BRIDGE_TOKEN
            );
            0
        }
        Err(error) => {
            eprintln!("openvape: {error}");
            1
        }
    }
}

fn owner_uid() -> u32 {
    util::target_owner_uid()
}

fn list_targets() -> i32 {
    let found = targets::scan(owner_uid());
    if found.is_empty() {
        println!("No Minecraft JVM found for uid {}.", owner_uid());
        return 1;
    }
    println!(
        "{:<10} {:<16} {:<9} {}",
        "PID", "KIND", "ATTACH", "EXECUTABLE"
    );
    for target in &found {
        println!(
            "{:<10} {:<16} {:<9} {}",
            target.pid,
            target.kind,
            if target.attach_disabled {
                "disabled"
            } else {
                "ok"
            },
            target.exe.display()
        );
    }
    0
}

fn inject(pid: u32, force: bool, wait: bool) -> i32 {
    let mode = if force {
        InjectMode::Force
    } else {
        InjectMode::Attach
    };

    let target: Target = match targets::inspect(pid, owner_uid()) {
        Ok(target) => target,
        Err(error) => {
            eprintln!("openvape: {error}");
            return 2;
        }
    };
    println!(
        "Target: PID {} ({}) · {}",
        target.pid,
        target.kind,
        target.exe.display()
    );

    let artifacts: Artifacts = match inject::find_artifacts(util::repo_root().as_deref()) {
        Some(artifacts) => artifacts,
        None => {
            eprintln!("openvape: the injection bundle was not found (build/injection)");
            return 2;
        }
    };
    let java = match util::discover_java(17, None) {
        Some(java) => java,
        None => {
            eprintln!("openvape: no Java 17+ runtime found");
            return 2;
        }
    };

    let log = LogBuffer::new();
    let job = match inject::launch(&target, &artifacts, mode, &java, owner_uid(), &log) {
        Ok(job) => job,
        Err(error) => {
            flush(&log, &mut 0);
            eprintln!("openvape: {error}");
            return 1;
        }
    };

    let mut job = job;
    let mut printed = 0usize;
    let started = Instant::now();
    let _finished = loop {
        flush(&log, &mut printed);
        if let Some(done) = job.poll() {
            break done;
        }
        if started.elapsed() > Duration::from_secs(180) {
            job.terminate();
            eprintln!("openvape: the injector timed out");
            return 1;
        }
        std::thread::sleep(Duration::from_millis(80));
    };
    std::thread::sleep(Duration::from_millis(120));
    flush(&log, &mut printed);

    let outcome = inject::interpret(mode, &job);
    match outcome {
        Ok(message) => println!("{message}"),
        Err(error) => {
            eprintln!("openvape: {error}");
            return 1;
        }
    }

    if !wait {
        return 0;
    }

    println!("Waiting for the payload to confirm (up to 95s)…");
    let (tx, rx) = std::sync::mpsc::channel();
    let cancel = crate::runtime::CancelToken::new();
    inject::watch_bootstrap(target.pid, &artifacts, tx, cancel);
    match rx.recv_timeout(Duration::from_secs(100)) {
        Ok(crate::runtime::Msg::Bootstrap(Ok(_))) => {
            println!("Verified: the agent is active in PID {}", target.pid);
            0
        }
        Ok(crate::runtime::Msg::Bootstrap(Err(reason))) => {
            eprintln!("openvape: {reason}");
            if let Some(tail) = inject::bootstrap_tail(target.pid, &artifacts, 8) {
                eprintln!("--- native bootstrap log ---\n{tail}");
            }
            1
        }
        Ok(_) => {
            eprintln!("openvape: unexpected verification message");
            1
        }
        Err(_) => {
            eprintln!("openvape: no confirmation was received");
            1
        }
    }
}

/// Prints log lines the UI would have shown, so CLI output matches the window.
fn flush(log: &LogBuffer, printed: &mut usize) {
    let lines = log.snapshot();
    if lines.len() <= *printed {
        return;
    }
    for line in &lines[*printed..] {
        let prefix = match line.kind {
            LogKind::Error => "!",
            LogKind::Warn => "!",
            LogKind::Good => "+",
            _ => " ",
        };
        println!("{prefix} [{}] {}", line.tag, line.text);
    }
    *printed = lines.len();
}

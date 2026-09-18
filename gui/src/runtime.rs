//! Background job plumbing: a shared log ring buffer and child process handles.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::util::strip_ansi;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogKind {
    Info,
    Good,
    Warn,
    Error,
    Dim,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub tag: String,
    pub text: String,
    pub kind: LogKind,
}

const LOG_CAPACITY: usize = 900;

/// Shared, thread-safe activity log.
#[derive(Clone, Default)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<LogLine>>>,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, tag: &str, kind: LogKind, text: impl AsRef<str>) {
        let text = strip_ansi(text.as_ref());
        let text = text.trim_end();
        if text.trim().is_empty() {
            return;
        }
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        for line in text.lines() {
            guard.push_back(LogLine {
                tag: tag.to_string(),
                text: line.trim_end().to_string(),
                kind,
            });
        }
        while guard.len() > LOG_CAPACITY {
            guard.pop_front();
        }
    }

    pub fn snapshot(&self) -> Vec<LogLine> {
        self.inner
            .lock()
            .map(|guard| guard.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.clear();
        }
    }
}

/// Messages sent from worker threads back to the UI thread.
pub enum Msg {
    Health {
        http: bool,
        zeus: bool,
    },
    ServiceBuilt(Result<std::path::PathBuf, String>),
    BundleBuilt(Result<(), String>),
    Bootstrap(Result<String, String>),
    /// Result of probing the service with the payload's access token.
    BridgeAccount(Result<(), String>),
}

/// Handle for cancelling a blocking worker thread.
#[derive(Clone, Default)]
pub struct CancelToken {
    stop: StopFlag,
    pid: Arc<Mutex<Option<u32>>>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self {
            stop: StopFlag::new(),
            pid: Arc::new(Mutex::new(None)),
        }
    }

    pub fn stopped(&self) -> bool {
        self.stop.stopped()
    }

    /// Terminates the running child, if any.
    pub fn cancel(&self) {
        self.stop.stop();
        if let Ok(guard) = self.pid.lock() {
            if let Some(pid) = *guard {
                let _ = Command::new("kill").arg(pid.to_string()).status();
            }
        }
    }
}

/// Runs a command to completion, streaming its output into the log.
///
/// Used by worker threads that do not need the UI to poll them.
pub fn run_streamed(
    command: &mut Command,
    tag: &str,
    log: &LogBuffer,
    cancel: &CancelToken,
) -> std::io::Result<std::process::ExitStatus> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .env("TERM", "dumb");

    let mut child = command.spawn()?;
    if let Ok(mut guard) = cancel.pid.lock() {
        *guard = Some(child.id());
    }
    let sink: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    if let Some(stdout) = child.stdout.take() {
        pump(stdout, tag, log.clone(), sink.clone(), LogKind::Info);
    }
    if let Some(stderr) = child.stderr.take() {
        pump(stderr, tag, log.clone(), sink, LogKind::Dim);
    }
    let status = child.wait();
    if let Ok(mut guard) = cancel.pid.lock() {
        *guard = None;
    }
    status
}

/// A long-lived child process whose stdout/stderr stream into the log.
pub struct Job {
    pub child: Child,
    pub pid: u32,
    pub started: Instant,
    pub tag: String,
    lines: Arc<Mutex<Vec<String>>>,
    finished: Option<Finished>,
}

#[derive(Clone, Debug)]
pub struct Finished {
    pub code: Option<i32>,
    pub success: bool,
    pub duration: Duration,
}

impl Job {
    #[allow(dead_code)]
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// `die_with_parent` asks the kernel to signal the child when this process
    /// exits, so a long running service can never be orphaned by the GUI.
    pub fn spawn(
        tag: &str,
        command: &mut Command,
        log: &LogBuffer,
        env: &[(&str, String)],
        die_with_parent: bool,
    ) -> std::io::Result<Self> {
        for (key, value) in env {
            command.env(key, value);
        }
        if die_with_parent {
            // Safety: only async-signal-safe calls run in the child.
            unsafe {
                use std::os::unix::process::CommandExt;
                command.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("NO_COLOR", "1")
            .env("TERM", "dumb");

        let mut child = command.spawn()?;
        let pid = child.id();
        let lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        if let Some(stdout) = child.stdout.take() {
            pump(stdout, tag, log.clone(), lines.clone(), LogKind::Info);
        }
        if let Some(stderr) = child.stderr.take() {
            pump(stderr, tag, log.clone(), lines.clone(), LogKind::Dim);
        }

        Ok(Self {
            child,
            pid,
            started: Instant::now(),
            tag: tag.to_string(),
            lines,
            finished: None,
        })
    }

    /// Non-blocking completion check.
    pub fn poll(&mut self) -> Option<Finished> {
        if let Some(done) = &self.finished {
            return Some(done.clone());
        }
        match self.child.try_wait() {
            Ok(Some(status)) => {
                let done = Finished {
                    code: status.code(),
                    success: status.success(),
                    duration: self.started.elapsed(),
                };
                self.finished = Some(done.clone());
                Some(done)
            }
            Ok(None) => None,
            Err(_) => {
                let done = Finished {
                    code: None,
                    success: false,
                    duration: self.started.elapsed(),
                };
                self.finished = Some(done.clone());
                Some(done)
            }
        }
    }

    /// Ask the process to stop, escalating to a kill after a grace period.
    pub fn terminate(&mut self) {
        if self.poll().is_some() {
            return;
        }
        let pid = self.pid.to_string();
        let _ = Command::new("kill").arg(&pid).status();
        let deadline = Instant::now() + Duration::from_millis(1200);
        while Instant::now() < deadline {
            if self.poll().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = self.poll();
    }

    pub fn stdout_contains(&self, needle: &str) -> bool {
        self.lines
            .lock()
            .map(|lines| lines.iter().any(|line| line.contains(needle)))
            .unwrap_or(false)
    }

    pub fn exit_code(&self) -> Option<i32> {
        self.finished.as_ref().and_then(|done| done.code)
    }
}

fn pump<R: std::io::Read + Send + 'static>(
    stream: R,
    tag: &str,
    log: LogBuffer,
    lines: Arc<Mutex<Vec<String>>>,
    kind: LogKind,
) {
    let tag = tag.to_string();
    std::thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let clean = strip_ansi(&line);
            let clean = clean.trim_end();
            if clean.is_empty() {
                continue;
            }
            if let Ok(mut guard) = lines.lock() {
                guard.push(clean.to_string());
                if guard.len() > 4000 {
                    guard.remove(0);
                }
            }
            log.push(&tag, kind, clean);
        }
    });
}

/// Cooperative stop flag shared with the health prober thread.
#[derive(Clone)]
pub struct StopFlag(Arc<AtomicBool>);

impl StopFlag {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
    pub fn stop(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn stopped(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

impl Default for StopFlag {
    fn default() -> Self {
        Self::new()
    }
}

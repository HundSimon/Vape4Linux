//! The single-window interface: service control, target selection and injection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align2, Color32, CornerRadius, FontFamily, FontId, Margin, Pos2, Rect, RichText, Sense, Stroke,
    Ui, Vec2,
};

use crate::inject::{self, Artifacts, InjectMode};
use crate::runtime::{CancelToken, Finished, Job, LogBuffer, LogKind, Msg, StopFlag};
use crate::service::{self, ServiceConfig, ServiceState};
use crate::targets::{self, Target};
use crate::theme::{label_font, medium, mix, mono_font, with_alpha, Pal};
use crate::ui;
use crate::util::{self, JavaRuntime};

const INJECT_LABEL: &str = "Inject";
const BODY_FONT: f32 = 11.5;
const TITLE_FONT: f32 = 12.5;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub java_path: Option<String>,
    pub force_attach: bool,
    pub auto_build: bool,
    pub show_log: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            java_path: None,
            force_attach: false,
            auto_build: true,
            show_log: true,
        }
    }
}

impl Config {
    fn load() -> Self {
        std::fs::read_to_string(util::config_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let path = util::config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Injecting,
    Loaded,
    Verified,
    Failed,
}

impl Phase {
    fn progress(self) -> f32 {
        match self {
            Self::Injecting => 0.35,
            Self::Loaded => 0.72,
            Self::Verified | Self::Failed => 1.0,
        }
    }

    fn progress_label(self) -> &'static str {
        match self {
            Self::Injecting => "Starting injector",
            Self::Loaded => "Agent loaded · waiting for game",
            Self::Verified => "Injection verified",
            Self::Failed => "Injection failed",
        }
    }

    fn is_active(self) -> bool {
        matches!(self, Self::Injecting | Self::Loaded)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Inject,
    Logs,
    Settings,
}

#[derive(Clone, Debug)]
struct Outcome {
    phase: Phase,
    #[allow(dead_code)]
    message: String,
}

pub struct App {
    page: Page,

    // Environment
    repo_root: Option<PathBuf>,
    artifacts: Option<Artifacts>,
    service_jar: Option<PathBuf>,
    service_java: Option<JavaRuntime>,
    gradle_jdk: Option<JavaRuntime>,
    owner_uid: u32,

    config: Config,

    // Service
    service_state: ServiceState,
    service_job: Option<Job>,
    service_healthy: bool,
    zeus_open: bool,
    health_misses: u32,
    probe_active: Arc<AtomicBool>,
    probe_stop: StopFlag,
    /// Set right before a service start; the prober then verifies that the
    /// service accepts the token the injected payload presents.
    verify_bridge_account: Arc<AtomicBool>,

    // Builds
    service_building: bool,
    service_build_cancel: Option<CancelToken>,
    bundle_building: bool,
    bundle_build_cancel: Option<CancelToken>,

    // Targets
    target_list: Vec<Target>,
    selected: Option<u32>,
    outcomes: HashMap<u32, Outcome>,
    scan_at: Instant,

    // Injection
    inject_job: Option<Job>,
    inject_target: Option<u32>,
    inject_mode: InjectMode,
    bootstrap_cancel: Option<CancelToken>,
    status: Option<(LogKind, String)>,

    // Diagnostics
    log: LogBuffer,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,

    time: f64,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let boot = Instant::now();
        let mark = |what: &str| {
            if std::env::var_os("OPENVAPE_DEBUG").is_some() {
                eprintln!("[boot] {what}: {:?}", boot.elapsed());
            }
        };

        crate::theme::install(&cc.egui_ctx);
        mark("theme");

        let config = Config::load();
        let repo_root = util::repo_root();
        let artifacts = inject::find_artifacts(repo_root.as_deref());
        let service_jar = util::find_service_jar(repo_root.as_deref());
        let service_java = config
            .java_path
            .as_deref()
            .map(PathBuf::from)
            .and_then(|path| util::probe_java_path(&path))
            .or_else(|| util::discover_java(17, None));
        mark("artifacts");
        let gradle_jdk = util::discover_java(17, Some(23));
        mark("gradle jdk");

        let (tx, rx) = channel();
        let log = LogBuffer::new();
        let force_attach = config.force_attach;

        let mut app = Self {
            page: Page::Inject,
            repo_root,
            artifacts,
            service_jar,
            service_java,
            gradle_jdk,
            owner_uid: util::target_owner_uid(),
            config,
            service_state: ServiceState::Missing,
            service_job: None,
            service_healthy: false,
            zeus_open: false,
            health_misses: 0,
            probe_active: Arc::new(AtomicBool::new(true)),
            probe_stop: StopFlag::new(),
            verify_bridge_account: Arc::new(AtomicBool::new(true)),
            service_building: false,
            service_build_cancel: None,
            bundle_building: false,
            bundle_build_cancel: None,
            target_list: Vec::new(),
            selected: None,
            outcomes: HashMap::new(),
            scan_at: Instant::now() - Duration::from_secs(10),
            inject_job: None,
            inject_target: None,
            inject_mode: if force_attach {
                InjectMode::Force
            } else {
                InjectMode::Attach
            },
            bootstrap_cancel: None,
            status: None,
            log,
            tx,
            rx,
            time: 0.0,
        };

        app.log.push(
            "app",
            LogKind::Dim,
            format!(
                "OpenVape {} · uid {} · {}",
                env!("CARGO_PKG_VERSION"),
                app.owner_uid,
                app.repo_root
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "checkout not detected".to_string())
            ),
        );
        if app.service_java.is_none() {
            app.log.push(
                "app",
                LogKind::Error,
                "No Java 17+ runtime found: the service and the attach injector cannot run",
            );
        }

        app.spawn_prober();
        app.bootstrap_service();
        mark("service");
        app.scan();
        mark("scan");
        app
    }

    // ---------------------------------------------------------------- service

    fn bootstrap_service(&mut self) {
        if let Some(jar) = self.service_jar.clone() {
            self.start_service(jar);
        } else if self.config.auto_build {
            self.start_service_build();
        } else {
            self.service_state = ServiceState::Missing;
        }
    }

    fn start_service(&mut self, jar: PathBuf) {
        let Some(java) = self.service_java.clone() else {
            self.service_state = ServiceState::Failed("No Java runtime available".to_string());
            return;
        };

        // Never fight another process for the same ports.
        if service::http_healthy(service::DEFAULT_HTTP_PORT) {
            self.service_state = ServiceState::External;
            self.service_jar = Some(jar);
            self.log.push(
                "service",
                LogKind::Good,
                "Reusing the service already listening on port 8080",
            );
            return;
        }

        let data_file = util::service_state_dir().join("vape-service.json");

        // The injected payload authenticates with the stub loader token, which
        // no VapeService build creates an account for. Provision it while the
        // service is stopped, otherwise the client shows
        // "Error establishing connection".
        match service::provision_bridge_account(&data_file, &self.log) {
            Ok(_) => {}
            Err(error) => self.log.push(
                "service",
                LogKind::Error,
                format!("Could not provision the bridge account: {error}"),
            ),
        }
        self.verify_bridge_account.store(true, Ordering::Relaxed);

        let config = ServiceConfig {
            java,
            jar: jar.clone(),
            http_port: service::DEFAULT_HTTP_PORT,
            zeus_port: service::DEFAULT_ZEUS_PORT,
            data_file,
        };
        match service::launch(&config, &self.log) {
            Ok(job) => {
                self.log.push(
                    "service",
                    LogKind::Info,
                    format!("Started the local service (pid {})", job.pid),
                );
                self.service_job = Some(job);
                self.service_jar = Some(jar);
                self.service_state = ServiceState::Starting;
                self.service_healthy = false;
                self.health_misses = 0;
            }
            Err(error) => {
                let message = error.to_string();
                self.log.push("service", LogKind::Error, message.clone());
                self.service_state = ServiceState::Failed(message);
            }
        }
    }

    fn stop_service(&mut self) {
        if let Some(job) = self.service_job.as_mut() {
            job.terminate();
        }
        self.service_job = None;
        self.service_healthy = false;
        self.service_state = if service::http_healthy(service::DEFAULT_HTTP_PORT) {
            ServiceState::External
        } else {
            ServiceState::Stopped
        };
        self.verify_bridge_account.store(true, Ordering::Relaxed);
        self.log
            .push("service", LogKind::Warn, "Stopped the local service");
    }

    fn start_service_build(&mut self) {
        if self.service_building {
            return;
        }
        let Some(jdk) = self.gradle_jdk.clone() else {
            self.service_state = ServiceState::Failed(
                "A JDK between 17 and 22 is required to build the service".to_string(),
            );
            self.log.push(
                "build",
                LogKind::Error,
                "No usable JDK found; install openjdk-17 or set JAVA_HOME",
            );
            return;
        };

        let dir = util::service_source_dir();
        let log = self.log.clone();
        let tx = self.tx.clone();
        let cancel = CancelToken::new();
        self.service_build_cancel = Some(cancel.clone());
        self.service_building = true;
        self.service_state = ServiceState::Building("cloning the service repository…".to_string());
        self.config.show_log = true;
        self.log.push(
            "build",
            LogKind::Info,
            "Preparing the service (clone + Gradle build)",
        );

        std::thread::spawn(move || {
            let result = service::ensure_source(&dir, &log, &cancel)
                .and_then(|_| service::build_source(&dir, &jdk, &log, &cancel));
            let _ = tx.send(Msg::ServiceBuilt(result));
        });
    }

    fn start_bundle_build(&mut self) {
        if self.bundle_building {
            return;
        }
        let Some(root) = self.repo_root.clone() else {
            self.status = Some((
                LogKind::Error,
                "The Vape4Linux checkout was not found; set OPENVAPE_ROOT".to_string(),
            ));
            return;
        };
        let Some(jdk) = self.gradle_jdk.clone() else {
            self.status = Some((
                LogKind::Error,
                "A JDK between 17 and 22 is required to rebuild the bundle".to_string(),
            ));
            return;
        };

        let log = self.log.clone();
        let tx = self.tx.clone();
        let cancel = CancelToken::new();
        self.bundle_build_cancel = Some(cancel.clone());
        self.bundle_building = true;
        self.config.show_log = true;

        std::thread::spawn(move || {
            let result = service::build_injection_bundle(&root, &jdk, &log, &cancel);
            let _ = tx.send(Msg::BundleBuilt(result));
        });
    }

    fn spawn_prober(&self) {
        let tx = self.tx.clone();
        let active = self.probe_active.clone();
        let stop = self.probe_stop.clone();
        let verify = self.verify_bridge_account.clone();
        std::thread::spawn(move || {
            while !stop.stopped() {
                if active.load(Ordering::Relaxed) {
                    let http = service::http_healthy(service::DEFAULT_HTTP_PORT);
                    let zeus = service::port_open(service::DEFAULT_ZEUS_PORT);
                    if tx.send(Msg::Health { http, zeus }).is_err() {
                        return;
                    }
                    if http && verify.swap(false, Ordering::Relaxed) {
                        let result = service::check_bridge_account(service::DEFAULT_HTTP_PORT);
                        if tx.send(Msg::BridgeAccount(result)).is_err() {
                            return;
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(1400));
            }
        });
    }

    // ---------------------------------------------------------------- targets

    fn scan(&mut self) {
        self.scan_at = Instant::now();
        let previous = self.selected;
        let list = targets::scan(self.owner_uid);
        if let Some(pid) = previous {
            if !list.iter().any(|target| target.pid == pid) {
                self.selected = None;
            }
        }
        if self.selected.is_none() && list.len() == 1 {
            self.selected = Some(list[0].pid);
        }
        self.outcomes
            .retain(|pid, _| list.iter().any(|target| target.pid == *pid));
        self.target_list = list;
    }

    fn selected_target(&self) -> Option<&Target> {
        let pid = self.selected?;
        self.target_list.iter().find(|target| target.pid == pid)
    }

    fn injection_outcome(&self) -> Option<&Outcome> {
        self.outcomes.get(&self.inject_target?)
    }

    // ------------------------------------------------------------- injection

    fn start_injection(&mut self) {
        let Some(pid) = self.selected else {
            self.status = Some((
                LogKind::Error,
                "Select a Minecraft process first".to_string(),
            ));
            return;
        };
        // Re-check the target exactly like `tools/inject.sh` does: the process
        // may have exited, changed owner, or been replaced since the last scan.
        let target: Target = match targets::inspect(pid, self.owner_uid) {
            Ok(target) => target,
            Err(error) => {
                self.log.push("inject", LogKind::Error, error.clone());
                self.status = Some((LogKind::Error, error));
                self.scan();
                return;
            }
        };
        let Some(artifacts) = self.artifacts.clone() else {
            self.status = Some((
                LogKind::Error,
                "The injection bundle is missing; rebuild it first".to_string(),
            ));
            return;
        };
        let Some(java) = self.service_java.clone() else {
            self.status = Some((LogKind::Error, "No Java 17+ runtime found".to_string()));
            return;
        };

        let mode = self.inject_mode;
        self.config.show_log = true;
        match inject::launch(&target, &artifacts, mode, &java, self.owner_uid, &self.log) {
            Ok(job) => {
                self.inject_target = Some(target.pid);
                self.inject_job = Some(job);
                self.outcomes.insert(
                    target.pid,
                    Outcome {
                        phase: Phase::Injecting,
                        message: format!("Injecting into PID {}", target.pid),
                    },
                );
                self.status = Some((
                    LogKind::Info,
                    format!("{} → PID {} ({})", mode.label(), target.pid, target.kind),
                ));
            }
            Err(error) => {
                self.inject_target = Some(target.pid);
                self.outcomes.insert(
                    target.pid,
                    Outcome {
                        phase: Phase::Failed,
                        message: error.clone(),
                    },
                );
                self.log.push("inject", LogKind::Error, error.clone());
                self.status = Some((LogKind::Error, error));
            }
        }
    }

    fn watch_bootstrap(&mut self, pid: u32) {
        let Some(artifacts) = self.artifacts.clone() else {
            return;
        };
        if let Some(cancel) = self.bootstrap_cancel.take() {
            cancel.cancel();
        }
        let cancel = CancelToken::new();
        self.bootstrap_cancel = Some(cancel.clone());
        inject::watch_bootstrap(pid, &artifacts, self.tx.clone(), cancel);
    }

    // -------------------------------------------------------------- messaging

    fn pump(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Msg::Health { http, zeus } => {
                    self.zeus_open = zeus;
                    if http {
                        self.health_misses = 0;
                        self.service_healthy = true;
                        match self.service_state {
                            ServiceState::Starting => {
                                self.service_state = ServiceState::Running;
                                self.log.push(
                                    "service",
                                    LogKind::Good,
                                    "The service answers on 127.0.0.1:8080",
                                );
                            }
                            ServiceState::Stopped | ServiceState::Missing => {
                                if self.service_job.is_none() {
                                    self.service_state = ServiceState::External;
                                }
                            }
                            _ => {}
                        }
                    } else {
                        self.health_misses += 1;
                        if self.health_misses >= 3 {
                            self.service_healthy = false;
                            if self.service_job.is_none() {
                                if matches!(self.service_state, ServiceState::External) {
                                    self.service_state = ServiceState::Stopped;
                                }
                            } else if matches!(self.service_state, ServiceState::Running) {
                                self.service_state = ServiceState::Starting;
                            }
                        }
                    }
                }
                Msg::BridgeAccount(result) => match result {
                    Ok(()) => self.log.push(
                        "service",
                        LogKind::Good,
                        format!(
                            "Access token \"{}\" authenticates against the service",
                            service::BRIDGE_TOKEN
                        ),
                    ),
                    Err(reason) => {
                        self.log.push(
                            "service",
                            LogKind::Error,
                            format!(
                                "The service rejected the payload's access token: {reason}. \
                                 The in-game Online page will show a connection error."
                            ),
                        );
                        self.status =
                            Some((LogKind::Error, format!("Service auth failed: {reason}")));
                    }
                },
                Msg::ServiceBuilt(result) => {
                    self.service_building = false;
                    self.service_build_cancel = None;
                    match result {
                        Ok(jar) => {
                            if let Some(root) = &self.repo_root {
                                mirror_service_jar(&jar, &root.join("build/service"));
                            }
                            self.service_jar = Some(jar.clone());
                            self.start_service(jar);
                        }
                        Err(error) => {
                            self.config.show_log = true;
                            self.service_state = ServiceState::Failed(error);
                        }
                    }
                }
                Msg::BundleBuilt(result) => {
                    self.bundle_building = false;
                    self.bundle_build_cancel = None;
                    match result {
                        Ok(()) => {
                            self.artifacts = inject::find_artifacts(self.repo_root.as_deref());
                            self.status =
                                Some((LogKind::Good, "Injection bundle rebuilt".to_string()));
                        }
                        Err(error) => self.status = Some((LogKind::Error, error)),
                    }
                }
                Msg::Bootstrap(result) => {
                    let Some(pid) = self.inject_target else {
                        continue;
                    };
                    match result {
                        Ok(_) => {
                            if let Some(outcome) = self.outcomes.get_mut(&pid) {
                                outcome.phase = Phase::Verified;
                                outcome.message = "Agent active in the game".to_string();
                            }
                            self.log.push(
                                "inject",
                                LogKind::Good,
                                format!("Bootstrap confirmed for PID {pid}"),
                            );
                            self.status = Some((
                                LogKind::Good,
                                format!("Agent verified in PID {pid} — OpenVape is live"),
                            ));
                        }
                        Err(reason) => {
                            if let Some(outcome) = self.outcomes.get_mut(&pid) {
                                outcome.phase = Phase::Failed;
                                outcome.message = reason.clone();
                            }
                            self.log.push("inject", LogKind::Warn, reason.clone());
                            if let Some(artifacts) = &self.artifacts {
                                if let Some(tail) = inject::bootstrap_tail(pid, artifacts, 6) {
                                    self.log.push("bootstrap", LogKind::Dim, tail);
                                }
                            }
                            self.status = Some((LogKind::Warn, reason));
                        }
                    }
                }
            }
        }
    }

    fn poll_jobs(&mut self) {
        // Long-lived service process
        let mut finished: Option<Finished> = None;
        if let Some(job) = self.service_job.as_mut() {
            finished = job.poll();
        }
        if let Some(done) = finished {
            self.service_job = None;
            self.service_healthy = false;
            if service::http_healthy(service::DEFAULT_HTTP_PORT) {
                self.service_state = ServiceState::External;
            } else if done.success {
                self.log
                    .push("service", LogKind::Warn, "The service exited");
                self.service_state = ServiceState::Stopped;
            } else {
                self.service_state = ServiceState::Failed(format!(
                    "The service exited with status {}",
                    done.code
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "?".to_string())
                ));
            }
        }

        // Injector process
        let mut finished: Option<Finished> = None;
        if let Some(job) = self.inject_job.as_mut() {
            finished = job.poll();
        }
        if let Some(done) = finished {
            let interpretation = self
                .inject_job
                .as_ref()
                .map(|job| inject::interpret(self.inject_mode, job))
                .unwrap_or_else(|| Err("The injector disappeared".to_string()));
            self.inject_job = None;
            let pid = self.inject_target;
            self.log.push(
                "inject",
                LogKind::Dim,
                format!(
                    "The injector finished in {:.1}s",
                    done.duration.as_secs_f32()
                ),
            );

            match interpretation {
                Ok(message) => {
                    if let Some(pid) = pid {
                        if let Some(outcome) = self.outcomes.get_mut(&pid) {
                            outcome.phase = Phase::Loaded;
                            outcome.message = "Loaded, waiting for the game".to_string();
                        }
                    }
                    self.log.push("inject", LogKind::Good, message.clone());
                    self.status = Some((
                        LogKind::Good,
                        format!("{message} — waiting for the payload to confirm"),
                    ));
                    if let Some(pid) = pid {
                        self.watch_bootstrap(pid);
                    }
                }
                Err(error) => {
                    if let Some(pid) = pid {
                        if let Some(outcome) = self.outcomes.get_mut(&pid) {
                            outcome.phase = Phase::Failed;
                            outcome.message = error.clone();
                        }
                    }
                    self.log.push("inject", LogKind::Error, error.clone());
                    self.status = Some((LogKind::Error, error));
                }
            }
        }
    }

    fn tick(&mut self) {
        let interval = if self.inject_job.is_some() {
            Duration::from_millis(1500)
        } else {
            Duration::from_millis(2000)
        };
        if self.scan_at.elapsed() >= interval {
            self.scan();
        }
    }

    fn shutdown(&mut self) {
        self.probe_stop.stop();
        if let Some(cancel) = self.bootstrap_cancel.take() {
            cancel.cancel();
        }
        if let Some(job) = self.service_job.as_mut() {
            job.terminate();
        }
        self.service_job = None;
    }

    // ------------------------------------------------------------------ paint

    fn header(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let height = 44.0;
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.max_rect().width(), height), Sense::hover());

        let close = Rect::from_center_size(
            Pos2::new(rect.right() - 24.0, rect.center().y),
            Vec2::splat(28.0),
        );
        let minimise = close.translate(Vec2::new(-32.0, 0.0));

        let close_response = ui.interact(close, egui::Id::new("window-close"), Sense::click());
        let minimise_response =
            ui.interact(minimise, egui::Id::new("window-minimise"), Sense::click());
        if close_response.clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if minimise_response.clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        paint_square_icon(ui, &close_response, ui::Icon::Close, true);
        paint_square_icon(ui, &minimise_response, ui::Icon::Minimize, false);

        // Dragging anywhere else in the bar moves the window.
        let drag_rect = Rect::from_min_max(rect.min, Pos2::new(minimise.left() - 10.0, rect.max.y));
        let drag = ui.interact(
            drag_rect,
            egui::Id::new("window-drag"),
            Sense::click_and_drag(),
        );
        if drag.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
    }

    fn body(&mut self, ui: &mut Ui) {
        match self.page {
            Page::Inject => {
                let time = self.time;
                self.targets_section(ui, time);
                if self.selected_target().is_some() {
                    ui.add_space(14.0);
                    self.options_section(ui);
                    ui.add_space(10.0);
                    self.inject_section(ui, time);
                }
            }
            Page::Logs => self.logs_page(ui),
            Page::Settings => self.settings_page(ui),
        }
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        ui.set_width(172.0);
        ui.set_max_width(172.0);
        ui.set_min_height(ui.available_height());
        ui.add_space(19.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("VAPE")
                    .font(FontId::new(16.0, medium()))
                    .color(Pal::TEXT),
            );
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(
                RichText::new("LINUX")
                    .font(FontId::new(8.5, medium()))
                    .color(Pal::ACCENT),
            );
        });

        ui.add_space(30.0);
        for (page, label) in [
            (Page::Inject, "Inject"),
            (Page::Logs, "Logs"),
            (Page::Settings, "Settings"),
        ] {
            let selected = self.page == page;
            let (nav, response) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::click());
            if response.clicked() {
                self.page = page;
            }
            let fill = if selected || response.hovered() {
                Pal::CARD
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(nav, CornerRadius::ZERO, fill);
            ui.painter().circle_filled(
                Pos2::new(nav.left() + 10.0, nav.center().y),
                3.0,
                if selected {
                    Pal::ACCENT
                } else {
                    Pal::TEXT_MUTE
                },
            );
            ui.painter().text(
                Pos2::new(nav.left() + 22.0, nav.center().y),
                Align2::LEFT_CENTER,
                label,
                FontId::new(12.5, medium()),
                if selected { Pal::ACCENT } else { Pal::TEXT_DIM },
            );
        }
    }

    fn logs_page(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Logs")
                    .font(FontId::new(16.0, medium()))
                    .color(Pal::TEXT),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui::ghost_button(ui, "Clear", true).clicked() {
                    self.log.clear();
                }
                if ui::ghost_button(ui, "Copy", true).clicked() {
                    let text = self
                        .log
                        .snapshot()
                        .iter()
                        .map(|line| format!("[{}] {}", line.tag, line.text))
                        .collect::<Vec<_>>()
                        .join("\n");
                    ui.ctx().copy_text(text);
                }
            });
        });
        ui.add_space(18.0);

        let lines = self.log.snapshot();
        egui::Frame::new()
            .fill(Pal::CARD)
            .stroke(Stroke::new(1.0, Pal::LINE_SOFT))
            .corner_radius(CornerRadius::same(Pal::RADIUS_CARD))
            .inner_margin(Margin::symmetric(14, 12))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height((ui.available_height() - 16.0).max(220.0))
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if lines.is_empty() {
                            ui.label(RichText::new("No activity yet").color(Pal::TEXT_MUTE));
                        }
                        for line in lines.iter().rev().take(220).rev() {
                            let mut job = LayoutJob::default();
                            job.wrap.max_width = ui.available_width();
                            job.append(
                                &format!("{:<8}", line.tag),
                                0.0,
                                TextFormat::simple(mono_font(), Pal::TEXT_MUTE),
                            );
                            job.append(
                                &line.text,
                                0.0,
                                TextFormat::simple(mono_font(), kind_color(line.kind)),
                            );
                            ui.label(job);
                        }
                    });
            });
    }

    fn settings_page(&mut self, ui: &mut Ui) {
        ui.label(
            RichText::new("Settings")
                .font(FontId::new(16.0, medium()))
                .color(Pal::TEXT),
        );
        ui.add_space(18.0);

        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Local service")
                            .font(FontId::new(TITLE_FONT, medium()))
                            .color(Pal::TEXT),
                    );
                    let (label, color) = match &self.service_state {
                        ServiceState::Running | ServiceState::External => ("Running", Pal::GREEN),
                        ServiceState::Starting | ServiceState::Building(_) => {
                            ("Starting", Pal::AMBER)
                        }
                        ServiceState::Failed(_) => ("Unavailable", Pal::RED),
                        _ => ("Stopped", Pal::TEXT_MUTE),
                    };
                    ui.label(RichText::new(label).color(color));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let running = self.service_state.is_alive();
                    if ui::ghost_button(ui, if running { "Stop" } else { "Start" }, true).clicked()
                    {
                        if running {
                            self.stop_service();
                        } else if let Some(jar) = self.service_jar.clone() {
                            self.start_service(jar);
                        } else {
                            self.start_service_build();
                        }
                    }
                });
            });
        });

        ui.add_space(10.0);
        let mut changed = false;
        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                let response = ui::toggle(ui, "auto-build", &mut self.config.auto_build);
                changed |= response.changed();
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Build missing components automatically")
                        .font(FontId::new(TITLE_FONT, medium()))
                        .color(Pal::TEXT),
                );
            });
        });
        if changed {
            self.config.save();
        }
    }

    fn service_section(&mut self, ui: &mut Ui, time: f64) {
        section_header(ui, "Service", |ui| {
            if self.service_building {
                let cancel = ui::ghost_button(ui, "Cancel", true);
                if cancel.clicked() {
                    if let Some(token) = self.service_build_cancel.take() {
                        token.cancel();
                    }
                }
            } else {
                let running = self.service_state.is_alive();
                let label = if running { "Stop" } else { "Start" };
                let action = ui::ghost_button(ui, label, self.service_java.is_some());
                if action.clicked() {
                    if running {
                        self.stop_service();
                    } else if let Some(jar) = self.service_jar.clone() {
                        self.start_service(jar);
                    } else {
                        self.start_service_build();
                    }
                }
            }
        });

        ui::card(ui, |ui| {
            let (color, breathe) = match &self.service_state {
                ServiceState::Running | ServiceState::External => (Pal::GREEN, false),
                ServiceState::Starting | ServiceState::Building(_) => (Pal::AMBER, true),
                ServiceState::Failed(_) => (Pal::RED, false),
                _ => (Pal::TEXT_MUTE, false),
            };
            let state_text = service::describe(
                &self.service_state,
                service::DEFAULT_HTTP_PORT,
                service::DEFAULT_ZEUS_PORT,
            );
            let busy = self.service_state.is_busy();
            let failed = matches!(self.service_state, ServiceState::Failed(_));

            let (row, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
            {
                let painter = ui.painter();
                ui::status_dot(
                    painter,
                    Pos2::new(row.left() + 5.0, row.center().y),
                    4.5,
                    color,
                    breathe,
                    time,
                );
                painter.text(
                    Pos2::new(row.left() + 18.0, row.center().y),
                    Align2::LEFT_CENTER,
                    "Local service",
                    FontId::new(TITLE_FONT, medium()),
                    Pal::TEXT,
                );

                let truncated = truncate(ui, &state_text, 240.0);
                if busy {
                    let center = Pos2::new(row.right() - 8.0, row.center().y);
                    ui::spinner(painter, center, 6.5, Pal::AMBER, time);
                    painter.text(
                        Pos2::new(center.x - 15.0, row.center().y),
                        Align2::RIGHT_CENTER,
                        truncated,
                        FontId::new(BODY_FONT, FontFamily::Proportional),
                        Pal::TEXT_DIM,
                    );
                } else {
                    painter.text(
                        Pos2::new(row.right(), row.center().y),
                        Align2::RIGHT_CENTER,
                        truncated,
                        FontId::new(BODY_FONT, FontFamily::Proportional),
                        if failed { Pal::RED } else { Pal::TEXT_DIM },
                    );
                }
            }

            ui.add_space(6.0);
            let mut facts: Vec<String> = vec![
                format!("HTTP {}", service::DEFAULT_HTTP_PORT),
                format!("Zeus {}", service::DEFAULT_ZEUS_PORT),
            ];
            if self.service_healthy {
                facts.push("healthy".to_string());
            }
            if self.zeus_open {
                facts.push("zeus online".to_string());
            }
            if let Some(java) = &self.service_java {
                facts.push(java.display());
            }
            ui.label(
                RichText::new(facts.join("   ·   "))
                    .font(FontId::new(10.5, FontFamily::Proportional))
                    .color(Pal::TEXT_MUTE),
            );

            if matches!(self.service_state, ServiceState::Missing) {
                ui.add_space(10.0);
                ui.label(
                    RichText::new(
                        "The local service is not built yet. It will be cloned from \
                         OpenVapeCN/VapeService and assembled with Gradle.",
                    )
                    .font(FontId::new(BODY_FONT, FontFamily::Proportional))
                    .color(Pal::TEXT_DIM),
                );
                ui.add_space(9.0);
                let clicked =
                    ui::primary_button(ui, "Build service", self.gradle_jdk.is_some(), false, time);
                if clicked.clicked() {
                    self.start_service_build();
                }
            } else if failed && !self.service_building {
                ui.add_space(10.0);
                let retry = ui::ghost_button(ui, "Retry", self.service_java.is_some());
                if retry.clicked() {
                    match self.service_jar.clone() {
                        Some(jar) => self.start_service(jar),
                        None => self.start_service_build(),
                    }
                }
            }
        });
    }

    fn targets_section(&mut self, ui: &mut Ui, time: f64) {
        let count = self.target_list.len();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Minecraft")
                        .font(FontId::new(16.0, medium()))
                        .color(Pal::TEXT),
                );
                if count > 0 {
                    ui.label(
                        RichText::new(format!(
                            "{count} process{}",
                            if count == 1 { "" } else { "es" }
                        ))
                        .font(FontId::new(11.0, FontFamily::Proportional))
                        .color(Pal::TEXT_MUTE),
                    );
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let refresh = ui::icon_button(ui, ui::Icon::Refresh, 30.0, false);
                if refresh.clicked() {
                    self.scan();
                }
                refresh.on_hover_text("Rescan");
            });
        });
        ui.add_space(18.0);

        let missing = self
            .artifacts
            .as_ref()
            .map(|artifacts| artifacts.missing())
            .unwrap_or_else(|| vec!["build/injection".to_string()]);

        if !missing.is_empty() {
            ui::card(ui, |ui| {
                ui.label(
                    RichText::new("Injector is not built")
                        .font(FontId::new(TITLE_FONT, medium()))
                        .color(Pal::AMBER),
                );
                ui.add_space(9.0);
                if self.bundle_building {
                    let cancel = ui::ghost_button(ui, "Cancel build", true);
                    if cancel.clicked() {
                        if let Some(token) = self.bundle_build_cancel.take() {
                            token.cancel();
                        }
                    }
                } else {
                    let clicked = ui::primary_button(
                        ui,
                        "Build injector",
                        self.gradle_jdk.is_some(),
                        false,
                        time,
                    );
                    if clicked.clicked() {
                        self.start_bundle_build();
                    }
                }
            });
            ui.add_space(10.0);
        }

        let targets = self.target_list.clone();
        let mut clicked_pid: Option<u32> = None;
        if targets.is_empty() {
            empty_state(
                ui,
                "No Minecraft process found",
                "Start Minecraft and rescan.",
            );
        } else {
            ui::card(ui, |ui| {
                let selected = self.selected;
                let mode = self.inject_mode;
                for (index, target) in targets.iter().enumerate() {
                    if index > 0 {
                        ui.add_space(2.0);
                    }
                    let outcome = self.outcomes.get(&target.pid);
                    if target_row(
                        ui,
                        target,
                        selected == Some(target.pid),
                        outcome,
                        mode,
                        time,
                        index,
                    ) {
                        clicked_pid = Some(target.pid);
                    }
                }
            });
        }
        if let Some(pid) = clicked_pid {
            self.selected = Some(pid);
        }
    }

    fn options_section(&mut self, ui: &mut Ui) {
        let mut changed = false;
        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                let response = ui::toggle(ui, "force-attach", &mut self.config.force_attach);
                if response.changed() {
                    changed = true;
                }
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Force attach")
                            .font(FontId::new(TITLE_FONT, medium()))
                            .color(Pal::TEXT),
                    );
                });
            });
        });

        if changed {
            self.inject_mode = if self.config.force_attach {
                InjectMode::Force
            } else {
                InjectMode::Attach
            };
            self.config.save();
        }
    }

    fn inject_section(&mut self, ui: &mut Ui, time: f64) {
        let target = self.selected_target().cloned();
        let ready = self
            .artifacts
            .as_ref()
            .map(|artifacts| artifacts.ready())
            .unwrap_or(false);
        // The injector process can exit before the payload has attached to the
        // game thread. Keep the operation busy, and visible, through that
        // second stage so the user cannot accidentally start another attach.
        let progress = self.injection_outcome().map(|outcome| outcome.phase);
        let busy = progress.map(Phase::is_active).unwrap_or(false);
        let enabled = target.is_some() && ready && !busy;

        let label = if busy {
            "Injecting…".to_string()
        } else {
            match &target {
                Some(_) => INJECT_LABEL.to_string(),
                None => "Select a Minecraft process".to_string(),
            }
        };

        let response = ui::primary_button(ui, &label, enabled, busy, time);
        if response.clicked() && enabled {
            self.start_injection();
        }

        if let Some(phase) = progress {
            ui.add_space(9.0);
            let color = match phase {
                Phase::Injecting | Phase::Loaded => Pal::ACCENT_HOVER,
                Phase::Verified => Pal::GREEN,
                Phase::Failed => Pal::RED,
            };
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(phase.progress_label())
                        .font(FontId::new(BODY_FONT, FontFamily::Proportional))
                        .color(color),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{:.0}%", phase.progress() * 100.0))
                            .font(FontId::new(10.5, FontFamily::Monospace))
                            .color(Pal::TEXT_MUTE),
                    );
                });
            });
            ui.add_space(2.0);
            ui::progress_bar(ui, phase.progress(), color);
        }

        if let Some((kind, message)) = self.status.clone() {
            ui.add_space(8.0);
            let color = kind_color(kind);
            ui.horizontal_wrapped(|ui| {
                let (dot, _) = ui.allocate_exact_size(Vec2::new(12.0, 15.0), Sense::hover());
                ui.painter()
                    .circle_filled(Pos2::new(dot.left() + 4.0, dot.center().y), 3.0, color);
                ui.label(
                    RichText::new(message)
                        .font(FontId::new(BODY_FONT, FontFamily::Proportional))
                        .color(color),
                );
            });
        }
    }

    fn activity_section(&mut self, ui: &mut Ui) {
        let showing = self.config.show_log;
        section_header(ui, "Activity", |ui| {
            let toggle = ui::ghost_button(ui, if showing { "Hide" } else { "Show" }, true);
            if toggle.clicked() {
                self.config.show_log = !self.config.show_log;
                self.config.save();
            }
            let copy = ui::ghost_button(ui, "Copy", true);
            if copy.clicked() {
                let text = self
                    .log
                    .snapshot()
                    .iter()
                    .map(|line| format!("[{}] {}", line.tag, line.text))
                    .collect::<Vec<_>>()
                    .join("\n");
                ui.ctx().copy_text(text);
            }
            let clear = ui::ghost_button(ui, "Clear", true);
            if clear.clicked() {
                self.log.clear();
            }
        });

        if !showing {
            return;
        }

        let lines = self.log.snapshot();
        egui::Frame::new()
            .fill(Pal::CARD)
            .stroke(Stroke::new(1.0, Pal::LINE))
            .corner_radius(CornerRadius::same(Pal::RADIUS_CARD))
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| {
                // Use the leftover height when the compositor hands us a
                // large tile, but never squeeze the log to nothing.
                let room = (ui.available_height() - 62.0).clamp(170.0, 460.0);
                egui::ScrollArea::vertical()
                    .max_height(room)
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if lines.is_empty() {
                            ui.label(
                                RichText::new("Nothing has happened yet")
                                    .font(FontId::new(BODY_FONT, FontFamily::Proportional))
                                    .color(Pal::TEXT_MUTE),
                            );
                        }
                        let start = lines.len().saturating_sub(220);
                        for line in &lines[start..] {
                            let mut job = LayoutJob::default();
                            job.wrap.max_width = ui.available_width();
                            job.append(
                                &format!("{:<8}", line.tag),
                                0.0,
                                TextFormat::simple(
                                    FontId::new(9.5, FontFamily::Monospace),
                                    Pal::TEXT_MUTE,
                                ),
                            );
                            job.append(
                                &line.text,
                                0.0,
                                TextFormat::simple(mono_font(), kind_color(line.kind)),
                            );
                            ui.label(job);
                        }
                    });
            });
    }

    fn footer(&mut self, ui: &mut Ui) {
        ui::hairline(ui, Pal::LINE_SOFT);
        ui.add_space(9.0);
        let bundle = self
            .artifacts
            .as_ref()
            .map(|artifacts| artifacts.dir.display().to_string())
            .unwrap_or_else(|| "build/injection not found".to_string());
        dim_mono_row(ui, "bundle", &bundle);
        ui.add_space(3.0);
        dim_mono_row(
            ui,
            "state",
            &util::injection_state_dir().display().to_string(),
        );
        ui.add_space(8.0);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.time = ui.ctx().input(|input| input.time);
        let ctx = ui.ctx().clone();
        self.pump();
        self.poll_jobs();
        self.tick();

        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(120));

        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, CornerRadius::ZERO, Pal::BG);

        let sidebar = Rect::from_min_max(rect.min, Pos2::new(rect.left() + 204.0, rect.bottom()));
        let content =
            Rect::from_min_max(Pos2::new(sidebar.right(), rect.top()), rect.right_bottom());

        ui.painter()
            .rect_filled(sidebar, CornerRadius::ZERO, Pal::SIDEBAR);
        ui.painter().line_segment(
            [sidebar.right_top(), sidebar.right_bottom()],
            Stroke::new(1.0, Pal::LINE_SOFT),
        );
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(sidebar.shrink2(Vec2::new(16.0, 0.0))),
            |ui| self.sidebar(ui),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            self.header(ui);
            ui::hairline(ui, Pal::LINE_SOFT);
            egui::Frame::new()
                .inner_margin(Margin::symmetric(30, 24))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.body(ui);
                });
        });
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let color = Pal::BG;
        [
            color.r() as f32 / 255.0,
            color.g() as f32 / 255.0,
            color.b() as f32 / 255.0,
            1.0,
        ]
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shutdown();
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ------------------------------------------------------------------ helpers

fn paint_square_icon(ui: &Ui, response: &egui::Response, kind: ui::Icon, danger: bool) {
    let rect = response.rect;
    let t = ui
        .ctx()
        .animate_bool_with_time(response.id, response.hovered(), 0.12);
    let painter = ui.painter();
    if t > 0.01 {
        let base = if danger { Pal::RED } else { Pal::CARD_HOVER };
        ui::rounded_rect(
            painter,
            rect,
            Pal::RADIUS_SMALL,
            with_alpha(base, (t * 235.0) as u8),
            None,
        );
    }
    let tint = if response.hovered() && danger {
        Color32::WHITE
    } else {
        mix(
            if danger {
                Pal::TEXT_DIM
            } else {
                Pal::TEXT_MUTE
            },
            Pal::TEXT,
            t,
        )
    };
    ui::icon(painter, kind, rect.center(), 15.0, tint);
}

fn section_header(ui: &mut Ui, title: &str, trailing: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title.to_uppercase())
                .font(label_font())
                .color(Pal::TEXT_MUTE)
                .extra_letter_spacing(0.9),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), trailing);
    });
    ui.add_space(7.0);
}

fn dim_mono_row(ui: &mut Ui, key: &str, value: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(
            RichText::new(key.to_uppercase())
                .font(label_font())
                .color(Pal::TEXT_MUTE)
                .extra_letter_spacing(0.6),
        );
        ui.label(
            RichText::new(value)
                .font(FontId::new(10.0, FontFamily::Monospace))
                .color(Pal::TEXT_MUTE),
        );
    });
}

fn kind_color(kind: LogKind) -> Color32 {
    match kind {
        LogKind::Info => Pal::TEXT_DIM,
        LogKind::Good => Pal::GREEN,
        LogKind::Warn => Pal::AMBER,
        LogKind::Error => Pal::RED,
        LogKind::Dim => Pal::TEXT_MUTE,
    }
}

/// Clips a string to `max_width`, appending an ellipsis when it does not fit.
fn truncate(ui: &Ui, text: &str, max_width: f32) -> String {
    let width_of = |candidate: &str| -> f32 {
        ui.painter()
            .layout_no_wrap(
                candidate.to_string(),
                FontId::new(BODY_FONT, FontFamily::Proportional),
                Pal::TEXT_DIM,
            )
            .size()
            .x
    };
    let full = width_of(text);
    if full <= max_width {
        return text.to_string();
    }
    let ratio = (max_width / full).clamp(0.0, 1.0);
    let mut keep = ((text.chars().count() as f32) * ratio) as usize;
    loop {
        let candidate: String = text.chars().take(keep).collect();
        if keep == 0 || width_of(&format!("{candidate}…")) <= max_width {
            return format!("{candidate}…");
        }
        keep -= 1;
    }
}

fn empty_state(ui: &mut Ui, title: &str, hint: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 74.0), Sense::hover());
    ui::rounded_rect(
        ui.painter(),
        rect,
        Pal::RADIUS_CARD,
        Pal::CARD,
        Some(Pal::LINE_SOFT),
    );
    ui.painter().text(
        Pos2::new(rect.left() + 14.0, rect.center().y - 9.0),
        Align2::LEFT_CENTER,
        title,
        FontId::new(TITLE_FONT, medium()),
        Pal::TEXT_DIM,
    );
    ui.painter().text(
        Pos2::new(rect.left() + 14.0, rect.center().y + 10.0),
        Align2::LEFT_CENTER,
        hint,
        FontId::new(11.0, FontFamily::Proportional),
        Pal::TEXT_MUTE,
    );
}

/// One selectable Minecraft process.
fn target_row(
    ui: &mut Ui,
    target: &Target,
    selected: bool,
    outcome: Option<&Outcome>,
    mode: InjectMode,
    time: f64,
    index: usize,
) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 54.0), Sense::click());
    let t =
        ui.ctx()
            .animate_bool_with_time(egui::Id::new(("target", index)), response.hovered(), 0.12);

    let painter = ui.painter();
    let fill = if selected {
        with_alpha(Pal::ACCENT, 20)
    } else {
        mix(Color32::TRANSPARENT, Pal::CARD_HOVER, t)
    };
    let stroke = if selected {
        Some(with_alpha(Pal::ACCENT, 105))
    } else if t > 0.02 {
        Some(with_alpha(Pal::LINE, (t * 255.0) as u8))
    } else {
        None
    };
    ui::rounded_rect(painter, rect, Pal::RADIUS_CONTROL, fill, stroke);

    if selected {
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(rect.left() + 3.0, rect.center().y - 11.0),
                Vec2::new(3.0, 22.0),
            ),
            CornerRadius::same(2),
            Pal::ACCENT,
        );
    }

    let radio = Pos2::new(rect.left() + 24.0, rect.center().y);
    painter.circle_stroke(
        radio,
        7.0,
        Stroke::new(
            1.5,
            if selected {
                Pal::ACCENT
            } else {
                Pal::TEXT_MUTE
            },
        ),
    );
    if selected {
        painter.circle_filled(radio, 3.6, Pal::ACCENT);
    }

    let text_left = rect.left() + 42.0;
    let right = rect.right() - 12.0;
    // The command-line-free part of the row is clipped so a long Java path can
    // never bleed over the status area or past the card edge.
    let status_left = match outcome.map(|outcome| outcome.phase) {
        Some(Phase::Verified) => right - 62.0,
        Some(Phase::Injecting) | Some(Phase::Loaded) => right - 62.0,
        Some(Phase::Failed) => right - 44.0,
        None if target.attach_disabled && mode == InjectMode::Attach => right - 96.0,
        None => right,
    };
    let text_clip = Rect::from_min_max(
        Pos2::new(text_left, rect.top() + 2.0),
        Pos2::new(status_left.max(text_left + 20.0), rect.bottom() - 2.0),
    );
    let text_painter = painter.with_clip_rect(text_clip);
    text_painter.text(
        Pos2::new(text_left, rect.center().y - 9.0),
        Align2::LEFT_CENTER,
        target.kind,
        FontId::new(TITLE_FONT, medium()),
        Pal::TEXT,
    );
    text_painter.text(
        Pos2::new(text_left, rect.center().y + 10.0),
        Align2::LEFT_CENTER,
        format!("PID {}  ·  {}", target.pid, target.exe.display()),
        FontId::new(10.2, FontFamily::Monospace),
        Pal::TEXT_DIM,
    );

    match outcome.map(|outcome| outcome.phase) {
        Some(Phase::Verified) => {
            let center = Pos2::new(right - 30.0, rect.center().y);
            ui::icon(painter, ui::Icon::Check, center, 13.0, Pal::GREEN);
            painter.text(
                Pos2::new(center.x + 10.0, rect.center().y),
                Align2::LEFT_CENTER,
                "live",
                FontId::new(10.8, FontFamily::Proportional),
                Pal::GREEN,
            );
        }
        Some(Phase::Injecting) | Some(Phase::Loaded) => {
            let center = Pos2::new(right - 22.0, rect.center().y);
            ui::spinner(painter, center, 6.0, Pal::AMBER, time);
            painter.text(
                Pos2::new(center.x - 12.0, rect.center().y),
                Align2::RIGHT_CENTER,
                "loading",
                FontId::new(10.8, FontFamily::Proportional),
                Pal::AMBER,
            );
        }
        Some(Phase::Failed) => {
            painter.text(
                Pos2::new(right, rect.center().y),
                Align2::RIGHT_CENTER,
                "failed",
                FontId::new(10.8, FontFamily::Proportional),
                Pal::RED,
            );
        }
        None => {
            if target.attach_disabled && mode == InjectMode::Attach {
                painter.text(
                    Pos2::new(right, rect.center().y),
                    Align2::RIGHT_CENTER,
                    "attach disabled",
                    FontId::new(10.5, FontFamily::Proportional),
                    Pal::AMBER,
                );
            }
        }
    }

    response.clicked()
}

fn mirror_service_jar(jar: &Path, target_dir: &Path) {
    if std::fs::create_dir_all(target_dir).is_err() {
        return;
    }
    if let Some(name) = jar.file_name() {
        let _ = std::fs::copy(jar, target_dir.join(name));
    }
}

#[cfg(test)]
mod tests {
    use super::Phase;

    #[test]
    fn injection_progress_tracks_real_milestones() {
        assert_eq!(Phase::Injecting.progress(), 0.35);
        assert_eq!(Phase::Loaded.progress(), 0.72);
        assert_eq!(Phase::Verified.progress(), 1.0);
        assert_eq!(Phase::Failed.progress(), 1.0);
        assert!(Phase::Injecting.is_active());
        assert!(Phase::Loaded.is_active());
        assert!(!Phase::Verified.is_active());
        assert!(!Phase::Failed.is_active());
    }
}

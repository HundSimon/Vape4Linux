//! Lifecycle of the local VapeService (OpenVapeCN/VapeService).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::runtime::{run_streamed, CancelToken, Job, LogBuffer, LogKind};
use crate::util::{human_bytes, JavaRuntime};

pub const SERVICE_REPOSITORY: &str = "https://github.com/OpenVapeCN/VapeService.git";
pub const DEFAULT_HTTP_PORT: u16 = 8080;
pub const DEFAULT_ZEUS_PORT: u16 = 8091;

/// The access token the injected client presents.
///
/// The POSIX loader bootstrap is a stub that hands the payload the literal
/// `"0"` (`native/loader_bootstrap_posix.c: vape_loader_access_token()`).
/// VapeService documents that it creates an account for that token on first
/// start, but nothing in the service does: accounts are only minted by
/// `POST /loader/login` (which returns a random 48 hex digit token) or by the
/// App Auth browser challenge, and `AccountRecord.developmentAccount()` is
/// never called. Because `FileStore.requireAccount` rejects unknown tokens,
/// every `/api/v1/0/...` request answers `{"successful":false,
/// "error":"Invalid access token"}` and the Zeus handshake closes the socket.
///
/// The client then never reaches the REGISTERED state and shows
/// "Error establishing connection / Unknown error", so the GUI provisions the
/// account itself before starting the service.
pub const BRIDGE_TOKEN: &str = "0";

#[derive(Clone, PartialEq, Debug)]
pub enum ServiceState {
    Missing,
    Building(String),
    Starting,
    Running,
    External,
    Stopped,
    Failed(String),
}

impl ServiceState {
    pub fn is_alive(&self) -> bool {
        matches!(self, Self::Running | Self::External)
    }
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Starting | Self::Building(_))
    }
}

pub struct ServiceConfig {
    pub java: JavaRuntime,
    pub jar: PathBuf,
    pub http_port: u16,
    pub zeus_port: u16,
    pub data_file: PathBuf,
}

impl ServiceConfig {
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.java.path);
        command
            .arg("-jar")
            .arg(&self.jar)
            .arg("--bind-address")
            .arg("127.0.0.1")
            .arg("--http-port")
            .arg(self.http_port.to_string())
            .arg("--zeus-port")
            .arg(self.zeus_port.to_string())
            .arg("--data-file")
            .arg(&self.data_file);
        command
    }
}

pub fn launch(config: &ServiceConfig, log: &LogBuffer) -> std::io::Result<Job> {
    if let Some(parent) = config.data_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    Job::spawn("service", &mut config.command(), log, &[], true)
}

pub fn http_healthy(port: u16) -> bool {
    let Some(mut stream) = connect(port) else {
        return false;
    };
    let request = "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = String::new();
    use std::io::Read;
    let _ = stream.take(4096).read_to_string(&mut response);
    response.contains("200") && response.contains("UP")
}

pub fn port_open(port: u16) -> bool {
    connect(port).is_some()
}

fn connect(port: u16) -> Option<TcpStream> {
    let address: SocketAddr = ([127, 0, 0, 1], port).into();
    let stream = TcpStream::connect_timeout(&address, Duration::from_millis(350)).ok()?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(600)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(600)));
    Some(stream)
}

/// Clones the service repository when the checkout is absent.
pub fn ensure_source(dir: &Path, log: &LogBuffer, cancel: &CancelToken) -> Result<(), String> {
    if dir.join("gradlew").is_file() {
        return Ok(());
    }
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
    }

    log.push(
        "build",
        LogKind::Info,
        format!("Cloning {SERVICE_REPOSITORY}"),
    );
    let mut command = Command::new("git");
    command
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg(SERVICE_REPOSITORY)
        .arg(dir);
    let status = run_streamed(&mut command, "build", log, cancel)
        .map_err(|error| format!("Cannot run git: {error}"))?;
    if !status.success() {
        return Err("git clone failed".to_string());
    }
    if !dir.join("gradlew").is_file() {
        return Err("The cloned service repository has no Gradle wrapper".to_string());
    }
    Ok(())
}

/// Builds the service jar with the project's own Gradle wrapper.
pub fn build_source(
    dir: &Path,
    jdk: &JavaRuntime,
    log: &LogBuffer,
    cancel: &CancelToken,
) -> Result<PathBuf, String> {
    let jdk_home = jdk_home(jdk);
    log.push(
        "build",
        LogKind::Info,
        format!("Building VapeService with Java {}", jdk.major),
    );

    let mut command = Command::new("bash");
    command
        .arg("gradlew")
        .arg("jar")
        .arg("--console=plain")
        .arg("--no-daemon")
        .current_dir(dir)
        .env("JAVA_HOME", &jdk_home)
        .env("GRADLE_OPTS", "-Dorg.gradle.jvmargs=-Xmx1g");
    let status = run_streamed(&mut command, "build", log, cancel)
        .map_err(|error| format!("Cannot run the Gradle wrapper: {error}"))?;
    if !status.success() {
        return Err("Gradle could not build the service".to_string());
    }

    let libs = dir.join("build/libs");
    let jar = newest_service_jar(&libs)
        .ok_or_else(|| format!("No service jar was produced in {}", libs.display()))?;
    log.push(
        "build",
        LogKind::Good,
        format!(
            "Service jar ready ({}): {}",
            human_bytes(jar.metadata().map(|m| m.len()).unwrap_or(0)),
            jar.display()
        ),
    );
    Ok(jar)
}

pub fn newest_service_jar(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("vape421-experimental-service") || !name.ends_with(".jar") {
            continue;
        }
        if name.contains("sources") || name.contains("javadoc") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(time, _)| modified > *time) {
            best = Some((modified, entry.path()));
        }
    }
    best.map(|(_, path)| path)
}

/// Rebuilds the native injection bundle in the Vape4Linux checkout.
pub fn build_injection_bundle(
    repo_root: &Path,
    jdk: &JavaRuntime,
    log: &LogBuffer,
    cancel: &CancelToken,
) -> Result<(), String> {
    if !repo_root.join("gradlew").is_file() {
        return Err(format!(
            "No Gradle wrapper found in {}",
            repo_root.display()
        ));
    }
    let jdk_home = jdk_home(jdk);
    log.push(
        "bundle",
        LogKind::Info,
        "Building the native injection bundle (Gradle + CMake)…",
    );
    let mut command = Command::new("bash");
    command
        .arg("gradlew")
        .arg("prepareInjectionBundle")
        .arg("--console=plain")
        .arg(format!("-PnativeJavaHome={}", jdk_home.display()))
        .current_dir(repo_root)
        .env("JAVA_HOME", &jdk_home);
    let status = run_streamed(&mut command, "bundle", log, cancel)
        .map_err(|error| format!("Cannot run the Gradle wrapper: {error}"))?;
    if !status.success() {
        return Err("The injection bundle build failed".to_string());
    }
    log.push("bundle", LogKind::Good, "Injection bundle rebuilt");
    Ok(())
}

/// Turns `/usr/lib/jvm/java-17-openjdk/bin/java` into its JDK home.
pub fn jdk_home(java: &JavaRuntime) -> PathBuf {
    java.path
        .parent()
        .and_then(|bin| bin.parent())
        .map(|home| home.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("/usr/lib/jvm/default"))
}

/// One request over loopback, returning the whole raw response.
fn http_request(port: u16, request: &str) -> Option<String> {
    let mut stream = connect(port)?;
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = String::new();
    let _ = stream.take(64 * 1024).read_to_string(&mut response);
    Some(response)
}

fn response_body(response: &str) -> &str {
    response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or(response)
}

/// Confirms that the service accepts the token the injected payload will use.
pub fn check_bridge_account(port: u16) -> Result<(), String> {
    let request = format!(
        "GET /api/v1/{BRIDGE_TOKEN}/authenticated HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    );
    let response = http_request(port, &request)
        .ok_or_else(|| "the service did not answer on loopback".to_string())?;
    let body = response_body(&response);
    let json: serde_json::Value =
        serde_json::from_str(body).map_err(|_| format!("unreadable reply: {body}"))?;
    if json.get("successful").and_then(|value| value.as_bool()) == Some(true) {
        Ok(())
    } else {
        Err(json
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("the account was rejected")
            .to_string())
    }
}

/// Creates the account for [`BRIDGE_TOKEN`] in the service data file.
///
/// Must run while the service is stopped: it caches its store at startup.
/// Returns whether the file was changed.
pub fn provision_bridge_account(data_file: &Path, log: &LogBuffer) -> Result<bool, String> {
    let mut state: serde_json::Value = match std::fs::read_to_string(data_file) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str(&text)
            .map_err(|error| format!("{} is not valid JSON: {error}", data_file.display()))?,
        _ => serde_json::json!({}),
    };

    let root = state
        .as_object_mut()
        .ok_or_else(|| format!("{} does not contain a JSON object", data_file.display()))?;

    let user_id = {
        if !root
            .get("accountsByToken")
            .is_some_and(|value| value.is_object())
        {
            root.insert("accountsByToken".to_string(), serde_json::json!({}));
        }
        let accounts = root
            .get_mut("accountsByToken")
            .and_then(|value| value.as_object_mut())
            .ok_or_else(|| "accountsByToken is not an object".to_string())?;

        if accounts.contains_key(BRIDGE_TOKEN) {
            return Ok(false);
        }
        let user_id = accounts
            .values()
            .filter_map(|account| account.get("userId").and_then(|value| value.as_i64()))
            .max()
            .unwrap_or(0)
            + 1;
        accounts.insert(BRIDGE_TOKEN.to_string(), development_account(user_id));
        user_id
    };

    // Keep the service's id counter ahead of the id we just handed out.
    let counter = root
        .entry("nextUserId".to_string())
        .or_insert_with(|| serde_json::json!(1));
    let current = counter.as_i64().unwrap_or(1);
    *counter = serde_json::json!(current.max(user_id + 1));

    let parent = data_file.parent();
    if let Some(parent) = parent {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
    }
    let serialized = serde_json::to_string_pretty(&state)
        .map_err(|error| format!("Cannot serialise the service store: {error}"))?;
    let temporary = data_file.with_extension("json.tmp");
    std::fs::write(&temporary, serialized)
        .map_err(|error| format!("Cannot write {}: {error}", temporary.display()))?;
    std::fs::rename(&temporary, data_file)
        .map_err(|error| format!("Cannot replace {}: {error}", data_file.display()))?;

    log.push(
        "service",
        LogKind::Good,
        format!("Created the bridge account for access token \"{BRIDGE_TOKEN}\" (user {user_id})"),
    );
    Ok(true)
}

/// Mirrors `AccountRecord.developmentAccount()` plus the service's own
/// default settings, so the client finds every field it expects.
fn development_account(user_id: i64) -> serde_json::Value {
    serde_json::json!({
        "userId": user_id,
        "username": "Developer",
        "accountCreation": utc_timestamp(),
        "licensed": true,
        "registered": true,
        "profiles": true,
        "banned": false,
        "minecraftUuid": "00000000-0000-0000-0000-000000000000",
        "minecraftUsername": "",
        "showUsername": true,
        "presence": 2,
        "serverAddress": serde_json::Value::Null,
        "activeProfileId": -1,
        "onlineFriends": [],
        "globalSettings": { "cache": false, "firstRun": true },
        "onlineSettings": {
            "inventorySwitchMode": 0,
            "partyShowTarget": true,
            "autoLogin": true,
            "showSelf": true,
            "showUsername": true,
            "showInventoryKeybind": [],
            "friendStates": {},
            "shareInventory": false,
            "showServer": true,
            "pingKeybind": []
        },
        "localFriends": [],
        "otherData": [],
        "privateProfiles": {}
    })
}

/// `yyyy-MM-dd'T'HH:mm:ss.SSSXXX` in UTC, the format the service writes.
fn utc_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs() as i64;
    let millis = now.subsec_millis();
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// Howard Hinnant's days-to-civil algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Human readable one-liner for the service card.
pub fn describe(state: &ServiceState, http_port: u16, zeus_port: u16) -> String {
    match state {
        ServiceState::Running => format!("127.0.0.1:{http_port} · Zeus {zeus_port}"),
        ServiceState::External => format!("reusing 127.0.0.1:{http_port} · Zeus {zeus_port}"),
        ServiceState::Starting => "waiting for the HTTP endpoint…".to_string(),
        ServiceState::Building(step) => step.clone(),
        ServiceState::Stopped => "not running".to_string(),
        ServiceState::Missing => "not built yet".to_string(),
        ServiceState::Failed(reason) => reason.clone(),
    }
}

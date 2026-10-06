//! The live desktop application (Phase H4, behind `tauri-shell`).
//!
//! IPC surface (G1 audit + Phase H additions) — every command returns state
//! fields ONLY; the identity seed, engine API key, and lease tokens never
//! cross into the webview:
//!
//! | Command | Returns |
//! |---|---|
//! | `get_status` | tracker, engine, artifact, node, roster, license state |
//! | `set_tracker` | saves the tracker base URL |
//! | `accept_privacy` | records the first-run license acceptance |
//! | `list_models` | verified-catalog active profiles |
//! | `download_model` | streams `download` events, returns artifact state |
//! | `set_hosting` | starts/stops the in-process node (+ roster heartbeat) |
//! | `send_chat` | one function: chat through the node's loopback gateway |
//! | `hf_search_models` | public HF Hub repo search (ADR-023 picker) |
//! | `hf_list_ggufs` | pinned revision + artifact rows for one repo |
//! | `request_model` | files a community model request with the tracker |
//!
//! Honest beta boundary (shown in the UI, not hidden): the node hosts and
//! serves **locally**; remote execution over a transport listener is the
//! Phase F wiring and is surfaced as the `direct-connect` degraded state.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use modelswarm_gateway::DEFAULT_PORT;
use modelswarm_node::artifact::{ArtifactManager, EnsuredArtifact};
use modelswarm_node::{catalog, load_or_create_identity, Node, NodeConfig, NodeHandle};
use modelswarm_telemetry::Telemetry;
use modelswarm_tracker_api::TrackerClient;
use serde::{Deserialize, Serialize};
#[cfg(feature = "tauri-shell")]
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

/// The registered no-listener-yet multiaddr: present in the roster, fails
/// direct-connect honestly for every remote peer (beta boundary).
const ADDR_NO_LISTENER: &str = "/ip4/0.0.0.0/tcp/0";

pub struct DesktopState {
    inner: Mutex<Inner>,
}

struct Inner {
    data_dir: PathBuf,
    tracker_url: String,
    profile_id: Option<String>,
    /// Cached HF artifact sizes for requirement hints: profile -> (bytes, fetched_unix_ms).
    size_cache: std::collections::HashMap<String, (u64, u64)>,
    license_accepted: bool,
    artifact: Option<EnsuredArtifact>,
    node: Option<RunningNode>,
    chat_log: Vec<ChatTurn>,
}

struct RunningNode {
    handle: NodeHandle,
    shutdown: tokio::sync::watch::Sender<bool>,
    heartbeat: tokio::task::JoinHandle<()>,
    enrollment: Arc<tokio::sync::RwLock<EnrollmentView>>,
}

/// The enrollment/roster half of "swarm readiness", shared between the
/// heartbeat task (writer) and `get_status` (reader) so the UI can show the
/// honest phase instead of a silent 401 loop.
#[derive(Debug, Clone, Default, Serialize)]
struct EnrollmentView {
    /// `enrolling` | `pending` | `enrolled` | `error`
    phase: String,
    user_code: String,
    verify_url: String,
    expires_at: String,
    error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurn {
    role: String,
    content: String,
}

impl DesktopState {
    pub async fn new() -> Self {
        let data_dir = modelswarm_node::default_data_dir();
        let _ = std::fs::create_dir_all(&data_dir);
        let config = read_config(&data_dir);
        let license_accepted = license_accepted(&config.tracker, &data_dir).await;
        Self {
            inner: Mutex::new(Inner {
                data_dir,
                tracker_url: config.tracker,
                size_cache: Default::default(),
                profile_id: config.profile_id,
                license_accepted,
                artifact: None,
                node: None,
                chat_log: Vec::new(),
            }),
        }
    }
}

/// Loads (or creates) the installation identity and checks the
/// privacy-gate acceptance row. Any failure reads as "not accepted".
async fn license_accepted(tracker_url: &str, data_dir: &std::path::Path) -> bool {
    let Ok(tracker) = tracker_client(tracker_url, data_dir).await else {
        return false;
    };
    let installation_id = tracker.identity().installation_id();
    let Ok(store) = modelswarm_store::Store::open(data_dir.join("state.sqlite")) else {
        return false;
    };
    store
        .has_license(&installation_id, "privacy-gate")
        .unwrap_or(false)
}

#[derive(Serialize, Deserialize, Default)]
struct PersistedConfig {
    #[serde(default = "default_tracker")]
    tracker: String,
    #[serde(default)]
    profile_id: Option<String>,
}

fn default_tracker() -> String {
    "https://modelswarm.deepflux.space".to_string()
}

fn read_config(data_dir: &std::path::Path) -> PersistedConfig {
    let mut config: PersistedConfig = std::fs::read_to_string(data_dir.join("config.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    // A persisted empty string would otherwise override the default tracker
    // (serde defaults only apply to missing keys) and every fetch would die
    // on a relative URL.
    if config.tracker.trim().is_empty() {
        config.tracker = default_tracker();
    }
    config
}

/// Where each OS bundle actually installs the pinned engine (tauri.conf
/// `resources: ["engine/*"]` preserves the `engine/` prefix under the
/// platform resource dir): beside the exe on Windows NSIS, `/usr/lib/…` on
/// Linux deb/AppImage, `Contents/Resources` on macOS — i.e. exactly what
/// `resource_dir()` resolves. Dev fallbacks cover staged/flat layouts.
fn engine_binary_path(app: &tauri::AppHandle) -> PathBuf {
    let name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("engine").join(name));
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("modelswarm"));
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    candidates.push(dir.join("engine").join(name));
    candidates.push(dir.join(name));
    candidates.push(dir.join("resources").join(name));
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .unwrap_or_else(|| dir.join("engine").join(name))
}

/// Appends one event to the node log (same sink/format as the node's own
/// events) so client-side failures — catalog fetches above all — leave a
/// trail even when the UI is the only thing reporting them.
fn log_event(data_dir: &std::path::Path, level: &str, event: &str, fields: &[(&str, &str)]) {
    let _ = std::fs::create_dir_all(data_dir.join("logs"));
    let Ok(sink) = modelswarm_telemetry::FileSink::open(&data_dir.join("logs").join("node.jsonl"))
    else {
        return;
    };
    let telemetry = Telemetry::with_sink(Box::new(sink));
    match level {
        "warn" => telemetry.warn(event, fields),
        _ => telemetry.info(event, fields),
    };
}

async fn tracker_client(
    tracker_url: &str,
    data_dir: &std::path::Path,
) -> Result<Arc<TrackerClient>, String> {
    // Telemetry sink is required by the loader; the desktop reuses the node's
    // log file. Missing dirs are created by Node normally; here we tolerate.
    let logs_dir = data_dir.join("logs");
    let _ = std::fs::create_dir_all(&logs_dir);
    let log_path = logs_dir.join("node.jsonl");
    let sink = modelswarm_telemetry::FileSink::open(&log_path)
        .map_err(|e| format!("telemetry sink: {e}"))?;
    let telemetry = Telemetry::with_sink(Box::new(sink));
    let identity = load_or_create_identity(data_dir, &telemetry).map_err(|e| e.to_string())?;
    Ok(Arc::new(TrackerClient::new(
        tracker_url,
        Arc::new(identity),
    )))
}

/// Resolves a profile from the verified catalog: by id when given, else the
/// first active one.
async fn resolve_profile(
    tracker_url: &str,
    data_dir: &std::path::Path,
    profile_id: Option<&str>,
) -> Result<catalog::ProfileListing, String> {
    let tracker = tracker_client(tracker_url, data_dir).await?;
    let profiles = catalog::active_profiles(&tracker)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(id) = profile_id {
        return profiles
            .into_iter()
            .find(|p| p.profile_id == id)
            .ok_or_else(|| format!("profile {id} is not in the active catalog"));
    }
    profiles
        .into_iter()
        .next()
        .ok_or_else(|| "catalog has no active profile".to_string())
}

fn persist_config(inner: &Inner) -> Result<(), String> {
    std::fs::write(
        inner.data_dir.join("config.json"),
        serde_json::to_string_pretty(&PersistedConfig {
            tracker: inner.tracker_url.clone(),
            profile_id: inner.profile_id.clone(),
        })
        .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// Basic hardware requirements for a model card — honest estimates, labeled
/// as such: download size is exact (HF HEAD, cached); RAM adds a conservative
/// KV-cache + runtime overhead on top of the weights; the pinned engine is a
/// CPU build, so no GPU is required (or used).
pub fn hardware_requirements(download_bytes: Option<u64>) -> Option<serde_json::Value> {
    let bytes = download_bytes?;
    let download_mb = bytes / 1_000_000;
    const KV_CACHE_MB: u64 = 128; // <=0.5B params @ 4k ctx, f16 K+V
    const RUNTIME_OVERHEAD_MB: u64 = 256; // llama-server + webview + app
    let ram_mb = (download_mb + KV_CACHE_MB + RUNTIME_OVERHEAD_MB).div_ceil(128) * 128;
    Some(serde_json::json!({
        "download_mb": download_mb,
        "ram_mb_est": ram_mb,
        "gpu": "not required (CPU engine)",
        "note": "estimates; RAM = weights + KV cache + runtime overhead",
    }))
}

/// Exact artifact size via HEAD (redirect-following), 10-minute cache.
async fn artifact_size(http: &reqwest::Client, url: &str) -> Option<u64> {
    let response = http
        .head(url)
        .timeout(Duration::from_secs(4))
        .send()
        .await
        .ok()?;
    response.content_length()
}

// ---- ChatML rendering (Qwen/ChatML family; the profile pins the template) ----

pub fn render_chatml(messages: &[ChatTurn]) -> String {
    let mut out = String::new();
    for turn in messages {
        out.push_str(&format!(
            "<|im_start|>{}\n{}<|im_end|>\n",
            turn.role, turn.content
        ));
    }
    out.push_str("<|im_start|>assistant\n");
    out
}

// ---- IPC commands ---------------------------------------------------------

#[tauri::command]
async fn get_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<serde_json::Value, String> {
    let inner = state.inner.lock().await;
    let engine = engine_binary_path(&app);
    let node_running = inner.node.is_some();
    let enrollment = match &inner.node {
        Some(running) => Some(running.enrollment.read().await.clone()),
        None => None,
    };
    let (gateway_addr, installation_id, engine_port) = match &inner.node {
        Some(running) => (
            Some(running.handle.local_addr.to_string()),
            Some(running.handle.installation_id.clone()),
            running.handle.engine_port,
        ),
        None => (None, None, None),
    };
    Ok(serde_json::json!({
        "version": app.package_info().version.to_string(),
        "tracker": inner.tracker_url,
        "data_dir": inner.data_dir.display().to_string(),
        "engine_available": engine.is_file(),
        "engine_path": engine.display().to_string(),
        "license_accepted": inner.license_accepted,
        "profile_id": inner.profile_id,
        "artifact": inner.artifact.as_ref().map(|a| serde_json::json!({
            "path": a.path.display().to_string(),
            "bytes": a.bytes,
        })),
        "node": {
            "running": node_running,
            "hosting": node_running,
            "gateway_addr": gateway_addr,
            "installation_id": installation_id,
            "engine_port": engine_port,
            "enrollment": enrollment,
        },
        "chat_turns": inner.chat_log.len(),
    }))
}

#[tauri::command]
async fn set_tracker(state: tauri::State<'_, DesktopState>, url: String) -> Result<(), String> {
    let mut inner = state.inner.lock().await;
    if inner.node.is_some() {
        return Err("stop hosting before changing the tracker".into());
    }
    let url = url.trim().trim_end_matches('/');
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("tracker URL must start with http:// or https://".into());
    }
    inner.tracker_url = url.to_string();
    let config = PersistedConfig {
        tracker: inner.tracker_url.clone(),
        profile_id: inner.profile_id.clone(),
    };
    std::fs::write(
        inner.data_dir.join("config.json"),
        serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn accept_privacy(state: tauri::State<'_, DesktopState>) -> Result<(), String> {
    let mut inner = state.inner.lock().await;
    let tracker = tracker_client(&inner.tracker_url, &inner.data_dir).await?;
    let installation_id = tracker.identity().installation_id();
    let store = modelswarm_store::Store::open(inner.data_dir.join("state.sqlite"))
        .map_err(|e| e.to_string())?;
    store
        .record_license(&installation_id, "privacy-gate")
        .map_err(|e| e.to_string())?;
    inner.license_accepted = true;
    Ok(())
}

#[tauri::command]
async fn list_models(
    state: tauri::State<'_, DesktopState>,
) -> Result<Vec<serde_json::Value>, String> {
    let mut inner = state.inner.lock().await;
    let tracker = tracker_client(&inner.tracker_url, &inner.data_dir).await?;
    // One retry after a short pause: a single reset packet between here and
    // the hub should not read as "catalog down". Both attempts are logged;
    // the second failure is what the UI shows.
    let profiles = match catalog::active_profiles(&tracker).await {
        Ok(profiles) => profiles,
        Err(first) => {
            log_event(
                &inner.data_dir,
                "warn",
                "catalog.error",
                &[
                    ("error", &first.to_string()),
                    ("tracker", &inner.tracker_url),
                ],
            );
            tokio::time::sleep(Duration::from_secs(1)).await;
            catalog::active_profiles(&tracker).await.map_err(|second| {
                format!(
                    "{second} (tracker {url}, retried once)",
                    url = inner.tracker_url
                )
            })?
        }
    };
    let store = modelswarm_store::Store::open(inner.data_dir.join("state.sqlite")).ok();
    let manager = ArtifactManager::new(&inner.data_dir);
    let head_client = reqwest::Client::new();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    const SIZE_TTL_MS: u64 = 10 * 60 * 1000;
    let mut out = Vec::new();
    for p in profiles {
        let downloaded = store
            .as_ref()
            .and_then(|s| s.get_artifact(&p.profile_id).ok())
            .flatten()
            .map(|(_, _, bytes, _state)| serde_json::json!({ "bytes": bytes }))
            .or_else(|| {
                manager
                    .artifact_path(&p.manifest)
                    .ok()
                    .filter(|path| path.exists())
                    .map(|_| serde_json::json!({ "bytes": 0 }))
            });
        // Exact artifact size: store row when downloaded, else cached HEAD.
        let size_bytes = match &downloaded {
            Some(d) if d["bytes"].as_u64().unwrap_or(0) > 0 => {
                Some(d["bytes"].as_u64().unwrap_or(0))
            }
            _ => {
                let fresh = inner
                    .size_cache
                    .get(&p.profile_id)
                    .is_some_and(|(_, at)| now_ms.saturating_sub(*at) < SIZE_TTL_MS);
                if fresh {
                    inner.size_cache.get(&p.profile_id).map(|(bytes, _)| *bytes)
                } else {
                    let url = format!(
                        "https://huggingface.co/{}/resolve/{}/{}",
                        p.manifest.hf_repo(),
                        p.manifest.hf_revision(),
                        p.manifest.artifact_hashes()[0].path(),
                    );
                    let bytes = artifact_size(&head_client, &url).await;
                    if let Some(bytes) = bytes {
                        inner
                            .size_cache
                            .insert(p.profile_id.clone(), (bytes, now_ms));
                    }
                    bytes
                }
            }
        };
        out.push(serde_json::json!({
            "profile_id": p.profile_id,
            "display_name": p.display_name,
            "quantization": format!("{} ({} bit)", p.manifest.quantization().method(), p.manifest.quantization().bits()),
            "runtime": p.manifest.runtime().version(),
            "downloaded": downloaded,
            "hw": hardware_requirements(size_bytes),
            "selected": inner.profile_id.as_deref() == Some(p.profile_id.as_str()),
        }));
    }
    Ok(out)
}

#[tauri::command]
async fn download_model(
    state: tauri::State<'_, DesktopState>,
    window: tauri::Window,
) -> Result<serde_json::Value, String> {
    let (data_dir, tracker_url) = {
        let inner = state.inner.lock().await;
        (inner.data_dir.clone(), inner.tracker_url.clone())
    };
    let profile = resolve_profile(&tracker_url, &data_dir, None).await?;
    let manager = ArtifactManager::new(&data_dir);
    let store = std::sync::Mutex::new(
        modelswarm_store::Store::open(data_dir.join("state.sqlite")).map_err(|e| e.to_string())?,
    );
    let ensured = manager
        .ensure_artifact(&profile.manifest, &store, |done, total| {
            let _ = window.emit(
                "download",
                serde_json::json!({ "done": done, "total": total }),
            );
        })
        .await
        .map_err(|e| e.to_string())?;
    let mut inner = state.inner.lock().await;
    inner.profile_id = Some(profile.profile_id.clone());
    inner.artifact = Some(ensured.clone());
    Ok(serde_json::json!({
        "profile_id": profile.profile_id,
        "display_name": profile.display_name,
        "bytes": ensured.bytes,
        "path": ensured.path.display().to_string(),
    }))
}

#[tauri::command]
async fn set_hosting(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    window: tauri::Window,
    on: bool,
) -> Result<serde_json::Value, String> {
    set_hosting_inner(&app, &state, &window, on).await
}

async fn set_hosting_inner(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, DesktopState>,
    _window: &tauri::Window,
    on: bool,
) -> Result<serde_json::Value, String> {
    let mut inner = state.inner.lock().await;
    if !on {
        if let Some(running) = inner.node.take() {
            let _ = running.shutdown.send(true);
            running.heartbeat.abort();
            let _ = running.handle.stopped().await;
        }
        return Ok(serde_json::json!({ "hosting": false }));
    }

    if !inner.license_accepted {
        return Err("accept the privacy disclosure first".into());
    }
    let engine = engine_binary_path(app);
    if !engine.is_file() {
        return Err(format!(
            "pinned engine not found next to the app (expected {})",
            engine.display()
        ));
    }
    let listing = resolve_profile(
        &inner.tracker_url,
        &inner.data_dir,
        inner.profile_id.as_deref(),
    )
    .await
    .map_err(|e| format!("catalog: {e}"))?;
    let manager = ArtifactManager::new(&inner.data_dir);
    let store = std::sync::Mutex::new(
        modelswarm_store::Store::open(inner.data_dir.join("state.sqlite"))
            .map_err(|e| e.to_string())?,
    );
    let ensured = manager
        .ensure_artifact(&listing.manifest, &store, |_, _| {})
        .await
        .map_err(|e| format!("artifact: {e}"))?;

    let (shutdown_tx, _) = tokio::sync::watch::channel(false);
    let mut config = NodeConfig {
        tracker_base: Some(inner.tracker_url.clone()),
        // Stable local API first (DEFAULT_PORT 11435); fall back to an
        // ephemeral port when it is taken so a second installation runs too.
        gateway_port: DEFAULT_PORT,
        data_dir: inner.data_dir.clone(),
        profile_id: Some(listing.profile_id.clone()),
        mock: false,
        engine_binary: Some(engine),
        engine_model: Some(ensured.path.clone()),
        engine_threads: None,
    };
    let handle = match Node::start(config.clone(), shutdown_tx.subscribe()).await {
        Ok(handle) => handle,
        Err(_) => {
            config.gateway_port = 0;
            Node::start(config, shutdown_tx.subscribe())
                .await
                .map_err(|e| e.to_string())?
        }
    };

    // Roster heartbeat: device-enroll (approval gated) → session →
    // register + heartbeat so exact-profile peers see this machine (with
    // the honest no-listener address until transport lands).
    let heartbeat_tracker = tracker_client(&inner.tracker_url, &inner.data_dir).await?;
    // peerId is derived inside the heartbeat task (ADR-020); the
    // installation_id stays a UI label.
    let profile_id = listing.profile_id.clone();
    let build = listing.manifest.runtime().build_hash().to_string();
    let mut heartbeat_shutdown = shutdown_tx.subscribe();
    let hb_data_dir = inner.data_dir.clone();
    let enrollment_view = Arc::new(tokio::sync::RwLock::new(EnrollmentView {
        phase: "enrolling".into(),
        ..EnrollmentView::default()
    }));
    let enroll_view = Arc::clone(&enrollment_view);
    let heartbeat = tokio::spawn(async move {
        // peerId is the ADR-020 identity-multihash derivation of the pubKey
        // — NOT the installationId label (the tracker rejects the latter;
        // found live 2026-10-06 when registration first got past the
        // schema gate).
        let peer_id = heartbeat_tracker.identity().peer_id();
        let runtime_desc = runtime_wire_desc(&build);
        let mut lease_id: Option<String> = None;
        let mut roster_failures: u32 = 0;
        let mut pending_device: Option<String> = None;
        // One-shot: poll /complete immediately after a fresh device code
        // (auto-approval enrolls within one round-trip) instead of waiting
        // a full tick. Cleared after a single short sleep so a pathological
        // start→complete-401 cycle can never spin the loop hot.
        let mut poll_now = false;
        loop {
            if *heartbeat_shutdown.borrow() {
                return;
            }

            // ---- device enrollment (automatic, approval-gated) ----
            if !heartbeat_tracker.has_session() {
                if let Some(code) = pending_device.clone() {
                    match heartbeat_tracker.device_complete(&code).await {
                        Ok(done) => {
                            if let Some(token) = done.get("token").and_then(|v| v.as_str()) {
                                heartbeat_tracker.set_session(token.to_string());
                                pending_device = None;
                                *enroll_view.write().await = EnrollmentView {
                                    phase: "enrolled".into(),
                                    ..EnrollmentView::default()
                                };
                                log_event(&hb_data_dir, "info", "enroll.approved", &[]);
                                lease_id = None; // register below with the new session
                            }
                            // Malformed-but-2xx: keep polling the same code.
                        }
                        Err(modelswarm_tracker_api::TrackerError::Api { code, .. })
                            if code == "pending" =>
                        {
                            // Still awaiting owner approval — keep polling.
                        }
                        Err(modelswarm_tracker_api::TrackerError::Api { status: 401, .. }) => {
                            // Device code expired/unknown: restart enrollment.
                            pending_device = None;
                            *enroll_view.write().await = EnrollmentView {
                                phase: "enrolling".into(),
                                ..EnrollmentView::default()
                            };
                        }
                        Err(error) => {
                            roster_failures += 1;
                            if roster_failures == 1 || roster_failures % 20 == 0 {
                                log_event(
                                    &hb_data_dir,
                                    "warn",
                                    "enroll.error",
                                    &[("error", &error.to_string())],
                                );
                            }
                            *enroll_view.write().await = EnrollmentView {
                                phase: "error".into(),
                                error: format!(
                                    "approval check failed: {error} (retrying every 30 s)"
                                ),
                                ..EnrollmentView::default()
                            };
                        }
                    }
                } else {
                    match heartbeat_tracker.device_start().await {
                        Ok(start) => {
                            let view = EnrollmentView {
                                phase: "pending".into(),
                                user_code: start
                                    .get("userCode")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                verify_url: start
                                    .get("verifyUrl")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                expires_at: start
                                    .get("expiresAt")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                error: String::new(),
                            };
                            log_event(
                                &hb_data_dir,
                                "info",
                                "enroll.pending",
                                &[("user_code", &view.user_code)],
                            );
                            pending_device = start
                                .get("deviceCode")
                                .and_then(|v| v.as_str())
                                .map(str::to_string);
                            *enroll_view.write().await = view;
                            poll_now = true;
                        }
                        Err(error) => {
                            roster_failures += 1;
                            if roster_failures == 1 || roster_failures % 20 == 0 {
                                log_event(
                                    &hb_data_dir,
                                    "warn",
                                    "enroll.error",
                                    &[("error", &error.to_string())],
                                );
                            }
                            *enroll_view.write().await = EnrollmentView {
                                phase: "error".into(),
                                error: format!(
                                    "device enrollment failed: {error} (retrying every 30 s)"
                                ),
                                ..EnrollmentView::default()
                            };
                        }
                    }
                }
            }

            // ---- roster register/heartbeat (session required) ----
            let result = if heartbeat_tracker.has_session() {
                match &lease_id {
                    Some(id) => heartbeat_tracker
                        .heartbeat(id, std::slice::from_ref(&profile_id), 1, 0, false)
                        .await
                        .map(|_| ())
                        .or_else(|_| {
                            // Lease lost (expiry/revocation): re-register next tick.
                            lease_id = None;
                            Ok(())
                        }),
                    None => heartbeat_tracker
                        .register(
                            &peer_id,
                            &[ADDR_NO_LISTENER.to_string()],
                            std::slice::from_ref(&profile_id),
                            1,
                            runtime_desc.clone(),
                        )
                        .await
                        .map(|response| {
                            lease_id = response
                                .get("leaseId")
                                .and_then(|v| v.as_str())
                                .map(str::to_string);
                        }),
                }
            } else {
                Ok(())
            };
            if let Err(error) = result {
                // A windowed app's stderr is invisible: roster failures must
                // reach the log file, throttled to the first failure and
                // then every 20th (10 min). A 401/403 here means the session
                // died (24 h expiry / revocation): drop it and re-enroll.
                roster_failures += 1;
                if roster_failures == 1 || roster_failures % 20 == 0 {
                    log_event(
                        &hb_data_dir,
                        "warn",
                        "roster.error",
                        &[
                            ("error", &error.to_string()),
                            ("attempt", &roster_failures.to_string()),
                        ],
                    );
                }
                if let modelswarm_tracker_api::TrackerError::Api { status, .. } = &error {
                    if *status == 401 || *status == 403 {
                        heartbeat_tracker.clear_session();
                        *enroll_view.write().await = EnrollmentView {
                            phase: "enrolling".into(),
                            ..EnrollmentView::default()
                        };
                    }
                }
                eprintln!("modelswarm-desktop: roster: {error}");
            }
            if poll_now {
                poll_now = false;
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                    _ = heartbeat_shutdown.changed() => return,
                }
            } else {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(30)) => {}
                    _ = heartbeat_shutdown.changed() => return,
                }
            }
        }
    });

    let status = serde_json::json!({
        "hosting": true,
        "profile_id": listing.profile_id,
        "gateway_addr": handle.local_addr.to_string(),
        "installation_id": handle.installation_id,
    });
    inner.profile_id = Some(listing.profile_id.clone());
    inner.artifact = Some(ensured);
    inner.node = Some(RunningNode {
        handle,
        shutdown: shutdown_tx,
        heartbeat,
        enrollment: enrollment_view,
    });
    Ok(status)
}

/// The picker's single action (owner UX spec): select a model → download or
/// load it → the swarm starts working. UX-study P0 (2026-10-06): selecting a
/// DIFFERENT profile while hosting is a one-action SWAP — the new artifact
/// downloads while the old node keeps serving, then the old node stops and
/// the new one starts. One advertised profile at a time (project rule);
/// downtime is the engine restart only. Re-selecting the hosted profile is
/// an idempotent no-op.
#[tauri::command]
async fn select_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    window: tauri::Window,
    profile_id: String,
) -> Result<serde_json::Value, String> {
    {
        let inner = state.inner.lock().await;
        if !inner.license_accepted {
            return Err("accept the privacy disclosure first".into());
        }
        if inner.node.is_some() && inner.profile_id.as_deref() == Some(profile_id.as_str()) {
            return Ok(serde_json::json!({ "hosting": true, "profile_id": profile_id }));
        }
    }

    // Resolve + fetch the new artifact FIRST (download events flow to the
    // UI) so the running swarm serves throughout the download.
    let (tracker_url, data_dir) = {
        let inner = state.inner.lock().await;
        (inner.tracker_url.clone(), inner.data_dir.clone())
    };
    let listing = resolve_profile(&tracker_url, &data_dir, Some(&profile_id))
        .await
        .map_err(|e| format!("catalog: {e}"))?;
    let manager = ArtifactManager::new(&data_dir);
    let store = std::sync::Mutex::new(
        modelswarm_store::Store::open(data_dir.join("state.sqlite")).map_err(|e| e.to_string())?,
    );
    manager
        .ensure_artifact(&listing.manifest, &store, |done, total| {
            let _ = window.emit(
                "download",
                serde_json::json!({ "done": done, "total": total }),
            );
        })
        .await
        .map_err(|e| format!("artifact: {e}"))?;

    // Artifact ready: stop the old node (if any), then start the new one
    // via the normal hosting path — ensure_artifact hits the cache, so
    // there is no second download.
    {
        let mut inner = state.inner.lock().await;
        if let Some(running) = inner.node.take() {
            let _ = running.shutdown.send(true);
            running.heartbeat.abort();
            let _ = running.handle.stopped().await;
        }
        inner.profile_id = Some(profile_id.clone());
        persist_config(&inner)?;
    }
    let result = set_hosting_inner(&app, &state, &window, true).await?;
    Ok(result)
}

#[tauri::command]
async fn lookup_peers(
    state: tauri::State<'_, DesktopState>,
) -> Result<Vec<serde_json::Value>, String> {
    let (tracker_url, data_dir, profile) = {
        let inner = state.inner.lock().await;
        match &inner.profile_id {
            Some(profile) => (
                inner.tracker_url.clone(),
                inner.data_dir.clone(),
                profile.clone(),
            ),
            None => return Ok(Vec::new()),
        }
    };
    let tracker = tracker_client(&tracker_url, &data_dir).await?;
    let peers = tracker
        .lookup(&profile, 10)
        .await
        .map_err(|e| e.to_string())?;
    Ok(peers
        .into_iter()
        .map(|p| {
            serde_json::json!({
                "peer_id": p.peer_id,
                "lease_expires_at": p.lease_expires_at,
                "capacity_class": p.capacity_class,
                "nat_path": "no listener (beta) — direct-connect will fail",
            })
        })
        .collect())
}

#[tauri::command]
async fn send_chat(
    state: tauri::State<'_, DesktopState>,
    message: String,
    max_tokens: Option<u32>,
) -> Result<serde_json::Value, String> {
    let (gateway, profile, mut log) = {
        let inner = state.inner.lock().await;
        let running = inner.node.as_ref().ok_or("hosting is off")?;
        (
            running.handle.local_addr.to_string(),
            inner.profile_id.clone().ok_or("no profile selected")?,
            inner.chat_log.clone(),
        )
    };

    // One function, honestly local: the whole conversation is rendered with
    // the profile's ChatML template and sent as a single greedy request
    // through the node's loopback gateway.
    log.push(ChatTurn {
        role: "user".into(),
        content: message,
    });
    if log.len() > 20 {
        let drop = log.len() - 20;
        log.drain(0..drop);
    }
    let prompt = render_chatml(&log);

    let started = Instant::now();
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())?;
    let response: serde_json::Value = http
        .post(format!("http://{gateway}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": profile,
            "messages": [{ "role": "user", "content": prompt }],
            "max_tokens": max_tokens.unwrap_or(256),
            "temperature": 0.0,
            "stream": false,
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let content = response["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string();
    let completion_tokens = response["usage"]["completion_tokens"].as_u64().unwrap_or(0);
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let tok_s = if elapsed_ms > 0 && completion_tokens > 0 {
        (completion_tokens as f64) * 1000.0 / elapsed_ms as f64
    } else {
        0.0
    };

    let mut inner = state.inner.lock().await;
    inner.chat_log = log;
    // An empty reply (engine hiccup, gateway error) must NOT enter the
    // rendered history: an empty assistant turn teaches the model to emit
    // EOS immediately and every later turn comes back empty too.
    if !content.is_empty() {
        inner.chat_log.push(ChatTurn {
            role: "assistant".into(),
            content: content.clone(),
        });
    }
    Ok(serde_json::json!({
        "content": content,
        "completion_tokens": completion_tokens,
        "elapsed_ms": elapsed_ms,
        "tokens_per_second": tok_s,
        "mode": "local single",
    }))
}

/// One-click device approval: open the tracker's /verify page (pairing code
/// prefilled by the site) in the system browser. Rust-side validation — only
/// the tracker's approval page may be opened, and the opener call bypasses
/// the webview entirely (the shipped 0.2.7 JS-side `__TAURI__.opener` call
/// produced no visible effect; the OS path itself was verified working).
#[tauri::command]
async fn open_approval_page(app: tauri::AppHandle, url: String) -> Result<(), String> {
    const VERIFY_PREFIX: &str = "https://modelswarm.deepflux.space/verify";
    let url = url.trim().to_string();
    if !url.starts_with(VERIFY_PREFIX) {
        return Err(format!("refusing to open non-tracker URL: {url}"));
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

/// Runtime descriptor for /peers/register (msp-v1 §3.3): the schema is
/// STRICT — exactly `{name, build}`, both ≤64 chars. Extra keys (the old
/// `version`/`build_hash` pair) 400 the registration; keep this shape
/// pinned by test.
fn runtime_wire_desc(build: &str) -> serde_json::Value {
    serde_json::json!({ "name": "llama.cpp", "build": build })
}

// ---- Hugging Face picker (ADR-023): search → file list → model request ----
// The webview CSP never gains a network origin: all Hub calls run in the
// node process; the UI receives plain JSON rows.

/// Quantization token parsed from an artifact filename — requestable quants
/// only (≤8 bit), longest tokens first so `q4_k_m` wins over `q4_0`.
fn quant_of_filename(path: &str) -> Option<(&'static str, u8)> {
    const TABLE: &[(&str, u8)] = &[
        ("q3_k_s", 3),
        ("q3_k_m", 3),
        ("q3_k_l", 3),
        ("q4_k_s", 4),
        ("q4_k_m", 4),
        ("q5_k_s", 5),
        ("q5_k_m", 5),
        ("q2_k", 2),
        ("q4_0", 4),
        ("q5_0", 5),
        ("q5_1", 5),
        ("q6_k", 6),
        ("q8_0", 8),
    ];
    let lower = path.to_ascii_lowercase();
    let delimited = |token: &str| {
        let mut start = 0;
        while let Some(at) = lower[start..].find(token) {
            let s = start + at;
            let e = s + token.len();
            let before_ok = s == 0 || !lower.as_bytes()[s - 1].is_ascii_alphanumeric();
            let after_ok = e == lower.len() || !lower.as_bytes()[e].is_ascii_alphanumeric();
            if before_ok && after_ok {
                return true;
            }
            start = s + 1;
        }
        false
    };
    TABLE
        .iter()
        .find(|(token, _)| delimited(token))
        .map(|(token, bits)| (*token, *bits))
}

fn valid_hf_repo(repo: &str) -> bool {
    let mut parts = repo.splitn(2, '/');
    let user = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    let ok = |s: &str| {
        !s.is_empty()
            && s.len() <= 100
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    };
    ok(user)
        && name.len() <= 100
        && repo.len() <= 120
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        && repo.chars().filter(|c| *c == '/').count() == 1
}

fn hf_http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn hf_search_models(query: String) -> Result<Vec<serde_json::Value>, String> {
    let q = query.trim().to_string();
    if q.is_empty() || q.len() > 100 {
        return Err("search query must be 1–100 characters".into());
    }
    let http = hf_http()?;
    let hits: serde_json::Value = http
        .get("https://huggingface.co/api/models")
        .query(&[
            ("library", "gguf"),
            ("search", q.as_str()),
            ("sort", "downloads"),
            ("direction", "-1"),
            ("limit", "20"),
        ])
        .header("user-agent", "modelswarm-desktop")
        .send()
        .await
        .map_err(|e| format!("huggingface.co unreachable: {e}"))?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let Some(list) = hits.as_array() else {
        return Ok(Vec::new());
    };
    Ok(list
        .iter()
        .filter_map(|m| {
            let repo = m.get("id")?.as_str()?.to_string();
            if !valid_hf_repo(&repo) {
                return None;
            }
            Some(serde_json::json!({
                "repo": repo,
                "downloads": m.get("downloads").and_then(|v| v.as_u64()).unwrap_or(0),
                "likes": m.get("likes").and_then(|v| v.as_u64()).unwrap_or(0),
            }))
        })
        .collect())
}

#[tauri::command]
async fn hf_list_ggufs(repo: String) -> Result<serde_json::Value, String> {
    if !valid_hf_repo(&repo) {
        return Err("not a valid huggingface repo id".into());
    }
    let http = hf_http()?;
    let meta: serde_json::Value = http
        .get(format!("https://huggingface.co/api/models/{repo}"))
        .header("user-agent", "modelswarm-desktop")
        .send()
        .await
        .map_err(|e| format!("huggingface.co unreachable: {e}"))?
        .error_for_status()
        .map_err(|e| format!("repo not found: {e}"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let revision = meta
        .get("sha")
        .and_then(|v| v.as_str())
        .ok_or("hub returned no pinned revision")?
        .to_string();
    if revision.len() != 40 || !revision.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("hub returned a malformed revision sha".into());
    }
    // List at the pinned sha (not `main`) so file rows and revision agree.
    let tree: serde_json::Value = http
        .get(format!(
            "https://huggingface.co/api/models/{repo}/tree/{revision}"
        ))
        .query(&[("limit", "1000")])
        .header("user-agent", "modelswarm-desktop")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let Some(files) = tree.as_array() else {
        return Err("hub returned no file tree".into());
    };
    let mut out = Vec::new();
    for f in files {
        let path = match f.get("path").and_then(|v| v.as_str()) {
            Some(p) if p.ends_with(".gguf") => p,
            _ => continue,
        };
        let lfs = f.get("lfs");
        let (bytes, sha) = match (f.get("size").and_then(|v| v.as_u64()), lfs) {
            (Some(bytes), Some(lfs)) => {
                let sha = lfs.get("oid").and_then(|v| v.as_str()).unwrap_or("");
                if sha.len() != 64 {
                    continue; // not an LFS artifact we can pin
                }
                (bytes, sha.to_string())
            }
            _ => continue,
        };
        // Only requestable quants (≤8 bit): the resolver's quant cross-check
        // would reject anything else anyway.
        let Some((method, bits)) = quant_of_filename(path) else {
            continue;
        };
        out.push(serde_json::json!({
            "path": path,
            "bytes": bytes,
            "sha256": sha,
            "quant_method": method,
            "quant_bits": bits,
            "hw": hardware_requirements(Some(bytes)),
        }));
    }
    out.sort_by_key(|f| f["bytes"].as_u64().unwrap_or(0));
    Ok(serde_json::json!({ "repo": repo, "revision": revision, "files": out }))
}

#[tauri::command]
async fn request_model(
    state: tauri::State<'_, DesktopState>,
    payload: serde_json::Value,
) -> Result<serde_json::Value, String> {
    // Shape-validate everything again server-of-the-client: the tracker's
    // strict zod would reject it anyway, but the error should be legible here.
    let repo = payload["hf_repo"].as_str().unwrap_or_default().to_string();
    let revision = payload["hf_revision"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let path = payload["artifact_path"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let sha = payload["artifact_sha256"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let bytes = payload["artifact_bytes"].as_u64().unwrap_or(0);
    let quant_method = payload["quant_method"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let quant_bits = payload["quant_bits"].as_u64().unwrap_or(0);
    let display_name = payload["display_name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    if !valid_hf_repo(&repo)
        || revision.len() != 40
        || !revision.chars().all(|c| c.is_ascii_hexdigit())
        || path.is_empty()
        || path.len() > 200
        || !path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c))
        || sha.len() != 64
        || !sha.chars().all(|c| c.is_ascii_hexdigit())
        || bytes == 0
        || quant_method.is_empty()
        || quant_method.len() > 24
        || !(1..=8).contains(&quant_bits)
        || display_name.is_empty()
        || display_name.len() > 80
    {
        return Err("request payload failed validation".into());
    }
    let (tracker_url, data_dir) = {
        let inner = state.inner.lock().await;
        (inner.tracker_url.clone(), inner.data_dir.clone())
    };
    log_event(
        &data_dir,
        "info",
        "model.request",
        &[("repo", &repo), ("artifact", &path)],
    );
    let http = hf_http()?;
    let response: serde_json::Value = http
        .post(format!("{tracker_url}/api/v1/catalog/requests"))
        .json(&serde_json::json!({
            "hf_repo": repo,
            "hf_revision": revision,
            "artifact_path": path,
            "artifact_sha256": sha,
            "artifact_bytes": bytes,
            "quant_method": quant_method,
            "quant_bits": quant_bits,
            "display_name": display_name,
        }))
        .send()
        .await
        .map_err(|e| format!("tracker unreachable: {e}"))?
        .error_for_status()
        .map_err(|e| format!("tracker rejected the request: {e}"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(response)
}

pub fn run() {
    tauri::Builder::default()
        // Opens the device-approval page in the system browser (capability
        // scopes it to the tracker origin only).
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::block_on(async move {
                let state = DesktopState::new().await;
                handle.manage(state);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            set_tracker,
            accept_privacy,
            list_models,
            download_model,
            set_hosting,
            select_model,
            lookup_peers,
            send_chat,
            open_approval_page,
            hf_search_models,
            hf_list_ggufs,
            request_model
        ])
        .run(tauri::generate_context!())
        .expect("modelswarm desktop shell failed to start");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quant_tokens_parse_from_filenames_with_delimiters() {
        assert_eq!(
            quant_of_filename("qwen2.5-1.5b-instruct-q4_k_m.gguf"),
            Some(("q4_k_m", 4))
        );
        assert_eq!(
            quant_of_filename("Qwen3-30B-A3B-Q4_K_M.gguf"),
            Some(("q4_k_m", 4))
        );
        assert_eq!(
            quant_of_filename("model-Q4_K_M-imod.gguf"),
            Some(("q4_k_m", 4))
        );
        assert_eq!(quant_of_filename("llama-8b-q8_0.gguf"), Some(("q8_0", 8)));
        // Unrequestable / absent quants must not guess.
        assert_eq!(quant_of_filename("model-bf16.gguf"), None);
        assert_eq!(quant_of_filename("tokenizer.json"), None);
        // q4_0 must not match inside q4_k_m (delimiters prevent it).
        assert_ne!(quant_of_filename("m-q4_k_m.gguf"), Some(("q4_0", 4)));
    }

    #[test]
    fn hf_repo_ids_are_validated() {
        assert!(valid_hf_repo("Qwen/Qwen2.5-1.5B-Instruct-GGUF"));
        assert!(valid_hf_repo("ggml-org/Qwen3-1.7B-GGUF"));
        assert!(!valid_hf_repo("Qwen/Qwen2.5/x"));
        assert!(!valid_hf_repo("../etc/passwd"));
        assert!(!valid_hf_repo("no-slash"));
        assert!(!valid_hf_repo(""));
    }

    #[test]
    fn runtime_descriptor_is_exactly_name_and_build() {
        // /peers/register's runtime object is strict zod {name, build} —
        // anything else 400s the registration (found live 2026-10-06: the
        // desktop sent version/build_hash and never registered).
        let desc = runtime_wire_desc("353c4aab0f0d");
        let obj = desc.as_object().unwrap();
        assert_eq!(obj.len(), 2, "no extra keys allowed: {desc}");
        assert_eq!(obj["name"], "llama.cpp");
        assert_eq!(obj["build"], "353c4aab0f0d");
    }

    #[test]
    fn hardware_requirements_are_conservative_and_labeled() {
        let hw = hardware_requirements(Some(105_454_432)).unwrap();
        assert_eq!(hw["download_mb"], 105);
        assert_eq!(hw["ram_mb_est"], 512);
        assert!(hw["note"].as_str().unwrap().contains("estimates"));

        let hw = hardware_requirements(Some(491_400_032)).unwrap();
        assert_eq!(hw["download_mb"], 491);
        assert_eq!(hw["ram_mb_est"], 896);

        // Unknown size -> no invented numbers (fail-closed display).
        assert!(hardware_requirements(None).is_none());
    }

    #[test]
    fn chatml_renders_full_conversation_with_generation_prompt() {
        let turns = vec![
            ChatTurn {
                role: "system".into(),
                content: "You are terse.".into(),
            },
            ChatTurn {
                role: "user".into(),
                content: "Hi".into(),
            },
            ChatTurn {
                role: "assistant".into(),
                content: "Hello.".into(),
            },
            ChatTurn {
                role: "user".into(),
                content: "Bye".into(),
            },
        ];
        assert_eq!(
            render_chatml(&turns),
            "<|im_start|>system\nYou are terse.<|im_end|>\n\
             <|im_start|>user\nHi<|im_end|>\n\
             <|im_start|>assistant\nHello.<|im_end|>\n\
             <|im_start|>user\nBye<|im_end|>\n\
             <|im_start|>assistant\n"
        );
    }
}

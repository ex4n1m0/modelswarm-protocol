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

fn engine_binary_path() -> PathBuf {
    let name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("modelswarm"));
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    // The NSIS bundle installs the pinned engine into `engine/` beside the
    // exe (tauri.conf.json `resources: ["engine/*"]`); dev builds may stage
    // it flat or under `resources/`. First hit wins.
    let candidates = [
        format!("engine/{name}"),
        name.to_string(),
        format!("resources/{name}"),
    ];
    for candidate in &candidates {
        let path = dir.join(candidate);
        if path.is_file() {
            return path;
        }
    }
    dir.join(&candidates[0])
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
    let engine = engine_binary_path();
    let node_running = inner.node.is_some();
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
    state: tauri::State<'_, DesktopState>,
    window: tauri::Window,
    on: bool,
) -> Result<serde_json::Value, String> {
    set_hosting_inner(&state, &window, on).await
}

async fn set_hosting_inner(
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
    let engine = engine_binary_path();
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

    // Roster heartbeat: register + heartbeat so exact-profile peers see this
    // machine (with the honest no-listener address until transport lands).
    let heartbeat_tracker = tracker_client(&inner.tracker_url, &inner.data_dir).await?;
    let peer_id = handle.installation_id.clone();
    let profile_id = listing.profile_id.clone();
    let version = listing.manifest.runtime().version().to_string();
    let build = listing.manifest.runtime().build_hash().to_string();
    let mut heartbeat_shutdown = shutdown_tx.subscribe();
    let hb_data_dir = inner.data_dir.clone();
    let heartbeat = tokio::spawn(async move {
        let runtime_desc = serde_json::json!({
            "name": "llama.cpp", "version": version, "build_hash": build,
        });
        let mut lease_id: Option<String> = None;
        let mut roster_failures: u32 = 0;
        loop {
            if *heartbeat_shutdown.borrow() {
                return;
            }
            let result = match &lease_id {
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
            };
            if let Err(error) = result {
                // A windowed app's stderr is invisible: roster failures (an
                // unenrolled installation reads as 401 until the ops
                // approval gate clears) must reach the log file, throttled
                // to the first failure and then every 20th (10 min).
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
                eprintln!("modelswarm-desktop: roster: {error}");
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(30)) => {}
                _ = heartbeat_shutdown.changed() => return,
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
    });
    Ok(status)
}

/// The picker's single action (owner UX spec): select a model → download or
/// load it → the swarm starts working (node up, registered, local API live).
#[tauri::command]
async fn select_model(
    state: tauri::State<'_, DesktopState>,
    window: tauri::Window,
    profile_id: String,
) -> Result<serde_json::Value, String> {
    {
        let mut inner = state.inner.lock().await;
        if !inner.license_accepted {
            return Err("accept the privacy disclosure first".into());
        }
        if inner.node.is_some() {
            return Err("stop hosting before switching models".into());
        }
        inner.profile_id = Some(profile_id.clone());
        persist_config(&inner)?;
    }
    let result = set_hosting_inner(&state, &window, true).await?;
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
    inner.chat_log.push(ChatTurn {
        role: "assistant".into(),
        content: content.clone(),
    });
    Ok(serde_json::json!({
        "content": content,
        "completion_tokens": completion_tokens,
        "elapsed_ms": elapsed_ms,
        "tokens_per_second": tok_s,
        "mode": "local single",
    }))
}

pub fn run() {
    tauri::Builder::default()
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
            send_chat
        ])
        .run(tauri::generate_context!())
        .expect("modelswarm desktop shell failed to start");
}

#[cfg(test)]
mod tests {
    use super::*;

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

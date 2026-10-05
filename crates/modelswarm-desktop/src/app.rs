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

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
struct ChatTurn {
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
                profile_id: None,
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
}

fn default_tracker() -> String {
    "https://modelswarm.deepflux.space".to_string()
}

fn read_config(data_dir: &std::path::Path) -> PersistedConfig {
    std::fs::read_to_string(data_dir.join("config.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn engine_binary_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("llama-server.exe")))
        .unwrap_or_else(|| PathBuf::from("llama-server.exe"))
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

/// Resolves the one active profile from the verified catalog.
async fn resolve_profile(
    tracker_url: &str,
    data_dir: &std::path::Path,
) -> Result<catalog::ProfileListing, String> {
    let tracker = tracker_client(tracker_url, data_dir).await?;
    let profiles = catalog::active_profiles(&tracker)
        .await
        .map_err(|e| e.to_string())?;
    profiles
        .into_iter()
        .next()
        .ok_or_else(|| "catalog has no active profile".to_string())
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
async fn get_status(state: tauri::State<'_, DesktopState>) -> Result<serde_json::Value, String> {
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
        "tracker": inner.tracker_url,
        "data_dir": inner.data_dir.display().to_string(),
        "engine_available": engine.is_file(),
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
    inner.tracker_url = url.trim_end_matches('/').to_string();
    let config = PersistedConfig {
        tracker: inner.tracker_url.clone(),
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
    let inner = state.inner.lock().await;
    let tracker = tracker_client(&inner.tracker_url, &inner.data_dir).await?;
    let profiles = catalog::active_profiles(&tracker)
        .await
        .map_err(|e| e.to_string())?;
    Ok(profiles
        .into_iter()
        .map(|p| {
            serde_json::json!({
                "profile_id": p.profile_id,
                "display_name": p.display_name,
                "quantization": format!("{} ({} bit)", p.manifest.quantization().method(), p.manifest.quantization().bits()),
                "runtime": p.manifest.runtime().version(),
            })
        })
        .collect())
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
    let profile = resolve_profile(&tracker_url, &data_dir).await?;
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
    on: bool,
) -> Result<serde_json::Value, String> {
    let mut inner = state.inner.lock().await;
    if !on {
        if let Some(mut running) = inner.node.take() {
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
    let listing = resolve_profile(&inner.tracker_url, &inner.data_dir)
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

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let config = NodeConfig {
        tracker_base: Some(inner.tracker_url.clone()),
        gateway_port: 0, // ephemeral; the UI and send_chat learn it
        data_dir: inner.data_dir.clone(),
        profile_id: Some(listing.profile_id.clone()),
        mock: false,
        engine_binary: Some(engine),
        engine_model: Some(ensured.path.clone()),
        engine_threads: None,
    };
    let handle = Node::start(config, shutdown_rx)
        .await
        .map_err(|e| e.to_string())?;

    // Roster heartbeat: register + heartbeat so exact-profile peers see this
    // machine (with the honest no-listener address until transport lands).
    let heartbeat_tracker = tracker_client(&inner.tracker_url, &inner.data_dir).await?;
    let peer_id = handle.installation_id.clone();
    let profile_id = listing.profile_id.clone();
    let version = listing.manifest.runtime().version().to_string();
    let build = listing.manifest.runtime().build_hash().to_string();
    let mut heartbeat_shutdown = shutdown_tx.subscribe();
    let heartbeat = tokio::spawn(async move {
        let runtime_desc = serde_json::json!({
            "name": "llama.cpp", "version": version, "build_hash": build,
        });
        let mut lease_id: Option<String> = None;
        loop {
            if *heartbeat_shutdown.borrow() {
                return;
            }
            let result = match &lease_id {
                Some(id) => heartbeat_tracker
                    .heartbeat(id, &[profile_id.clone()], 1, 0, false)
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
                        &[profile_id.clone()],
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

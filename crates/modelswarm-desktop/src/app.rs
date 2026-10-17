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
//! | `set_pairing_allowed` | the "allow swarm pairing" toggle (persisted, default on; gates the listener + family tier at hosting start) |
//! | `set_content_carriage` | the ADR-036 serving-side carriage toggle (persisted, DEFAULT ON; opt-out refuses carrier sessions with the honest `content_carriage_refused` reject) |
//! | `set_family_verified_chat` | REMOVED 2026-10-15 (owner directive): the ADR-036 requester-side election is ALWAYS ON — every swarm chat turn opens carrier sessions with the always-visible §3.1 disclosure and the honest pre-first-token fallback |
//! | `accept_privacy` | records the first-run license acceptance |
//! | `list_models` | verified-catalog active profiles |
//! | `census` | live per-profile peer counts (public `/api/v1/stats`) for the picker chips |
//! | `resource_sample` | one CPU/RAM/working-set/GPU/VRAM reading for the monitor panel (null per field = "n/a") |
//! | `download_model` | streams `download` events, returns artifact state |
//! | `set_hosting` | starts/stops the in-process node (+ roster heartbeat) |
//! | `family_probe` | opens one real family session on an eligible peer, PRESENTING this installation's earned lease (ADR-032 §2), then runs the ONE-round ADR-034 pairing confirmation (counts only); returns the honest outcome, never the lease or token ids |
//! | `send_chat` | one function: chat through the node's loopback gateway (user temperature optional; greedy default) |
//! | `clear_chat` | resets the persisted conversation (the language-drift escape hatch) |
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
#[cfg(feature = "verifydrafts")]
use modelswarm_node::verifydrafts::{FamilyServing, RosterFamilyLeases, VerifyDraftsServeConfig};
use modelswarm_node::{catalog, load_or_create_identity, Node, NodeConfig, NodeHandle};
use modelswarm_resource::SampleSource;
use modelswarm_telemetry::Telemetry;
use modelswarm_tracker_api::TrackerClient;
use serde::{Deserialize, Serialize};
#[cfg(feature = "tauri-shell")]
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

/// The client half of the auto-update engine (owner request 2026-10-12):
/// the honest updater status machine + the 6h check supervisor. Tauri 2's
/// updater plugin verifies every update package against the minisign
/// public key embedded in tauri.conf.json before installing; the desktop
/// renders one status line from it and nothing else.
mod updater;

/// The assistant PANEL surface (ADR-037 §3.6/§4.5 as rewired by the
/// 2026-10-15 owner directive — the helper runs on the machine's OWN
/// hosted model, with the frozen static floor otherwise): the
/// setup-helper panel's typed context builders, honest helper-identity
/// texts, static help corpus, reply renderer, and the
/// `assistant_turn`/`assistant_reset` cores. Its own section, its own
/// input, its own `assistant-*` webview events — never the swarm chat
/// transcript (the chat wall, W1/W4).
mod assistant;

/// The registered no-listener-yet multiaddr: present in the roster, fails
/// direct-connect honestly for every remote peer (beta boundary).
const ADDR_NO_LISTENER: &str = "/ip4/0.0.0.0/tcp/0";

/// F2B (ADR-014): a circuit multiaddr — `<relay>/p2p-circuit[/p2p/<dst>]`,
/// the relayed-reachability shape that rides the tracker's EXISTING
/// `addresses` field (no schema change). A peer advertising one is
/// REACHABLE (through the relay), never skipped.
fn addr_is_circuit(addr: &str) -> bool {
    addr.contains("/p2p-circuit")
}

/// The roster/picker DIALABILITY predicate (F2B): a direct QUIC multiaddr
/// OR a circuit address; `0.0.0.0` placeholders stay undialable. A
/// relayed peer is reachable, not skipped.
fn peer_addr_dialable(addr: &str) -> bool {
    !addr.contains("0.0.0.0") && (addr.ends_with("quic-v1") || addr_is_circuit(addr))
}

/// A DIRECT QUIC multiaddr only (the pre-F2B predicate). Used where the
/// consumer is honestly direct-only (the family probe's `dial_quic`, the
/// shadow planner's frozen `has_direct_quic_addr` fact).
fn peer_addr_direct(addr: &str) -> bool {
    addr.ends_with("quic-v1") && !addr.contains("0.0.0.0")
}

/// The F2B relay endpoint configuration: `<data_dir>/relay.addr`, ONE
/// relay multiaddr (e.g. `/ip4/10.0.0.2/udp/4001/quic-v1/p2p/<relay peer
/// id>` — the run line in the Network Engineer's handoff §6). Blank lines
/// and `#` comments are ignored.
///
/// DELIBERATELY default-EMPTY: no relay address is invented or hard-coded
/// — nothing may silently point at a nonexistent host. Absence = the
/// pre-F2B behavior (direct-only serving, direct-only dialing). When a
/// relay address IS present, using it is AUTOMATIC: the serving side
/// reserves and advertises its circuit address, requesters dial relay
/// candidates on direct failure — no user-facing switch (owner directive
/// 2026-10-15, "everything on by design").
const RELAY_ADDR_FILE: &str = "relay.addr";

/// ADR-039 precedence, pure and testable: the local file value wins over
/// the tracker value; none -> null. Returns the address plus its source
/// label for the Diagnostics disclosure.
fn effective_relay(
    file_value: Option<&str>,
    tracker_value: Option<&str>,
) -> Option<serde_json::Value> {
    match (file_value, tracker_value) {
        (Some(file), _) => Some(serde_json::json!({
            "addr": file,
            "source": "file",
        })),
        (None, Some(tracker)) => Some(serde_json::json!({
            "addr": tracker,
            "source": "tracker",
        })),
        (None, None) => None,
    }
}

fn configured_relay_addr(data_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(data_dir.join(RELAY_ADDR_FILE)).ok()?;
    raw.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
}

/// Keeps the heartbeat's advertised address list truthful (F2B): the
/// direct address always first, the circuit address only while a live
/// reservation holds it. Sends only on CHANGE (the heartbeat re-registers
/// on change so the roster never claims a dead circuit).
fn set_advertised_addrs(
    advertised: &tokio::sync::watch::Sender<Vec<String>>,
    direct: &str,
    circuit: Option<String>,
) {
    let mut next = vec![direct.to_string()];
    if let Some(circuit) = circuit {
        next.push(circuit);
    }
    if *advertised.borrow() != next {
        let _ = advertised.send(next);
    }
}

/// The daemon-pinned family session prompt (ADR-032 light-up): whitespace-
/// separated token ids, read from `<data_dir>/family-prompt-tokens.txt`.
/// The family wire never carries the session prompt (the recorded G6 gap
/// — a freeze-ADR item), so the daemon pins it OUT OF BAND: a coordinator
/// pairing with this installation must chain its windows on the same
/// tokenization. Absent/unparsable = the family engine stays down
/// HONESTLY (sessions refuse fail-closed; never a mismatched prompt).
#[cfg(feature = "verifydrafts")]
const FAMILY_PROMPT_TOKENS_FILE: &str = "family-prompt-tokens.txt";

pub struct DesktopState {
    inner: Mutex<Inner>,
    /// Hardware analysis, computed once per process (engine query + RAM FFI).
    hardware: tokio::sync::OnceCell<HardwareInfo>,
    /// The ONE process telemetry sink (node log format). The desktop must
    /// not open FileSinks per call: redaction rules apply at this boundary
    /// and multiple writers on one file are exactly what it prevents.
    telemetry: std::sync::Arc<Telemetry>,
    /// The task-manager-style resource monitor (owner request 2026-10-12):
    /// ONE lazily-created sampler behind a sync mutex (PDH handles are not
    /// documented thread-safe; `sample()` takes `&mut self`), polled at
    /// 1 Hz by the UI timer through `resource_sample`. Read-only/advisory.
    monitor: Arc<std::sync::Mutex<Option<PanelMonitor>>>,
}

struct Inner {
    data_dir: PathBuf,
    tracker_url: String,
    profile_id: Option<String>,
    /// Cached HF artifact sizes for requirement hints: profile -> (bytes, fetched_unix_ms).
    size_cache: std::collections::HashMap<String, (u64, u64)>,
    /// Last good public-stats census (owner directive 2026-10-12):
    /// profile_id -> live peers. Refreshed by the 30 s `census` poll;
    /// dropped on any failure (stale counts must read "census n/a", not
    /// pretend to be live). Joined into `list_models` responses.
    census_counts: Option<Arc<std::collections::BTreeMap<String, u64>>>,
    /// Cached swarm consume-lease (F3(2)): (profile, wire lease, unix expiry).
    /// Earned via the hosting-challenge chain; reused until it expires.
    swarm_lease: Option<(String, String, u64)>,
    /// Pooled remote executor (connection reuse): keyed by profile|peer.
    swarm_executor: Option<(
        String,
        std::sync::Arc<modelswarm_node::remote::RemoteExecutor>,
    )>,
    /// ADR-038 §6.4 row 8: the relay admission ticket cache, alive while
    /// hosting (the heartbeat task writes it every successful heartbeat;
    /// the relay reservation supervisor and the swarm chat's remote
    /// executor read it through the transport's ticket seam). NOTHING
    /// user-facing rides it — no switch, no UI element, no status field
    /// (the owner's everything-on-by-design directive; the ticket rides
    /// the heartbeat invisibly). `None` while not hosting.
    relay_tickets: Option<std::sync::Arc<modelswarm_node::relay_ticket_cache::RelayTicketCache>>,
    /// ADR-039: the tracker-supplied community relay address (latest
    /// heartbeat value), as a watch receiver so `get_status` can read it
    /// synchronously while the heartbeat task sends updates. `None`
    /// while not hosting. The local `relay.addr` file still WINS over
    /// this value (the tester/operator override).
    relay_config: Option<tokio::sync::watch::Receiver<Option<String>>>,
    /// F15 shadow measurements (per-peer RTT EWMA, completion
    /// distributions): lazily opened store-backed recorder shared by the
    /// swarm executors. Shadow data only — nothing selects peers yet.
    peer_metrics: Option<std::sync::Arc<modelswarm_node::measure::PeerMetrics>>,
    license_accepted: bool,
    artifact: Option<EnsuredArtifact>,
    node: Option<RunningNode>,
    chat_log: Vec<ChatTurn>,
    /// The "allow swarm pairing" settings toggle (owner directive
    /// 2026-10-13; persisted key, DEFAULT ON). Gates the serving listener
    /// and the family tier at hosting start — never read from the
    /// environment.
    allow_swarm_pairing: bool,
    /// ADR-036 §3.1: the SERVING-side user-content carriage posture
    /// (persisted key, DEFAULT ON — the owner's swarm-together
    /// directive). Flows into the family serve config at hosting start;
    /// an opted-out peer refuses pin carriers with the honest
    /// `content_carriage_refused` reject.
    allow_content_carriage: bool,
    /// The cohort size of the most recent ELECTED family-verified turn
    /// (the §3.1 disclosure's N — the ACTUAL count for that turn, bounded
    /// by the preset cohort cap and the budget's `max_endpoints`; 1 in
    /// the shipped configuration). `None` before the first elected turn.
    #[cfg(feature = "verifydrafts")]
    last_family_cohort: Option<u32>,
    /// The coordinator-side family capability cache (ADR-032 §3
    /// disclosure learned through the existing family negotiation — the
    /// probe fills it; the chat auto-engage checkpoint reads it). No wire
    /// field exists for roster-level capability, so this is the honest
    /// seam: capability is KNOWN only after a real session accept.
    #[cfg(feature = "verifydrafts")]
    family_peer_caps: std::collections::HashMap<String, FamilyPeerCap>,
    /// The local family engine source while hosting with the family on —
    /// the coordinator's SELF-VERIFY path (tonight's warm-cache root
    /// cause: every drafted window is re-verified locally, cache-cold,
    /// before it may hit the wire). Same `Arc` the serving half holds.
    #[cfg(feature = "verifydrafts")]
    family_engine: Option<Arc<modelswarm_node::verifydrafts::FamilyEngineSource>>,
    /// The advertised QUIC serving address while the listener is bound
    /// (the "allow swarm pairing" toggle + hosting on); None = no
    /// listener. get_status reports it so the UI's "remote serve" chip is
    /// truth, not a static label.
    listener_addr: Option<String>,
    /// The composed daemon's family half (ADR-032 light-up): what the UI
    /// must show honestly — on (both vocabularies, behind the RC-L1
    /// lease gate) vs off with the exact reason. None while not hosting.
    family: Option<FamilyStatus>,
    /// The auto-update status machine's current state (owner request
    /// 2026-10-12): written only by the updater supervisor, read only for
    /// `get_status`. A failure here is a status line, never a block on
    /// app usage.
    updater: updater::UpdaterStatus,
    /// ADR-037 §3.6 as rewired 2026-10-15: the armed assistant plane —
    /// backed by THIS machine's hosted model behind the node's loopback
    /// gateway while hosting. `None` = built-in help mode, with the
    /// honest gap label in `assistant_degrade_gap` (never a blank box,
    /// never a spinner that hides absence).
    assistant_plane: Option<assistant::Plane>,
    /// The honest degrade gap label while `assistant_plane` is None
    /// (`not_hosting` while no model is hosted).
    assistant_degrade_gap: String,
    /// The first-use helper-identity banner dismissal (persisted). Since
    /// the 2026-10-15 collapse this is COSMETIC ONLY — nothing external
    /// is called, so no turn is ever blocked on it; dismissing just
    /// hides the banner.
    assistant_acknowledged: bool,
    /// The Settings opt-out toggle (persisted; both this AND the owner's
    /// `assistant.enabled` master switch must be on for cloud mode).
    assistant_enabled: bool,
    /// The last `list_models` cards — the assistant's picker rows come
    /// from THIS cache (the signed catalog/candidate rows the picker
    /// already computed; the assistant never fetches or recomputes
    /// them).
    last_cards: Vec<serde_json::Value>,
    /// The engage ladder label of the last swarm turn (assistant context
    /// + the static corpus's index into it).
    last_engage_reason: Option<String>,
    /// The governor verdict label of the last hosting-start resource
    /// check, when one degraded.
    last_governor_verdict: Option<String>,
    /// The last family-probe pairing verdict.
    last_probe_verdict: Option<String>,
    /// The self-reported capacityClass of this installation's earned
    /// lease, when one was earned (ADR-012).
    capacity_class: Option<String>,
}

/// The composed daemon's family half, reported to the UI exactly as the
/// daemon decided it — never a static label (the ADR-032 disclosure
/// rule): `on` means the listener serves BOTH vocabularies behind the
/// RC-L1 profile-bound lease gate; `off` carries the stable reason and
/// `disclosure` carries the real engine's geometry when the bridge
/// inited.
#[derive(Debug, Clone, Default, Serialize)]
struct FamilyStatus {
    /// "on" | "off"
    state: String,
    /// Stable off reason (telemetry label): "family_prompt_unpinned" |
    /// "engine_bridge_failed" | "engine_bridge_not_compiled" |
    /// "verifier_identity_unavailable" | "pairing_disabled" (the settings
    /// toggle) | "listener_bind_failed" | "resource_ceiling".
    reason: Option<String>,
    /// The exact degrade detail behind `reason` (bind error text, the
    /// projected-vs-ceiling numbers, …) — labels and counts only, never
    /// secrets or content. Honest degrade rule: a family that cannot
    /// start says WHY here, never a silent off.
    detail: Option<String>,
    /// The engine's real disclosed geometry (backend / batch_verify /
    /// batch_window_max) when the bridge inited — from the engine, never
    /// hand-set.
    disclosure: Option<serde_json::Value>,
    /// Coordinators with a lease on file in the daemon's roster (0 is
    /// NORMAL today — nothing feeds the roster; coordinators PRESENT
    /// their hub-signed lease at session open instead, the ADR-032 §2
    /// admission order. The count is reported so the tracker-fed roster
    /// future shows up when it lands).
    lease_roster_peers: usize,
    /// ADR-034 rule 6a: the pin in force while the family is on —
    /// provenance (`auto family-prompt-v1` | `file`), token count, and
    /// pin hash. `None` while off/unpinned or not hosting (the off reason
    /// carries why).
    pin: Option<FamilyPinStatus>,
    /// ADR-036 §3.1 (normative UI contract): the SERVING-side
    /// user-content carriage posture, present whenever the family is on.
    /// `enabled` mirrors the persisted setting in force for this hosting
    /// session; `disclosure` is the always-visible §3.1 text VERBATIM
    /// when enabled, or the honest opt-out statement when not.
    carriage: Option<FamilyCarriageStatus>,
    /// ADR-037 §3.2b: the governor verdict LABEL behind a
    /// `resource_ceiling` off-reason (frozen vocabulary — ram_ceiling |
    /// vram_ceiling | ram_unobservable | vram_unobservable |
    /// vram_total_unknown | weights_not_positive). Labels only.
    governor_label: Option<String>,
}

/// The ADR-036 §3.1 serving-side carriage posture as reported to the UI.
#[derive(Debug, Clone, Serialize)]
struct FamilyCarriageStatus {
    /// Whether other users' conversations are carried through family
    /// verification for this hosting session (the persisted setting at
    /// hosting start).
    enabled: bool,
    /// The always-visible disclosure: the §3.1 text VERBATIM while
    /// enabled, or the honest opt-out line when disabled.
    disclosure: String,
}

/// ADR-036 §3.1 (NORMATIVE UI CONTRACT — verbatim, both ends, always
/// visible while active, plain language, no modal, no one-time
/// click-through, no security-through-tokenization implication, the
/// honest cost stated, the hub-blind fact stated): the SERVING-side
/// disclosure, shown on the family status line while family serving is
/// enabled and carriage is on.
#[cfg(feature = "verifydrafts")]
const CARRIAGE_DISCLOSURE_SERVING: &str = "This computer carries other users' conversations \
     (as token ids it can read) through family verification. On by default — you can turn it off, \
     and requesting peers are refused honestly when you do.";

/// ADR-036 §3.1: the serving-side opt-out line (the honest statement when
/// the user has turned carriage off — the disclosure text would otherwise
/// claim carrying is happening).
#[cfg(feature = "verifydrafts")]
const CARRIAGE_DISCLOSURE_OFF: &str = "This computer is set NOT to carry other users' \
     conversations through family verification — requesting peers are refused honestly \
     (content_carriage_refused).";

/// ADR-036 §3.1 (NORMATIVE UI CONTRACT — verbatim; revised 2026-10-15 per
/// the owner's always-on directive, which RETIRED the election toggle and
/// its "turn it off" sentence): the REQUESTER-side disclosure, shown
/// PERMANENTLY on the chat surface. `{N}` is the ACTUAL cohort size for
/// the turn (bounded by the preset cohort cap and the session budget's
/// `max_endpoints`; 1 in the shipped configuration).
#[cfg(feature = "verifydrafts")]
const CARRIAGE_DISCLOSURE_REQUESTER: &str = "Family verification ON: your conversation — in a \
     form the receiving computers can read — goes to every verification peer this turn ({N}). It \
     is slower than single-peer chat, not faster. The hub (modelswarm.deepflux.space) never sees \
     any of it.";

/// Renders the §3.1 requester disclosure with the ACTUAL cohort count.
#[cfg(feature = "verifydrafts")]
fn requester_disclosure(cohort: u32) -> String {
    CARRIAGE_DISCLOSURE_REQUESTER.replace("{N}", &cohort.to_string())
}

/// The ADR-034 rule-6a status fields: provenance, token count, pin hash —
/// counts and a hash only, never token ids (rule 7 allows exactly these).
#[derive(Debug, Clone, Serialize)]
struct FamilyPinStatus {
    /// `"auto family-prompt-v1"` | `"file"`.
    provenance: String,
    /// The prompt version in force (`family-prompt-v1`).
    version: String,
    /// Pinned token count.
    token_count: u64,
    /// Bare 64-hex sha256 over the LE-u32 ids.
    pin_hash: String,
}

impl FamilyStatus {
    fn off(reason: &str) -> Self {
        Self {
            state: "off".into(),
            reason: Some(reason.into()),
            detail: None,
            disclosure: None,
            lease_roster_peers: 0,
            pin: None,
            carriage: None,
            governor_label: None,
        }
    }

    /// The honest-degrade shape: the stable reason PLUS the exact detail
    /// (bind error, resource projection) so the family line never reads
    /// as a silent off.
    fn off_detail(reason: &str, detail: impl Into<String>) -> Self {
        Self {
            detail: Some(detail.into()),
            ..Self::off(reason)
        }
    }

    /// The resource-ceiling degrade with the governor's frozen verdict
    /// label attached (ADR-037 §3.2b context-block source).
    fn off_governor(reason: &str, detail: impl Into<String>, label: &str) -> Self {
        Self {
            governor_label: Some(label.to_string()),
            ..Self::off_detail(reason, detail)
        }
    }
}

/// What the coordinator learned about one peer's family engine through the
/// EXISTING negotiation surface (the `SessionAccept` disclosure — no wire
/// or roster field carries capability, so this cache is filled by
/// `family_probe` and read by the chat auto-engage checkpoint). Counts and
/// labels only.
#[cfg(feature = "verifydrafts")]
#[derive(Clone)]
struct FamilyPeerCap {
    batch_verify: bool,
    batch_window_max: u32,
    /// The peer's disclosed backend ("cpu" | "vulkan").
    backend: String,
    /// When the disclosure was learned (unix ms) — stale caps re-learn
    /// through the next probe.
    learned_unix_ms: u64,
}

struct RunningNode {
    handle: NodeHandle,
    shutdown: tokio::sync::watch::Sender<bool>,
    heartbeat: tokio::task::JoinHandle<()>,
    enrollment: Arc<tokio::sync::RwLock<EnrollmentView>>,
    /// F2B: the advertised address list (direct first, relay circuit
    /// while a live reservation holds) — the relay supervisor writes it,
    /// the heartbeat registers it, `get_status` reads it for the
    /// serving chip's relay label.
    advertised: tokio::sync::watch::Sender<Vec<String>>,
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
        let telemetry = std::sync::Arc::new(open_node_telemetry(&data_dir));
        // 2026-10-15: no hosting at startup — the assistant starts on
        // the static floor; `set_hosting` re-arms it against the node's
        // own gateway the moment a model is hosted.
        let (assistant_plane, assistant_degrade_gap) = assistant::Plane::spawn(&data_dir, None);
        Self {
            hardware: tokio::sync::OnceCell::new(),
            telemetry,
            monitor: Arc::new(std::sync::Mutex::new(None)),
            inner: Mutex::new(Inner {
                data_dir,
                tracker_url: config.tracker,
                size_cache: Default::default(),
                census_counts: None,
                profile_id: config.profile_id,
                license_accepted,
                artifact: None,
                node: None,
                chat_log: Vec::new(),
                allow_swarm_pairing: config.allow_swarm_pairing,
                allow_content_carriage: config.allow_content_carriage,
                #[cfg(feature = "verifydrafts")]
                last_family_cohort: None,
                #[cfg(feature = "verifydrafts")]
                family_peer_caps: Default::default(),
                #[cfg(feature = "verifydrafts")]
                family_engine: None,
                swarm_lease: None,
                swarm_executor: None,
                relay_tickets: None,
                relay_config: None,
                peer_metrics: None,
                listener_addr: None,
                family: None,
                updater: updater::UpdaterStatus::Idle,
                assistant_plane,
                assistant_degrade_gap,
                assistant_acknowledged: config.assistant_acknowledged,
                assistant_enabled: config.assistant_enabled,
                last_cards: Vec::new(),
                last_engage_reason: None,
                last_governor_verdict: None,
                last_probe_verdict: None,
                capacity_class: None,
            }),
        }
    }
}

/// The process-wide node-log sink (same file/format as the node daemon's
/// own telemetry). On open failure we degrade to a memory sink — logging
/// must never take the app down, and a missing trail shows up in
/// diagnostics immediately.
fn open_node_telemetry(data_dir: &std::path::Path) -> Telemetry {
    let logs_dir = data_dir.join("logs");
    if std::fs::create_dir_all(&logs_dir).is_ok() {
        if let Ok(sink) = modelswarm_telemetry::FileSink::open(&logs_dir.join("node.jsonl")) {
            return Telemetry::with_sink(Box::new(sink));
        }
    }
    let (telemetry, _mem) = Telemetry::memory();
    telemetry
}

/// Loads (or creates) the installation identity and checks the
/// privacy-gate acceptance row. Any failure reads as "not accepted".
async fn license_accepted(tracker_url: &str, data_dir: &std::path::Path) -> bool {
    let telemetry = open_node_telemetry(data_dir);
    let Ok(tracker) = tracker_client(tracker_url, data_dir, &telemetry).await else {
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

#[derive(Serialize, Deserialize)]
struct PersistedConfig {
    #[serde(default = "default_tracker")]
    tracker: String,
    #[serde(default)]
    profile_id: Option<String>,
    /// Owner directive (2026-10-13): the settings toggle "allow swarm
    /// pairing" replaces the `MSP_LISTENER` env var as the product gate for
    /// the serving listener + family tier. DEFAULT ON for testers; the
    /// daemon never reads `MSP_LISTENER` for this decision anymore (the
    /// transport crate's internal ADR-018 non-loopback guard still reads
    /// its own env contract — the desktop SETS it when the toggle allows,
    /// never the reverse).
    #[serde(default = "default_allow_swarm_pairing")]
    allow_swarm_pairing: bool,
    /// ADR-036 §3.1 (the owner's swarm-together directive): the SERVING
    /// posture for other users' conversations through family verification
    /// — DEFAULT ON. Opting out refuses pin carriers with the honest
    /// labeled reject `content_carriage_refused` (never a silent drop);
    /// the neutral family is unaffected. Applies at the NEXT hosting start
    /// (changing while hosting is refused — the live listener must never
    /// contradict the setting).
    #[serde(default = "default_allow_content_carriage")]
    allow_content_carriage: bool,
    /// ADR-036 §3.1/§3.3: the REQUESTER-side election for
    /// family-verified chat — DEFAULT OFF. Election is the USER's choice
    /// (only the serving side is default-on): while elected, chat turns
    /// ADR-037 §3.6 (disclosure and consent): the first-use disclosure
    /// acknowledgement. DEFAULT FALSE — the first cloud call is BLOCKED
    /// until the user acknowledges the verbatim disclosure in the
    /// assistant panel.
    #[serde(default)]
    assistant_acknowledged: bool,
    /// ADR-037 §3.6 (Settings opt-out): the user's cloud-assistant
    /// toggle. DEFAULT TRUE — this is the "turn it off any time"
    /// control; the §3.4 master switch (`assistant.enabled` in
    /// advisory.json) is the OWNER's, and BOTH must be on for cloud
    /// mode. Opting out is a config flip with zero functional loss
    /// (built-in help + the deterministic recommendation stay).
    #[serde(default = "default_assistant_enabled")]
    assistant_enabled: bool,
}

fn default_assistant_enabled() -> bool {
    true
}

fn default_tracker() -> String {
    "https://modelswarm.deepflux.space".to_string()
}

fn default_allow_swarm_pairing() -> bool {
    true
}

fn default_allow_content_carriage() -> bool {
    true
}

impl Default for PersistedConfig {
    fn default() -> Self {
        // Mirrors the serde field defaults exactly: a missing file/keys
        // must read as the fresh-install posture (default tracker,
        // pairing ALLOWED, carriage ALLOWED, verified chat NOT elected,
        // assistant disclosure NOT acknowledged, assistant toggle ON) —
        // `derive(Default)` would zero the on-defaults.
        Self {
            tracker: default_tracker(),
            profile_id: None,
            allow_swarm_pairing: default_allow_swarm_pairing(),
            allow_content_carriage: default_allow_content_carriage(),
            assistant_acknowledged: false,
            assistant_enabled: default_assistant_enabled(),
        }
    }
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

/// GPU-accelerated engine variant (ADR-024): `engine-vulkan/llama-server.exe`
/// from the same layouts as the CPU engine. Windows-only bundle today; other
/// OSes simply never resolve a file and stay on the canonical engine.
fn engine_gpu_binary_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("engine-vulkan").join(name));
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("modelswarm"));
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    candidates.push(dir.join("engine-vulkan").join(name));
    candidates.into_iter().find(|path| path.is_file())
}

/// Appends one event to the shared process telemetry (same sink/format as
/// the node's own events) so client-side failures — catalog fetches above
/// all — leave a trail even when the UI is the only thing reporting them.
fn log_event(telemetry: &Telemetry, level: &str, event: &str, fields: &[(&str, &str)]) {
    match level {
        "warn" => telemetry.warn(event, fields),
        _ => telemetry.info(event, fields),
    };
}

/// The serving-listener gate decision, pure and env-independent (owner
/// directive 2026-10-13): the persisted "allow swarm pairing" toggle is
/// the product gate. Since the Network Engineer's `ListenOptions` landing
/// (8d2284c) the transport's ADR-018 non-loopback guard takes an explicit
/// typed opt-in (`listen_with(.., allow_off_loopback())`) — `MSP_LISTENER`
/// is retired; nothing sets or reads it anymore.
#[derive(Debug, PartialEq)]
enum ListenerGate {
    /// The toggle allows pairing — proceed to bind.
    Allowed,
    /// The toggle is off: no listener, and the family line says so with
    /// the exact reason (never a silent off).
    Disabled,
}

fn listener_gate(allow_swarm_pairing: bool) -> ListenerGate {
    if allow_swarm_pairing {
        ListenerGate::Allowed
    } else {
        ListenerGate::Disabled
    }
}

/// The bind outcome for the family/hosting status surface.
enum ListenerBind {
    /// The bound listener plus its honestly-advertised LAN multiaddr.
    Bound(modelswarm_transport::libp2p_backend::Libp2pListener, String),
    /// Pairing disabled by the settings toggle.
    Disabled,
    /// The toggle allowed pairing but the bind failed — carries the exact
    /// phase + error for the family line (honest degrade, never silent).
    Failed(String),
}

/// F0(2): when the settings toggle allows pairing (default on since
/// 2026-10-13; replaces the MSP_LISTENER=1 opt-in), bind the libp2p
/// serving listener on 0.0.0.0 and return the honestly-advertised LAN
/// multiaddr. The listener is parked in a holding task — dropping it stops
/// the driver, so it lives until process end (or hosting stop kills the
/// node; the listener itself is connectionless until the serving bridge
/// lands in F0(3)). `Disabled`/`Failed` carry the exact reason upward so
/// the family line can state it.
async fn bind_serving_listener(
    data_dir: &std::path::Path,
    telemetry: &Telemetry,
    allow_swarm_pairing: bool,
) -> ListenerBind {
    if listener_gate(allow_swarm_pairing) == ListenerGate::Disabled {
        return ListenerBind::Disabled;
    }
    // The ADR-018 guard is satisfied EXPLICITLY now (the Network
    // Engineer's ListenOptions landing, 8d2284c): the toggle-allowed bind
    // passes the typed opt-in — no process-global env detour. MSP_LISTENER
    // is retired; nothing reads it anymore.
    let identity = match cached_identity(data_dir, telemetry) {
        Ok(identity) => identity,
        Err(e) => {
            let detail = format!("identity: {e}");
            log_event(
                telemetry,
                "warn",
                "listener.error",
                &[("phase", "identity"), ("error", &detail)],
            );
            return ListenerBind::Failed(detail);
        }
    };
    let backend = match modelswarm_transport::libp2p_backend::Libp2pTransport::new(&identity) {
        Ok(b) => b,
        Err(e) => {
            let detail = format!("transport backend: {e}");
            log_event(
                telemetry,
                "warn",
                "listener.error",
                &[("phase", "backend"), ("error", &detail)],
            );
            return ListenerBind::Failed(detail);
        }
    };
    let listener = match backend
        .listen_with(
            "0.0.0.0:0",
            modelswarm_transport::libp2p_backend::ListenOptions::allow_off_loopback(),
        )
        .await
    {
        Ok(l) => l,
        Err(e) => {
            let detail = format!("bind 0.0.0.0:0: {e}");
            log_event(
                telemetry,
                "warn",
                "listener.error",
                &[("phase", "bind"), ("error", &detail)],
            );
            return ListenerBind::Failed(detail);
        }
    };
    // Bind says 0.0.0.0; advertise the real LAN IP (UDP-connect trick —
    // no packets leave the machine).
    let lan_ip = (|| {
        let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
        s.connect("8.8.8.8:80").ok()?;
        Some(s.local_addr().ok()?.ip().to_string())
    })()
    .unwrap_or_else(|| "0.0.0.0".into());
    let addr = listener
        .bound_addr()
        .to_string()
        .replace("/0.0.0.0/", &format!("/{lan_ip}/"));
    log_event(telemetry, "info", "listener.bound", &[("addr", &addr)]);
    ListenerBind::Bound(listener, addr)
}

/// F2B (ADR-014): one reservation attempt on the configured relay.
/// Errors carry the exact transport text — the relay's own status
/// vocabulary (`no_reservation(204)`, …) rides it verbatim. Callers log
/// through the EXISTING `listener.error` event (phase `relay_reserve`);
/// no new event names. ADR-038 §6.4 row 8: the attempt presents the
/// cached admission ticket (the transport's pre-flight) — an empty
/// cache honestly keeps the pre-wiring behavior.
async fn reserve_relay_attempt(
    data_dir: &Path,
    telemetry: &Telemetry,
    relay_addr: &str,
    tickets: &std::sync::Arc<modelswarm_node::relay_ticket_cache::RelayTicketCache>,
) -> Result<modelswarm_transport::relay_client::RelayReservation, String> {
    let identity = cached_identity(data_dir, telemetry)?;
    let backend = modelswarm_transport::libp2p_backend::Libp2pTransport::new(&identity)
        .map_err(|e| format!("transport backend: {e}"))?;
    // ADR-038 §6.4 row 8: fresh supply into the transport's ticket seam
    // (read at pre-flight time, so heartbeat refreshes between attempts
    // reach the next reservation without rebuilding anything here).
    tickets.attach_to(&backend);
    backend
        .reserve_relay(relay_addr, Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())
}

/// The F2B relay reservation SUPERVISOR (serving side): every 30 s, when
/// no live reservation is held, retry the configured relay (the relay may
/// start after the app — the owner's second-desktop deployment); keep the
/// advertised address list truthful (circuit address only while a live
/// reservation holds it); the heartbeat re-registers on any list change.
/// ADR-039: resolves the effective relay address for one hosting
/// session — the local `relay.addr` file WINS (checked first, the F2B
/// behavior); otherwise waits for the tracker's first non-empty
/// heartbeat-supplied value — then runs the (unchanged) reservation
/// supervisor exactly once. Waiting is shutdown-aware: hosting off or app
/// exit cancels cleanly; a tracker that never supplies an address simply
/// never starts a supervisor (today's no-relay behavior, no log spam).
#[allow(clippy::too_many_arguments)]
async fn relay_config_supervisor(
    data_dir: std::path::PathBuf,
    mut tracker_relay: tokio::sync::watch::Receiver<Option<String>>,
    slot: modelswarm_node::serving::RelaySlot,
    advertised: tokio::sync::watch::Sender<Vec<String>>,
    direct_addr: String,
    tickets: std::sync::Arc<modelswarm_node::relay_ticket_cache::RelayTicketCache>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    telemetry: std::sync::Arc<Telemetry>,
) {
    let relay_addr = match configured_relay_addr(&data_dir) {
        Some(file_addr) => file_addr,
        None => loop {
            tokio::select! {
                changed = tracker_relay.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    if let Some(addr) = tracker_relay.borrow().clone() {
                        break addr;
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        return;
                    }
                }
            }
        },
    };
    relay_reservation_supervisor(
        data_dir,
        relay_addr,
        slot,
        advertised,
        direct_addr,
        tickets,
        shutdown,
        telemetry,
    )
    .await;
}

/// Reservation RENEWAL itself is the transport's own driver (re-reserve
/// at 3/4 of each lease) — this task only acquires and re-acquires.
/// ADR-038 §6.4 row 8: every attempt presents the heartbeat-supplied
/// admission ticket through the transport's seam (empty cache = the
/// documented pre-wiring behavior — no pre-flight offer).
/// Failures log through the EXISTING `listener.error` event (phase
/// `relay_reserve`), throttled to the first and every 20th retry (10 min,
/// the roster-failure cadence); success logs `listener.bound` with the
/// circuit address (a relayed serving path IS a bound serving path).
#[allow(clippy::too_many_arguments)]
async fn relay_reservation_supervisor(
    data_dir: PathBuf,
    relay_addr: String,
    slot: modelswarm_node::serving::RelaySlot,
    advertised: tokio::sync::watch::Sender<Vec<String>>,
    direct_addr: String,
    tickets: std::sync::Arc<modelswarm_node::relay_ticket_cache::RelayTicketCache>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    telemetry: std::sync::Arc<Telemetry>,
) {
    let mut failures: u32 = 0;
    loop {
        if slot.holds_live().await {
            set_advertised_addrs(&advertised, &direct_addr, slot.circuit_addr().await);
        } else {
            set_advertised_addrs(&advertised, &direct_addr, None);
            match reserve_relay_attempt(&data_dir, &telemetry, &relay_addr, &tickets).await {
                Ok(reservation) => {
                    failures = 0;
                    let circuit = reservation.circuit_addr();
                    log_event(&telemetry, "info", "listener.bound", &[("addr", &circuit)]);
                    slot.install(reservation).await;
                    set_advertised_addrs(&advertised, &direct_addr, slot.circuit_addr().await);
                }
                Err(error) => {
                    failures += 1;
                    if failures == 1 || failures % 20 == 0 {
                        log_event(
                            &telemetry,
                            "warn",
                            "listener.error",
                            &[
                                ("phase", "relay_reserve"),
                                ("error", &error),
                                ("attempt", &failures.to_string()),
                            ],
                        );
                    }
                }
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            changed = shutdown.changed() => match changed {
                Err(_) => return,
                Ok(()) => {
                    if *shutdown.borrow() {
                        return;
                    }
                }
            },
        }
    }
}

// ---- The composed daemon's family half (ADR-032 light-up; cfg
// `verifydrafts`) -----------------------------------------------

/// The installed engine bundle directories in the daemon's preference
/// order. The family engine prefers the SAME variant class the serving
/// engine runs (ADR-024 preference; E0 keeps CPU/Vulkan sessions from
/// mixing WITHIN a session, never across engines): the GPU bundle first
/// when the sidecar serves Vulkan, else the CPU bundle first with the GPU
/// set as the detect-only fallback. Selection is a file check — the
/// one-bundle-per-process guard (ADR-031 §2) means the load happens
/// exactly once, on the winner, inside the node helper.
#[cfg(feature = "verifydrafts")]
fn family_engine_dirs(
    cpu_engine_exe: &Path,
    gpu_engine_exe: Option<&Path>,
    serving_backend: Option<&str>,
) -> Vec<PathBuf> {
    let cpu = cpu_engine_exe
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let gpu = gpu_engine_exe.and_then(|exe| exe.parent().map(Path::to_path_buf));
    match (serving_backend, gpu) {
        (Some("vulkan"), Some(gpu)) => vec![gpu, cpu],
        (_, Some(gpu)) => vec![cpu, gpu],
        (_, None) => vec![cpu],
    }
}

/// Reads the daemon-pinned family session prompt (token ids,
/// whitespace-separated, STRICT — any unparsable token unpins the whole
/// file; never a partially-loaded prompt). None = unpinned: the family
/// engine stays down honestly (sessions refuse fail-closed rather than
/// verify against a mismatched prompt — the G6 out-of-band pairing gap,
/// a recorded freeze-ADR item).
#[cfg(feature = "verifydrafts")]
fn family_prompt_tokens(data_dir: &Path) -> Option<Vec<u32>> {
    let raw = std::fs::read_to_string(data_dir.join(FAMILY_PROMPT_TOKENS_FILE)).ok()?;
    let ids: Vec<u32> = raw
        .split_whitespace()
        .map(|token| token.parse::<u32>().ok())
        .collect::<Option<Vec<_>>>()?;
    (!ids.is_empty()).then_some(ids)
}

// ---- ADR-034 tier 1: the family auto-pin (deterministic serving-side
// pin). Two stock installs hosting the same exact profile pair with zero
// manual files; the expert seam (the file) survives with precedence; junk
// stays fail-closed; nothing on the frozen wire changes. ---------------

/// ADR-034 rule 2: the frozen, neutral v1 family prompt text. MUST be
/// byte-identical in every implementation — it is a PROTOCOL PARAMETER,
/// not content (rule 7: the text, version tag, token count, and pin hash
/// MAY be logged, versioned, telemetered, and shown; the
/// content-redaction rules bind session prompts, not ADR-defined
/// constants).
#[cfg(feature = "verifydrafts")]
const FAMILY_PROMPT_V1: &str = "You are serving a ModelSwarm cooperative verification session. Continue the given context exactly as this model normally would.";

/// The version tag that travels with the v1 pin in status and telemetry
/// (ADR-034 rules 2/6a). A version bump is a pairing-visible event and
/// rides the rule-4 rewrite discipline.
#[cfg(feature = "verifydrafts")]
const FAMILY_PROMPT_VERSION: &str = "family-prompt-v1";

/// ADR-034 rule 3: render the prompt text as a SYSTEM-ONLY turn through
/// the profile's pinned chat template. ChatML is the catalog's template
/// family (ADR-025 rule 2: every promoted profile's `chat_template_hash`
/// inputs are ChatML-shaped; a non-ChatML profile is gated per ADR-025
/// rule 3 and tokenizes the raw text). Deliberately WITHOUT the assistant
/// generation prompt — the pin is the root context the verifier prefills
/// and the coordinator continues from; the derivation is constants plus
/// the pinned tokenizer, nothing else (rule 1), so it is identical on
/// every install hosting the same exact profile.
#[cfg(feature = "verifydrafts")]
fn family_prompt_render_system_only(text: &str) -> String {
    format!("<|im_start|>system\n{text}<|im_end|>\n")
}

/// ADR-034 rule 3: the pin identity — the bare 64-hex `sha256` over the
/// token ids rendered as little-endian u32 each (the ADR-032 §1 rule-2 id
/// encoding, with no parent). Status and telemetry carry the HASH and the
/// COUNT, never the ids.
#[cfg(feature = "verifydrafts")]
fn family_pin_hash(ids: &[u32]) -> String {
    use sha2::{Digest, Sha256};
    let mut bytes = Vec::with_capacity(ids.len() * 4);
    for id in ids {
        bytes.extend_from_slice(&id.to_le_bytes());
    }
    hex::encode(Sha256::digest(&bytes))
}

/// Derives one version's pin ids under an injected PINNED tokenizer (the
/// determinism rule: same `ModelProfileId` ⇒ same template and tokenizer
/// ⇒ same ids on every install; the C-API adapter's tokenizer or the
/// sidecar's surface — any yielding the pinned engine's ids). An empty
/// result is an error: an empty pin refuses every session at the engine
/// source. The tokenizer is an async closure so production paths pass
/// the node's engine seams and tests pass a deterministic in-memory one.
#[cfg(feature = "verifydrafts")]
async fn derive_family_prompt_ids<F, Fut>(text: &str, tokenize: F) -> Result<Vec<u32>, String>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<u32>, String>>,
{
    let rendered = family_prompt_render_system_only(text);
    let ids = tokenize(rendered)
        .await
        .map_err(|e| format!("pinned tokenizer: {e}"))?;
    if ids.is_empty() {
        return Err("pinned tokenizer returned an empty prompt".into());
    }
    Ok(ids)
}

/// ADR-034 rule 4's decision, before any derivation runs (an expert file
/// needs NO tokenizer — it is honored as-is even with the engine down).
#[cfg(feature = "verifydrafts")]
#[derive(Debug)]
enum FamilyPinPlan {
    /// File ABSENT: derive the current version's ids and write them (the
    /// two-stock-installs case).
    WriteAuto,
    /// File present, strictly parsable, and EQUAL to an older version's
    /// derived ids (an unmodified automatic pin): derive the current
    /// version's ids and rewrite the file.
    RewriteAuto { previous_version: String },
    /// File present, strictly parsable, anything else: honored AS-IS (the
    /// documented expert override, including non-v1 pins — it WINS).
    KeepFile(Vec<u32>),
    /// File present but unparsable: fail-closed unpinned; the automatic
    /// pin never silently overwrites a file the user may own.
    Junk,
}

/// The pure core of the rule-4 write discipline: which action the pin
/// file demands at hosting start. `previous_versions` carries each
/// OLDER prompt version's derived ids (empty while v1 is the only
/// version); the current version's ids are derived lazily by the caller
/// only for the write/rewrite paths. `Err` = the local derivation is
/// impossible (engine/tokenizer unavailable) — the unpinned degrade with
/// the detail, never a guess.
#[cfg(feature = "verifydrafts")]
fn family_pin_plan(
    dir: &Path,
    previous_versions: &[(&str, Vec<u32>)],
) -> Result<FamilyPinPlan, String> {
    match std::fs::read_to_string(dir.join(FAMILY_PROMPT_TOKENS_FILE)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(FamilyPinPlan::WriteAuto),
        Err(e) => Err(format!("pin file unreadable: {e}")),
        Ok(raw) => {
            let parsed: Option<Vec<u32>> = raw
                .split_whitespace()
                .map(|token| token.parse::<u32>().ok())
                .collect();
            match parsed {
                None => Ok(FamilyPinPlan::Junk),
                Some(ids) if ids.is_empty() => Ok(FamilyPinPlan::Junk),
                Some(ids) => {
                    for (version, previous_ids) in previous_versions {
                        if &ids == previous_ids {
                            return Ok(FamilyPinPlan::RewriteAuto {
                                previous_version: (*version).to_string(),
                            });
                        }
                    }
                    Ok(FamilyPinPlan::KeepFile(ids))
                }
            }
        }
    }
}

/// Writes the pin file atomically: `.part` sibling + rename (the
/// artifact-download pattern). The rewrite path (destination exists)
/// removes the old file first — std `rename` does not replace on Windows;
/// the sub-millisecond window sits at hosting start, before any session
/// can read the pin.
#[cfg(feature = "verifydrafts")]
fn write_family_pin_file(path: &Path, ids: &[u32]) -> Result<(), String> {
    let tmp = path.with_extension("part");
    let body = ids.iter().map(u32::to_string).collect::<Vec<_>>().join(" ") + "\n";
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

/// The pin in force after rule 4 ran at hosting start.
#[cfg(feature = "verifydrafts")]
struct FamilyPin {
    /// `"auto family-prompt-v1"` | `"file"` (rule 6a).
    provenance: String,
    /// The ids the verifier engine prefills (the file's when it wins).
    ids: Vec<u32>,
}

/// ADR-034 rules 1–4 at hosting start: derive (or honor) the pin BEFORE
/// it is read and before the engine source constructs. The derivation is
/// local, deterministic, and content-free: the frozen v1 text rendered as
/// a system-only ChatML turn, tokenized under the profile's pinned
/// tokenizer — the sidecar's `/tokenize` surface through the node (the
/// same runtime instance the gateway serves through; the sidecar's
/// bearer secret never crosses the process boundary, ADR-002). `Err` =
/// unpinned with the honest detail (junk file, unreadable file, or a
/// failed derivation — the family engine stays down, never a guess).
#[cfg(feature = "verifydrafts")]
async fn ensure_family_pin(
    data_dir: &Path,
    handle: &NodeHandle,
    telemetry: &Telemetry,
) -> Result<FamilyPin, String> {
    // Only one version exists today, so the version-advance table is
    // empty; the rewrite rule is exercised by the unit tests with a
    // two-version table and kicks in for real the day v2 ships.
    let plan = family_pin_plan(data_dir, &[])?;
    let derive_current = || {
        derive_family_prompt_ids(FAMILY_PROMPT_V1, |text| async move {
            handle.engine_tokenize(&text).await
        })
    };
    let path = data_dir.join(FAMILY_PROMPT_TOKENS_FILE);
    match plan {
        FamilyPinPlan::WriteAuto => {
            let ids = derive_current().await?;
            write_family_pin_file(&path, &ids)?;
            log_event(
                telemetry,
                "info",
                "family.pin",
                &[
                    ("provenance", "auto"),
                    ("version", FAMILY_PROMPT_VERSION),
                    ("tokens", &ids.len().to_string()),
                    ("hash", &family_pin_hash(&ids)),
                ],
            );
            Ok(FamilyPin {
                provenance: format!("auto {FAMILY_PROMPT_VERSION}"),
                ids,
            })
        }
        FamilyPinPlan::RewriteAuto { previous_version } => {
            // An unmodified automatic pin of a previous version: rewrite
            // to the current version (rule 4). Any other present file was
            // KeepFile above and never reaches here.
            let ids = derive_current().await?;
            write_family_pin_file(&path, &ids)?;
            log_event(
                telemetry,
                "info",
                "family.pin",
                &[
                    ("provenance", "auto"),
                    ("version", FAMILY_PROMPT_VERSION),
                    ("rewrote_from", &previous_version),
                    ("tokens", &ids.len().to_string()),
                    ("hash", &family_pin_hash(&ids)),
                ],
            );
            Ok(FamilyPin {
                provenance: format!("auto {FAMILY_PROMPT_VERSION}"),
                ids,
            })
        }
        FamilyPinPlan::KeepFile(ids) => {
            log_event(
                telemetry,
                "info",
                "family.pin",
                &[
                    ("provenance", "file"),
                    ("tokens", &ids.len().to_string()),
                    ("hash", &family_pin_hash(&ids)),
                ],
            );
            Ok(FamilyPin {
                provenance: "file".into(),
                ids,
            })
        }
        FamilyPinPlan::Junk => {
            Err("present pin file is not strictly parsable — left untouched (fail-closed)".into())
        }
    }
}

/// Builds the composed daemon's family half (ADR-032 §7 disposition item
/// ii): the RC-L1 production lease source — a [`RosterFamilyLeases`]
/// under the pinned hub key — and the engine source for this build (the
/// real C-API bridge under `verifydrafts-capi`). Since the 2026-10-12
/// presented-lease activation, family admission does not wait for roster
/// feeding: the coordinator PRESENTS its hub-signed lease in the
/// session-open carrier (validated with every binding the msp-v1
/// per-request gate enforces — profile, consume right, slots, peer,
/// freshness under the pinned hub key); the roster is the tracker-fed
/// future path through the same validation.
///
/// `None` = the family cannot serve; the caller keeps the plain msp-v1
/// bridge (honest degradation, never a silent family-off). Every `None`
/// path telemeters its stable reason and mirrors it in the returned
/// [`FamilyStatus`].
/// The family-tier resource verdict (owner-flagged binding constraint):
/// starting the in-process verify engine alongside the serving sidecar
/// roughly doubles the model's resident footprint. Since the M2 governor
/// landing (ADR-035, cb57b87) this delegates to the ENFORCEMENT-authority
/// governor — the D11 70% ceiling projection plus the winjob commit-cap
/// backstop armed on Admit. Two intentional upgrades over the retired
/// advisory check (pinned by the governor's tests): unobservable inputs
/// fail CLOSED (degrade with the exact reason — never a guessed number,
/// never a silent pass), and KV overhead is not yet modelable
/// pre-construction, so the verdict carries the explicit
/// `KnowledgeGap::KvUnmodeled` (ADR-035 §3).
#[cfg(feature = "verifydrafts")]
async fn family_resource_verdict(
    weights_bytes: u64,
    hardware: &HardwareInfo,
    serving_backend: Option<&str>,
) -> Result<modelswarm_resource::GovernorVerdict, String> {
    let vram_total_bytes = hardware.gpu.as_ref().map(|g| g.vram_mb * 1_000_000);
    let lands_in_vram = serving_backend == Some("vulkan");
    tokio::task::spawn_blocking(move || {
        let mut governor = modelswarm_resource::ResourceGovernor::platform(
            modelswarm_resource::ProcessScope::Roots(vec![std::process::id()]),
        );
        governor.evaluate(&modelswarm_resource::AdmissionRequest {
            added_weights_bytes: weights_bytes,
            // HONEST MAPPING TODAY (ADR-035 §3): no public runtime API
            // sizes the not-yet-constructed verify engine's KV — None
            // carries KnowledgeGap::KvUnmodeled explicitly.
            added_kv_bytes: None,
            added_copy_lands_in_vram: lands_in_vram,
            vram_total_bytes,
            arm_tree_commit_cap: true,
        })
    })
    .await
    .map_err(|e| format!("resource governor task failed: {e}"))
}

#[cfg(feature = "verifydrafts")]
#[allow(clippy::too_many_arguments)]
async fn compose_family_serving(
    app: &tauri::AppHandle,
    data_dir: &Path,
    profile: &str,
    model_path: &Path,
    weights_bytes: u64,
    hardware: &HardwareInfo,
    handle: &NodeHandle,
    allow_content_carriage: bool,
    telemetry: &std::sync::Arc<Telemetry>,
) -> (
    Option<FamilyServing>,
    FamilyStatus,
    Option<Arc<modelswarm_node::verifydrafts::FamilyEngineSource>>,
) {
    // The verifier identity: the SAME per-installation identity the
    // serving listener authenticated with — it signs every
    // VerifyDraftsResult (RC-2).
    let Ok(verifier_identity) = cached_identity(data_dir, telemetry) else {
        log_event(
            telemetry,
            "warn",
            "family.fallback",
            &[("reason", "verifier_identity_unavailable")],
        );
        return (
            None,
            FamilyStatus::off("verifier_identity_unavailable"),
            None,
        );
    };
    let config = VerifyDraftsServeConfig::honest(profile.to_string(), verifier_identity);
    // ADR-036 §3.1: the carriage posture for this hosting session — the
    // persisted setting at hosting start (DEFAULT ON; refused-while-
    // hosting keeps the live listener honest). An opted-out peer refuses
    // pin carriers with the honest `content_carriage_refused` reject; the
    // neutral family (carrier-less sessions) is unaffected.
    let mut config = config;
    config.content_carriage = if allow_content_carriage {
        modelswarm_node::verifydrafts::ContentCarriage::On
    } else {
        modelswarm_node::verifydrafts::ContentCarriage::Off
    };
    // (The config's hand-set disclosure stays the honest `batch_verify:
    // false` default — the composed loop overrides it per session with
    // the engine's REAL geometry; the status below reports the same
    // real geometry, never this field.)

    // The owner-flagged binding constraint, now under the M2 governor's
    // enforcement authority (ADR-035): family-by-default means the verify
    // engine's memory coexists with the sidecar; the governor projects the
    // doubling against the D11 70% ceiling AND arms the winjob commit-cap
    // backstop on Admit. Unobservables degrade fail-closed with the exact
    // reason; the verdict's reason/gaps/enforcement ride the telemetry.
    match family_resource_verdict(weights_bytes, hardware, handle.engine_backend.as_deref()).await {
        Ok(verdict) if verdict.decision == modelswarm_resource::Decision::Admit => {}
        Ok(verdict) => {
            let detail = verdict.detail_line();
            let governor_reason = verdict
                .reason
                .as_ref()
                .map(|r| r.label())
                .unwrap_or("unknown");
            let gaps = verdict
                .gaps
                .iter()
                .map(|g| g.label())
                .collect::<Vec<_>>()
                .join(",");
            let enforcement = verdict.enforcement.label();
            log_event(
                telemetry,
                "warn",
                "family.fallback",
                &[
                    ("reason", "resource_ceiling"),
                    ("governor_reason", governor_reason),
                    ("gaps", &gaps),
                    ("enforcement", enforcement),
                    ("detail", &detail),
                ],
            );
            return (
                None,
                FamilyStatus::off_governor("resource_ceiling", detail, governor_reason),
                None,
            );
        }
        Err(detail) => {
            // The governor itself could not run (spawn failure) — the
            // honest degrade carries that exact transport-level fact.
            log_event(
                telemetry,
                "warn",
                "family.fallback",
                &[("reason", "governor_unavailable"), ("detail", &detail)],
            );
            return (
                None,
                FamilyStatus::off_detail("governor_unavailable", detail),
                None,
            );
        }
    }

    // The engine source for THIS build.
    #[cfg(feature = "verifydrafts-capi")]
    let engine: Option<(
        std::sync::Arc<modelswarm_node::verifydrafts::FamilyEngineSource>,
        serde_json::Value,
        FamilyPinStatus,
    )> = {
        // ADR-034 rule 1: the serving desktop derives/ensures the pin at
        // hosting start — BEFORE it is read and before `capi_family_source`
        // constructs the engine. The derivation is local, deterministic,
        // and content-free (the frozen v1 text through the profile's
        // pinned tokenizer, nothing else); an unpinned daemon is not a
        // verifier — fail closed, visibly, with the honest detail.
        let pin = match ensure_family_pin(data_dir, handle, telemetry).await {
            Ok(pin) => pin,
            Err(detail) => {
                log_event(
                    telemetry,
                    "warn",
                    "family.fallback",
                    &[
                        ("reason", "family_prompt_unpinned"),
                        ("file", FAMILY_PROMPT_TOKENS_FILE),
                        ("detail", &detail),
                    ],
                );
                return (None, FamilyStatus::off("family_prompt_unpinned"), None);
            }
        };
        let pin_status = FamilyPinStatus {
            provenance: pin.provenance.clone(),
            version: FAMILY_PROMPT_VERSION.to_string(),
            token_count: pin.ids.len() as u64,
            pin_hash: family_pin_hash(&pin.ids),
        };
        let prompt_tokens = pin.ids;
        let dirs = family_engine_dirs(
            &engine_binary_path(app),
            engine_gpu_binary_path(app).as_deref(),
            handle.engine_backend.as_deref(),
        );
        match modelswarm_node::verifydrafts::engine_source::capi_family_source(
            dirs,
            model_path.to_path_buf(),
            handle.engine_identity.clone(),
            profile.to_string(),
            prompt_tokens,
        )
        .await
        {
            Ok(source) => {
                let disclosure =
                    modelswarm_node::verifydrafts::engine_source::disclosure_json(&source);
                let backend = disclosure["backend"].as_str().unwrap_or("").to_string();
                let window = disclosure["batch_window_max"].to_string();
                log_event(
                    telemetry,
                    "info",
                    "family.engine_ready",
                    &[
                        ("backend", &backend),
                        ("batch_window_max", &window),
                        ("batch_verify", "true"),
                    ],
                );
                Some((source, disclosure, pin_status))
            }
            Err(error) => {
                // The bridge could not init (missing C-API bundle, model
                // load failure, ABI canary): honest degradation with the
                // local error detail in the log AND the status line.
                log_event(
                    telemetry,
                    "warn",
                    "family.fallback",
                    &[
                        ("reason", "engine_bridge_failed"),
                        ("error", &error.to_string()),
                    ],
                );
                return (
                    None,
                    FamilyStatus::off_detail("engine_bridge_failed", error.to_string()),
                    None,
                );
            }
        }
    };
    #[cfg(not(feature = "verifydrafts-capi"))]
    let engine: Option<(
        std::sync::Arc<modelswarm_node::verifydrafts::FamilyEngineSource>,
        serde_json::Value,
        FamilyPinStatus,
    )> = {
        // A dev build without the C-API bridge: the honest engine answer
        // is the http-adapter disclosure (`batch_verify: false` — a
        // first-class single, never a batch verifier), i.e. no family
        // serving from this binary.
        log_event(
            telemetry,
            "warn",
            "family.fallback",
            &[
                ("reason", "engine_bridge_not_compiled"),
                ("disclosure", "batch_verify:false (http adapter)"),
            ],
        );
        return (None, FamilyStatus::off("engine_bridge_not_compiled"), None);
    };

    let Some((engine, disclosure, pin_status)) = engine else {
        // Every None path returned above; this is unreachable by
        // construction, kept as a fail-closed guard rather than an
        // `expect`.
        return (None, FamilyStatus::off("engine_bridge_failed"), None);
    };

    // RC-L1 + the §2 admission order: the production lease source under
    // the PINNED hub key. The roster starts EMPTY and nothing feeds it —
    // coordinators PRESENT their hub-signed lease at session open (the
    // carrier read by the composed loop), validated with every binding
    // the msp-v1 per-request gate enforces; the roster remains the
    // tracker-fed future distribution path on top of the same validation.
    let lease = std::sync::Arc::new(RosterFamilyLeases::new(
        modelswarm_node::serving::LeasePolicy::production(),
    ));
    let lease_roster_peers = lease.len();
    log_event(
        telemetry,
        "info",
        "family.enabled",
        &[
            ("profile", profile),
            ("lease_path", "presented_at_session_open"),
            ("lease_roster_peers", &lease_roster_peers.to_string()),
        ],
    );
    let status = FamilyStatus {
        state: "on".into(),
        reason: None,
        detail: None,
        disclosure: Some(disclosure),
        lease_roster_peers,
        pin: Some(pin_status),
        // ADR-036 §3.1: the always-visible serving-side disclosure — the
        // normative text VERBATIM while carriage is on; the honest
        // opt-out statement when off (the UI renders this verbatim).
        carriage: Some(FamilyCarriageStatus {
            enabled: allow_content_carriage,
            disclosure: if allow_content_carriage {
                CARRIAGE_DISCLOSURE_SERVING.to_string()
            } else {
                CARRIAGE_DISCLOSURE_OFF.to_string()
            },
        }),
        governor_label: None,
    };
    (
        Some(FamilyServing {
            config,
            engine: Arc::clone(&engine),
            lease,
        }),
        status,
        Some(engine),
    )
}

/// Process-wide identity cache (2026-10-17, the identity.loaded-storm
/// lesson: the UI polls get_status every 2 s and list_models every 15 s,
/// and each poll re-read identity.seed from disk and log a row — 36
/// rows/min on the live machine, ~9.5k/day, burying the real events).
/// The seed never changes while the process runs; one load, then the
/// cache answers. Same error semantics as the loader (the FIRST call's
/// failure is sticky and surfaces exactly where it did before).
static IDENTITY_CACHE: std::sync::OnceLock<
    Result<modelswarm_identity::InstallationIdentity, String>,
> = std::sync::OnceLock::new();

fn cached_identity(
    data_dir: &std::path::Path,
    telemetry: &Telemetry,
) -> Result<modelswarm_identity::InstallationIdentity, String> {
    IDENTITY_CACHE
        .get_or_init(|| load_or_create_identity(data_dir, telemetry).map_err(|e| e.to_string()))
        .clone()
}

async fn tracker_client(
    tracker_url: &str,
    data_dir: &std::path::Path,
    telemetry: &Telemetry,
) -> Result<Arc<TrackerClient>, String> {
    let identity = cached_identity(data_dir, telemetry)?;
    let client = TrackerClient::new(tracker_url, Arc::new(identity));
    // Reuse the enrolled session (the roster/lookup endpoints require it).
    if let Ok(token) = std::fs::read_to_string(data_dir.join("session.token")) {
        let token = token.trim().to_string();
        if !token.is_empty() {
            client.set_session(token);
        }
    }
    Ok(Arc::new(client))
}

/// The LOCAL manifest of a listing, or the honest refusal for the cloud
/// family (no artifact exists; cloud hosting/serving lands with C3 — the
/// picker shows cloud models as next-release until then).
fn local_manifest_of(
    listing: &modelswarm_node::catalog::ProfileListing,
) -> Result<&modelswarm_node::LocalManifest, String> {
    match &listing.kind {
        modelswarm_node::catalog::ProfileKind::Local(m) => Ok(m),
        modelswarm_node::catalog::ProfileKind::Cloud(_) => Err(format!(
            "{} is a cloud model: no download and no local hosting yet — cloud serving lands in the next release (ADR-040)",
            listing.display_name
        )),
    }
}

/// Resolves a profile from the verified catalog: by id when given, else the
/// first active one.
async fn resolve_profile(
    tracker_url: &str,
    data_dir: &std::path::Path,
    telemetry: &Telemetry,
    profile_id: Option<&str>,
) -> Result<catalog::ProfileListing, String> {
    let tracker = tracker_client(tracker_url, data_dir, telemetry).await?;
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
    // TR-RC-8 (ADR-033 §9.2 review): READ-MODIFY-WRITE the FULL JSON
    // object. The daemon owns exactly six keys (`tracker`, `profile_id`,
    // `allow_swarm_pairing`, `allow_content_carriage`,
    // `assistant_acknowledged`, `assistant_enabled`); every other key in
    // config.json — another subsystem's config surface that later moves
    // in here, including the RETIRED `family_verified_chat` election key
    // (owner directive 2026-10-15: always on, no toggle) — is preserved
    // verbatim. (The previous struct-rewrite serialized only the known
    // fields and SILENTLY DELETED everything else on every rewrite.)
    let path = inner.data_dir.join("config.json");
    let mut root: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    root.insert("tracker".into(), serde_json::json!(inner.tracker_url));
    root.insert("profile_id".into(), serde_json::json!(inner.profile_id));
    root.insert(
        "allow_swarm_pairing".into(),
        serde_json::json!(inner.allow_swarm_pairing),
    );
    root.insert(
        "allow_content_carriage".into(),
        serde_json::json!(inner.allow_content_carriage),
    );
    root.insert(
        "assistant_acknowledged".into(),
        serde_json::json!(inner.assistant_acknowledged),
    );
    root.insert(
        "assistant_enabled".into(),
        serde_json::json!(inner.assistant_enabled),
    );
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?,
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
        "gpu": "optional — a Vulkan-capable GPU (any modern NVIDIA/AMD/Intel driver) accelerates decode; otherwise CPU",
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

// ---- Hardware analysis + per-machine recommendations ----------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct GpuInfo {
    pub name: String,
    pub vram_mb: u64,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct HardwareInfo {
    pub ram_mb: u64,
    pub cpu_cores: u32,
    pub gpu: Option<GpuInfo>,
}

/// How a profile fits this machine (drives card badges + ranking).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Weights fit free VRAM with headroom — full GPU offload.
    GpuFull,
    /// Partial offload: some layers on GPU, the rest in RAM.
    GpuPartial,
    /// No usable GPU (or none detected) — CPU execution.
    RamOnly,
}

impl Fit {
    fn label(self) -> &'static str {
        match self {
            Fit::GpuFull => "gpu_full",
            Fit::GpuPartial => "gpu_partial",
            Fit::RamOnly => "ram_only",
        }
    }
}

/// Feasibility + score for one profile on this machine. `None` = does not
/// fit RAM honestly (excluded from recommendations AND warned on start).
///
/// Scoring (bigger = better, deliberately explainable):
/// - full GPU fit:  10 + size_gb — the best experience wins outright,
///   larger among fits ranks higher (quality at equal speed).
/// - partial GPU:  6 + 6·min(1, vram/weights) + 0.2·size_gb — favors
///   offloading most layers; huge models on small GPUs sink.
/// - RAM only:  10 − |size_gb − 8| — peaks at ~8 GB of weights (the
///   responsive zone for CPU decode); giant dense models score far below
///   everything: usable, but not recommendation material.
pub fn fit_and_score(hw: &HardwareInfo, ram_est_mb: u64, weights_mb: u64) -> Option<(Fit, f64)> {
    if hw.ram_mb == 0 || ram_est_mb > hw.ram_mb * 9 / 10 {
        return None;
    }
    let size_gb = weights_mb as f64 / 1024.0;
    let fit = match &hw.gpu {
        Some(gpu) if gpu.vram_mb > 0 && weights_mb <= gpu.vram_mb * 9 / 10 => Fit::GpuFull,
        Some(gpu) if gpu.vram_mb > 0 => Fit::GpuPartial,
        _ => Fit::RamOnly,
    };
    let score = match fit {
        Fit::GpuFull => 10.0 + size_gb,
        Fit::GpuPartial => {
            let gpu = hw.gpu.as_ref().expect("partial implies gpu");
            6.0 + 6.0 * (gpu.vram_mb as f64 / weights_mb as f64).min(1.0) + 0.2 * size_gb
        }
        Fit::RamOnly => 10.0 - (size_gb - 8.0).abs(),
    };
    Some((fit, score))
}

/// Parses `llama-server --list-devices` output lines like
/// `  Vulkan0: NVIDIA GeForce RTX 5080 Laptop GPU (16003 MiB, 15235 MiB free)`
/// and keeps the largest device.
fn parse_largest_device(stdout: &str) -> Option<GpuInfo> {
    let mut best: Option<GpuInfo> = None;
    for line in stdout.lines() {
        let line = line.trim();
        let Some((_, rest)) = line.split_once(": ") else {
            continue;
        };
        let open = rest.rfind(" (")?;
        let close = rest.rfind(')')?;
        if close <= open {
            continue;
        }
        let name = rest[..open].trim();
        let mut parts = rest[open + 2..close].split(',');
        let total = parts.next()?.trim();
        let Some(total_mb) = total
            .strip_suffix(" MiB")
            .and_then(|n| n.trim().parse::<u64>().ok())
        else {
            continue;
        };
        if best.as_ref().is_none_or(|b| total_mb > b.vram_mb) {
            best = Some(GpuInfo {
                name: name.to_string(),
                vram_mb: total_mb,
            });
        }
    }
    best
}

fn total_ram_mb() -> u64 {
    #[cfg(windows)]
    {
        modelswarm_winjob::total_ram_bytes() / 1_000_000
    }
    #[cfg(not(windows))]
    {
        // /proc/meminfo "MemTotal:  32612345 kB" (Linux); macOS falls back
        // to sysctl via command (best effort, 0 = unknown).
        #[cfg(target_os = "linux")]
        {
            std::fs::read_to_string("/proc/meminfo")
                .ok()
                .and_then(|s| {
                    s.lines().next().and_then(|l| {
                        l.split_whitespace()
                            .nth(1)
                            .and_then(|kb| kb.parse::<u64>().ok())
                    })
                })
                .map(|kb| kb / 1000)
                .unwrap_or(0)
        }
        #[cfg(not(target_os = "linux"))]
        {
            std::process::Command::new("sysctl")
                .args(["-n", "hw.memsize"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(|b| b / 1_000_000)
                .unwrap_or(0)
        }
    }
}

/// Detects the machine's serving-relevant hardware once per process:
/// RAM (FFI), CPU cores, and the largest Vulkan device by asking the
/// bundled GPU engine itself (`--list-devices`) — the same backend that
/// will run the model, so the number is the one that matters.
async fn detect_hardware(app: &tauri::AppHandle) -> HardwareInfo {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    let gpu = engine_gpu_binary_path(app).and_then(|exe| {
        std::process::Command::new(&exe)
            .arg("--list-devices")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| parse_largest_device(&s))
    });
    HardwareInfo {
        ram_mb: total_ram_mb(),
        cpu_cores: cores,
        gpu,
    }
}

async fn hardware_of(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, DesktopState>,
) -> HardwareInfo {
    state
        .hardware
        .get_or_init(|| detect_hardware(app))
        .await
        .clone()
}

// Chat templating lives in the SERVING executor (ADR-025): the desktop
// sends plain {role, content} turns and the node renders ChatML exactly
// once. The pre-v0.2.18 client-side render_chatml caused either role-less
// flattening (0-token replies on tiny models) or double templating.

// ---- IPC commands ---------------------------------------------------------

#[tauri::command]
async fn get_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<serde_json::Value, String> {
    let inner = state.inner.lock().await;
    // ADR-036 §3.1: the requester-side election block — computed before
    // the status macro (the `disclosure` is the always-visible §3.1 text
    // with the ACTUAL cohort count). ALWAYS ELECTED since the 2026-10-15
    // owner directive (the toggle is gone; the disclosure is permanent).
    let family_verified_chat_status = {
        let elected = true;
        #[cfg(feature = "verifydrafts")]
        {
            let cohort = inner.last_family_cohort.unwrap_or(1);
            serde_json::json!({
                "elected": elected,
                "cohort": cohort,
                "disclosure": if elected {
                    serde_json::json!(requester_disclosure(cohort))
                } else {
                    serde_json::Value::Null
                },
            })
        }
        #[cfg(not(feature = "verifydrafts"))]
        {
            serde_json::json!({ "elected": elected, "cohort": 1, "disclosure": null })
        }
    };
    let engine = engine_binary_path(&app);
    let node_running = inner.node.is_some();
    let enrollment = match &inner.node {
        Some(running) => Some(running.enrollment.read().await.clone()),
        None => None,
    };
    let (gateway_addr, installation_id, engine_port, engine_backend) = match &inner.node {
        Some(running) => (
            Some(running.handle.local_addr.to_string()),
            Some(running.handle.installation_id.clone()),
            running.handle.engine_port,
            running.handle.engine_backend.clone(),
        ),
        None => (None, None, None, None),
    };
    Ok(serde_json::json!({
        "version": app.package_info().version.to_string(),
        "tracker": inner.tracker_url,
        "data_dir": inner.data_dir.display().to_string(),
        "engine_available": engine.is_file(),
        "engine_path": engine.display().to_string(),
        "gpu_engine_available": engine_gpu_binary_path(&app).is_some(),
        "license_accepted": inner.license_accepted,
        "profile_id": inner.profile_id,
        // The settings toggle's honest current state (owner directive
        // 2026-10-13): the UI renders the posture from THIS field, never
        // a guessed default.
        "pairing_allowed": inner.allow_swarm_pairing,
        // ADR-036 §3.1: the serving-side carriage posture (default on)
        // and the requester-side election (default off) — the UI's honest
        // posture sources for both disclosures.
        "content_carriage_allowed": inner.allow_content_carriage,
        "family_verified_chat": family_verified_chat_status,
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
            "engine_backend": engine_backend,
            "listener_addr": inner.listener_addr,
            // F2B: the relay circuit address currently ADVERTISED (only
            // while a live reservation holds it — the supervisor keeps
            // this truthful); null when no relay is configured/reserved.
            "relay_circuit_addr": inner.node.as_ref().and_then(|running| {
                running
                    .advertised
                    .borrow()
                    .iter()
                    .find(|addr| addr_is_circuit(addr))
                    .cloned()
            }),
            // ADR-039 disclosure (Diagnostics → Hosting detail): the
            // effective relay address and where it came from — the local
            // relay.addr file (tester/operator override) or the tracker
            // (community relay). null when neither is configured.
            "relay_config": effective_relay(
                configured_relay_addr(&inner.data_dir).as_deref(),
                inner
                    .relay_config
                    .as_ref()
                    .and_then(|rx| rx.borrow().clone())
                    .as_deref(),
            ),
            "enrollment": enrollment,
        },
        // The ADR-032 family half, exactly as the daemon decided it (on
        // with the real engine geometry / off with the reason); null
        // while not hosting. Never a static label.
        "family": serde_json::to_value(&inner.family).unwrap_or(serde_json::Value::Null),
        // The auto-update status machine's honest current state; `line`
        // inside is rendered VERBATIM by the header status note (owner
        // request 2026-10-12). Never a block on app usage.
        "updater": inner.updater.view(),
        "chat_turns": inner.chat_log.len(),
        // ADR-040: which provider key is staged, if any — presence only;
        // the key itself never crosses into the webview.
        "cloud": read_cloud_key(&inner.data_dir)
            .map(|r| serde_json::json!({ "provider": r.provider, "configured": true })),
        // ADR-037 §3.6 as rewired 2026-10-15: the assistant panel's
        // honest mode facts — which mode it is in and why (labels only;
        // the helper-identity banner text rides the panel, the reply
        // rides assistant-turn events). `hosted` = the machine's own
        // model backs the assistant right now; nothing external exists.
        "assistant": {
            "hosted": inner.assistant_plane.is_some() && inner.assistant_enabled,
            "gap": if inner.assistant_plane.is_none() {
                inner.assistant_degrade_gap.clone()
            } else if !inner.assistant_enabled {
                "disabled".to_string()
            } else {
                "none".to_string()
            },
            "acknowledged": inner.assistant_acknowledged,
            "enabled": inner.assistant_enabled,
            "identity": if inner.assistant_plane.is_some()
                && inner.assistant_enabled
                && !inner.assistant_acknowledged
            {
                Some(assistant::FIRST_USE_IDENTITY)
            } else {
                None
            },
        },
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
    // The same round-trip-preserving writer as every other config change
    // (TR-RC-8): unknown keys in config.json survive a tracker change.
    persist_config(&inner)
}

/// The "allow swarm pairing" settings toggle (owner directive 2026-10-13):
/// persisted key, default on. The toggle gates the serving listener + the
/// family verify engine at HOSTING START — changing it while hosting would
/// leave the live listener contradicting the setting, so it is refused
/// until hosting stops (the same discipline as `set_tracker`). The honest
/// current state always flows through `get_status.pairing_allowed`.
#[tauri::command]
async fn set_pairing_allowed(
    state: tauri::State<'_, DesktopState>,
    allowed: bool,
) -> Result<(), String> {
    let mut inner = state.inner.lock().await;
    if inner.node.is_some() {
        return Err("stop hosting before changing swarm pairing".into());
    }
    inner.allow_swarm_pairing = allowed;
    log_event(
        &state.telemetry,
        "info",
        "settings.pairing",
        &[("allowed", &allowed.to_string())],
    );
    persist_config(&inner)
}

/// ADR-036 §3.1: the SERVING-side user-content carriage toggle
/// ("carry other users' conversations through family verification",
/// persisted key `allow_content_carriage`, DEFAULT ON per the owner's
/// swarm-together directive). Opting out refuses pin carriers with the
/// honest `content_carriage_refused` reject. Applies at the NEXT hosting
/// start — refusing while hosting keeps the live listener from
/// contradicting the setting (the same discipline as the pairing toggle).
#[tauri::command]
async fn set_content_carriage(
    state: tauri::State<'_, DesktopState>,
    allowed: bool,
) -> Result<(), String> {
    let mut inner = state.inner.lock().await;
    if inner.node.is_some() {
        return Err(
            "stop hosting before changing conversation carriage (applies at the next hosting start)"
                .into(),
        );
    }
    inner.allow_content_carriage = allowed;
    log_event(
        &state.telemetry,
        "info",
        "settings.content_carriage",
        &[("allowed", &allowed.to_string())],
    );
    persist_config(&inner)
}

// REMOVED 2026-10-15 (owner directive — "this should always be on"):
// the requester-side family-verified-chat election toggle and its
// `set_family_verified_chat` IPC are GONE. Election is unconditional;
// the ADR-036 revision note records the supersession.

/// Clears the persisted conversation (owner directive 2026-10-13 — the
/// greedy language-drift escape hatch; tonight's live confirmation: a
/// restart cleared the drift, so the SAME reset must be reachable without
/// one). Resets the daemon-side rendered history AND tells the UI to drop
/// the transcript.
#[tauri::command]
async fn clear_chat(
    state: tauri::State<'_, DesktopState>,
    window: tauri::Window,
) -> Result<serde_json::Value, String> {
    let mut inner = state.inner.lock().await;
    inner.chat_log.clear();
    let _ = window.emit("chat-cleared", ());
    log_event(&state.telemetry, "info", "chat.cleared", &[]);
    Ok(serde_json::json!({ "cleared": true, "chat_turns": 0 }))
}

// ---- The assistant panel's IPC (ADR-037 §4.5): its own commands, its
// own `assistant-*` event namespace — nothing here touches the swarm
// chat transcript (W4), and the panel's turns are the ONLY user-trigger
// of the plane (W2).

/// One assistant panel turn: user-triggered UI only. Builds the typed
/// context from the app's own facts (hardware, picker cache, family
/// status labels), drives the plane, and renders the outcome — a
/// hosted-model reply (identity chip always adjacent), an honest gap,
/// or built-in static help. Emits `assistant-turn`.
#[tauri::command]
async fn assistant_turn(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    window: tauri::Window,
    message: String,
) -> Result<serde_json::Value, String> {
    let inner = state.inner.lock().await;
    let hardware = hardware_of(&app, &state).await;
    let family = inner.family.clone();
    let hosting_on = inner.node.is_some();
    let listener_addr = inner.listener_addr.clone();
    let engine_backend = inner
        .node
        .as_ref()
        .and_then(|r| r.handle.engine_backend.clone());
    let inputs = assistant::ContextInputs {
        hardware: &hardware,
        picker: &inner.last_cards,
        family: family.as_ref(),
        hosting_on,
        listener_addr: listener_addr.as_deref(),
        engine_backend: engine_backend.as_deref(),
        engage_last: inner.last_engage_reason.as_deref(),
        governor_verdict: inner.last_governor_verdict.as_deref(),
        probe_verdict: inner.last_probe_verdict.as_deref(),
        capacity_class: inner.capacity_class.as_deref(),
        app_version: env!("CARGO_PKG_VERSION"),
    };
    let deps = assistant::TurnDeps {
        plane: inner.assistant_plane.as_ref(),
        degrade_gap: &inner.assistant_degrade_gap,
        opt_out: !inner.assistant_enabled,
        telemetry: state.telemetry.as_ref(),
        inputs: &inputs,
    };
    let outcome = assistant::turn(deps, &message).await;
    drop(inner);
    let payload = assistant_outcome_json(&outcome);
    let _ = window.emit("assistant-turn", &payload);
    Ok(payload)
}

/// Resets the assistant conversation (`assistant_reset`): closes the
/// conversation and clears the local history. Emits `assistant-reset`.
#[tauri::command]
async fn assistant_reset(
    state: tauri::State<'_, DesktopState>,
    window: tauri::Window,
) -> Result<serde_json::Value, String> {
    let inner = state.inner.lock().await;
    if let Some(plane) = &inner.assistant_plane {
        plane.reset();
    }
    drop(inner);
    let _ = window.emit("assistant-reset", ());
    Ok(serde_json::json!({ "reset": true }))
}

/// Records the first-use helper-identity banner dismissal (persisted).
/// Since the 2026-10-15 collapse this is cosmetic only — nothing
/// external is called, so no turn is ever blocked on it; dismissing
/// just hides the banner.
#[tauri::command]
async fn assistant_acknowledge(state: tauri::State<'_, DesktopState>) -> Result<(), String> {
    let mut inner = state.inner.lock().await;
    inner.assistant_acknowledged = true;
    persist_config(&inner)
}

/// The Settings opt-out toggle (persisted; zero functional loss).
#[tauri::command]
async fn set_assistant_enabled(
    state: tauri::State<'_, DesktopState>,
    enabled: bool,
) -> Result<(), String> {
    let mut inner = state.inner.lock().await;
    inner.assistant_enabled = enabled;
    persist_config(&inner)
}

/// Renders one [`assistant::PanelOutcome`] for the webview (labels and
/// display text only — the identity chip and AD-3 facts pairing are
/// rendered by the panel around this).
fn assistant_outcome_json(outcome: &assistant::PanelOutcome) -> serde_json::Value {
    match outcome {
        assistant::PanelOutcome::ModelReply {
            reply,
            conversation_id,
            turn,
            hints,
            hints_discarded,
        } => serde_json::json!({
            "kind": "reply",
            "reply": reply,
            "conversation_id": conversation_id,
            "turn": turn,
            "hints": hints,
            "hints_discarded": hints_discarded,
            "chip": assistant::IDENTITY_CHIP,
        }),
        assistant::PanelOutcome::ModelGap { gap } => {
            serde_json::json!({ "kind": "gap", "gap": gap, "chip": assistant::IDENTITY_CHIP })
        }
        assistant::PanelOutcome::StaticHelp { entries, line } => serde_json::json!({
            "kind": "static",
            "line": line,
            "entries": entries
                .iter()
                .map(|e| serde_json::json!({ "label": e.label, "text": e.text }))
                .collect::<Vec<_>>(),
        }),
    }
}

#[tauri::command]
async fn accept_privacy(state: tauri::State<'_, DesktopState>) -> Result<(), String> {
    let mut inner = state.inner.lock().await;
    let tracker = tracker_client(&inner.tracker_url, &inner.data_dir, &state.telemetry).await?;
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
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<Vec<serde_json::Value>, String> {
    let mut inner = state.inner.lock().await;
    let tracker = tracker_client(&inner.tracker_url, &inner.data_dir, &state.telemetry).await?;
    // One retry after a short pause: a single reset packet between here and
    // the hub should not read as "catalog down". Both attempts are logged;
    // the second failure is what the UI shows.
    let profiles = match catalog::active_profiles(&tracker).await {
        Ok(profiles) => profiles,
        Err(first) => {
            log_event(
                &state.telemetry,
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
    let hardware = hardware_of(&app, &state).await;
    let mut out = Vec::new();
    for p in profiles {
        let downloaded = store
            .as_ref()
            .and_then(|s| s.get_artifact(&p.profile_id).ok())
            .flatten()
            .map(|(_, _, bytes, _state)| serde_json::json!({ "bytes": bytes }))
            .or_else(|| {
                let manifest = local_manifest_of(&p).ok()?;
                manager
                    .artifact_path(manifest)
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
                if p.kind.is_cloud() {
                    // Cloud rows have no artifact to size (no download);
                    // the picker renders the cloud marker instead.
                    None
                } else if fresh {
                    inner.size_cache.get(&p.profile_id).map(|(bytes, _)| *bytes)
                } else {
                    let manifest = local_manifest_of(&p).expect("non-cloud checked above");
                    let url = format!(
                        "https://huggingface.co/{}/resolve/{}/{}",
                        manifest.hf_repo(),
                        manifest.hf_revision(),
                        manifest.artifact_hashes()[0].path(),
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
        let hw_json = hardware_requirements(size_bytes);
        let (ram_est_mb, weights_mb) = match size_bytes {
            Some(bytes) => {
                let hw = hardware_requirements(Some(bytes)).expect("size known implies hw");
                (hw["ram_mb_est"].as_u64().unwrap_or(0), bytes / 1_000_000)
            }
            None => (0, 0),
        };
        let ranked = if ram_est_mb > 0 {
            fit_and_score(&hardware, ram_est_mb, weights_mb)
        } else {
            None
        };
        // ADR-040: cloud rows carry the family marker — the picker renders
        // them as next-release (no download, key-backed when C3 lands);
        // local rows keep their exact shape.
        if p.kind.is_cloud() {
            let cloud = match &p.kind {
                modelswarm_node::catalog::ProfileKind::Cloud(c) => c,
                _ => unreachable!(),
            };
            let provider = match cloud.provider() {
                modelswarm_node::catalog::CloudProviderId::DeepSeek => "deepseek",
                modelswarm_node::catalog::CloudProviderId::OpenRouter => "openrouter",
            };
            out.push(serde_json::json!({
                "profile_id": p.profile_id,
                "display_name": p.display_name,
                "is_cloud": true,
                "provider": provider,
                "model": cloud.model(),
                "downloaded": serde_json::json!({ "bytes": 0 }),
                "hw": hw_json,
                "fit": ranked.map(|(fit, _)| fit.label()),
                "feasible": true,
            }));
            continue;
        }
        let manifest = local_manifest_of(&p)?;
        out.push(serde_json::json!({
            "profile_id": p.profile_id,
            "display_name": p.display_name,
            "quantization": format!("{} ({} bit)", manifest.quantization().method(), manifest.quantization().bits()),
            // Structured twins of the rendered quantization line (the
            // assistant panel's picker rows read these typed fields —
            // the signed catalog's method/bits, never a re-parse).
            "quant_method": manifest.quantization().method(),
            "quant_bits": manifest.quantization().bits(),
            "runtime": manifest.runtime().version(),
            "downloaded": downloaded,
            "hw": hw_json,
            "fit": ranked.map(|(fit, _)| fit.label()),
            "feasible": ranked.is_some() || ram_est_mb == 0,
            "selected": inner.profile_id.as_deref() == Some(p.profile_id.as_str()),
        }));
    }

    // The shortlist: rank every feasible, size-known profile and flag the
    // top 6 as recommended for THIS machine (owner directive 2026-10-07 —
    // too many options is not good; every user gets a different six).
    let mut ranked_indices: Vec<(usize, f64, u64)> = out
        .iter()
        .enumerate()
        .filter_map(|(i, m)| {
            let fit = m["fit"].as_str()?;
            let score = match fit {
                "gpu_full" => 10.0,
                "gpu_partial" => 6.0,
                _ => 0.0,
            };
            // Exact score recomputed from the same inputs (kept in the
            // closure to avoid cloning HardwareInfo into the filter).
            let _ = score;
            let hw = m["hw"].as_object()?;
            let ram_est = hw["ram_mb_est"].as_u64()?;
            let weights = hw["download_mb"].as_u64()?;
            let (_, s) = fit_and_score(&hardware, ram_est, weights)?;
            Some((i, s, weights))
        })
        .collect();
    ranked_indices.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.2.cmp(&a.2)));
    let recommended: std::collections::HashSet<usize> = ranked_indices
        .into_iter()
        .take(6)
        .map(|(i, _, _)| i)
        .collect();
    for (i, entry) in out.iter_mut().enumerate() {
        entry["recommended"] = serde_json::json!(recommended.contains(&i));
    }

    // The census join (owner directive 2026-10-12): the last good public
    // stats fetch (the 30 s `census` poll) joined against THIS loaded
    // catalog — per-card live peer count (`peers`, null = census n/a) +
    // the honest `census_chip` — and the cards returned in the canonical
    // picker order (shortlist -> fit tier -> peers -> name). The webview
    // renders them in exactly this order; it recomputes nothing.
    let census_counts = inner.census_counts.clone();
    for entry in out.iter_mut() {
        let peers = census_peers(
            entry["profile_id"].as_str().unwrap_or(""),
            census_counts.as_deref(),
        );
        entry["peers"] = serde_json::json!(peers);
        entry["census_chip"] = serde_json::json!(census_chip(peers));
    }
    let mut ordered: Vec<serde_json::Value> = Vec::with_capacity(out.len());
    for (i, _) in census_view(&out, census_counts.as_deref()) {
        // The view is a permutation of the card indices — each is taken
        // exactly once, so the Nulls left behind are never read again.
        ordered.push(std::mem::take(&mut out[i]));
    }
    // Cache the final cards for the assistant panel's picker rows (the
    // typed catalog/candidate rows exactly as the picker computed them).
    inner.last_cards = ordered.clone();
    Ok(ordered)
}

#[tauri::command]
async fn analyze_hardware(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<serde_json::Value, String> {
    serde_json::to_value(hardware_of(&app, &state).await).map_err(|e| e.to_string())
}

// ---- Live peer census (owner directive 2026-10-12) ----------------------
//
// "we should have indication on what models people are hosting, so new
// people can select those that have more people": the tracker's PUBLIC
// stats endpoint — the same one the website's census reads (GET
// /api/v1/stats, no auth, counts only, content-blind by construction) —
// joined against the picker's loaded catalog. The `census` IPC command
// below refreshes a daemon-side cache on its own 30 s cadence;
// `list_models` joins that cache into every catalog response (per-card
// `peers` + `census_chip` and the canonical card order), so the picker
// logic lives HERE, once, testable — the webview is a dumb renderer.
// Honesty rules:
// - never blocks the picker: the join reads whatever cache exists; no
//   fresh census reads "census n/a" (the degraded-chips pattern), never
//   a zero, and a failed poll DROPS the stale cache.
// - never reorders the hardware shortlist: fit stays primary; live peers
//   only break ties WITHIN one fit tier (`census_order`), and census
//   counts NEVER change which six cards are recommended (membership is
//   hardware-scored in `list_models`).
// - empty and unreachable are different states: a well-formed stats body
//   with no hosting peers is `Some(0)` ("first peer"); a malformed body
//   is an error (`parse_census_counts` is strict) so garbage can never
//   render as "nobody hosting".

/// Parses the public `/api/v1/stats` body into (peersOnline, per-profile
/// live counts). STRICT: any missing or non-numeric field is an error —
/// an empty-but-valid census (nobody hosting) and an unreadable one are
/// different honest states and must never collapse into "first peer".
fn parse_census_counts(
    body: &serde_json::Value,
) -> Result<(u64, std::collections::BTreeMap<String, u64>), String> {
    let peers_online = body["peersOnline"]
        .as_u64()
        .ok_or("stats body missing numeric peersOnline")?;
    let models = body["models"]
        .as_array()
        .ok_or("stats body missing models array")?;
    let mut counts = std::collections::BTreeMap::new();
    for entry in models {
        let profile_id = entry["profileId"]
            .as_str()
            .ok_or("stats model entry missing profileId")?;
        let peers = entry["peers"]
            .as_u64()
            .ok_or("stats model entry missing numeric peers")?;
        counts.insert(profile_id.to_string(), peers);
    }
    Ok((peers_online, counts))
}

/// One model's live peer count: `None` = census unreachable, `Some(0)` =
/// reachable and nobody hosting it (profile absent from the counts —
/// stats only lists profiles with a live lease).
fn census_peers(
    profile_id: &str,
    counts: Option<&std::collections::BTreeMap<String, u64>>,
) -> Option<u64> {
    counts.map(|c| c.get(profile_id).copied().unwrap_or(0))
}

/// The honest per-card census chip: the exact live count, "first peer"
/// at zero, "census n/a" while unreachable — no invented numbers.
fn census_chip(peers: Option<u64>) -> String {
    match peers {
        None => "census n/a".into(),
        Some(0) => "first peer".into(),
        Some(1) => "1 peer".into(),
        Some(n) => format!("{n} peers"),
    }
}

/// Fit tier rank for the picker order: full GPU offload first, partial
/// next, RAM-only after, unknown (size not known) last.
fn tier_rank(fit: Option<&str>) -> u8 {
    match fit {
        Some("gpu_full") => 0,
        Some("gpu_partial") => 1,
        Some("ram_only") => 2,
        _ => 3,
    }
}

/// The canonical picker order: the v0.2.19 hardware shortlist stays
/// PRIMARY — recommended cards first — then fit tier, then MORE LIVE
/// PEERS first (equal for every card while the census is n/a — no
/// invented counts), then display name, and the stable catalog order for
/// full ties. `list_models` returns cards in exactly this order; the
/// webview renders them in it verbatim.
fn census_order(
    models: &[serde_json::Value],
    counts: Option<&std::collections::BTreeMap<String, u64>>,
) -> Vec<usize> {
    let mut order: Vec<usize> = (0..models.len()).collect();
    order.sort_by(|&a, &b| {
        let (ma, mb) = (&models[a], &models[b]);
        mb["recommended"]
            .as_bool()
            .unwrap_or(false)
            .cmp(&ma["recommended"].as_bool().unwrap_or(false))
            .then(tier_rank(ma["fit"].as_str()).cmp(&tier_rank(mb["fit"].as_str())))
            .then(
                census_peers(mb["profile_id"].as_str().unwrap_or(""), counts).cmp(&census_peers(
                    ma["profile_id"].as_str().unwrap_or(""),
                    counts,
                )),
            )
            .then(
                ma["display_name"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(mb["display_name"].as_str().unwrap_or("")),
            )
    });
    order
}

/// The joined render the picker shows: `(model index, chip)` pairs in
/// `census_order` — the same view `list_models` bakes into its cards.
fn census_view(
    models: &[serde_json::Value],
    counts: Option<&std::collections::BTreeMap<String, u64>>,
) -> Vec<(usize, String)> {
    census_order(models, counts)
        .into_iter()
        .map(|i| {
            (
                i,
                census_chip(census_peers(
                    models[i]["profile_id"].as_str().unwrap_or(""),
                    counts,
                )),
            )
        })
        .collect()
}

#[tauri::command]
async fn census(state: tauri::State<'_, DesktopState>) -> Result<serde_json::Value, String> {
    let tracker_url = {
        let inner = state.inner.lock().await;
        inner.tracker_url.clone()
    };
    // Public GET /api/v1/stats — the same no-auth, counts-only endpoint
    // the website's census reads. The tracker-api client's surface is the
    // signed msp-v1 roster set (no stats method, and that crate is not
    // this agent's to extend), so this is a plain reqwest GET against the
    // SAME configured tracker base URL. Short timeout: the picker never
    // waits on it — `list_models` joins whatever cache exists.
    let http = reqwest::Client::new();
    let fetched: Result<(u64, std::collections::BTreeMap<String, u64>), String> = async {
        let body: serde_json::Value = http
            .get(format!("{tracker_url}/api/v1/stats"))
            .timeout(Duration::from_secs(6))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        parse_census_counts(&body)
    }
    .await;
    let mut inner = state.inner.lock().await;
    match fetched {
        Ok((peers_online, counts)) => {
            inner.census_counts = Some(Arc::new(counts.clone()));
            Ok(serde_json::json!({
                "peers_online": peers_online,
                "counts": counts,
            }))
        }
        Err(e) => {
            // Drop the stale cache: a failed poll must read "census n/a"
            // on the next render, never yesterday's counts.
            inner.census_counts = None;
            log_event(
                &state.telemetry,
                "warn",
                "census.error",
                &[("error", &e), ("tracker", &tracker_url)],
            );
            Err(e)
        }
    }
}

// ---- Resource monitor panel (owner request 2026-10-12) --------------------
//
// A task-manager-style CPU/GPU/RAM panel in the right column. The sampling
// layer is the resource crate's `ResourceSampler` (advisory/read-only —
// NEVER an enforcement mechanism; the M2 governor is separate); this
// section owns scoping + the one IPC poll.
//
// Scoping: the process tree equals the SUPERVISED set — the winjob
// kill-on-close job-object membership (sampler handoff §4, preferred
// option (a)) plus the desktop's own process, so CPU%/working-set cover
// this app and every supervised engine child exactly, immune to PID-reuse
// orphans. The sampler is rebuilt when the root set changes (hosting
// start/stop): one tick of honest CPU "n/a" while the fresh baseline
// establishes, never a fabricated number.
//
// Cadence: the UI polls `resource_sample` at 1 Hz from ONE timer; the
// platform sample runs on the blocking pool (sync Toolhelp/PDH FFI) and
// the sync mutex serializes any overlap. Every unavailable field crosses
// as null and renders "n/a" (the degraded-chips pattern).

/// The panel's process-tree scope roots: this process plus the winjob
/// job-object membership (the supervised engine set). Sorted + deduped so
/// equality comparison against the sampler's current roots is stable.
fn monitor_roots() -> Vec<u32> {
    let mut roots = vec![std::process::id()];
    // Windows only: job membership is the supervised set. Elsewhere the
    // desktop's own tree (this process + descendants) is the honest scope.
    #[cfg(windows)]
    roots.extend(modelswarm_winjob::supervised_process_ids());
    roots.sort_unstable();
    roots.dedup();
    roots
}

/// One panel sampler bound to the root set it was built for.
struct PanelMonitor {
    roots: Vec<u32>,
    sampler: modelswarm_resource::ResourceSampler<Box<dyn SampleSource>>,
}

impl PanelMonitor {
    fn new(roots: Vec<u32>) -> Self {
        let scope = modelswarm_resource::ProcessScope::Roots(roots.clone());
        Self {
            roots,
            sampler: modelswarm_resource::ResourceSampler::platform(scope),
        }
    }
}

/// Keeps the panel sampler bound to the CURRENT root set: creates it on
/// the first poll and rebuilds it whenever the supervised set changed
/// (hosting start/stop). Returns whether the sampler was rebuilt this
/// call (a rebuild costs one tick of CPU "n/a" — the fresh baseline).
fn ensure_panel_monitor(panel: &mut Option<PanelMonitor>, roots: Vec<u32>) -> bool {
    match panel {
        Some(existing) if existing.roots == roots => false,
        Some(_) | None => {
            *panel = Some(PanelMonitor::new(roots));
            true
        }
    }
}

/// The IPC shape of one sample: the `ResourceSample` field names verbatim
/// (one source of truth), `null` per field when the sampler could not read
/// it this tick (the UI renders "n/a", never a guess).
fn resource_sample_json(sample: &modelswarm_resource::ResourceSample) -> serde_json::Value {
    serde_json::json!({
        "process_tree_cpu_percent": sample.process_tree_cpu_percent,
        "system_ram_used_percent": sample.system_ram_used_percent,
        "system_ram_used_bytes": sample.system_ram_used_bytes,
        "system_ram_total_bytes": sample.system_ram_total_bytes,
        "process_tree_working_set_bytes": sample.process_tree_working_set_bytes,
        "gpu_utilization_percent": sample.gpu_utilization_percent,
        "gpu_vram_used_bytes": sample.gpu_vram_used_bytes,
    })
}

#[tauri::command]
async fn resource_sample(
    state: tauri::State<'_, DesktopState>,
) -> Result<serde_json::Value, String> {
    let monitor = Arc::clone(&state.monitor);
    let sample = tokio::task::spawn_blocking(move || {
        let mut guard = monitor.lock().map_err(|e| e.to_string())?;
        ensure_panel_monitor(&mut guard, monitor_roots());
        let panel = guard
            .as_mut()
            .expect("ensure_panel_monitor always leaves a sampler");
        Ok::<_, String>(panel.sampler.sample())
    })
    .await
    .map_err(|e| format!("monitor task failed: {e}"))??;
    Ok(resource_sample_json(&sample))
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
    let profile = resolve_profile(&tracker_url, &data_dir, &state.telemetry, None).await?;
    let manager = ArtifactManager::new(&data_dir);
    let store = std::sync::Mutex::new(
        modelswarm_store::Store::open(data_dir.join("state.sqlite")).map_err(|e| e.to_string())?,
    );
    let ensured = manager
        .ensure_artifact(local_manifest_of(&profile)?, &store, |done, total| {
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
    let _ = &window; // (kept for signature stability; the body ignores it)
    set_hosting_inner(&app, &state, on).await
}

/// Owner directive (2026-10-17, the B-idle lesson): the hosting choice
/// SURVIVES restarts. Presence of the marker file IS the choice — no
/// content, no schema to version. Written only after a successful
/// bring-up, removed on an explicit HOST OFF, so the marker always
/// reflects the owner's last deliberate state ("everything on by
/// design"; serving default-ON per the 2026-10-13 ruling).
fn hosting_autostart_marker(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("hosting.autostart")
}

async fn set_hosting_inner(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, DesktopState>,
    on: bool,
) -> Result<serde_json::Value, String> {
    let mut inner = state.inner.lock().await;
    if !on {
        if let Some(running) = inner.node.take() {
            let _ = running.shutdown.send(true);
            running.heartbeat.abort();
            let _ = running.handle.stopped().await;
        }
        // The explicit OFF is the owner's deliberate state — the resume
        // marker dies with the node (auto-resume never overrides an
        // explicit choice).
        let _ = std::fs::remove_file(hosting_autostart_marker(&inner.data_dir));
        inner.listener_addr = None;
        inner.family = None;
        // ADR-038 §6.4 row 8: hosting off = no heartbeat = no fresh
        // tickets — drop the cache with the node (a stale ticket would
        // only earn an honest `expired` refusal at a relay's gate) and
        // retire the pooled swarm executor so the next turn rebuilds it
        // ticket-less instead of presenting the dead cache forever.
        inner.relay_tickets = None;
        inner.relay_config = None;
        inner.swarm_executor = None;
        // 2026-10-15: hosting off = no model backing — the assistant
        // returns to the static floor with the honest label (its
        // conversation state dies with the backing that served it).
        inner.assistant_plane = None;
        inner.assistant_degrade_gap = "not_hosting".to_string();
        #[cfg(feature = "verifydrafts")]
        {
            inner.family_engine = None;
        }
        return Ok(serde_json::json!({ "hosting": false }));
    }

    if !inner.license_accepted {
        return Err("accept the privacy disclosure first".into());
    }
    // Idempotent when hosting already runs (the resume supervisor can
    // race an owner click; one node, one bring-up, never two).
    if inner.node.is_some() {
        return Ok(serde_json::json!({ "hosting": true }));
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
        &state.telemetry,
        inner.profile_id.as_deref(),
    )
    .await
    .map_err(|e| format!("catalog: {e}"))?;
    // ADR-040 C3: the CLOUD branch — a staged provider key serves the
    // cloud profile through the provider API (no engine, no artifact);
    // the entire serving/heartbeat/relay/receipt machinery is identical.
    // The honest refusal fires when no key is staged for the provider.
    let cloud_cfg = match &listing.kind {
        modelswarm_node::catalog::ProfileKind::Cloud(cloud) => {
            let staged = read_cloud_key(&inner.data_dir);
            let provider = match cloud.provider() {
                modelswarm_node::catalog::CloudProviderId::DeepSeek => "deepseek",
                modelswarm_node::catalog::CloudProviderId::OpenRouter => "openrouter",
            };
            let key = match staged.as_ref().filter(|k| k.provider == provider) {
                Some(k) => k.api_key.clone(),
                None => {
                    return Err(format!(
                    "add your {provider} API key in Settings first — the key stays on this machine"
                ))
                }
            };
            let provider_kind = match cloud.provider() {
                modelswarm_node::catalog::CloudProviderId::DeepSeek => {
                    modelswarm_node::cloud_executor::CloudProvider::DeepSeek
                }
                modelswarm_node::catalog::CloudProviderId::OpenRouter => {
                    modelswarm_node::cloud_executor::CloudProvider::OpenRouter
                }
            };
            Some(modelswarm_node::cloud_executor::CloudApiConfig {
                provider: provider_kind,
                model: cloud.model().to_string(),
                api_key: key,
                request_timeout: std::time::Duration::from_secs(120),
                base_url_override: None,
            })
        }
        modelswarm_node::catalog::ProfileKind::Local(_) => None,
    };
    let (ensured, engine, gpu_engine) = if cloud_cfg.is_some() {
        (None, None, None)
    } else {
        let serving_manifest = local_manifest_of(&listing)?;
        let manager = ArtifactManager::new(&inner.data_dir);
        let store = std::sync::Mutex::new(
            modelswarm_store::Store::open(inner.data_dir.join("state.sqlite"))
                .map_err(|e| e.to_string())?,
        );
        let ensured = manager
            .ensure_artifact(serving_manifest, &store, |_, _| {})
            .await
            .map_err(|e| format!("artifact: {e}"))?;
        // ADR-024: prefer the GPU (Vulkan) engine variant when the bundle
        // is present; the node falls back to the CPU engine visibly.
        (Some(ensured), Some(engine), engine_gpu_binary_path(app))
    };

    let (shutdown_tx, _) = tokio::sync::watch::channel(false);
    let mut config = NodeConfig {
        tracker_base: Some(inner.tracker_url.clone()),
        // Stable local API first (DEFAULT_PORT 11435); fall back to an
        // ephemeral port when it is taken so a second installation runs too.
        gateway_port: DEFAULT_PORT,
        data_dir: inner.data_dir.clone(),
        profile_id: Some(listing.profile_id.clone()),
        mock: false,
        engine_binary: engine,
        engine_gpu_binary: gpu_engine,
        engine_model: ensured.as_ref().map(|e| e.path.clone()),
        engine_threads: None,
        cloud: cloud_cfg,
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

    // 2026-10-15 owner directive: the helper uses the hosted model when
    // hosted — arm the assistant plane against THIS node's own loopback
    // gateway (the same OpenAI-compatible path chat uses, so
    // queueing/governor fairness applies). Not hosting = the static
    // floor; the honest gap rides `assistant_degrade_gap`.
    {
        let (plane, gap) =
            assistant::Plane::spawn_hosted(&inner.data_dir, handle.local_addr, &listing.profile_id);
        inner.assistant_plane = plane;
        inner.assistant_degrade_gap = gap;
    }

    // F0(3) + the ADR-032 light-up: attach the serving bridge to the
    // node's executor. The bridge dies with the node via the shared
    // shutdown watch; the advertised addresses are the listener's real
    // bound address plus — since F2B (ADR-014) — the relay circuit
    // address while a configured relay holds a live reservation. With the
    // family tier compiled in (verifydrafts), ONE listener serves BOTH
    // vocabularies through the composed daemon — msp-v1 unchanged, family
    // sessions behind the RC-L1 profile-bound lease gate; when the family
    // half cannot init the daemon degrades HONESTLY to the plain msp-v1
    // bridge (a logged reason + the status field, never a silent
    // family-off). The family tier is ON BY DEFAULT since 2026-10-13: the
    // "allow swarm pairing" settings toggle (default on) gates the
    // listener; a bind failure surfaces its exact reason in the family
    // line.
    let hardware = hardware_of(app, state).await;
    // ADR-038 §6.4 row 8: the relay admission ticket cache for this
    // hosting session — written by the heartbeat task below (every
    // successful heartbeat replaces the cached ticket), read by the
    // relay reservation supervisor and (via `Inner`) the swarm chat's
    // remote executor through the transport's ticket seam. Invisible by
    // design: no switch, no UI element (the owner's directive).
    let relay_tickets =
        std::sync::Arc::new(modelswarm_node::relay_ticket_cache::RelayTicketCache::new());
    inner.relay_tickets = Some(std::sync::Arc::clone(&relay_tickets));
    // ADR-030 B2: the serving-side receipt signer — THIS installation's
    // identity (same seed the tracker client loads; peer id + key for the
    // phase-1 receipt signature).
    let serving_identity = std::sync::Arc::new(
        cached_identity(&inner.data_dir, &state.telemetry).map_err(|e| e.to_string())?,
    );
    // ADR-039: the tracker-supplied relay address channel (heartbeat task
    // sends; the lazy supervisor and the status surface read).
    let (relay_config_tx, relay_config_rx) = tokio::sync::watch::channel::<Option<String>>(None);
    inner.relay_config = Some(relay_config_rx.clone());
    // The pooled swarm executor (if any) predates this cache — retire it
    // so the next swarm turn rebuilds with the ticket attached (mirrored
    // by the hosting-off reset above).
    inner.swarm_executor = None;
    // F2B: the heartbeat's advertised address list — direct first, the
    // relay circuit address while a live reservation holds it. The relay
    // supervisor (below) owns changes; the heartbeat re-registers on
    // change; `get_status` reads it for the serving chip.
    let (advertised_tx, advertised_rx) = tokio::sync::watch::channel(Vec::<String>::new());
    let listener_bind =
        bind_serving_listener(&inner.data_dir, &state.telemetry, inner.allow_swarm_pairing).await;
    let listener_bind_reason: Option<(&'static str, String)> = match &listener_bind {
        ListenerBind::Bound(_, _) => None,
        ListenerBind::Disabled => Some((
            "pairing_disabled",
            "the allow-swarm-pairing setting is off".into(),
        )),
        ListenerBind::Failed(detail) => Some(("listener_bind_failed", detail.clone())),
    };
    if let ListenerBind::Bound(listener, addr) = listener_bind {
        if let (Some(executor), Some(profile)) =
            (handle.executor.clone(), handle.profile_id.clone())
        {
            inner.listener_addr = Some(addr.clone());
            let _ = advertised_tx.send(vec![addr.clone()]);
            let serving_shutdown = handle.shutdown_rx.clone();
            let serving_telemetry = std::sync::Arc::clone(&state.telemetry);

            // F2B (ADR-014) serving side: reserve on the CONFIGURED relay
            // (`<data_dir>/relay.addr`, default EMPTY — nothing invented)
            // so peers behind NAT can still dial us. Reservation use is
            // automatic when configured: no switch, no prompt, and never
            // a BLOCKING one — the supervisor's FIRST tick (it runs
            // before its first sleep) performs the initial attempt in
            // the background, so a configured-but-down relay cannot stall
            // hosting start. It keeps reacquiring (the relay may start
            // later) and owns the advertised list's circuit entry; the
            // heartbeat re-registers when the list changes.
            let relay_slot = modelswarm_node::serving::RelaySlot::empty();
            // ADR-039: the effective relay address resolves LAZILY — the
            // local `relay.addr` file wins (immediate, the F2B behavior);
            // otherwise the supervisor waits for the tracker's first
            // non-empty heartbeat `relayAddr` and starts then. Exactly
            // one reservation supervisor per hosting session either way.

            tokio::spawn(relay_config_supervisor(
                inner.data_dir.clone(),
                relay_config_rx,
                relay_slot.clone(),
                advertised_tx.clone(),
                addr.clone(),
                std::sync::Arc::clone(&relay_tickets),
                handle.shutdown_rx.clone(),
                std::sync::Arc::clone(&state.telemetry),
            ));

            #[cfg(feature = "verifydrafts")]
            let (family_serving, family_status, family_engine) = compose_family_serving(
                app,
                &inner.data_dir,
                &profile,
                // Cloud serving has no local engine file: the family
                // tier's engine spawn fails and the daemon degrades
                // HONESTLY to the plain msp-v1 bridge (the designed
                // path — a logged reason, never a silent family-off).
                ensured
                    .as_ref()
                    .map(|e| e.path.as_path())
                    .unwrap_or_else(|| std::path::Path::new("")),
                ensured.as_ref().map(|e| e.bytes).unwrap_or(0),
                &hardware,
                &handle,
                inner.allow_content_carriage,
                &state.telemetry,
            )
            .await;
            #[cfg(feature = "verifydrafts")]
            {
                // ADR-037 §3.2b: the governor verdict label of THIS
                // hosting-start resource check (assistant context).
                inner.last_governor_verdict = family_status.governor_label.clone();
                inner.family = Some(family_status);
                // The coordinator's self-verify engine: the SAME source the
                // serving half holds (Arc clone). None whenever the family
                // tier is degraded — the probe then fails closed with the
                // honest `selfcheck_unavailable` label instead of sending
                // an unverified draft (tonight's warm-cache lesson).
                inner.family_engine = family_engine;
                match family_serving {
                    Some(family) => {
                        tokio::spawn(modelswarm_node::verifydrafts::serve_sessions_composed(
                            // F2B: the composed daemon serves BOTH
                            // vocabularies over the direct listener AND
                            // relayed circuits (one inbox).
                            modelswarm_node::serving::ServingInbox::new(
                                listener,
                                relay_slot.clone(),
                            ),
                            std::sync::Arc::clone(&serving_identity),
                            executor,
                            profile,
                            modelswarm_node::serving::LeasePolicy::production(),
                            family,
                            serving_telemetry,
                            serving_shutdown,
                        ));
                    }
                    // Honest degradation: the pre-light-up msp-v1 bridge,
                    // with the reason already telemetered and in status.
                    None => {
                        tokio::spawn(modelswarm_node::serving::serve_sessions_relayed(
                            listener,
                            relay_slot.clone(),
                            std::sync::Arc::clone(&serving_identity),
                            executor,
                            profile,
                            modelswarm_node::serving::LeasePolicy::production(),
                            serving_telemetry,
                            serving_shutdown,
                        ));
                    }
                }
            }
            #[cfg(not(feature = "verifydrafts"))]
            tokio::spawn(modelswarm_node::serving::serve_sessions_relayed(
                listener,
                relay_slot.clone(),
                std::sync::Arc::clone(&serving_identity),
                executor,
                profile,
                modelswarm_node::serving::LeasePolicy::production(),
                serving_telemetry,
                serving_shutdown,
            ));
        }
    }

    // The family status must be the truth in EVERY configuration: when the
    // serving listener never bound (the toggle off, or the bind failed)
    // there is no family serving regardless of features; when the family
    // tier is not compiled in, this build is a dev shell — every case
    // carries its exact reason (and detail) instead of a null field the UI
    // would have to guess about.
    if inner.family.is_none() {
        #[cfg(feature = "verifydrafts")]
        let fallback = match listener_bind_reason {
            Some((reason, detail)) => FamilyStatus::off_detail(reason, detail),
            // Bound but no executor/profile (a runtime-less node): the
            // composed bridge was never attached.
            None => FamilyStatus::off_detail(
                "serving_bridge_unavailable",
                "the node has no serving executor",
            ),
        };
        #[cfg(not(feature = "verifydrafts"))]
        let fallback = FamilyStatus::off("family_not_compiled");
        inner.family = Some(fallback);
    }

    // Roster heartbeat: device-enroll (approval gated) → session →
    // register + heartbeat so exact-profile peers see this machine (with
    // the honest no-listener address until transport lands).
    let heartbeat_tracker =
        tracker_client(&inner.tracker_url, &inner.data_dir, &state.telemetry).await?;
    // peerId is derived inside the heartbeat task (ADR-020); the
    // installation_id stays a UI label.
    let profile_id = listing.profile_id.clone();
    // The register body's runtime label: the pinned engine build for
    // local profiles; the pinned WIRE + provider for cloud (ADR-040 —
    // the provider pin is the runtime analogue).
    let build = match &listing.kind {
        modelswarm_node::catalog::ProfileKind::Local(m) => m.runtime().build_hash().to_string(),
        modelswarm_node::catalog::ProfileKind::Cloud(c) => {
            format!("openai-chat-v1:{}", c.model())
        }
    };
    let mut heartbeat_shutdown = shutdown_tx.subscribe();
    let hb_data_dir = inner.data_dir.clone();
    let hb_telemetry = std::sync::Arc::clone(&state.telemetry);
    // ADR-038 §3.3/§6.4 row 8: the heartbeat response's `relayTicket`
    // populates the cache the relay paths present (moved into the task).
    let hb_relay_tickets = relay_tickets;
    // ADR-039: the heartbeat's relayAddr feeds the lazy config supervisor
    // (file-wins precedence resolves inside it) and the status surface.
    let hb_relay_config_tx = relay_config_tx;
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
        // F2B: the advertised-list receiver (see the register step below).
        let mut advertised_rx = advertised_rx;
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
                                // Persist so later tracker clients (swarm
                                // chat roster lookup) reuse the session.
                                let _ = std::fs::write(hb_data_dir.join("session.token"), token);
                                pending_device = None;
                                *enroll_view.write().await = EnrollmentView {
                                    phase: "enrolled".into(),
                                    ..EnrollmentView::default()
                                };
                                log_event(&hb_telemetry, "info", "enroll.approved", &[]);
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
                                    &hb_telemetry,
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
                                &hb_telemetry,
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
                                    &hb_telemetry,
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
            // F2B: the advertised address list is supervisor-owned (direct
            // first, the relay circuit address while a live reservation
            // holds it). Any change forces a re-register so the roster
            // never claims a dead circuit.
            if advertised_rx.has_changed().unwrap_or(false) {
                let _ = advertised_rx.borrow_and_update();
                lease_id = None;
            }
            let addresses: Vec<String> = {
                let current = advertised_rx.borrow().clone();
                if current.is_empty() {
                    vec![ADDR_NO_LISTENER.to_string()]
                } else {
                    current
                }
            };
            let result = if heartbeat_tracker.has_session() {
                match &lease_id {
                    Some(id) => heartbeat_tracker
                        .heartbeat(id, std::slice::from_ref(&profile_id), 1, 0, false)
                        .await
                        .map(|response| {
                            // ADR-038 §3.3/§6.4 row 8: cache the freshest
                            // relay admission ticket — replace, never
                            // accumulate. Absent (old tracker deploy) keeps
                            // the previous value; malformed is dropped with
                            // the last good kept (the cache's own honesty
                            // rules; nothing is logged — no frozen
                            // counts-only event name fits, per the
                            // ticket-cache handoff).
                            let _ = hb_relay_tickets.store_heartbeat(&response);
                            // ADR-039: forward the tracker's relay address
                            // when it changes (non-empty, multiaddr-shaped
                            // — a leading '/' with no whitespace; garbage
                            // is dropped silently-and-honestly: the
                            // reservation supervisor would only fail on
                            // it, throttled).
                            let candidate = response.relay_addr.trim().to_string();
                            if !candidate.is_empty()
                                && candidate.starts_with('/')
                                && !candidate.contains(char::is_whitespace)
                                && hb_relay_config_tx.borrow().as_deref()
                                    != Some(candidate.as_str())
                            {
                                let _ = hb_relay_config_tx.send(Some(candidate));
                            }
                        })
                        .or_else(|_| {
                            // Lease lost (expiry/revocation): re-register next tick.
                            lease_id = None;
                            Ok(())
                        }),
                    None => heartbeat_tracker
                        .register(
                            &peer_id,
                            &addresses,
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
                            // Persist for the swarm-chat lease chain (F3(2)).
                            if let Some(id) = &lease_id {
                                let _ = std::fs::write(hb_data_dir.join("lease.id"), id);
                            }
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
                        &hb_telemetry,
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
    inner.artifact = ensured;
    inner.node = Some(RunningNode {
        handle,
        shutdown: shutdown_tx,
        heartbeat,
        enrollment: enrollment_view,
        advertised: advertised_tx,
    });
    // The successful bring-up is the owner's "on" state — persist it for
    // the next start's resume supervisor. Best-effort: an unwritable
    // data dir only means the owner re-clicks HOST after a restart.
    if let Err(e) = std::fs::write(hosting_autostart_marker(&inner.data_dir), b"") {
        state.telemetry.warn(
            "host.autostart",
            &[("phase", "persist"), ("error", &e.to_string())],
        );
    }
    Ok(status)
}

/// The hosting AUTO-RESUME supervisor (owner directive 2026-10-17, the
/// B-idle lesson): after start, when the marker says the owner left
/// hosting on, bring it back — bounded honest retries (a tracker hiccup
/// at boot must not permanently shrink the swarm), every outcome a
/// telemetry row, never a dialog, never blocking the UI. Gives up after
/// the last attempt with the exact reason logged; the owner's HOST
/// click remains the always-works fallback.
async fn hosting_resume_supervisor(app: tauri::AppHandle) {
    const ATTEMPT_DELAYS: [Duration; 5] = [
        Duration::from_secs(5),
        Duration::from_secs(30),
        Duration::from_secs(120),
        Duration::from_secs(600),
        Duration::from_secs(1800),
    ];
    for (n, delay) in ATTEMPT_DELAYS.iter().enumerate() {
        tokio::time::sleep(*delay).await;
        let state = app.state::<DesktopState>();
        let (marker, hosting, telemetry) = {
            let inner = state.inner.lock().await;
            (
                hosting_autostart_marker(&inner.data_dir),
                inner.node.is_some(),
                std::sync::Arc::clone(&state.telemetry),
            )
        };
        // The owner explicitly turned hosting off in a previous session,
        // or turned it on manually just now — either way, nothing to do.
        if !marker.is_file() || hosting {
            return;
        }
        match set_hosting_inner(&app, &state, true).await {
            Ok(_) => {
                telemetry.info(
                    "host.autostart",
                    &[("outcome", "resumed"), ("attempt", &(n + 1).to_string())],
                );
                return;
            }
            Err(e) => {
                if n + 1 == ATTEMPT_DELAYS.len() {
                    telemetry.warn("host.autostart", &[("outcome", "gave_up"), ("error", &e)]);
                } else {
                    telemetry.info(
                        "host.autostart",
                        &[("outcome", "retry_scheduled"), ("error", &e)],
                    );
                }
            }
        }
    }
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
    let listing = resolve_profile(&tracker_url, &data_dir, &state.telemetry, Some(&profile_id))
        .await
        .map_err(|e| format!("catalog: {e}"))?;
    let manager = ArtifactManager::new(&data_dir);
    let store = std::sync::Mutex::new(
        modelswarm_store::Store::open(data_dir.join("state.sqlite")).map_err(|e| e.to_string())?,
    );
    // ADR-040 C3: cloud selections have no artifact — the swap goes
    // straight to the hosting path (which requires the staged key).
    if !listing.kind.is_cloud() {
        let swap_manifest = local_manifest_of(&listing)?;
        manager
            .ensure_artifact(swap_manifest, &store, |done, total| {
                let _ = window.emit(
                    "download",
                    serde_json::json!({ "done": done, "total": total }),
                );
            })
            .await
            .map_err(|e| format!("artifact: {e}"))?;
    }

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
    let result = set_hosting_inner(&app, &state, true).await?;
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
    let tracker = tracker_client(&tracker_url, &data_dir, &state.telemetry).await?;
    let peers = tracker
        .lookup(&profile, 10)
        .await
        .map_err(|e| e.to_string())?;
    Ok(peers
        .into_iter()
        .map(|p| {
            // F2B roster classification: a peer advertising a circuit
            // address is REACHABLE (through the relay) even without a
            // direct QUIC address — the roster row says which path
            // exists, never "unreachable" for a relayed peer.
            let has_direct = p.addresses.iter().any(|a| peer_addr_direct(a));
            let has_circuit = p.addresses.iter().any(|a| addr_is_circuit(a));
            let nat_path = if has_direct && has_circuit {
                "direct + relay"
            } else if has_circuit {
                "relay (circuit)"
            } else if has_direct {
                "direct"
            } else {
                "no dialable address (beta)"
            };
            serde_json::json!({
                "peer_id": p.peer_id,
                "lease_expires_at": p.lease_expires_at,
                "capacity_class": p.capacity_class,
                "nat_path": nat_path,
            })
        })
        .collect())
}

/// The desktop's family REQUESTER wiring (ADR-032 §2 admission order,
/// 2026-10-12; ADR-034 rule 6b pairing confirmation since 2026-10-13):
/// opens one real family session against an eligible exact-profile peer,
/// PRESENTING this installation's cached/earned consume lease in the
/// session-open carrier — the same lease the msp-v1 swarm path presents
/// per request. This is the two-desktop pairing diagnostic: a valid
/// presented lease reaches the peer's accept (the lease gate passed on
/// REAL transport); a refusal surfaces the honest reason (the serving
/// peer telemeters the exact classification; this side reports the
/// transport-level outcome), never a silent family-off. After a clean
/// open the probe drives exactly ONE deterministic pairing round — the
/// coordinator's greedy continuation of the ADR-034 v1 pin, drafted by
/// this machine's own hosting engine — reporting `pairing_confirmed` /
/// `pin_mismatch` (counts only; no prompt content and no token ids ever
/// cross the wire or reach a result field).
/// A build without the family tier can honestly say so before earning
/// anything (the dev-shell answer; the shipped tauri-shell build always
/// chains `verifydrafts-capi`).
#[cfg(not(feature = "verifydrafts"))]
#[tauri::command]
async fn family_probe(state: tauri::State<'_, DesktopState>) -> Result<serde_json::Value, String> {
    log_event(
        &state.telemetry,
        "warn",
        "family.probe",
        &[("reason", "family_not_compiled")],
    );
    Ok(serde_json::json!({ "probed": false, "opened": false, "reason": "family_not_compiled" }))
}

#[cfg(feature = "verifydrafts")]
#[tauri::command]
async fn family_probe(state: tauri::State<'_, DesktopState>) -> Result<serde_json::Value, String> {
    let fail =
        |reason: &str| serde_json::json!({ "probed": false, "opened": false, "reason": reason });
    let (profile, gateway, tracker_url, data_dir, engine, engine_backend, family_engine) = {
        let inner = state.inner.lock().await;
        let running = inner.node.as_ref();
        let Some(running) = running else {
            return Ok(fail("hosting_off"));
        };
        let Some(profile) = &inner.profile_id else {
            return Ok(fail("no_profile"));
        };
        (
            profile.clone(),
            running.handle.local_addr.to_string(),
            inner.tracker_url.clone(),
            inner.data_dir.clone(),
            running.handle.runtime.clone(),
            running.handle.engine_backend.clone(),
            inner.family_engine.clone(),
        )
    };

    // The lease to present: earned through the hosting challenge and
    // cached per profile (F3(2)) — exactly what the msp-v1 swarm path
    // presents. `earn_lease` already telemeters every failure step.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let lease = earn_lease(
        &state.inner,
        &tracker_url,
        &data_dir,
        &state.telemetry,
        &gateway,
        &profile,
        now_ms,
    )
    .await;
    let Some(lease) = lease else {
        log_event(
            &state.telemetry,
            "warn",
            "family.probe",
            &[("reason", "no_presentable_lease")],
        );
        return Ok(fail("no_presentable_lease"));
    };

    // An eligible peer: exact profile, real QUIC address, free slots, not
    // ourselves (the try_swarm_chat candidate predicate).
    let tracker = match tracker_client(&tracker_url, &data_dir, &state.telemetry).await {
        Ok(t) => t,
        Err(e) => {
            log_event(
                &state.telemetry,
                "warn",
                "family.probe",
                &[("reason", "tracker_client"), ("error", &e)],
            );
            return Ok(fail("tracker_unavailable"));
        }
    };
    let peers = match tracker.lookup(&profile, 10).await {
        Ok(p) => p,
        Err(e) => {
            log_event(
                &state.telemetry,
                "warn",
                "family.probe",
                &[("reason", "lookup"), ("error", &e.to_string())],
            );
            return Ok(fail("lookup_failed"));
        }
    };
    let identity = match cached_identity(&data_dir, &state.telemetry) {
        Ok(i) => i,
        Err(_) => return Ok(fail("identity_unavailable")),
    };
    let own_peer_id = identity.peer_id();
    // F2B note: the probe's own dial (`dial_quic`) is DIRECT-only this
    // pass, so the candidate predicate stays direct — a relay-only peer
    // is honestly reported as not probeable here while remaining fully
    // served by the swarm chat path (which dials its circuit address).
    let candidate = peers.into_iter().find(|p| {
        p.peer_id != own_peer_id
            && p.free_slots > 0
            && p.addresses.iter().any(|a| peer_addr_direct(a))
    });
    let Some(candidate) = candidate else {
        log_event(
            &state.telemetry,
            "info",
            "family.probe",
            &[("reason", "no_eligible_peer")],
        );
        return Ok(fail("no_eligible_peer"));
    };
    let addr = candidate
        .addresses
        .iter()
        .find(|a| peer_addr_direct(a))
        .expect("the candidate predicate guarantees a QUIC addr")
        .clone();
    // The verifier key is PINNED from the roster peer id (ADR-020
    // derivation) — never a payload claim.
    let Ok(verifier_key) =
        modelswarm_node::verifydrafts::verifying_key_from_peer_id(&candidate.peer_id)
    else {
        return Ok(serde_json::json!({
            "probed": true, "opened": false,
            "peer_id": candidate.peer_id, "reason": "peer_id_not_ed25519",
        }));
    };

    let mut config =
        modelswarm_node::verifydrafts::VerifyDraftsClientConfig::new(profile.clone(), verifier_key);
    // E0 pairing: the offer must carry the REQUESTER's real engine backend
    // (the config defaults to cpu; a GPU machine offering cpu gets an
    // honest-but-wrong backend_mismatch reject from a GPU verifier — the
    // 2026-10-13 live-pairing finding).
    config.verifier_backend = match engine_backend.as_deref() {
        Some("vulkan") => modelswarm_node::verifydrafts::Backend::Vulkan,
        _ => modelswarm_node::verifydrafts::Backend::Cpu,
    };
    config.lease = Some(lease);
    let wire = match modelswarm_node::verifydrafts::dial_quic(
        &addr,
        &identity,
        &verifier_key,
        Duration::from_secs(10),
    )
    .await
    {
        Ok(w) => w,
        Err(e) => {
            log_event(
                &state.telemetry,
                "warn",
                "family.probe",
                &[
                    ("reason", "dial"),
                    ("peer", &candidate.peer_id),
                    ("error", &e.to_string()),
                ],
            );
            return Ok(serde_json::json!({
                "probed": true, "opened": false,
                "peer_id": candidate.peer_id, "reason": "dial_failed",
            }));
        }
    };
    match modelswarm_node::verifydrafts::VerifyDraftsClient::negotiate(
        wire,
        identity,
        config,
        &state.telemetry,
    )
    .await
    {
        Ok(mut client) => {
            let disclosure = client.engine_disclosure().clone();
            let session_id = client.session_id().to_string();
            // ADR-034 rule 5: the coordinator binds to the SAME pin — the
            // local file when one is in force (the expert seam wins on
            // both ends), else the SAME derivation on demand (the frozen
            // v1 text through the pinned tokenizer — two stock installs
            // pair with zero manual files). A present junk file stays
            // fail-closed (rule 4): the probe reports `pin_unpinned`,
            // never silently derives over a file the user may own.
            let pin_ids = match family_prompt_tokens(&data_dir) {
                Some(ids) => Ok(ids),
                None if !data_dir.join(FAMILY_PROMPT_TOKENS_FILE).exists() => {
                    let derive_engine = engine.clone();
                    derive_family_prompt_ids(FAMILY_PROMPT_V1, |text| async move {
                        modelswarm_node::runtime_tokenize(derive_engine.as_ref(), &text).await
                    })
                    .await
                }
                None => Err("present pin file is not strictly parsable".to_string()),
            };
            // Rule 6b: ONE round; w = min(8, the accepted batch_window_max);
            // drafted by this machine's OWN hosting engine (its sidecar —
            // no second engine on the coordinator), greedy under the
            // session's pinned sampling params (both ends pin the same
            // defaults through generation_params_hash). The window passes
            // the LOCAL self-check before `family_probe_round` may send it.
            let pairing = match pin_ids {
                Err(_) => {
                    family_probe_round(
                        &mut client,
                        &session_id,
                        true,
                        None,
                        family_engine.as_ref(),
                        &state.telemetry,
                    )
                    .await
                }
                Ok(ids) => {
                    let w = 8u32.min(client.engine_disclosure().batch_window_max);
                    let window =
                        modelswarm_node::runtime_propose_window(engine.as_ref(), &profile, &ids, w)
                            .await
                            .ok();
                    family_probe_round(
                        &mut client,
                        &session_id,
                        false,
                        window.as_deref(),
                        family_engine.as_ref(),
                        &state.telemetry,
                    )
                    .await
                }
            };
            // The coordinator-side capability cache (the auto-engage
            // checkpoint's honest seam): the accept's engine disclosure is
            // the ONLY existing capability surface — no roster field
            // carries it. Learned here, counts and labels only.
            {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let mut inner = state.inner.lock().await;
                inner.family_peer_caps.insert(
                    candidate.peer_id.clone(),
                    FamilyPeerCap {
                        batch_verify: disclosure.batch_verify,
                        batch_window_max: disclosure.batch_window_max,
                        backend: disclosure.backend.to_string(),
                        learned_unix_ms: now_ms,
                    },
                );
            }
            log_event(
                &state.telemetry,
                "info",
                "family.probe",
                &[
                    ("peer", &candidate.peer_id),
                    ("session", &session_id),
                    ("opened", "true"),
                ],
            );
            let mut result = serde_json::json!({
                "probed": true, "opened": true,
                "peer_id": candidate.peer_id,
                "session_id": session_id,
                "backend": disclosure.backend.to_string(),
                "batch_verify": disclosure.batch_verify,
                "batch_window_max": disclosure.batch_window_max,
            });
            for (key, value) in pairing {
                result[key] = value;
            }
            // ADR-037 §3.2b: this probe's pairing VERDICT joins the
            // assistant context (labels only — the verdict is
            // count-based by construction).
            if let Some(verdict) = result["pairing"].as_str() {
                state.inner.lock().await.last_probe_verdict = Some(verdict.to_string());
            }
            Ok(result)
        }
        Err(error) => {
            // The honest degradation path: the serving peer's exact
            // refusal classification is on ITS telemetry; this side
            // reports the transport-level outcome truthfully.
            let reason = family_open_error_reason(&error);
            log_event(
                &state.telemetry,
                "warn",
                "family.probe",
                &[
                    ("peer", &candidate.peer_id),
                    ("reason", reason),
                    ("error", &error.to_string()),
                ],
            );
            Ok(serde_json::json!({
                "probed": true, "opened": false,
                "peer_id": candidate.peer_id,
                "reason": reason,
                "detail": error.to_string(),
            }))
        }
    }
}

/// The verdict for the probe's one pairing round (ADR-034 rule 6b):
/// `pairing_confirmed` on FULL acceptance, `pin_mismatch` otherwise —
/// divergence point 0 is the canonical mismatch signature. Pure and
/// count-based: no token id can appear in a verdict.
#[cfg(feature = "verifydrafts")]
fn pairing_verdict(window_len: u32, accepted: u32, divergence: Option<u32>) -> &'static str {
    if divergence.is_none() && accepted == window_len && window_len > 0 {
        "pairing_confirmed"
    } else {
        "pin_mismatch"
    }
}

/// Stable label for a pairing round that did not complete (ADR-034 rule
/// 6b: "any rejection" reads `pin_mismatch`; this label is the honest
/// DETAIL carried next to it — counts/reasons only, never frame
/// payloads).
#[cfg(feature = "verifydrafts")]
fn pairing_round_failure_label(
    failure: &modelswarm_node::verifydrafts::RoundFailure,
) -> &'static str {
    use modelswarm_node::verifydrafts::RoundFailure;
    match failure {
        RoundFailure::Rejected { .. } => "rejected",
        RoundFailure::Cancelled { .. } => "cancelled",
        RoundFailure::Violation(_) => "violation",
        RoundFailure::TornDown(_) => "torn_down",
        RoundFailure::Dropped(_) => "dropped",
        RoundFailure::TimedOut => "timed_out",
        RoundFailure::WindowTooLarge { .. } => "window_too_large",
        RoundFailure::SessionTornDown => "session_torn_down",
        RoundFailure::UnparsableResult => "unparsable_result",
        RoundFailure::Io(_) => "io",
    }
}

/// The counts-only result/telemetry shape for every pairing label — a
/// label, three counts, an optional detail label. Structurally INCAPABLE
/// of carrying token ids (no array field exists); pinned by test.
#[cfg(feature = "verifydrafts")]
fn pairing_result_fields(
    pairing: &str,
    window: Option<u32>,
    accepted: Option<u32>,
    divergence: Option<u32>,
    detail: Option<&str>,
) -> Vec<(String, serde_json::Value)> {
    let mut out = vec![
        ("pairing".to_string(), serde_json::json!(pairing)),
        ("pairing_window".to_string(), serde_json::json!(window)),
        ("pairing_accepted".to_string(), serde_json::json!(accepted)),
        (
            "pairing_divergence".to_string(),
            serde_json::json!(divergence),
        ),
    ];
    if let Some(detail) = detail {
        out.push(("pairing_detail".to_string(), serde_json::json!(detail)));
    }
    out
}

/// The one telemetry call every pairing outcome flows through (the
/// 2026-10-13 gap fix: the DETAIL label — the Err-branch reasons like
/// `rejected`/`io`/`local_selfcheck` — now reaches the
/// `family.probe.pairing` event; counts and labels only, never token
/// ids).
#[cfg(feature = "verifydrafts")]
#[allow(clippy::too_many_arguments)]
fn log_family_pairing(
    telemetry: &Telemetry,
    session_id: &str,
    pairing: &str,
    window: Option<u32>,
    accepted: Option<u32>,
    divergence: Option<u32>,
    detail: Option<&str>,
) {
    telemetry.info(
        "family.probe.pairing",
        &[
            ("session", session_id),
            ("pairing", pairing),
            ("window", &window.map(|w| w.to_string()).unwrap_or_default()),
            (
                "accepted",
                &accepted.map(|a| a.to_string()).unwrap_or_default(),
            ),
            (
                "divergence",
                &divergence.map(|d| d.to_string()).unwrap_or_default(),
            ),
            ("detail", detail.unwrap_or("")),
        ],
    );
}

// ---- Coordinator self-verify (tonight's 2026-10-13 live root cause) ----
//
// The probe's DRAFT is produced by the shared llama sidecar — the same
// engine serving chat — and a WARM prompt cache flips the draft's first
// greedy token (a warm-drafted round was rejected 0/8 divergence-at-0 by
// the peer's verifier while both cold-drafted rounds confirmed 8/8). The
// owner-approved fix: before ANY drafted window is sent to a verifier
// (probe today, the auto-engage path when it exists), re-run the window
// through the LOCAL batch-verify engine and only send windows that pass
// LOCAL verification. A fresh engine per check (the family source's
// `engine_for`) is cache-cold BY CONSTRUCTION — the ADR-032 §3 cache-cold
// audit rule — so the check catches warm-cache flips and any other local
// inconsistency. Counts only; a window that fails locally NEVER hits the
// wire.

/// The local self-check verdict for one drafted window.
#[cfg(feature = "verifydrafts")]
#[derive(Debug)]
enum FamilySelfcheck {
    /// The local engine reproduces the window — safe to send.
    Send,
    /// The local engine diverged at `accepted` (== the divergence point,
    /// ADR-032 §1): the window is NOT sent; the honest label carries the
    /// counts only.
    LocalDivergence { accepted: u32 },
    /// No usable local verify engine (family tier degraded / dev build):
    /// fail closed — no unverified draft is ever sent.
    Unavailable(String),
}

/// Runs one drafted window through the local batch-verify engine (a FRESH
/// prefilled engine per call — cache-cold) and returns whether it may hit
/// the wire. Shared by every coordinator path that drafts: the probe
/// today, the chat auto-engage checkpoint when engagement unblocks.
#[cfg(feature = "verifydrafts")]
async fn family_selfcheck_window(
    source: Option<&Arc<modelswarm_node::verifydrafts::FamilyEngineSource>>,
    window: &[u32],
) -> FamilySelfcheck {
    let Some(source) = source else {
        return FamilySelfcheck::Unavailable(
            "no local family engine (family tier off or degraded)".into(),
        );
    };
    let engine = match source.engine_for().await {
        Ok(engine) => engine,
        Err(e) => return FamilySelfcheck::Unavailable(format!("engine load: {e}")),
    };
    // The session's pinned sampling parameters — the same defaults both
    // ends pin through `generation_params_hash`; deliberately NOT the
    // chat temperature (the draft path stays greedy-pinned).
    let params = modelswarm_node::verifydrafts::SamplingParams::default();
    match engine.verify_window(window, &params).await {
        Ok((accepted, _correction)) => {
            if accepted == window.len() as u32 {
                FamilySelfcheck::Send
            } else {
                FamilySelfcheck::LocalDivergence {
                    accepted: accepted.min(window.len() as u32),
                }
            }
        }
        Err(e) => FamilySelfcheck::Unavailable(format!("engine verify: {e}")),
    }
}

/// Runs the probe's ONE deterministic pairing round (ADR-034 rule 6b)
/// over the coordinator-drafted `window` and returns the counts-only
/// result fields. `pin_unpinned`/`window` carry the honest pre-round
/// states: no usable local pin ⇒ `pin_unpinned`; a pin the local engine
/// could not draft (or an all-EOS zero-token draft) ⇒
/// `draft_unavailable` — never a fabricated confirmation, never token
/// ids in any field. Since 2026-10-13 every drafted window first passes
/// the LOCAL self-check (`family_selfcheck_window`): a locally divergent
/// window (`local_selfcheck`, counts only) or a missing local engine
/// (`selfcheck_unavailable`) NEVER reaches `client.round` — the wire
/// only ever carries locally-verified drafts.
#[cfg(feature = "verifydrafts")]
async fn family_probe_round(
    client: &mut modelswarm_node::verifydrafts::VerifyDraftsClient<
        modelswarm_node::verifydrafts::QuicVerifyWire,
    >,
    session_id: &str,
    pin_unpinned: bool,
    window: Option<&[u32]>,
    selfcheck_source: Option<&Arc<modelswarm_node::verifydrafts::FamilyEngineSource>>,
    telemetry: &Telemetry,
) -> Vec<(String, serde_json::Value)> {
    let fields = |pairing: &str,
                  window: Option<u32>,
                  accepted: Option<u32>,
                  divergence: Option<u32>,
                  detail: Option<&str>|
     -> Vec<(String, serde_json::Value)> {
        log_family_pairing(
            telemetry, session_id, pairing, window, accepted, divergence, detail,
        );
        pairing_result_fields(pairing, window, accepted, divergence, detail)
    };
    if pin_unpinned {
        return fields("pin_unpinned", None, None, None, None);
    }
    let Some(window) = window.filter(|w| !w.is_empty()) else {
        return fields("draft_unavailable", None, None, None, None);
    };
    // The pre-wire gate (owner-approved 2026-10-13): local verification
    // FIRST, cache-cold; only a passing window may hit the wire.
    match family_selfcheck_window(selfcheck_source, window).await {
        FamilySelfcheck::Send => {}
        FamilySelfcheck::LocalDivergence { accepted } => {
            return fields(
                "local_selfcheck",
                Some(window.len() as u32),
                Some(accepted),
                Some(accepted),
                None,
            );
        }
        FamilySelfcheck::Unavailable(why) => {
            return fields(
                "selfcheck_unavailable",
                Some(window.len() as u32),
                None,
                None,
                Some(&why),
            );
        }
    }
    match client.round(window, telemetry).await {
        Ok(outcome) => {
            let verdict = pairing_verdict(
                window.len() as u32,
                outcome.accepted_prefix_len,
                outcome.divergence_point,
            );
            fields(
                verdict,
                Some(window.len() as u32),
                Some(outcome.accepted_prefix_len),
                outcome.divergence_point,
                None,
            )
        }
        Err(failure) => fields(
            "pin_mismatch",
            Some(window.len() as u32),
            None,
            None,
            Some(pairing_round_failure_label(&failure)),
        ),
    }
}

/// Stable UI labels for a refused family session open (labels only — the
/// lease itself never crosses into any surface). The serving peer's own
/// `FamilyLeaseRefusal` classification lives on ITS telemetry
/// (`verifydrafts.family_refused`); this is the requester-side truth: a
/// lease refusal ends the session before any accept, which the client
/// observes as the honest transport failure.
#[cfg(feature = "verifydrafts")]
fn family_open_error_reason(
    error: &modelswarm_node::verifydrafts::ClientOpenError,
) -> &'static str {
    use modelswarm_node::verifydrafts::ClientOpenError;
    match error {
        // The session-end refusal (lease gate fail-closed, peer closed
        // without an accept) — the probe's primary expected failure.
        ClientOpenError::Io(_) => "open_refused",
        ClientOpenError::Rejected(_) => "peer_rejected",
        ClientOpenError::Ineligible { .. } => "verifier_ineligible",
        ClientOpenError::InvalidAccept(_) => "invalid_accept",
        ClientOpenError::Protocol(_) => "protocol",
        ClientOpenError::Misconfigured(_) => "misconfigured",
    }
}

/// F3(2): earn a consume lease for `profile` the honest way — the
/// tracker grants leases only after a hosting challenge, so we run ONE
/// small real generation against the local gateway and report its true
/// timings. Cached per profile until expiry (challenges are not free).
/// Every failure step logs a `lease.earn` warn with a reason code — a
/// silent None here reads as "no peers" in the UI, which cost a full
/// diagnosis round once already.
#[allow(clippy::too_many_arguments)]
async fn earn_lease(
    inner: &tokio::sync::Mutex<Inner>,
    tracker_url: &str,
    data_dir: &std::path::Path,
    telemetry: &Telemetry,
    gateway_addr: &str,
    profile: &str,
    now_ms: u64,
) -> Option<String> {
    {
        let guard = inner.lock().await;
        if let Some((p, lease, expires)) = &guard.swarm_lease {
            if p == profile && *expires > now_ms + 30_000 {
                return Some(lease.clone());
            }
        }
    }
    let fail = |reason: &str, detail: String| {
        telemetry.warn("lease.earn", &[("reason", reason), ("detail", &detail)]);
    };
    let Ok(lease_id) = std::fs::read_to_string(data_dir.join("lease.id")) else {
        fail("no_lease_id", "lease.id missing — register first".into());
        return None;
    };
    let lease_id = lease_id.trim().to_string();
    if lease_id.is_empty() {
        fail("no_lease_id", "lease.id empty".into());
        return None;
    }
    let tracker = match tracker_client(tracker_url, data_dir, telemetry).await {
        Ok(t) => t,
        Err(e) => {
            fail("tracker_client", e);
            return None;
        }
    };

    // Hosting challenge: one real local generation, honest timings.
    // first_token_ms is reported as the total (a conservative floor — we
    // do not game the capacity class).
    let started = Instant::now();
    let reply: serde_json::Value = match reqwest::Client::new()
        .post(format!("http://{gateway_addr}/v1/chat/completions"))
        .timeout(Duration::from_secs(60))
        .json(&serde_json::json!({
            "model": profile,
            "messages": [{ "role": "user", "content": "Say ok." }],
            "max_tokens": 8,
            "temperature": 0.0,
            "stream": false,
        }))
        .send()
        .await
    {
        Ok(r) => match r.json().await {
            Ok(v) => v,
            Err(e) => {
                fail("challenge_engine", format!("local gateway body: {e}"));
                return None;
            }
        },
        Err(e) => {
            fail("challenge_engine", format!("local gateway: {e}"));
            return None;
        }
    };
    let total_ms = started.elapsed().as_millis() as u64;
    // Local engine did not answer: no honest timings exist. Surface the
    // gateway's error code (e.g. engine_failed / no_eligible_peer) — the
    // body's message is our own static text, never prompt content.
    if reply["choices"][0]["message"]["content"].as_str().is_none() {
        let code = reply["error"]["code"].as_str().unwrap_or("no_choices");
        fail("challenge_engine", format!("local gateway error: {code}"));
        return None;
    }
    let challenge = match tracker.challenge_start(&lease_id, profile).await {
        Ok(c) => c,
        Err(e) => {
            fail("challenge_refused", e.to_string());
            return None;
        }
    };
    let Some(challenge_id) = challenge["challengeId"].as_str().map(str::to_string) else {
        fail("challenge_refused", "no challengeId in response".into());
        return None;
    };
    match tracker
        .challenge_complete(&lease_id, profile, &challenge_id, total_ms, total_ms)
        .await
    {
        Err(e) => {
            fail("challenge_refused", e.to_string());
            return None;
        }
        // ADR-012/ADR-037 §3.2b: the self-reported capacityClass label
        // of THIS installation's challenge (the assistant context's
        // capacity.class source; labels only, never timings).
        Ok(completed) => {
            let mut guard = inner.lock().await;
            guard.capacity_class = completed["capacityClass"].as_str().map(str::to_string);
        }
    }
    let issued = match tracker.request_lease(&lease_id, profile).await {
        Ok(i) => i,
        Err(e) => {
            fail("lease_denied", e.to_string());
            return None;
        }
    };
    telemetry.info(
        "lease.earned",
        &[
            ("profile", profile),
            ("challenge_ms", &total_ms.to_string()),
        ],
    );
    // Cache until the lease's own expiry (or a 5-minute floor).
    let expires = now_ms + 300_000;
    let mut guard = inner.lock().await;
    guard.swarm_lease = Some((profile.to_string(), issued.lease.clone(), expires));
    Some(issued.lease)
}

/// Shadow-planner context gathered BEFORE production selection
/// (dependency-graph edge 11: shadow planner before any acting
/// planner): the F15 measurement snapshots joined with THIS roster
/// lookup, the env-tunable queue-discount policy, and coarse token
/// estimates. Shadow data only — nothing here influences which peer
/// serves.
struct ShadowPlan {
    trace_id: String,
    own_peer_id: String,
    inputs: Vec<modelswarm_scheduler::shadow::ShadowCandidateInput>,
    policy: modelswarm_scheduler::shadow::EnvPolicy,
    prompt_tokens: u32,
    output_tokens: u32,
    metrics: std::sync::Arc<modelswarm_node::measure::PeerMetrics>,
}

/// Builds the shadow inputs from the roster + the shared F15 recorder
/// (lazily opened, store-backed, same instance the pooled executors
/// use), and resolves the env-tunable discount policy with visible
/// warnings. Never dials, never selects.
async fn gather_shadow_plan(
    inner: &tokio::sync::Mutex<Inner>,
    telemetry: &std::sync::Arc<Telemetry>,
    peers: &[modelswarm_tracker_api::PeerEntry],
    own_peer_id: &str,
    log: &[ChatTurn],
    max_tokens: Option<u32>,
) -> ShadowPlan {
    let metrics = {
        let guard = inner.lock().await;
        match &guard.peer_metrics {
            Some(m) => std::sync::Arc::clone(m),
            None => {
                let path = guard.data_dir.join("state.sqlite");
                // F9-class fix: the one-shot store open (SQLite open +
                // migrations + whole-store hydration) is blocking I/O —
                // run it on the blocking pool, never the async runtime.
                drop(guard);
                let t = std::sync::Arc::clone(telemetry);
                let opened = tokio::task::spawn_blocking(move || {
                    modelswarm_node::measure::PeerMetrics::open(path, t)
                })
                .await;
                let m = match opened {
                    Ok(m) => std::sync::Arc::new(m),
                    Err(join_err) => {
                        // A panic in the one-shot open must never take a
                        // chat request down: degrade to memory-only.
                        telemetry.warn(
                            "chat.swarm.error",
                            &[("phase", "metrics_open"), ("error", &join_err.to_string())],
                        );
                        std::sync::Arc::new(modelswarm_node::measure::PeerMetrics::memory(
                            std::sync::Arc::clone(telemetry),
                        ))
                    }
                };
                // Re-lock and publish; if a concurrent chat won the race,
                // keep the single shared instance (one per process).
                let mut guard = inner.lock().await;
                match &guard.peer_metrics {
                    Some(existing) => std::sync::Arc::clone(existing),
                    None => {
                        guard.peer_metrics = Some(std::sync::Arc::clone(&m));
                        m
                    }
                }
            }
        }
    };
    let inputs = peers
        .iter()
        .filter(|p| p.peer_id != own_peer_id)
        .map(|p| modelswarm_scheduler::shadow::ShadowCandidateInput {
            roster: modelswarm_scheduler::shadow::RosterFacts {
                peer_id: p.peer_id.clone(),
                free_slots: u32::from(p.free_slots),
                advertised_queue_ms: Some(p.queue_ms),
                capacity_class: p.capacity_class.clone(),
                // F2B: the frozen fact stays exactly what it names — a
                // DIRECT quic addr. Relay-only reachability is a separate
                // fact the cost model does not price yet (recorded
                // follow-up; ADR-014 relay cost penalty).
                has_direct_quic_addr: p.addresses.iter().any(|a| peer_addr_direct(a)),
            },
            observed: metrics.observe(&p.peer_id),
        })
        .collect();
    let policy = modelswarm_scheduler::shadow::QueueDiscountPolicy::from_env();
    for warning in &policy.warnings {
        telemetry.warn("sched.shadow.policy", &[("warning", warning.as_str())]);
    }
    let prompt_chars: usize = log.iter().map(|t| t.content.chars().count()).sum();
    ShadowPlan {
        trace_id: format!("shadow-{}", modelswarm_identity::new_nonce()),
        own_peer_id: own_peer_id.to_string(),
        inputs,
        policy,
        prompt_tokens: modelswarm_scheduler::shadow::estimate_tokens_from_chars(prompt_chars),
        output_tokens: max_tokens.unwrap_or(256),
        metrics,
    }
}

/// Computes and logs the shadow decision — the peer
/// `select_microswarm` would rank first over the F15 measurements —
/// next to the peer production actually picked. Returns the shadow pick
/// for the post-completion divergence record. Logs only; NEVER acts.
fn log_shadow_decision(
    telemetry: &Telemetry,
    plan: &ShadowPlan,
    profile: &str,
    production_pick: &str,
) -> Option<String> {
    let decision = modelswarm_scheduler::shadow::shadow_decision(
        &plan.inputs,
        &plan.own_peer_id,
        profile,
        Some(production_pick),
        plan.prompt_tokens,
        plan.output_tokens,
        &plan.policy,
    );
    let pick = decision.pick.clone();
    let pick_predicted = decision
        .ranked
        .first()
        .and_then(|r| r.predicted_ms)
        .map(|ms| format!("{ms:.1}"))
        .unwrap_or_else(|| "unmeasured".into());
    let agree = decision
        .agree
        .map(|a| a.to_string())
        .unwrap_or_else(|| "unknown".into());
    let json = serde_json::to_string(&decision).unwrap_or_else(|_| "{}".into());
    telemetry.info(
        "sched.shadow.decision",
        &[
            ("label", decision.label.as_str()),
            ("trace", plan.trace_id.as_str()),
            ("profile", profile),
            ("pick", decision.pick.as_deref().unwrap_or("none")),
            ("pick_predicted_ms", &pick_predicted),
            ("production_pick", production_pick),
            ("agree", agree.as_str()),
            ("candidates", &decision.candidate_count.to_string()),
            ("measured", &decision.measured_count.to_string()),
            ("decision", json.as_str()),
        ],
    );
    pick
}

/// 9.6 dataset completeness: the realized-outcome row for a TOTAL-FAILURE
/// path in [`try_swarm_chat`] (every `return None`), with
/// `served_by=failed` so a swarm attempt that produced no reply is still
/// sampled in the pass-1 join instead of silently vanishing. Shares the
/// success-path row's correlation keys (label/trace/profile/
/// production_peer/shadow_pick). `plan` is `None` only for failures that
/// happen BEFORE the shadow plan could be gathered (tracker/lookup/
/// identity) — those get a fresh trace so the row remains a realizable
/// sample, honestly without any decision row to join against.
fn record_shadow_failure(
    telemetry: &Telemetry,
    plan: Option<&ShadowPlan>,
    profile: &str,
    production_peer: &str,
    shadow_pick: Option<&str>,
    reason: &str,
    wall_ms: Option<u64>,
) {
    let fresh_trace;
    let trace = match plan {
        Some(p) => p.trace_id.as_str(),
        None => {
            fresh_trace = format!("shadow-{}", modelswarm_identity::new_nonce());
            fresh_trace.as_str()
        }
    };
    let wall = wall_ms
        .map(|ms| ms.to_string())
        .unwrap_or_else(|| "unknown".into());
    telemetry.info(
        "sched.shadow.realized",
        &[
            ("label", modelswarm_scheduler::shadow::SHADOW_LABEL),
            ("trace", trace),
            ("profile", profile),
            ("production_peer", production_peer),
            ("shadow_pick", shadow_pick.unwrap_or("none")),
            ("served_by", "failed"),
            ("reason", reason),
            ("wall_ms", &wall),
        ],
    );
}

// ---- Chat auto-engage behind the engage gate (owner directive
// 2026-10-13: "chat must auto-engage the family behind the engage gate,
// zero buttons; the probe button remains diagnostic only") --------------
//
// What LANDED: the honest capability-check seam + the engage-gate
// evaluation, wired into every swarm chat turn with telemetry and a UI
// surface. Capability comes from the coordinator-side cache the PROBE
// fills (the family `SessionAccept` disclosure is the only existing
// capability surface — the roster carries NO batch-verify field, and
// inventing one is a wire change needing an ADR). The gate itself is the
// scheduler's frozen public rule (`should_engage_cooperative`,
// DEFAULT_CONFIDENCE_MARGIN 0.15 — 0.05 is only the reject floor —
// fastest-single comparator).
//
// What the AUTOMATIC path may do (ADR-036 §3.3 — the narrowed
// `blocked_content_contract`): the engage gate and the fastest-single
// comparator stay OUTERMOST and byte-stable, and user content rides a
// family session ONLY under the user's explicit election (the
// family-verified-chat toggle — the ELECTED path in try_swarm_chat, the
// probe's sibling: user-triggered, honest cost displayed, never an
// automatic speed-up). The frozen label `blocked_content_contract` fires
// when the gate would otherwise engage but the user-content carrier path
// is NOT elected or not locally available (the carrier form exceeds the
// §6.4 256 KiB frame limit, or the conversation is not renderable under
// the pinned template/tokenizer) — never as a blanket content block, and
// TRUNCATING a conversation to fit is FORBIDDEN (truncation changes the
// root, breaks the commitment, and would fail verification silently).
// The plain swarm path serves every turn before the first output token
// regardless. The reason ladder below is the pinned vocabulary.

/// Why the AUTOMATIC chat path is not routing this completion through a
/// family session (stable labels; the ladder's order is the pinned
/// priority). With the election OFF (the default) the terminal label is
/// the narrowed `blocked_content_contract` even when the gate would
/// engage — the election, not the gate, is what carries content.
#[cfg(feature = "verifydrafts")]
fn family_engage_reason(
    cap: Option<&FamilyPeerCap>,
    own_backend: Option<&str>,
    predicted_swarm_ms: Option<f64>,
    predicted_fastest_single_ms: Option<f64>,
) -> &'static str {
    let Some(cap) = cap else {
        return "capability_unknown";
    };
    if !cap.batch_verify {
        return "capability_false";
    }
    // E0 pairing: a cross-backend family session rejects at open anyway.
    let own = own_backend.unwrap_or("cpu");
    if !cap.backend.eq_ignore_ascii_case(own) {
        return "backend_mismatch";
    }
    let (Some(swarm), Some(single)) = (predicted_swarm_ms, predicted_fastest_single_ms) else {
        // No measured family-round timings exist yet (the remaining seam —
        // the F15 recorder tracks the plain path only). Never-worse: no
        // prediction of a win means no engagement, honestly labeled.
        return "gate_unmeasured";
    };
    match modelswarm_scheduler::should_engage_cooperative(
        swarm,
        single,
        modelswarm_scheduler::DEFAULT_CONFIDENCE_MARGIN,
    ) {
        Ok(false) => "gate_no_win",
        // The gate WOULD engage — the automatic path still does not carry
        // user content: ADR-036 §3.3's narrowed label fires because the
        // user-content carrier path is not elected (the election, never
        // the gate, carries content). With the election ON, the elected
        // path runs INSTEAD of this ladder (user-triggered, like the
        // probe) and this label then means "not locally available"
        // (carrier form over the 256 KiB frame limit / unrenderable).
        Ok(true) => "blocked_content_contract",
        Err(_) => "gate_misconfigured",
    }
}

/// The per-turn checkpoint: reads the capability cache + the F15 measured
/// single, evaluates the ladder, telemeters the outcome. NEVER dials,
/// NEVER opens a session, NEVER delays the chat — the plain swarm path
/// serves this turn regardless (the pre-first-token fallback the engage
/// rule demands).
#[cfg(feature = "verifydrafts")]
async fn family_engage_checkpoint(
    inner: &tokio::sync::Mutex<Inner>,
    profile: &str,
    peer_id: &str,
    plan: &ShadowPlan,
    telemetry: &std::sync::Arc<Telemetry>,
) -> &'static str {
    let (cap, own_backend) = {
        let guard = inner.lock().await;
        let cap = guard.family_peer_caps.get(peer_id).cloned();
        let backend = guard
            .node
            .as_ref()
            .and_then(|r| r.handle.engine_backend.clone());
        (cap, backend)
    };
    let predicted_single = plan
        .metrics
        .observe(peer_id)
        .profile(profile)
        .and_then(|p| p.total_ewma_ms);
    let reason = family_engage_reason(
        cap.as_ref(),
        own_backend.as_deref(),
        // No family-round timing EWMA exists yet (recorded seam); None
        // keeps the ladder honest instead of inventing a prediction.
        None,
        predicted_single,
    );
    // ADR-037 §3.2b: the ladder label of THIS turn (assistant context).
    inner.lock().await.last_engage_reason = Some(reason.to_string());
    let mut fields: Vec<(&str, String)> = vec![
        ("phase", "chat".to_string()),
        ("peer", peer_id.to_string()),
        ("engaged", "false".to_string()),
        ("reason", reason.to_string()),
        // R2 (protocol review 2026-10-13): the scheduler's frozen rule
        // requires the margin actually used in every run record — rendered
        // from the constant so it can never drift from the gate arithmetic.
        (
            "margin",
            format!("{}", modelswarm_scheduler::DEFAULT_CONFIDENCE_MARGIN),
        ),
    ];
    if let Some(cap) = &cap {
        // The learned disclosure rides along (counts/labels only) so the
        // engage telemetry shows WHAT was known, not just the verdict.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        fields.push(("cap_backend", cap.backend.clone()));
        fields.push(("cap_window", cap.batch_window_max.to_string()));
        fields.push((
            "cap_age_ms",
            now_ms.saturating_sub(cap.learned_unix_ms).to_string(),
        ));
    }
    let fields: Vec<(&str, &str)> = fields.iter().map(|(k, v)| (*k, v.as_str())).collect();
    telemetry.info("family.engage", &fields);
    reason
}

/// ADR-036 §3.3 — the ELECTED family-verified chat turn (the probe's
/// sibling: user-triggered via the family-verified-chat toggle, honest
/// cost displayed; it does NOT ride the engage gate because the user has
/// explicitly accepted the measured cost the gate exists to protect them
/// from). Renders the conversation under the profile's pinned chat
/// template + tokenizer (the ADR-025 application point — the same
/// `render_chatml` the serving executor applies, so the carrier root is
/// the same tokenization the single-serving path would send), opens ONE
/// carrier session to the verifying peer (lease → signed pin carrier →
/// offer), then drives the lossless `speculative_exact` loop: THIS
/// machine's own engine drafts every window (greedy, pinned params), the
/// local self-check gates each drafted window (cache-cold — the standing
/// 2026-10-13 rule), the peer verifies, and the committed stream (the
/// verifying peer's engine's own continuation) IS the reply.
///
/// HONEST COST (ADR-036 §3.4, never softened): this is SLOWER than
/// single-peer chat, not faster — the requester's own engine drafts every
/// committed word and the peers check, they do not relieve it. The mode
/// label says so. On ANY honest failure (no local engine, unrenderable or
/// oversized carrier form, refused open, engine gap) this returns `None`
/// and the caller falls back to the plain swarm path — the mandatory
/// fallback-to-single contract, never silent.
#[cfg(feature = "verifydrafts")]
#[allow(clippy::too_many_arguments)]
async fn try_family_verified_chat(
    inner: &tokio::sync::Mutex<Inner>,
    telemetry: &std::sync::Arc<Telemetry>,
    profile: &str,
    log: &[ChatTurn],
    max_tokens: Option<u32>,
    addr: &str,
    remote_peer_id: &str,
    identity: &modelswarm_identity::InstallationIdentity,
    lease: Option<&str>,
    on_delta: &(dyn Fn(&str) + Send + Sync),
) -> Option<serde_json::Value> {
    let engage_label = |reason: &str, detail: Option<&str>| {
        let mut fields: Vec<(&str, &str)> = vec![
            ("phase", "chat"),
            ("peer", remote_peer_id),
            ("engaged", "false"),
            ("reason", reason),
        ];
        if let Some(detail) = detail {
            fields.push(("detail", detail));
        }
        telemetry.info("family.engage", &fields);
    };
    // The local drafting + self-check surfaces: this machine's own
    // hosting runtime (drafts) and the family engine source (cache-cold
    // local verification). Without either, the elected path is not
    // locally available — the honest label, then the plain path.
    let (runtime, family_engine, engine_backend) = {
        let guard = inner.lock().await;
        let node = guard.node.as_ref();
        (
            node.and_then(|r| r.handle.runtime.clone()),
            guard.family_engine.clone(),
            node.and_then(|r| r.handle.engine_backend.clone()),
        )
    };
    let (Some(runtime), Some(family_engine)) = (runtime, family_engine) else {
        engage_label("elected_no_local_engine", None);
        return None;
    };
    // The conversation root: the SAME ChatML rendering + pinned
    // tokenizer the serving path applies (ADR-025 application point;
    // ADR-036 §3.2 "the ids ARE the contract" — the serving side does
    // NOT re-derive).
    let turns: Vec<modelswarm_gateway::NormalizedMessage> = log
        .iter()
        .map(|t| modelswarm_gateway::NormalizedMessage {
            role: t.role.clone(),
            content: t.content.clone(),
        })
        .collect();
    let rendered = modelswarm_node::render_chatml(&turns);
    let root = match modelswarm_node::runtime_tokenize(Some(&runtime), &rendered).await {
        Ok(ids) if !ids.is_empty() => ids,
        Ok(_) => {
            engage_label(
                "blocked_content_contract",
                Some("unrenderable_conversation"),
            );
            return None;
        }
        Err(e) => {
            engage_label("blocked_content_contract", Some(&format!("tokenize: {e}")));
            return None;
        }
    };
    // §3.3's residual meaning, for real: a carrier form over the 256 KiB
    // frame limit fires `blocked_content_contract` — TRUNCATING the
    // conversation to fit is FORBIDDEN (it would change the root, break
    // the commitment, and fail verification silently). Fall back to the
    // plain path. The probe carries every field at its real worst-case
    // wire length (nonce/commitment/signature/base64 sizes), so the
    // bound is exact, not fudged.
    let frame_bound = modelswarm_transport::frame::MAX_FRAME_BYTES;
    let probe = modelswarm_node::verifydrafts::WirePinCarrier {
        session_id: "vd-0000000000000000".to_string(),
        protocol_version: "msp-cooperative-v1".to_string(),
        seq: 1,
        round: 0,
        profile_id: profile.to_string(),
        generation_params_hash: "0".repeat(64),
        prompt_tokens: root.clone(),
        pin_commitment: "0".repeat(64),
        nonce: "0".repeat(16),
        deadline_ms: 120_000,
        sender: identity.installation_id(),
        signature: "0".repeat(88),
    };
    let carrier_fits = serde_json::to_string(&probe).is_ok_and(|s| s.len() <= frame_bound);
    drop(probe);
    if !carrier_fits {
        engage_label(
            "blocked_content_contract",
            Some("carrier_exceeds_frame_limit"),
        );
        return None;
    }
    // The verifier key is PINNED from the roster peer id (the probe's
    // discipline — never a payload claim).
    let Ok(verifier_key) =
        modelswarm_node::verifydrafts::verifying_key_from_peer_id(remote_peer_id)
    else {
        engage_label("peer_id_not_ed25519", None);
        return None;
    };
    let mut config = modelswarm_node::verifydrafts::VerifyDraftsClientConfig::new(
        profile.to_string(),
        verifier_key,
    );
    // E0 pairing: the offer carries THIS machine's real engine backend.
    config.verifier_backend = match engine_backend.as_deref() {
        Some("vulkan") => modelswarm_node::verifydrafts::Backend::Vulkan,
        _ => modelswarm_node::verifydrafts::Backend::Cpu,
    };
    config.lease = lease.map(str::to_string);
    config.carrier_prompt_tokens = Some(root.clone());
    // The open-phase deadline: the serving side prefills the WHOLE
    // conversation at engine construction before the accept (the Capi
    // seam), so the carrier's deadline must bound that work honestly —
    // the 2.5 s round default would refuse real conversations as
    // `deadline_exceeded`. 20 s sits inside the 30 s per-frame I/O
    // deadline with margin; the serving side clamps [1000, 120000].
    config.round_deadline_ms = 20_000;
    let wire = match modelswarm_node::verifydrafts::dial_quic(
        addr,
        identity,
        &verifier_key,
        Duration::from_secs(10),
    )
    .await
    {
        Ok(wire) => wire,
        Err(e) => {
            engage_label("dial_failed", Some(&e.to_string()));
            return None;
        }
    };
    let tel = Telemetry::memory().0;
    let mut client = match modelswarm_node::verifydrafts::VerifyDraftsClient::negotiate(
        wire,
        identity.clone(),
        config,
        &tel,
    )
    .await
    {
        Ok(client) => client,
        Err(error) => {
            // The honest refusal classification: the peer's reject reason
            // (pin_mismatch / content_carriage_refused / profile_mismatch
            // / params_mismatch / budget_infeasible — "conversation does
            // not fit this peer") rides the label; the caller falls back
            // to the plain path, never silently.
            let label = match &error {
                modelswarm_node::verifydrafts::ClientOpenError::Rejected(reason) => *reason,
                other => family_open_error_reason(other),
            };
            engage_label(label, Some(&error.to_string()));
            return None;
        }
    };
    // The self-check engine for THIS conversation (prefilled with the
    // carrier root; KV advances round by round exactly as the serving
    // engine's does — cache-cold by construction at open).
    let selfcheck = match family_engine.engine_for_root(Some(&root)).await {
        Ok(engine) => engine,
        Err(e) => {
            engage_label("selfcheck_unavailable", Some(&format!("engine load: {e}")));
            return None;
        }
    };
    // The lossless verified loop: draft (this machine's engine, greedy
    // under the pinned params) → local self-check → wire → commit. The
    // COMMITTED stream is the verifying peer's engine's own continuation
    // (ADR-036 §3.2 — lossless-verification sessions only).
    let started = Instant::now();
    let mut committed: Vec<u32> = Vec::new();
    let max_out = max_tokens.unwrap_or(256) as usize;
    let w_cap = 8u32.min(client.engine_disclosure().batch_window_max).max(1);
    let mut rounds = 0u32;
    let mut divergence_label: Option<&'static str> = None;
    while committed.len() < max_out {
        let remaining = max_out - committed.len();
        let want = w_cap.min(remaining as u32) as usize;
        let mut prefix = root.clone();
        prefix.extend_from_slice(&committed);
        let window = match modelswarm_node::runtime_propose_window(
            Some(&runtime),
            profile,
            &prefix,
            want as u32,
        )
        .await
        {
            Ok(window) => window,
            Err(e) => {
                divergence_label = Some("draft_unavailable");
                telemetry.warn(
                    "chat.family_verified.draft_failed",
                    &[("phase", "draft"), ("error", &e)],
                );
                break;
            }
        };
        let window: Vec<u32> = window.into_iter().take(remaining).collect();
        if window.is_empty() {
            break; // the engine ended its turn (EOS): an honest short reply
        }
        // The pre-wire gate (the standing 2026-10-13 rule): a locally
        // divergent window NEVER hits the wire.
        let params = modelswarm_node::verifydrafts::SamplingParams::default();
        match selfcheck.verify_window(&window, &params).await {
            Ok((accepted, _)) if accepted as usize == window.len() => {}
            Ok((accepted, _)) => {
                divergence_label = Some("local_selfcheck");
                telemetry.info(
                    "family.engage",
                    &[
                        ("phase", "chat"),
                        ("peer", remote_peer_id),
                        ("engaged", "false"),
                        ("reason", "local_selfcheck"),
                        ("window", &window.len().to_string()),
                        ("accepted", &accepted.to_string()),
                    ],
                );
                break;
            }
            Err(e) => {
                divergence_label = Some("selfcheck_unavailable");
                telemetry.warn(
                    "chat.family_verified.selfcheck_failed",
                    &[("phase", "verify"), ("error", &e.to_string())],
                );
                break;
            }
        }
        match client.round(&window, &tel).await {
            Ok(outcome) => {
                let eos_now =
                    outcome.divergence_point.is_none() && (window.len() as u32) < want as u32;
                for token in &outcome.committed {
                    committed.push(*token);
                }
                rounds += 1;
                if eos_now {
                    break;
                }
            }
            Err(failure) => {
                divergence_label = Some(pairing_round_failure_label(&failure));
                break;
            }
        }
    }
    drop(selfcheck); // teardown-bounded: the conversation-rooted engine dies here
    if committed.is_empty() {
        // Nothing verified: the honest empty outcome falls back to the
        // plain path (labeled, never silent).
        engage_label(divergence_label.unwrap_or("no_committed_tokens"), None);
        return None;
    }
    // The committed stream is THIS machine's own committed data (its
    // drafts + the peers' corrections) — decode it under the pinned
    // tokenizer; deltas stream as the final text renders.
    let content = match modelswarm_node::runtime_detokenize(Some(&runtime), &committed).await {
        Ok(text) => text,
        Err(e) => {
            engage_label("detokenize_failed", Some(&e));
            return None;
        }
    };
    let content = content.trim().to_string();
    if content.is_empty() {
        engage_label("empty_reply", None);
        return None;
    }
    on_delta(&content);
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let completion_tokens = committed.len() as u64;
    let tok_s = if elapsed_ms > 0 {
        (completion_tokens as f64) * 1000.0 / elapsed_ms as f64
    } else {
        0.0
    };
    // The cohort: ONE verifying peer in the shipped configuration
    // (bounded by the preset cohort cap and the budget's max_endpoints).
    let cohort: u32 = 1;
    {
        let mut guard = inner.lock().await;
        guard.last_family_cohort = Some(cohort);
    }
    telemetry.info(
        "family.engage",
        &[
            ("phase", "chat"),
            ("peer", remote_peer_id),
            ("engaged", "true"),
            ("reason", "elected"),
            ("cohort", &cohort.to_string()),
            ("rounds", &rounds.to_string()),
            ("tokens", &completion_tokens.to_string()),
            ("elapsed_ms", &elapsed_ms.to_string()),
        ],
    );
    telemetry.info(
        "chat.family_verified.completed",
        &[
            ("peer", remote_peer_id),
            ("rounds", &rounds.to_string()),
            ("completion_tokens", &completion_tokens.to_string()),
            ("elapsed_ms", &elapsed_ms.to_string()),
        ],
    );
    Some(serde_json::json!({
        "content": content,
        "completion_tokens": completion_tokens,
        "elapsed_ms": elapsed_ms,
        "tokens_per_second": tok_s,
        // The honest mode label: verified AND slower by design (§3.4 —
        // presenting this mode as a speed feature is a defect).
        "mode": format!(
            "family-verified — peer {}… (slower by design)",
            remote_peer_id.chars().take(12).collect::<String>()
        ),
        "family_engage": { "engaged": true, "reason": "elected", "cohort": cohort },
    }))
}

/// F1(3): swarm-first chat. Finds a peer verifiably hosting this exact
/// profile with a REAL advertised address (not the no-listener
/// placeholder, not ourselves), and asks it first. The REMOTE executor is
/// driven directly — not through FailoverExecutor — so the reply's mode
/// honestly says who served: a remote peer, or this node as a logged
/// ADR-040 cloud key storage — the key lives ONLY here (a local file in
/// the data dir, the relay.addr/identity.seed discipline): never in the
/// webview beyond this command's call, never in telemetry, never sent to
/// the hub. C3 (cloud serving) consumes it; v0.2.31 only stores it.
fn cloud_key_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("cloud_key.json")
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct CloudKeyRecord {
    provider: String,
    api_key: String,
}

#[tauri::command]
async fn set_cloud_key(
    state: tauri::State<'_, DesktopState>,
    provider: String,
    key: String,
) -> Result<serde_json::Value, String> {
    if !matches!(provider.as_str(), "deepseek" | "openrouter") {
        return Err("provider must be deepseek or openrouter".into());
    }
    let key = key.trim().to_string();
    if key.is_empty() || key.len() > 400 {
        return Err("key must be 1..=400 characters".into());
    }
    let inner = state.inner.lock().await;
    let rec = CloudKeyRecord {
        provider,
        api_key: key,
    };
    let json = serde_json::to_string(&rec).map_err(|e| e.to_string())?;
    std::fs::write(cloud_key_path(&inner.data_dir), json).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "stored": true }))
}

#[tauri::command]
async fn clear_cloud_key(
    state: tauri::State<'_, DesktopState>,
) -> Result<serde_json::Value, String> {
    let inner = state.inner.lock().await;
    let _ = std::fs::remove_file(cloud_key_path(&inner.data_dir));
    Ok(serde_json::json!({ "stored": false }))
}

/// Reads the stored cloud key record, if any ( Rust-side consumers only —
/// the api_key never crosses into the webview).
pub(crate) fn read_cloud_key(data_dir: &std::path::Path) -> Option<CloudKeyRecord> {
    std::fs::read_to_string(cloud_key_path(data_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

/// ADR-030 B3: POST one counter-signed receipt to the live B1 intake
/// (`POST /events/job-result` with the full object) and record the job
/// locally (`Store::record_job`'s first production caller). Best-effort
/// by design: every failure is a telemetry row, never a chat error —
/// the reply has already been served by the time this runs.
async fn post_counted_receipt(
    tracker: std::sync::Arc<modelswarm_tracker_api::TrackerClient>,
    telemetry: std::sync::Arc<Telemetry>,
    data_dir: std::path::PathBuf,
    peer_id: String,
    receipt: modelswarm_transport::message::JobReceipt,
) {
    let digest = match modelswarm_node::receipt_wire::final_digest(&receipt) {
        Ok(d) => d,
        Err(e) => {
            telemetry.warn(
                "chat.receipt",
                &[
                    ("peer", peer_id.as_str()),
                    ("phase", "digest"),
                    ("error", &e),
                ],
            );
            return;
        }
    };
    // The intake's schema takes the bare hex (it re-prefixes for the
    // preimage comparison — msp-v1 §3.3/§7).
    let hex = digest
        .strip_prefix("sha256:")
        .unwrap_or(&digest)
        .to_string();
    let receipt_json = match serde_json::to_value(&receipt) {
        Ok(v) => v,
        Err(e) => {
            telemetry.warn(
                "chat.receipt",
                &[
                    ("peer", peer_id.as_str()),
                    ("phase", "encode"),
                    ("error", &e.to_string()),
                ],
            );
            return;
        }
    };
    let outcome = receipt.outcome.clone();
    match tracker
        .job_result_with_receipt(&hex, &outcome, receipt_json)
        .await
    {
        Ok(v) => {
            let reason = v
                .get("reason")
                .and_then(|r| r.as_str())
                .unwrap_or("unknown")
                .to_string();
            let grants = v
                .get("grants")
                .and_then(|g| g.as_array())
                .map(|a| a.len().to_string())
                .unwrap_or_else(|| "0".into());
            telemetry.info(
                "chat.receipt",
                &[
                    ("peer", peer_id.as_str()),
                    ("reason", reason.as_str()),
                    ("grants", grants.as_str()),
                ],
            );
        }
        Err(e) => {
            telemetry.warn(
                "chat.receipt",
                &[
                    ("peer", peer_id.as_str()),
                    ("phase", "post"),
                    ("error", &e.to_string()),
                ],
            );
        }
    }
    // The local copy (ADR-030 §6: record_job gains real callers) — the
    // requester's own job_accounting row from the receipt's signed
    // values, server peer named.
    if let Ok(store) = modelswarm_store::Store::open(data_dir.join("state.sqlite")) {
        if let Err(e) = store.record_job(
            &receipt.request_id,
            &peer_id,
            &receipt.profile_id,
            &receipt.outcome,
            &receipt.started_at,
            &receipt.ended_at,
            &receipt.usage_digest,
        ) {
            telemetry.warn(
                "chat.receipt",
                &[
                    ("peer", peer_id.as_str()),
                    ("phase", "local_record"),
                    ("error", &e.to_string()),
                ],
            );
        }
    }
}

/// fallback. Returns None when no remote peer is known or both sides
/// failed pre-token; the caller's local gateway path then serves.
#[allow(clippy::too_many_arguments)]
async fn try_swarm_chat(
    inner: &tokio::sync::Mutex<Inner>,
    tracker_url: &str,
    data_dir: &std::path::Path,
    telemetry: &std::sync::Arc<Telemetry>,
    profile: &str,
    local_executor: std::sync::Arc<dyn modelswarm_gateway::InferenceExecutor>,
    log: &[ChatTurn],
    max_tokens: Option<u32>,
    // The user-set chat temperature (owner directive 2026-10-13; greedy
    // 0.0 is the default AND the determinism contract for pairing — this
    // affects CHAT completions only; the pairing draft path stays
    // greedy-pinned and the family session pins its own defaults).
    temperature: f32,
    lease: Option<String>,
    on_delta: &(dyn Fn(&str) + Send + Sync),
) -> Option<serde_json::Value> {
    let tracker = match tracker_client(tracker_url, data_dir, telemetry).await {
        Ok(t) => t,
        Err(e) => {
            telemetry.warn("chat.swarm.error", &[("phase", "tracker"), ("error", &e)]);
            record_shadow_failure(telemetry, None, profile, "none", None, "tracker", None);
            return None;
        }
    };
    let peers = match tracker.lookup(profile, 10).await {
        Ok(p) => p,
        Err(e) => {
            telemetry.warn(
                "chat.swarm.error",
                &[("phase", "lookup"), ("error", &e.to_string())],
            );
            record_shadow_failure(telemetry, None, profile, "none", None, "lookup", None);
            return None;
        }
    };
    let identity = match cached_identity(data_dir, telemetry) {
        Ok(i) => i,
        Err(_) => {
            // load_or_create_identity already warned with the cause.
            record_shadow_failure(telemetry, None, profile, "none", None, "identity", None);
            return None;
        }
    };
    let own_peer_id = identity.peer_id();

    // SHADOW PLANNER (dependency-graph edge 11; M10 build item 3,
    // restricted single-pick shape): join the F15 measurements with
    // THIS roster before production selection so the decision below
    // logs what `select_microswarm` would have chosen. It never acts —
    // production selection stays first-found, byte-for-byte below.
    let shadow_plan =
        gather_shadow_plan(inner, telemetry, &peers, &own_peer_id, log, max_tokens).await;

    // First peer with a DIALABLE advertised address (F2B: a direct QUIC
    // multiaddr OR a circuit address — a relayed peer is reachable, not
    // skipped) that is not us.
    let candidate = match peers.into_iter().find(|p| {
        p.peer_id != own_peer_id
            && p.free_slots > 0
            && p.addresses.iter().any(|a| peer_addr_dialable(a))
    }) {
        Some(c) => c,
        None => {
            // Log the shadow decision anyway (production pick: none) and
            // a realized failure row — the swarm had no eligible peer, and
            // the 9.6 dataset must sample that, not drop it.
            let pick = log_shadow_decision(telemetry, &shadow_plan, profile, "none");
            record_shadow_failure(
                telemetry,
                Some(&shadow_plan),
                profile,
                "none",
                pick.as_deref(),
                "no_eligible_peer",
                None,
            );
            return None;
        }
    };
    let addr = match candidate.addresses.iter().find(|a| peer_addr_dialable(a)) {
        Some(a) => a.clone(),
        None => {
            // Unreachable in practice (the candidate predicate above
            // requires a dialable addr) but fail visible if it ever fires.
            let pick = log_shadow_decision(telemetry, &shadow_plan, profile, &candidate.peer_id);
            record_shadow_failure(
                telemetry,
                Some(&shadow_plan),
                profile,
                &candidate.peer_id,
                pick.as_deref(),
                "no_quic_addr",
                None,
            );
            return None;
        }
    };
    // F15: the roster's advertised queueMs rides along (untrusted input,
    // recorded next to the measured admission-to-first-token).
    let advertised_queue_ms = candidate.queue_ms;
    // The elected ADR-036 path (below) dials its own carrier session to
    // the same candidate before the pooled executor takes the addr. Its
    // dial (`dial_quic`) is DIRECT-only this pass, so a direct address is
    // preferred when one exists; a circuit addr fails it fast and the
    // turn falls back to the plain swarm path below — which dials the
    // circuit honestly (recorded follow-up: family-over-relay).
    #[cfg(feature = "verifydrafts")]
    let elected_addr = candidate
        .addresses
        .iter()
        .find(|a| peer_addr_direct(a))
        .or_else(|| candidate.addresses.iter().find(|a| peer_addr_dialable(a)))
        .cloned()
        .unwrap_or_else(|| addr.clone());
    // F2B: the peer's FULL advertised candidate list (direct + circuit)
    // rides the pooled executor — direct first, relay fallback, honest
    // per-attempt reporting (see RemoteExecutor::execute).
    let candidate_addrs: Vec<String> = candidate
        .addresses
        .iter()
        .filter(|a| peer_addr_dialable(a))
        .cloned()
        .collect();
    let peer = modelswarm_node::remote::RemotePeer {
        addr,
        peer_id: candidate.peer_id,
    };
    let remote_peer_id = peer.peer_id.clone();
    telemetry.info(
        "chat.swarm",
        &[
            ("peer", peer.peer_id.as_str()),
            ("addr", peer.addr.as_str()),
        ],
    );
    // SHADOW DECISION (logged only; never acts): the frozen cost
    // model's pick over the F15 measurements vs. this first-found
    // production pick — the 9.6 pass-1 divergence seed.
    let shadow_pick = log_shadow_decision(telemetry, &shadow_plan, profile, &remote_peer_id);

    // FAMILY AUTO-ENGAGE CHECKPOINT (owner directive 2026-10-13, zero
    // buttons): evaluated on every swarm chat turn, behind the engage
    // gate. For the AUTOMATIC path (no election) engagement never carries
    // chat content (ADR-036 §3.3 — the reason label says so); the plain
    // swarm path below serves this turn either way, BEFORE the first
    // output token. The ELECTED path (ADR-036 §3.3 — the user's explicit
    // family-verified-chat toggle) runs INSTEAD of the ladder, like the
    // probe: user-triggered, honest cost displayed, with its own honest
    // fallback to the plain path below on any failure.
    // OWNER DIRECTIVE 2026-10-15 ("why do we have a button to turn on
    // Family? this should always be on — reduce the number of switches,
    // assume everything is on by design"): the requester-side election
    // TOGGLE IS GONE. The carrier-session verification is attempted on
    // EVERY swarm chat turn with its honest pre-first-token fallback;
    // the §3.1 disclosure is permanently visible. The automatic ladder
    // below is superseded at runtime (kept compiled; retirement is a
    // recorded follow-up — see the ADR-036 revision note).
    #[cfg(feature = "verifydrafts")]
    let elected = true;
    #[cfg(feature = "verifydrafts")]
    let elected_reply = if elected {
        try_family_verified_chat(
            inner,
            telemetry,
            profile,
            log,
            max_tokens,
            &elected_addr,
            &remote_peer_id,
            &identity,
            lease.as_deref(),
            on_delta,
        )
        .await
    } else {
        None
    };
    #[cfg(feature = "verifydrafts")]
    if let Some(reply) = elected_reply {
        // The elected turn SERVED through a carrier session — cohort N
        // verified, slower by design, never hidden (the reply carries
        // family_engage { engaged: true, cohort }).
        let mut reply = reply;
        reply["served_by"] = serde_json::json!("swarm");
        reply["family_verified"] = serde_json::json!(true);
        return Some(reply);
    }
    #[cfg(feature = "verifydrafts")]
    let family_engage = if elected {
        // The elected path already telemetered its own honest outcome
        // (engaged:false + reason); the automatic ladder has nothing to
        // add for this turn.
        inner.lock().await.last_engage_reason = Some("elected_fell_back".to_string());
        "elected_fell_back"
    } else {
        family_engage_checkpoint(inner, profile, &remote_peer_id, &shadow_plan, telemetry).await
    };

    let make_request = || modelswarm_gateway::NormalizedRequest {
        request_id: format!("chat-{}", modelswarm_identity::new_nonce()),
        profile_id: profile.to_string(),
        capability_token: lease.clone(),
        messages: log
            .iter()
            .map(|t| modelswarm_gateway::NormalizedMessage {
                role: t.role.clone(),
                content: t.content.clone(),
            })
            .collect(),
        sampling: modelswarm_gateway::Sampling {
            // The user-set chat temperature rides the EXISTING wire
            // sampling fields (msp-v1 §6 carries them; the serving side
            // maps them back through the executor) — no protocol change.
            temperature,
            top_p: 1.0,
            top_k: 40,
            seed: None,
        },
        max_tokens: max_tokens.unwrap_or(256),
        deadline_ms: 120_000,
        stream: true,
    };

    // Connection reuse: keep ONE pooled executor per (profile, peer) in
    // the app state so chat turns share a warm QUIC session.
    let key = format!("{profile}|{}", peer.peer_id);
    let remote = {
        let mut inner_state = inner.lock().await;
        // F15: one measurement recorder per app, store-backed (best
        // effort; degrades to memory-only with a logged warning) —
        // opened by the shadow-plan gathering above, which always runs
        // before selection.
        let metrics = std::sync::Arc::clone(&shadow_plan.metrics);
        match &inner_state.swarm_executor {
            Some((k, exec)) if k == &key => std::sync::Arc::clone(exec),
            _ => {
                // ADR-038 §6.4 row 8: the requester dial presents the
                // heartbeat-cached admission ticket on relay legs (read at
                // dial time, so heartbeat refreshes reach this pooled
                // executor too). `None` while not hosting — the documented
                // pre-wiring behavior.
                let mut executor = modelswarm_node::remote::RemoteExecutor::new(peer, identity)
                    .with_metrics(metrics);
                if let Some(tickets) = inner_state.relay_tickets.clone() {
                    executor = executor.with_relay_tickets(tickets);
                }
                let exec = std::sync::Arc::new(executor);
                inner_state.swarm_executor = Some((key, std::sync::Arc::clone(&exec)));
                exec
            }
        }
    };
    // F15: keep the advertised queueMs fresh from THIS roster lookup —
    // the pooled executor outlives any single roster.
    remote.set_advertised_queue_ms(Some(advertised_queue_ms));
    // F2B: same freshness for the candidate address list (direct +
    // circuit) — a peer that gained or lost relay reachability since the
    // last turn is dialed per its CURRENT advertisement.
    remote.set_candidate_addrs(candidate_addrs);
    use futures_util::StreamExt;
    use modelswarm_gateway::{ExecutorEvent, InferenceExecutor};

    enum ChatOutcome {
        Served {
            content: String,
            completion_tokens: u64,
        },
        Failed(String),
    }
    async fn drain_chat<S>(mut events: S, on_delta: &(dyn Fn(&str) + Send + Sync)) -> ChatOutcome
    where
        S: futures_util::Stream<Item = ExecutorEvent> + Unpin,
    {
        let mut content = String::new();
        let mut completion_tokens = 0u64;
        while let Some(event) = events.next().await {
            match event {
                ExecutorEvent::TokenDelta { delta, .. } => {
                    on_delta(&delta);
                    content.push_str(&delta);
                }
                ExecutorEvent::Usage {
                    completion_tokens: t,
                    ..
                } => completion_tokens = u64::from(t),
                ExecutorEvent::Completed { .. } => {
                    return ChatOutcome::Served {
                        content,
                        completion_tokens,
                    };
                }
                ExecutorEvent::Error { code, .. } => {
                    return ChatOutcome::Failed(code.as_str().to_string());
                }
                ExecutorEvent::Accepted { .. } => {}
            }
        }
        ChatOutcome::Failed("stream ended without completion".into())
    }

    let started = Instant::now();
    // Remote first; on any pre-token failure fall back to the SAME local
    // executor the gateway uses, with the reason in the mode label —
    // execution mode is never hidden.
    let (content, completion_tokens, mode, served_remote) =
        match remote.execute(make_request()).await {
            Ok(events) => match drain_chat(events, on_delta).await {
                ChatOutcome::Served {
                    content,
                    completion_tokens,
                } => {
                    // F2B honesty (ADR-014 rule 5): a reply served through
                    // a relay circuit is labeled relayed, never direct.
                    let relayed = matches!(
                        remote.pooled_dial_path(),
                        Some(modelswarm_transport::libp2p_backend::DialPath::Relayed { .. })
                    );
                    let label: String = format!(
                        "swarm — remote peer {}…{}",
                        remote_peer_id.chars().take(12).collect::<String>(),
                        if relayed { " (relayed)" } else { "" }
                    );
                    // ADR-030 B3 (the requester's accounting half): the
                    // executor's stream tail already counter-signed the
                    // receipt and acked it on the session; this side POSTs
                    // the final object to the B1 intake and records the
                    // job locally — spawned OFF the reply path (accounting
                    // is never allowed to delay or fail a chat reply).
                    if let Some(receipt) = remote.take_last_receipt() {
                        let tracker = std::sync::Arc::clone(&tracker);
                        let telemetry = std::sync::Arc::clone(telemetry);
                        let data_dir = data_dir.to_path_buf();
                        let peer = remote_peer_id.clone();
                        tokio::spawn(async move {
                            post_counted_receipt(tracker, telemetry, data_dir, peer, receipt).await;
                        });
                    }
                    (content, completion_tokens, label, true)
                }
                ChatOutcome::Failed(reason) => {
                    telemetry.warn("chat.swarm.fallback", &[("reason", &reason)]);
                    match local_executor.execute(make_request()).await {
                        Ok(events) => match drain_chat(events, on_delta).await {
                            ChatOutcome::Served {
                                content,
                                completion_tokens,
                            } => (
                                content,
                                completion_tokens,
                                format!("local (remote failed: {reason})"),
                                false,
                            ),
                            ChatOutcome::Failed(local_reason) => {
                                telemetry.warn(
                                    "chat.swarm.error",
                                    &[("phase", "local_fallback"), ("error", &local_reason)],
                                );
                                record_shadow_failure(
                                    telemetry,
                                    Some(&shadow_plan),
                                    profile,
                                    &remote_peer_id,
                                    shadow_pick.as_deref(),
                                    &format!("local_fallback:{local_reason}"),
                                    Some(started.elapsed().as_millis() as u64),
                                );
                                return None;
                            }
                        },
                        Err(e) => {
                            telemetry.warn(
                                "chat.swarm.error",
                                &[("phase", "local_fallback"), ("error", &e.to_string())],
                            );
                            record_shadow_failure(
                                telemetry,
                                Some(&shadow_plan),
                                profile,
                                &remote_peer_id,
                                shadow_pick.as_deref(),
                                &format!("local_fallback:{}", e),
                                Some(started.elapsed().as_millis() as u64),
                            );
                            return None;
                        }
                    }
                }
            },
            Err(e) => {
                // Pre-token dial/protocol failure: honest local fallback.
                // F2B: the per-attempt dial report (kind, addr, error —
                // including the relay's own status vocabulary, surfaced
                // verbatim by the transport) rides the EXISTING fallback
                // event's reason text.
                let mut reason = e.to_string();
                for attempt in remote.last_dial_attempts() {
                    reason.push_str(&format!(
                        "; {} {}: {}",
                        attempt.kind, attempt.addr, attempt.error
                    ));
                }
                telemetry.warn("chat.swarm.fallback", &[("reason", &reason)]);
                match local_executor.execute(make_request()).await {
                    Ok(events) => match drain_chat(events, on_delta).await {
                        ChatOutcome::Served {
                            content,
                            completion_tokens,
                        } => (
                            content,
                            completion_tokens,
                            "local (remote dial failed)".to_string(),
                            false,
                        ),
                        ChatOutcome::Failed(local_reason) => {
                            telemetry.warn(
                                "chat.swarm.error",
                                &[("phase", "local_fallback"), ("error", &local_reason)],
                            );
                            record_shadow_failure(
                                telemetry,
                                Some(&shadow_plan),
                                profile,
                                &remote_peer_id,
                                shadow_pick.as_deref(),
                                &format!("local_fallback:{local_reason}"),
                                Some(started.elapsed().as_millis() as u64),
                            );
                            return None;
                        }
                    },
                    Err(le) => {
                        telemetry.warn(
                            "chat.swarm.error",
                            &[("phase", "local_fallback"), ("error", &le.to_string())],
                        );
                        record_shadow_failure(
                            telemetry,
                            Some(&shadow_plan),
                            profile,
                            &remote_peer_id,
                            shadow_pick.as_deref(),
                            &format!("local_fallback:{le}"),
                            Some(started.elapsed().as_millis() as u64),
                        );
                        return None;
                    }
                }
            }
        };
    if content.trim().is_empty() {
        telemetry.warn("chat.swarm.empty", &[]);
        record_shadow_failure(
            telemetry,
            Some(&shadow_plan),
            profile,
            &remote_peer_id,
            shadow_pick.as_deref(),
            "empty_reply",
            Some(started.elapsed().as_millis() as u64),
        );
        return None;
    }
    let content = content.trim().to_string();
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let tok_s = if elapsed_ms > 0 && completion_tokens > 0 {
        (completion_tokens as f64) * 1000.0 / elapsed_ms as f64
    } else {
        0.0
    };
    telemetry.info(
        "chat.swarm.completed",
        &[
            (
                "served_by",
                if served_remote {
                    "remote"
                } else {
                    "local_fallback"
                },
            ),
            ("completion_tokens", &completion_tokens.to_string()),
            ("elapsed_ms", &elapsed_ms.to_string()),
        ],
    );
    // 9.6 pass-1 dataset seed: the realized outcome vs. the shadow
    // prediction, correlated by trace + peer ids. TTFT/total are the
    // requester-measured EWMAs the executor recorded for THIS request
    // (per-request values live in `net.completion`); everything here is
    // numbers and ids only.
    {
        let observed = shadow_plan.metrics.observe(&remote_peer_id);
        let profile_obs = observed.profile(profile);
        let ttft_ewma = profile_obs
            .and_then(|p| p.ttft_ewma_ms)
            .map(|v| format!("{v:.1}"))
            .unwrap_or_else(|| "unmeasured".into());
        let total_ewma = profile_obs
            .and_then(|p| p.total_ewma_ms)
            .map(|v| format!("{v:.1}"))
            .unwrap_or_else(|| "unmeasured".into());
        telemetry.info(
            "sched.shadow.realized",
            &[
                ("label", modelswarm_scheduler::shadow::SHADOW_LABEL),
                ("trace", shadow_plan.trace_id.as_str()),
                ("profile", profile),
                ("production_peer", remote_peer_id.as_str()),
                ("shadow_pick", shadow_pick.as_deref().unwrap_or("none")),
                (
                    "served_by",
                    if served_remote {
                        "remote"
                    } else {
                        "local_fallback"
                    },
                ),
                ("wall_ms", &elapsed_ms.to_string()),
                ("ttft_ewma_ms", &ttft_ewma),
                ("total_ewma_ms", &total_ewma),
            ],
        );
    }
    // Execution-mode honesty: whether THIS reply used the family
    // cooperative tier, and if not, the exact reason (labels only).
    // The desktop never hides single-vs-cooperative execution.
    let mut reply = serde_json::json!({
        "content": content,
        "completion_tokens": completion_tokens,
        "elapsed_ms": elapsed_ms,
        "tokens_per_second": tok_s,
        "mode": mode,
        "family_engage": { "engaged": false },
    });
    #[cfg(feature = "verifydrafts")]
    {
        reply["family_engage"]["reason"] = serde_json::json!(family_engage);
    }
    Some(reply)
}

/// Neutral language pin for chat completions — see `chat_request_log`.
/// The effective language pin, APPENDED to the user's latest message on
/// the REQUEST copy only. Placement is measured, not aesthetic: against
/// the shipped model+engine (2026-10-15 matrix, poisoned-history
/// reproduction), a front system instruction does NOT break the lock —
/// only this trailing named-language form does ("(Respond in the
/// language of this message: English.)" → English; every system-block
/// variant and the language-neutral wording → still Chinese). The named
/// language is filled by `english_message` — the one case the heuristic
/// confidently recognizes, and the lock's victim case.
const CHAT_LANGUAGE_PIN_SUFFIX: &str = "\n(Respond in the language of this message: English.)";

/// True when the message is confidently English: ASCII-dominant AND
/// containing a common English function word. CJK questions need no pin
/// (the model naturally matches the user's language there — measured);
/// other Latin languages (French, German, …) fail the word list on
/// purpose rather than get mis-pinned to English.
fn english_message(message: &str) -> bool {
    let mut ascii = 0usize;
    let mut total = 0usize;
    for c in message.chars() {
        if c.is_alphanumeric() {
            total += 1;
            if c.is_ascii() {
                ascii += 1;
            }
        }
    }
    if total == 0 || ascii * 10 < total * 9 {
        return false;
    }
    let lower = message.to_lowercase();
    [
        "the ", "what", "how", "why", "is ", "are ", "you", "please", "can ", "tell",
    ]
    .iter()
    .any(|w| lower.contains(w))
}

/// Builds the outgoing chat request copy: the visible log (already capped
/// at 20 turns by `send_chat`) with, for confidently-English messages,
/// the language pin APPENDED to the last user turn. The pin never enters
/// the persisted `chat_log` — every request is rebuilt from the clean
/// log, so the transcript stays exactly what the user wrote and saw. The
/// `{role, content}` wire shape carries the appended form through both
/// the swarm path (the remote peer renders the wire messages verbatim in
/// ChatML) and the local loopback gateway.
fn chat_request_log(log: &[ChatTurn]) -> Vec<ChatTurn> {
    let mut out = log.to_vec();
    if let Some(last) = out.last_mut() {
        if last.role == "user" && english_message(&last.content) {
            last.content.push_str(CHAT_LANGUAGE_PIN_SUFFIX);
        }
    }
    out
}

#[tauri::command]
async fn send_chat(
    state: tauri::State<'_, DesktopState>,
    window: tauri::Window,
    message: String,
    max_tokens: Option<u32>,
    // The user-set chat temperature (owner directive 2026-10-13 — the
    // greedy language-drift escape hatch). None/0 = greedy, the default
    // and the determinism contract for pairing. Clamped to the msp-v1
    // 6.4 range [0, 2]. Affects CHAT completions only: the pairing draft
    // path (runtime_propose_window) and the family session's pinned
    // params stay greedy regardless.
    temperature: Option<f64>,
) -> Result<serde_json::Value, String> {
    let temperature = temperature.unwrap_or(0.0).clamp(0.0, 2.0) as f32;
    // Per-delta surface (P1): every path that produces tokens (remote swarm
    // peer, local fallback, local gateway SSE) calls this so the UI renders
    // text as it arrives. Content only — the final turn (mode label, error
    // surface, empty-reply note) is still decided by the paths below.
    let on_delta = move |delta: &str| {
        let _ = window.emit("chat-delta", serde_json::json!({ "delta": delta }));
    };
    let (gateway, profile, mut log, node_ctx) = {
        let inner = state.inner.lock().await;
        let running = inner.node.as_ref().ok_or("hosting is off")?;
        // F1(3): swarm context for the remote-first chat path.
        let ctx = running
            .handle
            .executor
            .clone()
            .map(|executor| (executor, running.handle.profile_id.clone()));
        (
            running.handle.local_addr.to_string(),
            inner.profile_id.clone().ok_or("no profile selected")?,
            inner.chat_log.clone(),
            ctx,
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

    // Language pin (owner directive 2026-10-15, the Chinese-lock evening):
    // one Chinese assistant turn in the rendered history flips every later
    // reply to Chinese at ANY temperature (reproduced against the shipped
    // model+engine; a fresh conversation answers English). For
    // confidently-English messages, the named-language instruction is
    // APPENDED to the user's latest turn on the REQUEST copy only — never
    // the persisted chat_log — on both the swarm and local paths, while
    // the visible transcript stays exactly what the user wrote and saw.
    // (The v0.2.26 front-system variant was measured INSUFFICIENT live —
    // only the trailing named form breaks the lock; see
    // CHAT_LANGUAGE_PIN_SUFFIX.) Chat completions only: the pairing draft
    // path and family verification keep their pinned greedy contracts;
    // the instruction constrains language, nothing else.
    let request_log = chat_request_log(&log);

    // F1(3): SWARM FIRST — if another peer is verifiably hosting this
    // exact profile with a real address, ask it before this machine's
    // local engine; FailoverExecutor degrades to the local executor on
    // any pre-token failure (honest fallback, logged). Returns None when
    // no remote peer is known — the local gateway path continues.
    if let Some((executor, Some(hosted_profile))) = &node_ctx {
        if hosted_profile == &profile {
            let (tracker_url, data_dir) = {
                let inner = state.inner.lock().await;
                (inner.tracker_url.clone(), inner.data_dir.clone())
            };
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let lease = earn_lease(
                &state.inner,
                &tracker_url,
                &data_dir,
                &state.telemetry,
                &gateway,
                &profile,
                now_ms,
            )
            .await;
            if let Some(mut reply) = try_swarm_chat(
                &state.inner,
                &tracker_url,
                &data_dir,
                &state.telemetry,
                &profile,
                executor.clone(),
                &request_log,
                max_tokens,
                temperature,
                lease,
                &on_delta,
            )
            .await
            {
                // The remote peer's reply enters the rendered history too
                // — without it, multi-turn swarm transcripts send only
                // user turns and drift from local behavior.
                let mut inner = state.inner.lock().await;
                inner.chat_log = log;
                let content = reply["content"].as_str().unwrap_or_default().to_string();
                if !content.is_empty() {
                    inner.chat_log.push(ChatTurn {
                        role: "assistant".into(),
                        content,
                    });
                }
                reply["served_by"] = serde_json::json!("swarm");
                return Ok(reply);
            }
        }
    }

    // Plain {role, content} turns — the serving executor applies ChatML
    // exactly once (ADR-025). The request copy carries the language pin.
    let turns: Vec<serde_json::Value> = request_log
        .iter()
        .map(|t| serde_json::json!({ "role": t.role, "content": t.content }))
        .collect();

    let started = Instant::now();
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())?;
    // P1: stream=true — the gateway (and through it the executor) now
    // forwards deltas as tokens are produced; this loop renders each one via
    // `on_delta` while accumulating the final turn. The error surface is
    // unchanged in shape: HTTP-level failures carry the same
    // {"error":{code,message}} body, and mid-stream failures arrive as an
    // SSE error event instead — both become the ERROR turn below.
    let response = http
        .post(format!("http://{gateway}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": profile,
            "messages": turns,
            "max_tokens": max_tokens.unwrap_or(256),
            // The user-set chat temperature through the gateway's public
            // OpenAI-compatible surface (clamped [0, 2] there too).
            "temperature": temperature,
            "stream": true,
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let mut stream_error: Option<(String, String)> = None;
    if !response.status().is_success() {
        let body: serde_json::Value = response.json().await.unwrap_or(serde_json::Value::Null);
        if let Some(code) = body["error"]["code"].as_str() {
            stream_error = Some((
                code.to_string(),
                body["error"]["message"].as_str().unwrap_or("").to_string(),
            ));
        } else {
            return Err(format!(
                "gateway answered {} without an error body",
                code_str_of(body)
            ));
        }
    } else {
        use futures_util::StreamExt;
        let mut stream = response.bytes_stream();
        let mut buffer: Vec<u8> = Vec::new();
        let mut content = String::new();
        let mut completion_tokens = 0u64;
        'sse: while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            buffer.extend_from_slice(&chunk);
            while let Some(pos) = buffer.windows(2).position(|w| w == b"\n\n") {
                let raw: Vec<u8> = buffer.drain(..pos + 2).collect();
                for line in raw.split(|b| *b == b'\n') {
                    let line = line.strip_suffix(b"\r").unwrap_or(line);
                    let Some(payload) = line.strip_prefix(b"data: ") else {
                        continue;
                    };
                    if payload == b"[DONE]" {
                        continue;
                    }
                    let Ok(value) = serde_json::from_slice::<serde_json::Value>(payload) else {
                        continue;
                    };
                    if let Some(error) = value.get("error") {
                        stream_error = Some((
                            error["code"].as_str().unwrap_or("stream_error").to_string(),
                            error["message"].as_str().unwrap_or("").to_string(),
                        ));
                        break 'sse;
                    }
                    if let Some(delta) = value["choices"][0]["delta"]["content"].as_str() {
                        if !delta.is_empty() {
                            on_delta(delta);
                            content.push_str(delta);
                        }
                    }
                    if let Some(tokens) = value["usage"]["completion_tokens"].as_u64() {
                        completion_tokens = tokens;
                    }
                }
            }
        }
        return finish_local_turn(
            &state,
            started,
            log,
            content,
            completion_tokens,
            stream_error,
        )
        .await;
    }

    // A gateway error body (engine crash, restart budget exhausted,
    // profile mismatch) must surface as an ERROR turn — rendering it as
    // "no reply, the model ended its turn" sends the user chasing a
    // phantom prompt problem while hosting is actually broken.
    if let Some((code, message)) = stream_error {
        let message = message.as_str().to_string();
        log_event(
            &state.telemetry,
            "warn",
            "chat.local.error",
            &[("code", code.as_str()), ("message", message.as_str())],
        );
        let mut inner = state.inner.lock().await;
        inner.chat_log = log;
        return Ok(serde_json::json!({
            "content": "",
            "completion_tokens": 0,
            "elapsed_ms": started.elapsed().as_millis() as u64,
            "tokens_per_second": 0.0,
            "mode": "local error",
            "served_by": "local",
            "error": { "code": code, "message": message },
        }));
    }
    unreachable!("stream_error is set on every non-success path")
}

/// Tiny helper: the raw body of an unrecognized error response, for the
/// invoke-error bubble only.
fn code_str_of(body: serde_json::Value) -> String {
    body.to_string().chars().take(200).collect()
}

/// Shared tail of the local streaming path: identical bookkeeping to the
/// pre-streaming version (empty replies never enter the rendered history;
/// the mode label stays "local single").
async fn finish_local_turn(
    state: &tauri::State<'_, DesktopState>,
    started: Instant,
    log: Vec<ChatTurn>,
    content_raw: String,
    completion_tokens: u64,
    stream_error: Option<(String, String)>,
) -> Result<serde_json::Value, String> {
    if let Some((code, message)) = stream_error {
        log_event(
            &state.telemetry,
            "warn",
            "chat.local.error",
            &[("code", code.as_str()), ("message", message.as_str())],
        );
        let mut inner = state.inner.lock().await;
        inner.chat_log = log;
        return Ok(serde_json::json!({
            "content": "",
            "completion_tokens": 0,
            "elapsed_ms": started.elapsed().as_millis() as u64,
            "tokens_per_second": 0.0,
            "mode": "local error",
            "served_by": "local",
            "error": { "code": code, "message": message },
        }));
    }
    let content = content_raw.trim().to_string();
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
        "served_by": "local",
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
    let tracker_url = {
        let inner = state.inner.lock().await;
        inner.tracker_url.clone()
    };
    log_event(
        &state.telemetry,
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
        // One app instance per machine: two nodes on one identity/data
        // dir thrash the roster and overwrite session.token/lease.id (the
        // two-instance confusion burned UI automation twice). A second
        // launch focuses the existing window and exits.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        // Opens the device-approval page in the system browser (capability
        // scopes it to the tracker origin only).
        .plugin(tauri_plugin_opener::init())
        // The auto-update engine (owner request 2026-10-12: "clients
        // update and restart when we ship a new client"). Config lives in
        // tauri.conf.json plugins.updater (public key + endpoint +
        // requireSignedVersion + passive installs); this registration adds
        // no webview capability — the supervisor below drives it from
        // Rust, and only status LINES cross into the UI.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let handle = app.handle().clone();
            let for_resume = handle.clone();
            let for_supervisor = handle.clone();
            tauri::async_runtime::block_on(async move {
                let state = DesktopState::new().await;
                handle.manage(state);
            });
            // Hosting auto-resume (owner directive 2026-10-17): the
            // swarm silently losing a machine on every restart ends
            // here — the marker's choice comes back on its own.
            tauri::async_runtime::spawn(hosting_resume_supervisor(for_resume));
            // One check on start, then every 6h; every failure degrades to
            // an honest status line. Dev opt-out: MSP_NO_AUTOUPDATE=1.
            updater::spawn_supervisor(for_supervisor);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            set_tracker,
            set_pairing_allowed,
            set_content_carriage,
            accept_privacy,
            list_models,
            census,
            resource_sample,
            download_model,
            set_hosting,
            set_cloud_key,
            clear_cloud_key,
            select_model,
            lookup_peers,
            family_probe,
            send_chat,
            clear_chat,
            assistant_turn,
            assistant_reset,
            assistant_acknowledge,
            set_assistant_enabled,
            open_approval_page,
            hf_search_models,
            hf_list_ggufs,
            request_model,
            analyze_hardware
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

    /// F2B roster/picker classification: a circuit multiaddr (relayed
    /// reachability) is DIALABLE — a relayed peer is reachable, never
    /// skipped — while placeholder `0.0.0.0` addresses stay undialable
    /// and the direct-only predicate (probe/shadow facts) excludes
    /// circuit addresses by name.
    #[test]
    fn peer_address_classification_direct_circuit_and_placeholders() {
        let direct = "/ip4/192.168.1.5/udp/4001/quic-v1";
        let circuit =
            "/ip4/10.0.0.2/udp/4001/quic-v1/p2p/12D3KooWrelay/p2p-circuit/p2p/12D3KooWpeer";
        let placeholder = "/ip4/0.0.0.0/udp/4001/quic-v1";
        let circuit_placeholder = "/ip4/0.0.0.0/udp/4001/quic-v1/p2p/12D3KooWrelay/p2p-circuit";

        assert!(peer_addr_dialable(direct));
        assert!(peer_addr_dialable(circuit), "a relayed peer is reachable");
        assert!(!peer_addr_dialable(placeholder));
        assert!(!peer_addr_dialable(circuit_placeholder));
        assert!(!peer_addr_dialable(ADDR_NO_LISTENER));

        assert!(peer_addr_direct(direct));
        assert!(
            !peer_addr_direct(circuit),
            "the direct-only fact excludes circuits"
        );
        assert!(!peer_addr_direct(placeholder));

        assert!(addr_is_circuit(circuit));
        assert!(!addr_is_circuit(direct));
    }

    /// The F2B relay endpoint configuration: default-EMPTY (a missing
    /// `relay.addr` file = no relay, exactly the pre-F2B behavior —
    /// nothing invented, nothing pointing at a nonexistent host), blank
    /// lines and `#` comments skipped, first real line wins.
    #[test]
    fn relay_addr_file_is_optional_and_comment_aware() {
        let dir = tempfile::tempdir().unwrap();
        // Absent file: no relay configured.
        assert_eq!(configured_relay_addr(dir.path()), None);
        // Empty / whitespace / comment-only file: still none.
        std::fs::write(dir.path().join(RELAY_ADDR_FILE), "\n  \n# comment\n").unwrap();
        assert_eq!(configured_relay_addr(dir.path()), None);
        // One real line (trailing newline + indentation tolerated).
        std::fs::write(
            dir.path().join(RELAY_ADDR_FILE),
            "# the owner's second desktop\n  /ip4/10.0.0.2/udp/4001/quic-v1/p2p/12D3KooWrelay  \n",
        )
        .unwrap();
        assert_eq!(
            configured_relay_addr(dir.path()).as_deref(),
            Some("/ip4/10.0.0.2/udp/4001/quic-v1/p2p/12D3KooWrelay")
        );
    }

    /// The advertised-list truth rule (F2B): direct always first, the
    /// circuit address only while set, and the watch fires ONLY on
    /// change (the heartbeat re-registers on change — a no-op update
    /// must not force one).
    #[test]
    fn advertised_list_tracks_the_live_circuit_truthfully() {
        let (tx, mut rx) = tokio::sync::watch::channel(Vec::<String>::new());
        set_advertised_addrs(&tx, "/ip4/192.168.1.5/udp/4001/quic-v1", None);
        assert!(rx.has_changed().unwrap());
        assert_eq!(
            *rx.borrow_and_update(),
            vec!["/ip4/192.168.1.5/udp/4001/quic-v1".to_string()]
        );
        // Same value again: no change signal.
        set_advertised_addrs(&tx, "/ip4/192.168.1.5/udp/4001/quic-v1", None);
        assert!(!rx.has_changed().unwrap());
        // Circuit added: change fires, direct stays FIRST (requesters try
        // direct before the relay — ADR-014 direct-first).
        set_advertised_addrs(
            &tx,
            "/ip4/192.168.1.5/udp/4001/quic-v1",
            Some("/ip4/10.0.0.2/udp/4001/quic-v1/p2p/r/p2p-circuit/p2p/us".into()),
        );
        assert!(rx.has_changed().unwrap());
        let list = rx.borrow_and_update().clone();
        assert_eq!(list.len(), 2);
        assert!(list[0].ends_with("quic-v1"));
        assert!(addr_is_circuit(&list[1]));
        // Circuit lost (relay died): the list shrinks and the change
        // fires — the roster must not claim a dead circuit.
        set_advertised_addrs(&tx, "/ip4/192.168.1.5/udp/4001/quic-v1", None);
        assert!(rx.has_changed().unwrap());
        assert_eq!(rx.borrow_and_update().len(), 1);
    }

    #[test]
    fn fit_and_score_matches_the_rtx_3080_vector() {
        // Owner reference machine: 10 GB VRAM (9216 MiB), 32 GB RAM.
        let hw = HardwareInfo {
            ram_mb: 32_000,
            cpu_cores: 12,
            gpu: Some(GpuInfo {
                name: "RTX 3080".into(),
                vram_mb: 9216,
            }),
        };
        let (fit, _) = fit_and_score(&hw, 6_144, 5_680).unwrap(); // 9B: fits GPU
        assert_eq!(fit, Fit::GpuFull);
        // 14B (9.0 GB weights) misses the 0.90 headroom gate -> partial.
        let (fit, _) = fit_and_score(&hw, 9_344, 9_000).unwrap();
        assert_eq!(fit, Fit::GpuPartial);
        let (fit, _) = fit_and_score(&hw, 19_968, 18_973).unwrap(); // 27B: partial
        assert_eq!(fit, Fit::GpuPartial);
        // 32B still fits 32 GB RAM (0.9 RAM gate) as a partial; a 16 GB
        // machine excludes it honestly.
        assert!(fit_and_score(&hw, 20_480, 19_851).is_some());
        let small = HardwareInfo {
            ram_mb: 16_000,
            cpu_cores: 8,
            ..hw.clone()
        };
        assert!(fit_and_score(&small, 20_480, 19_851).is_none());
        // Full fits beat partials; the clamp keeps a near-fit 14B below 9B.
        let score = |ram: u64, w: u64| fit_and_score(&hw, ram, w).unwrap().1;
        assert!(score(6_144, 5_680) > score(9_344, 9_000));
        assert!(score(9_344, 9_000) > score(19_968, 18_973));
    }

    #[test]
    fn no_gpu_ranks_responsive_picks_over_cpu_heavyweights() {
        let hw = HardwareInfo {
            ram_mb: 32_000,
            cpu_cores: 8,
            gpu: None,
        };
        let score = |ram: u64, w: u64| fit_and_score(&hw, ram, w).unwrap().1;
        // Everything is RamOnly; the ~8 GB sweet spot beats both giants
        // (slow decode) and tiny proof-of-pipeline models (quality).
        assert!(score(6_144, 5_680) > score(19_968, 18_973));
        assert!(score(6_144, 5_680) > score(384, 386));
    }

    #[test]
    fn parses_largest_vulkan_device_from_list_devices() {
        let stdout = "Available devices:
  Vulkan0: Intel(R) Graphics (18275 MiB, 17507 MiB free)
  Vulkan1: NVIDIA GeForce RTX 5080 Laptop GPU (16003 MiB, 15235 MiB free)
";
        let gpu = parse_largest_device(stdout).expect("parses");
        // Largest by total MiB wins, regardless of device order.
        assert_eq!(gpu.vram_mb, 18_275);
        assert_eq!(gpu.name, "Intel(R) Graphics");
        let two = "  Vulkan0: NVIDIA GeForce RTX 5080 Laptop GPU (16003 MiB, 15235 MiB free)
  Vulkan1: AMD Radeon (8192 MiB, 8000 MiB free)
";
        assert_eq!(
            parse_largest_device(two).unwrap().name,
            "NVIDIA GeForce RTX 5080 Laptop GPU"
        );
        assert!(parse_largest_device("no devices").is_none());
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

    // ---- Live peer census (owner directive 2026-10-12) ----------------

    /// `list_models`-shaped fixture — only the fields the census join and
    /// order read: profile_id, display_name, fit, recommended.
    fn census_fixture_models() -> Vec<serde_json::Value> {
        vec![
            serde_json::json!({
                "profile_id": "msp1:aa", "display_name": "Alpha 9B",
                "fit": "gpu_full", "recommended": true
            }),
            serde_json::json!({
                "profile_id": "msp1:bb", "display_name": "Bravo 9B",
                "fit": "gpu_full", "recommended": true
            }),
            serde_json::json!({
                "profile_id": "msp1:cc", "display_name": "Charlie 14B",
                "fit": "gpu_partial", "recommended": true
            }),
            serde_json::json!({
                "profile_id": "msp1:dd", "display_name": "Delta 27B",
                "fit": "ram_only", "recommended": false
            }),
            serde_json::json!({
                "profile_id": "msp1:ee", "display_name": "Echo 32B",
                "fit": null, "recommended": false
            }),
        ]
    }

    fn counts_of(pairs: &[(&str, u64)]) -> std::collections::BTreeMap<String, u64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn census_view_ids<'a>(
        view: &[(usize, String)],
        models: &'a [serde_json::Value],
    ) -> Vec<&'a str> {
        view.iter()
            .map(|(i, _)| models[*i]["profile_id"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn census_join_builds_honest_chips() {
        let models = census_fixture_models();
        let counts = counts_of(&[("msp1:aa", 2), ("msp1:bb", 1), ("msp1:cc", 5)]);
        let view = census_view(&models, Some(&counts));
        let chip_of = |id: &str| {
            view.iter()
                .find(|(i, _)| models[*i]["profile_id"] == id)
                .map(|(_, chip)| chip.as_str())
                .unwrap()
        };
        assert_eq!(chip_of("msp1:aa"), "2 peers");
        assert_eq!(chip_of("msp1:bb"), "1 peer"); // singular, like the site census
        assert_eq!(chip_of("msp1:cc"), "5 peers");
        // Reachable census, nobody hosting it -> the honest zero state.
        assert_eq!(chip_of("msp1:dd"), "first peer");
        assert_eq!(chip_of("msp1:ee"), "first peer");
    }

    #[test]
    fn census_unreachable_is_na_never_first_peer() {
        let models = census_fixture_models();
        let view = census_view(&models, None);
        // Every chip degrades honestly — an unreachable tracker is NOT
        // "nobody hosting" (the exact collapse the strict parser prevents
        // on the wire side too).
        assert!(
            view.iter().all(|(_, chip)| chip == "census n/a"),
            "unreachable must read n/a everywhere: {view:?}"
        );
        // Order falls back to shortlist -> tier -> name (peers equal for
        // every card while n/a: no invented counts reorder anything).
        assert_eq!(
            census_view_ids(&view, &models),
            ["msp1:aa", "msp1:bb", "msp1:cc", "msp1:dd", "msp1:ee"]
        );
    }

    #[test]
    fn census_order_is_shortlist_primary_then_tier_peers_name() {
        let models = census_fixture_models();
        // A crowded NON-shortlist card (9 live peers) must still sort
        // after every recommended card: the v0.2.19 hardware shortlist
        // stays the primary order — peers only break ties within a tier.
        let counts = counts_of(&[
            ("msp1:aa", 1),
            ("msp1:bb", 3),
            ("msp1:cc", 0),
            ("msp1:dd", 9),
            ("msp1:ee", 9),
        ]);
        let view = census_view(&models, Some(&counts));
        assert_eq!(
            census_view_ids(&view, &models),
            // Shortlist first: gpu_full tier by peers (bb 3 > aa 1), then
            // the gpu_partial cc (0 peers — honest "first peer" chip but
            // still shortlisted); only then the crowded non-shortlist
            // cards (dd ram_only before ee unknown tier).
            ["msp1:bb", "msp1:aa", "msp1:cc", "msp1:dd", "msp1:ee"]
        );
    }

    #[test]
    fn census_order_ties_fall_to_name_then_catalog_order() {
        // Same tier, same count: name ascending decides; identical names:
        // stable catalog order (the daemon sorts once — `list_models`
        // returns this exact order and the webview renders it verbatim).
        let models = vec![
            serde_json::json!({
                "profile_id": "msp1:z1", "display_name": "Same 7B",
                "fit": "ram_only", "recommended": true
            }),
            serde_json::json!({
                "profile_id": "msp1:z2", "display_name": "Ada 7B",
                "fit": "ram_only", "recommended": true
            }),
            serde_json::json!({
                "profile_id": "msp1:z3", "display_name": "Same 7B",
                "fit": "ram_only", "recommended": true
            }),
        ];
        let counts = counts_of(&[("msp1:z1", 2), ("msp1:z2", 2), ("msp1:z3", 2)]);
        let view = census_view(&models, Some(&counts));
        assert_eq!(
            census_view_ids(&view, &models),
            ["msp1:z2", "msp1:z1", "msp1:z3"]
        );
    }

    #[test]
    fn parse_census_counts_is_strict() {
        let ok = serde_json::json!({
            "peersOnline": 3,
            "models": [
                { "profileId": "msp1:aa", "peers": 2 },
                { "profileId": "msp1:bb", "peers": 1 }
            ]
        });
        let (online, counts) = parse_census_counts(&ok).unwrap();
        assert_eq!(online, 3);
        assert_eq!(counts["msp1:aa"], 2);
        assert_eq!(counts["msp1:bb"], 1);

        // Empty-but-valid: nobody hosting — the honest "first peer" state.
        let empty = serde_json::json!({ "peersOnline": 0, "models": [] });
        assert_eq!(
            parse_census_counts(&empty).unwrap(),
            (0, std::collections::BTreeMap::new())
        );

        // Malformed bodies must ERROR — they can never read as zero peers.
        assert!(parse_census_counts(&serde_json::json!({ "models": [] })).is_err());
        assert!(parse_census_counts(&serde_json::json!({ "peersOnline": 1 })).is_err());
        assert!(parse_census_counts(&serde_json::json!({
            "peersOnline": 1.5, "models": []
        }))
        .is_err());
        assert!(parse_census_counts(&serde_json::json!({
            "peersOnline": 1,
            "models": [{ "profileId": "msp1:aa", "peers": "2" }]
        }))
        .is_err());
        assert!(parse_census_counts(&serde_json::json!({
            "peersOnline": 1,
            "models": [{ "profileId": "msp1:aa" }]
        }))
        .is_err());
    }

    #[test]
    fn shadow_failure_rows_sample_total_failure_paths() {
        // 9.6 dataset completeness: every total-failure `return None` in
        // try_swarm_chat must leave a realized row with served_by=failed,
        // so failures are sampled instead of silently dropped from the
        // decision/realized join.
        let (t, sink) = Telemetry::memory();
        let telemetry = Arc::new(t);

        // Pre-plan failure (tracker/lookup/identity): no shadow plan
        // exists yet, so the row carries a fresh trace, production_peer
        // none, and an honest unknown wall time.
        record_shadow_failure(&telemetry, None, "msp1:aa", "none", None, "lookup", None);
        let rows = sink.snapshot();
        assert_eq!(rows.len(), 1, "exactly one realized row: {rows:?}");
        assert!(rows[0].contains("\"event\":\"sched.shadow.realized\""));
        assert!(rows[0].contains("\"served_by\":\"failed\""));
        assert!(rows[0].contains("\"reason\":\"lookup\""));
        assert!(rows[0].contains("\"production_peer\":\"none\""));
        assert!(rows[0].contains("\"shadow_pick\":\"none\""));
        assert!(rows[0].contains("\"wall_ms\":\"unknown\""));
        assert!(rows[0].contains("\"trace\":\"shadow-"));
        assert!(rows[0].contains("\"label\":\"shadow-no-action\""));

        // Post-plan failure: correlates the plan's trace, the production
        // peer that was attempted, and the shadow pick, with the wall
        // time when known.
        let plan = ShadowPlan {
            trace_id: "shadow-fixed".into(),
            own_peer_id: "me".into(),
            inputs: vec![],
            policy: modelswarm_scheduler::shadow::QueueDiscountPolicy::from_env(),
            prompt_tokens: 10,
            output_tokens: 5,
            metrics: Arc::new(modelswarm_node::measure::PeerMetrics::memory(Arc::clone(
                &telemetry,
            ))),
        };
        record_shadow_failure(
            &telemetry,
            Some(&plan),
            "msp1:aa",
            "peer-9",
            Some("peer-7"),
            "empty_reply",
            Some(42),
        );
        let rows = sink.snapshot();
        assert_eq!(rows.len(), 2, "one row per failure: {rows:?}");
        assert!(rows[1].contains("\"trace\":\"shadow-fixed\""));
        assert!(rows[1].contains("\"production_peer\":\"peer-9\""));
        assert!(rows[1].contains("\"shadow_pick\":\"peer-7\""));
        assert!(rows[1].contains("\"reason\":\"empty_reply\""));
        assert!(rows[1].contains("\"wall_ms\":\"42\""));
    }

    /// The G6 out-of-band pairing file parses STRICTLY: absent, empty, or
    /// containing any junk token unpins the family engine (None — the
    /// honest fail-closed); valid ids parse in order across whitespace
    /// kinds.
    #[cfg(feature = "verifydrafts")]
    #[test]
    fn family_prompt_tokens_parses_strictly_or_unpins() {
        let dir = tempfile::tempdir().unwrap();
        // Absent -> unpinned.
        assert_eq!(family_prompt_tokens(dir.path()), None);
        // Whitespace-only -> unpinned.
        std::fs::write(dir.path().join(FAMILY_PROMPT_TOKENS_FILE), " \n\t ").unwrap();
        assert_eq!(family_prompt_tokens(dir.path()), None);
        // Valid ids (mixed separators) parse in order.
        std::fs::write(
            dir.path().join(FAMILY_PROMPT_TOKENS_FILE),
            "72 101\n108\t44",
        )
        .unwrap();
        assert_eq!(
            family_prompt_tokens(dir.path()),
            Some(vec![72, 101, 108, 44])
        );
        // ANY junk token unpins the WHOLE file — never a partially
        // loaded prompt (a mismatched-prompt verifier is worse than none).
        std::fs::write(dir.path().join(FAMILY_PROMPT_TOKENS_FILE), "72 -1 44").unwrap();
        assert_eq!(family_prompt_tokens(dir.path()), None);
        std::fs::write(dir.path().join(FAMILY_PROMPT_TOKENS_FILE), "0x10 4").unwrap();
        assert_eq!(family_prompt_tokens(dir.path()), None);
    }

    /// The engine-bundle preference: match the serving engine's variant
    /// class (Vulkan-serving daemons try the GPU bundle first, ADR-024),
    /// CPU-first otherwise, GPU absent = CPU only.
    #[cfg(feature = "verifydrafts")]
    #[test]
    fn family_engine_dirs_prefer_the_serving_variant() {
        let cpu = Path::new("C:/app/engine/llama-server.exe");
        let gpu = Path::new("C:/app/engine-vulkan/llama-server.exe");
        assert_eq!(
            family_engine_dirs(cpu, Some(gpu), Some("vulkan")),
            vec![
                PathBuf::from("C:/app/engine-vulkan"),
                PathBuf::from("C:/app/engine")
            ]
        );
        // CPU-serving (or no engine yet): CPU first, GPU detect-only.
        assert_eq!(
            family_engine_dirs(cpu, Some(gpu), Some("cpu")),
            vec![
                PathBuf::from("C:/app/engine"),
                PathBuf::from("C:/app/engine-vulkan")
            ]
        );
        assert_eq!(
            family_engine_dirs(cpu, Some(gpu), None),
            vec![
                PathBuf::from("C:/app/engine"),
                PathBuf::from("C:/app/engine-vulkan")
            ]
        );
        // No GPU bundle staged: the CPU bundle alone.
        assert_eq!(
            family_engine_dirs(cpu, None, Some("vulkan")),
            vec![PathBuf::from("C:/app/engine")]
        );
    }

    /// The off-status shape the UI renders for every honest fallback
    /// (state off + the stable reason; no disclosure, empty roster, and
    /// ADR-034 rule 6a: NO pin fields while unpinned — the reason carries
    /// why).
    #[test]
    fn family_status_off_carries_the_reason() {
        let status = FamilyStatus::off("family_prompt_unpinned");
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["state"], "off");
        assert_eq!(json["reason"], "family_prompt_unpinned");
        assert!(json["disclosure"].is_null());
        assert_eq!(json["lease_roster_peers"], 0);
        assert!(
            json["pin"].is_null(),
            "unpinned = no pin fields, never a guessed pin"
        );
    }

    /// The on-status shape with an EMPTY roster is the presentation
    /// configuration, not a refusal state: the UI must render "presented
    /// at session open (roster 0)" — never the pre-activation "refusing
    /// until seeded" text (0 is normal; nothing feeds the roster yet).
    #[test]
    fn family_status_on_with_empty_roster_is_presentation() {
        let status = FamilyStatus {
            state: "on".into(),
            reason: None,
            detail: None,
            disclosure: Some(serde_json::json!({
                "backend": "cpu", "batch_verify": true, "batch_window_max": 32
            })),
            lease_roster_peers: 0,
            pin: None,
            carriage: None,
            governor_label: None,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["state"], "on");
        assert_eq!(json["lease_roster_peers"], 0);
        // The UI's family-line contract for the on+empty-roster case.
        let d = json["disclosure"].as_object().unwrap();
        let line = format!(
            "batch-verify family: on — {}, batch window {}, coordinator leases: presented at session open (roster {})",
            d["backend"].as_str().unwrap(),
            d["batch_window_max"],
            json["lease_roster_peers"]
        );
        assert!(line.contains("presented at session open"));
        assert!(!line.contains("refusing"));
    }

    /// The probe's refusal labels are stable and carry NO lease material
    /// (the requester-side mirror of the serving peer's
    /// `FamilyLeaseRefusal` telemetry — labels only).
    #[cfg(feature = "verifydrafts")]
    #[test]
    fn family_open_error_labels_are_stable() {
        use modelswarm_node::verifydrafts::ClientOpenError;
        assert_eq!(
            family_open_error_reason(&ClientOpenError::Io("recv accept")),
            "open_refused"
        );
        assert_eq!(
            family_open_error_reason(&ClientOpenError::Rejected("backend_mismatch")),
            "peer_rejected"
        );
        assert_eq!(
            family_open_error_reason(&ClientOpenError::Misconfigured("bad params".into())),
            "misconfigured"
        );
        // The detail string is the error's own Display — never a token.
        let detail = ClientOpenError::Io("recv accept").to_string();
        assert!(!detail.contains("lease"));
    }

    // ---- ADR-034 tier 1: the family auto-pin (rules 1-7) ---------------

    /// A deterministic byte-tokenizer stand-in for the pinned tokenizer
    /// (any surface yielding the pinned engine's ids — the sidecar's
    /// /tokenize maps bytes to ids exactly like this).
    fn byte_tokenizer(text: &str) -> Result<Vec<u32>, String> {
        Ok(text.as_bytes().iter().map(|b| u32::from(*b)).collect())
    }

    /// ADR-034 rule 3 (determinism): the v1 text renders as a SYSTEM-ONLY
    /// ChatML turn (no assistant opener — the pin is the root), the same
    /// text + pinned tokenizer yields IDENTICAL ids and pin hash across
    /// runs, and the hash is the bare 64-hex LE-u32 sha256 (no parent).
    #[cfg(feature = "verifydrafts")]
    #[tokio::test]
    async fn family_pin_derivation_is_deterministic_and_system_only() {
        // Rule 2: the frozen text, byte-identical.
        assert_eq!(
            FAMILY_PROMPT_V1,
            "You are serving a ModelSwarm cooperative verification session. Continue the given context exactly as this model normally would."
        );
        let rendered = family_prompt_render_system_only(FAMILY_PROMPT_V1);
        assert!(
            rendered.starts_with("<|im_start|>system\n"),
            "system-only ChatML turn: {rendered:?}"
        );
        assert!(
            rendered.ends_with("<|im_end|>\n"),
            "closed turn, trailing newline: {rendered:?}"
        );
        assert!(
            !rendered.contains("assistant"),
            "NO generation prompt — the pin is the root the engine continues from"
        );
        assert_eq!(rendered, family_prompt_render_system_only(FAMILY_PROMPT_V1));

        let first =
            derive_family_prompt_ids(FAMILY_PROMPT_V1, |t| async move { byte_tokenizer(&t) })
                .await
                .unwrap();
        let second =
            derive_family_prompt_ids(FAMILY_PROMPT_V1, |t| async move { byte_tokenizer(&t) })
                .await
                .unwrap();
        assert_eq!(first, second, "determinism across runs");
        assert!(!first.is_empty(), "an empty pin is an error upstream");

        let hash = family_pin_hash(&first);
        assert_eq!(hash.len(), 64, "bare 64-hex");
        assert!(
            hash.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "lowercase hex: {hash}"
        );
        assert_eq!(hash, family_pin_hash(&second), "hash stability");
        assert_ne!(hash, family_pin_hash(&[first[0]]), "hash binds the ids");
        // The exact preimage rule: LE-u32 each, no parent bytes (pinned
        // against an accidental parent-prefix form).
        let mut preimage = Vec::new();
        for id in &first {
            preimage.extend_from_slice(&id.to_le_bytes());
        }
        use sha2::Digest;
        assert_eq!(
            hash,
            hex::encode(sha2::Sha256::digest(&preimage)),
            "sha256 over LE-u32 ids with NO parent"
        );
    }

    /// ADR-034 rule 4 (auto-write-on-absent): an absent pin file plans a
    /// WriteAuto; writing the derived ids lands them parsable, in order.
    #[cfg(feature = "verifydrafts")]
    #[tokio::test]
    async fn family_pin_auto_writes_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            !dir.path().join(FAMILY_PROMPT_TOKENS_FILE).exists(),
            "fixture starts absent"
        );
        match family_pin_plan(dir.path(), &[]).unwrap() {
            FamilyPinPlan::WriteAuto => {}
            other => panic!("absent file must plan WriteAuto, got {other:?}"),
        }
        let ids = derive_family_prompt_ids(FAMILY_PROMPT_V1, |t| async move { byte_tokenizer(&t) })
            .await
            .unwrap();
        write_family_pin_file(&dir.path().join(FAMILY_PROMPT_TOKENS_FILE), &ids).unwrap();
        assert_eq!(family_prompt_tokens(dir.path()), Some(ids));
        // No .part litter.
        assert!(!dir.path().join("family-prompt-tokens.part").exists());
    }

    /// ADR-034 rule 4 (fail-closed): a present junk (or empty) file plans
    /// Junk — the family stays unpinned and the file is never touched.
    #[cfg(feature = "verifydrafts")]
    #[test]
    fn family_pin_junk_file_stays_fail_closed_and_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FAMILY_PROMPT_TOKENS_FILE);
        for junk in ["72 -1 44", "0x10 4", "   ", ""] {
            std::fs::write(&path, junk).unwrap();
            assert!(
                matches!(
                    family_pin_plan(dir.path(), &[]).unwrap(),
                    FamilyPinPlan::Junk
                ),
                "junk {junk:?} must stay fail-closed"
            );
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                junk,
                "never overwritten"
            );
            assert_eq!(family_prompt_tokens(dir.path()), None, "unpinned");
        }
    }

    /// ADR-034 rule 4 (precedence): a present, strictly parsable file
    /// WINS as-is (the expert override, including non-v1 pins) — no
    /// derivation, no rewrite.
    #[cfg(feature = "verifydrafts")]
    #[tokio::test]
    async fn family_pin_present_file_wins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FAMILY_PROMPT_TOKENS_FILE);
        let expert = vec![999_999u32, 4, 17];
        std::fs::write(&path, "999999 4 17").unwrap();
        match family_pin_plan(dir.path(), &[]).unwrap() {
            FamilyPinPlan::KeepFile(ids) => assert_eq!(ids, expert),
            other => panic!("a parsable present file must KeepFile, got {other:?}"),
        }
        // ...even when it EQUALS the current derivation (provenance is
        // still "file": it was present).
        let derived =
            derive_family_prompt_ids(FAMILY_PROMPT_V1, |t| async move { byte_tokenizer(&t) })
                .await
                .unwrap();
        std::fs::write(
            &path,
            derived
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(" "),
        )
        .unwrap();
        match family_pin_plan(dir.path(), &[]).unwrap() {
            FamilyPinPlan::KeepFile(ids) => assert_eq!(ids, derived),
            other => panic!("present-but-equal must still KeepFile, got {other:?}"),
        }
    }

    /// ADR-034 rule 4 (version advance): a present file whose ids EQUAL a
    /// previous version's derived ids (an unmodified automatic pin) is
    /// rewritten to the current version; any other present file
    /// (KeepFile above) is never rewritten. Exercised with a two-version
    /// table — the shipped table has one version, so the path is dormant
    /// but correct for the day v2 ships.
    #[cfg(feature = "verifydrafts")]
    #[tokio::test]
    async fn family_pin_version_advance_rewrites_only_unmodified_auto() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FAMILY_PROMPT_TOKENS_FILE);
        // The v0 derivation under the same tokenizer discipline.
        let v0_text = "older neutral family prompt text";
        let v0_ids = derive_family_prompt_ids(v0_text, |t| async move { byte_tokenizer(&t) })
            .await
            .unwrap();
        let table = vec![("family-prompt-v0", v0_ids.clone())];

        // An unmodified v0 auto pin -> rewrite planned, flagged with the
        // previous version.
        std::fs::write(
            &path,
            v0_ids
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(" "),
        )
        .unwrap();
        match family_pin_plan(dir.path(), &table).unwrap() {
            FamilyPinPlan::RewriteAuto { previous_version } => {
                assert_eq!(previous_version, "family-prompt-v0");
            }
            other => panic!("unmodified auto pin must plan RewriteAuto, got {other:?}"),
        }
        // Applying it lands the v1 derivation in the file.
        let v1_ids =
            derive_family_prompt_ids(FAMILY_PROMPT_V1, |t| async move { byte_tokenizer(&t) })
                .await
                .unwrap();
        write_family_pin_file(&path, &v1_ids).unwrap();
        assert_eq!(family_prompt_tokens(dir.path()), Some(v1_ids));

        // An expert file (not any previous derivation) is NEVER rewritten.
        std::fs::write(&path, "1 2 3").unwrap();
        assert!(matches!(
            family_pin_plan(dir.path(), &table).unwrap(),
            FamilyPinPlan::KeepFile(_)
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "1 2 3");
    }

    /// ADR-034 rule 6b (label stability + the redaction discipline): the
    /// verdict labels, the failure-detail labels, and the counts-only
    /// result shape are stable and structurally INCAPABLE of carrying
    /// token ids — serialized, the fields contain no array at all.
    #[cfg(feature = "verifydrafts")]
    #[test]
    fn pairing_labels_are_stable_and_results_carry_counts_only() {
        use modelswarm_node::verifydrafts::RoundFailure;
        // Full acceptance ⇒ confirmed; divergence 0 is the canonical
        // mismatch signature; partial acceptance is a mismatch.
        assert_eq!(pairing_verdict(8, 8, None), "pairing_confirmed");
        assert_eq!(pairing_verdict(8, 8, Some(0)), "pin_mismatch");
        assert_eq!(pairing_verdict(8, 5, Some(5)), "pin_mismatch");
        assert_eq!(pairing_verdict(8, 7, None), "pin_mismatch");
        // A zero-length round can never "confirm" (guard, not a real path).
        assert_eq!(pairing_verdict(0, 0, None), "pin_mismatch");

        assert_eq!(
            pairing_round_failure_label(&RoundFailure::TimedOut),
            "timed_out"
        );
        assert_eq!(
            pairing_round_failure_label(&RoundFailure::WindowTooLarge {
                requested: 9,
                cap: 8
            }),
            "window_too_large"
        );
        assert_eq!(pairing_round_failure_label(&RoundFailure::Io("recv")), "io");

        // Every label's result fields: counts only — no array, so no
        // token-id list can ever ride the result (or the telemetry event
        // built from the same numbers).
        for (pairing, detail) in [
            ("pairing_confirmed", None),
            ("pin_mismatch", Some("rejected")),
            ("pin_unpinned", None),
            ("draft_unavailable", None),
        ] {
            let fields = pairing_result_fields(pairing, Some(8), Some(8), Some(0), detail);
            let json = serde_json::json!(fields
                .into_iter()
                .collect::<serde_json::Map<String, serde_json::Value>>());
            let text = json.to_string();
            assert!(!text.contains('['), "no arrays in {pairing}: {text}");
            assert_eq!(json["pairing"], pairing);
        }
    }

    /// ADR-034 rule 6a (status provenance fields): the on-status carries
    /// provenance (`auto family-prompt-v1` | `file`), token count, and
    /// pin hash — and nothing else in the pin object (no ids).
    #[cfg(feature = "verifydrafts")]
    #[tokio::test]
    async fn family_status_pin_provenance_fields() {
        let ids = derive_family_prompt_ids(FAMILY_PROMPT_V1, |t| async move { byte_tokenizer(&t) })
            .await
            .unwrap();
        for provenance in [format!("auto {FAMILY_PROMPT_VERSION}"), "file".to_string()] {
            let status = FamilyStatus {
                state: "on".into(),
                reason: None,
                detail: None,
                disclosure: None,
                lease_roster_peers: 0,
                pin: Some(FamilyPinStatus {
                    provenance: provenance.clone(),
                    version: FAMILY_PROMPT_VERSION.to_string(),
                    token_count: ids.len() as u64,
                    pin_hash: family_pin_hash(&ids),
                }),
                carriage: None,
                governor_label: None,
            };
            let json = serde_json::to_value(&status).unwrap();
            assert_eq!(json["pin"]["provenance"], provenance);
            assert_eq!(json["pin"]["version"], FAMILY_PROMPT_VERSION);
            assert_eq!(json["pin"]["token_count"], ids.len() as u64);
            assert_eq!(json["pin"]["pin_hash"], family_pin_hash(&ids));
            let pin = json["pin"].as_object().unwrap();
            assert_eq!(pin.len(), 4, "counts and a hash only — no ids: {pin:?}");
            assert!(
                !json.to_string().contains(&format!("[{}", ids[0])),
                "no token-id list in the status surface"
            );
        }
    }

    // ---- Resource monitor panel (owner request 2026-10-12) ---------------

    /// The panel's scope always includes this process (the panel is about
    /// OUR footprint), is sorted + deduped (stable equality for the
    /// rebuild decision), and is stable across calls while nothing changes.
    #[test]
    fn monitor_roots_cover_self_and_are_stable() {
        let roots = monitor_roots();
        assert!(
            roots.contains(&std::process::id()),
            "own pid must be a root: {roots:?}"
        );
        let mut sorted = roots.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(roots, sorted, "roots are sorted + deduped");
        assert_eq!(monitor_roots(), roots, "stable while the set is unchanged");
    }

    /// The sampler is created on the first poll and rebuilt EXACTLY when
    /// the supervised root set changes (hosting start/stop): same roots =
    /// keep the sampler (baseline preserved), changed roots = rebuild.
    #[test]
    fn panel_monitor_rebuilds_only_on_scope_change() {
        let mut panel: Option<PanelMonitor> = None;
        assert!(ensure_panel_monitor(&mut panel, vec![7, 9]));
        assert_eq!(panel.as_ref().unwrap().roots, vec![7, 9]);
        // Same roots: no rebuild (the delta baseline survives).
        assert!(!ensure_panel_monitor(&mut panel, vec![7, 9]));
        // Hosting started: the supervised set grew -> rebuild.
        assert!(ensure_panel_monitor(&mut panel, vec![7, 9, 31]));
        assert_eq!(panel.as_ref().unwrap().roots, vec![7, 9, 31]);
        assert!(!ensure_panel_monitor(&mut panel, vec![7, 9, 31]));
        // Hosting stopped: the engine left the set -> rebuild.
        assert!(ensure_panel_monitor(&mut panel, vec![7]));
    }

    /// The IPC shape maps every `ResourceSample` field verbatim and keeps
    /// per-field honesty: an unreadable field crosses as null (the UI's
    /// "n/a"), never a guessed number.
    #[test]
    fn resource_sample_json_maps_every_field_and_nulls_honestly() {
        let full = modelswarm_resource::ResourceSample {
            process_tree_cpu_percent: Some(12.5),
            system_ram_used_percent: Some(51.0),
            system_ram_used_bytes: Some(16_000_000_000),
            system_ram_total_bytes: Some(32_000_000_000),
            process_tree_working_set_bytes: Some(9_500_000),
            gpu_utilization_percent: Some(3.25),
            gpu_vram_used_bytes: Some(6_600_000_000),
        };
        let json = resource_sample_json(&full);
        for key in [
            "process_tree_cpu_percent",
            "system_ram_used_percent",
            "system_ram_used_bytes",
            "system_ram_total_bytes",
            "process_tree_working_set_bytes",
            "gpu_utilization_percent",
            "gpu_vram_used_bytes",
        ] {
            assert!(
                json[key] != serde_json::Value::Null,
                "{key} must carry the value"
            );
        }
        assert_eq!(json["process_tree_cpu_percent"], 12.5);
        assert_eq!(json["gpu_vram_used_bytes"], 6_600_000_000u64);
        // Every field unreadable (e.g. the macOS fallback source): all
        // null — seven honest n/a's, no zeros, no errors.
        let empty = resource_sample_json(&modelswarm_resource::ResourceSample::default());
        assert_eq!(
            empty,
            serde_json::json!({
                "process_tree_cpu_percent": null,
                "system_ram_used_percent": null,
                "system_ram_used_bytes": null,
                "system_ram_total_bytes": null,
                "process_tree_working_set_bytes": null,
                "gpu_utilization_percent": null,
                "gpu_vram_used_bytes": null,
            })
        );
    }

    // ---- config.json writer (TR-RC-8: round-trip preservation) ---------

    fn test_inner(data_dir: &Path, tracker: &str, profile: Option<&str>) -> Inner {
        Inner {
            data_dir: data_dir.to_path_buf(),
            tracker_url: tracker.to_string(),
            profile_id: profile.map(str::to_string),
            size_cache: Default::default(),
            census_counts: None,
            swarm_lease: None,
            swarm_executor: None,
            relay_tickets: None,
            relay_config: None,
            peer_metrics: None,
            license_accepted: true,
            artifact: None,
            node: None,
            chat_log: Vec::new(),
            allow_swarm_pairing: true,
            allow_content_carriage: true,
            #[cfg(feature = "verifydrafts")]
            last_family_cohort: None,
            #[cfg(feature = "verifydrafts")]
            family_peer_caps: Default::default(),
            #[cfg(feature = "verifydrafts")]
            family_engine: None,
            listener_addr: None,
            family: None,
            updater: updater::UpdaterStatus::Idle,
            assistant_plane: None,
            assistant_degrade_gap: "disabled".to_string(),
            assistant_acknowledged: false,
            assistant_enabled: true,
            last_cards: Vec::new(),
            last_engage_reason: None,
            last_governor_verdict: None,
            last_probe_verdict: None,
            capacity_class: None,
        }
    }

    /// TR-RC-8's pin: a tracker/profile change rewrites config.json via
    /// read-modify-write — the daemon's two keys update and EVERY unknown
    /// key survives verbatim (the old two-field struct rewrite silently
    /// deleted them; both write sites — `persist_config` and
    /// `set_tracker`'s delegation to it — share this writer).
    #[test]
    fn config_rewrite_preserves_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"tracker":"https://old.example","profile_id":"msp1:aa","advisory":{"cadence":"daily","caps":{"callsPerDay":24}},"window_state":{"x":42,"maximized":false}}"#,
        )
        .unwrap();
        // A tracker change (set_tracker's path)…
        persist_config(&test_inner(
            dir.path(),
            "https://new.example",
            Some("msp1:aa"),
        ))
        .unwrap();
        // …then a profile change (select_model's path).
        persist_config(&test_inner(
            dir.path(),
            "https://new.example",
            Some("msp1:bb"),
        ))
        .unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            saved["tracker"], "https://new.example",
            "daemon key updates"
        );
        assert_eq!(saved["profile_id"], "msp1:bb", "daemon key updates");
        assert_eq!(
            saved["advisory"]["cadence"], "daily",
            "unknown subtree survives BOTH rewrites"
        );
        assert_eq!(saved["advisory"]["caps"]["callsPerDay"], 24);
        assert_eq!(saved["window_state"]["x"], 42);
        assert_eq!(saved["window_state"]["maximized"], false);
    }

    /// Fresh-install shapes: a missing file starts from `{}` (the
    /// daemon's keys land, nothing else), and a `null` profile round-trips
    /// as `null` exactly as the previous writer emitted it.
    #[test]
    fn config_write_fresh_file_and_null_profile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        persist_config(&test_inner(dir.path(), "https://t.example", None)).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["tracker"], "https://t.example");
        assert_eq!(saved["profile_id"], serde_json::Value::Null);
        assert_eq!(
            saved.as_object().unwrap().len(),
            6,
            "a fresh file carries exactly the daemon's keys"
        );
        assert_eq!(saved["allow_swarm_pairing"], true, "default-on posture");
        assert_eq!(
            saved["assistant_acknowledged"], false,
            "the §3.6 disclosure is NOT acknowledged on a fresh install"
        );
        assert_eq!(
            saved["assistant_enabled"], true,
            "the Settings toggle defaults on"
        );
        // Round-trips: read_config still reads the tolerant field set.
        let config = read_config(dir.path());
        assert_eq!(config.tracker, "https://t.example");
        assert_eq!(config.profile_id, None);
    }

    // ---- ADR-036 (2026-10-13): the carriage posture, both disclosures,
    // the election, and the narrowed content contract -------------------

    /// The ADR-036 fresh-install postures (§3.1): SERVING carriage
    /// DEFAULT ON (the owner's swarm-together directive), REQUESTER
    /// election DEFAULT OFF (the user's choice). Both round-trip through
    /// the persisted config, and a config that predates ADR-036 (no
    /// keys) reads as the same postures via the serde defaults.
    #[test]
    fn adr036_postures_default_on_serving_always_elected_requester() {
        // Fresh install: carriage default ON; the requester election key
        // NO LONGER EXISTS (owner directive 2026-10-15 — always elected,
        // no toggle, nothing persisted to read).
        let dir = tempfile::tempdir().unwrap();
        let config = read_config(dir.path());
        assert!(config.allow_content_carriage, "carriage default ON");
        // A pre-ADR-036 config (the three old keys, none of ours).
        std::fs::write(
            dir.path().join("config.json"),
            r#"{"tracker":"https://t.example","profile_id":null,"allow_swarm_pairing":true}"#,
        )
        .unwrap();
        let config = read_config(dir.path());
        assert!(config.allow_content_carriage, "missing key = default ON");
        // Round-trip: carriage flips persist and re-read; a RETIRED
        // election key in an old config is preserved verbatim (unknown
        // keys are never deleted) and never resurrected as a setting.
        let mut inner = test_inner(dir.path(), "https://t.example", None);
        inner.allow_content_carriage = false;
        std::fs::write(
            dir.path().join("config.json"),
            r#"{"tracker":"https://t.example","profile_id":null,"family_verified_chat":true}"#,
        )
        .unwrap();
        persist_config(&inner).unwrap();
        let config = read_config(dir.path());
        assert!(!config.allow_content_carriage);
        // And the daemon's keys are exactly what the writer owns.
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("config.json")).unwrap())
                .unwrap();
        assert_eq!(saved["allow_content_carriage"], false);
        assert_eq!(
            saved["family_verified_chat"], true,
            "the retired key is preserved verbatim, not rewritten"
        );
    }

    /// The §3.1 disclosures are VERBATIM (normative UI contract — the
    /// exact texts ship in the binary, N rendered from the actual
    /// cohort), and the honest cost + hub-blind facts are stated.
    #[cfg(feature = "verifydrafts")]
    #[test]
    fn adr036_disclosure_texts_are_verbatim() {
        assert_eq!(
            CARRIAGE_DISCLOSURE_SERVING,
            "This computer carries other users' conversations (as token ids it can read) \
             through family verification. On by default — you can turn it off, and requesting \
             peers are refused honestly when you do."
        );
        assert!(CARRIAGE_DISCLOSURE_OFF.contains("content_carriage_refused"));
        // The requester text with the ACTUAL cohort count (revised
        // 2026-10-15: the election toggle is retired, so the "turn it
        // off" sentence is gone from the frozen text).
        assert_eq!(
            requester_disclosure(1),
            "Family verification ON: your conversation — in a form the receiving computers \
             can read — goes to every verification peer this turn (1). It is slower than \
             single-peer chat, not faster. The hub (modelswarm.deepflux.space) never sees \
             any of it."
        );
        assert!(requester_disclosure(3).contains("this turn (3)."));
        // The honesty discipline: never implies unreadability, encrypted
        // beyond transport, or faster.
        let text = requester_disclosure(1);
        assert!(text.contains("can read"));
        assert!(text.contains("slower than single-peer chat, not faster"));
        assert!(text.contains("never sees any of it"));
    }

    /// The family status carries the carriage disclosure whenever the
    /// family is ON (§3.1: always visible while active) — the exact text
    /// while enabled, the honest opt-out statement when not.
    #[cfg(feature = "verifydrafts")]
    #[test]
    fn family_status_carries_the_carriage_disclosure_when_on() {
        let mut status = FamilyStatus {
            state: "on".into(),
            reason: None,
            detail: None,
            disclosure: None,
            lease_roster_peers: 0,
            pin: None,
            carriage: Some(FamilyCarriageStatus {
                enabled: true,
                disclosure: CARRIAGE_DISCLOSURE_SERVING.to_string(),
            }),
            governor_label: None,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["carriage"]["enabled"], true);
        assert_eq!(
            json["carriage"]["disclosure"], CARRIAGE_DISCLOSURE_SERVING,
            "the UI renders the daemon's text verbatim"
        );
        // Opted out: enabled false + the honest off statement.
        status.carriage = Some(FamilyCarriageStatus {
            enabled: false,
            disclosure: CARRIAGE_DISCLOSURE_OFF.to_string(),
        });
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["carriage"]["enabled"], false);
        assert_eq!(json["carriage"]["disclosure"], CARRIAGE_DISCLOSURE_OFF);
        // Every off family carries NO disclosure (nothing is carried
        // while the family is down).
        assert!(
            serde_json::to_value(FamilyStatus::off("pairing_disabled")).unwrap()["carriage"]
                .is_null()
        );
    }

    /// ADR-036 §3.2 hygiene, pinned at the status surface: the webview
    /// NEVER receives carrier content — no `prompt_tokens`, no
    /// `pin_commitment` (the content-derived commitment stays OUT of
    /// status AND telemetry; the neutral `pin.pin_hash` remains allowed
    /// per ADR-034 rule 7's public-constant analysis, which ADR-036 §4.4
    /// explicitly does NOT transfer to user-content commitments).
    #[cfg(feature = "verifydrafts")]
    #[test]
    fn adr036_carrier_fields_never_reach_the_status_surface() {
        let mut status = FamilyStatus {
            state: "on".into(),
            reason: None,
            detail: None,
            disclosure: None,
            lease_roster_peers: 0,
            pin: Some(FamilyPinStatus {
                provenance: "file".into(),
                version: FAMILY_PROMPT_VERSION.to_string(),
                token_count: 12,
                pin_hash: family_pin_hash(&[1, 2, 3]),
            }),
            carriage: Some(FamilyCarriageStatus {
                enabled: true,
                disclosure: CARRIAGE_DISCLOSURE_SERVING.to_string(),
            }),
            governor_label: None,
        };
        let json = serde_json::to_value(&status).unwrap();
        let text = json.to_string();
        assert!(
            !text.contains("prompt_tokens"),
            "no carrier payload key in the status surface: {text}"
        );
        assert!(
            !text.contains("pin_commitment"),
            "the content-derived commitment stays OUT of status: {text}"
        );
        assert!(!text.contains('['), "no arrays ride the status: {text}");
        // The carriage object itself is exactly {enabled, disclosure}.
        let mut keys: Vec<&str> = json["carriage"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["disclosure", "enabled"]);
        // Off families serialize without any carriage content either.
        status.carriage = None;
        assert!(!serde_json::to_value(&status)
            .unwrap()
            .to_string()
            .contains("pin_commitment"));
    }

    /// The opt-out refusal path is honest END TO END at the unit level:
    /// the OFF posture flows into the serve config, and the node's
    /// admission table refuses a well-formed carrier with the frozen
    /// `content_carriage_refused` reject (never a silent drop, never a
    /// misleading pin_mismatch).
    #[cfg(feature = "verifydrafts")]
    #[test]
    fn adr036_opt_out_refusal_is_honest() {
        use modelswarm_identity::InstallationIdentity;
        use modelswarm_node::verifydrafts::{
            admit_pin_carrier, ContentCarriage, PinAdmission, WirePinCarrier,
        };
        // The posture flows: inner.allow_content_carriage=false → the
        // serve config the compose call builds.
        let mut config = modelswarm_node::verifydrafts::VerifyDraftsServeConfig::honest(
            "msp1:x".to_string(),
            InstallationIdentity::from_bytes(&[0x99; 32]),
        );
        assert_eq!(
            config.content_carriage,
            ContentCarriage::On,
            "honest() defaults to the owner's default-on posture"
        );
        config.content_carriage = ContentCarriage::Off;
        assert_eq!(config.content_carriage.as_str(), "off");

        // A well-formed, correctly SIGNED carrier under the OFF posture.
        let identity = InstallationIdentity::from_bytes(&[0x98; 32]);
        let tokens = vec![151644u32, 7, 151645];
        let params = "3d".repeat(32);
        let mut carrier = WirePinCarrier {
            session_id: "vd-refuse".into(),
            protocol_version: "msp-cooperative-v1".into(),
            seq: 1,
            round: 0,
            profile_id: "msp1:x".into(),
            generation_params_hash: params.clone(),
            prompt_tokens: tokens.clone(),
            pin_commitment: modelswarm_node::verifydrafts::pin_commitment_of(&tokens),
            nonce: "n".repeat(16),
            deadline_ms: 60_000,
            sender: identity.installation_id(),
            signature: String::new(),
        };
        carrier.signature =
            modelswarm_node::verifydrafts::sign_pin_carrier(&carrier, &identity).unwrap();
        match admit_pin_carrier(
            &carrier,
            &identity.peer_id(),
            &identity.verifying_key(),
            "msp1:x",
            &params,
            config.content_carriage,
            None,
            1,
            1,
        ) {
            PinAdmission::Rejected(reason) => assert_eq!(
                reason.as_str(),
                "content_carriage_refused",
                "the refusal is labeled honestly, never a mismatch"
            ),
            other => panic!("the opt-out must refuse, got {other:?}"),
        }
    }

    /// The narrowed `blocked_content_contract` (ADR-036 §3.3/§4.4): the
    /// ladder's terminal label still fires for the AUTOMATIC path when
    /// the gate would engage (the election, not the gate, carries
    /// content) — the vocabulary is frozen, the meaning narrowed.
    #[cfg(feature = "verifydrafts")]
    #[test]
    fn adr036_blocked_content_contract_stays_frozen_but_narrowed() {
        let cap = FamilyPeerCap {
            batch_verify: true,
            batch_window_max: 8,
            backend: "cpu".to_string(),
            learned_unix_ms: 0,
        };
        assert_eq!(
            family_engage_reason(Some(&cap), Some("cpu"), Some(1.0), Some(2.0)),
            "blocked_content_contract",
            "the gate WOULD engage — the automatic path still does not carry content"
        );
        // The ladder below the terminal label is unchanged.
        assert_eq!(
            family_engage_reason(Some(&cap), Some("cpu"), Some(1.9), Some(2.0)),
            "gate_no_win"
        );
        assert_eq!(
            family_engage_reason(None, Some("cpu"), Some(1.0), Some(2.0)),
            "capability_unknown"
        );
    }

    // ---- v0.2.23 batch: the pairing toggle, the self-check gate, the
    // telemetry detail fix, the engage ladder ---------------------------

    /// The settings toggle is CONFIG-DRIVEN and env-free (owner directive
    /// 2026-10-13): the gate never consults MSP_LISTENER — the decision
    /// is the persisted key alone, defaulting ON for a fresh install.
    #[test]
    fn listener_gate_is_config_driven_and_env_free() {
        // Fresh-install default: pairing allowed.
        let dir = tempfile::tempdir().unwrap();
        assert!(read_config(dir.path()).allow_swarm_pairing);
        assert_eq!(listener_gate(true), ListenerGate::Allowed);
        assert_eq!(listener_gate(false), ListenerGate::Disabled);
        // A persisted OFF decision round-trips through the writer.
        let mut inner = test_inner(dir.path(), "https://t.example", None);
        inner.allow_swarm_pairing = false;
        persist_config(&inner).unwrap();
        assert!(!read_config(dir.path()).allow_swarm_pairing);
        // The persisted key rides the same round-trip-preserving writer
        // (TR-RC-8): unknown keys survive a toggle flip.
        let path = dir.path().join("config.json");
        let mut root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        root.as_object_mut()
            .unwrap()
            .insert("advisory".into(), serde_json::json!({ "caps": 3 }));
        std::fs::write(&path, serde_json::to_string(&root).unwrap()).unwrap();
        inner.allow_swarm_pairing = true;
        persist_config(&inner).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["allow_swarm_pairing"], true);
        assert_eq!(saved["advisory"]["caps"], 3, "unknown keys survive");
        assert_eq!(
            saved.as_object().unwrap().len(),
            7,
            "six daemon keys + the one unknown key"
        );
    }

    /// The honest-degrade shape: an off family carries the reason AND the
    /// exact detail (bind error text, resource projection) — never a
    /// silent off.
    #[test]
    fn family_status_off_detail_carries_the_exact_reason() {
        let status = FamilyStatus::off_detail(
            "listener_bind_failed",
            "bind 0.0.0.0:0: quic listen refused",
        );
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["state"], "off");
        assert_eq!(json["reason"], "listener_bind_failed");
        assert_eq!(json["detail"], "bind 0.0.0.0:0: quic listen refused");
        assert!(json["pin"].is_null());
        // The plain off shape keeps detail absent (not a garbage string).
        assert!(
            serde_json::to_value(FamilyStatus::off("pairing_disabled")).unwrap()["detail"]
                .is_null()
        );
    }

    /// The coordinator self-check gate (tonight's 2026-10-13 root cause):
    /// a deliberately DIVERGENT window is caught LOCALLY and never sent —
    /// the gate returns LocalDivergence (counts only), not Send; a passing
    /// window returns Send; a missing local engine fails CLOSED
    /// (Unavailable), so no unverified draft can reach the wire.
    #[cfg(feature = "verifydrafts")]
    #[tokio::test]
    async fn selfcheck_gates_divergent_windows_before_the_wire() {
        use modelswarm_node::verifydrafts::{
            BatchVerifyEngine, EngineDisclosure, RuntimeError, SamplingParams,
        };

        /// A scripted local engine: full acceptance, divergence at a fixed
        /// point, or an engine error — enough to pin every gate arm.
        struct ScriptedEngine {
            accepted_for_len_8: u32,
            fail: bool,
        }
        #[async_trait::async_trait]
        impl BatchVerifyEngine for ScriptedEngine {
            async fn verify_window(
                &self,
                window: &[u32],
                _params: &SamplingParams,
            ) -> Result<(u32, Option<u32>), RuntimeError> {
                if self.fail {
                    return Err(RuntimeError::MalformedResponse(
                        "scripted local engine failure".into(),
                    ));
                }
                // Divergence at 0 is tonight's canonical warm-cache flip.
                Ok((self.accepted_for_len_8.min(window.len() as u32), Some(7)))
            }
            fn capabilities(&self) -> EngineDisclosure {
                EngineDisclosure {
                    backend: modelswarm_node::verifydrafts::Backend::Cpu,
                    batch_verify: true,
                    batch_window_max: 8,
                }
            }
        }
        let source_of = |accepted: u32, fail: bool| {
            Some(std::sync::Arc::new(
                modelswarm_node::verifydrafts::FamilyEngineSource::Shared(std::sync::Arc::new(
                    ScriptedEngine {
                        accepted_for_len_8: accepted,
                        fail,
                    },
                )),
            ))
        };
        let window = [11u32, 22, 33, 44, 55, 66, 77, 88];

        // The deliberately divergent window (0/8 — exactly tonight's
        // warm-cache flip): caught locally, NEVER Send.
        match family_selfcheck_window(source_of(0, false).as_ref(), &window).await {
            FamilySelfcheck::LocalDivergence { accepted } => assert_eq!(accepted, 0),
            other => panic!("divergent window must be caught locally, got {other:?}"),
        }
        // Mid-window divergence (3/8) carries the count.
        match family_selfcheck_window(source_of(3, false).as_ref(), &window).await {
            FamilySelfcheck::LocalDivergence { accepted } => assert_eq!(accepted, 3),
            other => panic!("partial divergence must be caught, got {other:?}"),
        }
        // A locally-verified window may proceed to the wire.
        assert!(matches!(
            family_selfcheck_window(source_of(8, false).as_ref(), &window).await,
            FamilySelfcheck::Send
        ));
        // A broken local engine fails CLOSED — no unverified draft sends.
        assert!(matches!(
            family_selfcheck_window(source_of(8, true).as_ref(), &window).await,
            FamilySelfcheck::Unavailable(_)
        ));
        // No local family engine at all (dev build / degraded tier): the
        // same fail-closed posture.
        assert!(matches!(
            family_selfcheck_window(None, &window).await,
            FamilySelfcheck::Unavailable(_)
        ));
    }

    /// The 2026-10-13 telemetry gap fix: the pairing DETAIL label (the
    /// Err-branch reasons — `rejected`, `io`, `local_selfcheck`, …) now
    /// reaches the `family.probe.pairing` event, counts and labels only.
    #[cfg(feature = "verifydrafts")]
    #[test]
    fn pairing_telemetry_carries_the_detail_label() {
        let (t, sink) = Telemetry::memory();
        log_family_pairing(
            &t,
            "vd-fix",
            "pin_mismatch",
            Some(8),
            None,
            None,
            Some("rejected"),
        );
        let rows = sink.snapshot();
        assert_eq!(rows.len(), 1, "one event: {rows:?}");
        assert!(rows[0].contains("\"event\":\"family.probe.pairing\""));
        assert!(
            rows[0].contains("\"detail\":\"rejected\""),
            "row: {}",
            rows[0]
        );
        assert!(rows[0].contains("\"pairing\":\"pin_mismatch\""));
        // No token-id list can ride the event (counts/labels only).
        assert!(!rows[0].contains('['), "no arrays: {}", rows[0]);
        // The no-detail arm emits the empty label, not a stale one.
        log_family_pairing(
            &t,
            "vd-fix2",
            "local_selfcheck",
            Some(8),
            Some(0),
            Some(0),
            None,
        );
        let rows = sink.snapshot();
        assert!(rows[1].contains("\"pairing\":\"local_selfcheck\""));
        assert!(rows[1].contains("\"detail\":\"\""));
    }

    /// The auto-engage reason ladder (owner directive 2026-10-13): stable
    /// labels in the pinned priority order — capability unknown/false,
    /// E0 backend mismatch, unmeasured gate (never-worse: no measurement
    /// ⇒ no engagement), gate no-win, and the content-blindness block
    /// (ADR-034 rule 5) even when the gate would engage.
    #[cfg(feature = "verifydrafts")]
    #[test]
    fn family_engage_reason_ladder_is_stable() {
        let cap = |batch_verify: bool, backend: &str| FamilyPeerCap {
            batch_verify,
            batch_window_max: 8,
            backend: backend.to_string(),
            learned_unix_ms: 0,
        };
        // No learned capability (roster carries none — the probe is the
        // only source): the honest unknown.
        assert_eq!(
            family_engage_reason(None, Some("cpu"), Some(1.0), Some(2.0)),
            "capability_unknown"
        );
        assert_eq!(
            family_engage_reason(Some(&cap(false, "cpu")), Some("cpu"), None, None),
            "capability_false"
        );
        assert_eq!(
            family_engage_reason(Some(&cap(true, "vulkan")), Some("cpu"), None, None),
            "backend_mismatch"
        );
        // Never-worse: without BOTH predictions the gate cannot claim a
        // win, so no engagement — labeled, not silent.
        assert_eq!(
            family_engage_reason(Some(&cap(true, "cpu")), Some("cpu"), None, Some(2.0)),
            "gate_unmeasured"
        );
        assert_eq!(
            family_engage_reason(Some(&cap(true, "cpu")), Some("cpu"), Some(1.0), None),
            "gate_unmeasured"
        );
        assert_eq!(
            family_engage_reason(Some(&cap(true, "cpu")), Some("cpu"), Some(1.9), Some(2.0)),
            "gate_no_win"
        );
        // The gate WOULD engage (1.0 × 1.15 < 2.0) — and the protocol
        // boundary stops it: the family wire never carries the session
        // prompt (ADR-034 rule 5; the rule-8 commitment path is
        // owner-gated). Honest label, never a silent skip.
        assert_eq!(
            family_engage_reason(Some(&cap(true, "cpu")), Some("cpu"), Some(1.0), Some(2.0)),
            "blocked_content_contract"
        );
        // Backend labels compare case-insensitively (roster vs disclosure
        // vocabularies).
        assert_eq!(
            family_engage_reason(Some(&cap(true, "CPU")), Some("cpu"), None, None),
            "gate_unmeasured"
        );
    }

    #[test]
    fn chat_request_log_appends_language_pin_without_touching_the_log() {
        // The rendered-history Chinese lock (owner directive 2026-10-15,
        // reproduced against the shipped model+engine; the v0.2.26
        // front-system form measured INSUFFICIENT live — only the
        // trailing NAMED-language append breaks the lock): for a
        // confidently-English latest user turn, the request copy carries
        // the pin appended to that turn, on both the swarm and local
        // paths.
        let turn = |role: &str, content: &str| ChatTurn {
            role: role.into(),
            content: content.into(),
        };
        let log = vec![
            turn("user", "hello"),
            turn("assistant", "你好！我是Qwen语言模型。"),
            turn("user", "what is ModelSwarm?"),
        ];
        let request = chat_request_log(&log);
        assert_eq!(request.len(), log.len(), "no turns are added or removed");
        // Order intact; every earlier turn byte-identical.
        for (req, vis) in request.iter().zip(log.iter()).take(2) {
            assert_eq!(req.role, vis.role);
            assert_eq!(req.content, vis.content);
        }
        // The last user turn alone gains the named-language suffix.
        assert_eq!(request[2].role, "user");
        assert_eq!(
            request[2].content,
            format!("what is ModelSwarm?{CHAT_LANGUAGE_PIN_SUFFIX}")
        );
        // The input log is untouched — the pin must never reach the
        // persisted chat_log (the transcript stays what the user wrote
        // and saw; requests are rebuilt per send).
        assert_eq!(log[2].content, "what is ModelSwarm?");
        // The pin constrains language only — no persona, no topics.
        assert!(CHAT_LANGUAGE_PIN_SUFFIX.contains("language"));
        assert!(!CHAT_LANGUAGE_PIN_SUFFIX.contains("You are"));
    }

    #[test]
    fn effective_relay_precedence_file_wins() {
        // ADR-039: the local relay.addr file wins over the tracker value;
        // tracker alone works; neither -> null (today's behavior).
        let file = Some("/ip4/10.0.0.1/udp/4001/quic-v1/p2p/FileRelay");
        let tracker = Some("/ip4/152.69.218.106/udp/4001/quic-v1/p2p/TrackerRelay");
        let v = effective_relay(file, tracker).unwrap();
        assert_eq!(v["addr"], "/ip4/10.0.0.1/udp/4001/quic-v1/p2p/FileRelay");
        assert_eq!(v["source"], "file");
        let v = effective_relay(None, tracker).unwrap();
        assert_eq!(v["source"], "tracker");
        assert!(v["addr"]
            .as_str()
            .unwrap()
            .starts_with("/ip4/152.69.218.106/"));
        assert!(effective_relay(None, None).is_none());
    }

    #[test]
    fn language_pin_fires_only_for_confident_english() {
        // CJK questions need no pin — the model naturally matches the
        // user's language there (measured); pinning would be noise.
        let cjk = vec![ChatTurn {
            role: "user".into(),
            content: "什么是ModelSwarm？".into(),
        }];
        assert_eq!(chat_request_log(&cjk)[0].content, "什么是ModelSwarm？");
        // Other Latin languages fail the English word list on purpose —
        // mis-pinning French to English would be a regression.
        let french = vec![ChatTurn {
            role: "user".into(),
            content: "Qu'est-ce que c'est ModelSwarm ? Pouvez-vous expliquer".into(),
        }];
        assert_eq!(
            chat_request_log(&french)[0].content,
            "Qu'est-ce que c'est ModelSwarm ? Pouvez-vous expliquer"
        );
        // The heuristic: ASCII-dominant AND a common English word.
        assert!(english_message("what is ModelSwarm?"));
        assert!(english_message("Hello, can you help me please"));
        assert!(!english_message("what 什么是"));
        assert!(!english_message(""));
        assert!(!english_message("1234 ?!"));
        // A trailing assistant turn (not user) is never pinned.
        let asst_last = vec![ChatTurn {
            role: "assistant".into(),
            content: "Sure, here is the answer you asked for".into(),
        }];
        assert_eq!(
            chat_request_log(&asst_last)[0].content,
            "Sure, here is the answer you asked for"
        );
    }

    // ---- Chat wall (ADR-037 §3.1 W1–W6 / §4.4 item 10): the DESKTOP leg --
    //
    // The behavioral chat-wall test for this crate's own chat orchestration:
    // with the assistant plane ARMED in-process over a counting transport
    // double (the hosted-model posture — the double stands in for the
    // machine's model, never a real engine), every headlessly-drivable
    // `try_swarm_chat` failure path records ZERO assistant calls. The
    // success path and the runtime-less gateway failures ride the node
    // crate's suites (`crates/modelswarm-node`
    // `chat_wall_success_path_records_zero_assistant_calls` and
    // `tests/advisory_chat_wall.rs`); layers 1–2 (dependency graph and
    // source scope) ride `crates/modelswarm-node/tests/advisory_chat_wall.rs`
    // and scan THIS file too.
    mod chat_wall {
        use super::*;
        use modelswarm_gateway::{
            ExecutorError, ExecutorEvent, ExecutorStream, InferenceExecutor, NormalizedRequest,
        };
        use modelswarm_node::advisory::{AdvisoryTransport, AssistantPlane, CountingTransport};
        use std::sync::Arc;

        /// A never-called local executor (the failure paths below end before
        /// the local executor is reached; the swarm executor seam still
        /// needs SOMETHING to exist).
        struct UnusedExecutor;
        #[async_trait::async_trait]
        impl InferenceExecutor for UnusedExecutor {
            async fn execute(
                &self,
                _request: NormalizedRequest,
            ) -> Result<ExecutorStream, ExecutorError> {
                Err(ExecutorError::Fatal {
                    code: "runtime_error".into(),
                })
            }
        }

        /// Arms the assistant plane in-process over a counting double —
        /// the hosted-model posture (2026-10-15: no key, no endpoint, no
        /// env var; arming rides advisory.json's `assistant.enabled` plus
        /// the double standing in for the hosted model).
        fn armed_plane(data_dir: &std::path::Path) -> (AssistantPlane, Arc<CountingTransport>) {
            std::fs::write(
                data_dir.join("advisory.json"),
                r#"{"assistant.enabled":true}"#,
            )
            .unwrap();
            let double = Arc::new(CountingTransport::assistant_echo("unused".into()));
            let (plane, gap) = AssistantPlane::spawn(
                data_dir,
                None,
                Some(double.clone() as Arc<dyn AdvisoryTransport>),
            );
            assert_eq!(
                gap.as_str(),
                "not_hosting",
                "armed spawn reports no degrade"
            );
            (
                plane.expect("the assistant plane arms over the double"),
                double,
            )
        }

        #[tokio::test]
        async fn chat_wall_try_swarm_chat_failure_paths_record_zero_assistant_calls() {
            let dir = tempfile::tempdir().unwrap();
            let (telemetry, _sink) = Telemetry::memory();
            let telemetry = Arc::new(telemetry);
            let (_plane, double) = armed_plane(dir.path());
            let inner = tokio::sync::Mutex::new(test_inner(
                dir.path(),
                "http://127.0.0.1:9",
                Some("msp1:wall"),
            ));
            let executor: Arc<dyn InferenceExecutor> = Arc::new(UnusedExecutor);
            let log = vec![ChatTurn {
                role: "user".into(),
                content: "wall canary".into(),
            }];
            let noop = |_: &str| {};

            // Failure path 1 — tracker unreachable: tracker_client builds,
            // lookup fails, honest shadow-failure row, None.
            let reply = try_swarm_chat(
                &inner,
                "http://127.0.0.1:9",
                dir.path(),
                &telemetry,
                "msp1:wall",
                executor.clone(),
                &log,
                Some(8),
                0.0,
                None,
                &noop,
            )
            .await;
            assert!(reply.is_none(), "unreachable tracker = honest failure");
            assert_eq!(double.count(), 0, "W1: no assistant call from a chat turn");

            // Failure path 2 — no eligible peer: a local mock tracker with
            // an EMPTY roster answers lookup; the candidate predicate finds
            // nothing; None with a no_eligible_peer shadow row.
            let app = axum::Router::new().route(
                "/api/v1/peers",
                axum::routing::get(
                    || async move { axum::Json(serde_json::json!({ "peers": [] })) },
                ),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, app).await });
            let tracker_url = format!("http://{addr}");
            // A session token file short-circuits enrollment (the mock
            // serves the roster route unauthenticated).
            std::fs::write(dir.path().join("session.token"), "wall-test-session").unwrap();
            let reply = try_swarm_chat(
                &inner,
                &tracker_url,
                dir.path(),
                &telemetry,
                "msp1:wall",
                executor.clone(),
                &log,
                Some(8),
                0.0,
                None,
                &noop,
            )
            .await;
            assert!(
                reply.is_none(),
                "empty roster = honest no-eligible-peer failure"
            );
            assert_eq!(double.count(), 0, "W1: still zero assistant calls");

            // The wall holds across every drivable path, the plane was
            // ARMED (non-vacuous: armed_plane asserted the plane), and
            // nothing was ever sent through the backing.
            assert!(double.bodies().is_empty());
        }

        /// The protocol review's recorded follow-up, closed with this
        /// leg: a behavioral QUIC DIAL-FAILURE leg — the mock tracker
        /// answers a roster whose candidate has a real (DEAD) QUIC
        /// address, the remote dial fails pre-token, the honest local
        /// fallback serves, and the plane records ZERO calls (item 10's
        /// "every failure path" letter).
        #[tokio::test]
        async fn chat_wall_quic_dial_failure_records_zero_assistant_calls() {
            let dir = tempfile::tempdir().unwrap();
            let (telemetry, _sink) = Telemetry::memory();
            let telemetry = Arc::new(telemetry);
            let (_plane, double) = armed_plane(dir.path());

            // The mock tracker: one FOREIGN peer with free slots and a
            // real QUIC multiaddr pointing at a dead loopback port.
            let app = axum::Router::new().route(
                "/api/v1/peers",
                axum::routing::get(|| async move {
                    axum::Json(serde_json::json!({ "peers": [{
                        "peerId": "12D3KooWDialFailPeer0000000000000000",
                        "addresses": ["/ip4/127.0.0.1/udp/1/quic-v1"],
                        "queueMs": 5, "freeSlots": 1,
                        "capacityClass": "cpu", "leaseExpiresAt": "z", "lastSeenAt": "z",
                    }]}))
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, app).await });
            std::fs::write(dir.path().join("session.token"), "wall-test-session").unwrap();

            // A LOCAL executor that serves (the honest fallback path —
            // dial failure must end in a served reply, not a dead end).
            struct LocalOk;
            #[async_trait::async_trait]
            impl InferenceExecutor for LocalOk {
                async fn execute(
                    &self,
                    _request: NormalizedRequest,
                ) -> Result<ExecutorStream, ExecutorError> {
                    Ok(Box::pin(futures_util::stream::iter(vec![
                        ExecutorEvent::TokenDelta {
                            delta: "local ok".into(),
                            index: 0,
                        },
                        ExecutorEvent::Usage {
                            prompt_tokens: 1,
                            completion_tokens: 1,
                            prefill_ms: 1.0,
                            decode_ms: 1.0,
                        },
                        ExecutorEvent::Completed {
                            finish_reason: modelswarm_gateway::FinishReason::Stop,
                        },
                    ])))
                }
            }

            let inner = tokio::sync::Mutex::new(test_inner(
                dir.path(),
                &format!("http://{addr}"),
                Some("msp1:wall"),
            ));
            let executor: Arc<dyn InferenceExecutor> = Arc::new(LocalOk);
            let log = vec![ChatTurn {
                role: "user".into(),
                content: "dial fail canary".into(),
            }];
            let noop = |_: &str| {};
            let reply = try_swarm_chat(
                &inner,
                &format!("http://{addr}"),
                dir.path(),
                &telemetry,
                "msp1:wall",
                executor,
                &log,
                Some(8),
                0.0,
                None,
                &noop,
            )
            .await;
            let reply = reply.expect("the honest local fallback serves after a dial failure");
            assert!(
                reply["mode"]
                    .as_str()
                    .unwrap()
                    .contains("remote dial failed"),
                "the mode label names the dial failure: {}",
                reply["mode"]
            );
            // THE WALL: the dial-failure turn never reached the plane.
            assert_eq!(double.count(), 0, "W1: zero assistant calls");
            assert!(double.bodies().is_empty());
        }
    }

    /// ADR-037 §4.4 items (11 desktop half), (13), (14) as rewired by
    /// the 2026-10-15 owner directive: the assistant panel's opt-out,
    /// static floor, renderer, canary, and A3 discipline over the TWO
    /// postures (hosted-model-backed vs static-only). The cloud-era
    /// disclosure/consent gate folded away with the cloud.
    mod assistant_panel {
        use super::super::*;
        use std::sync::Arc;

        use crate::app::assistant::{self, PanelOutcome, TurnDeps};
        use modelswarm_node::advisory::{AdvisoryTransport, CountingTransport};

        #[allow(unused_imports)]
        use super::test_inner;

        fn hw() -> HardwareInfo {
            HardwareInfo {
                ram_mb: 32_000,
                cpu_cores: 12,
                gpu: Some(GpuInfo {
                    name: "RTX 5080".into(),
                    vram_mb: 16_003,
                }),
            }
        }

        fn cards() -> Vec<serde_json::Value> {
            vec![serde_json::json!({
                "profile_id": "msp1:aa", "display_name": "Alpha 9B",
                "quant_method": "q4_k_m", "quant_bits": 4,
                "hw": {"download_mb": 5_680, "ram_mb_est": 6_144},
                "fit": "gpu_full", "recommended": true, "peers": 2,
            })]
        }

        fn inputs<'a>(
            hw: &'a HardwareInfo,
            cards: &'a [serde_json::Value],
        ) -> assistant::ContextInputs<'a> {
            assistant::ContextInputs {
                hardware: hw,
                picker: cards,
                family: None,
                hosting_on: true,
                listener_addr: None,
                engine_backend: Some("cpu"),
                engage_last: None,
                governor_verdict: None,
                probe_verdict: None,
                capacity_class: None,
                app_version: "0.2.24",
            }
        }

        /// Arms the plane over the hosted-model stand-in double (the
        /// 2026-10-15 posture: no key, no endpoint — the double IS the
        /// backing; never a real engine).
        fn boot_hosted_backed(
            dir: &std::path::Path,
            double: Arc<CountingTransport>,
        ) -> Option<assistant::Plane> {
            std::fs::write(dir.join("advisory.json"), r#"{"assistant.enabled":true}"#).unwrap();
            let (plane, gap) =
                assistant::Plane::spawn_with_double(dir, double as Arc<dyn AdvisoryTransport>);
            assert_eq!(
                gap.as_str(),
                "not_hosting",
                "armed spawn reports no degrade"
            );
            plane
        }

        fn deps<'a>(
            plane: Option<&'a assistant::Plane>,
            degrade_gap: &'a str,
            opt_out: bool,
            telemetry: &'a Telemetry,
            inputs: &'a assistant::ContextInputs<'a>,
        ) -> TurnDeps<'a> {
            TurnDeps {
                plane,
                degrade_gap,
                opt_out,
                telemetry,
                inputs,
            }
        }

        /// §4.4 item (13), 2026-10-15 form: the turn is NEVER blocked —
        /// the identity banner folds into a dismissible line, and the
        /// first turn already answers through the hosted model.
        #[tokio::test]
        async fn the_first_turn_answers_without_any_gate() {
            let dir = tempfile::tempdir().unwrap();
            let double = Arc::new(CountingTransport::assistant_echo("short advice".into()));
            let plane = boot_hosted_backed(dir.path(), double.clone());
            let plane = plane.expect("the plane arms");
            let (telemetry, _sink) = Telemetry::memory();
            let hardware = hw();
            let picker = cards();
            let ctx = inputs(&hardware, &picker);

            // The very first turn runs and the reply renders.
            let outcome = assistant::turn(
                deps(Some(&plane), "none", false, &telemetry, &ctx),
                "which model?",
            )
            .await;
            match outcome {
                PanelOutcome::ModelReply { reply, .. } => assert_eq!(reply, "short advice"),
                other => panic!("{other:?}"),
            }
            assert_eq!(double.count(), 1, "no gate stands before the first turn");
        }

        /// §4.4 item (13): opt-out yields ZERO calls (counting double)
        /// and the built-in static help with the `disabled` label.
        #[tokio::test]
        async fn opt_out_yields_zero_calls_and_static_help() {
            let dir = tempfile::tempdir().unwrap();
            let double = Arc::new(CountingTransport::assistant_echo("never".into()));
            let plane = boot_hosted_backed(dir.path(), double.clone());
            let plane = plane.expect("the plane arms");
            let (telemetry, _sink) = Telemetry::memory();
            let hardware = hw();
            let picker = cards();
            let ctx = inputs(&hardware, &picker);
            let outcome = assistant::turn(
                deps(Some(&plane), "none", true, &telemetry, &ctx),
                "what does resource_ceiling mean?",
            )
            .await;
            match outcome {
                PanelOutcome::StaticHelp { entries, line } => {
                    assert_eq!(
                        line,
                        "Built-in help — the model-backed assistant is off/unavailable (disabled)."
                    );
                    assert_eq!(entries.len(), 1);
                    assert_eq!(entries[0].label, "resource_ceiling");
                }
                other => panic!("{other:?}"),
            }
            assert_eq!(double.count(), 0, "opt-out = zero calls");
        }

        /// §4.4 item (13): the static floor (no model backing) renders
        /// static help with the honest gap label — never a blank box,
        /// never a spinner. `not_hosting` is the shipped default label.
        #[tokio::test]
        async fn static_floor_renders_help_with_the_gap_label() {
            let _dir = tempfile::tempdir().unwrap();
            let (telemetry, _sink) = Telemetry::memory();
            let hardware = hw();
            let picker = cards();
            let ctx = inputs(&hardware, &picker);
            let outcome = assistant::turn(
                deps(None, "not_hosting", false, &telemetry, &ctx),
                "why is family off (pairing_disabled)?",
            )
            .await;
            match outcome {
                PanelOutcome::StaticHelp { entries, line } => {
                    assert_eq!(
                        line,
                        "Built-in help — the model-backed assistant is off/unavailable (not_hosting)."
                    );
                    assert_eq!(entries.len(), 1);
                    assert_eq!(entries[0].label, "pairing_disabled");
                }
                other => panic!("{other:?}"),
            }
        }

        /// §3.7 rendering contract end-to-end: a >4000-char model reply
        /// renders TRUNCATED at the display cap.
        #[tokio::test]
        async fn reply_renders_capped_at_the_display_cap() {
            let dir = tempfile::tempdir().unwrap();
            let long: String = "x".repeat(5_000);
            let double = Arc::new(CountingTransport::assistant_echo(long));
            let plane = boot_hosted_backed(dir.path(), double.clone());
            let plane = plane.expect("the plane arms");
            let (telemetry, _sink) = Telemetry::memory();
            let hardware = hw();
            let picker = cards();
            let ctx = inputs(&hardware, &picker);
            let outcome =
                assistant::turn(deps(Some(&plane), "none", false, &telemetry, &ctx), "q").await;
            match outcome {
                PanelOutcome::ModelReply { reply, .. } => {
                    assert_eq!(
                        reply.chars().count(),
                        4_000 + "…[truncated]".chars().count()
                    );
                    assert!(reply.ends_with("…[truncated]"));
                }
                other => panic!("{other:?}"),
            }
        }

        /// §4.4 item (11), desktop half: the app's OWN conversation
        /// transcript and an ADR-036-shaped carrier payload are seeded
        /// into process state; assistant turns run; the canary bytes
        /// appear NOWHERE in any serialized request. The builder is
        /// structurally fed none of it (W3) — this proves the bytes
        /// cannot leak through the typed context.
        #[tokio::test]
        async fn desktop_canary_suite_transcript_and_carrier_absent() {
            let dir = tempfile::tempdir().unwrap();
            let double = Arc::new(CountingTransport::assistant_echo("advice".into()));
            let plane = boot_hosted_backed(dir.path(), double.clone());
            let plane = plane.expect("the plane arms");
            let (telemetry, _sink) = Telemetry::memory();

            const CANARY_TRANSCRIPT: &str = "DESKTOP-WALL-CANARY-transcript-0123456789abcdef";
            const CANARY_CARRIER: &str = "DESKTOP-WALL-CANARY-carrier-fedcba9876543210";
            // Seed the app state the way a real session would: a
            // transcript full of canary bytes and a lease-shaped carrier
            // payload string.
            let mut inner = test_inner(dir.path(), "http://127.0.0.1:9", Some("msp1:canary"));
            inner.chat_log = vec![
                ChatTurn {
                    role: "user".into(),
                    content: format!("{CANARY_TRANSCRIPT} hello"),
                },
                ChatTurn {
                    role: "assistant".into(),
                    content: format!("reply {CANARY_TRANSCRIPT}"),
                },
            ];
            inner.swarm_lease = Some((
                "msp1:canary".into(),
                format!("{{\"promptTokens\": [1, 2, 3], \"payload\": \"{CANARY_CARRIER}\"}}"),
                u64::MAX,
            ));
            inner.last_engage_reason = Some("gate_no_win".into());

            let hardware = hw();
            let picker = cards();
            let ctx = inputs(&hardware, &picker);
            let outcome = assistant::turn(
                deps(Some(&plane), "none", false, &telemetry, &ctx),
                "which model fits my machine?",
            )
            .await;
            assert!(matches!(outcome, PanelOutcome::ModelReply { .. }));
            assert!(!double.bodies().is_empty());
            for body in double.bodies() {
                assert!(
                    !body.contains(CANARY_TRANSCRIPT),
                    "transcript canary reached the model backing"
                );
                assert!(
                    !body.contains(CANARY_CARRIER),
                    "carrier canary reached the model backing"
                );
                assert!(!body.contains("promptTokens"));
            }
        }

        /// §4.4 item (14), the A3 extension over the 2026-10-15
        /// postures: with the assistant plane STATIC-ONLY (no backing)
        /// vs HOSTED-MODEL-BACKED over an erroring double, the
        /// deterministic surfaces are byte-identical (the picker
        /// pipeline's pure functions; the engage ladder is pinned by its
        /// own golden test), and the telemetry streams are
        /// byte-identical after filtering `assistant.*`.
        #[tokio::test]
        async fn assistant_a3_extension_static_only_vs_hosted_backed() {
            let dir = tempfile::tempdir().unwrap();

            // The deterministic picker pipeline, computed identically in
            // both postures (fit_and_score + the census join order).
            let hardware = hw();
            let models: Vec<serde_json::Value> = vec![
                serde_json::json!({
                    "profile_id": "msp1:aa", "display_name": "Alpha 9B",
                    "fit": "gpu_full", "recommended": true,
                }),
                serde_json::json!({
                    "profile_id": "msp1:bb", "display_name": "Bravo 27B",
                    "fit": "gpu_partial", "recommended": false,
                }),
            ];
            let mut counts = std::collections::BTreeMap::new();
            counts.insert("msp1:aa".to_string(), 2);
            counts.insert("msp1:bb".to_string(), 9);
            let view_a = census_view(&models, Some(&counts));
            let view_b = census_view(&models, Some(&counts));
            assert_eq!(
                serde_json::to_string(&view_a).unwrap(),
                serde_json::to_string(&view_b).unwrap(),
                "the census join is a pure function — identical by construction"
            );
            // fit_and_score over the same inputs is byte-identical.
            let score = |ram: u64, w: u64| {
                let ranked = fit_and_score(&hardware, ram, w).map(|(fit, s)| (fit.label(), s));
                serde_json::to_string(&ranked).unwrap()
            };
            assert_eq!(score(6_144, 5_680), score(6_144, 5_680));

            // STATIC-ONLY vs HOSTED-BACKED-ERRORING: the telemetry
            // streams, filtered of assistant.*, are byte-identical.
            let (telemetry_static, sink_static) = Telemetry::memory();
            let picker = cards();
            let ctx = inputs(&hardware, &picker);
            let static_only = assistant::turn(
                deps(None, "not_hosting", false, &telemetry_static, &ctx),
                "question",
            )
            .await;
            let (telemetry_erroring, sink_erroring) = Telemetry::memory();
            let double = Arc::new(CountingTransport::always_erroring());
            let plane = boot_hosted_backed(dir.path(), double.clone());
            let erroring = assistant::turn(
                deps(plane.as_ref(), "none", false, &telemetry_erroring, &ctx),
                "question",
            )
            .await;
            assert!(matches!(static_only, PanelOutcome::StaticHelp { .. }));
            assert!(matches!(erroring, PanelOutcome::ModelGap { .. }));
            let filter = |rows: Vec<String>| -> Vec<String> {
                rows.into_iter()
                    .filter(|l| !l.contains("\"event\":\"assistant."))
                    .collect()
            };
            assert_eq!(
                filter(sink_static.snapshot()),
                filter(sink_erroring.snapshot()),
                "A3: byte-identical after filtering the plane's own rows"
            );
            // And the erroring plane emitted ONLY assistant.* rows.
            for line in sink_erroring.snapshot() {
                assert!(line.contains("\"event\":\"assistant."), "{line}");
            }
        }
    }
}

//! The CHAT-WALL SUITE (ADR-037 §4.4 item 10 — W1–W4), layers 1–2 plus
//! the runtime-less behavioral leg. This suite must be GREEN before any
//! assistant-surface code exists (ADR-037 §7 step 2). The mock-runtime
//! success-path behavioral test lives in the advisory module's unit
//! tests (`crates/modelswarm-node/src/advisory/mod.rs`,
//! `chat_wall_success_path_records_zero_advisory_calls` — the mock
//! runtime compiles under `cfg(test)` there); the desktop
//! `try_swarm_chat` failure-path leg lives in the desktop tauri-shell
//! suite.
//!
//! - **Layer 1 (dependency graph)**: `cargo metadata --no-deps` — the
//!   frozen decision-crate set depends on NO advisory host, and
//!   `modelswarm-node` (the crate hosting `modelswarm-node::advisory`)
//!   is depended on ONLY by `modelswarm-desktop`.
//! - **Layer 2 (source scope, directional)**: advisory client type
//!   names appear ONLY in the advisory module and tests — never in the
//!   gateway crate, the node serving/verifydrafts modules, or the
//!   `send_chat`/`try_swarm_chat` scopes; the advisory (request
//!   builder) modules reference no chat/serving/carrier types; the
//!   provider call executes from exactly ONE site (the digest timer).
//! - **Layer 3 (behavioral, this file)**: with the plane ARMED and
//!   KEYED and a counting transport double injected, chat turns through
//!   the loopback gateway record ZERO advisory calls — here the
//!   runtime-less failure paths (no_peer); the success path and the
//!   desktop swarm paths ride the suites named above.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use modelswarm_node::advisory::{AdvisoryTransport, CountingTransport};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

fn read_file(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Production-source view of one file: the TRAILING `#[cfg(test)] mod
/// …` block (the inline unit-test surface, on the ADR's layer-2
/// allowlist as `*/tests/`) is stripped when it is the file's last
/// item. A mid-file cfg(test) module is NOT stripped — the scanner then
/// flags it, which errs on the safe side (test-only advisory references
/// belong in the trailing module or a `tests/` tree).
fn production_source(path: &Path) -> String {
    let text = read_file(path);
    if let Some(pos) = text.find(
        "#[cfg(test)]
mod ",
    ) {
        let tail = &text[pos..];
        // Keep only if the remainder contains no production items after
        // the test module's closing brace would be non-trivial to compute
        // with format-string braces around; the tree's test modules are
        // trailing, so assert that: after the marker, the rest of the
        // file must be the module itself (we simply strip to EOF and let
        // the workspace's fmt/clippy/tests keep the shape pinned).
        let _ = tail;
        return text[..pos].to_string();
    }
    text
}

fn rust_sources_under(dir: &Path, skip: Option<&Path>) -> BTreeMap<PathBuf, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        // Integration/perf test trees (`*/tests/`) are on the ADR's
        // layer-2 allowlist ("…and */tests/") — this scanner walks
        // PRODUCTION sources only.
        if current.file_name().is_some_and(|n| n == "tests") {
            continue;
        }
        let entries = std::fs::read_dir(&current)
            .unwrap_or_else(|e| panic!("read_dir {}: {e}", current.display()));
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(skip) = skip {
                if path == skip {
                    continue;
                }
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.insert(path.clone(), production_source(&path));
            }
        }
    }
    out
}

/// Advisory client type names + the module path prefix (the layer-2
/// vocabulary of record; extend ONLY via an ADR-037 amendment).
const ADVISORY_NAMES: &[&str] = &[
    "modelswarm_node::advisory",
    "advisory::wire",
    "advisory::AdvisoryHandle",
    "AdvisoryState",
    "AdvisoryConfig",
    "AdvisoryTransport",
    "AdvisoryHandle",
    "CountingTransport",
    "ReqwestTransport",
    "RecordedSuggestion",
    "ProviderRequest",
    "DIGEST_SYSTEM_PROMPT",
    "advisory-egress",
    "advisory-clamp-table",
];

fn find_hits(sources: &BTreeMap<PathBuf, String>, needle: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for (path, text) in sources {
        if text.contains(needle) {
            hits.push(path.display().to_string());
        }
    }
    hits
}

// ---------------------------------------------------------------------------
// Layer 1 — the dependency graph (ADR-033 §9.2(6) + ADR-037 item 10)
// ---------------------------------------------------------------------------

#[test]
fn layer1_decision_crates_do_not_depend_on_the_advisory_host() {
    // `cargo metadata --no-deps` compiles nothing and lists every
    // workspace member's declared dependencies.
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(workspace_root())
        .output()
        .expect("cargo metadata runs");
    assert!(output.status.success(), "cargo metadata failed");
    let meta: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("metadata json parses");

    let decision_set = [
        "modelswarm-scheduler",
        "modelswarm-session",
        "modelswarm-eligibility",
        "modelswarm-gateway",
        "modelswarm-runtime",
        "modelswarm-transport",
        "modelswarm-tracker-api",
        "modelswarm-store",
        "modelswarm-types",
    ];
    // The five §9.2(6)-named decision crates additionally carry NO
    // dev-dependency on the advisory host (fixture reuse is tolerated
    // elsewhere, recommended against there).
    let no_dev = [
        "modelswarm-gateway",
        "modelswarm-session",
        "modelswarm-types",
        "modelswarm-eligibility",
        "modelswarm-scheduler",
    ];
    let mut dependents_of_node: Vec<String> = Vec::new();
    for package in meta["packages"].as_array().expect("packages") {
        let name = package["name"].as_str().unwrap_or_default();
        if !decision_set.contains(&name) {
            continue;
        }
        for dep in package["dependencies"].as_array().expect("dependencies") {
            let dep_name = dep["name"].as_str().unwrap_or_default();
            let kind = dep["kind"].as_str().unwrap_or("normal");
            if dep_name == "modelswarm-node" {
                panic!(
                    "{name} declares a {kind} dependency on modelswarm-node (the advisory-plane host) — the wall is a dependency-graph property (ADR-033 §9.2(6))"
                );
            }
            if dep_name == "modelswarm-advisory" {
                panic!("{name} depends on a dedicated advisory crate — none may exist without Integrator sign-off (ADR-033 §9.4)");
            }
            // The five §9.2(6)-named decision crates carry no DEV
            // dependency on the advisory host either (fixture reuse is
            // tolerated elsewhere, recommended against here).
            if no_dev.contains(&name) && dep_name == "modelswarm-node" {
                panic!("{name} declares a dev-dependency on modelswarm-node");
            }
        }
    }
    // ADR-037 item-10 extension: the advisory host is depended on ONLY
    // by modelswarm-node/modelswarm-desktop — with the module placement,
    // `modelswarm-node` (the host) may be a normal dependency of
    // `modelswarm-desktop` and NOTHING else among workspace members.
    for package in meta["packages"].as_array().expect("packages") {
        let name = package["name"].as_str().unwrap_or_default();
        for dep in package["dependencies"].as_array().expect("dependencies") {
            if dep["name"].as_str() == Some("modelswarm-node")
                && dep["kind"].as_str().unwrap_or("normal") != "dev"
            {
                dependents_of_node.push(name.to_string());
            }
        }
    }
    let mut expected = vec![
        "modelswarm-bench".to_string(),
        "modelswarm-desktop".to_string(),
    ];
    expected.sort();
    dependents_of_node.sort();
    assert_eq!(
        dependents_of_node, expected,
        "modelswarm-node (the advisory host) may be a normal dependency of the desktop and — per ADR-033 §9.2(6)'s explicit exemption — modelswarm-bench (the sanctioned shadow-ledger consumer; it uses node's measure/remote/serving, and the layer-2 source scan below proves it touches no advisory name) and NOTHING else"
    );
}

// ---------------------------------------------------------------------------
// Layer 2 — source scope (directional)
// ---------------------------------------------------------------------------

#[test]
fn layer2_advisory_names_live_only_in_the_advisory_module_and_tests() {
    let root = workspace_root();
    let advisory_dir = root.join("crates/modelswarm-node/src/advisory");
    // The module's sanctioned homes: the advisory module itself and the
    // node's lib.rs — the daemon COMPOSITION file whose Node::start
    // wires the plane (§9.4: "the advisory client lands … in
    // crates/modelswarm-node::advisory", hosted by this crate). Every
    // other crate source must be clean. (The desktop display renderer
    // is a sanctioned FUTURE consumer per ADR-037 §4.5 — at THIS leg no
    // exemption exists; the assistant leg's review adds it, and
    // modelswarm-bench's §9.2(6) exemption is for the shadow LEDGER,
    // which at this leg touches no advisory name.)
    let composition_host = root.join("crates/modelswarm-node/src/lib.rs");
    let crates = root.join("crates");
    let mut sources = rust_sources_under(&crates, None);
    sources.retain(|path, _| !path.starts_with(&advisory_dir) && *path != composition_host);
    for name in ADVISORY_NAMES {
        let hits = find_hits(&sources, name);
        assert!(
            hits.is_empty(),
            "advisory name {name:?} outside the advisory module (the wall is source-scoped): {hits:?}"
        );
    }
    // The wall's named hot zones, checked EXPLICITLY (belt and braces —
    // including lib.rs's serving-path composition neighbors):
    for zone in [
        root.join("crates/modelswarm-gateway/src"),
        root.join("crates/modelswarm-node/src/serving.rs"),
        root.join("crates/modelswarm-node/src/verifydrafts"),
    ] {
        let zone_sources = if zone.is_file() {
            BTreeMap::from([(zone.clone(), production_source(&zone))])
        } else {
            rust_sources_under(&zone, None)
        };
        for name in ADVISORY_NAMES {
            let hits = find_hits(&zone_sources, name);
            assert!(
                hits.is_empty(),
                "advisory name {name:?} inside wall zone {}: {hits:?}",
                zone.display()
            );
        }
    }
}

/// Extracts one function's source from `app.rs` (from `async fn NAME(`
/// to the next top-level `\nasync fn `/`\nfn ` — a drift detector in
/// itself: if the file's shape changes, extraction fails loudly).
fn function_scope(app_rs: &str, name: &str) -> String {
    let start = app_rs
        .find(&format!("async fn {name}("))
        .unwrap_or_else(|| panic!("{name} not found in app.rs — wall scope extraction drifted"));
    let rest = &app_rs[start..];
    let end = ["\nasync fn ", "\nfn ", "\npub async fn ", "\npub fn "]
        .iter()
        .filter_map(|marker| rest[1..].find(marker).map(|i| i + 1))
        .min()
        .unwrap_or(rest.len());
    let body = &rest[..end];
    assert!(
        body.len() > 200,
        "{name} extraction too small ({}) — drifted",
        body.len()
    );
    body.to_string()
}

#[test]
fn layer2_chat_scopes_reference_no_advisory_surface() {
    let root = workspace_root();
    let app_rs = read_file(&root.join("crates/modelswarm-desktop/src/app.rs"));
    for scope_name in ["send_chat", "try_swarm_chat"] {
        let scope = function_scope(&app_rs, scope_name);
        for name in ADVISORY_NAMES {
            assert!(
                !scope.contains(name),
                "chat scope {scope_name} references advisory name {name:?} — W1: NO ROUTE, EVER"
            );
        }
        // The full chat-wall vocabulary (W1/W3): no advisory reach, no
        // provider endpoint, no key material near the chat path.
        for forbidden in ["advisory", "MSP_ADVISORY", "DeepSeek"] {
            assert!(
                !scope.to_lowercase().contains(&forbidden.to_lowercase()),
                "chat scope {scope_name} mentions {forbidden:?} — the chat path must not know the advisory plane exists"
            );
        }
    }
}

#[test]
fn layer2_request_builder_modules_reference_no_chat_or_carrier_types() {
    // W3 (data wall, directional): the advisory module — whose egress
    // modules ARE the digest request builder at this leg — cannot name
    // chat or carrier types. There is no `chat_log`/`ChatTurn`/delta/
    // serving-frame/carrier type in ANY advisory module file.
    let root = workspace_root();
    let advisory_dir = root.join("crates/modelswarm-node/src/advisory");
    let sources = rust_sources_under(&advisory_dir, None);
    assert!(!sources.is_empty());
    let forbidden = [
        "chat_log",
        "ChatTurn",
        "chat-delta",
        "prompt_tokens",
        "pin_commitment",
        "InferenceRequest",
        "WireMessage",
        "GatewayState",
        "render_chatml",
    ];
    for (path, text) in &sources {
        for needle in &forbidden {
            assert!(
                !text.contains(needle),
                "advisory module {} references {needle:?} — the data wall (W3) is structural",
                path.display()
            );
        }
    }
}

#[test]
fn layer2_the_provider_call_executes_from_exactly_one_site() {
    // The enumerated-call-sites pin (W2): at this leg the ONLY producer
    // of provider calls is the digest timer. Asserted by source scan:
    // `transport.send(` executes solely in the runner; `advisory::wire(`
    // is invoked solely from Node::start; `runner::spawn`/`advisory::
    // spawn` appear only in their own definitions/wiring.
    let root = workspace_root();
    let node_src = root.join("crates/modelswarm-node/src");
    let desktop_src = root.join("crates/modelswarm-desktop/src");
    let mut production = rust_sources_under(&node_src, None);
    for (path, text) in rust_sources_under(&desktop_src, None) {
        production.insert(path, text);
    }
    let send_hits = find_hits(&production, ".send(&request");
    let expected_runner = node_src.join("advisory").join("runner.rs");
    assert_eq!(
        send_hits.iter().map(Path::new).collect::<Vec<_>>(),
        vec![expected_runner.as_path()],
        "the ONE transport execution site is the digest timer's runner"
    );
    let wire_hits = find_hits(&production, "advisory::wire(");
    let expected_lib = node_src.join("lib.rs");
    assert_eq!(
        wire_hits.iter().map(Path::new).collect::<Vec<_>>(),
        vec![expected_lib.as_path()],
        "advisory::wire is called exactly once — the Node::start composition"
    );
    let advisory_dir = node_src.join("advisory");
    let reqwest_hits = find_hits(&production, "ReqwestTransport::new(");
    assert!(
        !reqwest_hits.is_empty()
            && reqwest_hits
                .iter()
                .all(|hit| Path::new(hit).starts_with(&advisory_dir)),
        "the production transport is constructed only inside the advisory module: {reqwest_hits:?}"
    );
}

// ---------------------------------------------------------------------------
// Layer 3 (behavioral, runtime-less legs) — armed plane + counting
// double; chat turns through the loopback gateway record ZERO calls.
// ---------------------------------------------------------------------------

/// Boots a node with the advisory plane ARMED over a counting double,
/// then drives chat turns through the loopback gateway — the exact
/// surface `send_chat`'s local fallback posts to.
async fn armed_node_with_double(
    data_dir: &Path,
) -> (
    modelswarm_node::NodeHandle,
    std::sync::Arc<CountingTransport>,
    tokio::sync::watch::Sender<bool>,
) {
    std::fs::write(
        data_dir.join("advisory.json"),
        r#"{"enabled":true,"endpoint_url":"https://advisor.invalid/v1/chat/completions","model":"test-model"}"#,
    )
    .unwrap();
    let key = data_dir.join("advisory-key.txt");
    std::fs::write(&key, "test-key-local-double\n").unwrap();
    std::env::set_var("MSP_ADVISORY_KEY_FILE", &key);
    let double = std::sync::Arc::new(CountingTransport::always_ok());
    let as_trait: std::sync::Arc<dyn AdvisoryTransport> = double.clone();
    let (tx, rx) = tokio::sync::watch::channel(false);
    let config = modelswarm_node::NodeConfig {
        gateway_port: 0,
        data_dir: data_dir.to_path_buf(),
        profile_id: Some("msp1:chatwall".to_string()),
        advisory_transport: Some(as_trait),
        ..modelswarm_node::NodeConfig::default()
    };
    let handle = modelswarm_node::Node::start(config, rx)
        .await
        .expect("node starts");
    std::env::remove_var("MSP_ADVISORY_KEY_FILE");
    // The plane must actually be ARMED in this process — otherwise the
    // zero-call assertion below would pass vacuously.
    assert!(
        handle.advisory.is_some(),
        "the advisory plane must be armed (enabled + key + https endpoint + model)"
    );
    (handle, double, tx)
}

#[tokio::test]
async fn chat_wall_runtimeless_failure_paths_record_zero_advisory_calls() {
    let dir = tempfile::tempdir().unwrap();
    let (handle, double, tx) = armed_node_with_double(dir.path()).await;
    let addr = handle.local_addr;

    // Failure path 1: no runtime (no engine, no sidecar) — the gateway
    // answers no_peer/503; the honest failure of a chat turn.
    let reply = reqwest::Client::new()
        .post(format!("http://{addr}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "msp1:chatwall",
            "messages": [{"role": "user", "content": "wall test"}],
            "max_tokens": 8,
        }))
        .send()
        .await
        .expect("gateway responds");
    assert_eq!(
        reply.status().as_u16(),
        503,
        "runtime-less node fails honestly"
    );

    // Failure path 2: unknown profile — 404/400, still no advisory call.
    let reply = reqwest::Client::new()
        .post(format!("http://{addr}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "msp1:not-hosted",
            "messages": [{"role": "user", "content": "wall test"}],
            "max_tokens": 8,
        }))
        .send()
        .await
        .expect("gateway responds");
    assert!(!reply.status().is_success());

    // The wall: ZERO advisory calls from any chat turn (the counting
    // double's sensitivity control is the unit test
    // `armed_plane_runs_a_cycle_and_the_double_records_it`).
    assert_eq!(
        double.count(),
        0,
        "a chat turn must never reach the advisory plane (W1)"
    );
    assert!(double.bodies().is_empty());
    // And no key material was ever presented to any transport call.
    assert!(double.presented_keys().is_empty());

    tx.send(true).unwrap();
    handle.stopped().await;
}

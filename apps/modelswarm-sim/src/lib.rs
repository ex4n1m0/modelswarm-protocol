//! `modelswarm-sim` — deterministic loopback scenario runner for the Phase C
//! transport gate (ADR-018).
//!
//! Every scenario runs against real TCP sockets on `127.0.0.1:0` and prints
//! exactly one JSON object per line (machine-readable; timings vary, the
//! schema does not). Identities are derived from fixed seeds so peer ids and
//! request ids are stable across runs.
//!
//! Scenarios (see `main.rs` for the CLI):
//!
//! - [`pair`]: two peers handshake, one inference round-trip against a
//!   canned response (`Hel`+`lo` deltas, usage, `stop`), plus an RTT probe.
//! - [`mesh`]: `n` peers on distinct loopback ports; every pair handshakes;
//!   adjacency matrix + RTT matrix.
//! - [`kill`]: a pair where the responder dies at a deadline; the client
//!   reports a structured failure event — no silent hang, ever.
//!
//! [`pair`]: run_pair
//! [`mesh`]: run_mesh
//! [`kill`]: run_kill

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use modelswarm_identity::InstallationIdentity;
use modelswarm_transport::ed25519_dalek::VerifyingKey;
use modelswarm_transport::{
    ChatMessage, Completed, Handshake, InferenceRequest, Listener, Sampling, Session,
    SignedFrameTransport, Stats, TokenDelta, TransportError, Usage, WireMessage,
};
use serde_json::{json, Value};

/// Canned profile id used by all sim scenarios.
pub const SIM_PROFILE: &str = "msp:sim-mock:v1";
const DEADLINE: Duration = Duration::from_secs(10);

/// Deterministic identity from a fixed seed.
pub fn sim_identity(seed: u8) -> InstallationIdentity {
    InstallationIdentity::from_bytes(&[seed; 32])
}

fn handshake_for(identity: &InstallationIdentity) -> Handshake {
    Handshake::build(
        identity,
        SIM_PROFILE,
        "modelswarm-sim",
        env!("CARGO_PKG_VERSION"),
    )
}

fn sample_request(request_id: &str) -> InferenceRequest {
    InferenceRequest {
        request_id: request_id.to_string(),
        profile_id: SIM_PROFILE.to_string(),
        capability_token: "sim.capability.token".to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: "Say hello".to_string(),
        }],
        sampling: Sampling {
            temperature: 0.7,
            top_p: 0.9,
            top_k: 40,
            seed: Some(42),
        },
        max_tokens: 32,
        deadline_ms: 10_000,
        stream: true,
    }
}

// ---------------------------------------------------------------------------
// pair
// ---------------------------------------------------------------------------

/// Serving side of `pair`/`kill`: accept with signature verification, then
/// answer every request with the canned stream. Stays alive until the client
/// goes away or `stop` fires (kill scenario).
async fn serve_canned(
    listener: Listener,
    expected: VerifyingKey,
    stop_after: Option<Duration>,
) -> Result<()> {
    let mut session = listener
        .accept(&expected, DEADLINE)
        .await
        .context("responder handshake")?;
    if let Some(delay) = stop_after {
        // kill scenario: one delta, then die abruptly at the deadline
        // (drop everything without Completed — an injected peer crash).
        let req = loop {
            match session.recv(DEADLINE).await? {
                WireMessage::InferenceRequest(req) => break req,
                _ => continue,
            }
        };
        session
            .send(
                WireMessage::TokenDelta(TokenDelta {
                    request_id: req.request_id.clone(),
                    delta: "Hel".to_string(),
                    index: 0,
                }),
                DEADLINE,
            )
            .await?;
        tokio::time::sleep(delay).await;
        session.close();
        drop(session);
        return Ok(());
    }
    let mut served = false;
    loop {
        let msg = match session.recv(DEADLINE).await {
            Ok(msg) => msg,
            Err(_) if served => return Ok(()),
            Err(err) => return Err(err.into()),
        };
        let WireMessage::InferenceRequest(req) = msg else {
            continue;
        };
        for (index, delta) in ["Hel", "lo"].iter().enumerate() {
            session
                .send(
                    WireMessage::TokenDelta(TokenDelta {
                        request_id: req.request_id.clone(),
                        delta: (*delta).to_string(),
                        index: index as u32,
                    }),
                    DEADLINE,
                )
                .await?;
        }
        session
            .send(
                WireMessage::Usage(Usage {
                    prompt_tokens: 12,
                    completion_tokens: 2,
                    prefill_ms: 3,
                    decode_ms: 4,
                }),
                DEADLINE,
            )
            .await?;
        session
            .send(
                WireMessage::Completed(Completed {
                    request_id: req.request_id.clone(),
                    finish_reason: "stop".to_string(),
                }),
                DEADLINE,
            )
            .await?;
        served = true;
    }
}

/// `pair`: one verified handshake + one inference round-trip + RTT probes.
/// Returns the JSON summary object (printed by the caller).
pub async fn run_pair(rtt_samples: usize) -> Result<Value> {
    let client = sim_identity(101);
    let listener = SignedFrameTransport::listen("127.0.0.1:0")
        .await
        .context("pair listener")?;
    let addr = listener
        .local_addr()
        .context("pair local addr")?
        .to_string();
    let responder = tokio::spawn(serve_canned(listener, client.verifying_key(), None));

    let started = Instant::now();
    let mut session = SignedFrameTransport::connect(&addr, handshake_for(&client), DEADLINE)
        .await
        .context("pair connect+handshake")?;
    session
        .send(
            WireMessage::InferenceRequest(sample_request("sim-pair-0001")),
            DEADLINE,
        )
        .await
        .context("pair request send")?;

    let mut text = String::new();
    let mut deltas = 0u32;
    let mut usage: Option<Usage> = None;
    let mut finish_reason = String::new();
    while let Ok(msg) = session.recv(DEADLINE).await {
        match msg {
            WireMessage::TokenDelta(d) => {
                text.push_str(&d.delta);
                deltas += 1;
            }
            WireMessage::Usage(u) => usage = Some(u),
            WireMessage::Completed(c) => {
                finish_reason = c.finish_reason;
                break;
            }
            WireMessage::StreamError(e) => anyhow::bail!("stream error: {} {}", e.code, e.message),
            other => anyhow::bail!("unexpected message: {other:?}"),
        }
    }
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;

    let stats: Stats = session
        .measure_rtt(rtt_samples.max(1), DEADLINE)
        .await
        .context("pair rtt probe")?;
    session.close();
    responder
        .await
        .context("responder task")?
        .context("responder flow")?;

    Ok(json!({
        "scenario": "pair",
        "ok": true,
        "client_peer_id": client.peer_id_label(),
        "request_id": "sim-pair-0001",
        "profile_id": SIM_PROFILE,
        "text": text,
        "token_deltas": deltas,
        "finish_reason": finish_reason,
        "prompt_tokens": usage.map(|u| u.prompt_tokens),
        "elapsed_ms": round2(elapsed_ms),
        "rtt_p50_ms": round2(stats.p50_ms),
        "rtt_p95_ms": round2(stats.p95_ms),
        "jitter_ms": round2(stats.jitter_ms),
    }))
}

// ---------------------------------------------------------------------------
// mesh
// ---------------------------------------------------------------------------

/// `mesh`: `n` peers (2..=16) on distinct loopback ports; every unordered
/// pair handshakes (direction i→j); prints adjacency + RTT matrices. The RTT
/// for pair (i,j) is measured on that single session and mirrored into both
/// matrix cells.
pub async fn run_mesh(n: usize, rtt_samples: usize) -> Result<Value> {
    anyhow::ensure!((2..=16).contains(&n), "mesh needs 2..=16 peers, got {n}");
    let identities: Vec<InstallationIdentity> = (0..n).map(|i| sim_identity(i as u8 + 1)).collect();
    let peer_ids: Vec<String> = identities.iter().map(|p| p.peer_id_label()).collect();

    let mut listeners = Vec::with_capacity(n);
    let mut addrs = Vec::with_capacity(n);
    for _ in &identities {
        let listener = SignedFrameTransport::listen("127.0.0.1:0")
            .await
            .context("mesh listener")?;
        addrs.push(
            listener
                .local_addr()
                .context("mesh local addr")?
                .to_string(),
        );
        listeners.push(Some(listener));
    }

    // Trusted peer directory (the tracker's job in production): peer id ->
    // verifying key, shared by every accepting side.
    let directory: HashMap<String, VerifyingKey> = identities
        .iter()
        .map(|p| (p.peer_id_label(), p.verifying_key()))
        .collect();

    // Peer j accepts exactly j incoming connections (from peers i < j) and
    // holds every session open until the whole mesh finished dialing — the
    // transport answers RTT pings itself, so the sessions must outlive the
    // dialing loop, and dropping them early would EOF the peer mid-probe.
    let (release_tx, release_rx) = tokio::sync::watch::channel(false);
    let mut acceptors = Vec::with_capacity(n);
    for (j, listener_slot) in listeners.into_iter().enumerate() {
        let listener = listener_slot.expect("each peer has a listener");
        let directory = directory.clone();
        let mut release = release_rx.clone();
        acceptors.push(tokio::spawn(async move {
            let mut sessions = Vec::new();
            for _ in 0..j {
                sessions.push(
                    listener
                        .accept_with(|hs| directory.get(&hs.peer_id).copied(), DEADLINE)
                        .await?,
                );
            }
            // Hold the sessions until the dialing phase is complete.
            let _ = release.changed().await;
            Ok::<(), TransportError>(())
        }));
    }

    let mut adjacency = vec![vec![0u8; n]; n];
    let mut rtt = vec![vec![0.0f64; n]; n];
    let mut jitter = vec![vec![0.0f64; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let dialer = identities[i].clone();
            let mut session =
                SignedFrameTransport::connect(&addrs[j], handshake_for(&dialer), DEADLINE)
                    .await
                    .with_context(|| format!("mesh connect {i}->{j}"))?;
            let stats = session
                .measure_rtt(rtt_samples.max(1), DEADLINE)
                .await
                .with_context(|| format!("mesh rtt {i}->{j}"))?;
            adjacency[i][j] = 1;
            adjacency[j][i] = 1;
            rtt[i][j] = stats.p50_ms;
            rtt[j][i] = stats.p50_ms; // same session, measured from i
            jitter[i][j] = stats.jitter_ms;
            jitter[j][i] = stats.jitter_ms;
            session.close();
        }
    }
    let _ = release_tx.send(true);
    for acceptor in acceptors {
        acceptor
            .await
            .context("mesh acceptor task")?
            .context("mesh accept flow")?;
    }

    Ok(json!({
        "scenario": "mesh",
        "ok": true,
        "peers": n,
        "peer_ids": peer_ids,
        "adjacency": adjacency,
        "rtt_p50_ms": rtt.iter().map(|row| row.iter().map(|v| round2(*v)).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "jitter_ms": jitter.iter().map(|row| row.iter().map(|v| round2(*v)).collect::<Vec<_>>()).collect::<Vec<_>>(),
    }))
}

// ---------------------------------------------------------------------------
// kill
// ---------------------------------------------------------------------------

/// `kill`: a pair where the responder dies (abrupt session drop) after
/// `at_ms`. The client must observe an explicit failure — `closed` (peer
/// EOF) or `timeout` (deadline, capped at at_ms + 2 s) — and the scenario
/// returns a structured failure event, never a hang.
pub async fn run_kill(at_ms: u64) -> Result<Value> {
    let at = Duration::from_millis(at_ms.max(1));
    let client = sim_identity(201);
    let listener = SignedFrameTransport::listen("127.0.0.1:0")
        .await
        .context("kill listener")?;
    let addr = listener
        .local_addr()
        .context("kill local addr")?
        .to_string();
    let responder = tokio::spawn(serve_canned(listener, client.verifying_key(), Some(at)));

    let started = Instant::now();
    let connect_result =
        SignedFrameTransport::connect(&addr, handshake_for(&client), DEADLINE).await;
    let mut session: Session = match connect_result {
        Ok(session) => session,
        // Responder died before the exchange completed: still an explicit,
        // structured failure — not a hang.
        Err(err) => {
            return Ok(kill_event(
                "connect_failed",
                &err.to_string(),
                err.msp_code().map(str::to_string),
                at_ms,
                started,
                0,
            ));
        }
    };
    session
        .send(
            WireMessage::InferenceRequest(sample_request("sim-kill-0001")),
            DEADLINE,
        )
        .await
        .context("kill request send")?;

    // First delta arrives; the responder then dies at `at`. Bound the wait
    // strictly (at + 2 s) so the scenario is hang-free by construction.
    let recv_deadline = at + Duration::from_secs(2);
    let mut deltas = 0u32;
    let failure: TransportError = loop {
        match session.recv(recv_deadline).await {
            Ok(WireMessage::TokenDelta(_)) => deltas += 1,
            Ok(_) => continue,
            Err(err) => break err,
        }
    };
    let observed = match &failure {
        TransportError::Timeout => "timeout",
        _ => "closed",
    };
    let code = failure.msp_code().map(str::to_string);
    let event = kill_event(observed, &failure.to_string(), code, at_ms, started, deltas);
    // The responder already died (that is the point); reap it so no task
    // outlives the scenario.
    let _ = responder.await;
    Ok(event)
}

fn kill_event(
    observed: &str,
    detail: &str,
    code: Option<String>,
    at_ms: u64,
    started: Instant,
    deltas: u32,
) -> Value {
    json!({
        "scenario": "kill",
        "ok": false,
        "event": "peer_failure",
        "observed": observed,
        "msp_code": code,
        "detail": detail,
        "died_after_ms": at_ms,
        "token_deltas_received": deltas,
        "elapsed_ms": round2(started.elapsed().as_secs_f64() * 1_000.0),
        "hang": false,
    })
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim_identities_are_deterministic() {
        assert_eq!(
            sim_identity(1).peer_id_label(),
            sim_identity(1).peer_id_label()
        );
        assert_ne!(
            sim_identity(1).peer_id_label(),
            sim_identity(2).peer_id_label()
        );
    }
}

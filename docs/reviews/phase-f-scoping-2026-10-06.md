# Phase F scoping — from local-only serving to a public hostile swarm

Status: scoping document (plan only; no implementation approved by this
document). Owner-requested 2026-10-06 after ADR-024/025 shipped.

## Where we actually are (2026-10-06, v0.2.17)

Every installation HOSTS and SERVES locally: the desktop's gateway binds
loopback, executes through `SingleLocalExecutor`, and the roster heartbeat
registers with an honest `/ip4/0.0.0.0/tcp/0` no-listener multiaddr. The
census (peersOnline, per-model counts) is live and truthful. What does NOT
exist: a serving peer accepting an inbound stream from another machine, a
requesting gateway routing to a remote peer, and every hostile-world
control that must ride with them. The libp2p backend (ADR-014/018) exists
with a loopback-only listener guard and the F11 driver-task fix (QUIC
inbound streams need the continuously-polled swarm driver; the dialer
speaks first).

## The gap, precisely

1. **Listener**: `libp2p_backend.rs` enforces loopback (ADR-018 honesty
   guard). Production serving needs a dialable listener with a real
   external address advertised in the heartbeat multiaddr, plus dial-time
   address verification so a peer cannot register a bogus address.
2. **Serving path**: a remote peer's request must drive the SAME executor
   the local gateway uses (`SingleLocalExecutor` today), not a parallel
   implementation. The gateway needs a `RemoteExecutor` that: opens a
   framed session (ADR-014 framing), forwards the NormalizedRequest,
   streams TokenDeltas back, and maps transport failures into the
   ADR-007 retry rules (retry only before first token — already encoded
   in `run_request`).
3. **Identity/eligibility on every frame**: peer authentication
   (ADR-020 peerId ↔ libp2p PeerId binding decision), lease/eligibility
   checks at session open, and replay protection (session nonces) — the
   session state machines exist in `modelswarm-session` for exactly this
   and have never run against a real remote peer.
4. **NAT reality**: direct dial fails for a large fraction of consumers
   (CGNAT). ADR-014 reserved relay semantics; Phase F must either ship a
   relay (TURN-like libp2p circuit relay v2) or enforce the honest
   "direct-connect fail" disclosure — no universal-traversal claims
   (project rule 7).
5. **Hostile-peer controls** (the plan's Phase F core): eligibility
   audits, reputation/suspension hooks, malformed-frame and load limits
   at the listener, prompt-privacy guarantees end-to-end (prompts cross
   the wire now — they were previously process-local!), and the
   tracker's content-blindness re-verified with serving traffic.

## Proposed milestones (each independently shippable + verifiable)

- **F0 — Dialable listener behind a flag** (dev + friendly-network
  testing): lift the loopback guard behind `MSP_LISTENER=1`, heartbeat
  advertises the real address, two machines on one LAN exchange a
  framed echo + one real completion. Exit: cross-machine request served
  with correct usage accounting, no protocol changes.
- **F1 — RemoteExecutor in the requesting gateway**: micro-swarm
  selection (existing scheduler cost model) picks the fastest eligible
  peer; retry-before-first-token rules hold; honest fallback to local
  single. Exit: machine A consumes machine B's hosted profile over LAN,
  census-verified, negative result published for the fallback cases.
- **F2 — Internet reality**: STUN/relay per ADR-014, direct-connect
  failure surfaced as a first-class state (chip already exists), relay
  cost/latency honestly benchmarked vs fastest single host. Exit:
  two peers behind different NATs complete a serving session, or the
  measured failure modes are published.
- **F3 — Hostile hardening** (the plan's Phase F exit): frame fuzzing,
  load limits, replay/usage forgery tests, suspension on audit failure.
  Security Engineer leads; release-gated.

## Sequencing note

F0/F1 are Windows Product + Network Engineer work with small protocol
surface (the session state machines already specify the frames). F2 is
genuinely uncertain — that is where honest negative results are most
likely and must be published. F3 is independent of F2's outcome and can
be built against local hostile peers meanwhile.

## Open decisions for the owner (when F starts)

- Relay hosting: who runs the relay(s)? (Vercel can't — it's data-plane
  traffic; a cheap VM or owner machine.)
- Minimum viable geography/latency targets for F2 exit claims.
- Whether micro-swarm selection (cooperative modes) stays out of scope
  until F1's single-remote-peer path is proven (recommended: yes — the
  cooperative track's prerequisite gate already says so).

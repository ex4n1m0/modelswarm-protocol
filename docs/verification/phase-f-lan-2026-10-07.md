# Phase F0/F1 verification — first cross-machine swarm completion

2026-10-07 (~23:40 setup, proof completed 2026-10-08 morning). Two real
machines on one LAN, production tracker, real model weights.

## Setup

| Machine | Role | Detail |
|---|---|---|
| A (dev laptop) | requester | 192.168.100.42, RTX 5080, dev build (F0+F1 code), MSP_LISTENER=1, hosts Qwen2.5-7B Q4_K_M on Vulkan |
| B (DESKTOP-MBQ7VBM) | serving peer | 192.168.100.43, LAN test zip (same build), MSP_LISTENER=1, hosts Qwen2.5-7B Q4_K_M |

Profile: `msp1:fc5a30ae36257718afbd1062f2920b47576eed72ee8d172eac35248034b2dcbe`
(Qwen2.5-7B Instruct Q4_K_M, bartowski@8911e8a4, sha256 65b8fcd9…1423,
engine llama.cpp b11407). Both peers registered on
modelswarm.deepflux.space with REAL multiaddrs (A:
`/ip4/192.168.100.42/udp/63475/quic-v1`, B: `/ip4/192.168.100.43/udp/56282/quic-v1`)
after `MSP_LISTENER=1` + a Windows inbound firewall rule on B.

## The proof

`cargo test -p modelswarm-node --features libp2p-backend
lan_cross_machine_completion -- --ignored` (env: MSP_LAN_PROFILE,
MSP_DATA_DIR, MSP_EXCLUDE_ADDR=192.168.100.42 for structural
self-exclusion):

1. Signed roster lookup against the PRODUCTION tracker (the Rust wire
   client, after 2026-10-07's two signing fixes) returns both peers.
2. The dial selects B by PeerId (`12D3KooWJkrYdaeCNMh3wDe3RCa833LN…`)
   at `/ip4/192.168.100.43/udp/56282/quic-v1`.
3. QUIC handshake authenticates B's PeerId; the InferenceRequest frames
   across; B's serving bridge drives its local 7B executor; TokenDelta
   frames stream back.

Observed (self-dial sanity, then the cross-machine run):

```
dialing /ip4/192.168.100.43/udp/56282/quic-v1 (peer 12D3KooWJkrYdaeC…)
finish: Length
REMOTE REPLY (24 tokens, 1.6s): Cross-machine swarm serving works by
allowing multiple machines to collaborate in serving content or
processing tasks, effectively scaling the system's capacity
test result: ok
```

(Self-dial to A's own listener the same evening: 24 tokens in 0.6 s —
the serving path is exercised identically; the cross-machine run adds
the network + B's hardware: ~1.6 s total.)

## What this gates

- **F0 exit gate met**: dialable listener, honest advertisement, serving
  bridge, cross-machine completion, census-visible peers.
- **F1's RemoteExecutor proven over a real network** (not just loopback
  tests): dial, frame exchange, terminal handling, honest error mapping.
- The desktop's swarm-first chat path (send_chat → try_swarm_chat) uses
  exactly this code path; the same two machines answering each other's
  chat is the user-visible demo.

## Bugs found and fixed on the way (all committed)

1. Rust client signed GET paths WITH query string; hub verifies pathname
   only — every roster lookup from Rust had always 401'd.
2. Signed GETs digested `"null"` instead of the empty body.
3. Tracker session was never persisted — swarm-chat roster auth could
   not work; now `data_dir/session.token`.
4. Window title stale through 0.2.18/0.2.19 (string-replace never
   matched the escaped em dash).
5. Wire-compatibility harness added (live_tracker.rs,
   `signed_flow_enroll_register_heartbeat_lookup`) — the structural
   catch-net for this bug class, green against production.

## Not yet proven (honest)

- Lease/capability verification at session open (F3 TODO): B served the
  request without verifying A's lease — next work item.
- Internet (non-LAN) operation: F2 (relay/DCUtR) untouched.
- Two machines is the smallest swarm; selection among >2 peers and any
  cooperative mode remain future work.

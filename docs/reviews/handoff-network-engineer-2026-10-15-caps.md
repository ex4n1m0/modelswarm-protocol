# Handoff — Network Engineer, 2026-10-15 (relay admission-bounds runtime surface: cap flags + --limit-summary)

Context: the F2B relay is live (Oracle VM, public UDP 4001, trial shape, interim IP allowlist —
docs/verification/f2b-relay-2026-10-15.md). The owner has directed the relay to open to everyone;
gate D12 (auth + caps) blocks that. ADR-038 (relay admission tickets) is being drafted by the
Protocol Architect in parallel. This change is the PROTOCOL-FREE half of D12's runtime surface:
operator-armed per-circuit caps and an honest startup posture line. No ticket/auth/protocol
surface is implemented here; nothing was deployed (the live relay picks this up at the next
operator-driven restart, coordinated with the ADR-038 rollout).

Design invariants honored:

- ABSENT FLAG = EXACTLY TODAY. `relay_config(None, None)` produces bit-identical `Config`
  fields to the pre-flag code (`max_circuit_duration = u32::MAX seconds`, `max_circuit_bytes =
  u64::MAX`, 4096/8/4096/128 counts). The live deployment, which runs flagless, changes nothing
  until the operator arms a cap. Pinned by `absent_cap_flags_keep_todays_uncapped_config`.
- 0 IS REJECTED, NOT REINTERPRETED. For bytes this is load-bearing: libp2p-relay 0.22's
  `CopyFuture` treats `max_circuit_bytes == 0` as "no limit" (`copy_future.rs`: the cut
  condition is `max_circuit_bytes > 0 && bytes_sent > max`), so accepting `--max-circuit-bytes 0`
  would arm nothing while announcing `data: Some(0)` on the wire. For duration, 0 would cut every
  circuit at acceptance — also not "uncapped".
- DURATION RANGE IS THE WIRE FORMAT'S. The reservation and HopConnect status messages encode
  the limit as u32 seconds and the behaviour PANICS above `u32::MAX` when announcing
  (`inbound_hop.rs`: `.expect("max_circuit_duration not to exceed u32::MAX")`), so
  `--max-circuit-duration-secs` accepts `1..=4294967295` and refuses one-past with a message
  naming the bound. The flagless default remains `u32::MAX` seconds (≈136 years), the same
  "uncapped" value the config has always used.
- A CAP IS A REAL CUT AND THE LOGS SAY SO. When any cap is armed the startup prints a
  `--limit-summary WARNING` naming ADR-007 (no retry after the first output token): a relay-side
  cut mid-token-stream is unrecoverable for the requester. `--help` carries the same warning next
  to each flag.
- THE DEFAULT IS STILL UNCAPPED ON PURPOSE. Phase-f research: the stock 2 min / 128 KiB caps
  reset token streams. The flags exist so an operator CAN bound abuse when the relay opens; the
  code does not choose caps for them.

## 1. Changed files and why

| File | Why |
|---|---|
| `crates/modelswarm-relay/src/main.rs` | The whole change. (a) `--max-circuit-duration-secs N` / `--max-circuit-bytes N` flags: parsed into `Args` as `Option<u64>`, validated (`0` refused for both; duration bounded by the u32-second wire field), and threaded through `relay_config(Option<u64>, Option<u64>)` into the real `relay::Config` the binary's behaviour stack runs. (b) Startup `--limit-summary` line (`limit_summary`) printing the EFFECTIVE posture as greppable key=value tokens (reservations, circuits, per-peer counts, duration/bytes caps or UNCAPPED), replacing the old fixed "limits reservations ... duration/bytes UNCAPPED" line whose content it fully contains; plus `caps_warning`, printed only when a cap is armed. (c) `--help` documents both flags honestly (default UNCAPPED, the ADR-007 cut warning, the ranges, why 0 is refused). (d) Module doc updated for the D12 runtime-half posture. (e) Six new tests (below); existing tests retargeted to the two-arg `relay_config(None, None)` with identical assertions; new `std_client_swarm_with_ping(interval)` helper (the cap-cut proofs need fast pings; `std_client_swarm()` delegates with the old 30s). |
| `docs/reviews/handoff-network-engineer-2026-10-15-caps.md` | This handoff. |

Not touched (per task): `protocol/`, `apps/tracker`, desktop/node crates, any ticket/auth surface,
the live deployment.

## 2. Exact commands and outcomes

- `cargo fmt -p modelswarm-relay` then `cargo fmt --all --check` — clean.
- `cargo clippy -p modelswarm-relay --all-targets -- -D warnings` — 0 warnings.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo clippy --workspace --all-targets --features modelswarm-node/libp2p-backend -- -D warnings` — clean.
- `cargo clippy --workspace --all-targets --features modelswarm-node/verifydrafts,modelswarm-node/libp2p-backend -- -D warnings` — clean.
- `cargo test -p modelswarm-relay` — 12 passed / 0 failed, run 3x, all green (5.7s each).
- `cargo test --workspace` — 607 passed / 0 failed across 78 test binaries.
- `cargo test --workspace --features modelswarm-node/libp2p-backend` — 633 passed / 0 failed (includes the transport crate's libp2p-backend suite: the pre-existing duration/byte cap-cut machinery proofs).
- Live smoke (dev build, loopback binds only): `cargo run -p modelswarm-relay -- --help` renders the new flag docs; startup with `--max-circuit-duration-secs 3600 --max-circuit-bytes 1073741824` prints the armed summary + WARNING; startup with no flags prints `max_circuit_duration=UNCAPPED max_circuit_bytes=UNCAPPED` and no warning; `0`/`4294967296`/`soon` values each exit 2 with the honest one-line diagnostic. Smoke artifacts (`msp-relay.key`, `err.txt`) removed.

## 3. Test evidence

New tests in `crates/modelswarm-relay/src/main.rs` (12 total in the crate, was 6):

- `cap_flags_parse_validate_and_default_to_absent` — flag parsing: absent = `None`/`None` =
  `Args::default()`; valid values land (alone, combined with `--listen`/`--public`);
  duration boundary `4294967295` ok and `4294967296` refused naming `u32::MAX` + wire format;
  `0` refused for both naming "omit the flag"; non-numeric/fractional/negative/valueless all
  refused.
- `absent_cap_flags_keep_todays_uncapped_config` — the regression: parse empty argv, build the
  config, assert every field equals today's pinned values (4096/8/4096/128, u64::MAX bytes,
  u32::MAX-seconds duration).
- `armed_cap_flags_reach_the_relay_config_exactly` — parsed values are the enforced values;
  the raised reservation/circuit counts are untouched; one flag alone leaves the other uncapped.
- `limit_summary_reports_the_effective_posture` — the ops line: full key=value vocabulary with
  both caps UNCAPPED and NO warning in the default posture; armed numbers appear, `UNCAPPED`
  disappears for the armed cap, the warning names CUT and ADR-007; one-cap-armed mixes correctly.
- `flag_armed_duration_cap_cuts_the_circuit` (e2e, standard libp2p clients) — THE FLAG PATH:
  `--max-circuit-duration-secs 2` is parsed from argv, fed through `relay_config`, and run by
  the binary's exact behaviour stack (`start_relay`); a dialer establishes a relayed circuit to
  a reserving server, pings flow (attributed by `ConnectionId`, so only pings that crossed the
  circuit count — the ping behaviour also pings the plain transport connection to the relay),
  and the relay CUTS the circuit: the client's relayed connection closes 1-10s after
  establishment (an uncapped circuit would live to the 60s idle timeout; a handshake failure
  would die under 1s). The relay's own `circuit A -> B` line is required as corroboration.
- `flag_armed_byte_cap_cuts_the_circuit` (e2e) — same shape with `--max-circuit-bytes 4096`:
  the circuit establishes, carries pings, and is cut once the relay's forwarded-byte counter
  crosses the cap. Deterministic because ping traffic never stops, so the counter grows past any
  finite cap; the window (0.5-20s after establishment) sits between "first ping delivered" and
  the 45s proof deadline.

These are the relay-crate, flag-path counterparts of the transport crate's pre-existing
`announced_circuit_duration_limit_cuts_the_stream` / `announced_circuit_byte_limit_cuts_the_stream`
(machinery proofs, unchanged and still green under the feature combo).

## 4. Assumptions

- "--limit-summary log line" means an unconditional startup log line carrying the literal token
  `--limit-summary` (greppable ops surface), not a new mode flag; the old fixed "limits ..."
  startup line was replaced because the summary line contains all of its information.
- Duration is reported `UNCAPPED` at `>= u32::MAX` seconds (the value the flagless config itself
  uses for "no signaling-grade cap"); a flag value of exactly `4294967295` therefore also prints
  UNCAPPED — semantically identical to absent.
- Last-flag-wins on repetition, matching the existing `--listen`/`--key` behavior; not newly
  restricted.
- The e2e cut windows (1-10s duration, 0.5-20s bytes) were chosen from the cut mechanics
  (deadline Delay from circuit acceptance; byte counter over handshake+identify+pings) and
  observed green 3x; they are generous on both sides rather than tight diagnostics.
- The live relay restart (operator-driven, with the ADR-038 rollout) will run flagless first:
  identical behavior, new summary line only.

## 5. Unresolved risks

- The caps protect the RELAY's resources, not the network's admission: D12 remains open until
  ADR-038's auth/ticket surface lands; the relay still refuses nothing on identity grounds, and
  per gate D12 it must not open to the public internet before that.
- An operator can arm a cap that silently degrades long completions (a cut mid-stream is
  unrecoverable per ADR-007). Mitigations shipped: UNCAPPED default, the startup WARNING, and
  `--help` guidance to size for the full streamed response — but the mistake remains possible.
- Count caps (reservations/circuits/per-peer) stay compile-time constants (4096/8/4096/128);
  they are NOT flag-wired yet. If ADR-038's "frozen cap policy" wants them runtime-tunable,
  that is follow-up work in this crate.
- The byte-cap e2e relies on ping traffic to cross the cap; if libp2p's ping payload or framing
  ever shrinks dramatically the test window widens but stays deterministic (finite cap, unbounded
  traffic). The duration e2e is timing-window based; CI slowness beyond ~10s propagation would
  fail it visibly rather than pass vacuously.
- systemd unit on the VM may need the flags added when the operator restarts; nothing in this
  change deploys or configures that.

## 6. Suggested next task

Merge this, then hold for ADR-038: when the Architect's admission-ticket ADR lands, implement the
protocol half in `crates/modelswarm-relay` (ticket validation at reservation/circuit admission)
plus — if ADR-038's frozen cap policy requires it — promoting the count caps
(reservations/circuits/per-peer) to the same flag surface, and only then reopen the firewall
beyond the allowlist per gate D12. Independent follow-up available now: F2B's real-network
DCUtR relay proof (the remaining F2B exit-gate item from the verification doc's checklist).

# Handoff — Network Engineer, 2026-10-13: explicit non-loopback listen opt-in (ADR-018 guard parameter)

Seam routed from `docs/reviews/handoff-windows-product-engineer-2026-10-13-v0223.md`
§5.1: the desktop's serving-listener bind satisfied the transport's ADR-018
non-loopback guard by setting `MSP_LISTENER=1` in its own process before
binding — an env detour recorded as the wart. This change replaces the env
contract with an explicit, typed opt-in parameter on the transport's listen
path. Scope: `crates/modelswarm-transport` only; the desktop call-site change
is the Integrator's (exact diff in §6).

## 1. Changed files and why

- `crates/modelswarm-transport/src/libp2p_backend.rs`
  - NEW `ListenOptions` (private field, two `#[must_use]` constructors):
    `ListenOptions::loopback_only()` (= `Default`) and
    `ListenOptions::allow_off_loopback()`.
  - `Libp2pTransport::listen(bind)` — signature UNCHANGED; now documents
    loopback-only and delegates to `listen_with(bind, ListenOptions::loopback_only())`.
  - NEW `Libp2pTransport::listen_with(bind, options)` — the only path that can
    lift the guard; enforcement moved here:
    `if !addr.ip().is_loopback() && !options.allow_off_loopback { Err(NonLoopbackDenied) }`.
  - REMOVED the `std::env::var("MSP_LISTENER")` read (was line 492) — the
    only reader of that variable in the entire repo.
  - Module docs, method docs, and the `Libp2pListener` doc line updated to
    the new contract.
  - NEW test `libp2p_listen_explicit_opt_in_is_the_sole_authority` (see §3);
    existing `libp2p_listen_refuses_non_loopback` kept, doc updated to state
    the refusal is now environment-independent.
- `crates/modelswarm-transport/src/lib.rs` — crate-root docs gain the
  non-loopback listen policy paragraph (staged TCP backend: never; libp2p
  backend: only via the explicit `ListenOptions::allow_off_loopback`
  argument; `MSP_LISTENER` retired, never read).

No other crate's bytes changed. No protocol/, catalog/, or shared-schema
change — crate-local API only, so no new ADR is required (ADR-018's own text
— non-loopback refused in code and tested — is TIGHTENED back to its letter;
the env escape was a Phase-F0 friendly-network convenience recorded in
handoffs, never in the ADR).

## 2. Env-contract decision (per ADR-018) + every in-repo reader

**Decision: REMOVED, not aliased. Opt-in-wins semantics — the explicit
parameter is the sole authority; `MSP_LISTENER` is never read by this crate
(ignored even when set to exactly `"1"`).**

Rationale: the Windows Product handoff records `MSP_LISTENER=1` still set at
MACHINE scope on both live test machines. Under a legacy-alias contract, any
code path that forgets the parameter would still get non-loopback binds on
those machines — the guard would be theater exactly where testing happens.
Removing the read makes the typed parameter the only key, everywhere, and
ends the process-global-state detour that motivated the seam.

Complete in-repo census (grep over the whole workspace incl. benches/tests/
scripts/installer/workflows; code sites only — the rest are docs):

| Site | Kind | Action |
|---|---|---|
| `crates/modelswarm-transport/src/libp2p_backend.rs:492` (pre-change) | the ONLY reader (`env::var`) | REMOVED in this change |
| `crates/modelswarm-desktop/src/app.rs:491` | setter (`set_var`, in `bind_serving_listener`) | Integrator removes with the §6 diff |
| `crates/modelswarm-bench/examples/pass1_serve.rs:118` | setter (`set_var`) | owner-run cross-machine harness; dead after this change — delete the line |
| `crates/modelswarm-bench/src/runner.rs:550` | behavioral caller (`.listen(bind_addr)`, non-loopback when the harness passes `--bind-ip` off-loopback; CI default `127.0.0.1` unaffected) | Scheduler-owned: switch to `listen_with(.., ListenOptions::allow_off_loopback())` |
| doc-comment mentions: `runner.rs:350`, desktop `app.rs:335/337/437/471/5691` | comments only | update opportunistically with the above |
| `docs/**` (ADRs 032/034, verification records, prior handoffs) | historical records | left as history; new contract documented in the crate docs + this file |

No `.bat`/`.ps1`/installer/workflow file reads or sets it (grepped all
extensions; the "set by the bat" comment in `pass1_serve.rs` refers to an
untracked owner-local script).

## 3. Test evidence

Baselines stated BEFORE the change (this machine, HEAD f85347d):

- `cargo test -p modelswarm-transport` → 22 + 12 + 0 passed (34 total).
- `cargo test -p modelswarm-transport --features libp2p-backend` → 30 + 12 + 0 (42).

After the change:

- `cargo test -p modelswarm-transport` → 22 + 12 + 0 (34, baseline exact).
- `cargo test -p modelswarm-transport --features libp2p-backend` → 31 + 12 + 0 (43; +1 = the new pin).
- `cargo test -p modelswarm-transport --features libp2p-backend libp2p_listen` →
  `libp2p_listen_refuses_non_loopback ... ok`,
  `libp2p_listen_explicit_opt_in_is_the_sole_authority ... ok`,
  `libp2p_listener_serves_two_sessions ... ok`.

The new test pins the designed semantics explicitly:
`MSP_LISTENER=1` set → `listen("0.0.0.0:0")`, `listen_with(.., Default)`,
and `listen_with("192.168.1.10:9000", loopback_only)` all still
`NonLoopbackDenied`; `listen_with("0.0.0.0:0", allow_off_loopback())` binds
with the variable set AND after `remove_var`; loopback binds work on both
entry points without any opt-in.

## 4. Exact commands run and outcomes

| Command | Outcome |
|---|---|
| `cargo fmt --check` (pre + post) | clean (one `cargo fmt` reflow applied to the new `listen` delegation) |
| `cargo clippy -p modelswarm-transport --all-targets -- -D warnings` (pre + post) | clean |
| `cargo clippy -p modelswarm-transport --all-targets --features libp2p-backend -- -D warnings` (pre + post) | clean |
| `cargo test -p modelswarm-transport` (pre + post) | 34 pass / 0 fail both |
| `cargo test -p modelswarm-transport --features libp2p-backend` (pre + post) | 42 → 43 pass / 0 fail |
| `cargo check --workspace --all-targets` | exit 0 — desktop/node/bench compile UNCHANGED against the new crate (the additive API kept `listen`'s signature) |
| `cargo test --workspace` | exit 0 — 496 passed / 0 failed / 6 ignored over 69 suites (baseline at HEAD f85347d: 495/0/6; +1 = the new pin, no regressions) |

## 5. Assumptions

- The staged TCP `SignedFrameTransport` stays loopback-only UNCONDITIONALLY
  (it has no confidentiality; ADR-018). The opt-in exists only on the libp2p
  QUIC backend. Deliberate asymmetry, documented on `ListenOptions`.
- The opt-in never influences what is ADVERTISED (heartbeat multiaddr stays
  the caller's honest separate decision) — unchanged from the env contract.
- Shipped v0.2.23 binaries embed the OLD transport and are unaffected; the
  interim on main (between this commit and the Integrator's desktop fix) is
  an honest runtime degrade, not a compile break: a dev build of current main
  with pairing allowed will report `listener_bind_failed` (FamilyStatus
  detail path added in 3a0bb60) until §6 lands.

## 6. Integrator call-site diffs (required next step)

Desktop (crates/modelswarm-desktop/src/app.rs, `bind_serving_listener`) —
current lines ~486-491 + ~518:

```diff
-    // The transport's ADR-018 guard refuses non-loopback binds without its
-    // env contract; the toggle is the USER-facing decision, so satisfy the
-    // guard on the user's behalf (this process binds exactly what the
-    // toggle allowed). Cross-crate seam: the guard should take an explicit
-    // opt-in parameter — recorded for the Network Engineer.
-    std::env::set_var("MSP_LISTENER", "1");
```

```diff
-    let listener = match backend.listen("0.0.0.0:0").await {
+    let listener = match backend.listen_with(
+        "0.0.0.0:0",
+        modelswarm_transport::libp2p_backend::ListenOptions::allow_off_loopback(),
+    )
+    .await
+    {
```

(the match arms are unchanged). New public API being consumed:

```rust
pub struct ListenOptions { /* private field */ }
impl ListenOptions {
    pub fn loopback_only() -> Self;      // = Default
    pub fn allow_off_loopback() -> Self;
}
impl Libp2pTransport {
    pub async fn listen(&self, bind: &str) -> Result<Libp2pListener, TransportError>;              // loopback-only, env-free
    pub async fn listen_with(&self, bind: &str, options: ListenOptions) -> Result<Libp2pListener, TransportError>;
}
```

Bench (Scheduler-owned, cross-machine harness only; CI loopback unaffected):
`crates/modelswarm-bench/src/runner.rs:550` →
`.listen_with(&(config.bind_addr.clone() + ":0"), ListenOptions::allow_off_loopback())`
(import from `modelswarm_transport::libp2p_backend`), and delete the dead
`std::env::set_var("MSP_LISTENER", "1");` in
`crates/modelswarm-bench/examples/pass1_serve.rs:118` (comment at :109 too).

## 7. Unresolved risks

- Interim main (until §6 lands): desktop serving-listener binds fail
  honestly (`listener_bind_failed` + exact error) — the family line shows it;
  no crash, no silent off. This is the agreed sequencing cost of
  remove-not-alias.
- The release-notes/ADR doc trail (v0.2.21-0.2.23 notes, ADR-032/034 §s)
  still tells users `MSP_LISTENER=1` is the opt-in gate; the next release
  notes must state the variable is now inert (shipped binaries pre-dating the
  change still honor it internally — they embed the old reader).
- CI gate evidence for `cargo test --workspace` at this commit: 496/0/6 (§4).
  The desktop tauri-shell suite was not re-run here (no desktop bytes
  changed; `cargo check --workspace --all-targets` covers compilation).

## 8. Suggested next task for the Integrator

Land §6 (desktop diff + bench fix) as the immediate next commit so the
interim honest-degrade window closes, then have Test/Release note the env
inertia in the next train's release notes. After that, the standing F2B
items (real-network relay proof incl. DCUtR; incremental Swarm migration
preserving this guard, dial-side PeerId verification, and size-before-
allocation framing) remain the open Network work.

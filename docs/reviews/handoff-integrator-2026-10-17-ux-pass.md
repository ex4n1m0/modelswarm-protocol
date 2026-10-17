# Handoff — Integrator UX pass (site + desktop app), 2026-10-17

Owner directive: "take a deep look at the website and the application in the
current status, and improve so that it is much easier to understand and to
use." Audit was performed against the LIVE site (modelswarm.deepflux.space,
screenshots + DOM) and the RUNNING installed app (v0.2.32 on machine A,
AX tree + screenshots), then against source. Commit: `1d61829` (pushed,
main == origin/main). Rides the v0.2.33 train that was building concurrently
(installer workflows in progress at push time).

## 1. Changed files and why

### Website (deploys with the next `vercel deploy --prod`)

- `apps/tracker/app/globals.css` (NEW) — the entire design system previously
  lived in an inline `<style>` on the landing page only. `/reference` (added
  2026-10-17) used the same class names but had NO stylesheet on its route:
  it rendered browser-default — dark text on the near-black body,
  effectively unreadable (verified by screenshot before the fix). Tokens and
  rules moved here verbatim; adds `.hero-band`, `.steps`, `.dl-grid`,
  `.btn.ghost` for the new blocks.
- `apps/tracker/app/layout.tsx` — imports `./globals.css` (one line).
- `apps/tracker/app/page.tsx` —
  - real visible `<h1>` ("Your computers, one AI swarm.") + plain-language
    subhead + two CTA buttons ("Get the app" → #get, "How it works" → #how)
    in a band under the hero artwork. Before: the only headline was baked
    into the hero PNG (sr-only h1 for AT) and the only CTA was a small nav
    button — a sighted first-time visitor got no headline and no visible
    next step.
  - lede rewritten to the one forward-looking line (mixed models as
    solvers/skeptics/judges, behind their own gates) — the hero band now
    carries the plain-language product description.
  - NEW `#how` "How it works" section — three honest steps, present tense
    only for what ships today (install + self-update, join local or cloud,
    chat + local API). Placed AFTER the census so the census remains the
    first section under the lede (2026-10-12 owner directive, kept).
  - downloads: OS-labeled buttons in a grid, Windows primary (`.dl
    primary`), replacing the bare three-bullet link list. Same v0.2.33
    counting-route URLs, byte-for-byte.
  - census script: 4 s honest fallback — a stalled first poll now shows
    "census unavailable — still trying…" instead of "loading…" forever
    (observed live: stats endpoint is fast, but the badge/census can sit at
    their initial strings if the fetch stalls). DOM ids `online-badge` /
    `census-body` and the 30 s refresh unchanged.
  - footer moved out of the `.wrap` div (it was nested inside; cosmetic).
  - copy-contract comment updated to record this pass and its kept
    invariants (honest tense, no engine/format names, no payments copy,
    census first, ids preserved).

### Desktop app (rides the next release — NOT in 0.2.33 binaries if the
train already bundled; lands with the following build)

- `crates/modelswarm-desktop/ui/index.html` —
  - model picker cloud row told the user "— cloud · next release" and was
    always disabled, but ADR-040 C3 cloud serving is LIVE (daemon
    `select_model` serves cloud through the staged key). Now key-aware:
    key set → enabled "— cloud · ready to join"; no key → disabled "—
    cloud · add your <provider> key (Settings)" (same key detection as the
    model cards). This was the picker contradicting the truth one screen
    away from the "Join the swarm" cards.
  - picker rows deduped: "Qwen2.5 7B Instruct (Q4_K_M) (q4_k_m (4 bit))" →
    "Qwen2.5 7B Instruct (Q4_K_M) · downloaded" (display_name already
    carries the quant).
  - "joining…" button feedback actually works now: the join-flow commit
    (9f8c4e2) restored the label synchronously after an un-awaited promise,
    so the feedback flashed for a microtask; the restore now runs in
    `.finally()`.
  - empty-catalog picker text points at the tracker generically ("check the
    tracker URL in Settings…") instead of the old menu name.
  - stale cross-reference fixed: peer-roster device-approval note said
    "Settings → Tracker, engine & device" (menu name that no longer
    exists) → "Settings → Cloud API key, tracker & engine…".
  - menu item + sheet title "Swarm pairing & content carriage…" → "Swarm
    pairing & conversation carriage…" (matches the daemon's own
    "conversation carriage" wording; harnesses open panels by stable
    `data-panel` ids, unaffected).
  - chat heading "Test box — chat with the hosted model" → "Chat with the
    hosted model" (drop the dev-tool framing; id `chat-h` kept).
  - Setup panel: cloud-key field widened (11em → 14em) + one-line
    explainer under it (joins as 1 peer, replies labeled cloud, key stays
    on this machine, never shown again).
  - swarm HUD off-state now points the right way: "pick a model in the
    picker above, or press 'Join the swarm' in Settings → Models."
- `crates/modelswarm-desktop/src/app.rs` — `local_manifest_of` cloud
  refusal said "cloud serving lands in the next release (ADR-040)" — false
  since C3 shipped. Now: "is a cloud model — it has no local download; join
  it from the model picker (it serves through your cloud API key)". No test
  pinned the old string (grepped).

## 2. Exact commands and outcomes

- `npm run typecheck` (apps/tracker) — clean.
- `npx vitest run` (apps/tracker) — 195 passed, 2 skipped, 0 failed.
- `npm run build` (apps/tracker) — success (/, /reference, /verify + API routes).
- `cargo fmt` then `cargo fmt --check` — clean.
- `cargo clippy -p modelswarm-desktop --features tauri-shell,verifydrafts --all-targets` — clean.
  (NOTE: plain `cargo clippy -p modelswarm-desktop` checks only the stub —
  `mod app` is behind the `tauri-shell` feature.)
- `cargo test -p modelswarm-desktop --features tauri-shell,verifydrafts` — 75/75 pass.
- UI harnesses (standalone HTML runners, served over localhost, run in a
  real browser): `ui-harness/pairing-chat.html` 15/15 PASS; `ui-harness/
  adr036-carriage.html` 18/18 PASS (both exercise the renamed serving panel
  and the chat section against the real index.html at 1152×648).

## 3. Assumptions — RESOLVED 20:00 local

- The concurrent v0.2.33 train staged and deployed as expected: stage
  commit `2507798` (signed 4/4, rehearsal ALL VERIFY) sits ON TOP of this
  pass's `1d61829`, and the production deploy at 20:00:39 carries the new
  landing page + /reference fix. Verified live: headline/CTA band renders
  without collision, /reference fully themed, census live,
  `/api/download/…-0.2.33-windows-x64.exe` 302 → 206 with bytes.
- The 0.2.33 BINARIES were built from the train's dispatched run on
  bf8963e (before this pass landed), so the desktop-app polish below rides
  the NEXT release (0.2.34). The push-triggered installer run on 1d61829
  (39030437738) completed with newer artifacts; restaging from it would
  change hashes — owner's call only.

## 4. Unresolved risks

- None open for the website. Desktop polish awaits the next release train.

## 5. Suggested next task for the integrator

Done for this pass (see §3). Next: run the v0.2.34 train whenever the
owner calls for it — it carries the desktop-app polish — and re-run the
two UI harnesses then.

# Handoff — Windows Product Engineer: per-model live peer census in the picker

Date: 2026-10-12 · Scope: `crates/modelswarm-desktop` only (+ this handoff).
Trigger: owner directive 2026-10-12 — "we should have indication on what models
people are hosting, so new people can select those that have more people". The
website side already landed (the Live swarm census section in
`apps/tracker/app/page.tsx`); this is the desktop-picker counterpart reading the
same public counts.

## 1. Changed files and why

| File | Change |
|---|---|
| `crates/modelswarm-desktop/src/app.rs` | New `census` IPC command (public `GET /api/v1/stats`, no auth, 6 s timeout, strict parse) + daemon-side census cache; `list_models` joins that cache into every response (per-card `peers` + `census_chip`) and returns cards in the canonical order; pure functions `parse_census_counts` / `census_peers` / `census_chip` / `tier_rank` / `census_order` / `census_view`; 5 new unit tests; IPC doc-table row. |
| `crates/modelswarm-desktop/ui/index.html` | `refreshModels` split into fetch (`refreshModels`) + render (`renderModels`); each model card renders the daemon-provided chip (` · N peers` / ` · first peer` / ` · census n/a`); card order is the daemon's, rendered verbatim; new 30 s `refreshCensus` poll (initial call + interval) that re-pulls the catalog after every census outcome; catalog-failure path clears the cached models so a late census tick cannot overwrite the honest "catalog unreachable" note. |
| `crates/modelswarm-desktop/ui-harness/census.html` (new) | Checked-in static stub harness: loads the REAL `ui/index.html` into a same-origin iframe with a stubbed `window.__TAURI__` bridge and asserts chips, order, and downloaded/selected visibility for the census-reachable and census-unreachable scenarios. Serve the crate root over http and open `/ui-harness/census.html` (instructions on the page). |

Not touched: every other crate, `apps/tracker/`, protocol/catalog (the stats
endpoint and its counts-only shape are consumed as-is; no new fields anywhere).

## 2. Design decisions (the "why" in one pass)

- **Where the join lives.** `census` returns the raw counts
  (`{peers_online, counts: {profile_id → peers}}`); the JOIN against display
  names happens in `list_models` — the command that owns the loaded, verified
  catalog — so the picker's chip text AND card order are computed once, in
  Rust, fully unit-tested. The webview renders both verbatim and recomputes
  nothing (no JS/Rust mirror that can drift). This is why no
  `allow(dead_code)` was needed: the contract functions are production code.
- **Cadence.** Dedicated 30 s census poll (the picker piggybacks the join on
  the existing 15 s catalog poll; after each census outcome the UI re-pulls
  the catalog, which is one extra small signed-envelope fetch per 30 s). The
  picker NEVER waits on the census: `list_models` joins whatever cache exists.
- **Honesty states.** Reachable + hosting peers → exact count; reachable +
  zero → "first peer"; unreachable/malformed/stale-dropped → "census n/a"
  (never a zero). A failed poll DROPS the last-good cache so stale counts
  can't masquerade as live. `parse_census_counts` is strict: any missing or
  non-numeric field is an error, so a garbage body can never render as
  "nobody hosting".
- **Order.** v0.2.19 hardware shortlist stays PRIMARY: recommended cards
  first, and census counts never change recommended membership (that is
  hardware-scored in `list_models`, untouched). Within that: fit tier
  (gpu_full > gpu_partial > ram_only > unknown) → more live peers first
  (equal for every card while census is n/a — no invented counts reorder
  anything) → display name → stable catalog order. Downloaded/selected cards
  remain always-visible exactly as before (same filter, new order).
- **No new copy/promises**: chips only state counts; no advice text added.
- **Telemetry**: one `warn` event `census.error` on failure only (counts-only
  fields; no per-poll success spam). No frozen-vocabulary conflict today
  (ADR-033's redaction suite is not yet implemented; when it lands this is
  just one more numeric-only event name).

## 3. Exact commands and outcomes

| Command | Outcome |
|---|---|
| `cargo fmt --all --check` (after `cargo fmt --all`) | CLEAN |
| `cargo clippy --workspace --all-targets -- -D warnings` | CLEAN (0 warnings/errors) |
| `TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo clippy -p modelswarm-desktop --all-targets --features tauri-shell -- -D warnings` | CLEAN |
| `cargo test --workspace` | 492 passed / 0 failed / 6 ignored, 69 suites ok |
| `TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo test -p modelswarm-desktop --features tauri-shell` | 18 passed / 0 failed / 0 ignored (was 13; +5 census tests) |
| Rendered check (below) | 6/6 PASS in headless Edge against the real `ui/index.html` |
| `curl https://modelswarm.deepflux.space/api/v1/stats` | `{"peersOnline":2,"models":[{"profileId":"msp1:fc5a30ae…","peers":2}],"downloads":{…}}` — live shape matches `parse_census_counts` exactly |

Baseline note: the task brief quoted the workspace baseline as 469/0/6; current
`main` is 492/0/6 because commit 83eb84a (resource sampler) added 23 tests
after that baseline was recorded — verified by running `cargo test -p
modelswarm-resource` alone (23 passed). This change adds zero default-run
tests (`app.rs` compiles only under `tauri-shell`); all +5 land in the
tauri-shell-gated desktop suite (13 → 18).

## 4. Test evidence

Rust (`-p modelswarm-desktop --features tauri-shell`), 5 new tests:

- `census_join_builds_honest_chips` — stats + catalog fixtures → exact chips:
  "2 peers" / "1 peer" (singular) / "5 peers" / "first peer" for absent
  profiles.
- `census_unreachable_is_na_never_first_peer` — counts `None` → every chip
  "census n/a" (unreachable ≠ nobody-hosting) and the order falls back to
  shortlist → tier → name.
- `census_order_is_shortlist_primary_then_tier_peers_name` — a crowded
  non-shortlist card (9 live peers) still sorts after every recommended card;
  within the gpu_full tier peers decide (3 before 1); ram_only before unknown
  tier; exact expected id sequence asserted.
- `census_order_ties_fall_to_name_then_catalog_order` — same tier + same
  count → name ascending; identical names → stable catalog order.
- `parse_census_counts_is_strict` — valid body parses; empty-but-valid
  (`peersOnline 0, models []`) is the honest zero state; missing peersOnline,
  missing models, fractional online count, stringly peers, missing peers →
  errors.

Rendered (committed harness `ui-harness/census.html`, run headlessly via
`msedge --headless=new --screenshot` against a local server serving the crate
root; the verification copy additionally beaconed its results to a local
recording server, log quoted in the handoff message):

- reachable: order `[bb, aa, cc, dd]` kept verbatim (bb 3 peers before aa 0
  peers inside the gpu_full tier; downloaded non-shortlist dd visible; ee
  hidden); chips `{bb: "3 peers", dd: "1 peer", aa: "first peer", cc: "first
  peer"}`; shortlist filter untouched ("show all 5 models (1 more)").
- unreachable: every chip "census n/a"; order falls back to shortlist → tier
  → name; dd still visible.

## 5. Assumptions

- `/api/v1/stats` stays public/no-auth with this counts-only shape (it is the
  website's own census source; verified live 2026-10-12). The `downloads`
  object is ignored.
- `crates/modelswarm-tracker-api` is Tracker-Engineer-owned and has no stats
  method, so `census` issues the public GET itself via reqwest against the
  SAME configured tracker base URL. If the tracker-api grows a `stats()`, the
  command should switch to it (one-call change).
- Stats rows for profile ids not in the local catalog are harmless: the join
  only reads counts for catalog profiles (unknown ids can't be hosted anyway).
- The render contract keeps `census_chip` as the single chip source; the UI's
  `?? "census n/a"` fallback only guards a missing key (fail-closed display).

## 6. Unresolved risks

- The census chips arrive with the catalog response, so a census outcome is
  visible within one catalog re-pull (immediately after each 30 s census tick;
  otherwise ≤15 s). Acceptable for a 30 s snapshot; noted, not fixed.
- `census` clears the cache on a single failed attempt (no retry). A one-poll
  blip reads "census n/a" for ≤30 s — honest by design, but if the tracker
  gets flaky under rate limiting (stats shares the 120 req/min/IP class with
  /health + /catalog), a one-retry pattern like `list_models`' would smooth it.
- Headless-Edge `--dump-dom` returned empty on this machine (screenshots
  work), so the rendered check used the beacon trick on a throwaway copy of
  the harness; the committed harness itself is plain-open-in-browser.
- UI-study finding A5 (innerHTML rebuild can swallow an in-flight click) now
  also fires on census ticks (every 30 s, offset from the 15 s catalog tick);
  same exposure as today's 15 s rebuild, unchanged scope here.

## 7. Suggested next task for the integrator

Merge as-is (single-crate change; reviewers: Protocol + Test and Release per
the map). Natural follow-ups, none blocking: (a) surface `peers_online` as a
one-line swarm total next to the census chips (data already returned by
`census`), (b) the A5 click-safety fix (diff-patch the card DOM instead of
innerHTML rebuild) which would also cover census re-renders, (c) switch the
`census` command to `TrackerClient::stats()` if/ADR when the tracker-api
grows one.

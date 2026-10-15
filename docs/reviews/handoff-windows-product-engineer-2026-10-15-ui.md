# Handoff — Windows Product Engineer, 2026-10-15 (nav-menu UI reorg)

Session record for the owner directive (verbatim, 2026-10-15): "all switches on
by default, all information that the user doesn't need should go inside a
navigation bar menu, file, settings, help, le'ts clean the UI to something more
polished while simple, just reorganise so the main page is super simple with
just enough for the user to operate, everything else goes into menus on the
navigation bar." — executed in my owned paths (`crates/modelswarm-desktop`
ui/ + the minimal app-adjacent accompaniments; zero IPC handler changes).

## 1. Changed files and why

| File | Why |
|---|---|
| `crates/modelswarm-desktop/ui/index.html` | THE REORG. A navigation bar (File / Settings / Help dropdown menus — webview HTML/CSS/JS, not a native menu bar) now fronts everything the user does not need to operate. The MAIN page keeps exactly the operating surface: the Host strip (hosting switch + current model + a compact `<select>` picker + the download progress bar) and the chat box with its permanent family-verification disclosure (`verified-chat-line`) directly under it; a compact status strip (bottom) carries version, `peers online: N · K hosted profiles` (from the existing 30 s `census` poll return), and a relay chip shown only while a live reservation holds a circuit address. ADR-036's serving-side `carriage-line` STAYS on the main page under the hosting switch (normative contract: "always visible while active" — a menu must never hide a standing consent disclosure; hidden only while nothing is carried, exactly as before). Everything else moved into a single right-side panel sheet opened by menu items (the navbar stays reachable so panels can be switched without closing): File → Clear conversation (direct action; same `clear_chat` path) + Exit (`getCurrentWindow().close()` — no new IPC handler); Settings → Models browser (cards + HF request picker), Tracker/engine/device approval, Swarm pairing & content carriage toggles, Chat temperature, Cloud assistant opt-out; Help → Setup assistant (ADR-037 panel intact, own section/input/namespace), Privacy disclosures, Resource monitor, Census detail (NEW table: per-profile live peers from the same census poll), Diagnostics & local API (degraded chips, family line + probe, hosting detail incl. the exact ModelProfileId, peer roster/NAT paths, curl snippet), updater status line (`update-note`, verbatim, role=status), About. Every mandated id and invoke name survives verbatim; the retired `verified-chat-switch` / `set_family_verified_chat` surfaces remain absent (CI negative control); NO new user-facing switches (still exactly four: hosting, pairing, carriage, assistant). One stale string retired in passing: the old Hosting note claimed "remote peers can't be served until the Phase F listener ships" — false since the F2B listener+relay adoption (364609e); the Diagnostics panel now states the real boundary (direct QUIC first, honest relay fallback, no universal NAT-traversal claim). |
| `crates/modelswarm-desktop/ui-harness/adr036-carriage.html` | Lockstep: opens Settings → pairing & carriage BEHAVIORALLY (menu button click → menu item click) before toggle assertions; keeps the §3.1 carriage disclosure checks on the MAIN page (adds a main-bounds geometry check proving it renders inside main without any menu open); replaces the stale Scenario-A "election toggle present, default OFF / disclosure hidden" checks — those predated the 2026-10-15 always-on retirement and contradicted the harness's own Scenario-D absence check — with the always-on truth (retired toggle ABSENT + requester disclosure ALWAYS visible via the null-disclosure fallback path). |
| `crates/modelswarm-desktop/ui-harness/assistant.html` | Lockstep: opens Help → Setup assistant (and Settings → Cloud assistant for the opt-out) behaviorally before asserting; disclosure/verbatim/blocking/plain-text/cap/chip/facts/namespace/W4 assertions unchanged. |
| `crates/modelswarm-desktop/ui-harness/census.html` | Lockstep: opens Settings → Models behaviorally before reading the card browser; NEW checks pin the compact picker (`#model-select`): full canonical catalog order, census + downloaded suffixes, placeholder (nothing selected) — the peers-first filter still governs only the card browser. Also adopts the harness family's ALL PASS title convention (it never set `document.title`). |
| `crates/modelswarm-desktop/ui-harness/monitor.html` | Lockstep: opens Help → Resource monitor behaviorally; geometry re-based for the single-column layout — the old right-column scroll-growth baseline is replaced by (a) the bare main page must not clip at 1152×648 (scrollH == clientH) and (b) the open panel must fit the sheet (bottom ≤ 618, right ≤ 1152) with the sheet scrolling instead of clipping. Values/aria/n/a assertions unchanged. Title convention added. |
| `crates/modelswarm-desktop/ui-harness/pairing-chat.html` | Lockstep: pairing toggle + notes opened from Settings → pairing & carriage; temperature control opened from Settings → Chat temperature (the "pairing/verification stays greedy" determinism note moved with it); Clear conversation driven as a real File-menu action (menu open → item click); the remote-serve chip pin updated "off (swarm pairing off in Setup)" → "…in Settings)" (the toggle's home changed); family line read after opening Diagnostics; geometry re-based to the single-column main (no clip, transcript ≥ 80 px floor, panel bounded). |
| `crates/modelswarm-desktop/ui-harness/updater.html` | Lockstep: the status note now lives as the Help menu's non-interactive status item; the harness clicks the Help dropdown open and asserts it fits 1152×648 + the navbar fits + conn-note stays live, alongside the verbatim-line checks (all eight phases + missing payload). Title convention added. |
| `crates/modelswarm-desktop/capabilities/default.json` | File → Exit needs `core:window:allow-close` — `core:default`'s window set carries getters only (verified against the generated schema). Scope: one window command on the existing `main` window; no Rust handler added. |
| `crates/modelswarm-desktop/gen/schemas/capabilities.json` | Regenerated by `tauri_build` from the capability addition (committed generated artifact, as tracked). |
| `docs/reviews/ui-2026-10-15/*.png` (9 files) | Rendered-geometry evidence (the owner's standing rule): main page, each menu open, representative panels at 1152×648, plus main + Settings menu at 800×600. |

Not touched: `src/app.rs`, `src/main.rs`, node/transport/relay/tracker/protocol —
the reorg is pure webview + capability accompaniment; every IPC name the UI
calls already existed.

## 2. Exact commands and outcomes

- `cargo fmt --all --check` — clean (exit 0).
- `TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo clippy -p modelswarm-desktop --features tauri-shell --all-targets -- -D warnings` — clean (exit 0).
- `TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo clippy --workspace --all-targets -- -D warnings` — clean (exit 0; extra safety, no Rust source changed).
- `TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo test -p modelswarm-desktop --bin modelswarm-desktop --features tauri-shell` — **72 passed / 0 failed / 0 ignored**.
- `TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo test --workspace` — **622 passed / 0 failed / 0 ignored** (default feature set).
- Harness family (real browser): `python -m http.server 4817` from `crates/modelswarm-desktop`, each harness driven in **headless Edge 155 over CDP** (Node 22 WebSocket; `--dump-dom` is dead on this host, per standing note; the CDP driver + jsdom runner live in `%TEMP%\msp-uireorg\`, outside the repo, per prior practice):

| Harness | Result |
|---|---|
| `ui-harness/adr036-carriage.html` | **ALL PASS 18/18** |
| `ui-harness/assistant.html` | **ALL PASS 24/24** |
| `ui-harness/census.html` | **ALL PASS 10/10** |
| `ui-harness/pairing-chat.html` | **ALL PASS 15/15** |
| `ui-harness/monitor.html` | **ALL PASS 11/11** |
| `ui-harness/updater.html` | **ALL PASS 17/17** |
| **Total** | **95/95** |

- jsdom behavioral leg (jsdom@25, `%TEMP%\msp-uireorg\jsdom-run.mjs`, over the REAL `ui/index.html`): **29/29 PASS** — nav structure, negative control (retired surfaces absent), main-page section count (exactly 2), always-on `verified-chat-line`, carriage disclosure verbatim on main, relay chip, census strip + table (ok and unreachable), compact picker (options/suffixes/placeholder/`select_model` write-through), File → Clear conversation, the full ADR-037 assistant flow (disclosure verbatim, first-call block, ack write, markup-hostile plain-text render, chip, W4 isolation, event namespace, Settings opt-out write).

## 3. Test evidence — rendered geometry (the standing rule)

Real layout measured in headless Edge (CDP `Emulation.setDeviceMetricsOverride`,
same rich stub as the screenshots; app shell = 40 px navbar + main + 30 px strip):

At **1152×648** (the default window):
- document `scrollWidth` 1152 / `scrollHeight` 648 — **zero page overflow**; navbar fits, status strip fits.
- `main` clientHeight 578 == scrollHeight 578 — nothing clipped behind `overflow:hidden`.
- Host strip 96 px (switch + picker + browse + progress in one wrapped row); chat section 452 px; transcript 308 px (≥ 80 floor).
- ADR-036 disclosures render with real height ON MAIN: `carriage-line` 17 px, `verified-chat-line` 33 px (both visible).
- Help dropdown: 300×223 px at (328, 36) — inside the viewport; sheet: top 40 / right 1152 / bottom 618 / width 560 — right-docked under the navbar, above the strip; navbar and chat stay visible while a panel is open.

At **800×600** (small window):
- `main` clientHeight 530 == scrollHeight 530 — no clipping; navbar + strip fit; host strip wraps to 113 px; chat 393 px; transcript 249 px; both disclosures still visible (33 px each). The sheet goes full-width (`100vw`) under the 900 px media query.

Screenshots (committed, `docs/reviews/ui-2026-10-15/`): `01-main-1152x648`,
`02-file-menu`, `03-settings-menu`, `04-help-menu`, `05-panel-models`,
`06-panel-assistant`, `07-panel-diagnostics` (all 1152×648), `08-main-800x600`,
`09-settings-menu-800x600`.

One regression caught and fixed during the reorg by the updater harness itself:
the first draft of `refresh()` dropped the `updater.line` verbatim render (9/17
FAIL) — restored with its comment; 17/17 after.

## 4. Assumptions made

1. "Compact picker" = a `<select>` with the full canonical catalog (census +
   downloaded suffixes, selected preselected, "choose a model…" placeholder) +
   the "browse all models…" button opening the full browser in Settings; the
   peers-first visibility filter governs only the card browser (unchanged
   behavior, pinned by census.html).
2. The status strip's "peers online / census count" renders the EXISTING
   `census` IPC return (`peers_online`, per-profile `counts`) — the same public
   stats the website census reads; no new IPC, no mirror logic (a failed poll
   degrades strip + table to census n/a, never stale counts).
3. File → Exit = closing the main window (the daemon and supervised engines
   live in this process; the winjob kill-on-close object tears the sidecar
   down). Implemented via the Tauri window bridge + one capability permission,
   not a new Rust handler.
4. The ADR-036 §3.1 serving `carriage-line` and the always-on requester
   `verified-chat-line` must NOT go into menus (normative: "always visible
   while active … no modal"); both remain on the main page. The updater STATUS
   is a passive informational line (auto-update installs passively), so Help is
   its honest home per the directive.
5. The stale Phase-F hosting note was corrected (truth-in-UI duty) rather than
   relocated verbatim; wording mirrors the shipped F2B behavior (364609e).
6. Screenshot PNGs committed under `docs/reviews/ui-2026-10-15/` as durable
   review evidence (~560 KB total); no prior precedent existed either way.

## 5. Unresolved risks

1. **No visual human pass yet.** Geometry + behavior are machine-verified
   (95/95 harness + 29/29 jsdom + the numbers above), but "polished" is an
   aesthetic judgment — the owner should eyeball the nine screenshots (or the
   app) and rule on spacing/wording.
2. **In-app menu ergonomics on very small windows** (< ~700 px wide): the
   navbar wraps nothing (menus stay one row; verified no overflow at 800×600),
   but I did not measure below 800 px — the Tauri window is resizable; a
   min-size could be set in `tauri.conf.json` if the owner wants a floor.
3. **Panel sheet vs. modal:** the assistant lives in a right sheet; a user who
   wants side-by-side chat + assistant gets it (navbar stays reachable), but a
   reviewer may prefer a modal for the first-run disclosure. Defer to owner
   taste.
4. The `e2e-windows` integration suite was not re-run this session (no Rust
   behavior changed; CI runs it on the installer path).

## 6. Suggested next task

Owner visual pass over `docs/reviews/ui-2026-10-15/*.png` (or a live build);
then, if approved, the T/R seat re-runs the harness family at the v0.2.25 train
(standing practice) — the six harness pages are self-checking (ALL PASS titles)
and need only `python -m http.server 4817` from the crate root. Natural
follow-ups in my seat afterwards: (a) retire the superseded automatic engage
ladder per the ADR-036 revision note (reviewed with the relay batch), and
(b) consider a `minimumSize` for the window if the small-window pass bothers
the owner.

# Desktop model start/stop/switch — UI/UX/HID study

Date: 2026-10-06 · Scope: `crates/modelswarm-desktop` (`ui/index.html`, `src/app.rs`)
· Trigger: owner report — "if I want to stop a model and start another one, not
very clear where to press".

## Method

1. Full code walk of the IPC surface (`app.rs`) and the single-file UI
   (`index.html`, 342 lines).
2. Live audit of the rendered UI: `index.html` served locally with a stubbed
   `window.__TAURI__` bridge (canned `invoke` responses driving hosting
   ON/OFF), rendered in Chromium at the app's **default window 1152×648**
   (`tauri.conf.json`). Screenshots + vision analysis of idle and
   hosting-ON states; scripted clicks, keyboard tests, and
   `getBoundingClientRect` measurements.
3. Comparable-pattern check (LM Studio verified against lmstudio.ai docs;
   Ollama/Jan/GPT4All from their established model-list behavior).

## Verdict in one paragraph

The user is right, and the root cause is structural: **model switching is not
a supported action.** While hosting is ON, clicking another model card is a
*silent dead control* (the frontend guard returns before the backend's
"stop hosting before switching models" error can ever surface). Stopping is
split across two low-affordance controls in two different sections (a 44×24
toggle in Hosting, a ghost "stop hosting" button miscategorized under Setup),
there is no switch instruction anywhere in the UI, the running model is
identified by hash rather than name, and — compounding everything — **the
page overflows the default window** (751 px of content on a 648 px viewport),
so the chat test box is below the fold.

## Findings (all verified live unless noted)

### A. The switch journey

| # | Finding | Evidence |
|---|---|---|
| A1 | **Dead click on cards while hosting.** `selectModel` returns silently on `busy \|\| hostingOn` (index.html:230). No invoke reaches the backend, no error, no tooltip; card keeps `cursor:pointer`, no `aria-disabled`. Backend already returns a good message ("stop hosting before switching models", app.rs:816) that the UI never shows. | Scripted click on 2nd card with hosting ON: invoke log shows only background `get_status`/`lookup_peers`; zero UI response. |
| A2 | **No switch instruction anywhere.** The only hint ("pick a model above — the swarm starts on selection") lives in the *Peers & NAT table*, right column, and only appears after hosting is off. Vision pass of both screenshots: no text explains how to change models. | Screenshots + vision analysis. |
| A3 | **Backend cannot switch atomically.** `select_model` refuses while the node runs (app.rs:814-816). A switch must be stop → select → start: 3 actions, 2 screen regions, seconds-long gap, one silent trap. | Code walk. |
| A4 | **Same silent guard during downloads and chats.** One global `busy` flag (index.html:198) also blocks card clicks while another model downloads or a chat reply is in flight — silently. | Code walk (index.html:230, 328). |
| A5 | **Card DOM rebuilt every 15 s.** `refreshModels` replaces all cards via `innerHTML` — can swallow an in-flight click and drops keyboard focus. | Code walk (index.html:214, 339). |

### B. Start/stop affordances

| # | Finding | Evidence |
|---|---|---|
| B1 | **Two stop controls, asymmetric.** Toggle switch (Hosting, left) and ghost "stop hosting" (Setup, right). Both call the same `set_hosting(on:false)`. The toggle is *also* a hidden start control (ON restarts the remembered profile) — undiscoverable. | Code walk (index.html:151, 296, 310). |
| B2 | **"stop hosting" is in the wrong section.** Setup holds tracker URL/engine/approval — configuration. A runtime lifecycle action sits there as the least prominent (ghost) control. | Screenshot + vision: "low-contrast gray button … easy to miss". |
| B3 | **Running model identified by hash only.** Hosting panel shows `msp1:9f2c…`; the human-readable name exists only on the card, marked solely by a green border. With hosting off, the green "selected" border persists (it means "last picked", not "running") — state conflation. | Screenshot + vision. |
| B4 | **No transitional states; stale state after stop.** Binary ON/OFF polled every 2 s. After stopping, `dl-label` still reads "swarm running · 127.0.0.1:11435" and the Local API panel still advertises the dead gateway. `busy` disables nothing visually. | Live: stop via ghost button → stale labels measured. |
| B5 | **Chat not disabled when off.** Input stays enabled with unchanged placeholder; the failure arrives as an after-the-fact error in the transcript. | Live test. |
| B6 | **Page overflows the default window.** scrollHeight 751 vs viewport 648 at 1152×648. The right column (Setup+Peers+API+Degraded+Privacy) has no height cap; chat input/send sit at y≈656 — **below the fold**. The CSS comment claims "designed to fit the 1152×648 default window without page scroll" (index.html:21) — false today. | `document.documentElement.scrollHeight` = 751; chat input rect y=656. |
| B7 | **Degraded chips are permanently lit.** `not hosting` is amber whenever hosting is off and `direct-connect fail` is amber whenever hosting is on — exactly one warning is always displayed in normal operation. Alarm fatigue by construction; state encoded in color alone. | Code walk (index.html:272-274). |

### C. HID / accessibility

| # | Finding | Evidence |
|---|---|---|
| C1 | **Hosting switch is keyboard-inoperable.** `tabindex=0`, `role=switch`, but no keydown handler — Space and Enter do nothing. Keyboard users cannot operate the primary lifecycle control. | Live: focus + Space/Enter → `aria-checked` unchanged, no invoke. |
| C2 | **No visible focus indicator.** `outline: none` on focused switch; no `:focus-visible` styles anywhere. | Live measurement. |
| C3 | **Switch hit target 44×24 px** — meets WCAG 2.5.8 minimum (24) but far below the 44 px recommended target for a primary control; no padding to enlarge the hit area. | `getBoundingClientRect`: 44×24. Stop button 99×31. |
| C4 | **ARIA model mismatch.** Cards are `role=option` in a `listbox` but behave as command buttons (immediate action); `aria-selected` persists after stop; no `aria-disabled` when guarded. | Live snapshot. |
| C5 | **Download progress not exposed to AT.** `aria-hidden` bar; the only accessible signal is the free-text label. | Code walk (index.html:106). |
| C6 | **Window/title/tray carry no state.** Static title; no tray; closing the window silently ends hosting. | `tauri.conf.json` + main.rs walk. |

## Comparable patterns (why users expect one-click switching)

- **LM Studio** (verified, lmstudio.ai docs — "Idle TTL and Auto-Evict"):
  loading a new model auto-unloads the previous one *by design* "useful when
  you want to switch between" models; loaded models get per-model unload
  controls in the sidebar.
- **Ollama / Jan / GPT4All**: model list with explicit per-model
  run/stop; picking a different model swaps the active one in a single action.

Every comparable treats *switch = load the new one*; none require an explicit
stop first. MSP's stop-first flow is the outlier, and its guard makes the
outlier path silent.

## Proposals

### P0-1 · Make switching one action (backend)

In `select_model` (app.rs), when a node is running and the target profile
differs: **keep the old node serving while the new artifact downloads**
(`ensure_artifact` does not need the engine stopped), then tear down the old
node and start the new one. Downtime = engine restart only (~1–3 s); a
required download produces zero downtime. Same-profile click while running =
treated as "show state" (no restart). Return a `switching` state the UI can
render. Owner rule kept intact: one profile, one advertised slot at a time —
the swap is sequential, never two swarms.

### P0-2 · Card-level affordances (frontend)

Each model card carries an explicit action, not just whole-card click:

```
┌─────────────────────────────┐   ┌─────────────────────────────┐
│ SmolLM2-135M-Instruct       │   │ SmolLM2-360M-Instruct       │
│ Q8_0 · 105 MB · ≈0.5 GB RAM │   │ Q4_K_M · 341 MB · ≈0.6 GB   │
│ ● hosting · gateway :11435  │   │ downloaded                  │
│ [ Stop ]   (card = running) │   │ [ Start ]  ← primary button │
└─────────────────────────────┘   └─────────────────────────────┘
┌─────────────────────────────┐
│ Qwen2.5-0.5B-Instruct       │   states: idle · downloading % ·
│ Q4_K_M · 491 MB · ≈0.8 GB   │   starting… · hosting(●) ·      │
│ not downloaded              │   switching… · stopping…        │
│ [ Download & start ]        │
└─────────────────────────────┘
```

- Running card: green ● + model name + explicit **Stop** button; clicking the
  card body is a no-op or opens details (never silent).
- Idle card: **Start** / **Download & start** button (size on disk shown).
- During any transition: spinner + word on the affected card, all other
  action buttons disabled *visually* (`disabled` attr, not a silent guard).
- Whole-card click may remain a shortcut for Start, but the visible button is
  the affordance that answers "where do I press".

### P0-3 · One canonical lifecycle surface; fix placement

- Remove the ghost "stop hosting" from Setup (Setup = configuration only).
- Hosting panel becomes the single status home: **model display name +
  quant**, profile id as secondary line, gateway, and the toggle — which
  becomes a real `<button role="switch">` with Space/Enter support, ≥32 px
  hit height, and a `:focus-visible` ring.
- Clear stale state on stop (status line, Local API endpoint) — B4.

### P0-4 · Make the page fit its own default window

Cap the right column / fold secondary panels (Peers, Local API, Degraded,
Privacy) so `main` never exceeds 648 px at 1152 px wide; the chat test box
must be fully visible. Add a rendered-geometry check (screenshot at
1152×648, `scrollHeight ≤ viewport`) to the release checklist — this is the
second time an unverified layout claim shipped (see `verify-rendered-layout`
memory: the squished hero).

### P1 · State machine + events

- UI lifecycle words: `idle · downloading n% · starting · hosting ·
  switching · stopping`, shown in the Hosting panel and on cards.
- Emit a `node_state` Tauri event from the backend instead of relying on the
  2 s poll for transitions (download events already exist as the pattern).
- Chat input disabled when off, placeholder "start hosting to chat".
- `list_models` refresh patches card state in place instead of `innerHTML`
  replacement (A5).

### P1 · HID/accessibility fixes

- Switch → `<button role="switch">` (keyboard operable, focus-visible).
- Cards → `role="button"` (or true buttons) with `aria-disabled` during
  transitions; drop the `listbox/option` fiction (C4).
- `role="progressbar"` + `aria-valuenow` on download (C5).
- Encode state in text as well as color: chips say what's wrong, not just
  turn amber; `not hosting` is a neutral state, not a degraded one; the
  permanent `direct-connect fail` amber becomes a one-time neutral "(known
  beta limit: no listener)" note (B7).

### P2 · Later

- Window title reflects hosting state ("ModelSwarm — hosting SmolLM2-135M");
  tray icon + close-confirmation while hosting (needs the daemon-composition
  decision — owner call, ties into Windows Product Engineer's Phase H work).
- Same-profile click on a running card could open model details (quant,
  digest, engine pin) — discovery without state change.

## Ownership / test notes (AGENTS.md)

- All changes live in `crates/modelswarm-desktop` → **Windows Product
  Engineer** owns; reviewers: Protocol Architect + Test and Release.
- The `select_model` swap sequence (P0-1) must keep the project rule: one
  advertised profile at a time — sequential stop→start, and the tracker
  registration for the old profile must be torn down before the new node
  registers (lease release path already exists via `set_hosting(false)`).
- Test evidence for the handoff: scripted UI journey (start → switch →
  stop) against the stubbed bridge (this study's harness, ~60 lines, can be
  checked in under `tests/`), plus the 1152×648 geometry gate, plus existing
  `cargo fmt/clippy/test` and IPC tests.

# ModelSwarm website — UI/UX/HID study (2026-10-05)

Scope: the public site served by `apps/tracker` at `modelswarm.deepflux.space`
(`app/page.tsx`, `app/layout.tsx`, `public/`). Method: full code read, live DOM
snapshot, rendered screenshots at 1440 px and 390 px, and runtime probes
(favicon, health, artifact HEAD checks, paint timings). No production state was
mutated (no download clicks).

## Measured baseline

- DOMContentLoaded 107 ms, first contentful paint 180 ms, total transfer
  ≈ 107 KB **excluding** `/hero.png` (2,073,138 bytes PNG on disk).
- Page height 5,665 px at 390 px viewport (single long scroll, no in-page nav).
- `GET /favicon.ico` → **404**; no `link[rel=icon]`, no touch icon, no
  `theme-color`.
- No `<h1>` anywhere (the title lives inside the hero bitmap).
- Live page advertises v0.2.1; `HEAD /downloads/ModelSwarm-Setup-0.2.2-windows-x64.exe`
  → **404**. Repo HEAD is v0.2.2 ("client had no TLS backend — catalog/downloads
  unreachable over https", commit b70e60d).
- Tracker API healthy (`/api/v1/health` → ok), census + counters working,
  chat iframe renders with visitor-locale default (observed Chinese in a
  zh-locale browser, with an in-widget English toggle).

## What already works well

- Honest disclosure copy (unsigned-build warning, content-blind hub claims
  tied to ADRs, "internal test" labeling) — matches the project's
  performance-honesty ethos; keep this voice.
- `aria-label`s on download links exclude the live counter from the accessible
  name; model census is `aria-live="polite"`; iframe has a real `title`;
  sections use `aria-labelledby`; `<html lang="en">` and full OG/Twitter card.
- Semantic download route (counted, strictly-shaped filenames), `width`/`height`
  on the hero (no CLS), dark `color-scheme` declared, links keep default focus
  outlines.

---

## P0 — correctness & trust (do first)

### 1. The site serves the known-broken 0.2.1 client
Evidence: page constants pin 0.2.1; no 0.2.2 artifact exists on the server
(404). v0.2.2 exists precisely because 0.2.1 "had no TLS backend —
catalog/downloads unreachable over https". Anyone downloading today gets a
client that cannot talk to this https tracker. This contradicts the site's own
"same version number on every OS = same build" promise.
Fix: publish 0.2.2 artifacts for all four OSes (per the all-OS release rule),
update the constants, and add the missing guard below.

### 2. Release constants have no single source of truth
`DOWNLOADS`/`DOWNLOAD` in `page.tsx` are hand-typed; the SHA-256 is hand-pasted
(Windows-only). Every release invites drift (this is how #1 happened).
Fix: generate a `lib/site-release.ts` (or JSON) from the release script that
also drops installers into `public/downloads/`, and add a CI/deploy check:
page version == tag, all four files exist, inline hashes == SHA256SUMS.txt.
Optionally render all four hashes instead of Windows-only.

### 3. "TRACKER LIVE" is a static claim, not a check
The badge is hardcoded text while the sibling badge *is* fetched. A down
tracker would still say LIVE.
Fix: drive it from the same `/api/v1/health` fetch (ok → `TRACKER LIVE`,
failure → `TRACKER UNREACHABLE`), `aria-live="polite"`. This is cheap and is
exactly the honesty the project demands of itself elsewhere.

### 4. Download counters show fake "· ↓0" before/without data
Initial and error states both render `↓0`, which understates real downloads.
Fix: render nothing until the first successful fetch; on error, hide the
count span. Same section: `n === 1 ? ' online' : ' online'` is a dead ternary
— drop it. The census error fallback `— online` reads ambiguous; use
`census unavailable`.

---

## P1 — visual & interaction quick wins

### 5. Hero sizing is unbounded and shrinks wrong on mobile
`.hero { width: 75% }` of the viewport (it sits outside `.wrap`): on ultrawide
monitors it grows past the 1080 px content column and breaks alignment with
everything below; at 390 px it collapses to ~281 px, making the in-artwork
title lockup illegible and wasting 25% of the screen.
Fix: `width: min(75%, 1044px); margin-inline: auto;` and inside the existing
`@media (max-width: 720px)` set `width: 100%`.

### 6. The 2.0 MB hero PNG is the entire page weight
Everything else is ~107 KB. It is also reused as the OG image (slow unfurls).
Fix: export an optimized WebP/AVIF (~250–400 KB) with PNG fallback — or use
`next/image` with `priority` (it is the LCP element) — and produce a separate
≤ 300 KB 1200×630 OG card. Keep exact `width/height` to preserve zero CLS.

### 7. No icon identity in the tab
Favicon 404, no apple-touch-icon, no `theme-color`.
Fix: derive a 32/180/192 icon set from the hero's node-tree motif; add
`icons` to the metadata export and `<meta name="theme-color" content="#050a1f">`.

### 8. There is no `<h1>` and the smallest type is 11.5 px
The page's only heading text starts at `<h2>`; the title is pixels in a bitmap.
`.sum`/`.chat-note`/badges at 0.72–0.75 rem are below comfortable minimums,
especially on mobile (confirmed in the 390 px capture).
Fix: add a visually-hidden or small `<h1>ModelSwarm — decentralized
cooperative inference</h1>` (SEO + screen readers + outline completeness);
raise the type floor to 0.8 rem (12.8 px), body notes to 0.85 rem.

### 9. Four equal download chips = no primary action
The gradient `.btn` class exists but is never used; every platform link is an
equal ghost chip, so the #1 task (download for *my* OS) has zero visual
priority — worst on mobile where chips wrap awkwardly.
Fix: detect platform (`navigator.userAgentData.platform` with fallback) and
promote the matching chip to the filled primary button, keep the rest ghost;
add `min-height: 44px` and `display:inline-flex; align-items:center` to all
chips for tap targets. On the macOS chip, label "Apple Silicon" and add a
one-line note for Intel Macs (known gap) so nobody installs the wrong dmg.

### 10. Checksum flow is Windows-only inline
Only the Windows sha256 is shown; everyone else must open SHA256SUMS.txt.
Fix: `<details>` under the download card listing all four hashes with a
copy button, plus a per-OS verify one-liner
(`certutil -SHA256 <file>` / `sha256sum <file>` / `shasum -a 256 <file>`).
Reinforces the unsigned-build warning instead of just warning.

---

## P2 — structure & polish

### 11. Section order interrupts the narrative
Current: Download → Models → **Chat** → What it is → API. A first-time visitor
meets the chat embed before learning what the product is.
Fix: Download → What it is → Models → API → Chat → footer (chat as the
"join us" close), or at minimum move "What it is" above the census.

### 12. The 21-row API table is unthemed bulk in the middle of the page
Fix: wrap in `<details><summary>Tracker API (v1) — 21 endpoints</summary>` so
devs open it on demand; add `<th scope="col">` (Method+path / Purpose) — today
the table is `<td>`-only, so screen readers announce no header row.

### 13. Chat embed dominates the page and surprises on locale
640 px fixed height (540 px mobile) is a tall scroll-trap region; the widget
follows the *visitor's* locale (observed defaulting to Chinese with an English
toggle), which can read as "wrong site" to en visitors on shared machines.
Fix: reduce to ~460 px, note in the meta line "the room widget follows your
browser language — switch inside the chat", keep the lazy load.

### 14. Keyboard/focus and motion affordances
Default outlines are kept (good), but custom `:focus-visible` (2 px cyan
outline + offset) would match the theme. Hover-only `brightness` has no
`:active` state — add a subtle press state. `prefers-reduced-motion` is
currently unnecessary (no animations) — keep it that way.

### 15. Census rows: long ids and 0-state
`msp1:` ids at 0.72 rem wrap unevenly across rows (`word-break: break-all`).
Fix: 0.78–0.8 rem, `overflow-wrap: anywhere` with `max-width` + fade, and a
full-id copy-on-click. Consider a friendly all-zero state ("No swarms online
right now — run the client to become the first node") — an empty census is an
onboarding opportunity, not just "0 online".

### 16. Small code hygiene
- The table caption references class `visually-hidden` that is never defined
  (inline `position:absolute;left:-9999px` does the work) — define the class
  or drop it.
- Model rows are built with `innerHTML` string concatenation; data is
  own-API so risk is low, but `textContent` building is the right pattern.
- `setInterval(update, 30000)` runs in background tabs; gate on
  `document.visibilityState` or use `requestIdleCallback` scheduling.

---

## P3 — nice to have

- **Human /status page**: "status"/"catalog" nav links dump raw JSON. A tiny
  `/status` (tracker health, protocol version, peers online, download counts)
  would serve both humans and the nav, with raw JSON still linked.
- **In-page nav / scroll cue** for the 5,600 px mobile page (sticky section
  dots or a compact TOC under the tagline).
- **GitHub Releases link** near downloads for source builds.
- **Analytics**: none today — consistent with the privacy posture; if numbers
  are ever needed, the existing download counters already cover the one metric
  that matters. State the choice in the footer ("no analytics, no cookies").
- **`next/image`** adoption would solve #6 and future image sizing centrally.

## Suggested order of execution

1. P0 #1–#4 (publish 0.2.2 + single-source constants + real badges/counts) —
   a release-hygiene PR, no visual redesign needed.
2. P1 #5–#8 (hero sizing/weight, icons, h1 + type floor) — one small
   CSS/layout PR with before/after screenshots at 1440/390.
3. P1 #9–#10 (primary CTA + OS hint, checksum disclosure) — conversion PR.
4. P2 items as one polish PR; P3 opportunistically.

All changes stay inside `apps/tracker` web surface; none touch the msp-v1
contract, the content-blind boundary, or any owned path outside Tracker
Engineer scope (coordination per AGENTS.md if another agent implements).

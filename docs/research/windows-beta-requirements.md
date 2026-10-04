# Windows Beta Requirements Inventory (Phase G; recorded Phase A)

Owner: Windows Product Engineer · Packaging: ADR-008 · These requirements are
frozen now so Phases B–F build toward visible states, not afterthoughts.

## Installer (signed)

- NSIS `setup.exe` plus signed `.msi`; per-user install, no elevation unless
  proven necessary; clean uninstall incl. model files (user-consented).
- Code signing before any public artifact; updater signatures mandatory
  (ADR-008). Unsigned internal builds must be labeled as such.

## Application states that must be visible (never hidden)

- Hosting on/off; exact profile id (`msp1:…`); artifact verification state;
  serving health; **earned eligibility** (lease status, capacity class);
  current peers + NAT path type; execution mode per request; fallback events;
  measured speedup **or slowdown** (no theoretical numbers, ADR-013).
- Degraded states: tracker unreachable, ineligible, draining, updating,
  model mismatch, direct-connect failure.

## Privacy disclosures

- First-run: serving peers can read prompts they serve; cooperative modes
  deliver the prompt to the whole micro-swarm roster (mode + count shown
  pre-request); no telemetry by default; opt-in diagnostics export excludes
  prompt/completion content by schema.

## Beta exit gate (from the revision, Phase G)

Clean install, upgrade, model migration, offline launch, firewall recovery,
crash recovery, and uninstall all pass on supported Windows versions; every
performance claim in the UI traces to a reproducible experiment record.

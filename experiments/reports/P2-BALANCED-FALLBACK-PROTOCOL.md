# P2 BALANCED — pre-registered fallback protocol (ADR-045 / T/R R5)

**Written BEFORE any P2 engagement code merges** (2026-10-20). This is the
pre-registration R5 demands: the property test that gates the Balanced
tier's merge must implement THIS protocol verbatim — if the protocol needs
changing after results are seen, the change is recorded as an amendment
with the reason, never silently.

## The claim under test

> A Balanced turn is never slower than the same turn under Fast, beyond a
> pre-registered noise bound.

## Engagement gates (each produces an honest not-engaged reason; a turn
that does not pass EVERY gate runs the plain Fast path and labels itself)

1. `preset == balanced` (the persisted selection, captured at send time).
2. `temperature == 0` — the composed path's lossless `speculative_exact`
   contract is greedy-only (ADR-032 §3; a sampled turn cannot be
   bit-verified, so it MUST NOT engage). Reason: `temperature_not_greedy`.
3. The turn executes LOCALLY (no remote peer pulled this turn — the
   composed path is in-process by construction; a remote-served turn
   cannot engage). Reason: `turn_not_local`.
4. The capi seam is live: the running node's family engine source is the
   `verifydrafts-capi` production venue over the hash-verified bundle for
   the hosted profile. Reason: `capi_seam_unavailable`.
5. The engage PREDICTION says the composed path wins: measured acceptance
   EWMA (persisted, warm-up exempt) ≥ the break-even acceptance for the
   configured window under the frozen ADR-013 cost model, with the
   pass-2 structural caveats labeled (loopback evidence only —
   `experiments/reports/PASS-2-LOOPBACK-CAPI.md`). Until in-situ
   measurements exist, the prediction CONSERVATIVELY refuses (reason:
   `prediction_insufficient_evidence`) — engagement begins only after the
   first measured window validates the model.
6. Per-turn fallback: ANY composed-path failure or deadline miss before
   the first committed token falls back to the plain path, labeled
   `composed_failed_before_first_token` (msp-v1 §6 retry discipline).
   After the first committed token there is no fallback (the turn
   completes composed, slower if it must).

## The pre-registered bound

- **Baseline:** the FASTEST ELIGIBLE SINGLE HOST for that turn — on the
  local-leg comparison, the plain local gateway path measured on the SAME
  machine, same model, same prompt, interleaved A/B/A/B (never an average
  of peers; the house comparator rule).
- **Metric:** per-turn wall time, first-token-to-last (the user-visible
  span; TTFT reported alongside, never aggregated away).
- **Bound (fixed now, before data):** Balanced P50 ≤ Fast P50 × 1.05, and
  Balanced P95 ≤ Fast P95 × 1.15. A slower tail is tolerated only because
  the fallback gate (6) can straddle a retry; if P95 breaches, the
  engagement prediction's threshold is WRONG and engagement is disabled
  (the tier falls back globally, honestly labeled, until re-calibrated).
- **Repetitions:** ≥ 60 paired turns per arm per model profile (the
  pass-2 protocol's scale), ≥ 3 distinct prompt regimes (the filler
  continuation class + two real-prompt classes), greedy, temperature 0.
- **Decision rule:** both quantile bounds hold → PASS. Either breaches →
  FAIL: the composed engagement ships DISABLED (prediction refuses), the
  negative result is published as visibly as a win (the honesty gate).
- **Provenance recorded per run:** hardware (CPU/GPU/RAM), model profile
  id + quant, engine tag + canonical_build_hash, runtime build (git sha),
  prompt corpus id, network condition (loopback/LAN, load), repetitions,
  per-turn samples JSONL (the pass-2 durability pattern: incremental,
  resumable).

## Fixtures that land with the engagement code (R1)

Per-preset golden turn fixtures asserting the REAL engaged set: for
Balanced, `spec_engage = {engaged, reason}` rides every local reply; the
not-engaged reasons above are each exercised by a fixture (including the
pre-measurement conservative refusal `prediction_insufficient_evidence`);
Fast/Deep/Verified/Maximum fixtures assert `spec_engage` absent-or-not-
engaged per their definitions. No fixture may claim an engagement that
did not occur (S-6).

## Non-goals (unchanged)

No output-changing modes (P3 needs its own ADR); no wire changes
(`SessionOffer.preset` negotiation is the Protocol Architect's separate
contract work — deploy-ordered before any preset-carrying cooperative
session); no relaxation of the family-verification baseline.

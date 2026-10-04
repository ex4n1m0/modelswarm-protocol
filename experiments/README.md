# Experiments

Reproducible benchmark home. Spec: `docs/research/bench-harness-spec.md`;
schemas frozen in Phase A (ADR-013): `schemas/run-manifest.schema.json`,
`schemas/mode-result.schema.json`.

| Dir | Contents | In git? |
|---|---|---|
| `manifests/` | Run manifests pinning environment per cell | yes |
| `schemas/` | Frozen record schemas (changes need an ADR) | yes |
| `raw/` | Machine-written per-run records | gitignored when large; CI retains artifacts |
| `processed/` | Aggregates computed by script (never by hand) | yes (small) |
| `reports/` | Human reports citing raw paths + commands | yes |

Rules: fastest-eligible-single comparator; negative results mandatory; no
performance claim without a reproducible run-manifest chain; no simulated
acceptance in any user-facing path.

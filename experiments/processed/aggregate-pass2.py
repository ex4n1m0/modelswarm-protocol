#!/usr/bin/env python3
"""Aggregate 9.6 pass-2 engagement cells into one processed record.

Usage: python3 experiments/processed/aggregate-pass2.py \
           experiments/raw/PASS-2-ENGAGE-LOOPBACK-2026-10-10 \
           experiments/processed/PASS-2-ENGAGE-LOOPBACK-2026-10-10-summary.json

Reads every <raw>/<cell>/summary.json + decision-join.jsonl the harness
wrote (the harness computes per-cell aggregates; this script folds them
and counts engagement/guard/acceptance evidence from the join rows —
nothing is aggregated by hand).
"""
import json
import statistics
import sys
from pathlib import Path


def main() -> int:
    raw_root = Path(sys.argv[1])
    out_path = Path(sys.argv[2])
    cells = []
    joins = {}
    for summary_path in sorted(raw_root.glob("*/summary.json")):
        cell = summary_path.parent.name
        cells.append(json.loads(summary_path.read_text(encoding="utf-8")))
        rows = [
            json.loads(line)
            for line in (summary_path.parent / "decision-join.jsonl")
            .read_text(encoding="utf-8")
            .splitlines()
        ]
        joins[cell] = rows
    if not cells:
        print(f"no summaries under {raw_root}", file=sys.stderr)
        return 1
    labels = sorted({c["label"] for c in cells})

    per_cell = []
    for cell in cells:
        rows = joins[cell["cell"]]
        arms = []
        for arm in cell["arms"]:
            arm_rows = [r for r in rows if r["arm"] == arm["arm"]]
            # ENGAGED = a cooperative round actually ran on the wire. The
            # per-row loss_guard record exists exactly when round 1 ran
            # (gate-fallback rows never round); a rounds>1 discriminator
            # would MISS single-round engaged completions (window >=
            # target with full acceptance commits everything in round 1).
            engaged_attempts = [r for r in arm_rows if r.get("loss_guard")]
            engaged = [
                r for r in engaged_attempts if r["status"] == "Completed"
            ]
            guard_rows = [r for r in arm_rows if r.get("loss_guard")]
            guard_would_fire = [
                r for r in guard_rows if r["loss_guard"]["would_fire_at_default"]
            ]
            engaged_ratios = []
            for r in engaged:
                single_rows = [
                    s
                    for s in rows
                    if s["arm"] == "fastest-single"
                    and s["status"] == "Completed"
                    and s["prompt_id"] == r["prompt_id"]
                ]
                if single_rows:
                    single_best = min(s["realized_total_ms"] for s in single_rows)
                    engaged_ratios.append(r["realized_total_ms"] / single_best)
            accepted = [r["accepted_tokens"] for r in engaged]
            proposed = [r["proposed_tokens"] for r in engaged]
            pred = [r["predicted_ms"] for r in engaged if r.get("predicted_ms")]
            realized = [r["realized_total_ms"] for r in engaged]
            arms.append(
                {
                    "arm": arm["arm"],
                    "n": arm["n"],
                    "fallbacks": arm["fallbacks"],
                    "failures": arm["failures"],
                    "engaged_attempts": len(engaged_attempts),
                    "engaged": len(engaged),
                    "median_vs_fastest_single_all_rows": arm[
                        "median_vs_fastest_single"
                    ],
                    "engaged_median_vs_prompt_best_single": (
                        round(statistics.median(engaged_ratios), 3)
                        if engaged_ratios
                        else None
                    ),
                    "engaged_worst_vs_prompt_best_single": (
                        round(max(engaged_ratios), 3) if engaged_ratios else None
                    ),
                    "measured_acceptance_rate": (
                        round(sum(accepted) / sum(proposed), 3)
                        if sum(proposed) > 0
                        else None
                    ),
                    "engaged_predicted_ms_median": (
                        round(statistics.median(pred), 1) if pred else None
                    ),
                    "engaged_realized_ms_median": (
                        round(statistics.median(realized), 1) if realized else None
                    ),
                    "guard_would_fire_at_default": f"{len(guard_would_fire)}/{len(guard_rows)}",
                }
            )
        per_cell.append(
            {
                "cell": cell["cell"],
                "label": cell["label"],
                "regime": cell["regime"],
                "injected_delay_ms": cell["injected_delay_ms"],
                "fastest_single_median_ms": cell["fastest_single_median_ms"],
                "arms": arms,
            }
        )

    total_records = sum(len(rows) for rows in joins.values())
    engaged_total = sum(
        1
        for rows in joins.values()
        for r in rows
        if r.get("loss_guard") and r["status"] == "Completed"
    )
    fallback_total = sum(
        1 for rows in joins.values() for r in rows if r["status"] == "FellBackToSingle"
    )
    # Gated re-runs can legitimately have ZERO engaged rows (the wire-true
    # verification term blocks every engagement on HTTP-only cohorts —
    # ADR-032 §5 option A): the best/worst engaged ratios are then None,
    # published as the honest result instead of crashing the aggregate.
    ratios = [
        (a["engaged_median_vs_prompt_best_single"], c["cell"], a["arm"])
        for c in per_cell
        for a in c["arms"]
        if a["engaged_median_vs_prompt_best_single"] is not None
    ]
    worst_ratio = max(ratios) if ratios else None
    best_ratio = min(ratios) if ratios else None
    document = {
        "experiment": "9.6-pass2-engagement",
        "environment_labels": labels,
        "is_lan_claim": labels == ["lan-2machine-quic"],
        "cells": [c["cell"] for c in per_cell],
        "total_join_rows": total_records,
        "cooperative_rows_engaged": engaged_total,
        "cooperative_rows_fell_back": fallback_total,
        "best_engaged_median_vs_prompt_best_single": best_ratio,
        "worst_engaged_median_vs_prompt_best_single": worst_ratio,
        "cells_detail": per_cell,
        "notes": (
            "Engagement is real protocol-level acceptance over the "
            "TEST-ONLY synthetic executor: propose and verify are real "
            "requests over the real transport; match lengths are DIALED "
            "by the driver through the prefix-match seed family (classic "
            "geometric per-token model). Realized engaged ratios are vs "
            "the best independently-executed fastest-single completion "
            "of the SAME prompt in the same cell (the inviolable "
            "comparator). The rule-6 observed-loss guard was disabled "
            "for engaged-curve measurement (MSP_BENCH_LOSS_MULTIPLIER="
            "off); every engaged row records would_fire_at_default — "
            "the production guard would have aborted it at round 1. "
            "Slowdowns are as visible as speedups; no product "
            "performance claim is made from synthetic-executor runs."
        ),
    }
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {out_path} ({len(cells)} cells, {total_records} join rows)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

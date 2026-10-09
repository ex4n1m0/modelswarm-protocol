#!/usr/bin/env python3
"""Aggregate 9.6 pass-1 cell summaries into one processed record.

Usage: python3 experiments/processed/aggregate-pass1.py \
           experiments/raw/PASS-1-LOOPBACK-DRYRUN \
           experiments/processed/PASS-1-LOOPBACK-DRYRUN-summary.json

Reads every <raw>/<cell>/summary.json the harness wrote (the harness
computes per-cell aggregates; this script only folds them — nothing is
aggregated by hand).
"""
import json
import sys
from pathlib import Path


def main() -> int:
    raw_root = Path(sys.argv[1])
    out_path = Path(sys.argv[2])
    cells = []
    for summary_path in sorted(raw_root.glob("*/summary.json")):
        cells.append(json.loads(summary_path.read_text(encoding="utf-8")))
    if not cells:
        print(f"no summaries under {raw_root}", file=sys.stderr)
        return 1
    labels = {c["label"] for c in cells}
    total_records = 0
    for records_path in raw_root.glob("*/mode-results.jsonl"):
        total_records += sum(1 for _ in records_path.open(encoding="utf-8"))
    engaged_runs = sum(
        1
        for c in cells
        for a in c["arms"]
        for _ in range(a["n"] - a["fallbacks"] - a["failures"])
        if a["arm"] != "fastest-single"
    )
    fallback_runs = sum(a["fallbacks"] for c in cells for a in c["arms"])
    ratios = [
        (c["cell"], a["arm"], a["median_vs_fastest_single"])
        for c in cells
        for a in c["arms"]
        if a["median_vs_fastest_single"] is not None and a["arm"] != "fastest-single"
    ]
    worst = max(ratios, key=lambda r: r[2]) if ratios else None
    best = min(ratios, key=lambda r: r[2]) if ratios else None
    document = {
        "experiment": "9.6-pass-1-loopback-dryrun",
        "environment_label": sorted(labels),
        "is_lan_claim": False,
        "cells": [c["cell"] for c in cells],
        "total_mode_result_records": total_records,
        "cooperative_runs_completed": engaged_runs,
        "cooperative_runs_fell_back": fallback_runs,
        "best_median_vs_fastest_single": best,
        "worst_median_vs_fastest_single": worst,
        "cells_detail": cells,
        "notes": (
            "Every number is loopback + executor-seam injected delay "
            "(synthetic token executor behind the REAL QUIC serving "
            "bridge); NOT a LAN claim and NOT a product performance "
            "claim. Negative results are the point: cooperative arms "
            "either fell back (engage gate) or realized slower than the "
            "independently executed fastest single."
        ),
    }
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {out_path} ({len(cells)} cells, {total_records} records)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

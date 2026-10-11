#!/usr/bin/env python3
"""Aggregate the accuracy-divergence experiment families (2026-10-11).

Two input families:

1. Structural (loopback + LAN): one raw dir per environment containing
   <cell>/{accuracy-manifest.json, divergence-checks.jsonl,
   cohort-derank.json, agreement-ledger.json}. Produces per-cell
   detection matrices vs the manifest's injected ground truth, ledger
   states, timing medians, and the redundant-racing availability delta
   measured from the SAME completions (no extra requests).

2. Real-engine: ACCURACY-DIVERGENCE-REALENGINE-2026-10-11's jsonl files
   (matrix run, p1-long repeat characterization, fresh-pair probe,
   cross-machine cache-cold rows). Produces the cross-machine agreement
   matrix, within-process repeat stability, and the Vulkan control.

Script-computed only — ANALYSIS files cite these numbers, never the
reverse. Output: one summary JSON next to this script.

Usage:
  python aggregate-accuracy.py <raw-root> <summary-out>
      (raw-root = the experiments/raw directory)
"""

from __future__ import annotations

import json
import statistics
import sys
from collections import Counter, defaultdict
from pathlib import Path

STRUCTURAL_DIRS = [
    "ACCURACY-DIVERGENCE-LOOPBACK-2026-10-11",
    "ACCURACY-DIVERGENCE-LAN-2026-10-11",
]
REALENGINE_DIR = "ACCURACY-DIVERGENCE-REALENGINE-2026-10-11"


def median(values):
    return statistics.median(values) if values else None


def p90(values):
    if not values:
        return None
    ordered = sorted(values)
    idx = min(len(ordered) - 1, (len(ordered) - 1) * 9 // 10)
    return ordered[idx]


def aggregate_structural_cell(cell_dir: Path) -> dict:
    manifest = json.loads((cell_dir / "accuracy-manifest.json").read_text())
    rows = [
        json.loads(line)
        for line in (cell_dir / "divergence-checks.jsonl").read_text().splitlines()
    ]
    ledger = json.loads((cell_dir / "agreement-ledger.json").read_text())
    derank = json.loads((cell_dir / "cohort-derank.json").read_text())

    peer_rows = [r for r in rows if r["reference_kind"].startswith("peer:")]
    quorum_rows = [r for r in rows if r["reference_kind"] == "quorum-majority"]
    peer_diverge = [r for r in peer_rows if r["verdict"] == "diverge"]
    quorum_diverge = [r for r in quorum_rows if r["verdict"] == "diverge"]

    injected = manifest["pool"]
    # Ground truth: the pool's divergent indexes vs the ledger state, via
    # the cohort-derank suspended list (peer ids).
    suspended = derank["suspended_peers"]
    return {
        "cell": manifest["cell"],
        "environment": manifest["environment"],
        "harness_version": manifest["harness_version"],
        "executor": manifest["executor"],
        "pool": injected,
        "ground_truth_divergent_indexes": [p["index"] for p in injected if p["divergent"]],
        "checks": len(peer_rows) // max(1, len(injected) - 1),
        "rows": len(rows),
        "peer_reference": {
            "comparisons": len(peer_rows),
            "diverge": len(peer_diverge),
            "agree": len(peer_rows) - len(peer_diverge),
            "divergence_positions": sorted(
                {r["divergence_position"] for r in peer_diverge}
            ),
            "divergent_probe_count": len({r["probe_peer"] for r in peer_diverge}),
        },
        "quorum": {
            "comparisons": len(quorum_rows),
            "diverge": len(quorum_diverge),
            "divergent_probe_count": len({r["probe_peer"] for r in quorum_diverge}),
        },
        "ledger": {
            entry[0]: {
                "checks": entry[1]["checks"],
                "divergences": entry[1]["divergences"],
                "agreement_ewma": round(entry[1]["agreement_ewma"], 6),
                "suspended": entry[1]["divergences"] >= 1,
            }
            for entry in ledger
        },
        "derank": derank,
        "timing_ms": {
            "check_wall_median": round(median([r["check_wall_ms"] for r in rows]), 2),
            "check_wall_p90": round(p90([r["check_wall_ms"] for r in rows]), 2),
            "probe_total_median": round(median([r["probe_total_ms"] for r in peer_rows]), 2),
            "raced_min_median": round(
                median([r["raced_min_total_ms"] for r in peer_rows]), 2
            ),
        },
        # Redundant-racing availability from the SAME completions (no extra
        # requests): racing a request across two peers bounds completion at
        # min(t_ref, t_probe); the tail protection vs having bet on the
        # slower peer is max-min per check.
        "racing_tail_saved_ms_median": round(
            median(
                [
                    max(r["reference_total_ms"], r["probe_total_ms"])
                    - r["raced_min_total_ms"]
                    for r in peer_rows
                ]
            )
            or 0.0,
            2,
        ),
        "racing_min_ms_median": round(
            median([r["raced_min_total_ms"] for r in peer_rows]) or 0.0, 2
        ),
        "racing_max_ms_median": round(
            median(
                [max(r["reference_total_ms"], r["probe_total_ms"]) for r in peer_rows]
            )
            or 0.0,
            2,
        ),
        "suspended_detected": sorted(suspended),
    }


def aggregate_realengine(raw: Path) -> dict:
    re_dir = raw / REALENGINE_DIR
    summary: dict = {"record_type": "realengine-summary/v1"}

    # Matrix run (3 instances; within-process repeat + cross-process +
    # Vulkan control).
    matrix = [
        json.loads(line)
        for line in (re_dir / "realengine-checks.jsonl").read_text().splitlines()
    ]
    summary["matrix"] = {
        "comparisons": len(matrix),
        "by_probe": {
            probe: {
                "comparisons": len([r for r in matrix if r["probe_instance"] == probe]),
                "diverge": len(
                    [r for r in matrix if r["probe_instance"] == probe and r["verdict"] == "diverge"]
                ),
            }
            for probe in sorted({r["probe_instance"] for r in matrix})
        },
        "divergent_rows": [
            {
                "prompt_id": r["prompt_id"],
                "probe_instance": r["probe_instance"],
                "first_diff_char": r["first_diff_char"],
            }
            for r in matrix
            if r["verdict"] == "diverge"
        ],
    }

    # p1-long repeat characterization (2 fresh CPU instances x 10).
    reps = [
        json.loads(line)
        for line in (re_dir / "realengine-p1long-reps.jsonl").read_text().splitlines()
    ]
    by_instance = defaultdict(list)
    for r in reps:
        by_instance[r["instance"]].append(r)
    summary["p1_long_repeats"] = {
        inst: {
            "requests": len(rows),
            "digest_counts": dict(Counter(r["sha256_16"] for r in rows)),
            "first_request_digest": rows[0]["sha256_16"],
            "minority_at_first_request": rows[0]["sha256_16"]
            != Counter(r["sha256_16"] for r in rows).most_common(1)[0][0],
        }
        for inst, rows in by_instance.items()
    }

    # Fresh-pair probe (1 fresh instance, same prompt 4x, nothing else).
    pair = [
        json.loads(line)
        for line in (re_dir / "realengine-p1long-fresh-pair.jsonl").read_text().splitlines()
    ]
    summary["p1_long_fresh_pair"] = {
        "requests": len(pair),
        "digest_sequence": [r["sha256_16"] for r in pair],
        "chars": [r["chars"] for r in pair],
    }

    # Cross-machine cache-cold: A rows (file) vs B rows (recorded from the
    # B-side run — the ANALYSIS pins the exact command and its output).
    acold = [
        json.loads(line)
        for line in (re_dir / "realengine-crossmachine-acold.jsonl").read_text().splitlines()
    ]
    bcold = {
        ("b1", "p1-short"): ("b8d0d0b4867c6407", 48),
        ("b1", "p1-medium"): ("dbbce3bd4c9ba6ba", 83),
        ("b1", "p1-long"): ("868701fe9a3958d9", 139),
        ("b1", "p1-xl"): ("dbbce3bd4c9ba6ba", 83),
        ("b2", "p1-short"): ("b8d0d0b4867c6407", 48),
        ("b2", "p1-medium"): ("dbbce3bd4c9ba6ba", 83),
        ("b2", "p1-long"): ("868701fe9a3958d9", 139),
        ("b2", "p1-xl"): ("dbbce3bd4c9ba6ba", 83),
    }
    cross = []
    for r in acold:
        b_digest, _ = bcold[(f"b{r['round']}", r["prompt_id"])]
        cross.append(
            {
                "prompt_id": r["prompt_id"],
                "a_instance": f"a{r['round']}",
                "b_instance": f"b{r['round']}",
                "a_sha256_16": r["sha256_16"],
                "b_sha256_16": b_digest,
                "agree": r["sha256_16"] == b_digest,
            }
        )
    summary["cross_machine_cache_cold"] = {
        "comparisons": len(cross),
        "agree": len([c for c in cross if c["agree"]]),
        "diverge": len([c for c in cross if not c["agree"]]),
        "rows": cross,
    }
    return summary


def main() -> int:
    raw_root = Path(sys.argv[1])
    out_path = Path(sys.argv[2])
    summary = {"record_type": "accuracy-summary/v1", "families": {}}
    for dirname in STRUCTURAL_DIRS:
        d = raw_root / dirname
        if not d.is_dir():
            continue
        cells = []
        for cell_dir in sorted(p for p in d.iterdir() if p.is_dir()):
            cells.append(aggregate_structural_cell(cell_dir))
        summary["families"][dirname] = {
            "cells": cells,
        }
    summary["families"][REALENGINE_DIR] = aggregate_realengine(raw_root)
    out_path.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf8")
    print(f"wrote {out_path}")
    # Compact verification printout.
    for name, family in summary["families"].items():
        if "cells" in family:
            for cell in family["cells"]:
                print(
                    f"  {name}/{cell['cell']}: checks={cell['checks']} "
                    f"diverge={cell['peer_reference']['diverge']}/"
                    f"{cell['peer_reference']['comparisons']} "
                    f"suspended={len(cell['suspended_detected'])} "
                    f"racing_tail_saved_ms={cell['racing_tail_saved_ms_median']}"
                )
        else:
            cm = family["cross_machine_cache_cold"]
            print(
                f"  {name}: cross-machine cache-cold agree "
                f"{cm['agree']}/{cm['comparisons']}"
            )
    return 0


if __name__ == "__main__":
    sys.exit(main())

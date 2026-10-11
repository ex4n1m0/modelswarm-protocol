#!/usr/bin/env python3
"""Real-engine greedy divergence check (accuracy-benefits study, 2026-10-11).

Validates the divergence-detection instrument against REAL llama.cpp
engines on this machine (machine A), mirroring the E0 determinism
discipline (docs/verification/e0-determinism-2026-10-07.md):

- two CPU llama-server instances (fresh processes, fresh memory) on
  different ports: same-backend-family replication — expected AGREE;
- one Vulkan instance: the NEGATIVE CONTROL (E0 proved CPU != Vulkan) —
  the instrument MUST fire on this real divergence;
- every prompt is also decoded twice on the first CPU instance to pin
  request-level determinism inside one process.

Prompts are the committed synthetic pass-1 corpus (no user content).
Outputs realengine-checks.jsonl (self-describing records; sha256-16
digests only, never token text) + prints a verdict matrix.

Usage: python realengine_divergence.py [--skip-vulkan]
"""

from __future__ import annotations

import argparse
import json
import hashlib
import subprocess
import sys
import time
import urllib.request

MODEL = (
    r"C:\Users\igorc\AppData\Local\ModelSwarm\Data\models"
    r"\msp1_948d898a6ace88592f19b8e02ccd776f3e09be1049e0809454ebe4ee2f1acc2f"
    r"\SmolLM2-135M-Instruct-Q4_K_M.gguf"
)
CPU_ENGINE = r"C:\Users\igorc\AppData\Local\ModelSwarm\engine\llama-server.exe"
VULKAN_ENGINE = (
    r"C:\Users\igorc\AppData\Local\ModelSwarm\engine-vulkan\llama-server.exe"
)

# The committed synthetic pass-1 corpus (crates/modelswarm-bench/src/corpus.rs)
# — texts duplicated here so the script is self-contained; the ANALYSIS
# pins the corpus digest equality with the compiled-in set.
PROMPTS = {
    "p1-short": (
        "modelswarm synthetic pass-1 short prompt: summarize the following "
        "sequence of labeled measurements in one sentence; "
    ),
    "p1-medium": (
        "modelswarm synthetic pass-1 medium prompt: the quick brown fox jumps "
        "over the lazy dog while five wizards judge twelve identical quanta of "
        "hedged, speculative, exactly verified inference; summarize the "
        "trade-offs in two sentences and keep every number intact; "
    ),
    "p1-long": (
        "modelswarm synthetic pass-1 long prompt: the quick brown fox jumps "
        "over the lazy dog while five wizards judge twelve identical quanta of "
        "hedged, speculative, exactly verified inference; the sluggish amber "
        "whale drifts beneath the eager cormorant as seven clerks audit "
        "fifteen mismatched ledgers of drained, verified, exactly cancelled "
        "speculation; summarize the trade-offs in three sentences, keep every "
        "number intact, and end with the word done; "
    ),
    "p1-xl": (
        "modelswarm synthetic pass-1 extra-long prompt: the quick brown fox "
        "jumps over the lazy dog while five wizards judge twelve identical "
        "quanta of hedged, speculative, exactly verified inference; the "
        "sluggish amber whale drifts beneath the eager cormorant as seven "
        "clerks audit fifteen mismatched ledgers of drained, verified, exactly "
        "cancelled speculation; the nimble jade heron strides beside the "
        "placid ox while nine archivists index nineteen divergent folios of "
        "queued, verified, exactly reconciled attribution; summarize the "
        "trade-offs in four sentences, keep every number intact, and end with "
        "the word done; "
    ),
}

# E0 sampling discipline: /v1/completions, temperature 0, seed 7,
# max_tokens 48.
SAMPLING = {"temperature": 0.0, "seed": 7, "max_tokens": 48}


def sha16(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()[:16]


def first_diff(a: str, b: str):
    """First differing character offset (None when equal)."""
    if a == b:
        return None
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return i
    return min(len(a), len(b))


def wait_healthy(port: int, timeout_s: float = 60.0) -> bool:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(
                f"http://127.0.0.1:{port}/health", timeout=2
            ) as r:
                if r.status == 200:
                    return True
        except Exception:
            time.sleep(0.5)
    return False


def complete(port: int, prompt_id: str, prompt: str, retry: int = 0):
    body = {
        "prompt": prompt,
        "temperature": SAMPLING["temperature"],
        "seed": SAMPLING["seed"],
        "max_tokens": SAMPLING["max_tokens"],
        "stream": False,
    }
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/v1/completions",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
    )
    started = time.perf_counter()
    with urllib.request.urlopen(req, timeout=120) as r:
        payload = json.loads(r.read())
    elapsed = (time.perf_counter() - started) * 1000.0
    text = payload["choices"][0]["text"]
    return {
        "prompt_id": prompt_id,
        "retry": retry,
        "text": text,
        "sha256_16": sha16(text),
        "total_ms": elapsed,
        "usage": payload.get("usage", {}),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--skip-vulkan", action="store_true")
    parser.add_argument(
        "--out",
        default="realengine-checks.jsonl",
        help="output jsonl path",
    )
    args = parser.parse_args()

    instances = [
        ("cpu-a", CPU_ENGINE, 8901, []),
        ("cpu-b", CPU_ENGINE, 8902, []),
    ]
    if not args.skip_vulkan:
        instances.append(("vulkan-a", VULKAN_ENGINE, 8903, ["-ngl", "99"]))

    procs = []
    try:
        for name, engine, port, extra in instances:
            cmd = [
                engine,
                "-m",
                MODEL,
                "--host",
                "127.0.0.1",
                "--port",
                str(port),
                "-c",
                "2048",
                *extra,
            ]
            print(f"[spawn] {name}: {' '.join(cmd)}")
            procs.append(
                (name, port, subprocess.Popen(cmd, stdout=subprocess.DEVNULL,
                                              stderr=subprocess.DEVNULL))
            )
        for name, port, _ in procs:
            if not wait_healthy(port):
                print(f"[fatal] {name} on :{port} never became healthy")
                return 2
        print("[spawn] all instances healthy")

        records = []
        for prompt_id, prompt in PROMPTS.items():
            results = {}
            # Two requests on cpu-a: within-process repetition determinism.
            results["cpu-a#1"] = complete(8901, prompt_id, prompt, retry=1)
            results["cpu-a#2"] = complete(8901, prompt_id, prompt, retry=2)
            for name, port, _ in procs:
                if name == "cpu-a":
                    continue
                results[name] = complete(port, prompt_id, prompt)

            reference = results["cpu-a#1"]
            for probe_name, probe in results.items():
                if probe_name == "cpu-a#1":
                    continue
                position = first_diff(reference["text"], probe["text"])
                records.append(
                    {
                        "record_type": "realengine-check/v1",
                        "prompt_id": prompt_id,
                        "reference_instance": "cpu-a",
                        "probe_instance": probe_name,
                        "verdict": "agree" if position is None else "diverge",
                        "first_diff_char": position,
                        "reference_sha256_16": reference["sha256_16"],
                        "probe_sha256_16": probe["sha256_16"],
                        "reference_total_ms": reference["total_ms"],
                        "probe_total_ms": probe["total_ms"],
                        "reference_completion_chars": len(reference["text"]),
                        "probe_completion_chars": len(probe["text"]),
                    }
                )
        with open(args.out, "w", encoding="utf8") as fh:
            for record in records:
                fh.write(json.dumps(record) + "\n")

        # Verdict matrix (stdout; the ANALYSIS cites it).
        agree = [r for r in records if r["verdict"] == "agree"]
        diverge = [r for r in records if r["verdict"] == "diverge"]
        print(f"[matrix] {len(records)} comparisons: "
              f"{len(agree)} agree, {len(diverge)} diverge")
        for r in records:
            print(
                f"  {r['prompt_id']:<8} {r['probe_instance']:<9} "
                f"{r['verdict']:<7} ref={r['reference_sha256_16']} "
                f"probe={r['probe_sha256_16']} "
                f"first_diff={r['first_diff_char']}"
            )
        return 0
    finally:
        for _, port, proc in procs:
            proc.terminate()
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()
        print("[spawn] all instances stopped")


if __name__ == "__main__":
    sys.exit(main())

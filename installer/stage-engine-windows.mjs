#!/usr/bin/env node
// Stages the pinned llama.cpp engines for the Windows NSIS bundle:
// the canonical CPU set (engine/) AND the Vulkan GPU variant
// (engine-vulkan/, ADR-024) when its source is present.
//
//   node installer/stage-engine-windows.mjs [--from DIR] [--from-vulkan DIR]
//
// CPU source (default %LOCALAPPDATA%\ModelSwarm\engine) must contain the
// pinned b11407 win-cpu-x64 files; Vulkan source (default
// %LOCALAPPDATA%\ModelSwarm\engine-vulkan) the win-vulkan-x64 set. EVERY
// staged file is SHA-256-verified against runtime-pins.json (fail-closed)
// and pruned to the bundle list — the exact set the node re-verifies before
// launch. The engine dirs are gitignored; a release built without this step
// ships a dead installer (v0.2.2 did).
import { createHash } from "node:crypto";
import { cp, mkdir, readdir, rm } from "node:fs/promises";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const pins = JSON.parse(readFileSync(join(root, "runtime-pins.json"), "utf8"));
const platform = pins.platforms["windows-x64"];
const argDir = (flag, fallback) =>
  resolve(
    process.argv.includes(flag)
      ? process.argv[process.argv.indexOf(flag) + 1]
      : fallback,
  );
const sha256 = (buf) => createHash("sha256").update(buf).digest("hex");

// The LLVM-OpenMP license ships beside libomp.dll (attribution) even though
// the node's launch-verification set (bundle) does not include it.
const LICENSE = "LICENSE-LLVM-OpenMP";

async function stageSet(label, pinsEntry, from, to) {
  const present = new Set(await readdir(from));
  const staged = [...pinsEntry.bundle, LICENSE];
  let bad = 0;
  for (const name of staged) {
    if (!present.has(name)) {
      console.error(`[${label}] MISSING in source: ${name}`);
      bad++;
      continue;
    }
    const digest = sha256(readFileSync(join(from, name)));
    if (digest !== pinsEntry.files[name]) {
      console.error(
        `[${label}] HASH MISMATCH: ${name}\n  got  ${digest}\n  want ${pinsEntry.files[name]}`,
      );
      bad++;
    }
  }
  if (bad > 0) {
    console.error(`\n[${label}] ${bad} file(s) failed verification — not staging this set.`);
    return false;
  }
  await mkdir(to, { recursive: true });
  for (const entry of await readdir(to)) {
    if (!entry.startsWith(".")) {
      await rm(join(to, entry), { recursive: true, force: true });
    }
  }
  for (const name of staged) {
    await cp(join(from, name), join(to, name));
  }
  console.log(`[${label}] staged ${staged.length} verified files into ${to}`);
  return true;
}

const cpuOk = await stageSet(
  "cpu",
  platform,
  argDir("--from", join(process.env.LOCALAPPDATA ?? ".", "ModelSwarm", "engine")),
  join(root, "crates", "modelswarm-desktop", "engine"),
);
if (!cpuOk) {
  console.error(
    `Expected the pinned ${pins.tag} win-cpu-x64 set from:\n  ${platform.archive_url}`,
  );
  process.exit(1);
}

const variant = platform.variants?.vulkan;
const vulkanFrom = argDir(
  "--from-vulkan",
  join(process.env.LOCALAPPDATA ?? ".", "ModelSwarm", "engine-vulkan"),
);
const vulkanOk = await stageSet(
  "vulkan",
  variant,
  vulkanFrom,
  join(root, "crates", "modelswarm-desktop", "engine-vulkan"),
);
if (!vulkanOk) {
  console.error(
    `Expected the pinned ${pins.tag} win-vulkan-x64 set from:\n  ${variant?.archive_url ?? "(variant missing from runtime-pins.json)"}`,
  );
  process.exit(1);
}
console.log(
  `All engine sets staged (llama.cpp ${pins.tag}, canonical ${pins.canonical_build_hash.slice(0, 12)}…).`,
);

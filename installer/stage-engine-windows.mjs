#!/usr/bin/env node
// Stages the pinned llama.cpp engine for the Windows NSIS bundle.
//
//   node installer/stage-engine-windows.mjs [--from DIR]
//
// Source (default %LOCALAPPDATA%\ModelSwarm\engine) must contain the pinned
// b11407 win-cpu-x64 files; EVERY staged file is SHA-256-verified against
// runtime-pins.json (fail-closed) and pruned to the bundle list — the exact
// set the node re-verifies before launch. The engine dir is gitignored; a
// release built without this step ships a dead installer (v0.2.2 did).
import { createHash } from "node:crypto";
import { cp, mkdir, readdir, rm } from "node:fs/promises";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const pins = JSON.parse(readFileSync(join(root, "runtime-pins.json"), "utf8"));
const platform = pins.platforms["windows-x64"];
const from = resolve(
  process.argv.includes("--from")
    ? process.argv[process.argv.indexOf("--from") + 1]
    : join(process.env.LOCALAPPDATA ?? ".", "ModelSwarm", "engine"),
);
const to = join(root, "crates", "modelswarm-desktop", "engine");

const sha256 = (buf) => createHash("sha256").update(buf).digest("hex");
const present = new Set(await readdir(from));

// The LLVM-OpenMP license ships beside libomp.dll (attribution) even though
// the node's launch-verification set (bundle) does not include it.
const STAGED = [...platform.bundle, "LICENSE-LLVM-OpenMP"];

let bad = 0;
for (const name of STAGED) {
  if (!present.has(name)) {
    console.error(`MISSING in source: ${name}`);
    bad++;
    continue;
  }
  const digest = sha256(readFileSync(join(from, name)));
  if (digest !== platform.files[name]) {
    console.error(`HASH MISMATCH: ${name}\n  got  ${digest}\n  want ${platform.files[name]}`);
    bad++;
  }
}
if (bad > 0) {
  console.error(`\n${bad} file(s) failed verification — not staging anything.`);
  console.error(`Expected the pinned ${pins.tag} win-cpu-x64 set from:\n  ${platform.archive_url}`);
  process.exit(1);
}

// Clear only previously staged engine files (keep the tracked dotfiles).
await mkdir(to, { recursive: true });
for (const entry of await readdir(to)) {
  if (!entry.startsWith(".")) {
    await rm(join(to, entry), { recursive: true, force: true });
  }
}
for (const name of STAGED) {
  await cp(join(from, name), join(to, name));
}
console.log(
  `Staged ${STAGED.length} verified files (llama.cpp ${pins.tag}, ` +
    `canonical ${pins.canonical_build_hash.slice(0, 12)}…) into ${to}`,
);

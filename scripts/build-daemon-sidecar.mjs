#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

function resolveTargetTriple() {
  if (process.env.TAURI_ENV_TARGET_TRIPLE) {
    return process.env.TAURI_ENV_TARGET_TRIPLE;
  }

  const output = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const match = output.match(/host:\s*(\S+)/);
  if (!match) {
    throw new Error("failed to determine host target triple from `rustc -vV`");
  }
  return match[1];
}

const target = resolveTargetTriple();
const isWindows = target.includes("windows");
const binaryName = isWindows ? "susun-studio-daemon.exe" : "susun-studio-daemon";
const sidecarName = isWindows
  ? `susun-studio-daemon-${target}.exe`
  : `susun-studio-daemon-${target}`;

console.log(`building susun-studio-daemon for ${target}`);
execFileSync("cargo", ["build", "--release", "-p", "susun-studio-daemon", "--target", target], {
  cwd: repoRoot,
  stdio: "inherit",
});

const sourcePath = join(repoRoot, "target", target, "release", binaryName);
const destDir = join(repoRoot, "src-tauri", "binaries");
mkdirSync(destDir, { recursive: true });
const destPath = join(destDir, sidecarName);

copyFileSync(sourcePath, destPath);
console.log(`copied daemon sidecar to ${destPath}`);

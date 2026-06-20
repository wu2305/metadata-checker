#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const defaultOutDir = join(repoRoot, "target/browser-offscreen-bench-wasm");

function cargoTargetDir() {
  const configured = process.env.CARGO_TARGET_DIR;
  if (!configured) {
    return join(repoRoot, "target");
  }
  return resolve(repoRoot, configured);
}

function browserWasmSource() {
  return join(
    cargoTargetDir(),
    "wasm32-unknown-unknown/release/metadata_checker.wasm",
  );
}

function readWasmBindgenVersion() {
  const lock = readFileSync(join(repoRoot, "Cargo.lock"), "utf8");
  const lines = lock.split(/\r?\n/);
  let found = false;
  for (const line of lines) {
    if (line.trim() === 'name = "wasm-bindgen"') {
      found = true;
      continue;
    }
    if (found && line.trim().startsWith("version")) {
      const match = line.match(/"([^"]+)"/);
      if (!match) {
        throw new Error("failed to parse wasm-bindgen version from Cargo.lock");
      }
      return match[1];
    }
  }
  throw new Error("wasm-bindgen not found in Cargo.lock");
}

function runChecked(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: options.stdio ?? "pipe",
  });
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed with status ${result.status}\n${result.stderr ?? ""}`,
    );
  }
  return result;
}

function parseArgs(argv) {
  const options = {
    outDir: defaultOutDir,
    skipBuild: false,
    profile: "release",
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const next = () => {
      index += 1;
      if (index >= argv.length) {
        throw new Error(`missing value for ${arg}`);
      }
      return argv[index];
    };
    switch (arg) {
      case "--out-dir":
        options.outDir = resolve(next());
        break;
      case "--skip-build":
        options.skipBuild = true;
        break;
      case "--profile":
        options.profile = next();
        break;
      case "--help":
      case "-h":
        console.log(`Usage: node browser/bench/build-browser-wasm.mjs [--out-dir DIR] [--skip-build]`);
        process.exit(0);
        break;
      default:
        throw new Error(`unknown argument: ${arg}`);
    }
  }
  return options;
}

function ensureWasmBindgenCli(expectedVersion) {
  const probe = spawnSync("wasm-bindgen", ["--version"], { encoding: "utf8" });
  if (probe.status !== 0) {
    throw new Error(
      `wasm-bindgen CLI is missing. Install with: cargo install wasm-bindgen-cli --version ${expectedVersion} --locked`,
    );
  }
  const installed = (probe.stdout || probe.stderr || "").trim();
  if (!installed.includes(expectedVersion)) {
    throw new Error(
      `wasm-bindgen CLI version mismatch. expected ${expectedVersion}, got ${installed}. Install with: cargo install wasm-bindgen-cli --version ${expectedVersion} --locked`,
    );
  }
}

export function buildBrowserWasm(options = {}) {
  const resolved = {
    outDir: options.outDir ?? defaultOutDir,
    skipBuild: options.skipBuild ?? false,
    profile: options.profile ?? "release",
  };
  const expectedVersion = readWasmBindgenVersion();
  ensureWasmBindgenCli(expectedVersion);

  if (!resolved.skipBuild) {
    const buildArgs = [
      "build",
      "--no-default-features",
      "--features",
      "browser-wasm",
      "--target",
      "wasm32-unknown-unknown",
    ];
    if (resolved.profile === "release") {
      buildArgs.push("--release");
    } else {
      buildArgs.push("--profile", resolved.profile);
    }
    runChecked("cargo", buildArgs, { stdio: "inherit" });
  }

  const wasmSource = browserWasmSource();
  if (!existsSync(wasmSource)) {
    throw new Error(`browser wasm artifact not found: ${wasmSource}`);
  }

  runChecked("wasm-bindgen", [
    "--target",
    "nodejs",
    "--out-dir",
    resolved.outDir,
    wasmSource,
  ], { stdio: "inherit" });

  const gluePath = join(resolved.outDir, "metadata_checker.js");
  const wasmPath = join(resolved.outDir, "metadata_checker_bg.wasm");
  if (!existsSync(gluePath) || !existsSync(wasmPath)) {
    throw new Error(`wasm-bindgen output missing under ${resolved.outDir}`);
  }

  return {
    expectedVersion,
    outDir: resolved.outDir,
    gluePath,
    wasmPath,
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const built = buildBrowserWasm(parseArgs(process.argv.slice(2)));
    console.log(JSON.stringify({ ok: true, ...built }, null, 2));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}

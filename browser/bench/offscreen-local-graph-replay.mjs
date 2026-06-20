#!/usr/bin/env node

import { createWriteStream, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { spawnSync } from "node:child_process";
import { buildBrowserWasm } from "./build-browser-wasm.mjs";
import {
  createColdOffscreenLocalGraphHost,
  createOffscreenLocalGraphHost,
} from "./offscreen-local-graph-host.mjs";
import { summarizeOffscreenBench } from "./summarize-offscreen-bench.mjs";
import { validateOffscreenBenchOutput } from "./validate-offscreen-bench-output.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const defaultFixtureDir = join(repoRoot, "tests/fixtures/browser-offscreen-real-project");
const defaultOutputDir = join(repoRoot, "target/browser-offscreen-bench");
const defaultWasmDir = join(repoRoot, "target/browser-offscreen-bench-wasm");

const SCENARIOS = [
  "cold",
  "warm_same_document_new_component",
  "repeat_same_component",
];

const REQUIRED_TIMING_KEYS = [
  "wasm_init_ms",
  "fixture_load_ms",
  "runtime_load_ms",
  "build_graph_ms",
  "runtime_load_build_graph_ms",
  "analyze_selection_ms",
  "serialize_output_ms",
  "total_ms",
];

function parseArgs(argv) {
  const options = {
    fixtureDir: defaultFixtureDir,
    outputDir: defaultOutputDir,
    wasmDir: defaultWasmDir,
    iterations: 10,
    warmup: 2,
    smoke: false,
    skipWasmBuild: false,
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
      case "--fixture-dir":
        options.fixtureDir = resolve(next());
        break;
      case "--output-dir":
        options.outputDir = resolve(next());
        break;
      case "--wasm-dir":
        options.wasmDir = resolve(next());
        break;
      case "--iterations":
        options.iterations = Number.parseInt(next(), 10);
        break;
      case "--warmup":
        options.warmup = Number.parseInt(next(), 10);
        break;
      case "--smoke":
        options.smoke = true;
        options.iterations = 1;
        options.warmup = 0;
        break;
      case "--skip-wasm-build":
        options.skipWasmBuild = true;
        break;
      case "--help":
      case "-h":
        printHelp();
        process.exit(0);
        break;
      default:
        throw new Error(`unknown argument: ${arg}`);
    }
  }
  if (!Number.isInteger(options.iterations) || options.iterations < 1) {
    throw new Error("--iterations must be a positive integer");
  }
  if (!Number.isInteger(options.warmup) || options.warmup < 0) {
    throw new Error("--warmup must be a non-negative integer");
  }
  return options;
}

function printHelp() {
  console.log(`Usage: node browser/bench/offscreen-local-graph-replay.mjs [options]

Options:
  --fixture-dir DIR     Fixture root with manifest.json (default: tests/fixtures/browser-offscreen-real-project)
  --output-dir DIR      Output directory (default: target/browser-offscreen-bench)
  --wasm-dir DIR        wasm-bindgen node glue directory
  --iterations N        Iterations per scenario (default: 10)
  --warmup N            Warmup iterations excluded from output (default: 2)
  --smoke               CI smoke mode: iterations=1 warmup=0
  --skip-wasm-build     Reuse existing wasm glue
`);
}

function currentCommit() {
  const result = spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  return result.status === 0 ? result.stdout.trim() : "unknown";
}

function readManifest(fixtureDir) {
  const manifestPath = join(fixtureDir, "manifest.json");
  if (!existsSync(manifestPath)) {
    throw new Error(`manifest not found: ${manifestPath}`);
  }
  return {
    manifestPath,
    manifest: JSON.parse(readFileSync(manifestPath, "utf8")),
  };
}

function buildPayload(manifest, sample, componentKind, componentId) {
  const item = {
    source_path: sample.source_path,
    project_name: manifest.project_name,
    file_id: `${manifest.project_name}/${sample.source_path}`,
    revision: 1,
    base_url: "http://fixture.local/",
  };
  const selection = {
    source_path: sample.source_path,
    project_name: manifest.project_name,
    active_component_id: componentId,
    selected_component_ids: [componentId],
  };
  return { item, selection, options: { depth: 2, visible_hop: 1 } };
}

function alternateComponentId(sample, componentKind) {
  const kinds = Object.keys(sample.components || {});
  const currentIndex = kinds.indexOf(componentKind);
  const nextKind = kinds[(currentIndex + 1) % kinds.length];
  return sample.components[nextKind]?.id ?? sample.components[componentKind].id;
}

async function runScenario({
  scenario,
  sample,
  componentKind,
  componentId,
  manifest,
  fixtureDir,
  wasmDir,
  iterations,
  warmup,
}) {
  const records = [];
  const fixtureRoot = join(fixtureDir, manifest.fixture_root || "project");
  let warmHost = null;

  for (let iteration = 1; iteration <= iterations + warmup; iteration += 1) {
    const isWarmup = iteration <= warmup;
    let host;
    let activeComponentId = componentId;

    if (scenario === "cold") {
      host = await createColdOffscreenLocalGraphHost({
        wasmDir,
        fixtureRoot,
        moduleNonce: `${sample.id}-${componentKind}-${iteration}`,
      });
    } else {
      if (!warmHost) {
        warmHost = await createOffscreenLocalGraphHost({
          wasmDir,
          fixtureRoot,
          moduleNonce: `${sample.id}-${componentKind}-warm`,
        });
      }
      host = warmHost;
      if (scenario === "warm_same_document_new_component" && iteration > warmup + 1) {
        activeComponentId = alternateComponentId(sample, componentKind);
      }
    }

    const payload = buildPayload(manifest, sample, componentKind, activeComponentId);
    const result = await host.analyzeLocalGraph(payload);
    if (!isWarmup) {
      records.push({
        capability: "browser_analyze_selection",
        mode: "browser_offscreen_replay_wasm",
        scenario: `${scenario}_${sample.id}_${componentKind}`,
        scenario_base: scenario,
        sample: sample.id,
        component_kind: componentKind,
        component_id: activeComponentId,
        iteration: iteration - warmup,
        ok: result.ok === true,
        artifact_ready: result.artifact_ready === true,
        duration_ms: result.timing?.total_ms ?? null,
        timing: result.timing ?? {},
        cache: result.cache ?? {},
        serialized_bytes: result.serialized_bytes ?? 0,
      });
    }
  }

  return records;
}

function writeJsonl(path, records) {
  const stream = createWriteStream(path, { encoding: "utf8" });
  for (const record of records) {
    stream.write(`${JSON.stringify(record)}\n`);
  }
  stream.end();
}

function writeEnv(path, env) {
  writeFileSync(path, `${JSON.stringify(env, null, 2)}\n`, "utf8");
}

function writeSamplesSnapshot(path, manifest) {
  writeFileSync(path, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
}

export async function runOffscreenLocalGraphReplay(options) {
  const { manifestPath, manifest } = readManifest(options.fixtureDir);
  const fixtureRoot = join(options.fixtureDir, manifest.fixture_root || "project");
  if (!existsSync(fixtureRoot)) {
    throw new Error(`fixture project root not found: ${fixtureRoot}`);
  }

  const wasmBuild = options.skipWasmBuild
    ? { outDir: options.wasmDir }
    : buildBrowserWasm({ outDir: options.wasmDir });

  const measuredAt = new Date().toISOString();
  const commit = currentCommit();
  const records = [];

  for (const sample of manifest.samples) {
    for (const componentKind of Object.keys(sample.components || {})) {
      const componentId = sample.components[componentKind].id;
      for (const scenario of SCENARIOS) {
        const scenarioRecords = await runScenario({
          scenario,
          sample,
          componentKind,
          componentId,
          manifest,
          fixtureDir: options.fixtureDir,
          wasmDir: wasmBuild.outDir,
          iterations: options.iterations,
          warmup: options.warmup,
        });
        records.push(...scenarioRecords);
      }
    }
  }

  mkdirSync(options.outputDir, { recursive: true });
  const jsonlPath = join(options.outputDir, "browser-offscreen-bench.jsonl");
  const summaryJsonPath = join(options.outputDir, "browser-offscreen-summary.json");
  const summaryMdPath = join(options.outputDir, "browser-offscreen-summary.md");
  const envPath = join(options.outputDir, "browser-offscreen-env.json");
  const samplesPath = join(options.outputDir, "browser-offscreen-samples.json");

  writeJsonl(jsonlPath, records);
  const summary = summarizeOffscreenBench(records);
  writeFileSync(summaryJsonPath, `${JSON.stringify(summary, null, 2)}\n`, "utf8");
  writeFileSync(summaryMdPath, `${summary.markdown}\n`, "utf8");
  writeEnv(envPath, {
    measured_at: measuredAt,
    commit,
    manifest_path: manifestPath,
    fixture_dir: options.fixtureDir,
    wasm_dir: wasmBuild.outDir,
    iterations: options.iterations,
    warmup: options.warmup,
    smoke: options.smoke,
    required_timing_keys: REQUIRED_TIMING_KEYS,
  });
  writeSamplesSnapshot(samplesPath, manifest);

  const validation = validateOffscreenBenchOutput({
    records,
    summary,
    requiredTimingKeys: REQUIRED_TIMING_KEYS,
  });
  if (!validation.ok) {
    throw new Error(`bench output validation failed: ${validation.errors.join("; ")}`);
  }

  return {
    ok: true,
    jsonlPath,
    summaryJsonPath,
    summaryMdPath,
    envPath,
    samplesPath,
    recordCount: records.length,
    validation,
  };
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const result = await runOffscreenLocalGraphReplay(options);
  console.log(JSON.stringify(result, null, 2));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  });
}

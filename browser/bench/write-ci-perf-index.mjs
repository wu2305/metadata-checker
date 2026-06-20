#!/usr/bin/env node

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const defaultOutputDir = join(repoRoot, "target/browser-offscreen-bench");

function parseArgs(argv) {
  const options = {
    outputDir: defaultOutputDir,
    bench: "browser-offscreen",
    ciTier: null,
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
      case "--output-dir":
        options.outputDir = resolve(next());
        break;
      case "--bench":
        options.bench = next();
        break;
      case "--ci-tier":
        options.ciTier = next();
        break;
      case "--help":
      case "-h":
        console.log(`Usage: node browser/bench/write-ci-perf-index.mjs [--output-dir DIR] [--bench NAME] [--ci-tier TIER]`);
        process.exit(0);
        break;
      default:
        throw new Error(`unknown argument: ${arg}`);
    }
  }
  return options;
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

export function writeCiPerfIndex(options = {}) {
  const outputDir = options.outputDir ?? defaultOutputDir;
  const bench = options.bench ?? "browser-offscreen";
  const envPath = join(outputDir, "browser-offscreen-env.json");
  const summaryPath = join(outputDir, "browser-offscreen-summary.json");
  const jsonlPath = join(outputDir, "browser-offscreen-bench.jsonl");
  const summaryMdPath = join(outputDir, "browser-offscreen-summary.md");
  const samplesPath = join(outputDir, "browser-offscreen-samples.json");
  const indexPath = join(outputDir, "browser-offscreen-ci-perf-index.json");

  for (const path of [envPath, summaryPath, jsonlPath]) {
    if (!existsSync(path)) {
      throw new Error(`required bench artifact not found: ${path}`);
    }
  }

  const env = readJson(envPath);
  const summary = readJson(summaryPath);
  const fixtureSampleCount = existsSync(samplesPath)
    ? (readJson(samplesPath).samples?.length ?? 0)
    : 0;

  const index = {
    schema_version: 1,
    bench,
    ci_tier: options.ciTier ?? process.env.CNB_PIPELINE_NAME ?? null,
    commit: process.env.CNB_COMMIT ?? env.commit ?? null,
    commit_short: process.env.CNB_COMMIT_SHORT ?? null,
    build_id: process.env.CNB_BUILD_ID ?? null,
    build_url: process.env.CNB_BUILD_WEB_URL ?? null,
    branch: process.env.CNB_BRANCH ?? null,
    event: process.env.CNB_EVENT ?? null,
    record_count: summary.record_count ?? 0,
    scenario_count: summary.scenario_count ?? 0,
    fixture_sample_count: fixtureSampleCount,
    jsonl_path: jsonlPath,
    summary_json_path: summaryPath,
    summary_md_path: summaryMdPath,
    env_path: envPath,
    measured_at: env.measured_at ?? null,
    smoke: env.smoke ?? null,
    generated_at: new Date().toISOString(),
  };

  writeFileSync(indexPath, `${JSON.stringify(index, null, 2)}\n`, "utf8");
  return { ok: true, indexPath, index };
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const result = writeCiPerfIndex(options);
  console.log(JSON.stringify(result, null, 2));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}

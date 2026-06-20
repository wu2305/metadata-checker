#!/usr/bin/env node

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const defaultSummaryPath = join(
  repoRoot,
  "target/browser-offscreen-bench/browser-offscreen-summary.json",
);
const defaultOutputPath = join(
  repoRoot,
  "target/browser-offscreen-bench/browser-offscreen-bencher-bmf.json",
);
const DEFAULT_STATS = ["p50", "p95", "max"];

function parseArgs(argv) {
  const options = {
    summaryPath: defaultSummaryPath,
    outputPath: defaultOutputPath,
    includeStages: true,
    stats: DEFAULT_STATS,
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
      case "--summary":
        options.summaryPath = resolve(next());
        break;
      case "--output":
        options.outputPath = resolve(next());
        break;
      case "--total-only":
        options.includeStages = false;
        break;
      case "--stats":
        options.stats = next()
          .split(",")
          .map((item) => item.trim())
          .filter(Boolean);
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

  for (const stat of options.stats) {
    if (!DEFAULT_STATS.includes(stat)) {
      throw new Error(`unsupported stat: ${stat}`);
    }
  }

  return options;
}

function printHelp() {
  console.log(`Usage: node browser/bench/offscreen-summary-to-bencher-bmf.mjs [options]

Options:
  --summary FILE   Browser offscreen summary JSON
  --output FILE    Output Bencher Metric Format JSON
  --total-only     Only export total_ms metrics
  --stats LIST     Comma-separated stats to export: p50,p95,max
`);
}

function readJson(path) {
  if (!existsSync(path)) {
    throw new Error(`summary not found: ${path}`);
  }
  return JSON.parse(readFileSync(path, "utf8"));
}

function toNanoseconds(valueMs) {
  return valueMs * 1_000_000;
}

function benchmarkName(row, metric, stat) {
  return [
    "browser_offscreen",
    row.scenario,
    row.sample,
    row.component_kind,
    metric,
    stat,
  ].join("/");
}

function addMetric(output, name, valueMs) {
  if (typeof valueMs !== "number" || !Number.isFinite(valueMs)) {
    return;
  }
  output[name] = {
    latency: {
      value: toNanoseconds(valueMs),
    },
  };
}

export function convertOffscreenSummaryToBencherBmf(summary, options = {}) {
  const includeStages = options.includeStages ?? true;
  const stats = options.stats ?? DEFAULT_STATS;
  if (!summary || !Array.isArray(summary.scenarios)) {
    throw new Error("summary.scenarios must be an array");
  }

  const output = {};
  for (const row of summary.scenarios) {
    for (const stat of stats) {
      addMetric(output, benchmarkName(row, "total_ms", stat), row.total_ms?.[stat]);
    }

    if (!includeStages || !row.timing) {
      continue;
    }

    for (const [stage, values] of Object.entries(row.timing)) {
      for (const stat of stats) {
        addMetric(output, benchmarkName(row, stage, stat), values?.[stat]);
      }
    }
  }

  return output;
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const summary = readJson(options.summaryPath);
  const bmf = convertOffscreenSummaryToBencherBmf(summary, options);
  writeFileSync(options.outputPath, `${JSON.stringify(bmf, null, 2)}\n`, "utf8");
  console.log(JSON.stringify({
    ok: true,
    output: options.outputPath,
    benchmark_count: Object.keys(bmf).length,
  }, null, 2));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}

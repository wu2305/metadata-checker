#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import {
  createWriteStream,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const defaultProjectDir = "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi";
const defaultGraphDbPath = "/tmp/metadata-checker-m50-real-project.graphdb";
const defaultOutputDir = "/tmp/metadata-checker-cli-cold-perf";

const explainConditionTarget = "comp:app/销售.app/销售/合同协议.spg|input3";
const queryPageLogicTarget = "page:app/销售.app/销售/合同协议.spg";

const timingScenarios = [
  {
    id: "cli_cold_explain_condition_compact",
    category: "condition",
    capability: "explain_condition",
    budget: "compact",
    intent: "writer",
    buildArgs: (options) => [
      ...baseCliArgs(options),
      "--explain-condition",
      explainConditionTarget,
      "--budget",
      "compact",
      "--intent",
      "writer",
    ],
  },
  {
    id: "cli_cold_query_page_logic_full",
    category: "page_logic",
    capability: "query_page_logic",
    budget: "full",
    buildArgs: (options) => [
      ...baseCliArgs(options),
      "--query-page-logic",
      queryPageLogicTarget,
      "--budget",
      "full",
    ],
  },
];

function baseCliArgs(options) {
  return [
    "--project-dir",
    options.projectDir,
    "--graph-db-path",
    options.graphDbPath,
  ];
}

function parseArgs(argv) {
  const options = {
    projectDir: defaultProjectDir,
    graphDbPath: defaultGraphDbPath,
    outputDir: defaultOutputDir,
    profile: "release-fast",
    bin: null,
    warmup: 1,
    minRuns: 3,
    maxRuns: 5,
    rebuildGraph: false,
    skipBuildBin: false,
    skipBuildGraph: false,
    skipHyperfine: false,
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
      case "--project-dir":
        options.projectDir = next();
        break;
      case "--graph-db-path":
        options.graphDbPath = next();
        break;
      case "--output-dir":
        options.outputDir = next();
        break;
      case "--profile":
        options.profile = next();
        break;
      case "--bin":
        options.bin = next();
        break;
      case "--warmup":
        options.warmup = Number.parseInt(next(), 10);
        break;
      case "--min-runs":
        options.minRuns = Number.parseInt(next(), 10);
        break;
      case "--max-runs":
        options.maxRuns = Number.parseInt(next(), 10);
        break;
      case "--rebuild-graph":
        options.rebuildGraph = true;
        break;
      case "--skip-build-bin":
        options.skipBuildBin = true;
        break;
      case "--skip-build-graph":
        options.skipBuildGraph = true;
        break;
      case "--skip-hyperfine":
        options.skipHyperfine = true;
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

  options.projectDir = resolve(options.projectDir);
  options.graphDbPath = resolve(options.graphDbPath);
  options.outputDir = resolve(options.outputDir);
  options.bin = options.bin
    ? resolve(options.bin)
    : resolve(repoRoot, "target", options.profile, "metadata-checker");
  return options;
}

function printHelp() {
  console.log(`Usage: node tools/cli-cold-start-perf.mjs [options]

Measure CLI cold-start wall time with hyperfine and validate --trace json stdout boundary.

Options:
  --project-dir DIR       Real project directory
  --graph-db-path PATH    GraphDB path (default: ${defaultGraphDbPath})
  --output-dir DIR        Artifact directory (default: ${defaultOutputDir})
  --profile NAME          Cargo profile (default: release-fast)
  --bin PATH              Existing metadata-checker binary
  --warmup N                hyperfine warmup runs (default: 1)
  --min-runs N              hyperfine minimum runs (default: 3)
  --max-runs N              hyperfine maximum runs (default: 5)
  --rebuild-graph           Delete and rebuild graphdb before running
  --skip-build-bin          Do not run cargo build
  --skip-build-graph        Do not build graphdb even if missing
  --skip-hyperfine          Only run trace boundary validation
  -h, --help                Show this help
`);
}

function cargoEnv() {
  return {
    ...process.env,
    CARGO_TARGET_DIR: resolve(repoRoot, "target"),
  };
}

function runChecked(command, args, options = {}) {
  const startedAt = performance.now();
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: options.stdio ?? "pipe",
    env: options.env ?? process.env,
  });
  const wallMs = Math.round(performance.now() - startedAt);
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed with status ${result.status}\n${result.stderr ?? ""}`,
    );
  }
  return {
    wallMs,
    stdout: result.stdout ?? "",
    stderr: result.stderr ?? "",
  };
}

function requireHyperfine() {
  const result = spawnSync("hyperfine", ["--version"], { encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(
      "hyperfine is required for cli-cold-start-perf. Install it first, e.g. `brew install hyperfine`.",
    );
  }
}

function buildBinary(options) {
  if (options.skipBuildBin) {
    if (!existsSync(options.bin)) {
      throw new Error(`binary not found: ${options.bin}`);
    }
    return { skipped: true, wallMs: 0 };
  }

  const args = [
    "build",
    "--profile",
    options.profile,
    "--features",
    "telemetry",
    "--bin",
    "metadata-checker",
  ];
  const result = runChecked("cargo", args, { stdio: "inherit", env: cargoEnv() });
  if (!existsSync(options.bin)) {
    throw new Error(`expected binary was not built: ${options.bin}`);
  }
  return { skipped: false, wallMs: result.wallMs };
}

function buildGraph(options) {
  if (options.rebuildGraph && existsSync(options.graphDbPath)) {
    rmSync(options.graphDbPath, { force: true });
  }
  if (existsSync(options.graphDbPath)) {
    return { skipped: true, wallMs: 0 };
  }
  if (options.skipBuildGraph) {
    throw new Error(`graphdb is missing and --skip-build-graph was set: ${options.graphDbPath}`);
  }

  const result = runChecked(options.bin, [
    ...baseCliArgs(options),
    "--build-graph",
  ]);
  return { skipped: false, wallMs: result.wallMs, stdout: result.stdout.trim() };
}

function currentCommit() {
  const result = spawnSync("git", ["rev-parse", "--short", "HEAD"], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  return result.status === 0 ? result.stdout.trim() : "unknown";
}

function shellQuote(value) {
  if (/^[A-Za-z0-9_./:=+-]+$/.test(value)) {
    return value;
  }
  return `'${value.replace(/'/g, "'\\''")}'`;
}

function formatHyperfineCommand(bin, args) {
  return [bin, ...args].map(shellQuote).join(" ");
}

function runHyperfineScenario(options, scenario) {
  const command = formatHyperfineCommand(options.bin, scenario.buildArgs(options));
  const exportPath = resolve(options.outputDir, `${scenario.id}.hyperfine.json`);
  runChecked("hyperfine", [
    "--warmup",
    String(options.warmup),
    "--min-runs",
    String(options.minRuns),
    "--max-runs",
    String(options.maxRuns),
    "--export-json",
    exportPath,
    command,
  ]);
  const hyperfine = JSON.parse(readFileSync(exportPath, "utf8"));
  const result = hyperfine.results[0];
  return {
    exportPath,
    hyperfine,
    meanMs: Math.round(result.mean * 1000),
    medianMs: Math.round(result.median * 1000),
    minMs: Math.round(result.min * 1000),
    maxMs: Math.round(result.max * 1000),
    timesMs: result.times.map((seconds) => Math.round(seconds * 1000)),
  };
}

function validateTraceStdoutClean(options) {
  const args = [
    "--trace",
    "json",
    ...baseCliArgs(options),
    "--explain-condition",
    explainConditionTarget,
    "--budget",
    "compact",
    "--intent",
    "writer",
  ];
  const result = spawnSync(options.bin, args, {
    cwd: repoRoot,
    encoding: "utf8",
    env: process.env,
  });
  const stdout = result.stdout ?? "";
  const stderr = result.stderr ?? "";
  const checks = {
    exit_code_zero: result.status === 0,
    stdout_non_empty: stdout.trim().length > 0,
    stdout_single_json_document: false,
    stdout_has_protocol_fields: false,
    stdout_trace_free: !stdout.includes('"timestamp"') && !stdout.includes('"fields":'),
    stderr_does_not_contain_stdout_payload:
      stderr.length === 0 || !stderr.includes('"schema_version"'),
    stderr_json_lines_when_present: true,
  };

  try {
    const payload = JSON.parse(stdout);
    checks.stdout_single_json_document = true;
    checks.stdout_has_protocol_fields =
      payload.schema_version != null && payload.summary != null;
  } catch {
    checks.stdout_single_json_document = false;
  }

  const stderrLines = stderr
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
  checks.stderr_json_lines_when_present = stderrLines.every((line) => {
    try {
      JSON.parse(line);
      return true;
    } catch {
      return false;
    }
  });
  checks.stderr_has_trace = stderrLines.length > 0;

  const ok =
    checks.exit_code_zero &&
    checks.stdout_non_empty &&
    checks.stdout_single_json_document &&
    checks.stdout_has_protocol_fields &&
    checks.stdout_trace_free &&
    checks.stderr_does_not_contain_stdout_payload &&
    checks.stderr_json_lines_when_present;

  return {
    ok,
    checks,
    stdout_bytes: Buffer.byteLength(stdout, "utf8"),
    stderr_bytes: Buffer.byteLength(stderr, "utf8"),
    stderr_line_count: stderrLines.length,
  };
}

function writeArtifacts(options, records, summary, meta) {
  mkdirSync(options.outputDir, { recursive: true });
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const jsonlPath = resolve(options.outputDir, `cli-cold-start-${stamp}.jsonl`);
  const mdPath = resolve(options.outputDir, `cli-cold-start-${stamp}.md`);

  const jsonl = createWriteStream(jsonlPath, { encoding: "utf8" });
  for (const record of records) {
    jsonl.write(`${JSON.stringify(record)}\n`);
  }
  jsonl.end();

  const lines = [
    "# CLI Cold Start / Trace Boundary Perf",
    "",
    `- commit: \`${meta.commit}\``,
    `- project_dir: \`${options.projectDir}\``,
    `- graph_db_path: \`${options.graphDbPath}\``,
    `- binary: \`${options.bin}\``,
    `- warmup: ${options.warmup}`,
    `- min_runs: ${options.minRuns}`,
    `- max_runs: ${options.maxRuns}`,
    `- build_binary_wall_ms: ${meta.buildBinary.wallMs}${meta.buildBinary.skipped ? " (skipped)" : ""}`,
    `- build_graph_wall_ms: ${meta.buildGraph.wallMs}${meta.buildGraph.skipped ? " (skipped)" : ""}`,
    `- graph_db_size_bytes: ${meta.graphDbSizeBytes}`,
    "",
    "| scenario | ok | mean_ms | median_ms | min_ms | max_ms |",
    "|---|---:|---:|---:|---:|---:|",
    ...summary.timing.map((row) => (
      `| ${row.scenario} | ${row.ok ? "yes" : "no"} | ${row.mean_ms ?? ""} | ${row.median_ms ?? ""} | ${row.min_ms ?? ""} | ${row.max_ms ?? ""} |`
    )),
    "",
    "## Trace boundary",
    "",
    `- scenario: \`cli_trace_json_stdout_clean\``,
    `- ok: ${summary.trace.ok}`,
    `- stdout_bytes: ${summary.trace.stdout_bytes}`,
    `- stderr_bytes: ${summary.trace.stderr_bytes}`,
    `- stderr_line_count: ${summary.trace.stderr_line_count}`,
    "",
    "Checks:",
    ...Object.entries(summary.trace.checks).map(([key, value]) => `- ${key}: ${value}`),
    "",
    `JSONL: \`${jsonlPath}\``,
  ];
  writeFileSync(mdPath, `${lines.join("\n")}\n`, "utf8");

  return { jsonlPath, mdPath };
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  if (!existsSync(options.projectDir)) {
    throw new Error(`real project directory not found: ${options.projectDir}`);
  }
  if (!options.skipHyperfine) {
    requireHyperfine();
  }

  const commit = currentCommit();
  const buildBinaryResult = buildBinary(options);
  const buildGraphResult = buildGraph(options);
  const measuredAt = new Date().toISOString();
  const records = [
    {
      schema_version: 1,
      record_type: "lifecycle",
      runner: "cli-cold-start-perf",
      commit,
      measured_at: measuredAt,
      project_dir: options.projectDir,
      graph_db_path: options.graphDbPath,
      binary: options.bin,
      mode: "cli_cold",
      scenario: "build_binary",
      category: "lifecycle",
      ok: true,
      skipped: buildBinaryResult.skipped,
      timing: { wall_ms: buildBinaryResult.wallMs },
    },
    {
      schema_version: 1,
      record_type: "lifecycle",
      runner: "cli-cold-start-perf",
      commit,
      measured_at: measuredAt,
      project_dir: options.projectDir,
      graph_db_path: options.graphDbPath,
      binary: options.bin,
      mode: "cli_cold",
      scenario: "build_graph",
      category: "lifecycle",
      ok: true,
      skipped: buildGraphResult.skipped,
      timing: { wall_ms: buildGraphResult.wallMs },
    },
  ];

  const timingSummary = [];
  if (!options.skipHyperfine) {
    for (const scenario of timingScenarios) {
      const timing = runHyperfineScenario(options, scenario);
      records.push({
        schema_version: 1,
        record_type: "capability",
        runner: "cli-cold-start-perf",
        commit,
        measured_at: measuredAt,
        project_dir: options.projectDir,
        graph_db_path: options.graphDbPath,
        binary: options.bin,
        mode: "cli_cold",
        scenario: scenario.id,
        category: scenario.category,
        capability: scenario.capability,
        budget: scenario.budget,
        ok: true,
        timing: {
          mean_ms: timing.meanMs,
          median_ms: timing.medianMs,
          min_ms: timing.minMs,
          max_ms: timing.maxMs,
          samples_ms: timing.timesMs,
        },
        hyperfine_export: timing.exportPath,
      });
      timingSummary.push({
        scenario: scenario.id,
        ok: true,
        mean_ms: timing.meanMs,
        median_ms: timing.medianMs,
        min_ms: timing.minMs,
        max_ms: timing.maxMs,
      });
    }
  }

  const trace = validateTraceStdoutClean(options);
  records.push({
    schema_version: 1,
    record_type: "boundary",
    runner: "cli-cold-start-perf",
    commit,
    measured_at: measuredAt,
    project_dir: options.projectDir,
    graph_db_path: options.graphDbPath,
    binary: options.bin,
    mode: "cli_cold",
    scenario: "cli_trace_json_stdout_clean",
    category: "trace",
    capability: "explain_condition",
    budget: "compact",
    ok: trace.ok,
    checks: trace.checks,
    timing: {
      stdout_bytes: trace.stdout_bytes,
      stderr_bytes: trace.stderr_bytes,
      stderr_line_count: trace.stderr_line_count,
    },
  });

  if (!trace.ok) {
    throw new Error("cli_trace_json_stdout_clean boundary validation failed");
  }

  const graphDbSizeBytes = existsSync(options.graphDbPath)
    ? statSync(options.graphDbPath).size
    : 0;
  const artifacts = writeArtifacts(options, records, { timing: timingSummary, trace }, {
    commit,
    buildBinary: buildBinaryResult,
    buildGraph: buildGraphResult,
    graphDbSizeBytes,
  });

  console.log(JSON.stringify({
    ok: true,
    commit,
    project_dir: options.projectDir,
    graph_db_path: options.graphDbPath,
    jsonl: artifacts.jsonlPath,
    markdown: artifacts.mdPath,
    timing: timingSummary,
    trace,
  }, null, 2));
}

try {
  main();
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}

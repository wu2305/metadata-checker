#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import { createWriteStream, existsSync, mkdirSync, rmSync, statSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const defaultProjectDir = "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi";
const defaultGraphDbPath = "/tmp/metadata-checker-m50-real-project.graphdb";
const defaultOutputDir = "/tmp/metadata-checker-m50-perf";

const scenarios = [
  {
    id: "explain_condition_input3_writer",
    category: "condition",
    command: "explain_condition",
    target: "comp:app/销售.app/销售/合同协议.spg|input3",
    budget: "compact",
    intent: "writer",
  },
  {
    id: "context_input3_depth2",
    category: "context",
    command: "context",
    target: "comp:app/销售.app/销售/合同协议.spg|input3",
    budget: "normal",
    depth: 2,
  },
  {
    id: "query_model_fact_qwSidebar",
    category: "model",
    command: "query_model",
    target: "model:fact_qwSidebar",
    budget: "compact",
  },
  {
    id: "query_page_logic_contract",
    category: "page_logic",
    command: "query_page_logic",
    target: "page:app/销售.app/销售/合同协议.spg",
    budget: "compact",
  },
  {
    id: "explain_condition_text41_display",
    category: "condition",
    command: "explain_condition",
    target: "comp:app/售后.app/绑定车辆/会员已注册.spg|text41",
    budget: "compact",
    intent: "display",
  },
  {
    id: "explain_condition_text41_value_source",
    category: "condition",
    command: "explain_condition",
    target: "comp:app/售后.app/绑定车辆/会员已注册.spg|text41",
    budget: "compact",
    intent: "value-source",
  },
  {
    id: "explain_condition_model11_availability",
    category: "condition",
    command: "explain_condition",
    target: "model:app/售后.app/绑定车辆/会员已注册.spg|model11",
    budget: "compact",
    intent: "availability",
  },
  {
    id: "query_page_logic_member_registered",
    category: "page_logic",
    command: "query_page_logic",
    target: "page:app/售后.app/绑定车辆/会员已注册.spg",
    budget: "compact",
  },
  {
    id: "find_page_member_registered",
    category: "lookup",
    command: "find_page",
    target: "会员已注册",
    budget: "compact",
  },
  {
    id: "find_model_auto_customer_rel",
    category: "lookup",
    command: "find_model",
    target: "fact_autoCustomerAutoRel",
    budget: "compact",
  },
  {
    id: "find_component_text41",
    category: "lookup",
    command: "find_component",
    target: "text41",
    budget: "compact",
  },
  {
    id: "status_warm_runtime",
    category: "lifecycle",
    command: "status",
  },
];

function parseArgs(argv) {
  const options = {
    projectDir: defaultProjectDir,
    graphDbPath: defaultGraphDbPath,
    outputDir: defaultOutputDir,
    iterations: 5,
    profile: "release-fast",
    bin: null,
    rebuildGraph: false,
    skipBuildBin: false,
    skipBuildGraph: false,
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
      case "--iterations":
        options.iterations = Number.parseInt(next(), 10);
        break;
      case "--profile":
        options.profile = next();
        break;
      case "--bin":
        options.bin = next();
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
  options.projectDir = resolve(options.projectDir);
  options.graphDbPath = resolve(options.graphDbPath);
  options.outputDir = resolve(options.outputDir);
  options.bin = options.bin
    ? resolve(options.bin)
    : resolve(repoRoot, "target", options.profile, "metadata-checker");
  return options;
}

function printHelp() {
  console.log(`Usage: node tools/m50-real-project-perf.mjs [options]

Build or reuse the real xiaoshouyi graphdb, replay standard M50 stdio scenarios,
and write JSONL + Markdown artifacts.

Options:
  --project-dir DIR       Real project directory
  --graph-db-path PATH    GraphDB path (default: ${defaultGraphDbPath})
  --output-dir DIR        Artifact directory (default: ${defaultOutputDir})
  --iterations N          Iterations per scenario (default: 5)
  --profile NAME          Cargo profile for binary build (default: release-fast)
  --bin PATH              Use an existing metadata-checker binary
  --rebuild-graph         Delete and rebuild graphdb before running
  --skip-build-bin        Do not run cargo build
  --skip-build-graph      Do not build graphdb even if missing
  -h, --help              Show this help
`);
}

function runChecked(command, args, options = {}) {
  const startedAt = performance.now();
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: options.stdio ?? "pipe",
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

function buildBinary(options) {
  if (options.skipBuildBin) {
    if (!existsSync(options.bin)) {
      throw new Error(`binary not found: ${options.bin}`);
    }
    return { skipped: true, wallMs: 0 };
  }

  const args = options.profile === "release"
    ? ["build", "--release"]
    : ["build", "--profile", options.profile];
  const result = runChecked("cargo", args, { stdio: "inherit" });
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
    "--project-dir",
    options.projectDir,
    "--graph-db-path",
    options.graphDbPath,
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

function requestPayload(scenario, iteration) {
  const payload = {
    request_id: `${scenario.id}_${iteration}`,
    command: scenario.command,
    target: scenario.target,
    budget: scenario.budget,
  };
  if (scenario.intent) {
    payload.intent = scenario.intent;
  }
  if (scenario.depth !== undefined) {
    payload.depth = scenario.depth;
  }
  if (scenario.pageScope) {
    payload.page_scope = scenario.pageScope;
  }
  return payload;
}

function runStdioReplay(options) {
  const startedAt = performance.now();
  const child = spawn(options.bin, [
    "--serve-stdio",
    "--graph-db-path",
    options.graphDbPath,
    "--project-dir",
    options.projectDir,
  ], {
    cwd: repoRoot,
    stdio: ["pipe", "pipe", "pipe"],
  });

  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    stdout += chunk;
  });
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });

  for (let iteration = 1; iteration <= options.iterations; iteration += 1) {
    for (const scenario of scenarios) {
      child.stdin.write(`${JSON.stringify(requestPayload(scenario, iteration))}\n`);
    }
  }
  child.stdin.end();

  return new Promise((resolvePromise, rejectPromise) => {
    child.on("error", rejectPromise);
    child.on("close", (code) => {
      const wallMs = Math.round(performance.now() - startedAt);
      if (code !== 0) {
        rejectPromise(new Error(`stdio replay failed with code ${code}\n${stderr}`));
        return;
      }
      resolvePromise({ stdout, stderr, wallMs });
    });
  });
}

function parseResponses(stdout) {
  return stdout
    .split(/\r?\n/)
    .filter((line) => line.trim().length > 0)
    .map((line) => JSON.parse(line));
}

function percentile(values, p) {
  if (values.length === 0) {
    return null;
  }
  const sorted = [...values].sort((a, b) => a - b);
  const index = Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1);
  return sorted[index];
}

function aggregate(records) {
  return scenarios.map((scenario) => {
    const rows = records.filter((record) => record.scenario === scenario.id);
    const totals = rows.map((record) => record.timing.total_ms);
    const compute = rows.map((record) => record.timing.query_compute_ms);
    const bytes = rows.map((record) => record.timing.output_size_bytes);
    return {
      scenario: scenario.id,
      category: scenario.category,
      command: scenario.command,
      target: scenario.target,
      iterations: rows.length,
      ok: rows.filter((record) => record.ok).length,
      p50_ms: percentile(totals, 50),
      p95_ms: percentile(totals, 95),
      max_ms: totals.length > 0 ? Math.max(...totals) : null,
      query_compute_p95_ms: percentile(compute, 95),
      output_size_max_bytes: bytes.length > 0 ? Math.max(...bytes) : null,
    };
  });
}

function writeArtifacts(options, records, summary, meta) {
  mkdirSync(options.outputDir, { recursive: true });
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const jsonlPath = resolve(options.outputDir, `m50-real-project-${stamp}.jsonl`);
  const mdPath = resolve(options.outputDir, `m50-real-project-${stamp}.md`);

  const jsonl = createWriteStream(jsonlPath, { encoding: "utf8" });
  for (const record of records) {
    jsonl.write(`${JSON.stringify(record)}\n`);
  }
  jsonl.end();

  const lines = [
    "# M50 真实项目性能测试",
    "",
    `- commit: \`${meta.commit}\``,
    `- project_dir: \`${options.projectDir}\``,
    `- graph_db_path: \`${options.graphDbPath}\``,
    `- binary: \`${options.bin}\``,
    `- iterations_per_scenario: ${options.iterations}`,
    `- build_binary_wall_ms: ${meta.buildBinary.wallMs}${meta.buildBinary.skipped ? " (skipped)" : ""}`,
    `- build_graph_wall_ms: ${meta.buildGraph.wallMs}${meta.buildGraph.skipped ? " (skipped)" : ""}`,
    `- stdio_process_wall_ms: ${meta.stdioWallMs}`,
    `- graph_db_size_bytes: ${meta.graphDbSizeBytes}`,
    "",
    "| category | scenario | ok/total | p50_ms | p95_ms | max_ms | query_compute_p95_ms | output_size_max_bytes |",
    "|---|---|---:|---:|---:|---:|---:|---:|",
    ...summary.map((row) => (
      `| ${row.category} | ${row.scenario} | ${row.ok}/${row.iterations} | ${row.p50_ms ?? ""} | ${row.p95_ms ?? ""} | ${row.max_ms ?? ""} | ${row.query_compute_p95_ms ?? ""} | ${row.output_size_max_bytes ?? ""} |`
    )),
    "",
    `JSONL: \`${jsonlPath}\``,
  ];
  createWriteStream(mdPath, { encoding: "utf8" }).end(`${lines.join("\n")}\n`);

  return { jsonlPath, mdPath };
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (!existsSync(options.projectDir)) {
    throw new Error(`real project directory not found: ${options.projectDir}`);
  }

  const commit = currentCommit();
  const buildBinaryResult = buildBinary(options);
  const buildGraphResult = buildGraph(options);
  const replay = await runStdioReplay(options);
  const responses = parseResponses(replay.stdout);
  const expectedResponses = scenarios.length * options.iterations;
  if (responses.length !== expectedResponses) {
    throw new Error(`expected ${expectedResponses} responses, got ${responses.length}`);
  }

  const byRequestId = new Map(responses.map((response) => [response.request_id, response]));
  const measuredAt = new Date().toISOString();
  const records = [
    {
      schema_version: 1,
      record_type: "lifecycle",
      runner: "m50-real-project-perf",
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
      runner: "m50-real-project-perf",
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
    {
      schema_version: 1,
      record_type: "lifecycle",
      runner: "m50-real-project-perf",
      commit,
      measured_at: measuredAt,
      project_dir: options.projectDir,
      graph_db_path: options.graphDbPath,
      binary: options.bin,
      mode: "stdio_warm",
      scenario: "stdio_process",
      category: "lifecycle",
      ok: true,
      timing: { wall_ms: replay.wallMs },
    },
  ];
  for (let iteration = 1; iteration <= options.iterations; iteration += 1) {
    for (const scenario of scenarios) {
      const request = requestPayload(scenario, iteration);
      const response = byRequestId.get(request.request_id);
      const timing = response?.timing ?? {};
      records.push({
        schema_version: 1,
        record_type: "capability",
        runner: "m50-real-project-perf",
        commit,
        measured_at: measuredAt,
        project_dir: options.projectDir,
        graph_db_path: options.graphDbPath,
        binary: options.bin,
        mode: "stdio_warm",
        scenario: scenario.id,
        category: scenario.category,
        iteration,
        request,
        ok: response?.ok === true,
        error: response?.error ?? null,
        timing,
        diagnostics: response?.diagnostics ?? [],
      });
    }
  }

  const failed = records.filter((record) => !record.ok);
  if (failed.length > 0) {
    throw new Error(`stdio replay had ${failed.length} failed responses`);
  }

  const summary = aggregate(records);
  const graphDbSizeBytes = existsSync(options.graphDbPath)
    ? statSync(options.graphDbPath).size
    : 0;
  const artifacts = writeArtifacts(options, records, summary, {
    commit,
    buildBinary: buildBinaryResult,
    buildGraph: buildGraphResult,
    stdioWallMs: replay.wallMs,
    graphDbSizeBytes,
  });

  console.log(JSON.stringify({
    ok: true,
    commit,
    project_dir: options.projectDir,
    graph_db_path: options.graphDbPath,
    jsonl: artifacts.jsonlPath,
    markdown: artifacts.mdPath,
    summary,
  }, null, 2));
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});

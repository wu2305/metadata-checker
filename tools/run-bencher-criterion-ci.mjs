#!/usr/bin/env node

import { createWriteStream, existsSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { spawn, spawnSync } from "node:child_process";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const defaultReportDir = join(repoRoot, "target/criterion-ci/reports");

function parseArgs(argv) {
  const options = {
    bench: null,
    targetDir: null,
    reportDir: defaultReportDir,
    features: null,
    criterionArgs: ["--color", "never"],
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--") {
      options.criterionArgs.push(...argv.slice(index + 1));
      break;
    }

    const next = () => {
      index += 1;
      if (index >= argv.length) {
        throw new Error(`missing value for ${arg}`);
      }
      return argv[index];
    };

    switch (arg) {
      case "--bench":
        options.bench = next();
        break;
      case "--target-dir":
        options.targetDir = next();
        break;
      case "--report-dir":
        options.reportDir = resolve(next());
        break;
      case "--features":
        options.features = next();
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

  if (!options.bench) {
    throw new Error("--bench is required");
  }
  if (!options.targetDir) {
    options.targetDir = `target/criterion-ci/${options.bench}`;
  }

  return options;
}

function printHelp() {
  console.log(`Usage: node tools/run-bencher-criterion-ci.mjs --bench NAME [options] [-- criterion args]

Options:
  --bench NAME       Cargo bench target name
  --target-dir DIR   CARGO_TARGET_DIR for this bench
  --report-dir DIR   Directory for captured Criterion stdout/stderr
  --features LIST    Cargo feature list passed to cargo bench
`);
}

function run(command, args, options = {}) {
  return spawnSync(command, args, {
    cwd: repoRoot,
    env: { ...process.env, ...(options.env ?? {}) },
    encoding: "utf8",
    maxBuffer: 1024 * 1024 * 128,
  });
}

function bencherConfigured() {
  return Boolean(process.env.BENCHER_API_KEY && process.env.BENCHER_PROJECT);
}

function reportToBencher(reportPath, benchName) {
  if (!bencherConfigured()) {
    console.log("skip Bencher report: BENCHER_API_KEY or BENCHER_PROJECT is not configured");
    return;
  }

  const args = [
    "run",
    "--project",
    process.env.BENCHER_PROJECT,
    "--branch",
    process.env.CNB_BRANCH ?? "local",
    "--testbed",
    process.env.BENCHER_TESTBED ?? "cnb-amd64",
    "--adapter",
    "rust_criterion",
    "--file",
    reportPath,
  ];
  const result = run("bencher", args);
  process.stdout.write(result.stdout ?? "");
  process.stderr.write(result.stderr ?? "");

  if (result.status !== 0) {
    if (process.env.BENCHER_UPLOAD_REQUIRED === "true") {
      process.exit(result.status ?? 1);
    }
    console.error(`warning: Bencher report failed for ${benchName}`);
  }
}

function runCargoBench(cargoArgs, env, reportPath) {
  return new Promise((resolve, reject) => {
    const report = createWriteStream(reportPath, { encoding: "utf8" });
    const child = spawn("cargo", cargoArgs, {
      cwd: repoRoot,
      env: { ...process.env, ...env },
      stdio: ["ignore", "pipe", "pipe"],
    });

    child.stdout.on("data", (chunk) => {
      process.stdout.write(chunk);
      report.write(chunk);
    });
    child.stderr.on("data", (chunk) => {
      process.stderr.write(chunk);
      report.write(chunk);
    });
    child.on("error", (error) => {
      report.end();
      reject(error);
    });
    child.on("close", (code) => {
      report.end(() => resolve(code ?? 1));
    });
  });
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  mkdirSync(options.reportDir, { recursive: true });
  const reportPath = join(options.reportDir, `${options.bench}.out`);

  const cargoArgs = ["bench"];
  if (options.features) {
    cargoArgs.push("--features", options.features);
  }
  cargoArgs.push("--bench", options.bench, "--", ...options.criterionArgs);

  const status = await runCargoBench(
    cargoArgs,
    { CARGO_TARGET_DIR: options.targetDir },
    reportPath,
  );
  if (status !== 0) {
    process.exit(status);
  }

  if (!existsSync(reportPath)) {
    throw new Error(`report not written: ${reportPath}`);
  }
  reportToBencher(reportPath, options.bench);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  });
}

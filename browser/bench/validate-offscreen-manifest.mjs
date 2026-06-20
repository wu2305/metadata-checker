#!/usr/bin/env node

import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const defaultFixtureDir = join(repoRoot, "tests/fixtures/browser-offscreen-real-project");

const SAMPLE_IDS = [
  "typical_p75_page",
  "large_raw_page",
  "high_component_page",
  "high_reference_page",
  "worst_combined_page",
];

const COMPONENT_KINDS = [
  "high_reference_component",
  "container_component",
  "leaf_component",
];

function parseArgs(argv) {
  let fixtureDir = defaultFixtureDir;
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--fixture-dir") {
      index += 1;
      fixtureDir = resolve(argv[index]);
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      console.log("Usage: node browser/bench/validate-offscreen-manifest.mjs [--fixture-dir DIR]");
      process.exit(0);
    }
    throw new Error(`unknown argument: ${arg}`);
  }
  return { fixtureDir };
}

export function validateOffscreenManifest(fixtureDir) {
  const errors = [];
  const manifestPath = join(fixtureDir, "manifest.json");
  if (!existsSync(manifestPath)) {
    return { ok: false, errors: [`manifest not found: ${manifestPath}`] };
  }

  let manifest;
  try {
    manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  } catch (error) {
    return {
      ok: false,
      errors: [`manifest JSON parse failed: ${error instanceof Error ? error.message : String(error)}`],
    };
  }

  if (manifest.schema_version !== 1) {
    errors.push("manifest.schema_version must be 1");
  }
  if (typeof manifest.project_name !== "string" || manifest.project_name.length === 0) {
    errors.push("manifest.project_name must be a non-empty string");
  }
  if (!Array.isArray(manifest.samples) || manifest.samples.length < SAMPLE_IDS.length) {
    errors.push(`manifest.samples must contain at least ${SAMPLE_IDS.length} entries`);
  }

  const fixtureRoot = join(fixtureDir, manifest.fixture_root || "project");
  const seenIds = new Set();
  for (const sample of manifest.samples || []) {
    if (!SAMPLE_IDS.includes(sample.id)) {
      errors.push(`unexpected sample id: ${sample.id}`);
    }
    if (seenIds.has(sample.id)) {
      errors.push(`duplicate sample id: ${sample.id}`);
    }
    seenIds.add(sample.id);
    if (typeof sample.source_path !== "string" || sample.source_path.length === 0) {
      errors.push(`sample ${sample.id} missing source_path`);
    }
    const fixturePath = join(fixtureRoot, sample.source_path || "");
    if (!existsSync(fixturePath)) {
      errors.push(`fixture file missing for ${sample.id}: ${fixturePath}`);
    }
    for (const kind of COMPONENT_KINDS) {
      const candidate = sample.components?.[kind];
      if (!candidate || typeof candidate.id !== "string" || candidate.id.length === 0) {
        errors.push(`sample ${sample.id} missing components.${kind}.id`);
      }
    }
    if (JSON.stringify(sample).includes('"raw"')) {
      errors.push(`manifest must not embed raw metadata for ${sample.id}`);
    }
  }

  for (const sampleId of SAMPLE_IDS) {
    if (!seenIds.has(sampleId)) {
      errors.push(`missing required sample id: ${sampleId}`);
    }
  }

  return {
    ok: errors.length === 0,
    errors,
    manifestPath,
    fixtureRoot,
    sampleCount: manifest.samples?.length ?? 0,
  };
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const result = validateOffscreenManifest(options.fixtureDir);
  if (!result.ok) {
    console.error(JSON.stringify(result, null, 2));
    process.exit(1);
  }
  console.log(JSON.stringify({ ok: true, ...result }, null, 2));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}

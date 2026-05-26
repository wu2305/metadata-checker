import { access, cp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";

import { copyDirectoryInto, patchManifest, stableDiagnostic } from "./prepare-extension-package.mjs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const DEFAULT_OUT_DIR = join(tmpdir(), "metadata-checker-extension-safari-staging");
const DEFAULT_BROWSER_ROOT = join(__dirname, "..");
const DEFAULT_CORE_DIR = join(DEFAULT_BROWSER_ROOT, "extension-core");
const DEFAULT_SAFARI_DIR = join(DEFAULT_BROWSER_ROOT, "extension-safari");
const DEFAULT_VERSION = "0.0.0";
const DEFAULT_HOST_MATCH = "https://autocrm-test.xiaoshouyi.com/*";
const TEMPLATE_MISSING_CODE = "SAFARI_TEMPLATE_MISSING";

const DEFAULT_SAFARI_NOTES = [
  "# Safari staging package notes",
  "",
  "This package is for Safari staging only.",
  "Please export to Xcode and finish conversion/signing in a Safari-compatible flow manually.",
  "This project deliberately avoids invoking Xcode tooling in packaging scripts.",
  "Inspect `info.plist` and adapter notes before submitting to App Store.",
].join("\n");

async function collectFiles(root, base = "") {
  const items = await readdir(root, { withFileTypes: true });
  const collected = [];
  for (const item of items) {
    if (item.name === ".DS_Store") {
      continue;
    }
    if (item.isDirectory()) {
      collected.push(...await collectFiles(join(root, item.name), join(base, item.name)));
      continue;
    }
    collected.push(join(base, item.name));
  }
  return collected.sort();
}

async function ensureSafariNotes(outDir) {
  const notesPath = join(outDir, "safari-notes.md");
  await writeFile(notesPath, `${DEFAULT_SAFARI_NOTES}\n`, "utf8");
  return "safari-notes.md";
}

function manifestTarget(sourceFileName) {
  if (sourceFileName === "manifest.template.json") {
    return "manifest.json";
  }
  return sourceFileName;
}

async function applyManifestFromTemplate(safariDir, outDir, options, diagnostics) {
  const sourceTemplate = join(safariDir, "manifest.template.json");
  const sourceManifest = join(safariDir, "manifest.json");
  let manifestSource = null;
  let sourceName = null;

  try {
    manifestSource = await readFile(sourceTemplate, "utf8");
    sourceName = "manifest.template.json";
  } catch {
    try {
      manifestSource = await readFile(sourceManifest, "utf8");
      sourceName = "manifest.json";
    } catch {
      diagnostics.push(stableDiagnostic("SAFARI_MANIFEST_MISSING", "Safari manifest source not found"));
      return null;
    }
  }

  let manifest;
  try {
    manifest = JSON.parse(manifestSource);
  } catch {
    diagnostics.push(stableDiagnostic("MANIFEST_PARSE_FAILED", "cannot parse Safari manifest source"));
    return null;
  }

  const patched = patchManifest(manifest, {
    version: options.version || DEFAULT_VERSION,
    hostMatch: options.hostMatch || DEFAULT_HOST_MATCH,
  });
  const target = join(outDir, manifestTarget(sourceName));
  await writeFile(target, `${JSON.stringify(patched, null, 2)}\n`, "utf8");
  return manifestTarget(sourceName);
}

function existsSyncLike(path) {
  return access(path).then(() => true, () => false);
}

async function prepareSafariExtensionPackage(options = {}) {
  const outDir = resolve(options.outDir || DEFAULT_OUT_DIR);
  const coreDir = resolve(options.extensionCoreDir || DEFAULT_CORE_DIR);
  const safariDir = resolve(options.extensionSafariDir || DEFAULT_SAFARI_DIR);
  const clean = options.clean ?? true;
  const diagnostics = [];

  if (clean) {
    await rm(outDir, { force: true, recursive: true });
  }

  await copyDirectoryInto(coreDir, outDir, "extension-core", "extension-core copy failed");
  try {
    await cp(safariDir, outDir, { recursive: true, force: true });
  } catch {
    diagnostics.push(
      stableDiagnostic(
        TEMPLATE_MISSING_CODE,
        "extension-safari template is not available; generating safari-notes.md only.",
      ),
    );
    const files = [await ensureSafariNotes(outDir), ...(await collectFiles(outDir))];
    return {
      outDir,
      files,
      diagnostics,
      manifest_path: null,
      notes: "safari-notes.md",
    };
  }

  const manifestWritten = await applyManifestFromTemplate(safariDir, outDir, options, diagnostics);
  const notesPath = `${outDir}/safari-notes.md`;
  let notesWritten = false;
  if (!await existsSyncLike(notesPath)) {
    await ensureSafariNotes(outDir);
    notesWritten = true;
  }

  const files = await collectFiles(outDir);
  return {
    outDir,
    files,
    diagnostics,
    manifest_path: manifestWritten ? manifestWritten : null,
    notes: notesWritten ? "safari-notes.md" : undefined,
  };
}

function parseArgs(argv) {
  const args = {};
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (!token.startsWith("--")) {
      throw new Error(`unexpected argument: ${token}`);
    }
    if (token === "--no-clean") {
      args.clean = false;
      continue;
    }
    const value = argv[index + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new Error(`missing value for ${token}`);
    }
    args[token.slice(2).replaceAll("-", "_")] = value;
    index += 1;
  }
  return args;
}

async function main(argv = process.argv.slice(2)) {
  const args = parseArgs(argv);
  const result = await prepareSafariExtensionPackage({
    outDir: args.out_dir,
    version: args.version,
    hostMatch: args.host_match,
    extensionCoreDir: args.extension_core_dir,
    extensionSafariDir: args.extension_safari_dir,
    clean: args.clean ?? true,
  });
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}

export { parseArgs, prepareSafariExtensionPackage };

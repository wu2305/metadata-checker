import { cp, copyFile, mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";

const __dirname = dirname(fileURLToPath(import.meta.url));
const DEFAULT_OUT_DIR = join(tmpdir(), "metadata-checker-extension-chromium");
const DEFAULT_BROWSER_ROOT = join(__dirname, "..");
const DEFAULT_CORE_DIR = join(DEFAULT_BROWSER_ROOT, "extension-core");
const DEFAULT_CHROMIUM_DIR = join(DEFAULT_BROWSER_ROOT, "extension-chromium");
const DEFAULT_HOST_MATCH = "https://autocrm-test.xiaoshouyi.com/*";
const DEFAULT_VERSION = "0.0.0";
const ZIP_DIAGNOSTIC_CODE = "ZIP_PACKAGING_NOT_SUPPORTED";

function stableDiagnostic(code, message, severity = "warning") {
  return {
    severity,
    code,
    message,
  };
}

function parseHostMatches(hostMatch) {
  if (!hostMatch || hostMatch.trim() === "") {
    return [DEFAULT_HOST_MATCH];
  }
  return hostMatch
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

function patchManifest(manifest, { version, hostMatch }) {
  const patched = { ...manifest };
  const matches = parseHostMatches(hostMatch);

  if (version) {
    patched.version = version;
  }
  patched.host_permissions = matches;

  const contentScripts = Array.isArray(patched.content_scripts) ? patched.content_scripts : [];
  patched.content_scripts = contentScripts.map((script) => {
    if (!script || typeof script !== "object") {
      return script;
    }
    return { ...script, matches };
  });
  if (patched.content_scripts.length === 0) {
    patched.content_scripts = [{ matches, js: [] }];
  }

  return patched;
}

async function collectFiles(root, base = "") {
  const items = await readdir(root, { withFileTypes: true });
  const collected = [];

  for (const item of items) {
    if (item.name === ".DS_Store") {
      continue;
    }
    const fullPath = join(root, item.name);
    if (item.isDirectory()) {
      const child = await collectFiles(fullPath, join(base, item.name));
      collected.push(...child);
      continue;
    }
    collected.push(join(base, item.name));
  }

  return collected.sort();
}

async function safeCopyDir(sourceDir, outDir, label) {
  try {
    await cp(sourceDir, outDir, { recursive: true, force: true });
  } catch (error) {
    throw new Error(`${label}: ${error.message}`);
  }
}

async function copyDirectoryInto(sourceDir, outDir, targetName, label) {
  const targetDir = join(outDir, targetName);
  await mkdir(dirname(targetDir), { recursive: true });
  await safeCopyDir(sourceDir, targetDir, label);
  return targetDir;
}

async function copyInputFile(sourceFile, outDir) {
  const target = join(outDir, basename(sourceFile));
  await copyFile(sourceFile, target);
  return target;
}

async function writePatchedManifest(manifestPath, options, diagnostics) {
  const source = await readFile(manifestPath, "utf8");
  let manifest;
  try {
    manifest = JSON.parse(source);
  } catch {
    diagnostics.push(stableDiagnostic("MANIFEST_PARSE_FAILED", "cannot parse manifest.json"));
    throw new Error("manifest parse failed");
  }

  const patched = patchManifest(manifest, options);
  await writeFile(manifestPath, `${JSON.stringify(patched, null, 2)}\n`, "utf8");
}

function reportZipUnavailable() {
  return stableDiagnostic(
    ZIP_DIAGNOSTIC_CODE,
    "zip packaging is not available without external tooling; unpacked extension package is ready.",
  );
}

async function prepareExtensionPackage(options = {}) {
  const outDir = resolve(options.outDir || DEFAULT_OUT_DIR);
  const coreDir = resolve(options.extensionCoreDir || DEFAULT_CORE_DIR);
  const chromiumDir = resolve(options.extensionChromiumDir || DEFAULT_CHROMIUM_DIR);
  const clean = options.clean ?? true;
  const diagnostics = [];

  if (clean) {
    await rm(outDir, { force: true, recursive: true });
  }
  await mkdir(outDir, { recursive: true });

  await copyDirectoryInto(coreDir, outDir, "extension-core", "extension-core copy failed");
  await safeCopyDir(chromiumDir, outDir, "extension-chromium copy failed");

  const manifestPath = join(outDir, "manifest.json");
  await writePatchedManifest(manifestPath, {
    version: options.version || DEFAULT_VERSION,
    hostMatch: options.hostMatch,
  }, diagnostics);

  if (options.wasmBindgenJs) {
    await copyInputFile(options.wasmBindgenJs, outDir);
  }

  if (options.wasmFile) {
    const target = await copyInputFile(options.wasmFile, outDir);
    const binary = await readFile(options.wasmFile);
    await writeFile(target, binary);
  }

  diagnostics.push(reportZipUnavailable());

  const files = await collectFiles(outDir);
  return {
    outDir,
    files,
    core_files: files.filter((file) => file.startsWith("extension-core/")),
    diagnostics,
    manifest_path: "manifest.json",
    zip: { status: "skipped", reason: ZIP_DIAGNOSTIC_CODE },
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
  const result = await prepareExtensionPackage({
    outDir: args.out_dir,
    version: args.version,
    hostMatch: args.host_match,
    wasmBindgenJs: args.wasm_bindgen_js,
    wasmFile: args.wasm_file,
    extensionCoreDir: args.extension_core_dir,
    extensionChromiumDir: args.extension_chromium_dir,
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

export {
  copyDirectoryInto,
  parseArgs,
  parseHostMatches,
  patchManifest,
  prepareExtensionPackage,
  stableDiagnostic,
};

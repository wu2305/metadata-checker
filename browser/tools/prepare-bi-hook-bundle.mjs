import { copyFile, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { basename, dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const DEFAULT_BROWSER_ROOT = join(__dirname, "..");
const DEFAULT_OUT_DIR = "/private/tmp/metadata-checker-bi-hooks";

const MODULE_FILES = [
  "metadata-checker-browser-entry.mjs",
  "plugin-core/metadata-checker-plugin.mjs",
  "integration/metadata-checker-controller.mjs",
  "platform-glue/superpage-designer-glue.mjs",
  "providers/page-rc-metadata-provider.mjs",
  "providers/fake-remote-metadata-provider.mjs",
  "renderer/echarts-runtime-resolver.mjs",
  "renderer/graph-panel-host.mjs",
  "renderer/graph-panel-renderer.mjs",
  "renderer/graph-layout.mjs",
  "renderer/graph-dom.mjs",
  "runtime-launchers/runtime-launcher.mjs",
  "runtime-launchers/page-runtime-launcher.mjs",
  "runtime-launchers/message-runtime-client.mjs",
  "runtime-launchers/web-worker-runtime-launcher.mjs",
  "runtime-launchers/service-worker-runtime-launcher.mjs",
  "runtime-launchers/browser-extension-runtime-launcher.mjs",
];

function rewriteModuleSpecifiers(source, options = {}) {
  const suffix = options.moduleVersion ? `.js?v=${encodeURIComponent(options.moduleVersion)}` : ".js";
  return source.replaceAll(".mjs", suffix);
}

function patchWasmBindgenNoModulesForWorker(source) {
  if (source.includes("self.wasm_bindgen")) {
    return source;
  }
  return `${source}\n;if (typeof self !== "undefined" && typeof wasm_bindgen === "function") self.wasm_bindgen = wasm_bindgen;\n`;
}

function toPublishedPath(modulePath) {
  return modulePath.replace(/\.mjs$/, ".js");
}

async function prepareBiHookBundle(options = {}) {
  const browserRoot = options.browserRoot ?? DEFAULT_BROWSER_ROOT;
  const outDir = options.outDir ?? DEFAULT_OUT_DIR;
  const clean = options.clean ?? true;

  if (clean) {
    await rm(outDir, { recursive: true, force: true });
  }
  await mkdir(outDir, { recursive: true });

  const outputs = [];
  for (const modulePath of MODULE_FILES) {
    const sourcePath = join(browserRoot, modulePath);
    const publishedPath = toPublishedPath(modulePath);
    const targetPath = join(outDir, publishedPath);
    const source = await readFile(sourcePath, "utf8");
    await mkdir(dirname(targetPath), { recursive: true });
    await writeFile(
      targetPath,
      rewriteModuleSpecifiers(source, { moduleVersion: options.moduleVersion }),
      "utf8",
    );
    outputs.push({
      sourcePath,
      targetPath,
      relativePath: publishedPath,
    });
  }

  const customSource = join(browserRoot, "tools", "metadata-checker-custom.js");
  const customTarget = join(outDir, "custom.js");
  await writeFile(customTarget, await readFile(customSource, "utf8"), "utf8");
  outputs.push({
    sourcePath: customSource,
    targetPath: customTarget,
    relativePath: "custom.js",
  });

  const serviceWorkerSource = join(browserRoot, "service-worker", "metadata-checker-sw.js");
  const serviceWorkerTarget = join(outDir, "metadata-checker-sw.js");
  await writeFile(serviceWorkerTarget, await readFile(serviceWorkerSource, "utf8"), "utf8");
  outputs.push({
    sourcePath: serviceWorkerSource,
    targetPath: serviceWorkerTarget,
    relativePath: "metadata-checker-sw.js",
  });

  if (options.wasmBindgenJs) {
    const wasmBindgenTarget = join(outDir, basename(options.wasmBindgenJs));
    const patchedGlue = patchWasmBindgenNoModulesForWorker(
      await readFile(options.wasmBindgenJs, "utf8"),
    );
    await writeFile(wasmBindgenTarget, patchedGlue, "utf8");
    outputs.push({
      sourcePath: options.wasmBindgenJs,
      targetPath: wasmBindgenTarget,
      relativePath: basename(options.wasmBindgenJs),
    });
  }

  if (options.wasmFile) {
    const wasmTarget = join(outDir, basename(options.wasmFile));
    await copyFile(options.wasmFile, wasmTarget);
    outputs.push({
      sourcePath: options.wasmFile,
      targetPath: wasmTarget,
      relativePath: basename(options.wasmFile),
    });
  }

  return {
    outDir,
    files: outputs.map((item) => ({
      ...item,
      displayPath: relative(process.cwd(), item.targetPath),
    })),
  };
}

function parseArgs(argv) {
  const args = {};
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    if (key === "--no-clean") {
      args.clean = false;
      continue;
    }
    if (!key.startsWith("--")) {
      throw new Error(`unexpected argument: ${key}`);
    }
    const value = argv[index + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new Error(`missing value for ${key}`);
    }
    index += 1;
    args[key.slice(2).replaceAll("-", "_")] = value;
  }
  return args;
}

async function main(argv = process.argv.slice(2)) {
  const args = parseArgs(argv);
  const result = await prepareBiHookBundle({
    browserRoot: args.browser_root,
    outDir: args.out_dir,
    clean: args.clean ?? true,
    wasmBindgenJs: args.wasm_bindgen_js,
    wasmFile: args.wasm_file,
    moduleVersion: args.module_version,
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
  MODULE_FILES,
  parseArgs,
  prepareBiHookBundle,
  patchWasmBindgenNoModulesForWorker,
  rewriteModuleSpecifiers,
  toPublishedPath,
};

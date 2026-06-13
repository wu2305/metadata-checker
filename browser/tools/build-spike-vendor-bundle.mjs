import { mkdir } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import * as esbuild from "esbuild";

const __dirname = dirname(fileURLToPath(import.meta.url));
const BROWSER_ROOT = join(__dirname, "..");
const DEFAULT_OUT_DIR = join(
  BROWSER_ROOT,
  "artifacts",
  "metadata-checker-extension-chromium-m47-pixi-spike",
  "spike-vendor",
);

function parseArgs(argv) {
  const args = {};
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (token === "--") {
      continue;
    }
    if (!token.startsWith("--")) {
      throw new Error(`unexpected argument: ${token}`);
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

async function buildSpikeVendorBundles(options = {}) {
  const outDir = resolve(options.outDir || DEFAULT_OUT_DIR);
  const commonOptions = {
    bundle: true,
    format: "esm",
    platform: "browser",
    target: "chrome120",
    minify: true,
    sourcemap: false,
    logLevel: "silent",
  };

  await mkdir(outDir, { recursive: true });
  await esbuild.build({
    ...commonOptions,
    entryPoints: [join(BROWSER_ROOT, "spike-vendor", "pixi-entry.mjs")],
    outfile: join(outDir, "pixi-bundle.mjs"),
  });
  await esbuild.build({
    ...commonOptions,
    entryPoints: [join(BROWSER_ROOT, "spike-vendor", "d3-force-entry.mjs")],
    outfile: join(outDir, "d3-force-3d-bundle.mjs"),
  });
  await esbuild.build({
    ...commonOptions,
    entryPoints: [join(BROWSER_ROOT, "spike-vendor", "echarts-entry.mjs")],
    outfile: join(outDir, "echarts-bundle.mjs"),
  });

  return {
    outDir,
    files: [
      "pixi-bundle.mjs",
      "d3-force-3d-bundle.mjs",
      "echarts-bundle.mjs",
    ],
  };
}

async function main(argv = process.argv.slice(2)) {
  const args = parseArgs(argv);
  const result = await buildSpikeVendorBundles({
    outDir: args.out_dir,
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
  buildSpikeVendorBundles,
  parseArgs,
};

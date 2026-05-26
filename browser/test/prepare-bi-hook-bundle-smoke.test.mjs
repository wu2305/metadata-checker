import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import test from "node:test";
import {
  MODULE_FILES,
  patchWasmBindgenNoModulesForWorker,
  parseArgs,
  prepareBiHookBundle,
  rewriteModuleSpecifiers,
  toPublishedPath,
} from "../tools/prepare-bi-hook-bundle.mjs";

test("rewriteModuleSpecifiers rewrites browser ESM sidecar suffixes for BI MIME rules", () => {
  const source = 'import "./plugin-core/metadata-checker-plugin.mjs";\nconst value = "x.mjs";';

  assert.equal(
    rewriteModuleSpecifiers(source),
    'import "./plugin-core/metadata-checker-plugin.js";\nconst value = "x.js";',
  );
});

test("rewriteModuleSpecifiers can append a cache-busting module version", () => {
  const source = 'import "./integration/metadata-checker-controller.mjs";';

  assert.equal(
    rewriteModuleSpecifiers(source, { moduleVersion: "m42 options" }),
    'import "./integration/metadata-checker-controller.js?v=m42%20options";',
  );
});

test("toPublishedPath changes only mjs module suffix to js", () => {
  assert.equal(toPublishedPath("metadata-checker-browser-entry.mjs"), "metadata-checker-browser-entry.js");
  assert.equal(toPublishedPath("renderer/graph-dom.mjs"), "renderer/graph-dom.js");
});

test("parseArgs supports out dir and no-clean", () => {
  assert.deepEqual(parseArgs(["--out-dir", "/tmp/hooks", "--no-clean", "--wasm-bindgen-js", "/tmp/metadata_checker.js", "--wasm-file", "/tmp/metadata_checker_bg.wasm", "--module-version", "m42"]), {
    out_dir: "/tmp/hooks",
    clean: false,
    wasm_bindgen_js: "/tmp/metadata_checker.js",
    wasm_file: "/tmp/metadata_checker_bg.wasm",
    module_version: "m42",
  });
});

test("patchWasmBindgenNoModulesForWorker exposes no-modules glue on self", () => {
  const source = "let wasm_bindgen = (function(exports) { return exports; })({});";
  const patched = patchWasmBindgenNoModulesForWorker(source);

  assert.match(patched, /self\.wasm_bindgen = wasm_bindgen/);
  assert.equal(patchWasmBindgenNoModulesForWorker(patched), patched);
});

test("prepareBiHookBundle publishes JS sidecars without mjs imports", async () => {
  const outDir = await mkdtemp(join(tmpdir(), "metadata-checker-bi-hooks-test-"));
  try {
    const result = await prepareBiHookBundle({ outDir });
    const publishedEntry = join(outDir, "metadata-checker-browser-entry.js");
    const entry = await readFile(publishedEntry, "utf8");
    const custom = await readFile(join(outDir, "custom.js"), "utf8");

    assert.equal(result.files.length, MODULE_FILES.length + 2);
    assert.match(entry, /metadata-checker-controller\.js/);
    assert.doesNotMatch(entry, /\.mjs/);
    assert.match(custom, /metadata-checker-browser-entry\.js/);
  } finally {
    await rm(outDir, { recursive: true, force: true });
  }
});

test("prepareBiHookBundle can include patched wasm-bindgen glue and wasm bytes", async () => {
  const tempDir = await mkdtemp(join(tmpdir(), "metadata-checker-wasm-input-"));
  const outDir = await mkdtemp(join(tmpdir(), "metadata-checker-bi-hooks-test-"));
  try {
    const gluePath = join(tempDir, "metadata_checker.js");
    const wasmPath = join(tempDir, "metadata_checker_bg.wasm");
    await writeFile(gluePath, "let wasm_bindgen = function() {};\\n", "utf8");
    await writeFile(wasmPath, "wasm-bytes");

    await prepareBiHookBundle({
      outDir,
      wasmBindgenJs: gluePath,
      wasmFile: wasmPath,
    });

    const publishedGlue = await readFile(join(outDir, "metadata_checker.js"), "utf8");
    const publishedWasm = await readFile(join(outDir, "metadata_checker_bg.wasm"), "utf8");
    assert.match(publishedGlue, /self\.wasm_bindgen = wasm_bindgen/);
    assert.equal(publishedWasm, "wasm-bytes");
  } finally {
    await rm(tempDir, { recursive: true, force: true });
    await rm(outDir, { recursive: true, force: true });
  }
});

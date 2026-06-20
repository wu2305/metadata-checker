/**
 * Node offscreen local graph host：复用 extension offscreen 链路语义，但不依赖 chrome。
 */

import { cp, mkdtemp, readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";

const MAX_CACHE_ENTRIES = 8;
const requireFromHere = createRequire(import.meta.url);

function safeJsonParse(value) {
  try {
    return JSON.parse(value);
  } catch (_error) {
    return null;
  }
}

function rememberCacheEntry(cache, key, value) {
  if (cache.has(key)) {
    cache.delete(key);
  }
  cache.set(key, value);
  while (cache.size > MAX_CACHE_ENTRIES) {
    const oldestKey = cache.keys().next().value;
    cache.delete(oldestKey);
  }
}

function documentCacheKey(baseUrl, item = {}) {
  return [
    baseUrl || "",
    item.project_name || "",
    item.file_id || "",
    item.source_path || "",
    item.revision ?? "",
  ].join("\u001f");
}

async function loadWasmModule(wasmDir, nonce = "0") {
  const startedAt = performance.now();
  const tempDir = await mkdtemp(join(tmpdir(), `metadata-checker-wasm-${nonce}-`));
  await cp(join(wasmDir, "metadata_checker.js"), join(tempDir, "metadata_checker.js"));
  await cp(join(wasmDir, "metadata_checker_bg.wasm"), join(tempDir, "metadata_checker_bg.wasm"));
  const modulePath = join(tempDir, "metadata_checker.js");
  const wasmModule = requireFromHere(modulePath);
  return {
    wasmModule,
    wasmInitMs: Math.round(performance.now() - startedAt),
    tempDir,
  };
}

export async function createOffscreenLocalGraphHost({
  wasmDir,
  fixtureRoot,
  moduleNonce = "0",
  reloadWasm = false,
}) {
  let loaded = reloadWasm
    ? await loadWasmModule(wasmDir, moduleNonce)
    : null;
  let runtimePromise = null;
  let initializedProjectRef = null;
  const metadataTextCache = new Map();
  const loadedDocumentKeys = new Set();

  async function ensureRuntime() {
    const startedAt = performance.now();
    if (!loaded) {
      loaded = await loadWasmModule(wasmDir, moduleNonce);
    }
    if (!runtimePromise) {
      runtimePromise = Promise.resolve(loaded.wasmModule);
    }
    await runtimePromise;
    return Math.round(performance.now() - startedAt);
  }

  async function callWasm(method, args = []) {
    await ensureRuntime();
    const fn = loaded.wasmModule[method];
    if (typeof fn !== "function") {
      throw new Error(`WASM runtime method missing: ${method}`);
    }
    const result = await fn(...args);
    if (typeof result !== "string") {
      return result;
    }
    return safeJsonParse(result) ?? result;
  }

  async function loadFixtureText(item) {
    const fixturePath = join(fixtureRoot, item.source_path);
    return readFile(fixturePath, "utf8");
  }

  async function analyzeLocalGraph(payload = {}) {
    const startedAt = performance.now();
    const timing = {};
    const item = payload.item || {};
    const selection = payload.selection || {};
    const baseUrl = payload.base_url || item.base_url || "http://fixture.local/";
    if (!item.source_path) {
      return {
        ok: false,
        artifact_ready: false,
        diagnostics: [{
          code: "METADATA_CHECKER_OFFSCREEN_LOCAL_GRAPH_SELECTION_INVALID",
          message: "offscreen local graph selection is missing source_path",
          severity: "error",
        }],
        timing: { total_ms: Math.round(performance.now() - startedAt) },
      };
    }

    timing.wasm_init_ms = await ensureRuntime();
    const projectRef = item.project_name || selection.project_name || null;
    const cacheKey = documentCacheKey(baseUrl, item);

    const fixtureLoadStartedAt = performance.now();
    let rawText = metadataTextCache.get(cacheKey);
    const metadataCacheHit = typeof rawText === "string";
    if (!metadataCacheHit) {
      rawText = await loadFixtureText(item);
      rememberCacheEntry(metadataTextCache, cacheKey, rawText);
    }
    timing.fixture_load_ms = Math.round(performance.now() - fixtureLoadStartedAt);

    const runtimeLoadStartedAt = performance.now();
    if (initializedProjectRef !== projectRef) {
      await callWasm("initRuntime", [JSON.stringify({ project_ref: projectRef })]);
      initializedProjectRef = projectRef;
      loadedDocumentKeys.clear();
    }
    timing.runtime_load_ms = Math.round(performance.now() - runtimeLoadStartedAt);

    let documentLoaded = loadedDocumentKeys.has(cacheKey);
    if (!documentLoaded) {
      const loadDocumentStartedAt = performance.now();
      await callWasm("loadSuperpageDocument", [item.source_path, rawText]);
      timing.runtime_load_ms += Math.round(performance.now() - loadDocumentStartedAt);
      loadedDocumentKeys.add(cacheKey);
      documentLoaded = true;
    }

    const buildGraphStartedAt = performance.now();
    await callWasm("buildOrUpdateSuperpageGraph", [item.source_path]);
    timing.build_graph_ms = Math.round(performance.now() - buildGraphStartedAt);
    timing.runtime_load_build_graph_ms = timing.runtime_load_ms + timing.build_graph_ms;

    const analyzeStartedAt = performance.now();
    const graph = await callWasm("analyzeLocalGraph", [
      JSON.stringify({
        source_path: item.source_path,
        file_id: item.file_id || "",
        project_name: item.project_name || selection.project_name || "",
        active_component_id: selection.active_component_id ?? item.active_component_id ?? null,
        selected_component_ids: Array.isArray(selection.selected_component_ids)
          ? selection.selected_component_ids
          : [],
      }),
      JSON.stringify(payload.options || { depth: 2, visible_hop: 1 }),
    ]);
    timing.analyze_selection_ms = Math.round(performance.now() - analyzeStartedAt);

    const serializeStartedAt = performance.now();
    const artifact = {
      kind: "metadata-analysis-artifact",
      source_path: item.source_path,
      project_name: item.project_name || selection.project_name || null,
      file_id: item.file_id || null,
      revision: item.revision ?? null,
      analysis_status: graph?.status || "ready",
      result: graph || { status: "ready" },
    };
    const serialized = JSON.stringify(artifact);
    timing.serialize_output_ms = Math.round(performance.now() - serializeStartedAt);
    timing.total_ms = Math.round(performance.now() - startedAt);

    return {
      ok: true,
      artifact_ready: graph?.status === "ready" || graph?.status === "empty",
      cache: {
        metadata_cache_hit: metadataCacheHit,
        document_cache_hit: documentLoaded && metadataCacheHit,
        cache_key: cacheKey,
      },
      timing,
      artifact,
      serialized_bytes: Buffer.byteLength(serialized, "utf8"),
    };
  }

  function resetCaches() {
    metadataTextCache.clear();
    loadedDocumentKeys.clear();
    initializedProjectRef = null;
  }

  return {
    analyzeLocalGraph,
    resetCaches,
  };
}

export async function createColdOffscreenLocalGraphHost(options) {
  return createOffscreenLocalGraphHost({
    ...options,
    moduleNonce: String(options.moduleNonce ?? Date.now()),
    reloadWasm: true,
  });
}

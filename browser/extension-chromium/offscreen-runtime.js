/* M45 offscreen WASM runtime host */

import initWasmRuntime, * as wasmModule from "./metadata_checker.js";

const MESSAGE_TYPE = "metadata-checker-offscreen-wasm-call";
const LOCAL_GRAPH_MESSAGE_TYPE = "metadata-checker-offscreen-local-graph";
const MAX_CACHE_ENTRIES = 8;

let runtimePromise = null;
let initializedProjectRef = null;
const metadataTextCache = new Map();
const loadedDocumentKeys = new Set();

function stableDiagnostic(code, message, severity = "error") {
  return { code, message, severity };
}

function safeJsonParse(value) {
  try {
    return JSON.parse(value);
  } catch (_error) {
    return null;
  }
}

async function ensureRuntime(wasmUrl) {
  if (!runtimePromise) {
    runtimePromise = initWasmRuntime(wasmUrl);
  }
  await runtimePromise;
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

function encodeMetaPath(path, preserveSlash) {
  const raw = String(path || "");
  if (!preserveSlash) {
    return encodeURIComponent(raw);
  }
  return raw.split("/").map((part) => encodeURIComponent(part)).join("/");
}

function buildUrl(baseUrl, path) {
  return new URL(path, baseUrl).toString();
}

function metadataContentPath(item = {}) {
  const ref = item.file_id || `${item.project_name}/${item.source_path}`;
  return `/api/meta/services/getFileContent/${encodeMetaPath(ref, true)}`;
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

async function fetchMetadataText(baseUrl, item) {
  const response = await fetch(buildUrl(baseUrl, metadataContentPath(item)), {
    method: "GET",
    credentials: "include",
    redirect: "follow",
    headers: {
      Accept: "application/json,text/plain,*/*",
    },
  });
  if (!response.ok) {
    const code = response.status === 401
      ? "REMOTE_METADATA_UNAUTHORIZED"
      : response.status === 403
        ? "REMOTE_METADATA_FORBIDDEN"
        : response.status === 404
          ? "REMOTE_METADATA_NOT_FOUND"
          : "REMOTE_METADATA_FETCH_FAILED";
    throw new Error(`${code}: HTTP ${response.status}`);
  }
  return response.text();
}

async function callWasm(method, args = []) {
  const fn = wasmModule[method];
  if (typeof fn !== "function") {
    throw new Error(`WASM runtime method missing: ${method}`);
  }
  const result = await fn(...args);
  if (typeof result !== "string") {
    return result;
  }
  return safeJsonParse(result) ?? result;
}

function projectRefFromInitRuntimeArgs(args = []) {
  const options = safeJsonParse(args[0]);
  if (!options || typeof options !== "object") {
    return null;
  }
  return typeof options.project_ref === "string" ? options.project_ref : null;
}

async function analyzeLocalGraphInOffscreen(payload = {}) {
  const startedAt = performance.now();
  const timings = {};
  const item = payload.item || {};
  const selection = payload.selection || {};
  const baseUrl = payload.base_url || item.base_url;
  if (!baseUrl || !item.source_path) {
    return {
      ok: false,
      diagnostics: [
        stableDiagnostic(
          "METADATA_CHECKER_OFFSCREEN_LOCAL_GRAPH_SELECTION_INVALID",
          "offscreen local graph selection is missing base_url or source_path",
        ),
      ],
    };
  }

  await ensureRuntime(payload.wasm_url || chrome.runtime.getURL("metadata_checker_bg.wasm"));
  timings.ensure_runtime_ms = Math.round(performance.now() - startedAt);
  const projectRef = item.project_name || selection.project_name || null;
  const cacheKey = documentCacheKey(baseUrl, item);
  const fetchStartedAt = performance.now();
  let rawText = metadataTextCache.get(cacheKey);
  const metadataCacheHit = typeof rawText === "string";
  if (!metadataCacheHit) {
    rawText = await fetchMetadataText(baseUrl, item);
    rememberCacheEntry(metadataTextCache, cacheKey, rawText);
  }
  timings.fetch_metadata_ms = Math.round(performance.now() - fetchStartedAt);
  const initStartedAt = performance.now();
  if (initializedProjectRef !== projectRef) {
    await callWasm("initRuntime", [JSON.stringify({ project_ref: projectRef })]);
    initializedProjectRef = projectRef;
    loadedDocumentKeys.clear();
  }
  timings.init_runtime_ms = Math.round(performance.now() - initStartedAt);
  if (!loadedDocumentKeys.has(cacheKey)) {
    const loadStartedAt = performance.now();
    await callWasm("loadSuperpageDocument", [item.source_path, rawText]);
    timings.load_document_ms = Math.round(performance.now() - loadStartedAt);
    loadedDocumentKeys.add(cacheKey);
  } else {
    timings.load_document_ms = 0;
  }
  const buildStartedAt = performance.now();
  await callWasm("buildOrUpdateSuperpageGraph", [item.source_path]);
  timings.build_graph_ms = Math.round(performance.now() - buildStartedAt);
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
  timings.analyze_local_graph_ms = Math.round(performance.now() - analyzeStartedAt);
  timings.total_ms = Math.round(performance.now() - startedAt);
  return {
    ok: true,
    artifact_ready: graph?.status === "ready" || graph?.status === "empty",
    cache: {
      metadata_cache_hit: metadataCacheHit,
      document_cache_hit: loadedDocumentKeys.has(cacheKey) && metadataCacheHit,
      cache_key: cacheKey,
    },
    timings,
    artifact: {
      kind: "metadata-analysis-artifact",
      source_path: item.source_path,
      project_name: item.project_name || selection.project_name || null,
      file_id: item.file_id || null,
      revision: item.revision ?? null,
      analysis_status: graph?.status || "ready",
      result: graph || { status: "ready" },
    },
  };
}

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (!message || (message.type !== MESSAGE_TYPE && message.type !== LOCAL_GRAPH_MESSAGE_TYPE)) {
    return false;
  }

  (async () => {
    if (message.type === LOCAL_GRAPH_MESSAGE_TYPE) {
      sendResponse(await analyzeLocalGraphInOffscreen(message.payload || {}));
      return;
    }

    const payload = message.payload || {};
    const method = typeof payload.method === "string" ? payload.method : "";
    const args = Array.isArray(payload.args) ? payload.args : [];
    await ensureRuntime(payload.wasm_url || chrome.runtime.getURL("metadata_checker_bg.wasm"));
    const fn = wasmModule[method];
    if (typeof fn !== "function") {
      sendResponse({
        ok: false,
        request_id: message.request_id || null,
        diagnostic: stableDiagnostic(
          "OFFSCREEN_WASM_METHOD_MISSING",
          `WASM runtime method missing: ${method}`,
        ),
      });
      return;
    }
    const result = await fn(...args);
    if (method === "initRuntime") {
      initializedProjectRef = projectRefFromInitRuntimeArgs(args);
      loadedDocumentKeys.clear();
    }
    sendResponse({
      ok: true,
      request_id: message.request_id || null,
      result,
    });
  })().catch((error) => {
    sendResponse({
      ok: false,
      request_id: message.request_id || null,
      diagnostic: stableDiagnostic(
        "OFFSCREEN_WASM_CALL_FAILED",
        error?.message || "offscreen WASM call failed",
      ),
    });
  });

  return true;
});

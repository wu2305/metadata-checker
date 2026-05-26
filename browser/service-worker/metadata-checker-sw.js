/**
 * M40.10：Service Worker Runtime Contract
 *
 * 提供最小 message protocol，并在 initRuntime 中执行 WASM lazy init。
 * 支持 lazy init、重复请求复用初始化 Promise、失败后重试或 fallback。
 * message response 携带 request_id，错误返回稳定 code。
 */

const SW_VERSION = "0.1.0-m40.10";
const DEFAULT_WASM_FILE = "metadata_checker_bg.wasm";
const DEFAULT_WASM_BINDGEN_FILE = "metadata_checker.js";
const DEFAULT_WASM_ASSET_VERSION = "m42-real-bi";

// runtime 状态
let _initPromise = null;
let _initFailed = false;
let _initError = null;
let _wasmExports = {};
let _wasmBindgenLoader = null;
let _wasmBindgenPreloadError = null;
let _wasmStatus = {
  state: "mock",
  mode: "mock",
  wasmUrl: null,
  fallbackUsed: false,
  diagnostic: null,
};
let _documentCache = new Map(); // sourcePath -> { rawText, loadedAt }
let _graphCache = new Map(); // sourcePath -> { builtAt }
const FORBIDDEN_SELECTION_KEYS = new Set([
  "raw_text",
  "rawText",
  "components",
  "component_json",
  "componentJson",
  "raw_component",
  "rawComponent",
  "canvas",
]);

function makeResponse(requestId, result) {
  return { id: requestId, ok: true, result, error: null };
}

function makeErrorResponse(requestId, code, message, diagnostic = null) {
  return {
    id: requestId,
    ok: false,
    result: null,
    error: { code, message, diagnostic },
  };
}

function _makeDiagnostic(code, message, detail = {}) {
  return {
    code,
    severity: "error",
    message,
    detail,
  };
}

function _makeWasmError(code, message, detail = {}) {
  const err = new Error(message);
  err.code = code;
  err.diagnostic = _makeDiagnostic(code, message, detail);
  return err;
}

function _findExportName(name) {
  const camelName = name?.replace(/_([a-z])/g, (_, ch) => ch.toUpperCase());
  const candidates = [name, camelName];
  const fallback = name?.replace(/[A-Z]/g, "_$&").toLowerCase();
  if (fallback !== name) {
    candidates.push(fallback);
  }
  for (const candidate of candidates) {
    if (typeof _wasmExports[candidate] === "function") {
      return candidate;
    }
  }
  return null;
}

function _hasWasmExport(name) {
  return _findExportName(name) !== null;
}

async function _callWasmExport(exportName, args = []) {
  const resolvedName = _findExportName(exportName);
  if (!resolvedName) {
    throw _makeWasmError("WASM_EXPORT_MISSING", "WASM export is missing", {
      export_name: exportName,
    });
  }
  try {
    const fn = _wasmExports[resolvedName];
    const result = fn(...args);
    if (result && typeof result?.then === "function") {
      return await result;
    }
    return result;
  } catch (err) {
    if (err && typeof err === "object" && typeof err.code === "string") {
      throw err;
    }
    throw _makeWasmError(
      "WASM_EXPORT_ERROR",
      err?.message ?? "WASM export execution failed",
      {
        export_name: resolvedName,
        cause_message: err?.message ?? String(err),
      },
    );
  }
}

async function _runtimeLoadSuperpageDocument(sourcePath, rawText) {
  if (_hasWasmExport("loadSuperpageDocument")) {
    const wasmResult = await _callWasmExport("loadSuperpageDocument", [sourcePath, rawText]);
    const result = _parseWasmResult(wasmResult);
    _documentCache.set(sourcePath, {
      rawText: rawText ?? "",
      loadedAt: Date.now(),
    });
    return result;
  }
  throw _makeWasmError("WASM_EXPORT_MISSING", "WASM export is missing", {
    export_name: "loadSuperpageDocument",
  });
}

async function _runtimeBuildOrUpdateSuperpageGraph(sourcePath) {
  if (_hasWasmExport("buildOrUpdateSuperpageGraph")) {
    const wasmResult = await _callWasmExport("buildOrUpdateSuperpageGraph", [sourcePath]);
    const result = _parseWasmResult(wasmResult);
    _graphCache.set(sourcePath, { builtAt: Date.now() });
    return result;
  }
  throw _makeWasmError("WASM_EXPORT_MISSING", "WASM export is missing", {
    export_name: "buildOrUpdateSuperpageGraph",
  });
}

async function _runtimeAnalyzeSuperpageSelection(selection, options = {}) {
  const forbiddenSelectionPath = _findForbiddenSelectionPayload(selection);
  if (forbiddenSelectionPath) {
    throw _makeWasmError(
      "INVALID_SELECTION_PAYLOAD",
      `${forbiddenSelectionPath} is not allowed in selection payload`,
    );
  }
  if (_hasWasmExport("analyzeSuperpageSelection")) {
    const wasmResult = await _callWasmExport("analyzeSuperpageSelection", [
      JSON.stringify(selection ?? {}),
      JSON.stringify(options ?? {}),
    ]);
    return _parseWasmResult(wasmResult);
  }
  throw _makeWasmError("WASM_EXPORT_MISSING", "WASM export is missing", {
    export_name: "analyzeSuperpageSelection",
  });
}

async function _runtimeStatus() {
  if (_hasWasmExport("runtimeStatus")) {
    const wasmResult = await _callWasmExport("runtimeStatus", []);
    const result = _parseWasmResult(wasmResult);
    if (result && typeof result === "object") {
      return {
        ...result,
        items: [
          ...(Array.isArray(result.items) ? result.items : []),
          _mockRuntimeStatus().items[0],
        ],
      };
    }
  }
  return _mockRuntimeStatus();
}

function _extractSourcePath(ref) {
  if (typeof ref === "string") {
    return ref;
  }
  if (ref && typeof ref === "object") {
    return ref.source_path ?? ref.sourcePath ?? ref.filePath ?? null;
  }
  return null;
}

function _extractRawText(result) {
  if (typeof result === "string") {
    try {
      const parsed = JSON.parse(result);
      return _extractRawText(parsed);
    } catch {
      return result;
    }
  }
  if (!result || typeof result !== "object") {
    return "";
  }
  return (
    result.raw_text ??
    result.rawText ??
    result.content ??
    result.detail?.raw_text ??
    result.detail?.rawText ??
    result.detail?.content ??
    ""
  );
}

function _parseWasmResult(result) {
  if (typeof result !== "string") {
    return result;
  }
  try {
    const parsed = JSON.parse(result);
    if (
      parsed &&
      typeof parsed === "object" &&
      ("status" in parsed || "items" in parsed || "diagnostics" in parsed)
    ) {
      return parsed;
    }
    return result;
  } catch {
    return result;
  }
}

function _baseUrlFromOptions(options) {
  return (
    options?.base_url ??
    options?.baseUrl ??
    options?.remote_base_url ??
    options?.remoteBaseUrl ??
    self.location?.origin ??
    ""
  );
}

function _remoteExportArgs(exportName, fileRef, options = {}) {
  const resolvedName = _findExportName(exportName);
  const fn = resolvedName ? _wasmExports[resolvedName] : null;
  const expectedBindingArity =
    exportName === "loadRemoteSuperpageDocument" ? 3 : 2;
  const shouldUseRustBindingShape =
    typeof fn === "function" &&
    (fn.length >= expectedBindingArity ||
      options?.base_url ||
      options?.baseUrl ||
      options?.remote_base_url ||
      options?.remoteBaseUrl);
  if (!shouldUseRustBindingShape) {
    return [fileRef, options];
  }
  const fileRefJson = typeof fileRef === "string" ? fileRef : JSON.stringify(fileRef ?? {});
  if (exportName === "loadRemoteSuperpageDocument") {
    return [_baseUrlFromOptions(options), fileRefJson, JSON.stringify(options ?? {})];
  }
  return [_baseUrlFromOptions(options), fileRefJson];
}

function _defaultWasmUrl() {
  const baseHref = self.location?.href;
  if (baseHref) {
    const url = new URL(DEFAULT_WASM_FILE, baseHref);
    url.searchParams.set("v", DEFAULT_WASM_ASSET_VERSION);
    return url.toString();
  }
  return `${DEFAULT_WASM_FILE}?v=${DEFAULT_WASM_ASSET_VERSION}`;
}

function _defaultWasmBindgenUrl() {
  const baseHref = self.location?.href;
  if (baseHref) {
    const url = new URL(DEFAULT_WASM_BINDGEN_FILE, baseHref);
    url.searchParams.set("v", DEFAULT_WASM_ASSET_VERSION);
    return url.toString();
  }
  return `${DEFAULT_WASM_BINDGEN_FILE}?v=${DEFAULT_WASM_ASSET_VERSION}`;
}

function _findForbiddenSelectionPayload(value, path = "selection", seen = new WeakSet()) {
  if (!value || typeof value !== "object") {
    return null;
  }
  if (seen.has(value)) {
    return null;
  }
  seen.add(value);

  if (Array.isArray(value)) {
    for (let i = 0; i < value.length; i += 1) {
      const nested = _findForbiddenSelectionPayload(value[i], `${path}[${i}]`, seen);
      if (nested) return nested;
    }
    return null;
  }

  for (const [key, item] of Object.entries(value)) {
    const childPath = `${path}.${key}`;
    if (FORBIDDEN_SELECTION_KEYS.has(key)) {
      return childPath;
    }
    const nested = _findForbiddenSelectionPayload(item, childPath, seen);
    if (nested) return nested;
  }
  return null;
}

async function _loadWasmRuntime(options = {}) {
  if (options?.mock === true) {
    const result = _mockInitRuntime(options);
    _wasmExports = result.exports ?? {};
    return result;
  }

  const wasmUrl = options?.wasmUrl ?? _defaultWasmUrl();
  const glueUrl = options?.wasmBindgenUrl ?? _defaultWasmBindgenUrl();
  const loader = await _loadWasmBindgenLoader(glueUrl, options);
  let initDiagnostics = [];
  try {
    await loader(wasmUrl);
  } catch (err) {
    throw _makeWasmError("WASM_BINDGEN_INIT_FAILED", "wasm-bindgen runtime init failed", {
      wasm_url: wasmUrl,
      wasm_bindgen_url: glueUrl,
      cause_message: err?.message ?? String(err),
    });
  }

  _wasmBindgenLoader = loader;
  _wasmExports = loader;
  const exportNames = Object.keys(loader ?? {});
  _wasmStatus = {
    state: "loaded",
    mode: "wasm-bindgen",
    wasmUrl,
    fallbackUsed: false,
    diagnostic: null,
  };

  const runtimeOptions = options?.runtimeOptions ?? {};
  const initResult = _parseWasmResult(
    await _callWasmExport("initRuntime", [JSON.stringify(runtimeOptions)]),
  );
  if (Array.isArray(initResult?.diagnostics)) {
    initDiagnostics = initResult.diagnostics;
  }

  return {
    ...(initResult && typeof initResult === "object" ? initResult : {}),
    status: initResult?.status ?? "ready",
    target: initResult?.target ?? null,
    items: [
      {
        kind: "runtime_ready",
        label: "SW WASM Runtime Initialized",
        detail: {
          version: SW_VERSION,
          wasm: {
            state: _wasmStatus.state,
            mode: _wasmStatus.mode,
            wasm_url: wasmUrl,
            wasm_bindgen_url: glueUrl,
            fallback_used: false,
            export_count: exportNames.length,
          },
        },
      },
      ...((Array.isArray(initResult?.items) ? initResult.items : [])),
    ],
    diagnostics: initDiagnostics,
  };
}

async function _loadWasmBindgenLoader(glueUrl, options = {}) {
  if (typeof options?.wasmBindgenFactory === "function") {
    return options.wasmBindgenFactory;
  }
  const existingLoader = _resolveWasmBindgenLoader();
  if (existingLoader) {
    return existingLoader;
  }
  if (_wasmBindgenPreloadError) {
    throw _wasmBindgenPreloadError;
  }
  if (typeof self.importScripts !== "function") {
    throw _makeWasmError(
      "WASM_BINDGEN_GLUE_UNAVAILABLE",
      "importScripts is required to load wasm-bindgen glue in this Service Worker",
      { wasm_bindgen_url: glueUrl },
    );
  }
  try {
    self.importScripts(glueUrl);
  } catch (err) {
    throw _makeWasmError("WASM_BINDGEN_GLUE_LOAD_FAILED", "Failed to load wasm-bindgen glue", {
      wasm_bindgen_url: glueUrl,
      cause_message: err?.message ?? String(err),
    });
  }
  const importedLoader = _resolveWasmBindgenLoader();
  if (!importedLoader) {
    throw _makeWasmError(
      "WASM_BINDGEN_GLUE_INVALID",
      "wasm-bindgen glue did not expose wasm_bindgen",
      { wasm_bindgen_url: glueUrl },
    );
  }
  return importedLoader;
}

function _resolveWasmBindgenLoader() {
  if (typeof self.wasm_bindgen === "function") {
    return self.wasm_bindgen;
  }
  try {
    if (typeof wasm_bindgen === "function") {
      self.wasm_bindgen = wasm_bindgen;
      return wasm_bindgen;
    }
  } catch {
    return null;
  }
  return null;
}

function _preloadWasmBindgenGlue() {
  const existingLoader = _resolveWasmBindgenLoader();
  if (existingLoader || _wasmBindgenPreloadError) {
    return;
  }
  const glueUrl = _defaultWasmBindgenUrl();
  if (typeof self.importScripts !== "function") {
    _wasmBindgenPreloadError = _makeWasmError(
      "WASM_BINDGEN_GLUE_UNAVAILABLE",
      "importScripts is required to load wasm-bindgen glue in this Service Worker",
      { wasm_bindgen_url: glueUrl },
    );
    return;
  }
  try {
    self.importScripts(glueUrl);
  } catch (err) {
    _wasmBindgenPreloadError = _makeWasmError(
      "WASM_BINDGEN_GLUE_LOAD_FAILED",
      "Failed to load wasm-bindgen glue",
      {
        wasm_bindgen_url: glueUrl,
        cause_message: err?.message ?? String(err),
      },
    );
    return;
  }
  if (!_resolveWasmBindgenLoader()) {
    _wasmBindgenPreloadError = _makeWasmError(
      "WASM_BINDGEN_GLUE_INVALID",
      "wasm-bindgen glue did not expose wasm_bindgen",
      { wasm_bindgen_url: glueUrl },
    );
  }
}

_preloadWasmBindgenGlue();

function _mockInitRuntime(options = {}) {
  if (options?.shouldFail) {
    throw new Error("mock initRuntime failed");
  }
  const exports = options?.mockExports ?? {
    analyze: () => {},
    fetch_remote_file_info: () => ({}),
    fetch_remote_file_content: () => "",
    load_remote_superpage_document: () => ({}),
  };
  _wasmExports = exports;
  _wasmStatus = {
    state: "mock",
    mode: "mock",
    wasmUrl: options?.wasmUrl ?? null,
    fallbackUsed: false,
    diagnostic: null,
  };
  return {
    exports,
    status: "ready",
    target: null,
    items: [
      {
        kind: "runtime_ready",
        label: "SW Runtime Initialized",
        detail: { version: SW_VERSION, options },
      },
    ],
    diagnostics: [],
  };
}

async function _mockFetchRemoteFileInfo(fileRef, options) {
  const result = await _callWasmExport(
    "fetchRemoteFileInfo",
    _remoteExportArgs("fetchRemoteFileInfo", fileRef, options),
  );
  return _parseWasmResult(result);
}

async function _mockFetchRemoteFileContent(fileRef, options) {
  const result = await _callWasmExport(
    "fetchRemoteFileContent",
    _remoteExportArgs("fetchRemoteFileContent", fileRef, options),
  );
  return _parseWasmResult(result);
}

async function _mockLoadRemoteSuperpageDocument(fileRef, options) {
  const wasmResult = await _callWasmExport(
    "loadRemoteSuperpageDocument",
    _remoteExportArgs("loadRemoteSuperpageDocument", fileRef, options),
  );
  const result = _parseWasmResult(wasmResult);
  const sourcePath = _extractSourcePath(fileRef) ?? _extractSourcePath(result);
  const remoteText = _extractRawText(result);
  if (!remoteText && result && typeof result === "object" && result.status) {
    return result;
  }
  if (!sourcePath || typeof sourcePath !== "string") {
    throw new Error("source_path is required");
  }
  const loaded = await _runtimeLoadSuperpageDocument(sourcePath, remoteText);
  return {
    ...loaded,
    items: [
      ...loaded.items,
      {
        kind: "remote_document_loaded",
        label: "Remote Document Loaded (SW)",
        detail: {
          source_path: sourcePath,
          wasm_detail: result && typeof result === "object" ? result : undefined,
        },
      },
    ],
  };
}

function _mockRuntimeStatus() {
  return {
    status: "ready",
    target: null,
    items: [
      {
        kind: "runtime_status",
        label: "SW Runtime Status",
        detail: {
          initialized: _initPromise !== null && !_initFailed,
          sw_version: SW_VERSION,
          document_count: _documentCache.size,
          graph_count: _graphCache.size,
          wasm: {
            state: _wasmStatus.state,
            mode: _wasmStatus.mode,
            wasm_url: _wasmStatus.wasmUrl,
            fallback_used: _wasmStatus.fallbackUsed,
            diagnostic: _wasmStatus.diagnostic,
            last_error_code: _initError?.code ?? null,
          },
        },
      },
    ],
    diagnostics: [],
  };
}

function _mockLoadSuperpageDocument(sourcePath, rawText) {
  if (typeof sourcePath !== "string") {
    throw new Error("sourcePath is required");
  }
  _documentCache.set(sourcePath, {
    rawText: rawText ?? "",
    loadedAt: Date.now(),
  });
  return {
    status: "ready",
    target: sourcePath,
    items: [
      {
        kind: "document_loaded",
        label: "Document Loaded (SW)",
        detail: { source_path: sourcePath, component_count: 1 },
      },
    ],
    diagnostics: [],
  };
}

function _mockBuildOrUpdateSuperpageGraph(sourcePath) {
  if (typeof sourcePath !== "string") {
    throw new Error("sourcePath is required");
  }
  if (!_documentCache.has(sourcePath)) {
    throw new Error("document not loaded, call loadSuperpageDocument first");
  }
  _graphCache.set(sourcePath, { builtAt: Date.now() });
  return {
    status: "ready",
    target: sourcePath,
    items: [
      {
        kind: "graph_built",
        label: "Graph Built (SW)",
        detail: { source_path: sourcePath, node_count: 2, edge_count: 1 },
      },
    ],
    diagnostics: [],
  };
}

function _mockAnalyzeSuperpageSelection(selection, options = {}) {
  if (!selection || typeof selection !== "object") {
    throw new Error("selection is required");
  }
  const sourcePath = selection.source_path;
  if (!_graphCache.has(sourcePath)) {
    throw new Error("graph not built, call buildOrUpdateSuperpageGraph first");
  }

  const forbiddenSelectionPath = _findForbiddenSelectionPayload(selection);
  if (forbiddenSelectionPath) {
    throw _makeWasmError(
      "INVALID_SELECTION_PAYLOAD",
      `${forbiddenSelectionPath} is not allowed in selection payload`,
    );
  }

  return {
    status: "ready",
    target: selection.active_component_id ?? selection.source_path,
    items: [
      {
        kind: "analysis",
        label: "Analysis Result (SW)",
        detail: {
          source_path: selection.source_path,
          active_component_id: selection.active_component_id,
          sw_version: SW_VERSION,
          options,
        },
      },
    ],
    diagnostics: [],
  };
}

function _ensureInit(options = {}) {
  if (_initPromise) {
    return _initPromise;
  }
  if (_initFailed) {
    // 允许重试
    _initFailed = false;
    _initError = null;
  }
  _initPromise = (async () => {
    try {
      const result = await _loadWasmRuntime(options);
      _initFailed = false;
      _initError = null;
      return result;
    } catch (err) {
      _initFailed = true;
      _initError = err;
      _wasmStatus = {
        state: "failed",
        mode: "failed",
        wasmUrl: options?.wasmUrl ?? _defaultWasmUrl(),
        fallbackUsed: false,
        diagnostic:
          err?.diagnostic ??
          _makeDiagnostic("WASM_RUNTIME_INIT_FAILED", err?.message ?? String(err)),
      };
      _initPromise = null;
      throw err;
    }
  })();
  return _initPromise;
}

async function handleRequest(request) {
  const { id, method, args = [] } = request;

  if (typeof id === "undefined") {
    // 无法回复没有 id 的请求
    return null;
  }

  try {
    if (method === "initRuntime") {
      const result = await _ensureInit(args[0]);
      return makeResponse(id, result);
    }

    if (method === "runtimeStatus") {
      const result = await _runtimeStatus();
      return makeResponse(id, result);
    }

    if (method === "loadSuperpageDocument") {
      if (_initPromise) {
        await _ensureInit();
      }
      const result = await _runtimeLoadSuperpageDocument(args[0], args[1]);
      return makeResponse(id, result);
    }

    if (method === "fetchRemoteFileInfo") {
      await _ensureInit();
      const result = await _mockFetchRemoteFileInfo(args[0], args[1]);
      return makeResponse(id, result);
    }

    if (method === "fetchRemoteFileContent") {
      await _ensureInit();
      const result = await _mockFetchRemoteFileContent(args[0], args[1]);
      return makeResponse(id, result);
    }

    if (method === "loadRemoteSuperpageDocument") {
      await _ensureInit();
      const result = await _mockLoadRemoteSuperpageDocument(args[0], args[1]);
      return makeResponse(id, result);
    }

    if (method === "buildOrUpdateSuperpageGraph") {
      if (_initPromise) {
        await _ensureInit();
      }
      const result = await _runtimeBuildOrUpdateSuperpageGraph(args[0]);
      return makeResponse(id, result);
    }

    if (method === "analyzeSuperpageSelection") {
      if (_initPromise) {
        await _ensureInit();
      }
      const result = await _runtimeAnalyzeSuperpageSelection(args[0], args[1]);
      return makeResponse(id, result);
    }

    return makeErrorResponse(id, "UNKNOWN_METHOD", `Unknown method: ${method}`);
  } catch (err) {
    return makeErrorResponse(
      id,
      err?.code ?? "RUNTIME_EXECUTION_ERROR",
      err?.message ?? String(err),
      err?.diagnostic ?? null,
    );
  }
}

self.addEventListener("install", (event) => {
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});

self.addEventListener("message", (event) => {
  const request = event.data;
  if (!request || typeof request !== "object") {
    return;
  }
  // 如果不是标准 request，忽略
  if (typeof request.method !== "string") {
    return;
  }
  event.waitUntil(
    handleRequest(request).then((response) => {
      if (response) {
        event.source?.postMessage(response);
      }
    }),
  );
});

// 导出给测试环境（Node.js 不会执行 addEventListener，但会 import 脚本）
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    SW_VERSION,
    handleRequest,
    _ensureInit,
    _loadWasmRuntime,
    _mockInitRuntime,
    _mockRuntimeStatus,
    _mockLoadSuperpageDocument,
    _mockBuildOrUpdateSuperpageGraph,
    _mockAnalyzeSuperpageSelection,
  };
}

/* M45 Chromium extension service worker */

export const M45_EVENT_TYPES = Object.freeze({
  SESSION_BOOTSTRAPPED: "session_bootstrapped",
  VISIBLE_METADATA_INDEXED: "visible_metadata_indexed",
  METADATA_PREFETCHED: "metadata_prefetched",
  BACKGROUND_ANALYSIS_PROGRESS: "background_analysis_progress",
  BACKGROUND_ANALYSIS_COMPLETED: "background_analysis_completed",
});

const SUPPORTED_METADATA_EXTENSIONS = new Set(["spg", "tbl"]);
const SENSITIVE_KEY_PATTERN = /(token|cookie|password|secret|auth|credential|cipherpassport)/i;
const SENSITIVE_PAIR_PATTERN =
  /["']?(token|cookie|password|secret|auth|credential|cipherpassport)["']?\s*[:=]\s*["']?[^&\s,;}]+/gi;
const OFFSCREEN_WASM_CALL_MESSAGE = "metadata-checker-offscreen-wasm-call";
const OFFSCREEN_LOCAL_GRAPH_MESSAGE = "metadata-checker-offscreen-local-graph";

function isObject(value) {
  return value !== null && typeof value === "object";
}

function asArray(value) {
  return Array.isArray(value) ? value : [];
}

function asString(value) {
  return typeof value === "string" ? value : "";
}

function asNumberOrNull(value) {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  if (typeof value === "string" && value.length > 0) {
    const parsed = Number.parseFloat(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

function asNullableString(value) {
  if (typeof value === "string") {
    return value;
  }
  if (value === null || value === undefined) {
    return null;
  }
  return String(value);
}

function redactText(text) {
  if (typeof text !== "string") {
    return text;
  }
  return text.replace(SENSITIVE_PAIR_PATTERN, "$1=***");
}

function redactForTelemetry(value, seen = new WeakSet()) {
  if (!isObject(value)) {
    return typeof value === "string" ? redactText(value) : value;
  }
  if (seen.has(value)) {
    return "[Circular]";
  }
  seen.add(value);
  if (Array.isArray(value)) {
    return value.map((item) => redactForTelemetry(item, seen));
  }
  const result = {};
  for (const [key, item] of Object.entries(value)) {
    if (SENSITIVE_KEY_PATTERN.test(key)) {
      result[key] = "***";
    } else {
      result[key] = redactForTelemetry(item, seen);
    }
  }
  return result;
}

function normalizeDiagnostic(value) {
  if (!isObject(value)) {
    return null;
  }
  if (typeof value.code !== "string" || typeof value.message !== "string") {
    return null;
  }
  return {
    severity: value.severity || "warning",
    code: value.code,
    message: redactText(value.message),
  };
}

function stableDiagnostic(code, message, severity = "warning") {
  return {
    severity,
    code,
    message: redactText(message),
  };
}

export function createBackgroundMessageFailureResponse(error) {
  return {
    ok: false,
    diagnostics: [
      stableDiagnostic(
        "METADATA_CHECKER_BACKGROUND_MESSAGE_FAILED",
        error?.message || "extension background message failed",
        "error",
      ),
    ],
  };
}

function encodeMetaPath(path, preserveSlash = true) {
  if (!preserveSlash) {
    return encodeURIComponent(path);
  }
  return String(path)
    .split("/")
    .map((segment) => encodeURIComponent(segment))
    .join("/");
}

function buildUrl(baseUrl, path) {
  const base = String(baseUrl || "").replace(/\/+$/, "");
  const suffix = String(path || "").replace(/^\/+/, "");
  return `${base}/${suffix}`;
}

function normalizeVisibleManifestEntry(projectName, file = {}) {
  const rawPath = asString(file.path || file.resourcePath || resourcePath(file) || file.source_path || file.sourcePath);
  const sourcePath = rawPath ? normalizeSourcePath(rawPath, projectName) : "";
  return {
    id: asNullableString(file.id ?? file.fileId ?? file.file_id) || null,
    path: rawPath || null,
    type: asString(file.type ?? file.file_type ?? file.fileType),
    source_path: sourcePath,
    isFolder: isFolder(file),
    revision: asNullableString(file.revision ?? file.modify_version ?? file.modifyVersion),
    modifyTime: asNumberOrNull(file.modifyTime ?? file.modify_time ?? file.modificationTime),
    modifier: asNullableString(file.modifier) || asNullableString(file.modifier_name) || asNullableString(file.modifierName),
    modifierName: asNullableString(file.modifierName ?? file.modifier_name) || asNullableString(file.modifier_name),
  };
}

function pickVisibleManifestDiffDetail(value) {
  if (!isObject(value)) {
    return null;
  }
  if (Array.isArray(value.added) && Array.isArray(value.modified) && Array.isArray(value.deleted)) {
    return value;
  }
  const items = asArray(value.items);
  const detail = items.find((item) => isObject(item) && item.kind === "visible_manifest_diff");
  if (detail && isObject(detail.detail)) {
    return detail.detail;
  }
  return null;
}

function normalizeManifestCount(value, fallback = 0) {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return fallback;
  }
  return value;
}

function safeJsonParse(text) {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

async function readJsonOrText(response) {
  const text = await response.text();
  const parsed = safeJsonParse(text);
  return parsed ?? text;
}

function wrappedArray(value, key) {
  if (Array.isArray(value)) {
    return value;
  }
  if (!isObject(value)) {
    return [];
  }
  if (Array.isArray(value[key])) {
    return value[key];
  }
  if (Array.isArray(value.children)) {
    return value.children;
  }
  if (Array.isArray(value.files)) {
    return value.files;
  }
  for (const wrapper of ["data", "result", "file"]) {
    const nested = value[wrapper];
    if (Array.isArray(nested)) {
      return nested;
    }
    if (isObject(nested)) {
      const found = wrappedArray(nested, key);
      if (found.length > 0) {
        return found;
      }
    }
  }
  return [];
}

function inferProjectName(project) {
  return project?.projectName ?? project?.project_name ?? project?.name ?? project?.id ?? null;
}

function contextProjectName(context = {}) {
  return asString(
    context.project_name
    ?? context.projectName
    ?? context.current_project_name
    ?? context.currentProjectName,
  );
}

function resourcePath(file) {
  if (typeof file?.path === "string" && file.path.length > 0) {
    return file.path;
  }
  const name = asString(file?.name);
  const parent = asString(file?.parentDir ?? file?.parent_dir);
  if (!name || !parent) {
    return "";
  }
  return `${parent.replace(/\/+$/, "")}/${name}`;
}

function normalizeSourcePath(path, projectName) {
  const raw = String(path || "").replace(/^\/+/, "");
  const prefix = `${projectName}/`;
  return raw.startsWith(prefix) ? raw.slice(prefix.length) : raw;
}

function fileExtension(file) {
  const path = resourcePath(file) || asString(file?.name);
  const match = path.match(/\.([^.\/]+)$/);
  return match ? match[1].toLowerCase() : "";
}

function isFolder(file) {
  return file?.isFolder === true || file?.is_folder === true;
}

function makeCacheKey(baseUrl, file) {
  const revision = file.revision ?? file.modifyTime ?? file.modify_time ?? "";
  return [
    String(baseUrl || ""),
    file.project_name ?? file.projectName ?? "",
    file.source_path ?? "",
    file.file_id ?? "",
    revision,
  ].join("|");
}

function makeAnalysisArtifactKey(item) {
  if (!item.foreground) {
    return `analysis-artifact|background|${item.cache_key}`;
  }
  const selected = asArray(item.selected_component_ids).join(",");
  const active = item.active_component_id ?? "";
  return `analysis-artifact|foreground|${item.cache_key}|active:${active}|selected:${selected}`;
}

function assertCachePayloadSafe(value, seen = new WeakSet()) {
  if (!isObject(value)) {
    SENSITIVE_PAIR_PATTERN.lastIndex = 0;
    if (typeof value === "string" && SENSITIVE_PAIR_PATTERN.test(value)) {
      throw new Error("cache payload contains sensitive value");
    }
    return;
  }
  if (seen.has(value)) {
    return;
  }
  seen.add(value);
  if (Array.isArray(value)) {
    for (const item of value) {
      assertCachePayloadSafe(item, seen);
    }
    return;
  }
  for (const [key, item] of Object.entries(value)) {
    if (SENSITIVE_KEY_PATTERN.test(key)) {
      throw new Error("cache payload contains sensitive keys");
    }
    assertCachePayloadSafe(item, seen);
  }
}

export function createMemoryMetadataCache() {
  const store = new Map();
  return {
    async get(key) {
      return store.get(key) ?? null;
    },
    async set(key, value) {
      assertCachePayloadSafe(value);
      const sanitized = redactForTelemetry(value);
      store.set(key, sanitized);
      return sanitized;
    },
    async keys() {
      return Array.from(store.keys());
    },
    _dump() {
      return Array.from(store.entries());
    },
  };
}

function requestToPromise(request) {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("IndexedDB request failed"));
  });
}

export function createIndexedDbMetadataCache(options = {}) {
  const indexedDBImpl = options.indexedDB ?? globalThis.indexedDB;
  const databaseName = options.databaseName ?? "metadata-checker-m45-cache";
  const storeName = options.storeName ?? "metadata_cache_entries";
  const version = options.version ?? 1;
  let openPromise = null;

  function openDatabase() {
    if (!indexedDBImpl || typeof indexedDBImpl.open !== "function") {
      throw new Error("IndexedDB is unavailable");
    }
    if (openPromise) {
      return openPromise;
    }
    openPromise = new Promise((resolve, reject) => {
      const request = indexedDBImpl.open(databaseName, version);
      request.onupgradeneeded = () => {
        const db = request.result;
        if (!db.objectStoreNames?.contains?.(storeName)) {
          db.createObjectStore(storeName, { keyPath: "key" });
        }
      };
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error ?? new Error("IndexedDB open failed"));
    });
    return openPromise;
  }

  async function withStore(mode, callback) {
    const db = await openDatabase();
    const tx = db.transaction(storeName, mode);
    const store = tx.objectStore(storeName);
    const result = await callback(store);
    if (tx.done && typeof tx.done.then === "function") {
      await tx.done;
    }
    return result;
  }

  return {
    async get(key) {
      const record = await withStore("readonly", (store) => requestToPromise(store.get(key)));
      return record?.value ?? null;
    },
    async set(key, value) {
      assertCachePayloadSafe(value);
      const sanitized = redactForTelemetry(value);
      await withStore("readwrite", (store) =>
        requestToPromise(store.put({ key, value: sanitized, updated_at: Date.now() })),
      );
      return sanitized;
    },
    async keys() {
      return withStore("readonly", async (store) => {
        if (typeof store.getAllKeys === "function") {
          return requestToPromise(store.getAllKeys());
        }
        return [];
      });
    },
  };
}

export function createDefaultMetadataCache(options = {}) {
  const indexedDBImpl = options.indexedDB ?? globalThis.indexedDB;
  if (indexedDBImpl && typeof indexedDBImpl.open === "function") {
    return createIndexedDbMetadataCache({ indexedDB: indexedDBImpl });
  }
  return createMemoryMetadataCache();
}

export function createWasmAnalysisClient(options = {}) {
  const chromeRuntime = options.chromeRuntime ?? globalThis.chrome?.runtime;
  const chromeApi = options.chromeApi ?? globalThis.chrome;
  if (
    options.preferOffscreen !== false &&
    !options.importImpl &&
    !options.wasmModule &&
    chromeApi?.offscreen &&
    chromeApi?.runtime
  ) {
    return createOffscreenWasmAnalysisClient({
      chromeApi,
      wasmFilePath: options.wasmFilePath,
    });
  }
  const importImpl = options.importImpl ?? null;
  const wasmModule = options.wasmModule ?? null;
  const wasmInit = options.wasmInit ?? null;
  const wasmModulePath = options.wasmModulePath ?? "metadata_checker.js";
  const wasmFilePath = options.wasmFilePath ?? "metadata_checker_bg.wasm";
  let runtimePromise = null;
  let initRuntimePromise = null;

  function extensionUrl(path) {
    if (!chromeRuntime || typeof chromeRuntime.getURL !== "function") {
      throw new Error("extension runtime URL resolver is unavailable");
    }
    return chromeRuntime.getURL(path);
  }

  async function loadRuntime() {
    if (runtimePromise) {
      return runtimePromise;
    }
    runtimePromise = (async () => {
      const module = importImpl
        ? await importImpl(extensionUrl(wasmModulePath))
        : wasmModule;
      if (!module) {
        throw new Error("WASM runtime module is unavailable outside offscreen document");
      }
      const init = importImpl
        ? module.default ?? module.init
        : wasmInit ?? module.default ?? module.init;
      if (typeof init === "function") {
        await init(extensionUrl(wasmFilePath));
      }
      return module;
    })();
    return runtimePromise;
  }

  async function callRuntime(method, args = []) {
    const module = await loadRuntime();
    const fn = module[method];
    if (typeof fn !== "function") {
      throw new Error(`WASM runtime method missing: ${method}`);
    }
    const result = await fn(...args);
    if (typeof result !== "string") {
      return result;
    }
    return safeJsonParse(result) ?? result;
  }

  async function callRuntimeWithFallback(methodCandidates, args = []) {
    const methodList = Array.isArray(methodCandidates)
      ? methodCandidates.filter((method) => typeof method === "string" && method.length > 0)
      : [];
    for (const method of methodList) {
      const module = await loadRuntime();
      const fn = module[method];
      if (typeof fn !== "function") {
        continue;
      }
      const result = await fn(...args);
      if (typeof result !== "string") {
        return result;
      }
      return safeJsonParse(result) ?? result;
    }
    throw new Error(`WASM runtime method missing: ${methodList[0]}`);
  }

  return {
    async initRuntime(optionsArg = {}) {
      if (!initRuntimePromise) {
        initRuntimePromise = callRuntime("initRuntime", [
          JSON.stringify(optionsArg ?? {}),
        ]).catch((error) => {
          initRuntimePromise = null;
          throw error;
        });
      }
      return initRuntimePromise;
    },
    async loadSuperpageDocument(sourcePath, rawText) {
      return callRuntime("loadSuperpageDocument", [sourcePath, rawText]);
    },
    async buildOrUpdateSuperpageGraph(sourcePath) {
      return callRuntime("buildOrUpdateSuperpageGraph", [sourcePath]);
    },
    async analyzeSuperpageSelection(selection, optionsArg = {}) {
      return callRuntime("analyzeSuperpageSelection", [
        JSON.stringify(selection ?? {}),
        JSON.stringify(optionsArg ?? {}),
      ]);
    },
    async analyzeLocalGraph(selection, optionsArg = {}) {
      return callRuntime("analyzeLocalGraph", [
        JSON.stringify(selection ?? {}),
        JSON.stringify(optionsArg ?? {}),
      ]);
    },
    async diffVisibleManifest(previousManifestJson, visibleManifestJson, projectRef = "") {
      return callRuntimeWithFallback(
        [
          "diffVisibleManifest",
          "diff_visible_manifest",
          "analyzeVisibleManifestDiff",
          "visibleManifestDiff",
          "diffManifest",
        ],
        [
          String(projectRef || ""),
          typeof previousManifestJson === "string"
            ? previousManifestJson
            : JSON.stringify(previousManifestJson ?? []),
          typeof visibleManifestJson === "string"
            ? visibleManifestJson
            : JSON.stringify(visibleManifestJson ?? []),
        ],
      );
    },
  };
}

export function createOffscreenWasmAnalysisClient(options = {}) {
  const chromeApi = options.chromeApi ?? globalThis.chrome;
  const wasmFilePath = options.wasmFilePath ?? "metadata_checker_bg.wasm";
  const offscreenPath = options.offscreenPath ?? "offscreen.html";
  const requestTimeoutMs = options.requestTimeoutMs ?? 30000;
  let creating = null;
  let requestCounter = 0;
  let initRuntimePromise = null;

  function extensionUrl(path) {
    if (!chromeApi?.runtime || typeof chromeApi.runtime.getURL !== "function") {
      throw new Error("extension runtime URL resolver is unavailable");
    }
    return chromeApi.runtime.getURL(path);
  }

  async function hasOffscreenDocument() {
    if (typeof chromeApi?.runtime?.getContexts === "function") {
      const contexts = await chromeApi.runtime.getContexts({
        contextTypes: ["OFFSCREEN_DOCUMENT"],
        documentUrls: [extensionUrl(offscreenPath)],
      });
      return Array.isArray(contexts) && contexts.length > 0;
    }
    if (typeof chromeApi?.offscreen?.hasDocument === "function") {
      return chromeApi.offscreen.hasDocument();
    }
    return false;
  }

  async function ensureOffscreenDocument() {
    if (!chromeApi?.offscreen || typeof chromeApi.offscreen.createDocument !== "function") {
      throw new Error("offscreen document API is unavailable");
    }
    if (await hasOffscreenDocument()) {
      return;
    }
    if (!creating) {
      creating = chromeApi.offscreen.createDocument({
        url: offscreenPath,
        reasons: ["WORKERS"],
        justification: "Run metadata-checker WebAssembly runtime outside the extension service worker.",
      }).finally(() => {
        creating = null;
      });
    }
    await creating;
  }

  async function sendOffscreenCall(method, args = []) {
    await ensureOffscreenDocument();
    requestCounter += 1;
    const requestId = `m45-offscreen-${requestCounter}`;
    const message = {
      type: OFFSCREEN_WASM_CALL_MESSAGE,
      request_id: requestId,
      payload: {
        method,
        args,
        wasm_url: extensionUrl(wasmFilePath),
      },
    };

    return new Promise((resolve, reject) => {
      let settled = false;
      const timeout = setTimeout(() => {
        if (!settled) {
          settled = true;
          reject(new Error("offscreen WASM runtime did not respond in time"));
        }
      }, requestTimeoutMs);
      chromeApi.runtime.sendMessage(message, (response) => {
        if (settled) {
          return;
        }
        settled = true;
        clearTimeout(timeout);
        const lastError = chromeApi.runtime.lastError;
        if (lastError) {
          const message = lastError.message || "offscreen WASM runtime unavailable";
          if (/receiving end does not exist/i.test(message)) {
            reject(new Error(
              "offscreen WASM runtime is not running; reload the extension and verify metadata_checker.js/metadata_checker_bg.wasm are packaged",
            ));
            return;
          }
          reject(new Error(message));
          return;
        }
        if (!response?.ok) {
          reject(new Error(response?.diagnostic?.message || "offscreen WASM runtime call failed"));
          return;
        }
        resolve(response.result);
      });
    });
  }

  async function callRuntime(method, args = []) {
    const result = await sendOffscreenCall(method, args);
    if (typeof result !== "string") {
      return result;
    }
    return safeJsonParse(result) ?? result;
  }

  async function callRuntimeWithFallback(methodCandidates, args = []) {
    const candidates = Array.isArray(methodCandidates)
      ? methodCandidates.filter((method) => typeof method === "string" && method.length > 0)
      : [];
    let lastError = null;
    for (const method of candidates) {
      try {
        return await callRuntime(method, args);
      } catch (error) {
        if (!error || !String(error.message || "").includes("WASM runtime method missing")) {
          throw error;
        }
        lastError = error;
      }
    }
    if (lastError) {
      throw lastError;
    }
    throw new Error(`WASM runtime method missing: ${candidates[0]}`);
  }

  return {
    async initRuntime(optionsArg = {}) {
      if (!initRuntimePromise) {
        initRuntimePromise = callRuntime("initRuntime", [
          JSON.stringify(optionsArg ?? {}),
        ]).catch((error) => {
          initRuntimePromise = null;
          throw error;
        });
      }
      return initRuntimePromise;
    },
    async loadSuperpageDocument(sourcePath, rawText) {
      return callRuntime("loadSuperpageDocument", [sourcePath, rawText]);
    },
    async buildOrUpdateSuperpageGraph(sourcePath) {
      return callRuntime("buildOrUpdateSuperpageGraph", [sourcePath]);
    },
    async analyzeSuperpageSelection(selection, optionsArg = {}) {
      return callRuntime("analyzeSuperpageSelection", [
        JSON.stringify(selection ?? {}),
        JSON.stringify(optionsArg ?? {}),
      ]);
    },
    async analyzeLocalGraph(selection, optionsArg = {}) {
      return callRuntime("analyzeLocalGraph", [
        JSON.stringify(selection ?? {}),
        JSON.stringify(optionsArg ?? {}),
      ]);
    },
    async diffVisibleManifest(previousManifestJson, visibleManifestJson, projectRef = "") {
      return callRuntimeWithFallback(
        [
          "diffVisibleManifest",
          "diff_visible_manifest",
          "analyzeVisibleManifestDiff",
          "visibleManifestDiff",
          "diffManifest",
        ],
        [
          String(projectRef || ""),
          typeof previousManifestJson === "string"
            ? previousManifestJson
            : JSON.stringify(previousManifestJson ?? []),
          typeof visibleManifestJson === "string"
            ? visibleManifestJson
            : JSON.stringify(visibleManifestJson ?? []),
        ],
      );
    },
  };
}

function createDefaultAnalysisClient() {
  if (!globalThis.chrome?.runtime?.getURL) {
    return null;
  }
  let client = null;
  function runtimeClient() {
    if (!client) {
      client = createWasmAnalysisClient();
    }
    return client;
  }
  return {
    async initRuntime(...args) {
      return runtimeClient().initRuntime(...args);
    },
    async loadSuperpageDocument(...args) {
      return runtimeClient().loadSuperpageDocument(...args);
    },
    async buildOrUpdateSuperpageGraph(...args) {
      return runtimeClient().buildOrUpdateSuperpageGraph(...args);
    },
    async analyzeSuperpageSelection(...args) {
      return runtimeClient().analyzeSuperpageSelection(...args);
    },
    async analyzeLocalGraph(...args) {
      return runtimeClient().analyzeLocalGraph(...args);
    },
    async diffVisibleManifest(...args) {
      return runtimeClient().diffVisibleManifest(...args);
    },
  };
}

export function createM45BackgroundController(options = {}) {
  const fetchImpl = options.fetchImpl ?? globalThis.fetch?.bind(globalThis);
  const clock = options.clock ?? (() => Date.now());
  const sleep = options.sleep ?? ((ms) => new Promise((resolve) => setTimeout(resolve, ms)));
  const cache = options.cache ?? createDefaultMetadataCache();
  const chromeApi = options.chrome ?? globalThis.chrome;
  const analysisClient = options.analysisClient === undefined
    ? createDefaultAnalysisClient()
    : options.analysisClient;
  const state = {
    last_bridge_status: null,
    last_diagnostic: null,
    updated_at: null,
    session: null,
    visible_manifest: [],
    visible_index: {
      status: "idle",
      projects: [],
      files: [],
      analyzable_count: 0,
      indexed_at: null,
    },
    background: {
      status: "idle",
      queue: [],
      pending_foreground: [],
      processed: 0,
      total: 0,
      active: 0,
      failed: 0,
      paused: false,
      max_concurrency: 1,
      min_interval_ms: 0,
      process_gate: Promise.resolve(),
      next_run_at: null,
      indexing_status: "idle",
      current_source_path: null,
      last_processed_source_path: null,
      last_failed_source_path: null,
      retry_available: false,
      last_event: null,
    },
    events: [],
    cache_stats: {
      hits: 0,
      misses: 0,
    },
  };

  function emit(eventName, payload = {}) {
    const event = {
      event: eventName,
      payload: redactForTelemetry(payload),
      timestamp: clock(),
    };
    state.events.push(event);
    state.background.last_event = event;
    if (state.events.length > 50) {
      state.events.shift();
    }
    return event;
  }

  function requireFetch() {
    if (typeof fetchImpl !== "function") {
      throw new Error("fetch is unavailable in extension service worker");
    }
    return fetchImpl;
  }

  async function request(baseUrl, path, init = {}) {
    const fetch = requireFetch();
    const response = await fetch(buildUrl(baseUrl, path), {
      method: "GET",
      credentials: "include",
      redirect: "follow",
      ...init,
      headers: {
        Accept: "application/json,text/plain,*/*",
        ...(init.headers ?? {}),
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
    return readJsonOrText(response);
  }

  async function requestText(baseUrl, path, init = {}) {
    const fetch = requireFetch();
    const response = await fetch(buildUrl(baseUrl, path), {
      method: "GET",
      credentials: "include",
      redirect: "follow",
      ...init,
      headers: {
        Accept: "application/json,text/plain,*/*",
        ...(init.headers ?? {}),
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

  async function tryCacheSet(key, value) {
    try {
      await cache.set(key, value);
      return true;
    } catch (error) {
      const diagnostic = stableDiagnostic(
        "REMOTE_METADATA_CACHE_WRITE_SKIPPED",
        error?.message || "metadata cache write skipped",
        "warning",
      );
      state.last_diagnostic = diagnostic;
      emit(M45_EVENT_TYPES.BACKGROUND_ANALYSIS_PROGRESS, {
        status: "cache_write_skipped",
        key,
        diagnostic,
      });
      return false;
    }
  }

  function metadataContentPath(file) {
    const ref = file.file_id || `${file.project_name}/${file.source_path}`;
    return `/api/meta/services/getFileContent/${encodeMetaPath(ref, true)}`;
  }

  function normalizeQueueLimit(value, fallback) {
    const parsed = typeof value === "string" ? Number(value) : value;
    if (!Number.isInteger(parsed) || parsed < 0) {
      return fallback;
    }
    return parsed;
  }

  function normalizeMaxConcurrency(value, fallback = 1) {
    const parsed = typeof value === "string" ? Number(value) : value;
    if (!Number.isInteger(parsed) || parsed < 1) {
      return fallback;
    }
    return parsed;
  }

  function normalizeMinIntervalMs(value, fallback = 0) {
    const parsed = typeof value === "string" ? Number(value) : value;
    if (!Number.isFinite(parsed) || parsed < 0) {
      return fallback;
    }
    return Math.max(0, Math.floor(parsed));
  }

  function normalizeLocalGraphOptions(options = {}) {
    const input = isObject(options) ? options : {};
    const maxNodes = normalizeQueueLimit(input.max_nodes, undefined);
    const maxEdges = normalizeQueueLimit(input.max_edges, undefined);
    const normalized = {
      depth: 2,
      visible_hop: 1,
    };
    if (maxNodes !== undefined) {
      normalized.max_nodes = maxNodes;
    }
    if (maxEdges !== undefined) {
      normalized.max_edges = maxEdges;
    }
    return normalized;
  }

  function buildForegroundSelectionItem(selection = {}) {
    const sourcePath = asString(selection.source_path ?? selection.sourcePath);
    if (!sourcePath) {
      return null;
    }
    const fileInfo = state.visible_index.files.find((file) => file.source_path === sourcePath) ?? {};
    const projectName = asString(
      selection.project_name ?? fileInfo.project_name ?? state.visible_index.projects[0]?.project_name,
    );
    const fileNameParts = sourcePath.split(".");
    const fileId = selection.file_id || fileInfo.file_id || null;
    const revision = selection.revision ?? fileInfo.revision ?? null;
    return {
      project_name: projectName,
      source_path: sourcePath,
      file_id: fileId,
      revision,
      extension: fileInfo.extension ?? fileNameParts.pop()?.toLowerCase() ?? "",
      analyzable: true,
      base_url: state.session?.base_url ?? null,
      priority: 0,
      foreground: true,
      active_component_id: selection.active_component_id ?? selection.activeComponentId ?? null,
      selected_component_ids: asArray(selection.selected_component_ids ?? selection.selectedComponentIds),
      cache_key: makeCacheKey(state.session?.base_url, {
        project_name: projectName,
        source_path: sourcePath,
        file_id: fileId,
        revision: revision ?? "",
      }),
    };
  }

  function buildLocalGraphSelectionPayload(selection = {}) {
    const item = buildForegroundSelectionItem(selection);
    if (!item) {
      return null;
    }
    return {
      source_path: item.source_path,
      file_id: item.file_id ?? "",
      project_name: item.project_name,
      active_component_id: item.active_component_id ?? null,
      selected_component_ids: item.selected_component_ids ?? [],
      cache_key: item.cache_key,
      revision: item.revision ?? "",
    };
  }

  async function getExistingLocalGraphArtifact(item, artifactKey) {
    function isUsableArtifact(value) {
      const status = String(value?.analysis_status || value?.result?.status || value?.status || "").toLowerCase();
      return status === "ready" || status === "empty";
    }
    if (artifactKey) {
      const cachedArtifact = await cache.get(artifactKey);
      if (isUsableArtifact(cachedArtifact)) {
        return {
          key: artifactKey,
          artifact: cachedArtifact,
        };
      }
    }
    const sourceArtifactKey = makeAnalysisArtifactKey({
      project_name: item.project_name,
      source_path: item.source_path,
      file_id: item.file_id,
      revision: item.revision,
      cache_key: item.cache_key,
      foreground: false,
    });
    const backgroundArtifact = await cache.get(sourceArtifactKey);
    if (isUsableArtifact(backgroundArtifact)) {
      return {
        key: sourceArtifactKey,
        artifact: backgroundArtifact,
      };
    }
    return null;
  }

  async function runLocalGraphAnalysis(item, optionsArg = {}, artifactKey) {
    const baseArtifact = {
      kind: "metadata-analysis-artifact",
      source_path: item.source_path,
      project_name: item.project_name,
      file_id: item.file_id,
      revision: item.revision,
    };
    if (!analysisClient) {
      return {
        ...baseArtifact,
        analysis_status: "runtime_unavailable",
        diagnostics: [
          stableDiagnostic(
            "LOCAL_GRAPH_ANALYSIS_RUNTIME_UNAVAILABLE",
            "local graph runtime client is unavailable",
          ),
        ],
      };
    }
    if (typeof analysisClient.analyzeLocalGraph !== "function") {
      return {
        ...baseArtifact,
        analysis_status: "runtime_unsupported",
        diagnostics: [
          stableDiagnostic(
            "METADATA_CHECKER_ANALYZE_LOCAL_GRAPH_UNSUPPORTED",
            "runtime analyzeLocalGraph API is unavailable",
            "error",
          ),
        ],
      };
    }
    try {
      const normalizedOptions = normalizeLocalGraphOptions(optionsArg);
      if (typeof analysisClient.buildOrUpdateSuperpageGraph === "function") {
        await analysisClient.buildOrUpdateSuperpageGraph(item.source_path);
      }
      const result = await analysisClient.analyzeLocalGraph(
        {
          source_path: item.source_path,
          file_id: item.file_id ?? "",
          project_name: item.project_name,
          active_component_id: item.active_component_id ?? null,
          selected_component_ids: item.selected_component_ids ?? [],
        },
        normalizedOptions,
      );
      const artifact = {
        ...baseArtifact,
        analysis_status: (result && result.status) || "ready",
        result: redactForTelemetry(result ?? { status: "ready" }),
      };
      if (artifactKey) {
        await tryCacheSet(artifactKey, artifact);
      }
      return artifact;
    } catch (error) {
      return {
        ...baseArtifact,
        analysis_status: "error",
        diagnostics: [
          stableDiagnostic(
            "METADATA_CHECKER_ANALYZE_LOCAL_GRAPH_FAILED",
            error?.message || "metadata local graph analysis failed",
            "error",
          ),
        ],
      };
    }
  }

  async function bootstrapWithAccessToken({ base_url, baseUrl, access_token }) {
    const base = base_url ?? baseUrl;
    if (!access_token || typeof access_token !== "string") {
      const diagnostic = stableDiagnostic(
        "ACCESS_TOKEN_UNAVAILABLE",
        "access token is required",
        "error",
      );
      state.last_diagnostic = diagnostic;
      return { ok: false, diagnostics: [diagnostic] };
    }
    try {
      const token = encodeURIComponent(access_token);
      const whoami = await request(base, `/api/me/whoami?access_token=${token}`);
      if (whoami?.anonymous === true) {
        const diagnostic = stableDiagnostic(
          "SESSION_BOOTSTRAP_ANONYMOUS",
          "whoami returned anonymous user",
          "error",
        );
        state.last_diagnostic = diagnostic;
        return { ok: false, diagnostics: [diagnostic] };
      }
      const userId = whoami?.userId ?? whoami?.user_id;
      if (!userId) {
        const diagnostic = stableDiagnostic(
          "SESSION_BOOTSTRAP_FAILED",
          "whoami response missing userId",
          "error",
        );
        state.last_diagnostic = diagnostic;
        return { ok: false, diagnostics: [diagnostic] };
      }
      state.session = {
        status: "ready",
        base_url: base,
        user_id: userId,
        user_name: whoami?.userName ?? whoami?.user_name ?? null,
        bootstrapped_at: clock(),
      };
      state.updated_at = clock();
      emit(M45_EVENT_TYPES.SESSION_BOOTSTRAPPED, {
        user_id: state.session.user_id,
        user_name: state.session.user_name,
      });
      return { ok: true, session: { ...state.session } };
    } catch (error) {
      const diagnostic = stableDiagnostic(
        "SESSION_BOOTSTRAP_FAILED",
        error?.message || "session bootstrap failed",
        "error",
      );
      state.last_diagnostic = diagnostic;
      return { ok: false, diagnostics: [diagnostic] };
    }
  }

  async function listVisibleMetadata(context = {}) {
    const { base_url, baseUrl } = context;
    const base = base_url ?? baseUrl ?? state.session?.base_url;
    if (!base) {
      throw new Error("SESSION_BOOTSTRAP_FAILED: base_url is required");
    }
    const permissionInfo = await request(base, "/api/me/getPermissionInfo");
    const projects = wrappedArray(permissionInfo, "metaProjects")
      .map((project) => ({
        ...project,
        project_name: inferProjectName(project),
      }))
      .filter((project) => project.project_name);
    const fallbackProjectName = contextProjectName(context);
    if (projects.length === 0 && fallbackProjectName) {
      projects.push({
        projectName: fallbackProjectName,
        project_name: fallbackProjectName,
      });
    }
    const files = [];
    for (const project of projects) {
      const projectName = project.project_name;
      const children = await request(
        base,
        `/api/meta/services/getFileChildren/${encodeMetaPath(projectName, false)}`,
      );
      for (const child of wrappedArray(children, "children")) {
        if (!isFolder(child)) {
          files.push(normalizeFile(projectName, child));
          continue;
        }
        const childPath = resourcePath(child).replace(/^\/+/, "");
        if (!childPath) {
          continue;
        }
        const descendants = await request(
          base,
          `/api/meta/services/getFileDescendant/${encodeMetaPath(childPath, true)}`,
        );
        for (const file of wrappedArray(descendants, "files")) {
          files.push(normalizeFile(projectName, file));
        }
      }
    }
    const visibleFiles = files.filter(Boolean);
    const analyzable = visibleFiles.filter((file) => file.analyzable);
    const projectManifest = projects.find((project) => isObject(project));
    const projectName = contextProjectName(context)
      || projectManifest?.project_name
      || projectManifest?.projectName
      || projectManifest?.name
      || "";
    const visibleManifest = visibleFiles.map((file) => normalizeVisibleManifestEntry(projectName, file));
    state.visible_index = {
      status: "ready",
      projects,
      files: visibleFiles,
      analyzable_count: analyzable.length,
      indexed_at: clock(),
    };
    state.visible_manifest = visibleManifest;
    await tryCacheSet(`visible-index|${base}`, state.visible_index);
    const shouldSeedQueue = context.seed_queue !== false;
    if (shouldSeedQueue) {
      seedBackgroundQueue(analyzable, {
        base_url: base,
        current_source_path: context.current_source_path ?? context.currentSourcePath,
        current_dependency_paths: context.current_dependency_paths
          ?? context.currentDependencyPaths
          ?? context.dependency_paths,
      });
    }
    emit(M45_EVENT_TYPES.VISIBLE_METADATA_INDEXED, {
      project_count: projects.length,
      file_count: visibleFiles.length,
      analyzable_count: analyzable.length,
    });
    return state.visible_index;
  }

  function buildBackgroundQueueItemFromManifestRef(fileRef = {}, context = {}) {
    const sourcePath = asString(fileRef.source_path);
    if (!sourcePath) {
      return null;
    }
    const fileInfo = state.visible_index.files.find((file) => file.source_path === sourcePath) ?? {};
    const projectName = asString(
      fileRef.project_ref
      || fileRef.project_name
      || fileInfo.project_name
      || contextProjectName(context)
      || state.visible_index.projects[0]?.project_name,
    );
    const fileId = fileRef.file_id ?? fileInfo.file_id ?? null;
    const revision = fileRef.revision ?? fileInfo.revision ?? null;
    const baseUrl = context.base_url ?? context.baseUrl ?? state.session?.base_url ?? null;
    const fileNameParts = sourcePath.split(".");
    return {
      project_name: projectName,
      source_path: sourcePath,
      file_id: fileId,
      revision,
      extension: fileInfo.extension ?? fileNameParts.pop()?.toLowerCase() ?? "",
      analyzable: true,
      base_url: baseUrl,
      priority: 1,
      foreground: false,
      active_component_id: null,
      selected_component_ids: [],
      cache_key: makeCacheKey(baseUrl, {
        project_name: projectName,
        source_path: sourcePath,
        file_id: fileId,
        revision: revision ?? "",
      }),
    };
  }

  async function refreshVisibleManifest(context = {}) {
    const previousManifest = asArray(state.visible_manifest);
    const timings = {
      manifest_fetch_ms: 0,
      manifest_diff_ms: 0,
      changed_content_fetch_ms: 0,
      wasm_update_ms: 0,
    };
    const manifestFetchStartedAt = clock();
    let visibleIndex;
    try {
      visibleIndex = await listVisibleMetadata({
        ...context,
        seed_queue: false,
      });
    } catch (error) {
      const diagnostic = stableDiagnostic(
        "METADATA_CHECKER_VISIBLE_MANIFEST_REFRESH_FAILED",
        error?.message || "visible manifest refresh failed",
        "error",
      );
      state.last_diagnostic = diagnostic;
      return {
        ok: false,
        diagnostics: [diagnostic],
        manifest_diff: {
          added: null,
          modified: null,
          deleted: null,
          unchanged: null,
          content_queue_count: null,
        },
        timings,
      };
    }
    timings.manifest_fetch_ms = Math.round(clock() - manifestFetchStartedAt);

    if (!analysisClient || typeof analysisClient.diffVisibleManifest !== "function") {
      const diagnostic = stableDiagnostic(
        "METADATA_CHECKER_VISIBLE_MANIFEST_DIFF_UNAVAILABLE",
        "WASM visible manifest diff API is unavailable",
        "error",
      );
      state.last_diagnostic = diagnostic;
      return {
        ok: false,
        diagnostics: [diagnostic],
        visible_index: visibleIndex,
        manifest_diff: {
          added: null,
          modified: null,
          deleted: null,
          unchanged: null,
          content_queue_count: null,
        },
        timings,
      };
    }

    const currentManifest = asArray(state.visible_manifest);
    const projectRef = contextProjectName(context)
      || state.visible_index.projects[0]?.project_name
      || currentManifest.find((entry) => asString(entry?.project_name))?.project_name
      || "";
    let diffDetail;
    try {
      const diffStartedAt = clock();
      const diffResult = await analysisClient.diffVisibleManifest(
        JSON.stringify(previousManifest),
        JSON.stringify(currentManifest),
        projectRef,
      );
      timings.manifest_diff_ms = Math.round(clock() - diffStartedAt);
      diffDetail = pickVisibleManifestDiffDetail(diffResult);
    } catch (error) {
      const diagnostic = stableDiagnostic(
        "METADATA_CHECKER_VISIBLE_MANIFEST_DIFF_FAILED",
        error?.message || "visible manifest diff failed",
        "error",
      );
      state.last_diagnostic = diagnostic;
      return {
        ok: false,
        diagnostics: [diagnostic],
        visible_index: visibleIndex,
        manifest_diff: {
          added: null,
          modified: null,
          deleted: null,
          unchanged: null,
          content_queue_count: null,
        },
        timings,
      };
    }

    if (!diffDetail) {
      const diagnostic = stableDiagnostic(
        "METADATA_CHECKER_VISIBLE_MANIFEST_DIFF_EMPTY",
        "WASM visible manifest diff did not return a visible_manifest_diff item",
        "error",
      );
      state.last_diagnostic = diagnostic;
      return {
        ok: false,
        diagnostics: [diagnostic],
        visible_index: visibleIndex,
        manifest_diff: {
          added: null,
          modified: null,
          deleted: null,
          unchanged: null,
          content_queue_count: null,
        },
        timings,
      };
    }

    if (diffDetail?.timing?.total_ms !== undefined) {
      timings.manifest_diff_ms = normalizeManifestCount(diffDetail.timing.total_ms, timings.manifest_diff_ms);
    }

    const contentQueue = asArray(diffDetail.content_queue)
      .map((fileRef) => buildBackgroundQueueItemFromManifestRef(fileRef, context))
      .filter(Boolean);
    if (contentQueue.length > 0) {
      const seen = new Set();
      const uniqueQueue = contentQueue.filter((item) => {
        if (seen.has(item.source_path)) {
          return false;
        }
        seen.add(item.source_path);
        return true;
      });
      state.background.queue = [
        ...uniqueQueue,
        ...state.background.queue.filter((item) => !seen.has(item.source_path)),
      ];
      state.background.total = Math.max(
        state.background.total,
        state.background.processed + state.background.active + state.background.queue.length,
      );
      state.background.status = "queued";
      state.background.indexing_status = "indexing_background";
      state.background.retry_available = true;
      const updateStartedAt = clock();
      const processed = await processBackgroundQueue({
        limit: normalizeQueueLimit(context.refresh_limit ?? context.limit, uniqueQueue.length),
        max_concurrency: normalizeMaxConcurrency(context.max_concurrency, state.background.max_concurrency),
        min_interval_ms: normalizeMinIntervalMs(context.min_interval_ms, state.background.min_interval_ms),
      });
      timings.wasm_update_ms = Math.round(clock() - updateStartedAt);
      timings.changed_content_fetch_ms = processed.artifacts
        .map((entry) => normalizeManifestCount(entry?.timings?.content_fetch_ms, 0))
        .reduce((total, value) => total + value, 0);
    }

    const manifestDiff = {
      added: normalizeManifestCount(diffDetail.added?.length, 0),
      modified: normalizeManifestCount(diffDetail.modified?.length, 0),
      deleted: normalizeManifestCount(diffDetail.deleted?.length, 0),
      unchanged: normalizeManifestCount(diffDetail.unchanged?.length, 0),
      content_queue_count: normalizeManifestCount(diffDetail.content_queue?.length, 0),
      changed_files_count: normalizeManifestCount(diffDetail.changed_files?.length, 0),
      previous_manifest_count: normalizeManifestCount(diffDetail.previous_manifest_count, previousManifest.length),
      visible_manifest_count: normalizeManifestCount(diffDetail.visible_manifest_count, currentManifest.length),
    };
    emit(M45_EVENT_TYPES.VISIBLE_METADATA_INDEXED, {
      status: "manifest_refreshed",
      ...manifestDiff,
    });
    return {
      ok: true,
      visible_index: visibleIndex,
      manifest_diff: manifestDiff,
      timings,
      diagnostics: asArray(diffDetail.diagnostics).map((item) =>
        stableDiagnostic(
          asString(item?.code) || "METADATA_CHECKER_VISIBLE_MANIFEST_DIFF_DIAGNOSTIC",
          asString(item?.message) || "visible manifest diff diagnostic",
          "warning",
        ),
      ),
      background: { ...state.background },
      cache_stats: { ...state.cache_stats },
    };
  }

  function normalizeFile(projectName, file) {
    if (!file || isFolder(file)) {
      return null;
    }
    const path = resourcePath(file);
    if (!path) {
      return null;
    }
    const sourcePath = normalizeSourcePath(path, projectName);
    const ext = fileExtension(file);
    const manifestEntry = normalizeVisibleManifestEntry(projectName, file);
    return {
      project_name: projectName,
      source_path: sourcePath,
      id: manifestEntry.id,
      path: manifestEntry.path,
      type: manifestEntry.type,
      file_id: file.id ?? file.fileId ?? file.file_id ?? null,
      revision: manifestEntry.revision,
      modifyTime: manifestEntry.modifyTime,
      modifier: manifestEntry.modifier,
      modifierName: manifestEntry.modifierName,
      extension: ext,
      analyzable: SUPPORTED_METADATA_EXTENSIONS.has(ext),
      isFolder: false,
    };
  }

  function priorityForFile(file, context = {}) {
    const current = context.current_source_path ?? context.currentSourcePath ?? "";
    const dependencyPaths = asArray(
      context.current_dependency_paths ?? context.currentDependencyPaths ?? context.dependency_paths,
    );
    if (current && file.source_path === current) {
      return 0;
    }
    if (dependencyPaths.includes(file.source_path)) {
      return 10;
    }
    const currentApp = current.match(/^(.*?\.app)\//)?.[1];
    if (currentApp && file.source_path.startsWith(`${currentApp}/`)) {
      return 20;
    }
    const currentModule = current.split("/")[0] || "";
    if (currentModule && file.source_path.startsWith(`${currentModule}/`)) {
      return 30;
    }
    return 50;
  }

  function seedBackgroundQueue(files, context = {}) {
    const base = context.base_url ?? context.baseUrl ?? state.session?.base_url;
    const indexedBySource = new Map(
      files
      .filter((file) => file && file.source_path)
      .map((file) => [file.source_path, file]),
    );
    const pendingSelections = state.background.pending_foreground;
    const pending = pendingSelections
      .map((item) => {
        const indexed = indexedBySource.get(item.source_path) ?? {};
        const projectName = item.project_name || indexed.project_name || "";
        const fileId = item.file_id || indexed.file_id || null;
        const revision = item.revision ?? indexed.revision ?? null;
        return {
          ...item,
          project_name: projectName,
          file_id: fileId,
          revision,
          extension: indexed.extension || item.extension,
          analyzable: true,
          base_url: base,
          priority: 0,
          foreground: true,
          cache_key: makeCacheKey(base, {
            project_name: projectName,
            source_path: item.source_path,
            file_id: fileId,
            revision: revision ?? "",
          }),
        };
      })
      .filter((item) => item.source_path);
    state.background.pending_foreground = [];
    const usedSources = new Set();
    const queueFromVisible = files
      .map((file) => ({
        ...file,
        base_url: base,
        priority: priorityForFile(file, context),
        cache_key: makeCacheKey(base, file),
      }))
      .sort((a, b) => a.priority - b.priority || a.source_path.localeCompare(b.source_path));
    const queue = [
      ...pending.filter((item) => {
        if (usedSources.has(item.source_path)) {
          return false;
        }
        usedSources.add(item.source_path);
        return true;
      }),
      ...queueFromVisible.filter((file) => {
        if (usedSources.has(file.source_path)) {
          return false;
        }
        usedSources.add(file.source_path);
        return true;
      }),
    ];
    state.background.queue = queue;
    state.background.total = queue.length;
    state.background.processed = 0;
    state.background.failed = 0;
    state.background.current_source_path = null;
    state.background.last_processed_source_path = null;
    state.background.last_failed_source_path = null;
    state.background.retry_available = queue.length > 0;
    state.background.next_run_at = null;
    state.background.status = queue.length > 0 ? "queued" : "idle";
    state.background.indexing_status = queue.length > 0 ? "queued" : "idle";
    return queue;
  }

  function enqueueForegroundSelection(selection = {}) {
    const file = buildForegroundSelectionItem(selection);
    if (!file) {
      return { queued: false };
    }
    const isReady = state.session?.base_url && state.visible_index.status === "ready";
    if (!isReady) {
      const indexingStatus = state.session?.base_url ? "indexing_current_page" : "waiting_for_metadata";
      state.background.pending_foreground = [
        file,
        ...state.background.pending_foreground.filter((item) => item.source_path !== file.source_path),
      ];
      state.background.status = indexingStatus;
      state.background.indexing_status = indexingStatus;
      state.background.current_source_path = file.source_path;
      state.background.retry_available = true;
      emit(M45_EVENT_TYPES.BACKGROUND_ANALYSIS_PROGRESS, {
        status: indexingStatus,
        source_path: file.source_path,
        retry_available: true,
      });
      return { queued: true, file, pending: true, indexing_status: indexingStatus, retry_available: true };
    }
    file.base_url = state.session.base_url;
    file.cache_key = makeCacheKey(file.base_url, {
      project_name: file.project_name,
      source_path: file.source_path,
      file_id: file.file_id,
      revision: file.revision ?? "",
    });
    state.background.queue = [
      file,
      ...state.background.queue.filter((item) => item.source_path !== file.source_path),
    ];
    state.background.total = Math.max(
      state.background.total,
      state.background.processed + state.background.active + state.background.queue.length,
    );
    state.background.status = "queued";
    state.background.indexing_status = "indexing_current_page";
    state.background.current_source_path = file.source_path;
    state.background.retry_available = true;
    emit(M45_EVENT_TYPES.BACKGROUND_ANALYSIS_PROGRESS, {
      status: "indexing_current_page",
      source_path: file.source_path,
      retry_available: true,
    });
    return { queued: true, file };
  }

  async function runBackgroundQueueItem(item) {
    const artifactKey = makeAnalysisArtifactKey(item);
    const timings = {
      content_fetch_ms: 0,
    };
    state.background.current_source_path = item.source_path;
    state.background.indexing_status = item.foreground ? "indexing_current_page" : "indexing_background";
    state.background.retry_available = true;
    const cached = await cache.get(artifactKey);
    if (cached) {
      state.cache_stats.hits += 1;
      state.background.last_processed_source_path = item.source_path;
      return {
        cached,
        item,
        artifact_key: artifactKey,
        cache_hit: true,
        timings,
      };
    }
    state.cache_stats.misses += 1;
    try {
      const fetchStartedAt = clock();
      const rawText = await requestText(item.base_url, metadataContentPath(item));
      timings.content_fetch_ms = Math.round(clock() - fetchStartedAt);
      await tryCacheSet(`raw-metadata|${item.cache_key}`, {
        kind: "raw-metadata",
        source_path: item.source_path,
        project_name: item.project_name,
        file_id: item.file_id,
        revision: item.revision,
        raw_text: rawText,
      });
      const artifact = await runBackgroundAnalysis(item, rawText);
      if (artifact?.analysis_status === "error") {
        state.background.failed += 1;
        state.background.last_failed_source_path = item.source_path;
        state.background.retry_available = true;
      }
      await tryCacheSet(artifactKey, artifact);
      state.background.last_processed_source_path = item.source_path;
      return {
        cached: artifact,
        item,
        artifact_key: artifactKey,
        cache_hit: false,
        timings,
      };
    } catch (error) {
      state.background.failed += 1;
      state.background.last_failed_source_path = item.source_path;
      const diagnostic = stableDiagnostic(
        "BACKGROUND_ANALYSIS_FAILED",
        error?.message || "background analysis failed",
        "error",
      );
      state.last_diagnostic = diagnostic;
      const artifact = {
        kind: "metadata-analysis-artifact",
        source_path: item.source_path,
        project_name: item.project_name,
        file_id: item.file_id,
        revision: item.revision,
        analysis_status: "error",
        diagnostics: [diagnostic],
      };
      await tryCacheSet(artifactKey, artifact);
      return {
        cached: artifact,
        item,
        artifact_key: artifactKey,
        cache_hit: false,
        timings,
      };
    }
  }

  async function processBackgroundQueue({
    limit = 3,
    max_concurrency = state.background.max_concurrency,
    min_interval_ms = state.background.min_interval_ms,
  } = {}) {
    const gate = state.background.process_gate;
    const result = gate.then(() => processBackgroundQueueUnserialized({
      limit,
      max_concurrency,
      min_interval_ms,
    }));
    state.background.process_gate = result.catch(() => {}).then(() => undefined);
    return result;
  }

  async function processBackgroundQueueUnserialized({
    limit = 3,
    max_concurrency = state.background.max_concurrency,
    min_interval_ms = state.background.min_interval_ms,
  } = {}) {
    const normalizedLimit = normalizeQueueLimit(limit, 3);
    const normalizedMaxConcurrency = normalizeMaxConcurrency(max_concurrency, state.background.max_concurrency ?? 1);
    const normalizedMinInterval = normalizeMinIntervalMs(min_interval_ms, state.background.min_interval_ms);
    state.background.max_concurrency = normalizedMaxConcurrency;
    state.background.min_interval_ms = normalizedMinInterval;
    const processed = [];
    const artifacts = [];
    const reservedItems = [];
    let reservedCount = 0;
    let nextStartAt = state.background.next_run_at ?? null;
    let startGate = Promise.resolve();
    state.background.status = "running";

    if (normalizedLimit <= 0) {
      state.background.status = state.background.paused
        ? "paused"
        : state.background.queue.length > 0 ? "queued" : "idle";
      state.background.indexing_status = state.background.status;
      return { processed, artifacts, background: { ...state.background }, cache_stats: { ...state.cache_stats } };
    }

    async function waitForStartSlot() {
      if (normalizedMinInterval <= 0) {
        state.background.next_run_at = null;
        return;
      }
      const gate = startGate;
      const slot = (async () => {
        await gate;
        const now = clock();
        const nextAt = Math.max(nextStartAt ?? now, now);
        const delayMs = Math.max(0, nextAt - now);
        if (delayMs > 0) {
          await sleep(delayMs);
        }
        const startedAt = clock();
        nextStartAt = startedAt + normalizedMinInterval;
        state.background.next_run_at = nextStartAt;
      })();
      startGate = slot;
      await slot;
    }

    function reserveSlot() {
      if (state.background.paused || reservedCount >= normalizedLimit || state.background.queue.length === 0) {
        return null;
      }
      const item = state.background.queue.shift();
      if (!item) {
        return null;
      }
      reservedCount += 1;
      return item;
    }

    function requeuePausedReservedItems() {
      if (reservedItems.length === 0) {
        return;
      }
      state.background.queue.unshift(...reservedItems);
      reservedItems.length = 0;
    }

    async function worker() {
      while (true) {
        const item = reserveSlot();
        if (!item) {
          return;
        }
        await waitForStartSlot();
        if (state.background.paused) {
          reservedItems.push(item);
          reservedCount -= 1;
          return;
        }
        state.background.active += 1;
        try {
          const result = await runBackgroundQueueItem(item);
          processed.push(item);
          artifacts.push(result);
          state.background.processed += 1;
          state.background.total = Math.max(
            state.background.total,
            state.background.processed,
          );
          emit(M45_EVENT_TYPES.METADATA_PREFETCHED, {
            source_path: item.source_path,
            cache_hit: result.cache_hit,
            artifact_key: result.artifact_key,
          });
          emit(M45_EVENT_TYPES.BACKGROUND_ANALYSIS_PROGRESS, {
            processed: state.background.processed,
            total: state.background.total,
            failed: state.background.failed,
            current_source_path: state.background.current_source_path,
            source_path: item.source_path,
            artifact_key: result.artifact_key,
            cache_hits: state.cache_stats.hits,
            cache_misses: state.cache_stats.misses,
            retry_available: state.background.retry_available,
          });
        } finally {
          state.background.active -= 1;
        }
        if (state.background.paused) {
          return;
        }
      }
    }

    await Promise.all(Array.from({ length: normalizedMaxConcurrency }, () => worker()));

    requeuePausedReservedItems();

    state.background.status = state.background.paused
      ? "paused"
      : state.background.queue.length === 0 ? "completed" : "queued";
    state.background.indexing_status = state.background.status;
    state.background.current_source_path = null;
    state.background.retry_available = state.background.queue.length > 0 || state.background.failed > 0;
    if (state.background.status === "completed") {
      emit(M45_EVENT_TYPES.BACKGROUND_ANALYSIS_COMPLETED, {
        processed: state.background.processed,
        total: state.background.total,
        failed: state.background.failed,
        cache_hits: state.cache_stats.hits,
        cache_misses: state.cache_stats.misses,
      });
    }
    return { processed, artifacts, background: { ...state.background }, cache_stats: { ...state.cache_stats } };
  }

  async function runBackgroundAnalysis(item, rawText) {
    const baseArtifact = {
      kind: "metadata-analysis-artifact",
      source_path: item.source_path,
      project_name: item.project_name,
      file_id: item.file_id,
      revision: item.revision,
    };
    if (!analysisClient) {
      return {
        ...baseArtifact,
        analysis_status: "runtime_unavailable",
        diagnostics: [
          stableDiagnostic(
            "BACKGROUND_ANALYSIS_RUNTIME_UNAVAILABLE",
            "background runtime client is unavailable",
          ),
        ],
      };
    }
    try {
      if (typeof analysisClient.initRuntime === "function") {
        await analysisClient.initRuntime({
          project_ref: item.project_name ?? null,
        });
      }
      if (typeof analysisClient.loadSuperpageDocument === "function") {
        await analysisClient.loadSuperpageDocument(item.source_path, rawText);
      }
      if (typeof analysisClient.buildOrUpdateSuperpageGraph === "function") {
        await analysisClient.buildOrUpdateSuperpageGraph(item.source_path);
      }
      const result = typeof analysisClient.analyzeSuperpageSelection === "function"
        ? await analysisClient.analyzeSuperpageSelection(
          {
            source_path: item.source_path,
            file_id: item.file_id ?? "",
            project_name: item.project_name,
            active_component_id: item.foreground ? item.active_component_id ?? null : null,
            selected_component_ids: item.foreground ? item.selected_component_ids ?? [] : [],
          },
          {
            mode: item.foreground ? "foreground" : "background",
            include_priority: false,
            include_conditions: true,
            include_dataflow: false,
          },
        )
        : null;
      return {
        ...baseArtifact,
        analysis_status: "ready",
        result: redactForTelemetry(result ?? { status: "ready" }),
      };
    } catch (error) {
      return {
        ...baseArtifact,
        analysis_status: "error",
        diagnostics: [
          stableDiagnostic(
            "BACKGROUND_ANALYSIS_FAILED",
            error?.message || "background analysis failed",
            "error",
          ),
        ],
      };
    }
  }

  async function bootstrapAndIndex(payload = {}) {
    const bootstrapped = await bootstrapWithAccessToken(payload);
    if (!bootstrapped.ok) {
      return bootstrapped;
    }
    let visibleIndex;
    try {
      visibleIndex = await listVisibleMetadata(payload);
    } catch (error) {
      const message = error?.message || "visible metadata index failed";
      const code = message.includes("REMOTE_METADATA_UNAUTHORIZED") || message.includes("REMOTE_METADATA_FORBIDDEN")
        ? "SESSION_COOKIE_NOT_ESTABLISHED"
        : "SESSION_BOOTSTRAP_FAILED";
      const diagnostic = stableDiagnostic(code, message, "error");
      state.last_diagnostic = diagnostic;
      return { ok: false, session: bootstrapped.session, diagnostics: [diagnostic] };
    }
    const queueResult = await processBackgroundQueue({
      limit: payload.initial_limit ?? 3,
      max_concurrency: payload.initial_max_concurrency ?? payload.max_concurrency,
      min_interval_ms: payload.min_interval_ms,
    });
    return {
      ok: true,
      session: bootstrapped.session,
      visible_index: visibleIndex,
      background: queueResult.background,
      cache_stats: { ...state.cache_stats },
    };
  }

  function getState() {
    return redactForTelemetry(state);
  }

  function asNumber(value, fallback = 0) {
    const candidate = typeof value === "number" ? value : Number(value);
    return Number.isFinite(candidate) ? candidate : fallback;
  }

  function pickPopupSession(session = null) {
    if (!isObject(session)) {
      return null;
    }
    return {
      status: asString(session.status) || null,
      base_url: asString(session.base_url) || null,
      user_id: asString(session.user_id) || asString(session.userId) || null,
      user_name: asString(session.user_name) || asString(session.userName) || null,
      bootstrapped_at: session.bootstrapped_at ?? null,
    };
  }

  function pickPopupProject(project = {}) {
    if (!isObject(project)) {
      return null;
    }
    return {
      project_name: asString(project.project_name || project.projectName) || null,
    };
  }

  function pickPopupFile(file = {}) {
    if (!isObject(file)) {
      return null;
    }
    return {
      project_name: asString(file.project_name) || null,
      source_path: asString(file.source_path) || null,
      file_id: asString(file.file_id) || null,
      extension: asString(file.extension) || null,
      analyzable: typeof file.analyzable === "boolean" ? file.analyzable : false,
      revision: asString(file.revision) || null,
    };
  }

  function pickPopupVisibleIndex(visibleIndex = {}) {
    return {
      status: asString(visibleIndex.status) || "idle",
      projects: asArray(visibleIndex.projects)
        .map(pickPopupProject)
        .filter((item) => item !== null),
      files: asArray(visibleIndex.files).map(pickPopupFile).filter((item) => item !== null),
      analyzable_count: asNumber(visibleIndex.analyzable_count),
      indexed_at: visibleIndex.indexed_at ?? null,
    };
  }

  function pickPopupBackground(background = {}) {
    return {
      status: asString(background.status) || "idle",
      indexing_status: asString(background.indexing_status) || "idle",
      total: asNumber(background.total),
      processed: asNumber(background.processed),
      active: asNumber(background.active),
      failed: asNumber(background.failed),
      queue_length: asArray(background.queue).length,
      pending_foreground_count: asArray(background.pending_foreground).length,
      paused: Boolean(background.paused),
      retry_available: Boolean(background.retry_available),
      current_source_path: asString(background.current_source_path) || null,
      last_processed_source_path: asString(background.last_processed_source_path) || null,
      last_failed_source_path: asString(background.last_failed_source_path) || null,
      max_concurrency: asNumber(background.max_concurrency, 1),
      min_interval_ms: asNumber(background.min_interval_ms),
      next_run_at: background.next_run_at ?? null,
    };
  }

  function pickPopupCacheStats(stats = {}) {
    return {
      hits: asNumber(stats.hits),
      misses: asNumber(stats.misses),
    };
  }

  function pickPopupRuntime() {
    const hasRuntime = Boolean(
      analysisClient
      && typeof analysisClient.loadSuperpageDocument === "function"
      && typeof analysisClient.buildOrUpdateSuperpageGraph === "function"
      && typeof analysisClient.analyzeSuperpageSelection === "function"
      && typeof analysisClient.analyzeLocalGraph === "function"
    );
    return {
      available: hasRuntime,
      diagnostic: null,
    };
  }

  function pickPopupOffscreen() {
    const offscreen = chromeApi?.offscreen;
    const available = Boolean(
      offscreen
      && (typeof offscreen.createDocument === "function"
        || typeof offscreen.hasDocument === "function"
        || typeof offscreen.getContexts === "function")
    );
    return {
      available,
      diagnostic: null,
    };
  }

  function pickPopupVersion() {
    const manifest = chromeApi?.runtime?.getManifest?.();
    return {
      value: isObject(manifest) ? asString(manifest.version) || null : null,
      diagnostic: null,
    };
  }

  function getPopupStatus() {
    return {
      session: pickPopupSession(state.session),
      visible_index: pickPopupVisibleIndex(state.visible_index),
      background: pickPopupBackground(state.background),
      cache_stats: pickPopupCacheStats(state.cache_stats),
      last_diagnostic: normalizeDiagnostic(state.last_diagnostic),
      runtime: pickPopupRuntime(),
      offscreen: pickPopupOffscreen(),
      version: pickPopupVersion(),
      updated_at: state.updated_at,
    };
  }

  function getPopupStatusSafe() {
    return redactForTelemetry(getPopupStatus());
  }

  async function handleMessage(message) {
    if (!message || typeof message !== "object") {
      return { ok: false };
    }

    if (message.type === "metadata-checker-bridge-status") {
      state.last_bridge_status = message.payload || null;
      state.last_diagnostic =
        normalizeDiagnostic(message.payload?.diagnostics?.[0]) || state.last_diagnostic;
      state.updated_at = clock();
      return { ok: true };
    }

    if (message.type === "metadata-checker-bootstrap-token") {
      return bootstrapAndIndex(message.payload || {});
    }

    if (message.type === "metadata-checker-refresh-visible-manifest") {
      return refreshVisibleManifest(message.payload || {});
    }

    if (message.type === "metadata-checker-ensure-offscreen-runtime") {
      if (!analysisClient || typeof analysisClient.initRuntime !== "function") {
        return {
          ok: false,
          diagnostics: [
            stableDiagnostic(
              "METADATA_CHECKER_LOCAL_GRAPH_RUNTIME_UNAVAILABLE",
              "local graph runtime client is unavailable",
              "error",
            ),
          ],
        };
      }
      await analysisClient.initRuntime({
        project_ref: message.payload?.project_name ?? null,
      });
      return { ok: true };
    }

    if (message.type === "metadata-checker-analyze-local-graph") {
      const payload = buildLocalGraphSelectionPayload(message.payload || {});
      if (!payload?.source_path) {
        return {
          ok: false,
          diagnostics: [
            stableDiagnostic(
              "METADATA_CHECKER_LOCAL_GRAPH_SELECTION_INVALID",
              "local graph selection is missing source_path",
              "error",
            ),
          ],
        };
      }

      const artifactKey = makeAnalysisArtifactKey({
        project_name: payload.project_name,
        source_path: payload.source_path,
        file_id: payload.file_id,
        revision: payload.revision,
        cache_key: payload.cache_key,
        foreground: true,
        active_component_id: payload.active_component_id,
        selected_component_ids: payload.selected_component_ids,
      });

      const existingArtifact = await getExistingLocalGraphArtifact(payload, artifactKey);
      if (!existingArtifact) {
        return {
          ok: false,
          diagnostics: [
            stableDiagnostic(
              "METADATA_CHECKER_LOCAL_GRAPH_ARTIFACT_MISSING",
              "local graph document artifact is not loaded",
              "error",
            ),
          ],
          artifact_key: artifactKey,
          artifact: null,
          background: { ...state.background },
          cache_stats: { ...state.cache_stats },
        };
      }

      const result = await runLocalGraphAnalysis(
        payload,
        message.payload?.options,
        artifactKey,
      );
      if (result?.analysis_status === "runtime_unsupported") {
        return {
          ok: false,
          diagnostics: result.diagnostics ?? [
            stableDiagnostic(
              "METADATA_CHECKER_ANALYZE_LOCAL_GRAPH_UNSUPPORTED",
              "runtime analyzeLocalGraph API is unavailable",
              "error",
            ),
          ],
          artifact_key: artifactKey,
          artifact: result,
          background: { ...state.background },
          cache_stats: { ...state.cache_stats },
        };
      }
      if (result?.analysis_status === "runtime_unavailable") {
        return {
          ok: false,
          diagnostics: result.diagnostics ?? [
            stableDiagnostic(
              "METADATA_CHECKER_LOCAL_GRAPH_RUNTIME_UNAVAILABLE",
              "local graph runtime client is unavailable",
              "error",
            ),
          ],
          artifact_key: artifactKey,
          artifact: result,
          background: { ...state.background },
          cache_stats: { ...state.cache_stats },
        };
      }
      return {
        ok: true,
        artifact_ready: result?.analysis_status === "ready" || result?.analysis_status === "empty",
        artifact_key: artifactKey,
        artifact: result,
        existing_artifact_source: existingArtifact?.key ?? null,
        background: { ...state.background },
        cache_stats: { ...state.cache_stats },
      };
    }

    if (message.type === "metadata-checker-selection-changed") {
      const queuedPayload = buildForegroundSelectionItem(message.payload || {});
      if (!queuedPayload?.source_path) {
        return { ok: false };
      }

      if (state.session?.base_url && state.visible_index.status === "ready") {
        const payloadArtifactKey = makeAnalysisArtifactKey(queuedPayload);
        const cachedPayloadArtifact = await cache.get(payloadArtifactKey);
        if (cachedPayloadArtifact) {
          return {
            ok: true,
            queued: false,
            file: queuedPayload,
            artifact_ready: true,
            artifact_key: payloadArtifactKey,
            artifact: cachedPayloadArtifact ?? null,
            background: { ...state.background },
            cache_stats: { ...state.cache_stats },
          };
        }
      }

      const queued = enqueueForegroundSelection(message.payload || {});
      if (!queued.queued || !queued.file?.source_path) {
        return { ok: false, ...queued };
      }
      const artifactKey = makeAnalysisArtifactKey(queued.file);
      if (!state.session?.base_url || state.visible_index.status !== "ready") {
        const cachedArtifact = await cache.get(artifactKey);
        return {
          ok: true,
          ...queued,
          artifact_ready: Boolean(cachedArtifact),
          artifact_key: artifactKey,
          artifact: cachedArtifact ?? null,
          indexing_status: queued.indexing_status ?? state.background.indexing_status,
          retry_available: queued.retry_available ?? state.background.retry_available,
          background: { ...state.background },
          cache_stats: { ...state.cache_stats },
        };
      }
      const processed = await processBackgroundQueue({
        limit: normalizeQueueLimit(message.payload?.limit, 1),
        max_concurrency: normalizeMaxConcurrency(message.payload?.max_concurrency, state.background.max_concurrency),
        min_interval_ms: normalizeMinIntervalMs(message.payload?.min_interval_ms, state.background.min_interval_ms),
      });
      const artifactEntry = processed.artifacts.find((entry) =>
        entry?.artifact_key === artifactKey,
      );
      const artifact = artifactEntry?.cached || (await cache.get(artifactKey));
      return {
        ok: true,
        ...queued,
        artifact_ready: Boolean(artifact),
        artifact_key: artifactKey,
        artifact: artifact ?? null,
        background: processed.background,
        cache_stats: { ...state.cache_stats },
      };
    }

    if (message.type === "metadata-checker-background-process") {
      return processBackgroundQueue(message.payload || {});
    }

    if (message.type === "metadata-checker-background-pause") {
      state.background.paused = true;
      state.background.status = "paused";
      state.background.indexing_status = "paused";
      return { ok: true, background: { ...state.background } };
    }

    if (message.type === "metadata-checker-background-resume") {
      state.background.paused = false;
      state.background.status = state.background.queue.length > 0 ? "queued" : "idle";
      state.background.indexing_status = state.background.status;
      return { ok: true, background: { ...state.background } };
    }

    if (message.type === "metadata-checker-popup-status") {
      return {
        ok: true,
        state: getPopupStatusSafe(),
      };
    }

    return {
      ok: false,
      diagnostics: [
        stableDiagnostic(
          "METADATA_CHECKER_EXTENSION_UNSUPPORTED_MESSAGE",
          `unsupported extension message: ${String(message.type)}`,
        ),
      ],
    };
  }

  return {
    state,
    getState,
    bootstrapWithAccessToken,
    listVisibleMetadata,
    refreshVisibleManifest,
    seedBackgroundQueue,
    enqueueForegroundSelection,
    processBackgroundQueue,
    bootstrapAndIndex,
    handleMessage,
  };
}

const controller = createM45BackgroundController();

if (typeof chrome !== "undefined" && chrome.runtime?.onMessage?.addListener) {
  chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
    if (message?.type === OFFSCREEN_WASM_CALL_MESSAGE || message?.type === OFFSCREEN_LOCAL_GRAPH_MESSAGE) {
      return false;
    }
    controller.handleMessage(message).then(
      sendResponse,
      (error) => sendResponse(createBackgroundMessageFailureResponse(error)),
    );
    return true;
  });
}

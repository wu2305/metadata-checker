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

function isObject(value) {
  return value !== null && typeof value === "object";
}

function asArray(value) {
  return Array.isArray(value) ? value : [];
}

function asString(value) {
  return typeof value === "string" ? value : "";
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
  const importImpl = options.importImpl ?? ((specifier) => import(specifier));
  const wasmModulePath = options.wasmModulePath ?? "metadata_checker.js";
  const wasmFilePath = options.wasmFilePath ?? "metadata_checker_bg.wasm";
  let runtimePromise = null;

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
      const module = await importImpl(extensionUrl(wasmModulePath));
      const init = module.default ?? module.init;
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

  return {
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
  };
}

function createDefaultAnalysisClient() {
  if (globalThis.chrome?.runtime?.getURL) {
    return createWasmAnalysisClient();
  }
  return null;
}

export function createM45BackgroundController(options = {}) {
  const fetchImpl = options.fetchImpl ?? globalThis.fetch?.bind(globalThis);
  const clock = options.clock ?? (() => Date.now());
  const cache = options.cache ?? createDefaultMetadataCache();
  const analysisClient = options.analysisClient === undefined
    ? createDefaultAnalysisClient()
    : options.analysisClient;
  const state = {
    last_bridge_status: null,
    last_diagnostic: null,
    updated_at: null,
    session: null,
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
      processed: 0,
      total: 0,
      active: 0,
      paused: false,
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

  async function listVisibleMetadata({ base_url, baseUrl } = {}) {
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
    state.visible_index = {
      status: "ready",
      projects,
      files: visibleFiles,
      analyzable_count: analyzable.length,
      indexed_at: clock(),
    };
    await cache.set(`visible-index|${base}`, state.visible_index);
    seedBackgroundQueue(analyzable, { base_url: base });
    emit(M45_EVENT_TYPES.VISIBLE_METADATA_INDEXED, {
      project_count: projects.length,
      file_count: visibleFiles.length,
      analyzable_count: analyzable.length,
    });
    return state.visible_index;
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
    return {
      project_name: projectName,
      source_path: sourcePath,
      file_id: file.id ?? file.fileId ?? file.file_id ?? null,
      revision: file.revision ?? file.modifyTime ?? file.modify_time ?? null,
      extension: ext,
      analyzable: SUPPORTED_METADATA_EXTENSIONS.has(ext),
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
    const queue = files
      .map((file) => ({
        ...file,
        base_url: base,
        priority: priorityForFile(file, context),
        cache_key: makeCacheKey(base, file),
      }))
      .sort((a, b) => a.priority - b.priority || a.source_path.localeCompare(b.source_path));
    state.background.queue = queue;
    state.background.total = queue.length;
    state.background.processed = 0;
    state.background.status = queue.length > 0 ? "queued" : "idle";
    return queue;
  }

  function enqueueForegroundSelection(selection = {}) {
    const sourcePath = selection.source_path;
    if (!sourcePath) {
      return { queued: false };
    }
    const indexedFile = state.visible_index.files.find((file) => file.source_path === sourcePath) ?? {};
    const projectName = selection.project_name
      ?? indexedFile.project_name
      ?? state.visible_index.projects[0]?.project_name
      ?? "";
    const file = {
      project_name: projectName,
      source_path: sourcePath,
      file_id: selection.file_id ?? indexedFile.file_id ?? null,
      revision: selection.revision ?? indexedFile.revision ?? null,
      extension: indexedFile.extension ?? sourcePath.split(".").pop()?.toLowerCase() ?? "",
      analyzable: true,
      base_url: state.session?.base_url,
      priority: 0,
      foreground: true,
      active_component_id: selection.active_component_id ?? selection.activeComponentId ?? null,
      selected_component_ids: asArray(selection.selected_component_ids ?? selection.selectedComponentIds),
      cache_key: makeCacheKey(state.session?.base_url, {
        project_name: projectName,
        source_path: sourcePath,
        file_id: selection.file_id ?? indexedFile.file_id ?? null,
        revision: selection.revision ?? indexedFile.revision ?? "",
      }),
    };
    state.background.queue = [
      file,
      ...state.background.queue.filter((item) => item.source_path !== sourcePath),
    ];
    state.background.total = Math.max(state.background.total, state.background.queue.length);
    state.background.status = "queued";
    emit(M45_EVENT_TYPES.BACKGROUND_ANALYSIS_PROGRESS, {
      status: "foreground_queued",
      source_path: sourcePath,
    });
    return { queued: true, file };
  }

  async function processBackgroundQueue({ limit = 3 } = {}) {
    const processed = [];
    state.background.status = "running";
    while (!state.background.paused && state.background.queue.length > 0 && processed.length < limit) {
      const item = state.background.queue.shift();
      state.background.active = 1;
      const artifactKey = makeAnalysisArtifactKey(item);
      const cached = await cache.get(artifactKey);
      if (cached) {
        state.cache_stats.hits += 1;
      } else {
        state.cache_stats.misses += 1;
        try {
          const rawContent = await request(item.base_url, metadataContentPath(item));
          const rawText = typeof rawContent === "string"
            ? rawContent
            : rawContent?.raw_text ?? rawContent?.rawText ?? rawContent?.content ?? "";
          await tryCacheSet(`raw-metadata|${item.cache_key}`, {
            kind: "raw-metadata",
            source_path: item.source_path,
            project_name: item.project_name,
            file_id: item.file_id,
            revision: item.revision,
            raw_text: rawText,
          });
          const artifact = await runBackgroundAnalysis(item, rawText);
          await tryCacheSet(artifactKey, artifact);
        } catch (error) {
          const diagnostic = stableDiagnostic(
            "BACKGROUND_ANALYSIS_FAILED",
            error?.message || "background analysis failed",
            "error",
          );
          state.last_diagnostic = diagnostic;
          await tryCacheSet(artifactKey, {
            kind: "metadata-analysis-artifact",
            source_path: item.source_path,
            project_name: item.project_name,
            file_id: item.file_id,
            revision: item.revision,
            analysis_status: "error",
            diagnostics: [diagnostic],
          });
        }
      }
      state.background.processed += 1;
      processed.push(item);
      emit(M45_EVENT_TYPES.METADATA_PREFETCHED, {
        source_path: item.source_path,
        cache_hit: Boolean(cached),
      });
      emit(M45_EVENT_TYPES.BACKGROUND_ANALYSIS_PROGRESS, {
        processed: state.background.processed,
        total: state.background.total,
        source_path: item.source_path,
      });
    }
    state.background.active = 0;
    state.background.status = state.background.queue.length === 0 ? "completed" : "queued";
    if (state.background.status === "completed") {
      emit(M45_EVENT_TYPES.BACKGROUND_ANALYSIS_COMPLETED, {
        processed: state.background.processed,
        total: state.background.total,
      });
    }
    return { processed, background: { ...state.background } };
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
            project_name: item.project_name,
            active_component_id: item.foreground ? item.active_component_id ?? null : null,
            selected_component_ids: item.foreground ? item.selected_component_ids ?? [] : [],
          },
          { mode: item.foreground ? "foreground" : "background" },
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
    const queueResult = await processBackgroundQueue({ limit: payload.initial_limit ?? 3 });
    return {
      ok: true,
      session: bootstrapped.session,
      visible_index: visibleIndex,
      background: queueResult.background,
    };
  }

  function getState() {
    return redactForTelemetry(state);
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

    if (message.type === "metadata-checker-selection-changed") {
      const queued = enqueueForegroundSelection(message.payload || {});
      if (!queued.queued || !state.session?.base_url) {
        return queued;
      }
      const processed = await processBackgroundQueue({ limit: 1 });
      return { ok: true, ...queued, background: processed.background };
    }

    if (message.type === "metadata-checker-background-process") {
      return processBackgroundQueue(message.payload || {});
    }

    if (message.type === "metadata-checker-background-pause") {
      state.background.paused = true;
      state.background.status = "paused";
      return { ok: true, background: { ...state.background } };
    }

    if (message.type === "metadata-checker-background-resume") {
      state.background.paused = false;
      state.background.status = state.background.queue.length > 0 ? "queued" : "idle";
      return { ok: true, background: { ...state.background } };
    }

    if (message.type === "metadata-checker-popup-status") {
      return {
        ok: true,
        state: getState(),
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
    controller.handleMessage(message).then(sendResponse);
    return true;
  });
}

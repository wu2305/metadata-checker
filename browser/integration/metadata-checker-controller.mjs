/**
 * M40.9：Integration Controller
 *
 * 串联 plugin + provider + runtimeClient + renderer，负责完整链路编排。
 * Controller 只消费标准 contract，不 import BI 特化对象。
 * 不把 .spg raw text 塞进 selection payload。
 */

const CONTROLLER_STATE = {
  IDLE: "idle",
  INITIALIZING: "initializing",
  READY: "ready",
  ANALYZING: "analyzing",
  ERROR: "error",
};

function _makeErrorEnvelope(code, message, target = null) {
  return {
    status: "error",
    target,
    items: [],
    diagnostics: [
      {
        severity: "error",
        code,
        message,
      },
    ],
  };
}

function _isLogicalPath(sourcePath) {
  if (typeof sourcePath !== "string") return false;
  if (sourcePath.startsWith("/")) return false;
  if (
    sourcePath.startsWith("http://") ||
    sourcePath.startsWith("https://") ||
    sourcePath.startsWith("file://")
  ) {
    return false;
  }
  if (sourcePath.includes("..")) return false;
  if (/^[A-Za-z]:[\\\/]/.test(sourcePath)) return false;
  return true;
}

function _emitHost(host, eventName, payload) {
  if (host && typeof host.emit === "function") {
    host.emit(eventName, payload);
  }
}

function _errorMessageFromProviderResult(value, fallback) {
  if (!value || typeof value !== "object") {
    return fallback;
  }
  return (
    value.message ??
    value.diagnostics?.[0]?.message ??
    value.error?.message ??
    fallback
  );
}

function _sortDeep(value) {
  if (value === null || typeof value !== "object") {
    return value;
  }

  if (Array.isArray(value)) {
    return value.map(_sortDeep);
  }

  const normalized = {};
  const keys = Object.keys(value).sort();
  for (const key of keys) {
    normalized[key] = _sortDeep(value[key]);
  }
  return normalized;
}

function _cloneResultForCache(value) {
  if (typeof structuredClone === "function") {
    return structuredClone(value);
  }
  return JSON.parse(JSON.stringify(value));
}

function _makeDeferred() {
  let resolve;
  const promise = new Promise((resolveFn, rejectFn) => {
    resolve = resolveFn;
  });
  return { promise, resolve };
}

function _findForbiddenSelectionPayload(value, path = "selection", seen = new WeakSet()) {
  if (!value || typeof value !== "object") {
    return null;
  }
  if (seen.has(value)) {
    return null;
  }
  seen.add(value);

  const forbiddenKeys = new Set([
    "raw_text",
    "rawText",
    "components",
    "component_json",
    "componentJson",
    "raw_component",
    "rawComponent",
    "canvas",
  ]);

  if (Array.isArray(value)) {
    for (let i = 0; i < value.length; i += 1) {
      const nested = _findForbiddenSelectionPayload(value[i], `${path}[${i}]`, seen);
      if (nested) return nested;
    }
    return null;
  }

  for (const [key, item] of Object.entries(value)) {
    const childPath = `${path}.${key}`;
    if (forbiddenKeys.has(key)) {
      return childPath;
    }
    const nested = _findForbiddenSelectionPayload(item, childPath, seen);
    if (nested) return nested;
  }
  return null;
}

export function createMetadataCheckerController(options = {}) {
  const plugin = options.plugin;
  const provider = options.provider;
  const runtimeClient = options.runtimeClient;
  const renderer = options.renderer;
  const graphRenderer = options.graphRenderer ?? renderer;
  const host = options.host;
  const logger = options.logger ?? console;
  const analysisOptions = options.analysisOptions ?? {};
  const runtimeOptions = options.runtimeOptions ?? {};
  const selectionDebounceMs =
    Number.isFinite(Number(options.selectionDebounceMs)) &&
    Number(options.selectionDebounceMs) > 0
      ? Number(options.selectionDebounceMs)
      : 0;
  const analysisCache = options.analysisCache || new Map();
  const remoteLoadMode = options.remoteLoadMode ?? "runtime-first";
  const clock = options.clock ?? (() => Date.now());
  const hasPageProvider = provider && typeof provider.getFileContent === "function";
  const hasRuntimeRemoteLoader =
    runtimeClient && typeof runtimeClient.loadRemoteSuperpageDocument === "function";
  const shouldPreferRuntimeRemote =
    remoteLoadMode === "runtime-first" && hasRuntimeRemoteLoader;

  if (!plugin || typeof plugin.onSelectionChanged !== "function") {
    throw new Error(
      "createMetadataCheckerController: plugin with onSelectionChanged is required",
    );
  }
  if (!hasPageProvider && !shouldPreferRuntimeRemote) {
    throw new Error(
      "createMetadataCheckerController: provider with getFileContent is required",
    );
  }
  if (
    !runtimeClient ||
    (typeof runtimeClient.loadSuperpageDocument !== "function" &&
      !hasRuntimeRemoteLoader)
  ) {
    throw new Error(
      "createMetadataCheckerController: runtimeClient with loadSuperpageDocument or loadRemoteSuperpageDocument is required",
    );
  }
  if (!renderer || typeof renderer.renderAnalysis !== "function") {
    throw new Error(
      "createMetadataCheckerController: renderer with renderAnalysis is required",
    );
  }

  let state = CONTROLLER_STATE.IDLE;
  let initialized = false;
  let _initPromise = null;
  let lastAnalysisResult = null;
  let lastError = null;
  let _previousState = CONTROLLER_STATE.IDLE;
  let _selectionSeq = 0;
  let _latestSelectionSeq = 0;
  let _activeSelectionSeq = 0;
  let _debounceTimer = null;
  const _selectionWaiters = new Map();

  function _makeAnalysisCacheKey(selection) {
    return JSON.stringify(
      _sortDeep({
        source_path: selection?.source_path,
        active_component_id: selection?.active_component_id ?? null,
        selected_component_ids:
          Array.isArray(selection?.selected_component_ids)
            ? selection.selected_component_ids
            : [],
        analysisOptions: _sortDeep(analysisOptions),
      }),
    );
  }

  function _emitStaleSelection(payload) {
    _emitHost(host, "analysis_stale_discarded", {
      ...payload,
      timestamp: clock(),
    });
  }

  function _buildStaleResult(target) {
    return _makeErrorEnvelope(
      "ANALYSIS_STALE",
      "selection is stale and will not be rendered",
      target,
    );
  }

  function _setState(newState) {
    const previousState = _previousState;
    state = newState;
    _previousState = newState;
    _emitHost(host, "controller_state_changed", {
      previous: previousState,
      current: newState,
      timestamp: clock(),
    });
  }

  function _emitRuntimeType() {
    const runtimeType =
      runtimeClient && runtimeClient._kind === "service-worker"
        ? "service-worker"
        : "page-fallback";
    _emitHost(host, "controller_runtime_type", {
      runtimeType,
      timestamp: clock(),
    });
  }

  function _isErrorEnvelope(value) {
    return value && typeof value === "object" && value.status === "error";
  }

  async function _loadDocumentViaPageProvider(fileRef, sourcePath) {
    if (!hasPageProvider) {
      throw new Error("page metadata provider is not available for fallback");
    }

    const contentResult = await provider.getFileContent(fileRef);
    if (_isErrorEnvelope(contentResult)) {
      throw new Error(
        _errorMessageFromProviderResult(contentResult, "metadata fetch failed"),
      );
    }
    const rawText = contentResult?.raw_text ?? "";

    if (typeof provider.getFileInfo === "function") {
      const infoResult = await provider.getFileInfo(fileRef);
      if (_isErrorEnvelope(infoResult)) {
        throw new Error(
          _errorMessageFromProviderResult(infoResult, "metadata info fetch failed"),
        );
      }
    }

    const loadResult = await runtimeClient.loadSuperpageDocument(sourcePath, rawText);
    if (_isErrorEnvelope(loadResult)) {
      throw new Error(
        _errorMessageFromProviderResult(loadResult, "runtime load failed"),
      );
    }
    return loadResult;
  }

  async function _loadDocumentForSelection(fileRef, sourcePath) {
    if (!shouldPreferRuntimeRemote) {
      return _loadDocumentViaPageProvider(fileRef, sourcePath);
    }

    try {
      const remoteResult = await runtimeClient.loadRemoteSuperpageDocument(
        fileRef,
        runtimeOptions,
      );
      if (_isErrorEnvelope(remoteResult)) {
        throw new Error(
          _errorMessageFromProviderResult(remoteResult, "runtime remote metadata load failed"),
        );
      }
      return remoteResult;
    } catch (err) {
      _emitHost(host, "controller_remote_load_fallback", {
        error: _makeErrorEnvelope(
          "REMOTE_RUNTIME_LOAD_FAILED",
          err?.message ?? String(err),
          sourcePath,
        ),
        timestamp: clock(),
      });
      return _loadDocumentViaPageProvider(fileRef, sourcePath);
    }
  }

  function _renderGraph(result) {
    if (graphRenderer && typeof graphRenderer.renderGraph === "function") {
      return graphRenderer.renderGraph(result);
    }
    if (graphRenderer && typeof graphRenderer.render === "function") {
      return graphRenderer.render(result);
    }
    if (
      graphRenderer !== renderer &&
      graphRenderer &&
      typeof graphRenderer.renderError === "function" &&
      result?.status === "error"
    ) {
      return graphRenderer.renderError(result);
    }
    return undefined;
  }

  function _renderDiagnosticGraph(error) {
    return _renderGraph(error);
  }

  async function _handleGraphExpandRequested(payload = {}) {
    const target = payload.target ?? payload.nodeId ?? payload.node?.id ?? null;
    const depth = payload.depth ?? payload.nextDepth ?? null;
    const expandToken = payload.expand_token ?? payload.expandToken ?? null;

    _emitHost(host, "graph_expand_requested", {
      target,
      depth,
      expandToken,
      timestamp: clock(),
    });

    const expandFn =
      (runtimeClient && typeof runtimeClient.analyzeGraphTarget === "function"
        ? runtimeClient.analyzeGraphTarget.bind(runtimeClient)
        : null) ??
      (runtimeClient && typeof runtimeClient.expandVisualGraph === "function"
        ? runtimeClient.expandVisualGraph.bind(runtimeClient)
        : null);

    if (!expandFn) {
      const error = _makeErrorEnvelope(
        "GRAPH_EXPAND_UNSUPPORTED",
        "runtime does not support graph expand target/depth query",
        target,
      );
      _emitHost(host, "graph_expand_failed", {
        error,
        timestamp: clock(),
      });
      _renderDiagnosticGraph(error);
      return error;
    }

    try {
      const result = await expandFn({ target, depth, expand_token: expandToken });
      _emitHost(host, "graph_expand_completed", {
        result,
        timestamp: clock(),
      });
      _renderGraph(result);
      return result;
    } catch (err) {
      const error = _makeErrorEnvelope(
        "GRAPH_EXPAND_FAILED",
        err?.message ?? String(err),
        target,
      );
      _emitHost(host, "graph_expand_failed", {
        error,
        timestamp: clock(),
      });
      _renderDiagnosticGraph(error);
      return error;
    }
  }

  function _wireGraphRendererEvents() {
    if (graphRenderer && typeof graphRenderer.on === "function") {
      graphRenderer.on("expand_requested", _handleGraphExpandRequested);
      return;
    }
    if (graphRenderer && typeof graphRenderer.setEventHandler === "function") {
      graphRenderer.setEventHandler((event) => {
        if (event?.type === "expand_requested") {
          _handleGraphExpandRequested(event).catch((err) => {
            logger.error("[controller] graph expand failed:", err);
          });
        }
      });
    }
  }

  async function _ensureInitialized() {
    if (initialized && state === CONTROLLER_STATE.READY) {
      return { ready: true };
    }
    if (_initPromise) {
      return _initPromise;
    }
    _initPromise = (async () => {
      _setState(CONTROLLER_STATE.INITIALIZING);
      try {
        const activationResult = await plugin.activate({
          runtimeOptions: {},
          analysisOptions,
        });
        if (
          activationResult &&
          typeof activationResult === "object" &&
          activationResult.status === "error"
        ) {
          throw new Error(
            activationResult.diagnostics?.[0]?.message ??
              "plugin activation failed",
          );
        }
        initialized = true;
        _setState(CONTROLLER_STATE.READY);
        _emitRuntimeType();
        lastError = null;
        _emitHost(host, "controller_ready", {
          timestamp: clock(),
        });
        return { ready: true };
      } catch (err) {
        initialized = false;
        _setState(CONTROLLER_STATE.ERROR);
        lastError = _makeErrorEnvelope(
          "CONTROLLER_INIT_FAILED",
          err?.message ?? String(err),
        );
        _emitHost(host, "controller_init_failed", {
          error: lastError,
          timestamp: clock(),
        });
        return { ready: false, error: lastError };
      } finally {
        _initPromise = null;
      }
    })();
    return _initPromise;
  }

  async function _handleSelectionChanged(selection, selectionSeq) {
    const initResult = await _ensureInitialized();
    if (!initResult.ready) {
      if (typeof renderer.renderError === "function") {
        renderer.renderError(
          initResult.error ??
            _makeErrorEnvelope(
              "CONTROLLER_NOT_READY",
              "controller initialization failed",
            ),
        );
      }
      _renderDiagnosticGraph(initResult.error);
      return initResult.error;
    }

    if (!selection || typeof selection !== "object") {
      const error = _makeErrorEnvelope(
        "INVALID_SELECTION",
        "selection is required",
      );
      if (typeof renderer.renderError === "function") {
        renderer.renderError(error);
      }
      _renderDiagnosticGraph(error);
      return error;
    }

    // 防御性检查：selection payload 不得携带 raw metadata 或完整组件 JSON。
    const forbiddenSelectionPath = _findForbiddenSelectionPayload(selection);
    if (forbiddenSelectionPath) {
      const error = _makeErrorEnvelope(
        "INVALID_SELECTION",
        `${forbiddenSelectionPath} is not allowed in selection payload`,
      );
      if (typeof renderer.renderError === "function") {
        renderer.renderError(error);
      }
      _renderDiagnosticGraph(error);
      return error;
    }

    const sourcePath = selection.source_path;
    const fileId = selection.file_id;

    if (!_isLogicalPath(sourcePath)) {
      const error = _makeErrorEnvelope(
        "INVALID_SELECTION",
        `source_path is not a logical path: ${sourcePath}`,
      );
      if (typeof renderer.renderError === "function") {
        renderer.renderError(error);
      }
      _renderDiagnosticGraph(error);
      return error;
    }

    const fileRef = {
      source_path: sourcePath,
      file_id: fileId,
      project_name: selection.project_name ?? null,
    };

    const cacheKey = _makeAnalysisCacheKey(selection);

    if (analysisCache.has(cacheKey)) {
      const cachedResult = _cloneResultForCache(analysisCache.get(cacheKey));

      _setState(CONTROLLER_STATE.ANALYZING);
      _emitHost(host, "analysis_started", {
        selection,
        cacheKey,
        timestamp: clock(),
      });
      _emitHost(host, "analysis_cache_hit", {
        cacheKey,
        source_path: sourcePath,
        active_component_id: selection.active_component_id,
        timestamp: clock(),
      });

      if (selectionSeq !== _selectionSeq) {
        const staleResult = _buildStaleResult(selection.active_component_id ?? sourcePath);
        _emitStaleSelection({
          reason: "cache-hit-stale",
          result: staleResult,
          selection,
        });
        return staleResult;
      }

      lastAnalysisResult = cachedResult;
      lastError = null;
      _setState(CONTROLLER_STATE.READY);
      _emitHost(host, "analysis_completed", {
        result: cachedResult,
        cacheKey,
        timestamp: clock(),
      });

      renderer.renderAnalysis(cachedResult);
      await _renderGraph(cachedResult);
      return cachedResult;
    }

    _setState(CONTROLLER_STATE.ANALYZING);
    _emitHost(host, "analysis_started", {
      selection,
      cacheKey,
      timestamp: clock(),
    });

    _emitHost(host, "analysis_cache_miss", {
      cacheKey,
      source_path: sourcePath,
      active_component_id: selection.active_component_id,
      timestamp: clock(),
    });

    try {
      await _loadDocumentForSelection(fileRef, sourcePath);
      const buildResult = await runtimeClient.buildOrUpdateSuperpageGraph(sourcePath);
      if (_isErrorEnvelope(buildResult)) {
        throw new Error(
          _errorMessageFromProviderResult(buildResult, "runtime build graph failed"),
        );
      }

      const result = await runtimeClient.analyzeSuperpageSelection(
        selection,
        analysisOptions,
      );
      if (_isErrorEnvelope(result)) {
        throw new Error(
          _errorMessageFromProviderResult(result, "runtime analyze failed"),
        );
      }

      if (result?.status === "ready") {
        analysisCache.set(cacheKey, _cloneResultForCache(result));
      }

      if (selectionSeq !== _selectionSeq) {
        _emitStaleSelection({
          reason: "superseded",
          result,
          selection,
        });
        return result;
      }

      lastAnalysisResult = result;
      lastError = null;
      _setState(CONTROLLER_STATE.READY);
      _emitHost(host, "analysis_completed", { result, timestamp: clock() });

      renderer.renderAnalysis(result);
      await _renderGraph(result);
      return result;
    } catch (err) {
      const error = _makeErrorEnvelope(
        "ANALYSIS_PIPELINE_FAILED",
        err?.message ?? String(err),
        selection.active_component_id ?? sourcePath,
      );
      if (selectionSeq !== _selectionSeq) {
        _emitStaleSelection({
          reason: "superseded",
          error,
          selection,
        });
        return error;
      }
      lastError = error;
      _setState(CONTROLLER_STATE.ERROR);
      _emitHost(host, "analysis_failed", { error, timestamp: clock() });

      if (typeof renderer.renderError === "function") {
        renderer.renderError(error);
      }
      await _renderDiagnosticGraph(error);
      return error;
    }
  }

  function _onPluginEvent(eventName, payload) {
    if (eventName === "selection_changed") {
      const selection = payload?.selection;
      if (selection) {
        controller.handleSelection(selection).catch((err) => {
          logger.error("[controller] _handleSelectionChanged failed:", err);
        });
      }
    }
  }

  function _dispatchSelection(selection) {
    const selectionSeq = ++_selectionSeq;
    const deferred = _makeDeferred();

    _selectionWaiters.set(selectionSeq, {
      selection,
      deferred,
    });

    _latestSelectionSeq = selectionSeq;

    const runLatest = async () => {
      const latestSeq = _latestSelectionSeq;
      const entry = _selectionWaiters.get(latestSeq);
      if (!entry) {
        return;
      }

      if (selectionDebounceMs > 0) {
        // 标记掉所有未执行完成且已被后续 selection supersede 的请求。
        for (const [seq, waiter] of _selectionWaiters.entries()) {
          if (seq === latestSeq || seq === _activeSelectionSeq) {
            continue;
          }
          if (seq < latestSeq) {
            const staleResult = _buildStaleResult(
              waiter.selection?.active_component_id ?? waiter.selection?.source_path,
            );
            waiter.deferred.resolve(staleResult);
            _emitStaleSelection({
              reason: "debounced",
              result: staleResult,
              selection: waiter.selection,
            });
            _selectionWaiters.delete(seq);
          }
        }
      }

      const active = _selectionWaiters.get(latestSeq);
      if (!active) {
        return;
      }

      const currentSeq = latestSeq;
      _activeSelectionSeq = currentSeq;
      try {
        const result = await _handleSelectionChanged(active.selection, currentSeq);
        active.deferred.resolve(result);
      } catch (err) {
        active.deferred.resolve(
          _makeErrorEnvelope(
            "ANALYSIS_FAILED",
            err?.message ?? String(err),
            active.selection?.active_component_id ?? active.selection?.source_path,
          ),
        );
      } finally {
        _selectionWaiters.delete(currentSeq);
        if (_activeSelectionSeq === currentSeq) {
          _activeSelectionSeq = 0;
        }
      }
    };

    if (selectionDebounceMs > 0) {
      if (_debounceTimer) {
        clearTimeout(_debounceTimer);
      }
      _debounceTimer = setTimeout(() => {
        _debounceTimer = null;
        runLatest().catch((err) => {
          logger.error("[controller] selection dispatch failed:", err);
        });
      }, selectionDebounceMs);
    } else {
      runLatest();
    }

    return deferred.promise;
  }

  // 监听 host 事件，若 plugin 也 emit 到同一 host
  if (host && typeof host.emit === "function") {
    const originalEmit = host.emit.bind(host);
    host.emit = function (eventName, payload) {
      originalEmit(eventName, payload);
      _onPluginEvent(eventName, payload);
    };
  }

  _wireGraphRendererEvents();

  const controller = {
    async init() {
      return _ensureInitialized();
    },

    async handleSelection(selection) {
      return _dispatchSelection(selection);
    },

    status() {
      return {
        state,
        initialized,
        lastAnalysisResult,
        lastError,
        hasProvider: !!provider,
        hasRuntimeClient: !!runtimeClient,
        hasRenderer: !!renderer,
      };
    },

    dispose() {
      if (plugin && typeof plugin.deactivate === "function") {
        plugin.deactivate();
      }
      initialized = false;
      _initPromise = null;
      _setState(CONTROLLER_STATE.IDLE);
      _previousState = CONTROLLER_STATE.IDLE;
    },
  };

  return controller;
}

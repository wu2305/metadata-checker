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

function _writeDomMarker(key, value) {
  if (typeof document === "undefined") return;
  const existing = document.querySelector(
    `[data-metadata-checker-marker="${key}"]`,
  );
  if (existing) {
    existing.setAttribute("data-metadata-checker-marker-value", value);
    return;
  }
  const el = document.createElement("span");
  el.setAttribute("data-metadata-checker-marker", key);
  el.setAttribute("data-metadata-checker-marker-value", value);
  el.style.display = "none";
  document.body.appendChild(el);
}

export function createMetadataCheckerController(options = {}) {
  const plugin = options.plugin;
  const provider = options.provider;
  const runtimeClient = options.runtimeClient;
  const renderer = options.renderer;
  const host = options.host;
  const logger = options.logger ?? console;
  const analysisOptions = options.analysisOptions ?? {};
  const clock = options.clock ?? (() => Date.now());

  if (!plugin || typeof plugin.onSelectionChanged !== "function") {
    throw new Error(
      "createMetadataCheckerController: plugin with onSelectionChanged is required",
    );
  }
  if (!provider || typeof provider.getFileContent !== "function") {
    throw new Error(
      "createMetadataCheckerController: provider with getFileContent is required",
    );
  }
  if (
    !runtimeClient ||
    typeof runtimeClient.loadSuperpageDocument !== "function"
  ) {
    throw new Error(
      "createMetadataCheckerController: runtimeClient with loadSuperpageDocument is required",
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

  function _setState(newState) {
    state = newState;
    _writeDomMarker("analysis-status", state);
  }

  function _setRuntimeTypeMarker() {
    const isSw =
      runtimeClient &&
      typeof runtimeClient.runtimeStatus === "function" &&
      runtimeClient._kind === "service-worker";
    _writeDomMarker("runtime", isSw ? "service-worker" : "page-fallback");
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
        _setRuntimeTypeMarker();
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

  async function _handleSelectionChanged(selection) {
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
      return error;
    }

    // 防御性检查：selection payload 不得包含 raw_text
    if ("raw_text" in selection) {
      const error = _makeErrorEnvelope(
        "INVALID_SELECTION",
        "selection.raw_text is not allowed in payload",
      );
      if (typeof renderer.renderError === "function") {
        renderer.renderError(error);
      }
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
      return error;
    }

    const fileRef = { source_path: sourcePath, file_id: fileId };

    _setState(CONTROLLER_STATE.ANALYZING);
    _emitHost(host, "analysis_started", { selection, timestamp: clock() });

    try {
      // 获取文件内容，但不塞进 selection payload
      const contentResult = await provider.getFileContent(fileRef);
      if (
        contentResult &&
        typeof contentResult === "object" &&
        contentResult.status === "error"
      ) {
        throw new Error(contentResult.message ?? "metadata fetch failed");
      }
      const rawText = contentResult?.raw_text ?? "";

      const infoResult = await provider.getFileInfo(fileRef);
      if (
        infoResult &&
        typeof infoResult === "object" &&
        infoResult.status === "error"
      ) {
        throw new Error(infoResult.message ?? "metadata info fetch failed");
      }

      await runtimeClient.loadSuperpageDocument(sourcePath, rawText);
      await runtimeClient.buildOrUpdateSuperpageGraph(sourcePath);

      const result = await runtimeClient.analyzeSuperpageSelection(
        selection,
        analysisOptions,
      );

      lastAnalysisResult = result;
      lastError = null;
      _setState(CONTROLLER_STATE.READY);
      _emitHost(host, "analysis_completed", { result, timestamp: clock() });

      renderer.renderAnalysis(result);
      return result;
    } catch (err) {
      const error = _makeErrorEnvelope(
        "ANALYSIS_PIPELINE_FAILED",
        err?.message ?? String(err),
        selection.active_component_id ?? sourcePath,
      );
      lastError = error;
      _setState(CONTROLLER_STATE.ERROR);
      _emitHost(host, "analysis_failed", { error, timestamp: clock() });

      if (typeof renderer.renderError === "function") {
        renderer.renderError(error);
      }
      return error;
    }
  }

  function _onPluginEvent(eventName, payload) {
    if (eventName === "selection_changed") {
      const selection = payload?.selection;
      if (selection) {
        _handleSelectionChanged(selection).catch((err) => {
          logger.error("[controller] _handleSelectionChanged failed:", err);
        });
      }
    }
  }

  // 监听 host 事件，若 plugin 也 emit 到同一 host
  if (host && typeof host.emit === "function") {
    const originalEmit = host.emit.bind(host);
    host.emit = function (eventName, payload) {
      originalEmit(eventName, payload);
      _onPluginEvent(eventName, payload);
    };
  }

  const controller = {
    async init() {
      return _ensureInitialized();
    },

    async handleSelection(selection) {
      return _handleSelectionChanged(selection);
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
    },
  };

  return controller;
}

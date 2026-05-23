/**
 * Metadata Checker Plugin Core
 *
 * M40.3：定义统一 JS 插件协议，插件核心不感知低代码平台、Service Worker、Web Worker、浏览器扩展或 DOM 细节。
 */

const REQUIRED_RUNTIME_METHODS = [
  "initRuntime",
  "runtimeStatus",
  "loadSuperpageDocument",
  "buildOrUpdateSuperpageGraph",
  "analyzeSuperpageSelection",
];

function _makeErrorEnvelope(code, message, selection) {
  const target =
    selection?.active_component_id ?? selection?.source_path ?? null;
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
  if (typeof sourcePath !== "string") {
    return false;
  }
  if (sourcePath.startsWith("/")) {
    return false;
  }
  if (
    sourcePath.startsWith("http://") ||
    sourcePath.startsWith("https://") ||
    sourcePath.startsWith("file://")
  ) {
    return false;
  }
  if (sourcePath.includes("..")) {
    return false;
  }
  if (/^[A-Za-z]:[\\\/]/.test(sourcePath)) {
    return false;
  }
  return true;
}

function _validateSelection(selection) {
  if (!selection || typeof selection !== "object") {
    return { valid: false, code: "INVALID_SELECTION", message: "selection must be an object" };
  }

  if (typeof selection.source_path !== "string") {
    return { valid: false, code: "INVALID_SELECTION", message: "selection.source_path is required and must be a string" };
  }

  if (!_isLogicalPath(selection.source_path)) {
    return { valid: false, code: "INVALID_SELECTION", message: `selection.source_path must be a logical path, got: ${selection.source_path}` };
  }

  if (typeof selection.file_id !== "string") {
    return { valid: false, code: "INVALID_SELECTION", message: "selection.file_id is required and must be a string" };
  }

  if (!Array.isArray(selection.selected_component_ids)) {
    return { valid: false, code: "INVALID_SELECTION", message: "selection.selected_component_ids is required and must be an array" };
  }
  for (const id of selection.selected_component_ids) {
    if (typeof id !== "string") {
      return { valid: false, code: "INVALID_SELECTION", message: "selection.selected_component_ids must contain only strings" };
    }
  }

  if (selection.active_component_id !== null && typeof selection.active_component_id !== "string") {
    return { valid: false, code: "INVALID_SELECTION", message: "selection.active_component_id is required and must be a string or null" };
  }
  if (!("active_component_id" in selection)) {
    return { valid: false, code: "INVALID_SELECTION", message: "selection.active_component_id is required" };
  }

  return { valid: true };
}

function _deepClone(obj) {
  if (obj === null || obj === undefined) {
    return obj;
  }
  return JSON.parse(JSON.stringify(obj));
}

function _mergeOptions(...sources) {
  const result = {};
  for (const source of sources) {
    if (source && typeof source === "object") {
      for (const key of Object.keys(source)) {
        result[key] = source[key];
      }
    }
  }
  return result;
}

export function createMetadataCheckerPlugin({
  runtimeClient,
  host,
  logger,
  clock,
  defaultAnalysisOptions,
}) {
  if (!host || typeof host !== "object" || typeof host.emit !== "function") {
    throw new Error(
      "createMetadataCheckerPlugin: host is required and must have an emit function"
    );
  }

  if (!runtimeClient || typeof runtimeClient !== "object") {
    throw new Error(
      "createMetadataCheckerPlugin: runtimeClient is required and must be an object"
    );
  }

  for (const method of REQUIRED_RUNTIME_METHODS) {
    if (typeof runtimeClient[method] !== "function") {
      throw new Error(
        `createMetadataCheckerPlugin: runtimeClient must implement ${method}`
      );
    }
  }

  const log = logger ?? console;
  const now = clock ?? (() => Date.now());
  const defaultOpts = defaultAnalysisOptions ?? {};

  let state = "inactive";
  let activated = false;
  let lastSelection = null;
  let lastResult = null;
  let activationResult = null;
  let lastError = null;
  let runtimeStatus = null;
  let _initPromise = null;
  let _contextAnalysisOptions = {};

  function _setState(newState) {
    state = newState;
  }

  function _resetState() {
    state = "inactive";
    activated = false;
    lastSelection = null;
    lastResult = null;
    activationResult = null;
    lastError = null;
    runtimeStatus = null;
    _initPromise = null;
    _contextAnalysisOptions = {};
  }

  const plugin = {
    async activate(context) {
      const ctx = context ?? {};
      const runtimeOptions = ctx.runtimeOptions ?? {};

      if (state === "ready") {
        return _deepClone(activationResult ?? runtimeStatus);
      }

      if (state === "activating" && _initPromise) {
        return _initPromise;
      }

      _contextAnalysisOptions = ctx.analysisOptions ?? {};

      _setState("activating");
      _initPromise = (async () => {
        try {
          const initResult = await Promise.resolve(
            runtimeClient.initRuntime(runtimeOptions)
          );
          runtimeStatus = await Promise.resolve(
            runtimeClient.runtimeStatus()
          );
          _setState("ready");
          activated = true;
          activationResult = initResult;
          lastError = null;
          host.emit("plugin_activated", { timestamp: now() });
          host.emit("runtime_ready", { runtimeStatus });
          return initResult;
        } catch (err) {
          const envelope = _makeErrorEnvelope(
            "RUNTIME_CLIENT_ERROR",
            err?.message ?? String(err),
            lastSelection
          );
          _setState("error");
          lastError = envelope;
          host.emit("runtime_error", { error: envelope, timestamp: now() });
          return envelope;
        }
      })();

      return _initPromise;
    },

    deactivate() {
      _resetState();
      host.emit("plugin_deactivated", { timestamp: now() });
      return { deactivated: true };
    },

    status() {
      return _deepClone({
        state,
        activated,
        lastSelection,
        lastResult,
        lastError,
        runtimeStatus,
      });
    },

    onSelectionChanged(selection) {
      const validation = _validateSelection(selection);
      if (!validation.valid) {
        return _makeErrorEnvelope(validation.code, validation.message, selection);
      }

      const clonedSelection = _deepClone(selection);
      lastSelection = clonedSelection;
      host.emit("selection_changed", {
        selection: _deepClone(clonedSelection),
        timestamp: now(),
      });
      return { handled: true };
    },

    async analyze(selection, options) {
      const targetSelection = selection ?? lastSelection;

      if (!targetSelection) {
        return _makeErrorEnvelope(
          "INVALID_SELECTION",
          "No selection provided and no lastSelection available",
          null
        );
      }

      const validation = _validateSelection(targetSelection);
      if (!validation.valid) {
        return _makeErrorEnvelope(validation.code, validation.message, targetSelection);
      }

      const validatedSelection = _deepClone(targetSelection);

      if (state !== "ready") {
        return _makeErrorEnvelope(
          "PLUGIN_NOT_ACTIVATED",
          "Plugin is not activated. Call activate() first.",
          targetSelection
        );
      }

      const mergedOptions = _mergeOptions(
        defaultOpts,
        _contextAnalysisOptions,
        options
      );

      _setState("analyzing");
      host.emit("analysis_started", {
        selection: _deepClone(validatedSelection),
        timestamp: now(),
      });

      try {
        const result = await Promise.resolve(
          runtimeClient.analyzeSuperpageSelection(validatedSelection, mergedOptions)
        );
        lastResult = result;
        lastError = null;
        _setState("ready");
        host.emit("analysis_completed", {
          result: _deepClone(result),
          timestamp: now(),
        });
        if (typeof host.renderAnalysis === "function") {
          host.renderAnalysis(result);
        }
        return result;
      } catch (err) {
        const envelope = _makeErrorEnvelope(
          "RUNTIME_CLIENT_ERROR",
          err?.message ?? String(err),
          targetSelection
        );
        lastError = envelope;
        _setState("error");
        host.emit("analysis_failed", {
          error: envelope,
          timestamp: now(),
        });
        if (typeof host.renderError === "function") {
          host.renderError(envelope);
        }
        return envelope;
      }
    },
  };

  return plugin;
}

/**
 * Fake Runtime Client for M40.3 Plugin Core testing
 *
 * 模拟 runtime client contract 的 5 个方法，支持同步/异步返回。
 */

export function createFakeRuntimeClient(options = {}) {
  const returnType = options.returnType ?? "promise";
  const analyzeDelayMs = options.analyzeDelayMs ?? 0;
  const analyzeShouldReject = options.analyzeShouldReject ?? false;
  const analyzeShouldThrow = options.analyzeShouldThrow ?? false;

  const callLog = [];

  function recordCall(method, args) {
    callLog.push({ method, args: Array.from(args) });
  }

  function wrap(value) {
    if (returnType === "sync") {
      return value;
    }
    return Promise.resolve(value);
  }

  function wrapReject(reason) {
    if (returnType === "sync") {
      throw reason;
    }
    return Promise.reject(reason);
  }

  const client = {
    callLog,

    initRuntime(options) {
      recordCall("initRuntime", arguments);
      if (options?.shouldFail) {
        return wrapReject(new Error("initRuntime failed"));
      }
      return wrap({
        status: "ready",
        target: null,
        items: [],
        diagnostics: [],
      });
    },

    runtimeStatus() {
      recordCall("runtimeStatus", arguments);
      return wrap({
        status: "ready",
        target: null,
        items: [
          {
            kind: "runtime_status",
            label: "Runtime Status",
            detail: {
              initialized: true,
              document_count: 0,
              graph_count: 0,
            },
          },
        ],
        diagnostics: [],
      });
    },

    loadSuperpageDocument(sourcePath, rawText) {
      recordCall("loadSuperpageDocument", arguments);
      return wrap({
        status: "ready",
        target: sourcePath,
        items: [
          {
            kind: "document_loaded",
            label: "Document Loaded",
            detail: { source_path: sourcePath, component_count: 1 },
          },
        ],
        diagnostics: [],
      });
    },

    buildOrUpdateSuperpageGraph(sourcePath) {
      recordCall("buildOrUpdateSuperpageGraph", arguments);
      return wrap({
        status: "ready",
        target: sourcePath,
        items: [
          {
            kind: "graph_built",
            label: "Graph Built",
            detail: { source_path: sourcePath, node_count: 2, edge_count: 1 },
          },
        ],
        diagnostics: [],
      });
    },

    analyzeSuperpageSelection(selection, options) {
      recordCall("analyzeSuperpageSelection", arguments);

      if (analyzeShouldThrow) {
        throw new Error("analyzeSuperpageSelection threw");
      }

      if (analyzeShouldReject) {
        return wrapReject(new Error("analyzeSuperpageSelection rejected"));
      }

      const result = {
        status: "ready",
        target: selection.active_component_id ?? selection.source_path,
        items: [
          {
            kind: "component",
            label: `Component: ${selection.active_component_id ?? "page"}`,
            detail: { id: selection.active_component_id, source_path: selection.source_path },
          },
        ],
        diagnostics: [],
      };

      if (analyzeDelayMs > 0 && returnType !== "sync") {
        return new Promise((resolve) => {
          setTimeout(() => resolve(result), analyzeDelayMs);
        });
      }

      return wrap(result);
    },
  };

  return client;
}

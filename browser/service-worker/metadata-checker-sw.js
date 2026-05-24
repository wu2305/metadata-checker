/**
 * M40.9：Service Worker Runtime Contract
 *
 * 提供最小 message protocol，先 mock runtime 行为，不接真实 WASM。
 * 支持 lazy init、重复请求复用初始化 Promise、失败后重试或 fallback。
 * message response 携带 request_id，错误返回稳定 code。
 */

const SW_VERSION = "0.1.0-m40.9";

// runtime 状态
let _initPromise = null;
let _initFailed = false;
let _initError = null;
let _documentCache = new Map(); // sourcePath -> { rawText, loadedAt }
let _graphCache = new Map(); // sourcePath -> { builtAt }

function makeResponse(requestId, result) {
  return { id: requestId, ok: true, result, error: null };
}

function makeErrorResponse(requestId, code, message) {
  return {
    id: requestId,
    ok: false,
    result: null,
    error: { code, message },
  };
}

function _mockInitRuntime(options = {}) {
  if (options?.shouldFail) {
    throw new Error("mock initRuntime failed");
  }
  return {
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
  // 5MB 限制检查：rawText 过大时拒绝通过 selection payload 传输，但这里是从 message 接收
  const MAX_RAW_TEXT_LENGTH = 5 * 1024 * 1024;
  if (rawText && rawText.length > MAX_RAW_TEXT_LENGTH) {
    throw new Error("raw_text exceeds 5MB limit");
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

  // 验证 selection 不包含 raw_text
  if (selection.raw_text !== undefined) {
    throw new Error("selection payload must not contain raw_text");
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
      // 这里未来替换为真实 WASM lazy init
      const result = _mockInitRuntime(options);
      _initFailed = false;
      _initError = null;
      return result;
    } catch (err) {
      _initFailed = true;
      _initError = err;
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
      const result = _mockRuntimeStatus();
      return makeResponse(id, result);
    }

    if (method === "loadSuperpageDocument") {
      const result = _mockLoadSuperpageDocument(args[0], args[1]);
      return makeResponse(id, result);
    }

    if (method === "buildOrUpdateSuperpageGraph") {
      const result = _mockBuildOrUpdateSuperpageGraph(args[0]);
      return makeResponse(id, result);
    }

    if (method === "analyzeSuperpageSelection") {
      const result = _mockAnalyzeSuperpageSelection(args[0], args[1]);
      return makeResponse(id, result);
    }

    return makeErrorResponse(id, "UNKNOWN_METHOD", `Unknown method: ${method}`);
  } catch (err) {
    return makeErrorResponse(
      id,
      "RUNTIME_EXECUTION_ERROR",
      err?.message ?? String(err),
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
    _mockInitRuntime,
    _mockRuntimeStatus,
    _mockLoadSuperpageDocument,
    _mockBuildOrUpdateSuperpageGraph,
    _mockAnalyzeSuperpageSelection,
  };
}

/**
 * Fake message transport for M40.4 runtime launcher tests.
 *
 * 提供与 Message Runtime transport 一致的最小消息能力：
 * - send(request)
 * - onMessage(handler)
 * - offMessage(handler)
 * - emitResponse(response)
 */

function createReadyEnvelope(method, args = []) {
  if (method === "initRuntime") {
    return {
      status: "ready",
      target: null,
      items: [
        {
          kind: "runtime_ready",
          label: "Runtime Initialized",
          detail: {
            options: args[0] ?? {},
          },
        },
      ],
      diagnostics: [],
    };
  }

  if (method === "runtimeStatus") {
    return {
      status: "ready",
      target: null,
      items: [
        {
          kind: "runtime_status",
          label: "Runtime Status",
          detail: {
            initialized: true,
            running: true,
          },
        },
      ],
      diagnostics: [],
    };
  }

  if (method === "loadSuperpageDocument") {
    const sourcePath = args[0];
    return {
      status: "ready",
      target: sourcePath ?? null,
      items: [
        {
          kind: "document_loaded",
          label: "Document Loaded",
          detail: {
            source_path: sourcePath,
          },
        },
      ],
      diagnostics: [],
    };
  }

  if (method === "loadRemoteSuperpageDocument") {
    const fileRef = args[0] ?? {};
    return {
      status: "ready",
      target: fileRef.source_path ?? null,
      items: [
        {
          kind: "remote_document_loaded",
          label: "Remote Document Loaded",
          detail: {
            source_path: fileRef.source_path,
            file_id: fileRef.file_id,
          },
        },
      ],
      diagnostics: [],
    };
  }

  if (method === "buildOrUpdateSuperpageGraph") {
    const sourcePath = args[0];
    return {
      status: "ready",
      target: sourcePath ?? null,
      items: [
        {
          kind: "graph_built",
          label: "Graph Built",
          detail: {
            source_path: sourcePath,
          },
        },
      ],
      diagnostics: [],
    };
  }

  if (method === "analyzeSuperpageSelection") {
    const selection = args[0] ?? {};
    return {
      status: "ready",
      target: selection.active_component_id ?? selection.source_path ?? null,
      items: [
        {
          kind: "analysis",
          label: "Analysis Result",
          detail: {
            source_path: selection.source_path,
            active_component_id: selection.active_component_id,
          },
        },
      ],
      diagnostics: [],
    };
  }

  return {
    status: "error",
    target: null,
    items: [],
    diagnostics: [
      {
        severity: "error",
        code: "LAUNCHER_REQUEST_FAILED",
        message: `Unknown runtime method: ${method}`,
      },
    ],
  };
}

export function createFakeMessageTransport(options = {}) {
  const {
    autoRespond = true,
    failSend = 0,
    throwOnSend = false,
    delayMs = 0,
  } = options;

  const sentRequests = [];
  const handlers = new Set();
  let failSendLeft = typeof failSend === "number" ? failSend : failSend ? 1 : 0;

  function shouldFailSend() {
    if (!failSendLeft) {
      return false;
    }
    if (failSendLeft > 0) {
      failSendLeft -= 1;
      return true;
    }
    return false;
  }

  function emitResponse(response) {
    for (const handler of handlers) {
      handler(response);
    }
  }

  function schedule(fn) {
    if (delayMs > 0) {
      setTimeout(fn, delayMs);
    } else {
      fn();
    }
  }

  return {
    get sentRequests() {
      return sentRequests;
    },

    send(request) {
      sentRequests.push(request);

      if (throwOnSend) {
        throw new Error("Fake transport send failure");
      }

      const shouldFail = shouldFailSend();
      if (!autoRespond) {
        return;
      }

      if (!request || typeof request.id === "undefined") {
        return;
      }

      schedule(() => {
        if (shouldFail) {
          emitResponse({
            id: request.id,
            ok: false,
            result: null,
            error: {
              code: "LAUNCHER_REQUEST_FAILED",
              message: "Fake transport injected send failure",
            },
          });
          return;
        }

        emitResponse({
          id: request.id,
          ok: true,
          result: createReadyEnvelope(request.method, request.args),
          error: null,
        });
      });
    },

    onMessage(handler) {
      if (typeof handler !== "function") {
        return;
      }
      handlers.add(handler);
    },

    offMessage(handler) {
      handlers.delete(handler);
    },

    emitResponse(response) {
      emitResponse(response);
    },

    getSentRequests() {
      return sentRequests;
    },
  };
}

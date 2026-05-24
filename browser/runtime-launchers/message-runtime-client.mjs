/**
 * Message-based runtime client for M40.4 runtime launcher tests.
 *
 * 通过 request/response 消息协议与 transport 通信：
 * request: { id, method, args }
 * response: { id, ok, result, error }
 */

export const MESSAGE_RUNTIME_CLIENT_CONTROL = Symbol(
  "metadata-checker.runtime-message-client-control"
);

function createErrorEnvelope(message, code = "LAUNCHER_REQUEST_FAILED", target = null) {
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

function getErrorMessage(error) {
  if (error && typeof error === "object" && "message" in error) {
    return String(error.message);
  }
  if (typeof error === "string") {
    return error;
  }
  return "Unknown request error";
}

export function createMessageRuntimeClient({
  transport,
  requestTimeoutMs = 0,
  idGenerator,
  ignoredRequestIds,
  onError = () => {},
}) {
  if (!transport || typeof transport !== "object") {
    throw new Error("createMessageRuntimeClient: transport is required");
  }
  if (typeof transport.onMessage !== "function") {
    throw new Error("createMessageRuntimeClient: transport.onMessage is required");
  }
  if (typeof transport.offMessage !== "function") {
    throw new Error("createMessageRuntimeClient: transport.offMessage is required");
  }
  if (typeof transport.send !== "function") {
    throw new Error("createMessageRuntimeClient: transport.send is required");
  }

  let nextId = 0;
  let disposed = false;
  const pendingRequests = new Map();
  const staleResponseIds = ignoredRequestIds || new Set();
  const fallbackGenerator =
    idGenerator ??
    (() => {
      nextId += 1;
      return `msg-${nextId}`;
    });

  function reportMismatch(response) {
    onError({
      code: "LAUNCHER_RESPONSE_MISMATCH",
      message: "Received response for unknown request id",
      response,
      responseId: response && typeof response === "object" ? response.id : undefined,
    });
  }

  function removePending(id) {
    const pending = pendingRequests.get(id);
    if (!pending) {
      return;
    }
    if (pending.timeoutId !== null) {
      clearTimeout(pending.timeoutId);
    }
    pendingRequests.delete(id);
  }

  function buildTimeoutEnvelope(id) {
    return createErrorEnvelope(
      `Request ${String(id)} timed out after ${requestTimeoutMs}ms`,
      "LAUNCHER_REQUEST_TIMEOUT"
    );
  }

  function buildResponseFailureEnvelope(response, target) {
    const message = getErrorMessage(response?.error);
    const code =
      response?.error &&
      typeof response.error === "object" &&
      typeof response.error.code === "string" &&
      response.error.code.length > 0
        ? response.error.code
        : "LAUNCHER_REQUEST_FAILED";
    return createErrorEnvelope(message || "request failed", code, target);
  }

  function getPendingId() {
    const id = fallbackGenerator();
    if (typeof id !== "string" && typeof id !== "number") {
      throw new Error("createMessageRuntimeClient: idGenerator must return string or number");
    }
    return String(id);
  }

  function handleResponse(response) {
    const responseId =
      response && typeof response === "object" ? String(response.id) : undefined;
    const pending = pendingRequests.get(responseId);
    if (!pending) {
      if (responseId !== undefined && staleResponseIds.delete(responseId)) {
        return;
      }
      reportMismatch(response);
      return;
    }

    removePending(responseId);

    if (response && response.ok === false) {
      pending.resolve(buildResponseFailureEnvelope(response, pending.target));
      return;
    }

    if (!response || response.ok === undefined) {
      pending.resolve(
        createErrorEnvelope("Malformed response from transport", "LAUNCHER_REQUEST_FAILED", pending.target)
      );
      return;
    }

    if (response.ok === true) {
      if (response.result !== undefined) {
        pending.resolve(response.result);
        return;
      }
      pending.resolve(
        createErrorEnvelope("Malformed response from transport", "LAUNCHER_REQUEST_FAILED", pending.target)
      );
      return;
    }

    pending.resolve(
      createErrorEnvelope("Malformed response from transport", "LAUNCHER_REQUEST_FAILED", pending.target)
    );
  }

  function clearPendingRequests(reason = "pending requests cleared") {
    const pendingIds = Array.from(pendingRequests.entries());
    const clearedRequestIds = [];
    for (const [id, pending] of pendingIds) {
      const requestId = String(id);
      removePending(id);
      staleResponseIds.add(requestId);
      clearedRequestIds.push(requestId);
      pending.resolve(
        createErrorEnvelope(
          typeof reason === "string" ? reason : String(reason),
          "LAUNCHER_REQUEST_FAILED",
          pending.target
        )
      );
    }
    return clearedRequestIds;
  }

  function getPendingRequestCount() {
    return pendingRequests.size;
  }

  function dispose() {
    if (disposed) {
      return;
    }
    disposed = true;
    transport.offMessage(onTransportMessage);
    clearPendingRequests("runtime client disposed");
  }

  function request(method, args = []) {
    if (disposed) {
      return Promise.resolve(
        createErrorEnvelope("Runtime client disposed", "LAUNCHER_REQUEST_FAILED", null)
      );
    }

    const id = getPendingId();
    const requestObj = {
      id,
      method,
      args,
    };
    const target =
      method === "loadSuperpageDocument" || method === "buildOrUpdateSuperpageGraph"
        ? args[0] ?? null
        : method === "analyzeSuperpageSelection"
          ? args?.[0]?.active_component_id ?? args?.[0]?.source_path ?? null
          : null;

    let timeoutId = null;
    if (requestTimeoutMs > 0) {
      timeoutId = setTimeout(() => {
        const pending = pendingRequests.get(id);
        if (pending) {
          removePending(id);
          pending.resolve(buildTimeoutEnvelope(id));
        }
      }, requestTimeoutMs);
    }

    const promise = new Promise((resolve) => {
      pendingRequests.set(id, {
        method,
        args,
        resolve,
        timeoutId,
        target,
      });
    });

    try {
      const sendResult = transport.send(requestObj);
      if (sendResult && typeof sendResult.then === "function") {
        sendResult.catch((error) => {
          const pending = pendingRequests.get(id);
          if (!pending) {
            return;
          }
          removePending(id);
          pending.resolve(
            createErrorEnvelope(
              getErrorMessage(error),
              "LAUNCHER_REQUEST_FAILED",
              target
            )
          );
        });
      }
    } catch (err) {
      removePending(id);
      return Promise.resolve(
        createErrorEnvelope(
          getErrorMessage(err),
          "LAUNCHER_REQUEST_FAILED",
          target
        )
      );
    }

    return promise;
  }

  const onTransportMessage = (response) => {
    handleResponse(response);
  };
  transport.onMessage(onTransportMessage);

  const clientControl = {
    clearPendingRequests,
    getPendingRequestCount,
    dispose,
  };

  const client = {
    initRuntime(options = {}) {
      return request("initRuntime", [options]);
    },
    runtimeStatus() {
      return request("runtimeStatus", []);
    },
    loadSuperpageDocument(sourcePath, rawText) {
      return request("loadSuperpageDocument", [sourcePath, rawText]);
    },
    buildOrUpdateSuperpageGraph(sourcePath) {
      return request("buildOrUpdateSuperpageGraph", [sourcePath]);
    },
    analyzeSuperpageSelection(selection, options) {
      return request("analyzeSuperpageSelection", [selection, options]);
    },
  };

  Object.defineProperty(client, MESSAGE_RUNTIME_CLIENT_CONTROL, {
    value: clientControl,
    enumerable: false,
  });

  return client;
}

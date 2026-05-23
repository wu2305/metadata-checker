import {
  createMessageRuntimeClient,
  MESSAGE_RUNTIME_CLIENT_CONTROL,
} from "./message-runtime-client.mjs";

const REQUIRED_RUNTIME_METHODS = [
  "initRuntime",
  "runtimeStatus",
  "loadSuperpageDocument",
  "buildOrUpdateSuperpageGraph",
  "analyzeSuperpageSelection",
];

function assertTransport(value) {
  if (!value || typeof value !== "object") {
    return "a transport is required";
  }
  if (typeof value.send !== "function") {
    return "transport.send is required";
  }
  if (typeof value.onMessage !== "function") {
    return "transport.onMessage is required";
  }
  if (typeof value.offMessage !== "function") {
    return "transport.offMessage is required";
  }
  return null;
}

function assertRuntimeClient(value) {
  if (!value || (typeof value !== "object" && typeof value !== "function")) {
    return "a runtime client is required";
  }

  for (const method of REQUIRED_RUNTIME_METHODS) {
    if (typeof value[method] !== "function") {
      return `runtime client must implement ${method}`;
    }
  }
  return null;
}

function makeStartError(error) {
  const message =
    error && typeof error.message === "string"
      ? error.message
      : "browser extension runtime launcher start failed";
  const startError = new Error(message);
  startError.code = "LAUNCHER_START_FAILED";
  startError.cause = error;
  return startError;
}

function makeTransportError(message) {
  const error = new Error(message);
  error.code = "LAUNCHER_START_FAILED";
  return error;
}

export function createBrowserExtensionRuntimeLauncher(options = {}) {
  let state = "idle";
  let started = false;
  let fallbackUsed = false;
  let lastError = null;

  let _startPromise = null;
  let _runtimeClient = null;
  const _staleResponseIds = new Set();

  const resolveTransport = () => {
    if (options.transport !== undefined) {
      return options.transport;
    }

    if (options.transportFactory !== undefined) {
      if (typeof options.transportFactory !== "function") {
        throw makeTransportError("transportFactory must be a function");
      }
      return options.transportFactory();
    }

    throw makeTransportError("transport or transportFactory is required");
  };

  const toRuntimeClient = (transport) => {
    const transportError = assertTransport(transport);
    if (transportError) {
      throw makeTransportError(transportError);
    }

    const client = createMessageRuntimeClient({
      transport,
      requestTimeoutMs: options.requestTimeoutMs ?? 0,
      idGenerator: options.idGenerator,
      ignoredRequestIds: _staleResponseIds,
      onError: (error) => {
        lastError = error;
      },
    });

    const missing = assertRuntimeClient(client);
    if (missing) {
      throw makeTransportError(missing);
    }

    return client;
  };

  const clearPending = () => {
    const control = _runtimeClient?.[MESSAGE_RUNTIME_CLIENT_CONTROL];
    if (control && typeof control.clearPendingRequests === "function") {
      control.clearPendingRequests("runtime launcher stopped");
    }
  };

  const getPendingRequestCount = () => {
    const control = _runtimeClient?.[MESSAGE_RUNTIME_CLIENT_CONTROL];
    if (control && typeof control.getPendingRequestCount === "function") {
      return control.getPendingRequestCount();
    }
    return 0;
  };

  const disposeRuntimeClient = () => {
    const control = _runtimeClient?.[MESSAGE_RUNTIME_CLIENT_CONTROL];
    if (control && typeof control.dispose === "function") {
      control.dispose();
    }
  };

  const start = () => {
    if (state === "ready" && _runtimeClient) {
      return Promise.resolve(_runtimeClient);
    }

    if (_startPromise) {
      return _startPromise;
    }

    _startPromise = (async () => {
      state = "starting";
      started = false;
      lastError = null;
      fallbackUsed = false;

      try {
        const transport = await Promise.resolve(resolveTransport());
        const client = toRuntimeClient(transport);
        const transportError = assertTransport(transport);
        if (transportError) {
          throw makeTransportError(transportError);
        }

        _runtimeClient = client;
        state = "ready";
        started = true;
        return _runtimeClient;
      } catch (error) {
        state = "error";
        started = false;
        _runtimeClient = null;
        lastError = error?.code ? error : makeStartError(error);
        throw lastError;
      } finally {
        if (state !== "starting") {
          _startPromise = null;
        }
      }
    })();

    return _startPromise;
  };

  const stop = () => {
    clearPending();
    disposeRuntimeClient();
    state = "stopped";
    started = false;
    fallbackUsed = false;
    lastError = null;
    _runtimeClient = null;
    _startPromise = null;
    return { stopped: true };
  };

  const getClient = () => {
    if (state !== "ready") {
      return null;
    }
    return _runtimeClient;
  };

  const status = () => ({
    kind: "browser-extension",
    state,
    started,
    fallbackUsed,
    lastError,
    pendingRequestCount: getPendingRequestCount(),
  });

  return {
    start,
    stop,
    getClient,
    status,
  };
}

const REQUIRED_RUNTIME_METHODS = [
  "initRuntime",
  "runtimeStatus",
  "loadSuperpageDocument",
  "buildOrUpdateSuperpageGraph",
  "analyzeSuperpageSelection",
];

function normalizeStartError(error) {
  const message =
    error && typeof error.message === "string" ? error.message : "runtime launcher start failed";

  const startError = new Error(message);
  startError.code = "LAUNCHER_START_FAILED";
  startError.cause = error;
  return startError;
}

function makeRequestError(method, error) {
  const message =
    `runtime client method ${method} failed` +
    (error && typeof error.message === "string" ? `: ${error.message}` : "");

  return {
    status: "error",
    target: null,
    items: [],
    diagnostics: [
      {
        severity: "error",
        code: "LAUNCHER_REQUEST_FAILED",
        message,
      },
    ],
  };
}

function makeCanceledRequestError(method, reason) {
  return {
    status: "error",
    target: null,
    items: [],
    diagnostics: [
      {
        severity: "error",
        code: "LAUNCHER_REQUEST_FAILED",
        message: `runtime client method ${method} was canceled (${reason})`,
      },
    ],
  };
}

function makeNotStartedError() {
  return {
    status: "error",
    target: null,
    items: [],
    diagnostics: [
      {
        severity: "error",
        code: "LAUNCHER_NOT_STARTED",
        message: "runtime client not started",
      },
    ],
  };
}

function assertRuntimeClient(value) {
  if (!value || (typeof value !== "object" && typeof value !== "function")) {
    return "a client object is required";
  }

  for (const method of REQUIRED_RUNTIME_METHODS) {
    if (typeof value[method] !== "function") {
      return `runtime client must implement ${method}`;
    }
  }

  return null;
}

function findRuntimeClientFromModule(moduleValue) {
  if (!moduleValue) {
    return null;
  }

  if (typeof moduleValue === "function" && moduleValue.prototype === undefined) {
    try {
      const called = moduleValue();
      if (called && typeof called.then === "function") {
        return called;
      }
      if (called) {
        return called;
      }
    } catch {
      // ignore and continue resolving through object fallback
    }
  }

  const candidates = [
    moduleValue,
    moduleValue.default,
    moduleValue.runtimeClient,
  ];

  for (const candidate of candidates) {
    if (candidate && typeof candidate === "object" && assertRuntimeClient(candidate) === null) {
      return candidate;
    }
  }
  return null;
}

function makeDefaultStatus(overrides) {
  return {
    kind: "page",
    state: "idle",
    started: false,
    fallbackUsed: false,
    lastError: null,
    pendingRequestCount: 0,
    ...overrides,
  };
}

export function createPageRuntimeLauncher(options = {}) {
  const kind = "page";
  let state = "idle";
  let started = false;
  let fallbackUsed = false;
  let lastError = null;
  let pendingRequestCount = 0;
  const pendingRequestTokens = new Set();

  let _startPromise = null;
  let _runtimeClient = null;
  let _wrappedClient = null;
  let launcherGeneration = 0;

  const nextGeneration = () => {
    launcherGeneration += 1;
    return launcherGeneration;
  };

  const resolveClient = async () => {
    if (options.runtimeClient !== undefined) {
      return options.runtimeClient;
    }

    if (options.runtimeClientFactory !== undefined) {
      if (typeof options.runtimeClientFactory !== "function") {
        throw normalizeMissingClient("runtimeClientFactory must be a function");
      }
      return options.runtimeClientFactory();
    }

    if (options.runtimeModule !== undefined) {
      let imported = options.runtimeModule;
      if (typeof imported === "string" || imported instanceof URL) {
        imported = await import(imported.toString());
      }
      if (typeof imported === "function" && imported.prototype === undefined) {
        const called = imported();
        imported = called && typeof called.then === "function" ? await called : called;
      }

      const candidate = findRuntimeClientFromModule(imported);
      if (candidate) {
        return candidate;
      }
    }

    throw normalizeMissingClient(
      "runtimeClient/runtimeClientFactory/runtimeModule is required"
    );
  };

  const makeRuntimeClient = (baseClient) => {
    const activeGeneration = launcherGeneration;
    const wrappedClient = {};

    const isCurrentRequest = () =>
      state === "ready" &&
      _runtimeClient === baseClient &&
      launcherGeneration === activeGeneration;

    const finalizeRequest = (token) => {
      if (pendingRequestTokens.delete(token)) {
        pendingRequestCount -= 1;
      }
    };

    const makeResultEnvelope = (method, result) => {
      if (!isCurrentRequest()) {
        return makeCanceledRequestError(method, "launcher stopped or restarted");
      }
      return result;
    };

    for (const method of REQUIRED_RUNTIME_METHODS) {
      wrappedClient[method] = (...args) => {
        if (state !== "ready" || _runtimeClient !== baseClient) {
          return makeNotStartedError();
        }

        const requestToken = Symbol(`${method}:${pendingRequestCount}`);
        pendingRequestTokens.add(requestToken);
        pendingRequestCount += 1;
        try {
          const result = baseClient[method].apply(baseClient, args);
          if (result && typeof result.then === "function") {
            return result
              .then((value) => makeResultEnvelope(method, value))
              .catch((error) =>
                isCurrentRequest()
                  ? makeRequestError(method, error)
                  : makeCanceledRequestError(method, "launcher stopped or restarted")
              )
              .finally(() => {
                finalizeRequest(requestToken);
              });
          }

          finalizeRequest(requestToken);
          return makeResultEnvelope(method, result);
        } catch (error) {
          finalizeRequest(requestToken);
          return isCurrentRequest()
            ? makeRequestError(method, error)
            : makeCanceledRequestError(method, "launcher stopped or restarted");
        }
      };
    }

    return wrappedClient;
  };

  const start = () => {
    if (state === "ready" && _wrappedClient) {
      return Promise.resolve(_wrappedClient);
    }

    if (_startPromise) {
      return _startPromise;
    }

    _startPromise = (async () => {
      state = "starting";
      lastError = null;
      const activeGeneration = nextGeneration();
      try {
        const candidate = await Promise.resolve(resolveClient());
        const loadedClient = await Promise.resolve(candidate);
        if (state !== "starting" || activeGeneration !== launcherGeneration) {
          throw makeCanceledRequestError("start", "launcher stopped before start completed");
        }
        const missing = assertRuntimeClient(loadedClient);
        if (missing) {
          throw new Error(missing);
        }

        _runtimeClient = loadedClient;
        _wrappedClient = makeRuntimeClient(_runtimeClient);
        if (state === "starting" && activeGeneration === launcherGeneration) {
          state = "ready";
          started = true;
          pendingRequestCount = 0;
          return _wrappedClient;
        }

        throw makeCanceledRequestError("start", "launcher stopped before ready");
      } catch (error) {
        if (state === "starting" && activeGeneration === launcherGeneration) {
          state = "error";
          started = false;
          _runtimeClient = null;
          _wrappedClient = null;
          pendingRequestCount = 0;
          pendingRequestTokens.clear();
          const startError =
            error?.code === "LAUNCHER_START_FAILED"
              ? error
              : normalizeStartError(error);
          lastError = startError;
          throw startError;
        }

        throw error;
      } finally {
        if (state !== "starting") {
          _startPromise = null;
        }
      }
    })();

    return _startPromise;
  };

  const stop = () => {
    state = "stopped";
    nextGeneration();
    started = false;
    lastError = null;
    pendingRequestCount = 0;
    pendingRequestTokens.clear();
    _runtimeClient = null;
    _wrappedClient = null;
    _startPromise = null;
    return { stopped: true };
  };

  const getClient = () => {
    if (state !== "ready" || !_wrappedClient) {
      return null;
    }
    return _wrappedClient;
  };

  const status = () => {
    return makeDefaultStatus({
      kind,
      state,
      started,
      fallbackUsed,
      lastError,
      pendingRequestCount,
    });
  };

  return {
    start,
    stop,
    getClient,
    status,
  };
}

function normalizeMissingClient(message) {
  const error = new Error(message);
  error.code = "LAUNCHER_START_FAILED";
  return error;
}

import { createPageRuntimeLauncher } from "./page-runtime-launcher.mjs";

const LAUNCHER_NOT_IMPLEMENTED =
  "runtime launcher not implemented for this environment; keep stub in M40.4";

const LAUNCHER_EXPORTS = {
  "web-worker": "createWebWorkerRuntimeLauncher",
  "service-worker": "createServiceWorkerRuntimeLauncher",
  "browser-extension": "createBrowserExtensionRuntimeLauncher",
};

const LAUNCHER_MODULES = {
  "web-worker": "./web-worker-runtime-launcher.mjs",
  "service-worker": "./service-worker-runtime-launcher.mjs",
  "browser-extension": "./browser-extension-runtime-launcher.mjs",
};

function makeError(message) {
  const error = new Error(message);
  error.code = "LAUNCHER_START_FAILED";
  return error;
}

function defaultState(kind) {
  return {
    kind,
    state: "idle",
    started: false,
    fallbackUsed: false,
    lastError: null,
    pendingRequestCount: 0,
  };
}

export function createRuntimeLauncher(options = {}) {
  const kind = options.kind ?? "page";

  if (kind === "page") {
    return createPageRuntimeLauncher({
      ...options,
      kind: "page",
    });
  }

  if (!LAUNCHER_EXPORTS[kind] || !LAUNCHER_MODULES[kind]) {
    throw makeError(`Unsupported runtime launcher kind: ${kind}`);
  }

  let loadedLauncher = null;
  let loadError = null;
  let loadPromise = null;
  let state = "idle";

  const ensureLauncher = async () => {
    if (loadedLauncher) {
      return loadedLauncher;
    }
    if (!loadPromise) {
      loadPromise = (async () => {
        try {
          const modulePath = LAUNCHER_MODULES[kind];
          const launcherModule = await import(modulePath);
          const factoryName = LAUNCHER_EXPORTS[kind];
          const factory = launcherModule[factoryName];
          if (typeof factory !== "function") {
            throw new Error(`Missing exported function ${factoryName} in ${modulePath}`);
          }
          loadedLauncher = factory({
            ...options,
            kind,
          });
          return loadedLauncher;
        } catch (error) {
          loadError = makeError(`${LAUNCHER_NOT_IMPLEMENTED}: ${kind}`);
          loadError.cause = error;
          throw loadError;
        }
      })();
    }
    return loadPromise;
  };

  const start = async () => {
    if (loadError) {
      throw loadError;
    }
    state = "starting";
    try {
      const launcher = await ensureLauncher();
      const client = await launcher.start();
      state = "ready";
      return client;
    } catch (error) {
      state = "error";
      loadError = error?.code === "LAUNCHER_START_FAILED" ? error : makeError(String(error?.message ?? error));
      throw loadError;
    }
  };

  const stop = () => {
    if (!loadedLauncher) {
      state = "stopped";
      return { stopped: true, noop: true };
    }
    state = "stopped";
    return loadedLauncher.stop();
  };

  const getClient = () => {
    if (!loadedLauncher) {
      return null;
    }
    if (typeof loadedLauncher.getClient === "function") {
      return loadedLauncher.getClient();
    }
    return null;
  };

  const status = () => {
    if (!loadedLauncher) {
      return {
        ...defaultState(kind),
        state,
        lastError: loadError,
      };
    }

    if (typeof loadedLauncher.status === "function") {
      return {
        ...defaultState(kind),
        ...loadedLauncher.status(),
      };
    }

    return defaultState(kind);
  };

  return {
    start,
    stop,
    getClient,
    status,
  };
}

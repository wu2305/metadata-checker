/**
 * BI ECharts 运行时解析器
 */

const DEFAULT_TIMEOUT_MS = 2000;
const FALLBACK_SOURCE = "fallback";
const DIAGNOSTIC_CODE = "ECHARTS_RESOLVER_UNAVAILABLE";
const DIAGNOSTIC_MESSAGE =
  "ECharts runtime is unavailable, renderer should fallback to HTML/SVG";

function makeLogger(value) {
  const fallback = {
    debug() {},
    warn() {},
    error() {},
  };

  if (!value || typeof value !== "object") {
    return fallback;
  }

  return {
    debug(...args) {
      if (typeof value.debug === "function") {
        value.debug(...args);
      }
    },
    warn(...args) {
      if (typeof value.warn === "function") {
        value.warn(...args);
      }
    },
    error(...args) {
      if (typeof value.error === "function") {
        value.error(...args);
      }
    },
  };
}

function toGlobalLike(value) {
  if (value && typeof value === "object") {
    return value;
  }
  return typeof globalThis === "object" ? globalThis : {};
}

function safeGet(obj, pathParts) {
  let cursor = obj;
  for (const part of pathParts) {
    if (!cursor || typeof cursor !== "object") {
      return undefined;
    }
    cursor = cursor[part];
  }
  return cursor;
}

function isPromiseLike(value) {
  return value && typeof value.then === "function";
}

function normalizeDiagnostics(attempts) {
  return [
    {
      severity: "warning",
      code: DIAGNOSTIC_CODE,
      message: DIAGNOSTIC_MESSAGE,
      detail: { attempts },
    },
  ];
}

async function withTimeout(promise, timeoutMs) {
  if (!timeoutMs || timeoutMs <= 0) {
    return promise;
  }

  return Promise.race([
    promise,
    new Promise((_, reject) => {
      const timer = setTimeout(
        () => reject(new Error(`resolve ECharts timed out after ${timeoutMs}ms`)),
        timeoutMs
      );
      timer.unref?.();
    }),
  ]);
}

function unwrapPossibleGetEcharts(mod) {
  if (!mod || typeof mod !== "object") {
    return mod;
  }
  if (typeof mod.getEcharts === "function") {
    return mod;
  }
  if (
    mod.default &&
    typeof mod.default === "object" &&
    typeof mod.default.getEcharts === "function"
  ) {
    return mod.default;
  }
  return mod;
}

async function resolveFromCandidate(source, candidate) {
  const normalized = unwrapPossibleGetEcharts(candidate);
  if (normalized && typeof normalized.getEcharts === "function") {
    const maybeEcharts = normalized.getEcharts();
    if (isPromiseLike(maybeEcharts)) {
      return maybeEcharts;
    }
    return maybeEcharts;
  }

  if (source === "window.echarts") {
    return candidate?.echarts ?? candidate;
  }

  return candidate;
}

async function resolveViaRequire(requireLike, moduleName, timeoutMs, logger) {
  if (typeof requireLike !== "function") {
    return { source: null, error: "requireLike is not a function", module: undefined };
  }

  let done = false;

  const resolveOnce = new Promise((resolve, reject) => {
    const onSuccess = (moduleValue) => {
      if (done) {
        return;
      }
      done = true;
      resolve(moduleValue);
    };

    const onError = (error) => {
      if (done) {
        return;
      }
      done = true;
      lastError = error;
      reject(error);
    };

    let returned;
    try {
      returned = requireLike([moduleName], onSuccess, onError);
    } catch (error) {
      return reject(error);
    }

    if (isPromiseLike(returned)) {
      returned.then(
        (value) => {
          onSuccess(value);
        },
        (error) => {
          onError(error);
        }
      );
      return;
    }

    if (returned !== undefined) {
      onSuccess(returned);
    }
  });

  try {
    const moduleValue = await withTimeout(resolveOnce, timeoutMs);
    return {
      source: moduleName,
      error: null,
      module: moduleValue,
    };
  } catch (error) {
    logger.warn(
      "resolveViaRequire failed",
      moduleName,
      error?.message ?? String(error)
    );
    return { source: null, error: error?.message ?? String(error), module: undefined };
  }
}

async function resolveEchartsFromRequire(requireLike, moduleName, timeoutMs, logger) {
  const result = await resolveViaRequire(requireLike, moduleName, timeoutMs, logger);
  if (!result.source) {
    return {
      source: null,
      value: null,
      error: result.error,
    };
  }

  try {
    const value = await resolveFromCandidate(moduleName, result.module);
    if (!value) {
      return {
        source: null,
        value: null,
        error: `AMD module ${moduleName} resolved to empty value`,
      };
    }
    return {
      source: moduleName,
      value,
      error: null,
    };
  } catch (error) {
    return {
      source: null,
      value: null,
      error: error?.message ?? String(error),
    };
  }
}

function resolveFromWindow(globalLike) {
  const windowLike = safeGet(globalLike, ["window"]);
  if (windowLike && windowLike.echarts) {
    return { source: "window.echarts", value: windowLike.echarts };
  }

  if (safeGet(globalLike, ["echarts"])) {
    return { source: "window.echarts", value: globalLike.echarts };
  }

  return { source: null, value: null };
}

export async function resolveEchartsRuntime(options = {}) {
  const {
    globalThisLike,
    requireLike,
    timeoutMs = DEFAULT_TIMEOUT_MS,
    logger: userLogger,
  } = options;

  const logger = makeLogger(userLogger);
  const attempts = [];
  const globalLike = toGlobalLike(globalThisLike);

  const extAttempt = await resolveEchartsFromRequire(
    requireLike,
    "commons/echarts/echarts-ext",
    timeoutMs,
    logger
  );
  attempts.push({
    source: "commons/echarts/echarts-ext",
    success: !!extAttempt.value,
    error: extAttempt.error ?? null,
  });
  if (extAttempt.value) {
    return {
      echarts: extAttempt.value,
      source: extAttempt.source,
      diagnostics: [],
    };
  }

  const amdAttempt = await resolveEchartsFromRequire(
    requireLike,
    "echarts",
    timeoutMs,
    logger
  );
  attempts.push({
    source: "echarts",
    success: !!amdAttempt.value,
    error: amdAttempt.error ?? null,
  });
  if (amdAttempt.value) {
    return {
      echarts: amdAttempt.value,
      source: amdAttempt.source,
      diagnostics: [],
    };
  }

  const globalAttempt = resolveFromWindow(globalLike);
  attempts.push({
    source: "window.echarts",
    success: !!globalAttempt.value,
    error: globalAttempt.error ?? null,
  });
  if (globalAttempt.value) {
    return {
      echarts: globalAttempt.value,
      source: globalAttempt.source,
      diagnostics: [],
    };
  }

  return {
    echarts: null,
    source: FALLBACK_SOURCE,
    diagnostics: normalizeDiagnostics(attempts),
  };
}

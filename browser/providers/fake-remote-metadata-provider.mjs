/**
 * Fake Remote Metadata Provider for testing
 */

function _makeError(code, message) {
  return { status: "error", code, message };
}

const SENSITIVE_KEY_PATTERN = /(token|cookie|password)/i;
const SENSITIVE_PAIR_PATTERN = /(token|cookie|password)\s*[:=]\s*[^&\s,;}]+/gi;

function _redactSensitiveText(text) {
  if (typeof text !== "string") return text;
  return text.replace(SENSITIVE_PAIR_PATTERN, "[REDACTED]");
}

function _redactSensitiveValue(key, value) {
  if (SENSITIVE_KEY_PATTERN.test(String(key))) {
    return "[REDACTED]";
  }
  if (typeof value === "string") {
    return _redactSensitiveText(value);
  }
  return value;
}

function _redactForTelemetry(value, seen = new WeakSet()) {
  if (!value || typeof value !== "object") {
    return typeof value === "string" ? _redactSensitiveText(value) : value;
  }
  if (seen.has(value)) return "[Circular]";
  seen.add(value);
  if (Array.isArray(value)) {
    return value.map((item) => _redactForTelemetry(item, seen));
  }
  const redacted = {};
  for (const [key, item] of Object.entries(value)) {
    if (SENSITIVE_KEY_PATTERN.test(String(key))) {
      redacted.redacted = "[REDACTED]";
      continue;
    }
    redacted[key] = _redactSensitiveValue(key, _redactForTelemetry(item, seen));
  }
  return redacted;
}

function _delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export function createFakeRemoteMetadataProvider(options = {}) {
  const fixtures = options.fixtures ?? new Map();
  const returnType = options.returnType ?? "promise";
  const delayMs = options.delayMs ?? 0;
  const callLog = [];

  function logCall(method, args) {
    callLog.push({ method, args: _redactForTelemetry(args) });
  }

  function _resolve(value) {
    if (returnType === "sync") {
      return value;
    }
    return Promise.resolve(value);
  }

  function _reject(code, message) {
    const err = _makeError(code, message);
    if (returnType === "sync") {
      return err;
    }
    return Promise.reject(err);
  }

  function _findFixture(fileRef) {
    const key = fileRef?.source_path;
    if (!key) return undefined;
    return fixtures.get(key);
  }

  const provider = {
    getFileInfo(fileRef) {
      logCall("getFileInfo", [fileRef]);
      if (delayMs > 0) {
        return _delay(delayMs).then(() => provider.getFileInfo(fileRef));
      }
      if (options.getFileInfoShouldReject) {
        return _reject("REMOTE_FETCH_FAILED", "mock getFileInfo reject");
      }
      const fixture = _findFixture(fileRef);
      if (!fixture) {
        return _resolve(_makeError("REMOTE_FETCH_NOT_FOUND", "file not found"));
      }
      return _resolve({
        source_path: fileRef.source_path,
        file_id: fileRef.file_id ?? null,
        revision: fixture.revision ?? null,
        content_type: fixture.content_type ?? "unknown",
        updated_at: fixture.updated_at ?? null,
      });
    },

    getFileContent(fileRef) {
      logCall("getFileContent", [fileRef]);
      if (delayMs > 0) {
        return _delay(delayMs).then(() => provider.getFileContent(fileRef));
      }
      if (options.getFileContentShouldThrow) {
        throw new Error("mock getFileContent throw");
      }
      if (options.getFileContentShouldReject) {
        return _reject("REMOTE_FETCH_FAILED", "mock getFileContent reject");
      }
      const fixture = _findFixture(fileRef);
      if (!fixture) {
        return _resolve(_makeError("REMOTE_FETCH_NOT_FOUND", "file not found"));
      }
      return _resolve({
        source_path: fileRef.source_path,
        file_id: fileRef.file_id ?? null,
        revision: fixture.revision ?? null,
        content_type: fixture.content_type ?? "unknown",
        raw_text: fixture.raw_text ?? "",
      });
    },

    getRelatedFiles(fileRef) {
      logCall("getRelatedFiles", [fileRef]);
      if (delayMs > 0) {
        return _delay(delayMs).then(() => []);
      }
      return _resolve([]);
    },

    callLog,
  };

  return provider;
}

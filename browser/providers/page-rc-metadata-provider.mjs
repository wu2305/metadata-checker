/**
 * M40.6：Page RC Metadata Provider
 *
 * 通过 JS bridge 调用 `window.SZ.rc` / `window.SZ.rc1` 作为 fallback。
 * 不参与解析、不建图、不保存数据。
 */

function _makeError(code, message) {
  return { status: "error", code, message };
}

function _emitHost(host, eventName, payload) {
  if (host && typeof host.emit === "function") {
    host.emit(eventName, payload);
  }
}

function _log(logger, level, ...args) {
  const log = logger ?? console;
  log?.[level]?.(...args);
}

function _safeJsonParse(text) {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

function _inferContentType(sourcePath) {
  if (typeof sourcePath !== "string") return "unknown";
  if (sourcePath.endsWith(".spg")) return "super_page";
  if (sourcePath.endsWith(".tbl")) return "table";
  return "unknown";
}

function _isLogicalSourcePath(path) {
  if (typeof path !== "string" || path === "") return false;
  if (path.startsWith("/") || path.startsWith("\\")) return false;
  if (path.startsWith("http://") || path.startsWith("https://") || path.startsWith("file://")) {
    return false;
  }
  if (/^[A-Za-z]:/.test(path)) return false;
  if (path.split(/[\/]/).some((segment) => segment === "..")) return false;
  return true;
}

function _validateFileRef(fileRef) {
  if (!fileRef || typeof fileRef !== "object") {
    return { valid: false, error: _makeError("REMOTE_RESPONSE_INVALID", "fileRef must be an object") };
  }
  if (!_isLogicalSourcePath(fileRef.source_path)) {
    return {
      valid: false,
      error: _makeError(
        "REMOTE_RESPONSE_INVALID",
        `source_path is not a project-internal logical path: ${fileRef.source_path}`
      ),
    };
  }
  return { valid: true };
}

export function createPageRcMetadataProvider(options = {}) {
  const rc = options.rc ?? (typeof window !== "undefined" ? window.SZ?.rc : undefined);
  const rc1 = options.rc1 ?? (typeof window !== "undefined" ? window.SZ?.rc1 : undefined);
  const host = options.host;
  const logger = options.logger;

  async function _callRc(method, args) {
    if (typeof rc === "function") {
      return rc(method, ...args);
    }
    if (typeof rc1 === "function") {
      return rc1(method, ...args);
    }
    return null;
  }

  const provider = {
    async getFileInfo(fileRef) {
      const validation = _validateFileRef(fileRef);
      if (!validation.valid) {
        _emitHost(host, "metadata_fetch_failed", { error: validation.error, timestamp: Date.now() });
        return validation.error;
      }

      const sourcePath = fileRef.source_path;
      const fileId = fileRef.file_id ?? null;

      if (!rc && !rc1) {
        const err = _makeError(
          "PAGE_RC_UNAVAILABLE",
          "window.SZ.rc / rc1 is not available"
        );
        _log(logger, "warn", "[page-rc-provider] rc unavailable:", err);
        _emitHost(host, "metadata_fetch_failed", { error: err, timestamp: Date.now() });
        return err;
      }

      try {
        const result = await _callRc("getFileInfo", [fileId, sourcePath]);
        if (!result) {
          const err = _makeError(
            "REMOTE_RESPONSE_INVALID",
            "rc returned null or undefined"
          );
          _emitHost(host, "metadata_fetch_failed", { error: err, timestamp: Date.now() });
          return err;
        }

        const parsed = typeof result === "string" ? _safeJsonParse(result) : result;
        if (!parsed || typeof parsed !== "object") {
          const err = _makeError(
            "REMOTE_RESPONSE_INVALID",
            "rc returned invalid response"
          );
          _emitHost(host, "metadata_fetch_failed", { error: err, timestamp: Date.now() });
          return err;
        }

        return {
          source_path: sourcePath,
          file_id: fileId,
          revision: parsed.revision ?? null,
          content_type: parsed.content_type ?? _inferContentType(sourcePath),
          updated_at: parsed.updated_at ?? null,
        };
      } catch (err) {
        const error = _makeError(
          "REMOTE_FETCH_FAILED",
          err?.message ?? String(err)
        );
        _log(logger, "error", "[page-rc-provider] getFileInfo failed:", err);
        _emitHost(host, "metadata_fetch_failed", { error, timestamp: Date.now() });
        return error;
      }
    },

    async getFileContent(fileRef) {
      const validation = _validateFileRef(fileRef);
      if (!validation.valid) {
        _emitHost(host, "metadata_fetch_failed", { error: validation.error, timestamp: Date.now() });
        return validation.error;
      }

      const sourcePath = fileRef.source_path;
      const fileId = fileRef.file_id ?? null;

      if (!rc && !rc1) {
        const err = _makeError(
          "PAGE_RC_UNAVAILABLE",
          "window.SZ.rc / rc1 is not available"
        );
        _log(logger, "warn", "[page-rc-provider] rc unavailable:", err);
        _emitHost(host, "metadata_fetch_failed", { error: err, timestamp: Date.now() });
        return err;
      }

      try {
        const result = await _callRc("getFileContent", [fileId, sourcePath]);
        if (!result) {
          const err = _makeError(
            "REMOTE_RESPONSE_INVALID",
            "rc returned null or undefined"
          );
          _emitHost(host, "metadata_fetch_failed", { error: err, timestamp: Date.now() });
          return err;
        }

        const rawText = typeof result === "string" ? result : JSON.stringify(result);

        return {
          source_path: sourcePath,
          file_id: fileId,
          revision: null,
          content_type: _inferContentType(sourcePath),
          raw_text: rawText,
        };
      } catch (err) {
        const error = _makeError(
          "REMOTE_FETCH_FAILED",
          err?.message ?? String(err)
        );
        _log(logger, "error", "[page-rc-provider] getFileContent failed:", err);
        _emitHost(host, "metadata_fetch_failed", { error, timestamp: Date.now() });
        return error;
      }
    },

    async getRelatedFiles(fileRef) {
      return [];
    },
  };

  return provider;
}

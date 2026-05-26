/*
 * M43 Runtime Adapter
 *
 * 将 page-script 的 request 转换为业务无关的响应对象。
 */

(function (root) {
  "use strict";

  const protocol = root.__metadata_checker_bridge_protocol__ || {};
  const createDiagnostic = protocol.createDiagnostic || ((code, message) => ({ severity: "error", code, message }));
  const DIAGNOSTIC_CODES = protocol.DIAGNOSTIC_CODES || {
    BRIDGE_MISSING: "METADATA_CHECKER_BRIDGE_MISSING",
    BRIDGE_REQUEST_FAILED: "METADATA_CHECKER_REQUEST_FAILED",
    BRIDGE_REQUEST_UNSUPPORTED: "METADATA_CHECKER_REQUEST_UNSUPPORTED",
  };

  function _safeCall(fn, args = []) {
    if (typeof fn !== "function") {
      return {
        ok: false,
        value: null,
        message: "function not provided",
      };
    }

    try {
      const value = fn(...args);
      if (value && typeof value?.then === "function") {
        return value.then(
          (resolved) => ({ ok: true, value: resolved }),
          (error) => ({ ok: false, message: _errorMessage(error) }),
        );
      }
      return { ok: true, value };
    } catch (error) {
      return { ok: false, message: _errorMessage(error) };
    }
  }

  function _errorMessage(error) {
    if (error && typeof error === "object" && typeof error.message === "string") {
      return error.message;
    }
    if (typeof error === "string") {
      return error;
    }
    return "request failed";
  }

  function _normalizeDiagnostics(list) {
    if (!Array.isArray(list)) {
      return [];
    }
    return list.filter(
      (item) => item && typeof item.code === "string" && typeof item.message === "string",
    );
  }

  function _normalizeSelection(selection) {
    if (!selection || typeof selection !== "object") {
      return null;
    }

    const selectedIds = Array.isArray(selection.selected_component_ids)
      ? selection.selected_component_ids.slice()
      : [];

    return {
      source_path: selection.source_path ?? null,
      file_id: selection.file_id ?? "",
      selected_component_ids: selectedIds,
      active_component_id: selection.active_component_id ?? null,
      selection_infos: selection.selection_infos ?? selection.selected_component_infos ?? {},
      selected_component_infos: selection.selected_component_infos ?? selection.selection_infos ?? {},
      project_name: selection.project_name ?? "",
      timestamp: typeof selection.timestamp === "number" ? selection.timestamp : Date.now(),
      page_type: selection.page_type ?? "unknown",
    };
  }

  function _normalizeBridgeStatusInput(value) {
    return value === true || value === false ? value : false;
  }

  async function _buildBridgeStatusContext(handlers, requestEnvelope) {
    const pageContextResult = await _safeCall(handlers.getPageContext, [requestEnvelope]);
    const selectionResult = await _safeCall(handlers.getSelectionSnapshot, [requestEnvelope]);

    const diagnostics = [];
    let pageContext = null;
    let selection = null;

    if (!pageContextResult.ok) {
      diagnostics.push(
        createDiagnostic(
          DIAGNOSTIC_CODES.BRIDGE_REQUEST_FAILED,
          `getPageContext failed: ${pageContextResult.message}`,
        ),
      );
    } else {
      pageContext = pageContextResult.value || null;
    }

    if (!selectionResult.ok) {
      diagnostics.push(
        createDiagnostic(
          DIAGNOSTIC_CODES.BRIDGE_REQUEST_FAILED,
          `getSelectionSnapshot failed: ${selectionResult.message}`,
        ),
      );
    } else {
      selection = _normalizeSelection(selectionResult.value?.selection ?? selectionResult.value);
    }

    return {
      bridge_detected: _normalizeBridgeStatusInput(pageContextResult.ok),
      page_context: pageContext,
      selection,
      diagnostics,
    };
  }

  const defaultOptions = {
    getBridgeStatus: null,
    getPageContext: null,
    getSelectionSnapshot: null,
    analyzeCurrentSelection: null,
  };

  function createRuntimeAdapter(rawOptions = {}) {
    const options = { ...defaultOptions, ...rawOptions };

    const handlers = {
      getBridgeStatus: options.getBridgeStatus,
      getPageContext: options.getPageContext,
      getSelectionSnapshot: options.getSelectionSnapshot,
      analyzeCurrentSelection: options.analyzeCurrentSelection,
    };

    async function getBridgeStatus(requestEnvelope) {
      if (typeof handlers.getBridgeStatus === "function") {
        const result = await _safeCall(handlers.getBridgeStatus, [requestEnvelope]);
        if (result.ok) {
          return {
            payload: result.value ?? {},
            diagnostics: [],
          };
        }
        return {
          payload: null,
          diagnostics: [
            createDiagnostic(
              DIAGNOSTIC_CODES.BRIDGE_REQUEST_FAILED,
              result.message || "getBridgeStatus failed",
            ),
          ],
        };
      }

      const context = await _buildBridgeStatusContext(handlers, requestEnvelope);
      return {
        payload: context,
        diagnostics: context.diagnostics,
      };
    }

    async function getPageContext(requestEnvelope) {
      if (typeof handlers.getPageContext === "function") {
        const result = await _safeCall(handlers.getPageContext, [requestEnvelope]);
        if (!result.ok) {
          return {
            payload: null,
            diagnostics: [
              createDiagnostic(
                DIAGNOSTIC_CODES.BRIDGE_REQUEST_FAILED,
                result.message || "getPageContext failed",
              ),
            ],
          };
        }
        return {
          payload: {
            page_context: result.value || null,
          },
          diagnostics: [],
        };
      }

      return {
        payload: null,
        diagnostics: [
          createDiagnostic(
            DIAGNOSTIC_CODES.BRIDGE_MISSING,
            "page bridge handler is not provided",
          ),
        ],
      };
    }

    async function getSelectionSnapshot(requestEnvelope) {
      if (typeof handlers.getSelectionSnapshot === "function") {
        const result = await _safeCall(handlers.getSelectionSnapshot, [requestEnvelope]);
        if (!result.ok) {
          return {
            payload: null,
            diagnostics: [
              createDiagnostic(
                DIAGNOSTIC_CODES.BRIDGE_REQUEST_FAILED,
                result.message || "getSelectionSnapshot failed",
              ),
            ],
          };
        }
        const selection = _normalizeSelection(result.value?.selection ?? result.value);
        return {
          payload: {
            selection,
            diagnostics: _normalizeDiagnostics(result.value?.diagnostics),
          },
          diagnostics: _normalizeDiagnostics(result.value?.diagnostics),
        };
      }

      return {
        payload: {
          selection: null,
        },
        diagnostics: [
          createDiagnostic(
            DIAGNOSTIC_CODES.BRIDGE_MISSING,
            "selection bridge handler is not provided",
          ),
        ],
      };
    }

    async function analyzeCurrentSelection(requestEnvelope) {
      if (typeof handlers.analyzeCurrentSelection === "function") {
        const result = await _safeCall(handlers.analyzeCurrentSelection, [requestEnvelope]);
        if (result.ok) {
          return {
            payload: result.value ?? { supported: true },
            diagnostics: _normalizeDiagnostics(result.value?.diagnostics),
          };
        }
        return {
          payload: { supported: false },
          diagnostics: [
            createDiagnostic(
              DIAGNOSTIC_CODES.BRIDGE_REQUEST_FAILED,
              result.message || "analyzeCurrentSelection failed",
            ),
          ],
        };
      }

      return {
        payload: { supported: false },
        diagnostics: [
          createDiagnostic(
            DIAGNOSTIC_CODES.BRIDGE_REQUEST_UNSUPPORTED,
            "analyzeCurrentSelection is not supported in this phase",
          ),
        ],
      };
    }

    async function handleRequest(requestEnvelope) {
      if (!requestEnvelope || typeof requestEnvelope !== "object") {
        return {
          payload: null,
          diagnostics: [
            createDiagnostic(
              DIAGNOSTIC_CODES.BRIDGE_REQUEST_INVALID,
              "request is not an object",
            ),
          ],
        };
      }

      const type = requestEnvelope.type;
      if (type === "getBridgeStatus") {
        return getBridgeStatus(requestEnvelope);
      }
      if (type === "getPageContext") {
        return getPageContext(requestEnvelope);
      }
      if (type === "getSelectionSnapshot") {
        return getSelectionSnapshot(requestEnvelope);
      }
      if (type === "analyzeCurrentSelection") {
        return analyzeCurrentSelection(requestEnvelope);
      }

      return {
        payload: null,
        diagnostics: [
          createDiagnostic(
            DIAGNOSTIC_CODES.BRIDGE_REQUEST_UNSUPPORTED,
            `unsupported request type: ${String(type)}`,
          ),
        ],
      };
    }

    return {
      handleRequest,
      handle: handleRequest,
      getBridgeStatus,
      getPageContext,
      getSelectionSnapshot,
      analyzeCurrentSelection,
    };
  }

  if (root && typeof root === "object") {
    root.__metadata_checker_runtime_adapter__ = {
      createRuntimeAdapter,
      createEnvelope: protocol.createResponseEnvelope,
    };
  }
})(typeof globalThis === "undefined" ? undefined : globalThis);

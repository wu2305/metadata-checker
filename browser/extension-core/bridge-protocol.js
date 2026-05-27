/*
 * M43 浏览器扩展 Bridge Protocol
 *
 * 约定字段：protocol / request_id / type / payload / diagnostics。
 */

(function (root) {
  "use strict";

  const BRIDGE_PROTOCOL_NAME = "metadata_checker_designer_bridge";
  const BRIDGE_PROTOCOL_VERSION = "m43-protocol-v1";
  const BRIDGE_PROTOCOL = Object.freeze({
    name: BRIDGE_PROTOCOL_NAME,
    version: BRIDGE_PROTOCOL_VERSION,
  });

  const BRIDGE_READY_EVENT_NAME = "__metadata_checker_designer_ready__";

  const BRIDGE_REQUEST_TYPES = Object.freeze([
    "getBridgeStatus",
    "getPageContext",
    "getSelectionSnapshot",
    "analyzeCurrentSelection",
    "getAccessToken",
  ]);

  const DIAGNOSTIC_CODES = Object.freeze({
    PROTOCOL_MISMATCH: "METADATA_CHECKER_PROTOCOL_MISMATCH",
    BRIDGE_MISSING: "METADATA_CHECKER_BRIDGE_MISSING",
    BRIDGE_REQUEST_UNSUPPORTED: "METADATA_CHECKER_REQUEST_UNSUPPORTED",
    BRIDGE_REQUEST_FAILED: "METADATA_CHECKER_REQUEST_FAILED",
    BRIDGE_REQUEST_INVALID: "METADATA_CHECKER_REQUEST_INVALID",
    BRIDGE_SELECTION_EMPTY: "METADATA_CHECKER_SELECTION_EMPTY",
  });

  const REQUEST_ID_PREFIX = "m43-rq";
  let requestCounter = 0;

  function _normalizeRequestId(requestId) {
    if (typeof requestId === "string" && requestId.length > 0) {
      return requestId;
    }
    if (typeof requestId === "number" && Number.isFinite(requestId)) {
      return String(requestId);
    }
    requestCounter += 1;
    return `${REQUEST_ID_PREFIX}-${String(requestCounter)}`;
  }

  function _isObject(value) {
    return value !== null && typeof value === "object";
  }

  function _protocolEquals(candidate) {
    return (
      _isObject(candidate) &&
      candidate.name === BRIDGE_PROTOCOL_NAME &&
      candidate.version === BRIDGE_PROTOCOL_VERSION
    );
  }

  function isSupportedType(type) {
    return BRIDGE_REQUEST_TYPES.includes(type);
  }

  function createDiagnostic(code, message, severity = "error") {
    return {
      severity,
      code,
      message,
    };
  }

  function createEnvelope({
    type,
    requestId = null,
    payload = null,
    diagnostics = [],
    protocol = BRIDGE_PROTOCOL,
  }) {
    return {
      protocol,
      request_id: _normalizeRequestId(requestId),
      type,
      payload,
      diagnostics,
    };
  }

  function createRequestEnvelope({ type, requestId = null, payload = {} }) {
    return createEnvelope({ type, requestId, payload });
  }

  function createResponseEnvelope({ requestEnvelope, type = requestEnvelope?.type, payload = null, diagnostics = [] }) {
    return createEnvelope({
      type,
      requestId: requestEnvelope?.request_id,
      payload,
      diagnostics,
      protocol: _protocolFromRequest(requestEnvelope),
    });
  }

  function createProtocolMismatchEnvelope(requestEnvelope, { details = "" } = {}) {
    const diagnostics = [
      createDiagnostic(
        DIAGNOSTIC_CODES.PROTOCOL_MISMATCH,
        details || "metadata checker protocol does not match",
      ),
    ];
    return createResponseEnvelope({
      requestEnvelope,
      type: requestEnvelope?.type || "unknown",
      payload: null,
      diagnostics,
    });
  }

  function createUnsupportedRequestEnvelope(requestEnvelope, details = "unsupported request type") {
    return createResponseEnvelope({
      requestEnvelope,
      payload: null,
      diagnostics: [createDiagnostic(DIAGNOSTIC_CODES.BRIDGE_REQUEST_UNSUPPORTED, details)],
    });
  }

  function _protocolFromRequest(requestEnvelope) {
    if (_protocolEquals(requestEnvelope?.protocol)) {
      return requestEnvelope.protocol;
    }
    return BRIDGE_PROTOCOL;
  }

  function isMetadataCheckerPayload(value) {
    return _isObject(value) && typeof value?.type === "string" &&
      typeof value?.request_id !== "undefined";
  }

  function isMetadataCheckerRequest(value) {
    return isMetadataCheckerPayload(value) && _protocolEquals(value.protocol);
  }

  function isMetadataCheckerRequestWithProtocolMismatch(value) {
    return (
      isMetadataCheckerPayload(value) &&
      !_protocolEquals(value.protocol)
    );
  }

  const exported = {
    BRIDGE_PROTOCOL,
    BRIDGE_PROTOCOL_NAME,
    BRIDGE_PROTOCOL_VERSION,
    BRIDGE_READY_EVENT_NAME,
    BRIDGE_REQUEST_TYPES,
    DIAGNOSTIC_CODES,
    createDiagnostic,
    createEnvelope,
    createRequestEnvelope,
    createResponseEnvelope,
    createProtocolMismatchEnvelope,
    createUnsupportedRequestEnvelope,
    isMetadataCheckerRequest,
    isMetadataCheckerRequestWithProtocolMismatch,
    isSupportedType,
  };

  if (root && typeof root === "object") {
    root.__metadata_checker_bridge_protocol__ = Object.freeze(exported);
  }
})(typeof globalThis === "undefined" ? undefined : globalThis);

/* M43 页面侧消息桥接脚本（页面上下文） */

(function (root) {
  "use strict";

  const protocol = root.__metadata_checker_bridge_protocol__ || {};
  const BRIDGE_PROTOCOL = protocol.BRIDGE_PROTOCOL || {
    name: "metadata_checker_designer_bridge",
    version: "m43-protocol-v1",
  };
  const BRIDGE_REQUEST_TYPES = protocol.BRIDGE_REQUEST_TYPES || [
    "getBridgeStatus",
    "getPageContext",
    "getSelectionSnapshot",
    "analyzeCurrentSelection",
  ];
  const DIAGNOSTIC_CODES = protocol.DIAGNOSTIC_CODES || {
    PROTOCOL_MISMATCH: "METADATA_CHECKER_PROTOCOL_MISMATCH",
    BRIDGE_MISSING: "METADATA_CHECKER_BRIDGE_MISSING",
    BRIDGE_REQUEST_UNSUPPORTED: "METADATA_CHECKER_REQUEST_UNSUPPORTED",
    BRIDGE_REQUEST_FAILED: "METADATA_CHECKER_REQUEST_FAILED",
  };
  const BRIDGE_READY_EVENT_NAME = protocol.BRIDGE_READY_EVENT_NAME || "__metadata_checker_designer_ready__";
  const createDiagnostic = protocol.createDiagnostic || ((code, message, severity = "error") => ({
    severity,
    code,
    message,
  }));
  const createResponseEnvelope = protocol.createResponseEnvelope;
  const createProtocolMismatchEnvelope = protocol.createProtocolMismatchEnvelope;
  const isMetadataCheckerRequest = protocol.isMetadataCheckerRequest;
  const isMetadataCheckerRequestWithProtocolMismatch = protocol.isMetadataCheckerRequestWithProtocolMismatch;

  const REQUEST_DIRECTION = "request";
  const RESPONSE_DIRECTION = "response";
  const BRIDGE_EVENT_NAME = "metadata-checker-bridge-message";

  function isObject(value) {
    return value !== null && typeof value === "object";
  }

  function asArray(value) {
    return Array.isArray(value) ? value : [];
  }

  function asDiagnostics(value) {
    return asArray(value).filter(
      (item) =>
        item &&
        typeof item.code === "string" &&
        typeof item.message === "string",
    );
  }

  function writeMarker(name, value) {
    const doc = root.document;
    if (!doc || typeof doc.createElement !== "function") {
      return;
    }
    const key = `data-metadata-checker-${name}`;
    const existing = typeof doc.querySelector === "function" ? doc.querySelector(`[${key}]`) : null;
    if (existing && typeof existing.setAttribute === "function") {
      existing.setAttribute(key, value);
      return;
    }
    const marker = doc.createElement("span");
    marker.setAttribute(key, value);
    marker.style.display = "none";
    doc.documentElement?.appendChild(marker);
  }

  function coerceSelection(selection) {
    if (!isObject(selection)) {
      return {
        source_path: null,
        file_id: "",
        selected_component_ids: [],
        active_component_id: null,
        selection_infos: {},
        project_name: "",
        timestamp: Date.now(),
        page_type: "unknown",
      };
    }

    return {
      source_path: selection.source_path ?? null,
      file_id: typeof selection.file_id === "string" ? selection.file_id : "",
      selected_component_ids: asArray(selection.selected_component_ids).slice(),
      active_component_id: typeof selection.active_component_id === "string" ? selection.active_component_id : null,
      selection_infos: isObject(selection.selection_infos)
        ? selection.selection_infos
        : isObject(selection.selected_component_infos)
          ? selection.selected_component_infos
          : {},
      selected_component_infos: isObject(selection.selected_component_infos)
        ? selection.selected_component_infos
        : isObject(selection.selection_infos)
          ? selection.selection_infos
          : {},
      project_name: typeof selection.project_name === "string" ? selection.project_name : "",
      timestamp: typeof selection.timestamp === "number" ? selection.timestamp : Date.now(),
      page_type: typeof selection.page_type === "string" ? selection.page_type : "unknown",
    };
  }

  function getBridge() {
    return root.__metadata_checker_designer_bridge__;
  }

  function safeInvoke(bridge, methodName, fallback) {
    if (!bridge || typeof bridge[methodName] !== "function") {
      return {
        payload: fallback,
        diagnostics: [
          createDiagnostic(
            DIAGNOSTIC_CODES.BRIDGE_MISSING,
            `designer bridge method ${methodName} unavailable`,
            "warning",
          ),
        ],
      };
    }

    try {
      return {
        payload: bridge[methodName](),
        diagnostics: [],
      };
    } catch (error) {
      return {
        payload: fallback,
        diagnostics: [
          createDiagnostic(
            DIAGNOSTIC_CODES.BRIDGE_REQUEST_FAILED,
            error?.message || `designer bridge method ${methodName} threw`,
            "error",
          ),
        ],
      };
    }
  }

  function extractPageContext(raw) {
    if (!isObject(raw)) {
      return null;
    }
    return isObject(raw.page_context) ? raw.page_context : raw;
  }

  function extractSelection(raw) {
    if (!isObject(raw)) {
      return null;
    }
    return isObject(raw.selection) ? coerceSelection(raw.selection) : coerceSelection(raw);
  }

  function buildBridgeMissingResponse(message, type = "getBridgeStatus") {
    const missingDiagnostic = createDiagnostic(
      DIAGNOSTIC_CODES.BRIDGE_MISSING,
      message,
      "warning",
    );
    return {
      bridge_detected: false,
      page_context: null,
      selection: null,
      diagnostics: [missingDiagnostic],
    };
  }

  function getPageContextFromBridge() {
    const bridge = getBridge();
    if (!bridge) {
      return buildBridgeMissingResponse("metadata checker bridge is not ready", "getPageContext");
    }

    const result = safeInvoke(bridge, "getPageContext", null);
    const context = extractPageContext(result.payload);
    return {
      bridge_detected: true,
      page_context: context,
      selection: null,
      diagnostics: asDiagnostics(result.diagnostics),
    };
  }

  function getSelectionSnapshotFromBridge() {
    const bridge = getBridge();
    if (!bridge) {
      return buildBridgeMissingResponse("metadata checker bridge is not ready", "getSelectionSnapshot");
    }

    const pageContextResult = safeInvoke(bridge, "getPageContext", null);
    const selectionResult = safeInvoke(bridge, "getSelectionSnapshot", null);
    return {
      bridge_detected: true,
      page_context: extractPageContext(pageContextResult.payload),
      selection: extractSelection(selectionResult.payload),
      diagnostics: asDiagnostics([...pageContextResult.diagnostics, ...selectionResult.diagnostics]),
    };
  }

  function getBridgeStatusFromBridge() {
    const snapshot = getSelectionSnapshotFromBridge();
    return {
      bridge_detected: snapshot.bridge_detected,
      page_context: isObject(snapshot.page_context) ? snapshot.page_context : null,
      selection: snapshot.selection,
      diagnostics: asDiagnostics(snapshot.diagnostics),
    };
  }

  function analyzeCurrentSelectionFromBridge() {
    const bridge = getBridge();
    if (!bridge) {
      return {
        supported: false,
        diagnostics: [
          createDiagnostic(
            DIAGNOSTIC_CODES.BRIDGE_MISSING,
            "metadata checker bridge is not ready",
            "warning",
          ),
        ],
      };
    }

    const result = safeInvoke(bridge, "analyzeCurrentSelection", { supported: false });
    if (result.payload && typeof result.payload === "object" && result.payload.supported !== undefined) {
      return {
        supported: result.payload.supported,
        diagnostics: asDiagnostics(result.diagnostics),
      };
    }

    return {
      supported: false,
      diagnostics: asDiagnostics(result.diagnostics).concat([
        createDiagnostic(
          DIAGNOSTIC_CODES.BRIDGE_REQUEST_UNSUPPORTED,
          "analyzeCurrentSelection is not supported in first implementation",
          "info",
        ),
      ]),
    };
  }

  function makeResponse(type, request, payload, diagnostics) {
    const response = createResponseEnvelope
      ? createResponseEnvelope({
        requestEnvelope: request,
        type,
        payload,
        diagnostics: asDiagnostics(diagnostics),
      })
      : {
        protocol: BRIDGE_PROTOCOL,
        request_id: request.request_id,
        type,
        payload,
        diagnostics: asDiagnostics(diagnostics),
      };
    return {
      ...response,
      __metadata_checker_bridge_token: request.__metadata_checker_bridge_token ?? null,
    };
  }

  function protocolMismatch(request) {
    if (typeof createProtocolMismatchEnvelope === "function") {
      return createProtocolMismatchEnvelope(request, {
        details: "metadata checker protocol mismatch",
      });
    }
    return makeResponse(
      request.type || "unknown",
      request,
      null,
      [
        createDiagnostic(
          DIAGNOSTIC_CODES.PROTOCOL_MISMATCH,
          "metadata checker protocol mismatch",
          "error",
        ),
      ],
    );
  }

  function unsupported(request) {
    return makeResponse(request.type, request, { supported: false }, [
      createDiagnostic(
        DIAGNOSTIC_CODES.BRIDGE_REQUEST_UNSUPPORTED,
        `unsupported request type: ${String(request.type)}`,
        "error",
      ),
    ]);
  }

  function handleRequest(request) {
    if (typeof isMetadataCheckerRequest !== "function" || !isMetadataCheckerRequest(request)) {
      if (
        typeof isMetadataCheckerRequestWithProtocolMismatch === "function" &&
        isMetadataCheckerRequestWithProtocolMismatch(request)
      ) {
        return protocolMismatch(request);
      }
      return null;
    }

    if (request.__metadata_checker_bridge_direction === RESPONSE_DIRECTION) {
      return null;
    }

    if (BRIDGE_REQUEST_TYPES.indexOf(request.type) < 0) {
      return unsupported(request);
    }

    if (request.type === "getBridgeStatus") {
      const payload = getBridgeStatusFromBridge();
      return makeResponse("getBridgeStatus", request, payload, payload.diagnostics);
    }

    if (request.type === "getPageContext") {
      const payload = getPageContextFromBridge();
      return makeResponse("getPageContext", request, payload, payload.diagnostics);
    }

    if (request.type === "getSelectionSnapshot") {
      const payload = getSelectionSnapshotFromBridge();
      return makeResponse("getSelectionSnapshot", request, {
        page_context: payload.page_context,
        selection: payload.selection,
        diagnostics: asDiagnostics(payload.diagnostics),
      }, payload.diagnostics);
    }

    const payload = analyzeCurrentSelectionFromBridge();
    return makeResponse("analyzeCurrentSelection", request, payload, payload.diagnostics);
  }

  function sendResponse(response) {
    if (!isObject(response)) {
      return;
    }
    const message = {
      ...response,
      __metadata_checker_bridge_token: response.__metadata_checker_bridge_token,
      __metadata_checker_bridge_direction: RESPONSE_DIRECTION,
      __metadata_checker_bridge_source: "page-script",
    };
    if (typeof root.postMessage === "function") {
      root.postMessage(message, "*");
      return;
    }
    if (root.document && typeof root.document.dispatchEvent === "function") {
      const event = root.document.createEvent("CustomEvent");
      event.initCustomEvent(BRIDGE_EVENT_NAME, false, false, message);
      root.document.dispatchEvent(event);
    }
  }

  function onMessage(event) {
    const response = handleRequest(event?.data);
    if (response) {
      Promise.resolve(response).then(sendResponse);
    }
  }

  function onReadyBridgeEvent() {
    const payload = getBridgeStatusFromBridge();
    if (typeof root.postMessage !== "function") {
      return;
    }
    root.postMessage(
      {
        type: "bridgeReady",
        protocol: BRIDGE_PROTOCOL,
        request_id: `bridge-ready-${Date.now()}`,
        payload,
        diagnostics: asDiagnostics(payload.diagnostics),
        __metadata_checker_bridge_token: null,
        __metadata_checker_bridge_direction: RESPONSE_DIRECTION,
        __metadata_checker_bridge_source: "page-script",
      },
      "*",
    );
  }

  function announceBridgeWhenReady(attempt = 0) {
    if (getBridge()) {
      onReadyBridgeEvent();
      return;
    }
    if (attempt < 50 && typeof root.setTimeout === "function") {
      root.setTimeout(() => announceBridgeWhenReady(attempt + 1), 100);
    }
  }

  function install() {
    if (typeof root.addEventListener !== "function") {
      return { installed: false, reason: "no-message-listener" };
    }
    root.addEventListener("message", onMessage);
    if (BRIDGE_READY_EVENT_NAME && typeof root.document?.addEventListener === "function") {
      root.document.addEventListener(BRIDGE_READY_EVENT_NAME, onReadyBridgeEvent);
    }
    announceBridgeWhenReady();
    return { installed: true };
  }

  function uninstall() {
    if (typeof root.removeEventListener === "function") {
      root.removeEventListener("message", onMessage);
      if (BRIDGE_READY_EVENT_NAME && typeof root.document?.removeEventListener === "function") {
        root.document.removeEventListener(BRIDGE_READY_EVENT_NAME, onReadyBridgeEvent);
      }
    }
  }

  root.__metadata_checker_page_script__ = {
    install,
    uninstall,
    getBridgeStatusFromBridge,
    getPageContextFromBridge,
    getSelectionSnapshotFromBridge,
    analyzeCurrentSelectionFromBridge,
    handleRequest,
    announceBridgeWhenReady,
  };
  writeMarker("extension-page-script", "loaded");

  if (root.__metadata_checker_page_script_auto_install !== false) {
    install();
    writeMarker("extension-page-script-status", "installed");
  }
})(typeof globalThis === "undefined" ? undefined : globalThis);

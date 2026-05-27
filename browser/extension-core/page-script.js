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
  const DEFAULT_DIAGNOSTIC_CODES = {
    PROTOCOL_MISMATCH: "METADATA_CHECKER_PROTOCOL_MISMATCH",
    BRIDGE_MISSING: "METADATA_CHECKER_BRIDGE_MISSING",
    BRIDGE_REQUEST_UNSUPPORTED: "METADATA_CHECKER_REQUEST_UNSUPPORTED",
    BRIDGE_REQUEST_FAILED: "METADATA_CHECKER_REQUEST_FAILED",
    SELECTION_BRIDGE_MISSING: "METADATA_CHECKER_SELECTION_BRIDGE_MISSING",
    SELECTION_PATCH_FAILED: "METADATA_CHECKER_SELECTION_PATCH_FAILED",
  };
  const DIAGNOSTIC_CODES = {
    ...DEFAULT_DIAGNOSTIC_CODES,
    ...(protocol.DIAGNOSTIC_CODES || {}),
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
  const SELECTION_CHANGED_EVENT_TYPE = "metadata-checker-selection-changed";
  const SELECTION_CHANGE_DEBOUNCE_MS = 150;

  const SELECTION_BRIDGE_MARKER = "selection-bridge";
  const SELECTION_BRIDGE_DIAGNOSTIC_MARKER = "selection-bridge-diagnostic";

  const SELECTION_FIELD_DENYLIST = new Set([
    "raw_text",
    "rawText",
    "components",
    "component_json",
    "componentJson",
    "raw_component",
    "rawComponent",
    "password",
    "token",
    "cookie",
    "canvas",
  ]);

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

  function getBridgeContext() {
    const bridge = getBridge();
    if (!bridge) {
      return null;
    }
    const result = safeInvoke(bridge, "getPageContext", null);
    const context = result.payload || {};
    return isObject(context) ? context : {};
  }

  function extractSourcePathFromContext(context) {
    if (!isObject(context)) {
      return null;
    }
    if (isObject(context.page_context)) {
      const sourcePath = context.page_context.source_path;
      if (typeof sourcePath === "string" && sourcePath.length > 0) {
        return sourcePath;
      }
    }
    const sourcePath = context.source_path;
    return typeof sourcePath === "string" && sourcePath.length > 0 ? sourcePath : null;
  }

  function writeSelectionBridgeDiagnostic(code, message, markerValue) {
    const severity = code === DIAGNOSTIC_CODES.SELECTION_BRIDGE_MISSING ? "warning" : "error";
    if (typeof markerValue === "string") {
      writeMarker(SELECTION_BRIDGE_DIAGNOSTIC_MARKER, `${code}:${markerValue}`);
    } else if (code) {
      writeMarker(SELECTION_BRIDGE_DIAGNOSTIC_MARKER, String(code));
    }
    if (!isObject(root.__metadata_checker_selection_bridge_state__)) {
      root.__metadata_checker_selection_bridge_state__ = { diagnostics: [] };
    }
    root.__metadata_checker_selection_bridge_state__.diagnostics.push(
      createDiagnostic(code, message, severity),
    );
  }

  function getBuilderFromDesigner(designer) {
    if (!isObject(designer)) {
      return null;
    }

    try {
      const currentPage = typeof designer.getCurrentPage === "function"
        ? designer.getCurrentPage()
        : null;
      if (currentPage && typeof currentPage.getBuilder === "function") {
        return currentPage.getBuilder();
      }

      const activeDesigner = typeof designer.getActiveDesigner === "function"
        ? designer.getActiveDesigner()
        : null;
      if (activeDesigner && typeof activeDesigner.getBuilder === "function") {
        return activeDesigner.getBuilder();
      }

      if (typeof designer.getBuilder === "function") {
        return designer.getBuilder();
      }
    } catch {
      return null;
    }
    return null;
  }

  function getBuilderFromContext(context) {
    if (!isObject(context)) {
      return null;
    }
    if (isObject(context.builder) && typeof context.builder === "object") {
      return context.builder;
    }
    if (isObject(context.designer) && getBuilderFromDesigner(context.designer)) {
      return getBuilderFromDesigner(context.designer);
    }
    if (isObject(context.page_context)) {
      const designerInContext = getBuilderFromDesigner(context.page_context)
        || getBuilderFromDesigner(context.page_context.designer)
        || (typeof context.page_context.getBuilder === "function"
          ? context.page_context.getBuilder()
          : null);
      if (designerInContext) {
        return designerInContext;
      }
    }
    if (typeof context.getBuilder === "function") {
      return context.getBuilder();
    }
    return getBuilderFromDesigner(context);
  }

  function asSelectionComponentSnapshot(component) {
    if (!isObject(component)) {
      return null;
    }
    const componentId = typeof component.getId === "function"
      ? component.getId()
      : component.id;
    const componentType = typeof component.getType === "function"
      ? component.getType()
      : component.type;
    if (typeof componentId !== "string") {
      return null;
    }
    const componentName = typeof component.getName === "function"
      ? component.getName()
      : component.name;
    const sanitized = {
      id: componentId,
      type: componentType,
      name: componentName,
    };
    for (const key of Object.keys(sanitized)) {
      if (!SELECTION_FIELD_DENYLIST.has(key) && sanitized[key] !== undefined) {
        continue;
      }
      delete sanitized[key];
    }
    return {
      id: sanitized.id,
      type: typeof sanitized.type === "string" ? sanitized.type : "",
      name: typeof sanitized.name === "string" ? sanitized.name : "",
    };
  }

  function captureSelectionPayload(sourceMethod) {
    const context = getBridgeContext();
    const sourcePath = extractSourcePathFromContext(context);
    const builder = root.__metadata_checker_selection_bridge_builder__ || getBuilderFromContext(context);
    if (!builder || typeof builder.getSelectedComponents !== "function") {
      return {
        source_path: sourcePath,
        selected_component_ids: [],
        selected_component_types: [],
        selected_count: 0,
        selection_source: sourceMethod,
        changed_at: Date.now(),
      };
    }

    let selectedComponents = [];
    try {
      selectedComponents = asArray(
        builder.getSelectedComponents(),
      );
    } catch {
      selectedComponents = [];
    }
    const snapshot = selectedComponents.map(asSelectionComponentSnapshot).filter(Boolean);
    const activeComponent = typeof builder.getSelectedComponent === "function"
      ? builder.getSelectedComponent()
      : selectedComponents[0];
    const activeSnapshot = asSelectionComponentSnapshot(activeComponent);
    return {
      source_path: sourcePath,
      selected_component_ids: snapshot.map((item) => item.id),
      selected_component_types: snapshot.map((item) => item.type),
      active_component_id: activeSnapshot?.id ?? snapshot[0]?.id ?? null,
      selected_count: snapshot.length,
      selection_source: sourceMethod,
      changed_at: Date.now(),
    };
  }

  let selectionChangeTimer = null;
  let pendingSelectionChange = null;

  function postSelectionChange(selectionSource, builder) {
    if (!builder || !selectionSource) {
      return;
    }
    let payload;
    try {
      payload = captureSelectionPayload(selectionSource);
    } catch (error) {
      writeSelectionBridgeDiagnostic(
        DIAGNOSTIC_CODES.SELECTION_PATCH_FAILED,
        error?.message || "failed to capture selection payload",
      );
      return;
    }
    if (typeof root.postMessage === "function") {
      root.postMessage({
        type: SELECTION_CHANGED_EVENT_TYPE,
        payload,
        selection_source: payload.selection_source,
        changed_at: payload.changed_at,
        __metadata_checker_bridge_source: "page-script",
        __metadata_checker_bridge_direction: "notification",
      }, "*");
    }
  }

  function dispatchSelectionChange(selectionSource, builder) {
    if (!builder || !selectionSource) {
      return;
    }
    pendingSelectionChange = { selectionSource, builder };
    if (selectionChangeTimer && typeof root.clearTimeout === "function") {
      root.clearTimeout(selectionChangeTimer);
    }
    if (typeof root.setTimeout !== "function") {
      postSelectionChange(selectionSource, builder);
      pendingSelectionChange = null;
      selectionChangeTimer = null;
      return;
    }
    selectionChangeTimer = root.setTimeout(() => {
      const pending = pendingSelectionChange;
      pendingSelectionChange = null;
      selectionChangeTimer = null;
      if (pending) {
        postSelectionChange(pending.selectionSource, pending.builder);
      }
    }, SELECTION_CHANGE_DEBOUNCE_MS);
  }

  function patchSelectionMethods(builder) {
    if (!builder || typeof builder !== "object") {
      return {
        installed: false,
        method: null,
      };
    }

    if (builder.__metadataCheckerSelectionBridgeInstalled) {
      return {
        installed: true,
        method: builder.__metadataCheckerSelectionBridgeMethod || null,
        alreadyInstalled: true,
      };
    }

    const doSelectedChange = builder.doSelectedChange;
    if (typeof doSelectedChange === "function" && doSelectedChange.__metadataCheckerSelectionBridgePatched) {
      builder.__metadataCheckerSelectionBridgeInstalled = true;
      builder.__metadataCheckerSelectionBridgeMethod = "doSelectedChange";
      return { installed: true, method: "doSelectedChange", alreadyInstalled: true };
    }

    if (typeof doSelectedChange === "function" && !doSelectedChange.__metadataCheckerSelectionBridgePatched) {
      const wrapped = function (...args) {
        const result = doSelectedChange.apply(this, args);
        dispatchSelectionChange("doSelectedChange", this);
        return result;
      };
      wrapped.__metadataCheckerSelectionBridgePatched = true;
      wrapped.__metadataCheckerSelectionBridgeSource = "doSelectedChange";
      builder.doSelectedChange = wrapped;
      builder.__metadataCheckerSelectionBridgeInstalled = true;
      builder.__metadataCheckerSelectionBridgeMethod = "doSelectedChange";
      return { installed: true, method: "doSelectedChange" };
    }

    const patchableMethods = [
      "selectComponents",
      "deselectComponents",
      "deselectAll",
    ];
    let patched = null;
    for (const method of patchableMethods) {
      const original = builder[method];
      if (typeof original !== "function" || original.__metadataCheckerSelectionBridgePatched) {
        continue;
      }
      const wrapped = function (...args) {
        const result = original.apply(this, args);
        dispatchSelectionChange(method, this);
        return result;
      };
      wrapped.__metadataCheckerSelectionBridgePatched = true;
      wrapped.__metadataCheckerSelectionBridgeSource = method;
      builder[method] = wrapped;
      patched ??= method;
      root.__metadata_checker_selection_bridge_methods__ = root.__metadata_checker_selection_bridge_methods__ || {};
      root.__metadata_checker_selection_bridge_methods__[method] = {
        original,
      };
    }
    if (patched === null) {
      return { installed: false, method: null };
    }
    builder.__metadataCheckerSelectionBridgeInstalled = true;
    builder.__metadataCheckerSelectionBridgeMethod = patched;
    return { installed: true, method: patched };
  }

  function installSelectionBridge() {
    const context = getBridgeContext();
    const bridge = getBridge();
    const bridgeBuilder = typeof bridge?.getSelectionBridgeBuilder === "function"
      ? bridge.getSelectionBridgeBuilder()
      : null;
    const builder = bridgeBuilder || getBuilderFromContext(context);
    const sourcePath = extractSourcePathFromContext(context);
    if (!builder) {
      writeSelectionBridgeDiagnostic(
        DIAGNOSTIC_CODES.SELECTION_BRIDGE_MISSING,
        "metadata checker selection bridge cannot locate superpage builder",
        "builder_missing",
      );
      writeMarker(SELECTION_BRIDGE_MARKER, "missing");
      root.__metadata_checker_selection_bridge_builder__ = null;
      root.__metadata_checker_selection_bridge_source__ = null;
      return { installed: false, reason: "builder_missing" };
    }

    if (
      root.__metadata_checker_selection_bridge_builder__ === builder
      && builder.__metadataCheckerSelectionBridgeInstalled
    ) {
      writeMarker(SELECTION_BRIDGE_MARKER, "installed");
      return {
        installed: true,
        method: builder.__metadataCheckerSelectionBridgeMethod || null,
        alreadyInstalled: true,
      };
    }

    root.__metadata_checker_selection_bridge_builder__ = builder;
    root.__metadata_checker_selection_bridge_source__ = context;
    const patchResult = patchSelectionMethods(builder);
    if (!patchResult.installed) {
      writeSelectionBridgeDiagnostic(
        DIAGNOSTIC_CODES.SELECTION_PATCH_FAILED,
        "metadata checker selection bridge did not find a selectable method",
      );
      writeMarker(SELECTION_BRIDGE_MARKER, "missing");
      root.__metadata_checker_selection_bridge_builder__ = null;
      root.__metadata_checker_selection_bridge_source__ = null;
      return {
        installed: false,
        reason: "selection_methods_missing",
        method: patchResult.method,
      };
    }

    writeMarker(SELECTION_BRIDGE_DIAGNOSTIC_MARKER, `installed:${sourcePath || "unknown"}`);
    writeMarker(SELECTION_BRIDGE_MARKER, "installed");
    return { installed: true, method: patchResult.method };
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
    installSelectionBridge();
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
    installSelectionBridge,
    SELECTION_CHANGE_DEBOUNCE_MS,
  };
  writeMarker("extension-page-script", "loaded");

  if (root.__metadata_checker_page_script_auto_install !== false) {
    install();
    writeMarker("extension-page-script-status", "installed");
  }
})(typeof globalThis === "undefined" ? undefined : globalThis);

/* M43 Chromium content bootstrap */

(function installMetadataCheckerChromiumBridge(root) {
  "use strict";

  const runtime = typeof chrome !== "undefined" ? chrome.runtime : null;

  const PANEL_SCRIPT_PATHS = [
    "extension-core/bridge-protocol.js",
    "extension-core/runtime-adapter.js",
    "extension-core/page-script.js",
  ];

  const sharedState = root.__metadata_checker_chromium_content_state__
    || (root.__metadata_checker_chromium_content_state__ = {
      panelHost: null,
      mounted: false,
      mountResult: null,
      runtimeListenerBound: false,
      windowListenerBound: false,
    });

  const BRIDGE_READY_MARKER = "bridgeReady";
  const SELECTION_CHANGED_MARKER = "metadata-checker-selection-changed";

  function isObject(value) {
    return value !== null && typeof value === "object";
  }

  function asArray(value) {
    return Array.isArray(value) ? value : [];
  }

  function asString(value) {
    return typeof value === "string" ? value : "";
  }

  function asDiagnostics(value) {
    return asArray(value).filter(
      (item) => isObject(item) && typeof item.code === "string" && typeof item.message === "string",
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

  function stableDiagnostic(code, message, severity = "warning") {
    return {
      severity,
      code,
      message,
    };
  }

  function getPanelHostFactory() {
    if (typeof root.__metadata_checker_panel_host_factory__ === "function") {
      return root.__metadata_checker_panel_host_factory__;
    }
    return null;
  }

  function getPanelHost() {
    if (sharedState.panelHost) {
      return sharedState.panelHost;
    }
    const factory = getPanelHostFactory();
    const host = typeof factory === "function" ? factory() : null;
    sharedState.panelHost = host;
    return host;
  }

  function mountPanelHost() {
    const host = getPanelHost();
    if (!host || typeof host.mountPanel !== "function") {
      return {
        mounted: false,
        error: {
          status: "error",
          diagnostics: [
            stableDiagnostic("PANEL_HOST_FACTORY_MISSING", "panel host factory is unavailable"),
          ],
        },
      };
    }

    const result = host.mountPanel({
      rootDocument: root.document,
    });
    if (result?.mounted) {
      sharedState.mounted = true;
    }
    sharedState.mountResult = result;
    return result;
  }

  function writeBridgeProbeMarkers(response) {
    const payload = response?.payload || response || {};
    const pageContext = payload.page_context || {};
    const selection = payload.selection || {};
    const diagnostics = asDiagnostics(response?.diagnostics).concat(asDiagnostics(payload.diagnostics));
    writeMarker("extension-bridge-request", payload.bridge_detected === false ? "missing" : "ready");
    writeMarker("extension-bridge-source-path", pageContext.source_path || pageContext.file_id || "");
    writeMarker(
      "extension-bridge-selection-count",
      String(Array.isArray(selection.selected_component_ids) ? selection.selected_component_ids.length : 0),
    );
    writeMarker("extension-bridge-diagnostic-code", diagnostics[0]?.code || "");
  }

  function applyPanelStateFromBridge(response, requestType) {
    const host = getPanelHost();
    if (!host || typeof host.updatePanel !== "function") {
      return;
    }

    const payload = response?.payload || {};
    const diagnostics = asDiagnostics(response?.diagnostics);
    const bridgeMissing = payload.bridge_detected === false
      || diagnostics.some((item) => item.code === "METADATA_CHECKER_BRIDGE_MISSING")
      || diagnostics.some((item) => item.code === "METADATA_CHECKER_CONTENT_BRIDGE_MISSING");

    if (payload.selection && typeof host.updateSelection === "function") {
      host.updateSelection(payload.selection);
    }

    if (requestType === "analyzeCurrentSelection") {
      const status = bridgeMissing || payload.supported === false ? "error" : "ready";
      host.updatePanel({
        status,
        target: payload.target ?? payload.page_context?.source_path ?? null,
        items: asArray(payload.items),
        diagnostics,
      });
      if (typeof host.setPanelStatus === "function") {
        host.setPanelStatus(status);
      }
      return;
    }

    host.updatePanel({
      status: bridgeMissing ? "error" : "ready",
      target: payload.target ?? payload.page_context?.source_path ?? null,
      items: asArray(payload.items),
      diagnostics,
    });
    if (typeof host.setPanelStatus === "function") {
      host.setPanelStatus(bridgeMissing ? "error" : "ready");
    }
  }

  function forwardStatus(message) {
    if (!runtime || typeof runtime.sendMessage !== "function") {
      return;
    }
    try {
      runtime.sendMessage({
        type: "metadata-checker-bridge-status",
        payload: message,
      });
    } catch {
      // Content scripts may run before the extension service worker is ready.
    }
  }

  async function requestPageBridge(requestType) {
    const bridge = root.__metadata_checker_content_bridge__;
    if (!bridge || typeof bridge.request !== "function") {
      const missing = {
        payload: {
          supported: false,
          bridge_detected: false,
        },
        diagnostics: [
          stableDiagnostic(
            "METADATA_CHECKER_CONTENT_BRIDGE_MISSING",
            "metadata checker content bridge is unavailable",
          ),
        ],
      };
      writeBridgeProbeMarkers(missing);
      applyPanelStateFromBridge(missing, requestType);
      return missing;
    }

    const response = await bridge.request(requestType);
    writeBridgeProbeMarkers(response);
    applyPanelStateFromBridge(response, requestType);
    return response;
  }

  function normalizeMessageFromPage(message) {
    if (!isObject(message)) {
      return false;
    }
    if (message.__metadata_checker_bridge_source !== "page-script") {
      return false;
    }
    if (message.type === SELECTION_CHANGED_MARKER) {
      return true;
    }
    if (message.type === BRIDGE_READY_MARKER) {
      return true;
    }
    if (message.__metadata_checker_bridge_direction !== "response") {
      return false;
    }
    if (typeof message.request_id !== "string") {
      return false;
    }
    return true;
  }

  function markSelectionFromPageMessage(message) {
    if (!isObject(message) || message.type !== SELECTION_CHANGED_MARKER) {
      return;
    }
    if (typeof message.payload !== "object") {
      return;
    }
    const host = getPanelHost();
    if (!host || typeof host.updateSelection !== "function") {
      return;
    }
    host.updateSelection(message.payload);
    writeMarker("extension-selection-event", "received");
    writeMarker("extension-selection-source", asString(message.payload.selection_source));
    writeMarker(
      "extension-selection-count",
      String(Array.isArray(message.payload.selected_component_ids) ? message.payload.selected_component_ids.length : 0),
    );
    writeMarker("extension-selection-active", asString(message.payload.active_component_id));
    writeMarker("extension-selection-changed-at", String(message.payload.changed_at || ""));
  }

  function normalizeAction(action) {
    if (typeof action !== "string") {
      return null;
    }
    if (action === "openPanel" || action === "open-panel" || action === "showPanel") {
      return "openPanel";
    }
    if (action === "hidePanel" || action === "hide-panel") {
      return "hidePanel";
    }
    if (action === "togglePanel" || action === "toggle-panel") {
      return "togglePanel";
    }
    if (action === "refreshBridge" || action === "refreshBridgeStatus" || action === "refresh") {
      return "refreshBridge";
    }
    if (action === "analyzeCurrentSelection" || action === "analyze") {
      return "analyzeCurrentSelection";
    }
    return null;
  }

  function mountIfNeeded() {
    if (!sharedState.mounted) {
      mountPanelHost();
    }
    return getPanelHost();
  }

  function handlePanelCommand(host, command) {
    if (!host) {
      return {
        ok: false,
        diagnostics: [stableDiagnostic("PANEL_HOST_NOT_AVAILABLE", "panel host is unavailable")],
      };
    }
    if (command === "openPanel") {
      const result = host.togglePanel ? host.togglePanel(true) : { visible: false };
      return { ok: result.visible === true, visible: result.visible };
    }
    if (command === "hidePanel") {
      const result = host.togglePanel ? host.togglePanel(false) : { visible: false };
      return { ok: result.visible === false, visible: result.visible };
    }
    if (command === "togglePanel") {
      const result = host.togglePanel ? host.togglePanel() : { visible: false };
      return { ok: true, visible: result.visible };
    }
    return {
      ok: false,
      diagnostics: [
        stableDiagnostic(
          "METADATA_CHECKER_PANEL_COMMAND_UNSUPPORTED",
          `unsupported panel command: ${String(command)}`,
        ),
      ],
    };
  }

  function wrapPanelCommandResponse(action, payload) {
    return {
      action,
      ...(isObject(payload) ? payload : {}),
    };
  }

  if (runtime && typeof runtime.onMessage?.addListener === "function") {
    if (!sharedState.runtimeListenerBound) {
      sharedState.runtimeListenerBound = true;
      runtime.onMessage.addListener((message, _sender, sendResponse) => {
      const requestType = message?.type === "metadata-checker-tab-request"
        ? (typeof message.request_type === "string" ? message.request_type : null)
        : normalizeAction(message?.action || message?.type);
      const mappedRequestType = requestType === "refreshBridge"
        ? "getBridgeStatus"
        : requestType;
      if (typeof requestType !== "string" || requestType.length === 0) {
        return false;
      }

      const host = mountIfNeeded();
      if (
        requestType === "openPanel"
        || requestType === "hidePanel"
        || requestType === "togglePanel"
      ) {
        sendResponse(wrapPanelCommandResponse(requestType, handlePanelCommand(host, requestType)));
        return false;
      }

      if (message?.type === "metadata-checker-tab-request") {
        requestPageBridge(mappedRequestType).then((response) => {
          forwardStatus(response);
          sendResponse(response);
        }).catch((error) => {
          const failed = {
            payload: {
              supported: false,
            },
            diagnostics: [
              stableDiagnostic(
                "METADATA_CHECKER_TAB_REQUEST_FAILED",
                error?.message || "metadata checker tab request failed",
                "error",
              ),
            ],
          };
          writeBridgeProbeMarkers(failed);
          applyPanelStateFromBridge(failed, requestType);
          sendResponse(failed);
        });
        return true;
      }

      if (requestType === "refreshBridge") {
        requestPageBridge("getBridgeStatus").then((response) => {
          sendResponse(wrapPanelCommandResponse("refreshBridge", response));
        }).catch((error) => {
          sendResponse(
            wrapPanelCommandResponse("refreshBridge", {
              ok: false,
              diagnostics: [stableDiagnostic("METADATA_CHECKER_PANEL_COMMAND_FAILED", error?.message || "refresh bridge failed", "error")],
            }),
          );
        });
        return true;
      }

      if (requestType === "analyzeCurrentSelection") {
        requestPageBridge("analyzeCurrentSelection").then((response) => {
          sendResponse(wrapPanelCommandResponse("analyzeCurrentSelection", response));
        }).catch((error) => {
          sendResponse(
            wrapPanelCommandResponse("analyzeCurrentSelection", {
              ok: false,
              diagnostics: [stableDiagnostic("METADATA_CHECKER_PANEL_COMMAND_FAILED", error?.message || "analyze current selection failed", "error")],
            }),
          );
        });
        return true;
      }

      sendResponse(wrapPanelCommandResponse(requestType, handlePanelCommand(host, requestType)));
      return false;
      });
    }
  }

  if (typeof root.addEventListener === "function") {
    if (!sharedState.windowListenerBound) {
      sharedState.windowListenerBound = true;
      const onWindowMessage = (event) => {
      const data = event?.data;
      if (!normalizeMessageFromPage(data)) {
        return;
      }

      if (data.type === SELECTION_CHANGED_MARKER) {
        markSelectionFromPageMessage(data);
        return;
      }

      if (data.type === BRIDGE_READY_MARKER) {
        writeBridgeProbeMarkers(data);
        applyPanelStateFromBridge(data, "getBridgeStatus");
        return;
      }
      if (typeof data.type === "string" && typeof data.request_id === "string") {
        writeBridgeProbeMarkers(data);
        forwardStatus(data);
        applyPanelStateFromBridge(data, data.type);
      }
    };

    root.addEventListener("message", onWindowMessage);
    root.addEventListener("metadata-checker-bridge-message", onWindowMessage);
    }
  }

  writeMarker("extension-content", "loaded");
  root.__metadata_checker_resolve_asset_url = function resolveAssetUrl(path) {
    if (runtime && typeof runtime.getURL === "function") {
      return runtime.getURL(path);
    }
    return path;
  };

  root.__metadata_checker_injected_script_paths = PANEL_SCRIPT_PATHS;
  mountPanelHost();
})(typeof globalThis === "undefined" ? undefined : globalThis);

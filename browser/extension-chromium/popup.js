/* M44.5 Chromium extension popup */

const requestTypeByAction = {
  open: "openPanel",
  hide: "hidePanel",
  refresh: "refreshBridge",
  analyze: "analyzeCurrentSelection",
};

const fields = {
  bridge: document.querySelector('[data-field="bridge"]'),
  source: document.querySelector('[data-field="source"]'),
  selection: document.querySelector('[data-field="selection"]'),
  diagnostic: document.querySelector('[data-field="diagnostic"]'),
};

function setText(node, value) {
  if (node) {
    node.textContent = value == null || value === "" ? "unknown" : String(value);
  }
}

function summarizeSelection(selection) {
  const ids = Array.isArray(selection?.selected_component_ids)
    ? selection.selected_component_ids
    : [];
  if (ids.length === 0) {
    return "empty";
  }
  return `${ids.length}: ${ids.slice(0, 3).join(", ")}`;
}

function normalizeError(error, fallbackCode, fallbackMessage) {
  if (error && typeof error === "object" && typeof error.code === "string" && typeof error.message === "string") {
    return {
      severity: error.severity || "error",
      code: error.code,
      message: error.message,
    };
  }
  return {
    severity: "error",
    code: fallbackCode,
    message: fallbackMessage,
  };
}

function firstDiagnostic(payload) {
  if (Array.isArray(payload?.diagnostics) && payload.diagnostics.length > 0) {
    return payload.diagnostics[0];
  }
  if (Array.isArray(payload?.payload?.diagnostics) && payload.payload.diagnostics.length > 0) {
    return payload.payload.diagnostics[0];
  }
  return null;
}

function renderStatus(state) {
  const status = state?.last_bridge_status?.payload || state?.last_bridge_status || {};
  const pageContext = status.page_context || status.payload?.page_context || {};
  const selection = status.selection || status.payload?.selection || {};
  const diagnostic = state?.last_diagnostic || status.diagnostics?.[0] || status.payload?.diagnostics?.[0] || null;

  setText(fields.bridge, status.bridge_detected === false ? "missing" : "ready");
  setText(fields.source, pageContext.source_path || pageContext.file_id || "unknown");
  setText(fields.selection, summarizeSelection(selection));
  setText(fields.diagnostic, diagnostic ? JSON.stringify(diagnostic, null, 2) : "");
}

function makeMissingTabDiagnostic(message) {
  return {
    severity: "warning",
    code: "METADATA_CHECKER_ACTIVE_TAB_MISSING",
    message,
  };
}

function getChromeApi() {
  return typeof chrome === "undefined" ? null : chrome;
}

function getTabApi() {
  const api = getChromeApi();
  return api?.tabs;
}

async function requestActiveTab(requestType) {
  const tabs = getTabApi();
  if (!tabs || typeof tabs.query !== "function" || typeof tabs.sendMessage !== "function") {
    throw {
      severity: "error",
      code: "METADATA_CHECKER_TABS_API_MISSING",
      message: "chrome.tabs query/sendMessage is unavailable",
    };
  }
  const matched = await tabs.query({ active: true, currentWindow: true });
  const tabId = Array.isArray(matched) && matched.length > 0 ? matched[0]?.id : null;
  if (!tabId || typeof tabId !== "number") {
    throw makeMissingTabDiagnostic("active tab is unavailable");
  }
  return chrome.tabs.sendMessage(tabId, {
    type: "metadata-checker-tab-request",
    request_type: requestType,
  });
}

function requestBridgeStatus() {
  return requestActiveTab("getBridgeStatus");
}

function requestAnalyzeCurrentSelection() {
  return requestActiveTab("analyzeCurrentSelection");
}

function sendPopupRequest(requestType, onErrorCode, onErrorMessage) {
  requestActiveTab(requestType)
    .then((response) => {
      renderStatus({
        last_bridge_status: response || { diagnostics: [makeMissingTabDiagnostic(onErrorMessage)] },
        last_diagnostic: firstDiagnostic(response) || null,
      });
    })
    .catch((error) => {
      renderStatus({
        last_diagnostic: normalizeError(
          error,
          onErrorCode,
          onErrorMessage || "request failed",
        ),
      });
    });
}

function requestOpenPanel() {
  sendPopupRequest(requestTypeByAction.open, "METADATA_CHECKER_POPUP_OPEN_PANEL_FAILED", "openPanel request failed");
}

function requestHidePanel() {
  sendPopupRequest(requestTypeByAction.hide, "METADATA_CHECKER_POPUP_HIDE_PANEL_FAILED", "hidePanel request failed");
}

function requestRefreshBridge() {
  requestActiveTab(requestTypeByAction.refresh)
    .then((response) => {
      renderStatus({
        last_bridge_status: response || {},
        last_diagnostic: firstDiagnostic(response) || null,
      });
    })
    .catch((error) => {
      renderStatus({
        last_diagnostic: normalizeError(
          error,
          "METADATA_CHECKER_POPUP_REFRESH_BRIDGE_FAILED",
          "refresh bridge request failed",
        ),
      });
    });
}

function requestAnalyzeCurrentSelectionFromPopup() {
  requestAnalyzeCurrentSelection()
    .then((response) => {
      renderStatus({
        last_bridge_status: response || {},
        last_diagnostic: firstDiagnostic(response) || null,
      });
    })
    .catch((error) => {
      renderStatus({
        last_diagnostic: normalizeError(
          error,
          "METADATA_CHECKER_POPUP_ANALYZE_FAILED",
          "analyze failed",
        ),
      });
    });
}

function loadStatus() {
  requestBridgeStatus()
    .then((response) => {
      renderStatus({
        last_bridge_status: response || {},
        last_diagnostic: firstDiagnostic(response) || null,
      });
    })
    .catch((error) => {
      renderStatus({
        last_diagnostic: normalizeError(
          error,
          "METADATA_CHECKER_POPUP_STATUS_FAILED",
          "status failed",
        ),
      });
    });
}

document.querySelector('[data-action="open-panel"]')?.addEventListener("click", () => {
  requestOpenPanel();
});
document.querySelector('[data-action="hide-panel"]')?.addEventListener("click", () => {
  requestHidePanel();
});
document.querySelector('[data-action="refresh-bridge"]')?.addEventListener("click", () => {
  requestRefreshBridge();
});
document.querySelector('[data-action="analyze"]')?.addEventListener("click", () => {
  requestAnalyzeCurrentSelectionFromPopup();
});

loadStatus();

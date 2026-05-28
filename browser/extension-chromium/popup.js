/* M44.5 Chromium extension popup */

const requestTypeByAction = {
  open: "openPanel",
  hide: "hidePanel",
  refresh: "refreshBridge",
  analyze: "analyzeCurrentSelection",
  retry: "retryCurrentSelection",
};

const fields = {
  bridge: document.querySelector('[data-field="bridge"]'),
  source: document.querySelector('[data-field="source"]'),
  selection: document.querySelector('[data-field="selection"]'),
  indexing: document.querySelector('[data-field="indexing"]'),
  progress: document.querySelector('[data-field="progress"]'),
  current: document.querySelector('[data-field="current"]'),
  cache: document.querySelector('[data-field="cache"]'),
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

function pickBackgroundState(state) {
  return state?.background || state?.state?.background || {};
}

function pickVisibleIndex(state) {
  return state?.visible_index || state?.state?.visible_index || {};
}

function pickCacheStats(state) {
  return state?.cache_stats || state?.state?.cache_stats || {};
}

function summarizeProgress(background, visibleIndex) {
  const discovered = Array.isArray(visibleIndex.files) ? visibleIndex.files.length : 0;
  const processed = background.processed ?? 0;
  const total = background.total ?? visibleIndex.analyzable_count ?? 0;
  const failed = background.failed ?? 0;
  return `${processed}/${total} processed, ${failed} failed, ${discovered} discovered`;
}

function renderBackgroundState(state) {
  const background = pickBackgroundState(state);
  const visibleIndex = pickVisibleIndex(state);
  const cacheStats = pickCacheStats(state);
  setText(fields.indexing, background.indexing_status || background.status || visibleIndex.status || "unknown");
  setText(fields.progress, summarizeProgress(background, visibleIndex));
  setText(fields.current, background.current_source_path || background.last_processed_source_path || "unknown");
  setText(fields.cache, `${cacheStats.hits ?? 0} hits, ${cacheStats.misses ?? 0} misses`);
}

function renderStatus(state = {}) {
  const status = state.last_bridge_status?.payload || state.last_bridge_status || {};
  const pageContext = status.page_context || status.payload?.page_context || {};
  const selection = status.selection || status.payload?.selection || {};
  const diagnostic = state?.last_diagnostic || status.diagnostics?.[0] || status.payload?.diagnostics?.[0] || null;

  setText(fields.bridge, status.bridge_detected === false ? "missing" : "ready");
  setText(fields.source, pageContext.source_path || pageContext.file_id || "unknown");
  setText(fields.selection, summarizeSelection(selection));
  renderBackgroundState(state);
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

function getRuntimeApi() {
  const api = getChromeApi();
  return api?.runtime;
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

function requestRetryCurrentSelection() {
  return requestActiveTab("retryCurrentSelection");
}

async function requestBackgroundState() {
  const runtime = getRuntimeApi();
  if (!runtime || typeof runtime.sendMessage !== "function") {
    return null;
  }
  return runtime.sendMessage({ type: "metadata-checker-popup-status" });
}

async function requestBackgroundProcess() {
  const runtime = getRuntimeApi();
  if (!runtime || typeof runtime.sendMessage !== "function") {
    throw {
      severity: "error",
      code: "METADATA_CHECKER_RUNTIME_API_MISSING",
      message: "chrome.runtime sendMessage is unavailable",
    };
  }
  return runtime.sendMessage({
    type: "metadata-checker-background-process",
    payload: { limit: 1, max_concurrency: 1 },
  });
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

function requestRetryCurrentSelectionFromPopup() {
  requestRetryCurrentSelection()
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
          "METADATA_CHECKER_POPUP_RETRY_SELECTION_FAILED",
          "retry current selection failed",
        ),
      });
    });
}

function requestBackgroundProcessFromPopup() {
  requestBackgroundProcess()
    .then((response) => {
      renderStatus({
        state: response || {},
        last_diagnostic: firstDiagnostic(response) || null,
      });
    })
    .catch((error) => {
      renderStatus({
        last_diagnostic: normalizeError(
          error,
          "METADATA_CHECKER_POPUP_BACKGROUND_PROCESS_FAILED",
          "background process failed",
        ),
      });
    });
}

function loadStatus() {
  Promise.allSettled([requestBridgeStatus(), requestBackgroundState()])
    .then(([bridgeResult, backgroundResult]) => {
      const bridge = bridgeResult.status === "fulfilled" ? bridgeResult.value : {};
      const background = backgroundResult.status === "fulfilled" ? backgroundResult.value : {};
      const bridgeError = bridgeResult.status === "rejected"
        ? normalizeError(
          bridgeResult.reason,
          "METADATA_CHECKER_POPUP_STATUS_FAILED",
          "status failed",
        )
        : null;
      const backgroundError = backgroundResult.status === "rejected"
        ? normalizeError(
          backgroundResult.reason,
          "METADATA_CHECKER_POPUP_BACKGROUND_STATUS_FAILED",
          "background status failed",
        )
        : null;
      renderStatus({
        last_bridge_status: bridge || {},
        state: background?.state || background || {},
        last_diagnostic: firstDiagnostic(bridge) || firstDiagnostic(background) || bridgeError || backgroundError || null,
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
document.querySelector('[data-action="process-background"]')?.addEventListener("click", () => {
  requestBackgroundProcessFromPopup();
});
document.querySelector('[data-action="retry-current-selection"]')?.addEventListener("click", () => {
  requestRetryCurrentSelectionFromPopup();
});
document.querySelector('[data-action="analyze"]')?.addEventListener("click", () => {
  requestAnalyzeCurrentSelectionFromPopup();
});

loadStatus();

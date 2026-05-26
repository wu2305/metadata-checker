/* M43 Chromium extension popup */

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

function renderStatus(state) {
  const status = state?.last_bridge_status?.payload || state?.last_bridge_status || {};
  const pageContext = status.page_context || status.payload?.page_context || {};
  const selection = status.selection || status.payload?.selection || {};
  const diagnostic =
    state?.last_diagnostic ||
    status.diagnostics?.[0] ||
    status.payload?.diagnostics?.[0] ||
    null;

  setText(fields.bridge, status.bridge_detected === false ? "missing" : "ready");
  setText(fields.source, pageContext.source_path || pageContext.file_id || "unknown");
  setText(fields.selection, summarizeSelection(selection));
  setText(fields.diagnostic, diagnostic ? JSON.stringify(diagnostic, null, 2) : "");
}

async function requestActiveTab(requestType) {
  const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
  const tabId = tabs?.[0]?.id;
  if (!tabId) {
    return {
      diagnostics: [
        {
          severity: "warning",
          code: "METADATA_CHECKER_ACTIVE_TAB_MISSING",
          message: "active tab is unavailable",
        },
      ],
    };
  }
  return chrome.tabs.sendMessage(tabId, {
    type: "metadata-checker-tab-request",
    request_type: requestType,
  });
}

async function loadStatus() {
  if (!chrome?.runtime?.sendMessage) {
    renderStatus({
      last_diagnostic: {
        severity: "error",
        code: "METADATA_CHECKER_EXTENSION_RUNTIME_MISSING",
        message: "chrome.runtime is unavailable",
      },
    });
    return;
  }
  try {
    const tabStatus = await requestActiveTab("getBridgeStatus");
    renderStatus({
      last_bridge_status: tabStatus,
      last_diagnostic: tabStatus?.diagnostics?.[0] || null,
    });
  } catch (_error) {
    const response = await chrome.runtime.sendMessage({
      type: "metadata-checker-popup-status",
    });
    renderStatus(response?.state || {});
  }
}

async function analyzeCurrentSelection() {
  const result = await requestActiveTab("analyzeCurrentSelection");
  renderStatus({
    last_bridge_status: result,
    last_diagnostic: result?.diagnostics?.[0] || null,
  });
}

document.querySelector('[data-action="analyze"]')?.addEventListener("click", () => {
  analyzeCurrentSelection().catch((error) => {
    renderStatus({
      last_diagnostic: {
        severity: "error",
        code: "METADATA_CHECKER_POPUP_ANALYZE_FAILED",
        message: error?.message || "analyze failed",
      },
    });
  });
});

loadStatus().catch((error) => {
  renderStatus({
    last_diagnostic: {
      severity: "error",
      code: "METADATA_CHECKER_POPUP_STATUS_FAILED",
      message: error?.message || "status failed",
    },
  });
});

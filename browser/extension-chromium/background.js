/* M43 Chromium extension service worker */

const state = {
  last_bridge_status: null,
  last_diagnostic: null,
  updated_at: null,
};

function normalizeDiagnostic(value) {
  if (!value || typeof value !== "object") {
    return null;
  }
  if (typeof value.code !== "string" || typeof value.message !== "string") {
    return null;
  }
  return {
    severity: value.severity || "warning",
    code: value.code,
    message: value.message,
  };
}

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (!message || typeof message !== "object") {
    return false;
  }

  if (message.type === "metadata-checker-bridge-status") {
    state.last_bridge_status = message.payload || null;
    state.last_diagnostic =
      normalizeDiagnostic(message.payload?.diagnostics?.[0]) || state.last_diagnostic;
    state.updated_at = Date.now();
    sendResponse({ ok: true });
    return false;
  }

  if (message.type === "metadata-checker-popup-status") {
    sendResponse({
      ok: true,
      state,
    });
    return false;
  }

  sendResponse({
    ok: false,
    diagnostics: [
      {
        severity: "warning",
        code: "METADATA_CHECKER_EXTENSION_UNSUPPORTED_MESSAGE",
        message: `unsupported extension message: ${String(message.type)}`,
      },
    ],
  });
  return false;
});

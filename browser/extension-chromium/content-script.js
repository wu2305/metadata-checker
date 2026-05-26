/* M43 Chromium content bootstrap */

(function installMetadataCheckerChromiumBridge(root) {
  "use strict";

  const runtime = typeof chrome !== "undefined" ? chrome.runtime : null;

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

  writeMarker("extension-content", "loaded");

  root.__metadata_checker_resolve_asset_url = function resolveAssetUrl(path) {
    if (runtime && typeof runtime.getURL === "function") {
      return runtime.getURL(path);
    }
    return path;
  };

  root.__metadata_checker_injected_script_paths = [
    "extension-core/bridge-protocol.js",
    "extension-core/runtime-adapter.js",
    "extension-core/page-script.js",
  ];

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

  function stableDiagnostic(code, message, severity = "warning") {
    return {
      severity,
      code,
      message,
    };
  }

  async function requestPageBridge(requestType) {
    const bridge = root.__metadata_checker_content_bridge__;
    if (!bridge || typeof bridge.request !== "function") {
      return {
        payload: { supported: false },
        diagnostics: [
          stableDiagnostic(
            "METADATA_CHECKER_CONTENT_BRIDGE_MISSING",
            "metadata checker content bridge is unavailable",
          ),
        ],
      };
    }
    return bridge.request(requestType || "getBridgeStatus", {});
  }

  if (runtime && typeof runtime.onMessage?.addListener === "function") {
    runtime.onMessage.addListener((message, _sender, sendResponse) => {
      if (!message || message.type !== "metadata-checker-tab-request") {
        return false;
      }
      requestPageBridge(message.request_type).then((response) => {
        forwardStatus(response);
        sendResponse(response);
      }).catch((error) => {
        sendResponse({
          payload: { supported: false },
          diagnostics: [
            stableDiagnostic(
              "METADATA_CHECKER_TAB_REQUEST_FAILED",
              error?.message || "metadata checker tab request failed",
              "error",
            ),
          ],
        });
      });
      return true;
    });
  }

  root.addEventListener("message", (event) => {
    const data = event?.data;
    if (
      data &&
      data.__metadata_checker_bridge_source === "page-script" &&
      data.__metadata_checker_bridge_direction === "response"
    ) {
      forwardStatus(data);
    }
  });
})(typeof globalThis === "undefined" ? window : globalThis);

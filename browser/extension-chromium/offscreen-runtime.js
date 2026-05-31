/* M45 offscreen WASM runtime host */

import initWasmRuntime, * as wasmModule from "./metadata_checker.js";

const MESSAGE_TYPE = "metadata-checker-offscreen-wasm-call";

let runtimePromise = null;

function stableDiagnostic(code, message, severity = "error") {
  return { code, message, severity };
}

async function ensureRuntime(wasmUrl) {
  if (!runtimePromise) {
    runtimePromise = initWasmRuntime(wasmUrl);
  }
  await runtimePromise;
}

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (!message || message.type !== MESSAGE_TYPE) {
    return false;
  }

  (async () => {
    const payload = message.payload || {};
    const method = typeof payload.method === "string" ? payload.method : "";
    const args = Array.isArray(payload.args) ? payload.args : [];
    await ensureRuntime(payload.wasm_url || chrome.runtime.getURL("metadata_checker_bg.wasm"));
    const fn = wasmModule[method];
    if (typeof fn !== "function") {
      sendResponse({
        ok: false,
        request_id: message.request_id || null,
        diagnostic: stableDiagnostic(
          "OFFSCREEN_WASM_METHOD_MISSING",
          `WASM runtime method missing: ${method}`,
        ),
      });
      return;
    }
    const result = await fn(...args);
    sendResponse({
      ok: true,
      request_id: message.request_id || null,
      result,
    });
  })().catch((error) => {
    sendResponse({
      ok: false,
      request_id: message.request_id || null,
      diagnostic: stableDiagnostic(
        "OFFSCREEN_WASM_CALL_FAILED",
        error?.message || "offscreen WASM call failed",
      ),
    });
  });

  return true;
});

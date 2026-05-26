/* M43 扩展 Content Script 与页面脚本桥接层 */

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
  const createRequestEnvelope = protocol.createRequestEnvelope;
  const createDiagnostic = protocol.createDiagnostic || ((code, message, severity = "error") => ({
    severity,
    code,
    message,
  }));

  const REQUEST_TIMEOUT_MS = 5000;
  const PENDING_REQUEST_PREFIX = "m43-extension-";
  const RESPONSE_DIRECTION = "response";
  const REQUEST_DIRECTION = "request";
  const BRIDGE_EVENT_NAME = "metadata-checker-bridge-message";

  let requestSeed = 0;
  const pendingRequests = new Map();
  let injected = false;
  let injecting = false;
  const sourceToken = `m43-${Date.now()}-${Math.random().toString(36).slice(2)}`;

  const injectedScriptPaths = root.__metadata_checker_injected_script_paths || [
    "extension-core/bridge-protocol.js",
    "extension-core/runtime-adapter.js",
    "extension-core/page-script.js",
  ];

  function isArray(value) {
    return Array.isArray(value);
  }

  function isObject(value) {
    return value !== null && typeof value === "object";
  }

  function isMetadataCheckerMessage(value) {
    if (!isObject(value) || typeof value.type !== "string") {
      return false;
    }
    if (typeof protocol.isMetadataCheckerRequest === "function") {
      return protocol.isMetadataCheckerRequest(value);
    }
    return (
      isObject(value.protocol) &&
      value.protocol.name === BRIDGE_PROTOCOL.name &&
      value.protocol.version === BRIDGE_PROTOCOL.version &&
      typeof value.request_id !== "undefined"
    );
  }

  function toMessage(payloadType, payload, requestId) {
    const envelope = typeof createRequestEnvelope === "function"
      ? createRequestEnvelope({
        type: payloadType,
        requestId,
        payload,
      })
      : {
          protocol: BRIDGE_PROTOCOL,
          request_id: requestId,
          type: payloadType,
          payload,
          diagnostics: [],
        };
    return {
      ...envelope,
      __metadata_checker_bridge_token: sourceToken,
    };
  }

  function nextRequestId() {
    requestSeed += 1;
    return `${PENDING_REQUEST_PREFIX}${requestSeed}`;
  }

  function rejectOnTimeout(requestId) {
    const pending = pendingRequests.get(requestId);
    if (!pending) {
      return;
    }
    pendingRequests.delete(requestId);
    pending.resolve({
      protocol: BRIDGE_PROTOCOL,
      request_id: requestId,
      type: pending.type,
      payload: { supported: false },
      diagnostics: [
        createDiagnostic("METADATA_CHECKER_BRIDGE_TIMEOUT", "metadata-checker bridge request timeout", "warning"),
      ],
    });
  }

  function createFallbackResolvers(requestId, requestType) {
    let resolve = null;
    let timeoutId = 0;
    const promise = new Promise((res) => {
      resolve = res;
      timeoutId = root.setTimeout
        ? root.setTimeout(() => rejectOnTimeout(requestId), REQUEST_TIMEOUT_MS)
        : 0;
    });
    return {
      promise,
      resolve: (response) => {
        if (timeoutId && root.clearTimeout) {
          root.clearTimeout(timeoutId);
        }
        resolve(response);
      },
      type: requestType,
    };
  }

  function resolveScriptUrl(path) {
    if (typeof root.__metadata_checker_resolve_asset_url === "function") {
      return root.__metadata_checker_resolve_asset_url(path);
    }
    return path;
  }

  function appendScript(path) {
    if (!root.document || typeof root.document.createElement !== "function") {
      return Promise.resolve();
    }

    return new Promise((resolve) => {
      const script = root.document.createElement("script");
      script.type = "text/javascript";
      script.src = resolveScriptUrl(path);
      script.async = false;
      const done = () => {
        if (typeof script.remove === "function") {
          script.remove();
        }
        resolve();
      };
      script.addEventListener("load", done, { once: true });
      script.addEventListener("error", done, { once: true });
      root.document.documentElement?.appendChild(script);
    });
  }

  function injectScripts() {
    if (injected) {
      return Promise.resolve();
    }
    if (injecting) {
      return new Promise((resolve) => {
        const wait = () => {
          if (injected) {
            resolve();
            return;
          }
          if (root.setTimeout) {
            root.setTimeout(wait, 10);
          } else {
            resolve();
          }
        };
        wait();
      });
    }
    if (!isArray(injectedScriptPaths) || injectedScriptPaths.length === 0) {
      injected = true;
      return Promise.resolve();
    }

    injecting = true;
    return Promise.all(injectedScriptPaths.map((path) => appendScript(path))).then(() => {
      injected = true;
      injecting = false;
    });
  }

  function normalizeMessageFromPage(message) {
    if (!isObject(message)) {
      return null;
    }
    if (message.__metadata_checker_bridge_direction === REQUEST_DIRECTION) {
      return null;
    }
    if (message.__metadata_checker_bridge_source !== "page-script") {
      return null;
    }
    if (message.__metadata_checker_bridge_token !== sourceToken) {
      return null;
    }
    if (isMetadataCheckerMessage(message) === false) {
      return null;
    }
    if (typeof message.request_id !== "string") {
      return null;
    }
    return message;
  }

  function onWindowMessage(event) {
    const message = normalizeMessageFromPage(event?.data);
    if (!message) {
      return;
    }
    const resolver = pendingRequests.get(message.request_id);
    if (!resolver) {
      return;
    }
    pendingRequests.delete(message.request_id);
    resolver.resolve(message);
  }

  function dispatchMessage(message) {
    const msg = {
      ...message,
      __metadata_checker_bridge_token: sourceToken,
      __metadata_checker_bridge_direction: REQUEST_DIRECTION,
      __metadata_checker_bridge_source: "content-script",
    };
    if (typeof root.postMessage === "function") {
      root.postMessage(msg, "*");
      return;
    }
    if (root.document && typeof root.document.dispatchEvent === "function") {
      const event = root.document.createEvent("CustomEvent");
      event.initCustomEvent(BRIDGE_EVENT_NAME, false, false, msg);
      root.document.dispatchEvent(event);
    }
  }

  async function request(type, payload = {}) {
    if (BRIDGE_REQUEST_TYPES.indexOf(type) < 0) {
      return {
        protocol: BRIDGE_PROTOCOL,
        request_id: nextRequestId(),
        type,
        payload: { supported: false },
        diagnostics: [
          createDiagnostic(
            "METADATA_CHECKER_REQUEST_UNSUPPORTED",
            `unsupported request type: ${type}`,
            "error",
          ),
        ],
      };
    }

    await injectScripts();
    const requestId = nextRequestId();
    const message = toMessage(type, payload, requestId);
    const record = createFallbackResolvers(requestId, type);
    pendingRequests.set(requestId, record);
    dispatchMessage(message);
    return record.promise;
  }

  function getBridgeStatus() {
    return request("getBridgeStatus", {});
  }

  function getPageContext() {
    return request("getPageContext", {});
  }

  function getSelectionSnapshot() {
    return request("getSelectionSnapshot", {});
  }

  function analyzeCurrentSelection() {
    return request("analyzeCurrentSelection", {});
  }

  root.__metadata_checker_content_bridge__ = {
    request,
    getBridgeStatus,
    getPageContext,
    getSelectionSnapshot,
    analyzeCurrentSelection,
    injectScripts,
  };

  if (typeof root.addEventListener === "function") {
    root.addEventListener("message", onWindowMessage);
    root.addEventListener(BRIDGE_EVENT_NAME, onWindowMessage);
  }
  if (root.__metadata_checker_content_bridge_auto_install !== false) {
    injectScripts().catch(() => {});
  }
})(typeof globalThis === "undefined" ? undefined : globalThis);

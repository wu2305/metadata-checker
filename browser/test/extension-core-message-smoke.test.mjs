import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import test from "node:test";
import vm from "node:vm";

const ROOT = new URL("..", import.meta.url);

async function loadScript(name) {
  return readFile(join(ROOT.pathname, "extension-core", name), "utf8");
}

function createFakeWindow({ bridge } = {}) {
  const listeners = new Map();
  const posted = [];
  const documentListeners = new Map();
  const document = {
    addEventListener(type, handler) {
      documentListeners.set(type, handler);
    },
    removeEventListener(type) {
      documentListeners.delete(type);
    },
    createEvent() {
      return {
        initCustomEvent(type, _bubbles, _cancelable, detail) {
          this.type = type;
          this.detail = detail;
        },
      };
    },
    dispatchEvent(event) {
      documentListeners.get(event.type)?.(event);
    },
    createElement() {
      return {
        style: {},
        setAttribute() {},
        addEventListener() {},
        remove() {},
        set src(_value) {},
        set type(_value) {},
        set async(_value) {},
      };
    },
    documentElement: {
      appendChild() {},
    },
  };

  const context = {
    console,
    document,
    setTimeout,
    clearTimeout,
    __metadata_checker_page_script_auto_install: false,
    __metadata_checker_content_bridge_auto_install: false,
    __metadata_checker_designer_bridge__: bridge,
    addEventListener(type, handler) {
      listeners.set(type, handler);
    },
    removeEventListener(type) {
      listeners.delete(type);
    },
    postMessage(message) {
      posted.push(message);
    },
    __dispatchMessage(data) {
      listeners.get("message")?.({ data });
    },
    __posted: posted,
  };
  context.globalThis = context;
  context.window = context;
  return vm.createContext(context);
}

function createContentBridgeWindow() {
  const listeners = new Map();
  const posted = [];
  const context = {
    console,
    document: {
      createElement() {
        return {
          style: {},
          setAttribute() {},
          addEventListener() {},
          remove() {},
          set src(_value) {},
          set type(_value) {},
          set async(_value) {},
        };
      },
      documentElement: {
        appendChild() {},
      },
      dispatchEvent() {},
      createEvent() {
        return {
          initCustomEvent(type, _bubbles, _cancelable, detail) {
            this.type = type;
            this.detail = detail;
          },
        };
      },
    },
    setTimeout,
    clearTimeout,
    __metadata_checker_content_bridge_auto_install: false,
    __metadata_checker_injected_script_paths: [],
    addEventListener(type, handler) {
      listeners.set(type, handler);
    },
    removeEventListener(type) {
      listeners.delete(type);
    },
    postMessage(message) {
      posted.push(message);
    },
    __dispatchMessage(data) {
      listeners.get("message")?.({ data });
    },
    __posted: posted,
  };
  context.globalThis = context;
  context.window = context;
  return vm.createContext(context);
}

async function loadPageScript(context) {
  vm.runInContext(await loadScript("bridge-protocol.js"), context);
  vm.runInContext(await loadScript("page-script.js"), context);
}

test("page script handles getBridgeStatus through page bridge", async () => {
  const context = createFakeWindow({
    bridge: {
      getPageContext() {
        return {
          page_context: {
            source_path: "app/Demo.app/Page.spg",
            project_name: "analyzer",
          },
          diagnostics: [],
        };
      },
      getSelectionSnapshot() {
        return {
          selection: {
            source_path: "app/Demo.app/Page.spg",
            selected_component_ids: ["text1"],
            active_component_id: "text1",
            page_type: "superpage",
          },
          diagnostics: [],
        };
      },
    },
  });
  await loadPageScript(context);

  const request = context.__metadata_checker_bridge_protocol__.createRequestEnvelope({
    type: "getBridgeStatus",
    requestId: "req-1",
  });
  const response = context.__metadata_checker_page_script__.handleRequest(request);

  assert.equal(response.request_id, "req-1");
  assert.equal(response.type, "getBridgeStatus");
  assert.equal(response.payload.bridge_detected, true);
  assert.equal(response.payload.page_context.source_path, "app/Demo.app/Page.spg");
  assert.deepEqual(response.payload.selection.selected_component_ids, ["text1"]);
  assert.equal(Array.isArray(response.diagnostics), true);
  assert.equal(response.diagnostics.length, 0);
});

test("page script returns stable diagnostic when bridge is missing", async () => {
  const context = createFakeWindow();
  await loadPageScript(context);
  const request = context.__metadata_checker_bridge_protocol__.createRequestEnvelope({
    type: "getPageContext",
    requestId: "req-missing",
  });

  const response = context.__metadata_checker_page_script__.handleRequest(request);

  assert.equal(response.request_id, "req-missing");
  assert.equal(response.payload.bridge_detected, false);
  assert.equal(response.diagnostics[0].code, "METADATA_CHECKER_BRIDGE_MISSING");
});

test("page script ignores non metadata-checker messages", async () => {
  const context = createFakeWindow();
  await loadPageScript(context);

  assert.equal(context.__metadata_checker_page_script__.handleRequest({ type: "hello" }), null);
});

test("page script returns protocol mismatch diagnostic", async () => {
  const context = createFakeWindow();
  await loadPageScript(context);
  const response = context.__metadata_checker_page_script__.handleRequest({
    protocol: { name: "metadata_checker_designer_bridge", version: "old" },
    request_id: "bad-protocol",
    type: "getBridgeStatus",
    payload: {},
    diagnostics: [],
  });

  assert.equal(response.request_id, "bad-protocol");
  assert.equal(response.diagnostics[0].code, "METADATA_CHECKER_PROTOCOL_MISMATCH");
});

test("content bridge unsupported request returns stable diagnostic without page access", async () => {
  const context = createFakeWindow();
  vm.runInContext(await loadScript("bridge-protocol.js"), context);
  vm.runInContext(await loadScript("content-bridge.js"), context);

  const response = await context.__metadata_checker_content_bridge__.request("unknownRequest");

  assert.equal(response.type, "unknownRequest");
  assert.equal(response.diagnostics[0].code, "METADATA_CHECKER_REQUEST_UNSUPPORTED");
});

test("content bridge ignores forged responses without page-script source and token", async () => {
  const context = createContentBridgeWindow();
  vm.runInContext(await loadScript("bridge-protocol.js"), context);
  vm.runInContext(await loadScript("content-bridge.js"), context);

  const pending = context.__metadata_checker_content_bridge__.getBridgeStatus();
  await new Promise((resolve) => setTimeout(resolve, 0));
  const request = context.__posted[0];

  context.__dispatchMessage({
    protocol: request.protocol,
    request_id: request.request_id,
    type: request.type,
    payload: { forged: true },
    diagnostics: [],
    __metadata_checker_bridge_direction: "response",
    __metadata_checker_bridge_source: "attacker",
    __metadata_checker_bridge_token: request.__metadata_checker_bridge_token,
  });
  context.__dispatchMessage({
    protocol: request.protocol,
    request_id: request.request_id,
    type: request.type,
    payload: { forged: true },
    diagnostics: [],
    __metadata_checker_bridge_direction: "response",
    __metadata_checker_bridge_source: "page-script",
    __metadata_checker_bridge_token: "wrong-token",
  });
  context.__dispatchMessage({
    protocol: request.protocol,
    request_id: request.request_id,
    type: request.type,
    payload: { ok: true },
    diagnostics: [],
    __metadata_checker_bridge_direction: "response",
    __metadata_checker_bridge_source: "page-script",
    __metadata_checker_bridge_token: request.__metadata_checker_bridge_token,
  });

  const response = await pending;
  assert.deepEqual(response.payload, { ok: true });
});

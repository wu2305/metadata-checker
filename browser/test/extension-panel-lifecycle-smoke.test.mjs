import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import test from "node:test";
import vm from "node:vm";

const ROOT = new URL("..", import.meta.url);

async function loadText(name) {
  return readFile(join(ROOT.pathname, name), "utf8");
}

function createElement(tagName) {
  return {
    tagName,
    children: [],
    attributes: {},
    style: {},
    listeners: {},
    textContent: "",
    setAttribute(name, value) {
      this.attributes[name] = String(value);
    },
    getAttribute(name) {
      return this.attributes[name] ?? null;
    },
    addEventListener(type, handler) {
      this.listeners[type] = handler;
    },
    removeEventListener(type) {
      delete this.listeners[type];
    },
    appendChild(child) {
      child.parentNode = this;
      this.children.push(child);
    },
    remove() {
      if (!this.parentNode || !Array.isArray(this.parentNode.children)) {
        return;
      }
      const index = this.parentNode.children.indexOf(this);
      if (index >= 0) {
        this.parentNode.children.splice(index, 1);
      }
      this.parentNode = null;
    },
    attachShadow() {
      const root = createElement("#shadow-root");
      root.host = this;
      return root;
    },
    dispatchEvent(event) {
      const handler = this.listeners[event.type];
      if (typeof handler === "function") {
        handler(event);
      }
    },
  };
}

function createFakeDocument() {
  const body = createElement("body");
  const documentElement = createElement("html");

  function walkSelector(nodes, name) {
    for (const node of nodes) {
      if (node.attributes[name] !== undefined) {
        return node;
      }
      const children = node.children || [];
      const found = walkSelector(children, name);
      if (found) {
        return found;
      }
    }
    return null;
  }

  return {
    body,
    documentElement,
    createElement,
    querySelector(selector) {
      const target = String(selector || "").match(/^\[(.+)\]$/)?.[1];
      if (!target) {
        return null;
      }
      return walkSelector([body, documentElement], target);
    },
    createEvent(type) {
      return {
        initCustomEvent(_name, _bubbles, _cancelable, detail) {
          this.type = type;
          this.detail = detail;
        },
      };
    },
    dispatchEvent(event) {
      const listeners = this.listeners?.get?.(event.type) || [];
      for (const listener of listeners) {
        listener(event);
      }
    },
    addEventListener(type, handler) {
      this.listeners = this.listeners || new Map();
      const list = this.listeners.get(type) || [];
      list.push(handler);
      this.listeners.set(type, list);
    },
    removeEventListener(type, handler) {
      if (!this.listeners?.has(type)) {
        return;
      }
      const list = this.listeners.get(type).filter((item) => item !== handler);
      this.listeners.set(type, list);
    },
  };
}

function createFakeWindow({ bridge }) {
  const listeners = new Map();
  const runtimeListeners = [];
  const sentStatuses = [];
  const document = createFakeDocument();

  const runtime = {
    onMessage: {
      addListener(handler) {
        runtimeListeners.push(handler);
      },
      listeners: runtimeListeners,
    },
    sendMessage(message) {
      sentStatuses.push(message);
    },
  };

  const context = {
    console,
    clearTimeout,
    setTimeout,
    document,
    addEventListener(type, handler) {
      const list = listeners.get(type) || [];
      list.push(handler);
      listeners.set(type, list);
    },
    removeEventListener(type, handler) {
      const list = listeners.get(type) || [];
      listeners.set(type, list.filter((item) => item !== handler));
    },
    postMessage(message) {
      const listenersForPost = listeners.get("message") || [];
      for (const listener of listenersForPost) {
        listener({ data: message });
      }
    },
    __metadata_checker_content_bridge__: bridge,
    __metadata_checker_content_bridge_auto_install: false,
    chrome: {
      runtime,
    },
    __metadata_checker_content_bridge_paths: [],
    __metadata_checker_runtime_listeners: runtimeListeners,
    __dispatchMessage(data) {
      const list = listeners.get("message") || [];
      for (const listener of list) {
        listener({ data });
      }
    },
    __sentStatuses() {
      return sentStatuses;
    },
  };
  context.globalThis = context;
  context.window = context;

  return vm.createContext(context);
}

function defaultBridge() {
  return {
    async request(type) {
      return {
        payload: {
          supported: true,
          bridge_detected: true,
          page_context: { source_path: "app/Start.spg" },
          selection: { source_path: "app/Start.spg", selected_component_ids: [] },
        },
        diagnostics: [],
      };
    },
  };
}

async function loadScripts(context, ...files) {
  for (const file of files) {
    const source = await loadText(file);
    vm.runInContext(source, context);
  }
}

async function runRuntimeMessage(context, message) {
  const listener = context.__metadata_checker_runtime_listeners?.[0];
  assert.ok(typeof listener === "function");
  let response;
  await new Promise((resolve) => {
    const sendResponse = (value) => {
      response = value;
      resolve();
    };
    const maybeAsync = listener(message, {}, sendResponse);
    if (maybeAsync !== true) {
      if (response !== undefined) {
        resolve();
      } else {
        resolve();
      }
    }
  });
  if (response === undefined && response !== 0) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  return response;
}

test("content script mounts panel host on startup and starts hidden", async () => {
  const context = createFakeWindow({ bridge: defaultBridge() });
  await loadScripts(context, "extension-core/panel-host.js", "extension-chromium/content-script.js");

  const state = context.__metadata_checker_chromium_content_state__;
  assert.equal(typeof state?.panelHost, "object");
  const host = state.panelHost;
  const hostState = host.getState();
  assert.equal(hostState.mounted, true);
  assert.equal(hostState.visible, false);
  assert.equal(hostState.hostElement?.getAttribute("data-metadata-checker-panel"), "hidden");
  assert.equal(context.document.body.children.length, 1);
});

test("content script handles open/hide/toggle actions", async () => {
  const context = createFakeWindow({ bridge: defaultBridge() });
  await loadScripts(context, "extension-core/panel-host.js", "extension-chromium/content-script.js");
  const host = context.__metadata_checker_chromium_content_state__.panelHost;

  const opened = await runRuntimeMessage(context, { action: "openPanel" });
  assert.equal(opened.visible, true);
  assert.equal(host.getState().visible, true);

  const hidden = await runRuntimeMessage(context, { action: "hidePanel" });
  assert.equal(hidden.visible, false);
  assert.equal(host.getState().visible, false);

  const toggled = await runRuntimeMessage(context, { action: "togglePanel" });
  assert.equal(toggled.visible, true);
  assert.equal(host.getState().visible, true);
});

test("content script handles popup tab-request panel commands locally", async () => {
  let bridgeRequestCount = 0;
  const context = createFakeWindow({
    bridge: {
      async request() {
        bridgeRequestCount += 1;
        return {
          payload: { supported: false },
          diagnostics: [
            {
              severity: "error",
              code: "SHOULD_NOT_FORWARD_PANEL_COMMAND",
              message: "panel command should not reach page bridge",
            },
          ],
        };
      },
    },
  });
  await loadScripts(context, "extension-core/panel-host.js", "extension-chromium/content-script.js");
  const host = context.__metadata_checker_chromium_content_state__.panelHost;

  const opened = await runRuntimeMessage(context, {
    type: "metadata-checker-tab-request",
    request_type: "openPanel",
  });

  assert.equal(opened.action, "openPanel");
  assert.equal(opened.visible, true);
  assert.equal(host.getState().visible, true);
  assert.equal(bridgeRequestCount, 0);
});

test("content script updates panel host on page script selection changed message", async () => {
  const context = createFakeWindow({ bridge: defaultBridge() });
  await loadScripts(context, "extension-core/panel-host.js", "extension-chromium/content-script.js");
  const host = context.__metadata_checker_chromium_content_state__.panelHost;

  context.__dispatchMessage({
    __metadata_checker_bridge_source: "page-script",
    __metadata_checker_bridge_direction: "notification",
    type: "metadata-checker-selection-changed",
    payload: {
      source_path: "app/Test.spg",
      selected_component_ids: ["a", "b", "c"],
      active_component_id: "b",
    },
  });

  const state = host.getState();
  assert.equal(state.selectionCount, 3);
  assert.equal(state.sourcePath, "app/Test.spg");
  assert.equal(
    state.hostElement?.getAttribute("data-metadata-checker-panel-source-path"),
    "app/Test.spg",
  );
  assert.equal(
    state.hostElement?.getAttribute("data-metadata-checker-panel-selection-count"),
    "3",
  );
});

test("missing bridge sets panel status diagnostic without throwing", async () => {
  const missingBridge = {
    async request() {
      return {
        payload: {
          supported: false,
          bridge_detected: false,
        },
        diagnostics: [
          {
            severity: "error",
            code: "METADATA_CHECKER_BRIDGE_MISSING",
            message: "bridge missing",
          },
        ],
      };
    },
  };

  const context = createFakeWindow({ bridge: missingBridge });
  await loadScripts(context, "extension-core/panel-host.js", "extension-chromium/content-script.js");
  const host = context.__metadata_checker_chromium_content_state__.panelHost;

  const response = await runRuntimeMessage(context, { action: "refreshBridge" });
  assert.equal(response?.action, "refreshBridge");
  assert.equal(host.getState().panelStatus, "error");
  assert.equal(
    host.getState().hostElement?.getAttribute("data-metadata-checker-panel-last-status"),
    "error",
  );
});

test("content script bootstrap is idempotent on repeated injection", async () => {
  const context = createFakeWindow({ bridge: defaultBridge() });
  await loadScripts(context, "extension-core/panel-host.js", "extension-chromium/content-script.js");
  await loadScripts(context, "extension-chromium/content-script.js");

  const state = context.__metadata_checker_chromium_content_state__;
  assert.equal(state.mounted, true);
  assert.equal(context.document.body.children.length, 1);
  assert.equal(state.mountResult?.mounted, true);
  assert.equal(state.panelHost.getState().visible, false);
});

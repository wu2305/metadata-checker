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

function createFakeWindow({ bridge, fetchImpl, runtimeSendMessage, runtimeSendMessageArityOne }) {
  const listeners = new Map();
  const runtimeListeners = [];
  const sentStatuses = [];
  const postedMessages = [];
  const document = createFakeDocument();

  const sendMessage = runtimeSendMessageArityOne
    ? function sendMessage(message) {
      sentStatuses.push(message);
      if (typeof runtimeSendMessage === "function") {
        return runtimeSendMessage(message, arguments[1]);
      }
      arguments[1]?.({ ok: true });
      return undefined;
    }
    : function sendMessage(message, callback) {
      sentStatuses.push(message);
      if (typeof runtimeSendMessage === "function") {
        return runtimeSendMessage(message, callback);
      }
      if (typeof callback === "function") {
        callback({ ok: true });
      }
      return undefined;
    };

  const runtime = {
    onMessage: {
      addListener(handler) {
        runtimeListeners.push(handler);
      },
      listeners: runtimeListeners,
    },
    sendMessage,
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
      postedMessages.push(message);
      const listenersForPost = listeners.get("message") || [];
      for (const listener of listenersForPost) {
        listener({ data: message });
      }
    },
    fetch: fetchImpl ?? (async () => ({
      ok: true,
      status: 200,
      async text() {
        return "one-shot-token";
      },
    })),
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
    __postedMessages() {
      return postedMessages;
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

function markerValue(context, name) {
  const attribute = `data-metadata-checker-${name}`;
  return context.document.querySelector(`[${attribute}]`)?.getAttribute(attribute) ?? null;
}

function waitForSelectionAnalysis() {
  return new Promise((resolve) => setTimeout(resolve, 70));
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
      selected_component_types: ["input", "button", "panel"],
      selection_source: "selectComponents",
      changed_at: 123456,
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
  assert.equal(
    context.document.querySelector("[data-metadata-checker-extension-selection-event]")?.getAttribute("data-metadata-checker-extension-selection-event"),
    "received",
  );
  assert.equal(
    context.document.querySelector("[data-metadata-checker-extension-selection-active]")?.getAttribute("data-metadata-checker-extension-selection-active"),
    "b",
  );
  assert.equal(
    context.document.querySelector("[data-metadata-checker-extension-selection-source]")?.getAttribute("data-metadata-checker-extension-selection-source"),
    "selectComponents",
  );
});

test("selection change result with foreground artifact keeps panel status ready and renders artifact data", async () => {
  const runtimeMessages = [];
  const context = createFakeWindow({
    bridge: defaultBridge(),
    runtimeSendMessage(message, callback) {
      runtimeMessages.push(message);
      if (message?.type === "metadata-checker-analyze-local-graph" && typeof callback === "function") {
        callback({
          ok: true,
          artifact: {
            kind: "metadata-analysis-artifact",
            source_path: "app/Test.spg",
            analysis_status: "ready",
            result: {
              status: "ready",
              target: "app/Test.spg",
              items: [
                {
                  kind: "foreground_selection_item",
                  label: "selection-result",
                  detail: {
                    visual_graph: {
                      depth: 2,
                      visible_hop: 1,
                      focus_node: "comp:app/Test.spg|a",
                      nodes: [{ id: "a" }, { id: "b" }, { id: "c" }],
                      edges: [{ source: "a", target: "b" }, { source: "a", target: "c" }],
                    },
                  },
                },
              ],
              diagnostics: [{ severity: "warning", code: "SEL_ANALYSIS_OK", message: "foreground artifact ready" }],
            },
          },
          background: {
            status: "running",
            processed: 1,
            total: 2,
          },
          diagnostics: [{ severity: "warning", code: "SEL_QUEUE", message: "selection queued" }],
        });
      } else if (typeof callback === "function") {
        callback({ ok: true });
      }
    },
  });
  await loadScripts(context, "extension-core/panel-host.js", "extension-chromium/content-script.js");
  const host = context.__metadata_checker_chromium_content_state__.panelHost;

  context.__dispatchMessage({
    __metadata_checker_bridge_source: "page-script",
    __metadata_checker_bridge_direction: "notification",
    type: "metadata-checker-selection-changed",
    payload: {
      source_path: "app/Test.spg",
      selected_component_ids: ["a", "b"],
      selected_component_types: ["input", "button"],
      selection_source: "selectComponents",
      changed_at: 123456,
      active_component_id: "a",
    },
  });

  await waitForSelectionAnalysis();

  const state = host.getState();
  assert.equal(state.hostElement?.getAttribute("data-metadata-checker-panel-last-status"), "ready");
  assert.equal(state.lastEnvelope?.status, "ready");
  assert.equal(state.lastEnvelope?.target, "app/Test.spg");
  assert.equal(state.lastEnvelope?.items?.some((item) => item.kind === "foreground_selection_item"), true);
  assert.equal(state.lastEnvelope?.diagnostics?.some((item) => item.code === "SEL_QUEUE"), true);
  assert.equal(state.lastEnvelope?.items?.some((item) => item.kind === "background_status"), true);
  assert.equal(state.lastEnvelope?.status !== "idle", true);
  assert.equal(runtimeMessages.some((message) => message.type === "metadata-checker-analyze-local-graph"), true);
  assert.equal(runtimeMessages.some((message) => message.type === "metadata-checker-selection-changed"), false);
  assert.equal(markerValue(context, "analysis-status"), "ready");
  assert.equal(markerValue(context, "embedded-popup"), "mounted");
  assert.equal(markerValue(context, "focus-component"), "comp:app/Test.spg|a");
  assert.equal(markerValue(context, "graph-depth"), "2");
  assert.equal(markerValue(context, "graph-visible-hop"), "1");
  assert.equal(markerValue(context, "graph-node-count"), "3");
  assert.equal(markerValue(context, "graph-edge-count"), "2");
});

test("content script retryCurrentSelection replays bridge selection to background queue", async () => {
  const runtimeMessages = [];
  const context = createFakeWindow({
    bridge: {
      async request(type) {
        assert.equal(type, "getBridgeStatus");
        return {
          payload: {
            supported: true,
            bridge_detected: true,
            page_context: { source_path: "app/Retry.spg" },
            selection: {
              source_path: "app/Retry.spg",
              selected_component_ids: ["input1"],
              active_component_id: "input1",
            },
          },
          diagnostics: [],
        };
      },
    },
    runtimeSendMessage(message, callback) {
      runtimeMessages.push(message);
      if (message?.type === "metadata-checker-analyze-local-graph") {
        callback?.({
          ok: true,
          artifact_ready: false,
          artifact: {
            source_path: "app/Retry.spg",
            analysis_status: "running",
            result: {
              status: "running",
            },
          },
          background: {
            status: "queued",
            indexing_status: "indexing_current_page",
            current_source_path: "app/Retry.spg",
            processed: 0,
            total: 1,
          },
        });
        return undefined;
      }
      callback?.({ ok: true });
      return undefined;
    },
  });
  await loadScripts(context, "extension-core/panel-host.js", "extension-chromium/content-script.js");

  const response = await runRuntimeMessage(context, {
    type: "metadata-checker-tab-request",
    request_type: "retryCurrentSelection",
  });

  assert.equal(response?.ok, true);
  assert.equal(
    runtimeMessages.some(
      (message) =>
        message.type === "metadata-checker-analyze-local-graph" &&
        message.payload.source_path === "app/Retry.spg" &&
        message.payload.active_component_id === "input1",
    ),
    true,
  );
  const host = context.__metadata_checker_chromium_content_state__.panelHost;
  assert.equal(host.getState().lastEnvelope?.target, "app/Retry.spg");
  assert.equal(host.getState().lastEnvelope?.status, "analyzing");
});

test("selection waiting_for_metadata reboots session and replays current selection", async () => {
  const runtimeMessages = [];
  const context = createFakeWindow({
    bridge: defaultBridge(),
    fetchImpl: async () => ({
      ok: true,
      status: 200,
      async text() {
        return "content-token";
      },
    }),
    runtimeSendMessage(message, callback) {
      runtimeMessages.push(message);
      if (message?.type === "metadata-checker-analyze-local-graph") {
        const selectionCalls = runtimeMessages.filter((item) => item.type === "metadata-checker-analyze-local-graph");
        if (selectionCalls.length === 1) {
          callback?.({
            ok: true,
            artifact_ready: false,
            diagnostics: [{
              severity: "warning",
              code: "METADATA_CHECKER_LOCAL_GRAPH_ARTIFACT_MISSING",
              message: "local graph document artifact is not loaded",
            }],
            background: {
              status: "waiting_for_metadata",
              indexing_status: "waiting_for_metadata",
              current_source_path: message.payload.source_path,
            },
          });
          return undefined;
        }
        callback?.({
          ok: true,
          artifact_ready: true,
          artifact: {
            source_path: message.payload.source_path,
            analysis_status: "ready",
            kind: "metadata-analysis-artifact",
            result: { status: "ready", items: [{ kind: "analysis", label: "ready" }] },
          },
          background: {
            status: "completed",
            indexing_status: "completed",
            processed: 1,
            total: 1,
          },
        });
        return undefined;
      }
      if (message?.type === "metadata-checker-bootstrap-token") {
        callback?.({
          ok: true,
          session: { status: "ready" },
          visible_index: { status: "ready", projects: [{ project_name: "xiaoshouyi" }], files: [], analyzable_count: 0 },
          background: { status: "completed", indexing_status: "completed", processed: 0, total: 0 },
        });
        return undefined;
      }
      callback?.({ ok: true });
      return undefined;
    },
  });
  await loadScripts(context, "extension-core/panel-host.js", "extension-chromium/content-script.js");

  context.__dispatchMessage({
    __metadata_checker_bridge_source: "page-script",
    __metadata_checker_bridge_direction: "notification",
    type: "metadata-checker-selection-changed",
    payload: {
      source_path: "app/Start.spg",
      selected_component_ids: ["text1"],
      active_component_id: "text1",
      selection_source: "selectComponents",
    },
  });
  await waitForSelectionAnalysis();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(
    runtimeMessages.filter((message) => message.type === "metadata-checker-analyze-local-graph").length,
    2,
  );
  assert.equal(
    runtimeMessages.some(
      (message) =>
        message.type === "metadata-checker-bootstrap-token" &&
        message.payload.access_token === "content-token",
    ),
    true,
  );
  const host = context.__metadata_checker_chromium_content_state__.panelHost;
  assert.equal(host.getState().lastEnvelope?.status, "ready");
  assert.equal(host.getState().lastEnvelope?.items?.some((item) => item.kind === "analysis"), true);
});

test("selection artifact missing after bootstrap uses offscreen local graph with indexed file id", async () => {
  const runtimeMessages = [];
  let localGraphCalls = 0;
  const context = createFakeWindow({
    bridge: defaultBridge(),
    fetchImpl: async () => ({
      ok: true,
      status: 200,
      async text() {
        return "content-token";
      },
    }),
    runtimeSendMessage(message, callback) {
      runtimeMessages.push(message);
      if (message?.type === "metadata-checker-analyze-local-graph") {
        localGraphCalls += 1;
        if (localGraphCalls <= 2) {
          callback?.({
            ok: false,
            artifact_ready: false,
            diagnostics: [{
              severity: "error",
              code: "METADATA_CHECKER_LOCAL_GRAPH_ARTIFACT_MISSING",
              message: "local graph document artifact is not loaded",
            }],
            background: {
              status: "queued",
              indexing_status: "queued",
              processed: 3,
              total: 1300,
            },
          });
          return undefined;
        }
        callback?.({
          ok: true,
          artifact_ready: true,
          artifact: {
            source_path: message.payload.source_path,
            analysis_status: "ready",
            kind: "metadata-analysis-artifact",
            result: {
              status: "ready",
              target: message.payload.source_path,
              items: [{
                kind: "foreground_selection_item",
                label: "local graph ready",
                detail: {
                  visual_graph: {
                    depth: 2,
                    visible_hop: 1,
                    focus_node: "comp:app/Start.spg|text1",
                    nodes: [{ id: "text1" }, { id: "model:customer" }],
                    edges: [{ source: "text1", target: "model:customer" }],
                  },
                },
              }],
            },
          },
          background: {
            status: "completed",
            indexing_status: "completed",
            processed: 4,
            total: 1300,
          },
        });
        return undefined;
      }
      if (message?.type === "metadata-checker-bootstrap-token") {
        callback?.({
          ok: true,
          session: { status: "ready" },
          visible_index: {
            status: "ready",
            projects: [{ project_name: "xiaoshouyi" }],
            files: [{
              project_name: "xiaoshouyi",
              source_path: "app/Start.spg",
              file_id: "file-start",
              revision: 7,
              extension: "spg",
              analyzable: true,
            }],
            analyzable_count: 1,
          },
          background: { status: "queued", indexing_status: "queued", processed: 0, total: 1 },
        });
        return undefined;
      }
      if (message?.type === "metadata-checker-ensure-offscreen-runtime") {
        callback?.({ ok: true });
        return undefined;
      }
      if (message?.type === "metadata-checker-offscreen-local-graph") {
        callback?.({
          ok: true,
          artifact_ready: true,
          artifact: {
            source_path: message.payload.item.source_path,
            file_id: message.payload.item.file_id,
            analysis_status: "ready",
            kind: "metadata-analysis-artifact",
            result: {
              status: "ready",
              target: message.payload.item.source_path,
              items: [{
                kind: "foreground_selection_item",
                label: "local graph ready",
                detail: {
                  visual_graph: {
                    depth: 2,
                    visible_hop: 1,
                    focus_node: "comp:app/Start.spg|text1",
                    nodes: [{ id: "text1" }, { id: "model:customer" }],
                    edges: [{ source: "text1", target: "model:customer" }],
                  },
                },
              }],
            },
          },
        });
        return undefined;
      }
      callback?.({ ok: true });
      return undefined;
    },
  });
  await loadScripts(context, "extension-core/panel-host.js", "extension-chromium/content-script.js");

  context.__dispatchMessage({
    __metadata_checker_bridge_source: "page-script",
    __metadata_checker_bridge_direction: "notification",
    type: "metadata-checker-selection-changed",
    payload: {
      source_path: "app/Start.spg",
      selected_component_ids: ["text1"],
      active_component_id: "text1",
      selection_source: "selectComponents",
    },
  });
  await waitForSelectionAnalysis();
  await new Promise((resolve) => setTimeout(resolve, 0));
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(
    runtimeMessages.filter((message) => message.type === "metadata-checker-analyze-local-graph").length,
    2,
  );
  assert.equal(
    runtimeMessages.filter((message) => message.type === "metadata-checker-selection-changed").length,
    0,
  );
  assert.equal(
    runtimeMessages.some((message) => message.type === "metadata-checker-bootstrap-token"),
    true,
  );
  const offscreenMessage = runtimeMessages.find((message) => message.type === "metadata-checker-offscreen-local-graph");
  assert.equal(offscreenMessage?.payload?.item?.file_id, "file-start");
  assert.equal(offscreenMessage?.payload?.item?.revision, 7);
  const host = context.__metadata_checker_chromium_content_state__.panelHost;
  assert.equal(host.getState().lastEnvelope?.status, "ready");
  assert.equal(markerValue(context, "analysis-status"), "ready");
  assert.equal(markerValue(context, "graph-node-count"), "2");
  assert.equal(markerValue(context, "graph-edge-count"), "1");
});

test("content script fetches access token without exposing it through page messages", async () => {
  const runtimeMessages = [];
  const context = createFakeWindow({
    bridge: defaultBridge(),
    fetchImpl: async (url, init) => {
      assert.equal(url, "/api/auth/getAccessToken");
      assert.equal(init.credentials, "include");
      return {
        ok: true,
        status: 200,
        async text() {
          return "content-token";
        },
      };
    },
    runtimeSendMessage(message, callback) {
      runtimeMessages.push(message);
      callback?.({
        ok: true,
        session: { status: "ready" },
        visible_index: { files: [], projects: [], analyzable_count: 0 },
        background: { status: "idle", processed: 0, total: 0 },
      });
    },
  });
  await loadScripts(context, "extension-core/panel-host.js", "extension-chromium/content-script.js");

  context.__dispatchMessage({
    __metadata_checker_bridge_source: "page-script",
    __metadata_checker_bridge_direction: "notification",
    type: "bridgeReady",
    payload: {
      page_context: { source_path: "app/Test.spg" },
      diagnostics: [],
    },
  });
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(
    runtimeMessages.some(
      (message) =>
        message.type === "metadata-checker-bootstrap-token" &&
        message.payload.access_token === "content-token",
    ),
    true,
  );
  assert.equal(JSON.stringify(context.__postedMessages()).includes("content-token"), false);
  assert.equal(
    context.document.querySelector("[data-metadata-checker-extension-token-source]")?.getAttribute("data-metadata-checker-extension-token-source"),
    "content-script",
  );
});

test("bootstrap failure keeps source path and renders fallback diagnostic", async () => {
  const context = createFakeWindow({
    bridge: {
      async request() {
        return {
          payload: {
            supported: true,
            bridge_detected: true,
            page_context: { source_path: "app/Test.spg" },
            selection: {
              source_path: "app/Test.spg",
              selected_component_ids: ["canvas"],
              active_component_id: "canvas",
            },
          },
          diagnostics: [],
        };
      },
    },
    fetchImpl: async () => ({
      ok: true,
      status: 200,
      async text() {
        return "content-token";
      },
    }),
    runtimeSendMessage(message, callback) {
      if (message?.type === "metadata-checker-bootstrap-token") {
        callback?.({ ok: false });
        return undefined;
      }
      callback?.({ ok: true });
      return undefined;
    },
  });
  await loadScripts(context, "extension-core/panel-host.js", "extension-chromium/content-script.js");

  context.__dispatchMessage({
    __metadata_checker_bridge_source: "page-script",
    __metadata_checker_bridge_direction: "notification",
    type: "bridgeReady",
    payload: {
      page_context: { source_path: "app/Test.spg" },
      diagnostics: [],
    },
  });
  await new Promise((resolve) => setTimeout(resolve, 0));

  const host = context.__metadata_checker_chromium_content_state__.panelHost;
  const state = host.getState();
  assert.equal(state.lastEnvelope?.status, "error");
  assert.equal(state.lastEnvelope?.target, "app/Test.spg");
  assert.equal(
    state.lastEnvelope?.diagnostics?.some((item) => item.code === "SESSION_BOOTSTRAP_FAILED"),
    true,
  );
  assert.equal(
    state.hostElement?.getAttribute("data-metadata-checker-panel-source-path"),
    "app/Test.spg",
  );
});

test("arity-one runtime sendMessage waits for delayed background callback", async () => {
  const context = createFakeWindow({
    bridge: defaultBridge(),
    runtimeSendMessageArityOne: true,
    runtimeSendMessage(message, callback) {
      if (message?.type === "metadata-checker-bootstrap-token") {
        setTimeout(() => {
          callback?.({
            ok: false,
            diagnostics: [
              {
                severity: "error",
                code: "SESSION_COOKIE_NOT_ESTABLISHED",
                message: "cookie jar was not established",
              },
            ],
          });
        }, 5);
        return undefined;
      }
      callback?.({ ok: true });
      return undefined;
    },
  });
  await loadScripts(context, "extension-core/panel-host.js", "extension-chromium/content-script.js");

  context.__dispatchMessage({
    __metadata_checker_bridge_source: "page-script",
    __metadata_checker_bridge_direction: "notification",
    type: "bridgeReady",
    payload: {
      page_context: { source_path: "app/Test.spg" },
      diagnostics: [],
    },
  });
  await new Promise((resolve) => setTimeout(resolve, 20));

  const host = context.__metadata_checker_chromium_content_state__.panelHost;
  const state = host.getState();
  assert.equal(state.lastEnvelope?.target, "app/Start.spg");
  assert.equal(
    state.lastEnvelope?.diagnostics?.some((item) => item.code === "SESSION_COOKIE_NOT_ESTABLISHED"),
    true,
  );
  assert.equal(
    state.lastEnvelope?.diagnostics?.some((item) => item.code === "SESSION_BOOTSTRAP_FAILED"),
    false,
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

test("refreshBridge triggers background visible manifest refresh and writes markers", async () => {
  const sentMessages = [];
  const bridge = {
    async request(type) {
      assert.equal(type, "getBridgeStatus");
      return {
        payload: {
          supported: true,
          bridge_detected: true,
          page_context: {
            project_name: "xiaoshouyi",
            source_path: "app/Test.app/Page.spg",
          },
          selection: {
            project_name: "xiaoshouyi",
            source_path: "app/Test.app/Page.spg",
            selected_component_ids: ["input1"],
          },
        },
        diagnostics: [],
      };
    },
  };
  const context = createFakeWindow({
    bridge,
    runtimeSendMessage(message, callback) {
      sentMessages.push(message);
      if (message.type === "metadata-checker-refresh-visible-manifest") {
        callback({
          ok: true,
          visible_index: { files: [{ source_path: "app/Test.app/Page.spg" }] },
          manifest_diff: {
            added: 1,
            modified: 2,
            deleted: 0,
            unchanged: 3,
            content_queue_count: 2,
          },
          timings: {
            manifest_fetch_ms: 11,
            manifest_diff_ms: 7,
            changed_content_fetch_ms: 13,
            wasm_update_ms: 17,
          },
          diagnostics: [],
        });
        return undefined;
      }
      callback({ ok: true });
      return undefined;
    },
  });
  context.location = { origin: "https://autocrm-test.xiaoshouyi.com" };
  await loadScripts(context, "extension-core/panel-host.js", "extension-chromium/content-script.js");
  context.__metadata_checker_chromium_content_state__.remoteSession = {
    base_url: "https://autocrm-test.xiaoshouyi.com",
    project_name: "xiaoshouyi",
  };

  const response = await runRuntimeMessage(context, { action: "refreshBridge" });

  assert.equal(response?.action, "refreshBridge");
  assert.equal(response?.manifest_diff?.modified, 2);
  assert.equal(markerValue(context, "manifest-added"), "1");
  assert.equal(markerValue(context, "manifest-modified"), "2");
  assert.equal(markerValue(context, "content-queue-count"), "2");
  assert.equal(markerValue(context, "local-graph-timing-manifest-fetch-ms"), "11");
  assert.equal(markerValue(context, "local-graph-timing-manifest-diff-ms"), "7");
  assert.equal(markerValue(context, "local-graph-timing-changed-content-fetch-ms"), "13");
  assert.equal(markerValue(context, "local-graph-timing-wasm-update-ms"), "17");
  assert.equal(
    sentMessages.some((message) => message.type === "metadata-checker-refresh-visible-manifest"),
    true,
  );
});

test("tab-request refreshBridge triggers background visible manifest refresh", async () => {
  const sentMessages = [];
  const context = createFakeWindow({
    bridge: defaultBridge(),
    runtimeSendMessage(message, callback) {
      sentMessages.push(message);
      if (message.type === "metadata-checker-refresh-visible-manifest") {
        callback({
          ok: true,
          visible_index: { files: [{ source_path: "app/Start.spg" }] },
          manifest_diff: {
            added: 0,
            modified: 1,
            deleted: 0,
            unchanged: 4,
            content_queue_count: 1,
          },
          timings: {
            manifest_fetch_ms: 3,
            manifest_diff_ms: 5,
            changed_content_fetch_ms: 7,
            wasm_update_ms: 11,
          },
          diagnostics: [],
        });
        return undefined;
      }
      callback({ ok: true });
      return undefined;
    },
  });
  context.location = { origin: "https://autocrm-test.xiaoshouyi.com" };
  await loadScripts(context, "extension-core/panel-host.js", "extension-chromium/content-script.js");
  context.__metadata_checker_chromium_content_state__.remoteSession = {
    base_url: "https://autocrm-test.xiaoshouyi.com",
    project_name: "xiaoshouyi",
  };

  const response = await runRuntimeMessage(context, {
    type: "metadata-checker-tab-request",
    request_type: "refreshBridge",
  });

  assert.equal(response?.ok, true);
  assert.equal(response?.manifest_diff?.modified, 1);
  assert.equal(markerValue(context, "manifest-modified"), "1");
  assert.equal(markerValue(context, "local-graph-timing-wasm-update-ms"), "11");
  assert.equal(
    sentMessages.some((message) => message.type === "metadata-checker-refresh-visible-manifest"),
    true,
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

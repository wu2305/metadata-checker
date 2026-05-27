/**
 * M44.2 SuperPage Selection Bridge Smoke Tests
 */

import assert from "node:assert";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import vm from "node:vm";
import { describe, it } from "node:test";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const pageScriptPath = join(__dirname, "../extension-core/page-script.js");
const protocolPath = join(__dirname, "../extension-core/bridge-protocol.js");

function createMarker(value = null) {
  const attributes = new Map();
  return {
    style: {},
    setAttribute(key, nextValue) {
      attributes.set(key, String(nextValue));
    },
    getAttribute(key) {
      return attributes.get(key) ?? null;
    },
    hasAttribute(key) {
      return attributes.has(key);
    },
    __value: value,
  };
}

function createDocument() {
  const markerNodes = [];

  return {
    markerNodes,
    createElement() {
      const node = createMarker();
      return node;
    },
    createEvent() {
      return {
        initCustomEvent(type, _bubbles, _cancelable, detail) {
          this.type = type;
          this.detail = detail;
        },
      };
    },
    addEventListener() {},
    removeEventListener() {},
    dispatchEvent() {},
    querySelector(selector) {
      const match = selector.match(/^\[([^\]]+)\]$/);
      if (!match) {
        return null;
      }
      const key = match[1];
      return markerNodes.find((node) => node.hasAttribute?.(key)) ?? null;
    },
    documentElement: {
      appendChild(node) {
        markerNodes.push(node);
      },
    },
  };
}

function markerValue(document, name) {
  const key = `data-metadata-checker-${name}`;
  return document.querySelector(`[${key}]`)?.getAttribute(key) ?? null;
}

function createFakeWindow({ bridge }) {
  const posted = [];
  const listeners = new Map();
  const messageListeners = new Map();
  const document = createDocument();

  const context = {
    console: { log() {}, warn() {}, error() {} },
    document,
    setTimeout,
    clearTimeout,
    __metadata_checker_page_script_auto_install: false,
    __metadata_checker_content_bridge_auto_install: false,
    __metadata_checker_designer_bridge__: bridge,
    addEventListener(event, handler) {
      listeners.set(event, handler);
    },
    removeEventListener(event) {
      listeners.delete(event);
    },
    postMessage(message) {
      posted.push(message);
    },
    fetch: async () => ({
      ok: true,
      status: 200,
      async text() {
        return "one-shot-token";
      },
    }),
    dispatchMessage(data) {
      listeners.get("message")?.({ data });
    },
  };

  context.window = context;
  context.globalThis = context;

  function loadScript(path) {
    vm.runInNewContext(readFileSync(path, "utf8"), context);
  }

  loadScript(protocolPath);
  loadScript(pageScriptPath);

  function getSelectionChangedMessages() {
    return posted.filter((item) => item.type === "metadata-checker-selection-changed");
  }

  function asSelectionComponent({ id, type, name, ...rest }) {
    return {
      id,
      type,
      name,
      ...rest,
      getId() {
        return this.id;
      },
      getType() {
        return this.type;
      },
      getName() {
        return this.name;
      },
    };
  }

  function createBuilder(initialComponents = []) {
    let selected = initialComponents;
    return {
      getSelectedComponents() {
        return selected;
      },
      selectComponents(infos = []) {
        selected = infos.map((component) => {
          if (component && typeof component === "object" && typeof component.getId !== "function") {
            return asSelectionComponent(component);
          }
          return component;
        });
      },
      deselectComponents(ids = []) {
        const idSet = new Set(
          ids.map((item) => (typeof item === "string" ? item : item?.id)).filter((item) => typeof item === "string"),
        );
        selected = selected.filter((item) => {
          const id = typeof item?.getId === "function" ? item.getId() : item?.id;
          return !idSet.has(id);
        });
      },
      deselectAll() {
        selected = [];
      },
      doSelectedChange() {},
    };
  }

  return {
    context,
    posted,
    messageListeners,
    getSelectionChangedMessages,
    createBuilder,
    asSelectionComponent,
    document,
  };
}

function createBridgeWithContext(pageContext = {}, extra = {}) {
  return {
    getPageContext() {
      return {
        page_context: pageContext,
        diagnostics: [],
        ...extra.pageContextExtra,
      };
    },
    getSelectionSnapshot() {
      return { selection: { source_path: pageContext.source_path, selected_component_ids: [] }, diagnostics: [] };
    },
    analyzeCurrentSelection() {
      return { supported: false };
    },
    ...extra,
  };
}

function readSelectionPayloads(windowState) {
  return windowState.getSelectionChangedMessages();
}

function waitForSelectionDebounce(windowState) {
  const delay = windowState.context.__metadata_checker_page_script__.SELECTION_CHANGE_DEBOUNCE_MS + 20;
  return new Promise((resolve) => setTimeout(resolve, delay));
}

describe("selection bridge", () => {
  it("does not support access token over page-visible bridge", async () => {
    const bridge = createBridgeWithContext({
      source_path: "app/Test.app/Page.spg",
    });
    const windowState = createFakeWindow({ bridge });

    const response = await windowState.context.__metadata_checker_page_script__.handleRequest({
      protocol: windowState.context.__metadata_checker_bridge_protocol__.BRIDGE_PROTOCOL,
      request_id: "token-1",
      type: "getAccessToken",
      payload: {},
    });

    assert.strictEqual(response.payload.supported, false);
    assert.strictEqual(JSON.stringify(response).includes("one-shot-token"), false);
  });

  it("patches selectComponents and emits full lightweight payload", async () => {
    const bridge = createBridgeWithContext({
      source_path: "app/Test.app/Page.spg",
    });
    const windowState = createFakeWindow({ bridge });
    const builder = windowState.createBuilder();
    delete builder.doSelectedChange;
    bridge.getPageContext = () => ({
      page_context: {
        source_path: "app/Test.app/Page.spg",
      },
      builder,
      diagnostics: [],
    });

    const script = windowState.context.__metadata_checker_page_script__;
    assert.strictEqual(script.installSelectionBridge().installed, true);

    builder.selectComponents([
      {
        id: "input1",
        type: "input",
        getType: () => "input",
        getName: () => "name",
        raw_text: "leak",
        rawText: "also-leak",
        components: [{ id: "hidden" }],
        password: "secret",
      },
      {
        id: "button1",
        type: "button",
        name: "提交",
      },
    ]);

    await waitForSelectionDebounce(windowState);
    const messages = readSelectionPayloads(windowState);
    assert.strictEqual(messages.length, 1);
    const payload = messages[0].payload;
    assert.strictEqual(payload.source_path, "app/Test.app/Page.spg");
    assert.deepStrictEqual(payload.selected_component_ids, ["input1", "button1"]);
    assert.deepStrictEqual(payload.selected_component_types, ["input", "button"]);
    assert.strictEqual(payload.active_component_id, "input1");
    assert.strictEqual(payload.selected_count, 2);
    assert.strictEqual(payload.selection_source, "selectComponents");
    assert.equal(typeof payload.changed_at, "number");
  });

  it("patches deselectComponents and deselectAll", async () => {
    const bridge = createBridgeWithContext({
      source_path: "app/Test.app/Page.spg",
    });
    const windowState = createFakeWindow({ bridge });
    const builder = windowState.createBuilder([
      { id: "input1", type: "input", name: "A" },
      { id: "button1", type: "button", name: "B" },
      { id: "canvas1", type: "canvas", name: "C" },
    ]);
    delete builder.doSelectedChange;
    bridge.getPageContext = () => ({
      page_context: {
        source_path: "app/Test.app/Page.spg",
      },
      builder,
      diagnostics: [],
    });

    const script = windowState.context.__metadata_checker_page_script__;
    assert.strictEqual(script.installSelectionBridge().installed, true);

    builder.deselectComponents(["canvas1"]);
    await waitForSelectionDebounce(windowState);
    const first = readSelectionPayloads(windowState).at(-1).payload;
    assert.deepStrictEqual(first.selected_component_ids, ["input1", "button1"]);
    assert.deepStrictEqual(first.selected_component_types, ["input", "button"]);
    assert.strictEqual(first.active_component_id, "input1");

    builder.deselectAll();
    await waitForSelectionDebounce(windowState);
    const second = readSelectionPayloads(windowState).at(-1).payload;
    assert.deepStrictEqual(second.selected_component_ids, []);
    assert.deepStrictEqual(second.selected_component_types, []);
    assert.strictEqual(second.active_component_id, null);
    assert.strictEqual(second.selected_count, 0);
  });

  it("patches doSelectedChange preferentially and is idempotent", async () => {
    const bridge = createBridgeWithContext({
      source_path: "app/Test.app/Flow.spg",
    });
    const windowState = createFakeWindow({ bridge });
    const builder = windowState.createBuilder([
      { id: "node1", type: "container" },
    ]);
    builder.selectComponents = function () {
      return [];
    };
    builder.deselectAll = function () {
      return [];
    };
    bridge.getPageContext = () => ({
      page_context: {
        source_path: "app/Test.app/Flow.spg",
      },
      builder,
      diagnostics: [],
    });

    const script = windowState.context.__metadata_checker_page_script__;
    const first = script.installSelectionBridge();
    const second = script.installSelectionBridge();
    assert.deepStrictEqual(first.alreadyInstalled || false, false);
    assert.deepStrictEqual(second.installed, true);
    assert.deepStrictEqual(second.alreadyInstalled, true);

    builder.doSelectedChange();
    await waitForSelectionDebounce(windowState);
    const messages = readSelectionPayloads(windowState);
    assert.strictEqual(messages.length, 1);
    const payload = messages[0].payload;
    assert.strictEqual(payload.selection_source, "doSelectedChange");
    assert.strictEqual(payload.active_component_id, "node1");
  });

  it("does not emit sensitive component fields", async () => {
    const bridge = createBridgeWithContext({
      source_path: "app/Test.app/Secret.spg",
    });
    const windowState = createFakeWindow({ bridge });
    const builder = windowState.createBuilder();
    delete builder.doSelectedChange;
    bridge.getPageContext = () => ({
      page_context: {
        source_path: "app/Test.app/Secret.spg",
      },
      builder,
      diagnostics: [],
    });

    const script = windowState.context.__metadata_checker_page_script__;
    assert.strictEqual(script.installSelectionBridge().installed, true);

    builder.selectComponents([
      {
        id: "secret",
        type: "secret",
        name: "secret",
        raw_text: "raw raw raw",
        rawText: "raw raw raw raw",
        component_json: "{}",
        raw_component: { foo: "bar" },
        components: [{ id: "x" }],
        password: "pw",
        token: "tk",
        cookie: "ck",
      },
    ]);

    await waitForSelectionDebounce(windowState);
    const payload = readSelectionPayloads(windowState).at(-1).payload;
    const serialized = JSON.stringify(payload);
    assert.equal(serialized.includes("raw_text"), false);
    assert.equal(serialized.includes("rawText"), false);
    assert.equal(serialized.includes("component_json"), false);
    assert.equal(serialized.includes("raw_component"), false);
    assert.equal(serialized.includes("components"), false);
    assert.equal(serialized.includes("password"), false);
    assert.equal(serialized.includes("token"), false);
    assert.equal(serialized.includes("cookie"), false);
  });

  it("reports missing builder via marker without throwing", () => {
    const bridge = createBridgeWithContext({
      source_path: "app/Missing.spg",
    });
    const windowState = createFakeWindow({ bridge });
    const script = windowState.context.__metadata_checker_page_script__;
    const result = script.installSelectionBridge();
    assert.equal(result.installed, false);
    assert.equal(result.reason, "builder_missing");
    assert.equal(markerValue(windowState.document, "selection-bridge"), "missing");
    assert.equal(
      markerValue(windowState.document, "selection-bridge-diagnostic"),
      "METADATA_CHECKER_SELECTION_BRIDGE_MISSING:builder_missing",
    );
  });

  it("supports designer context forms for builder location", () => {
    const bridge = createBridgeWithContext({
      source_path: "app/Test.app/Context.spg",
    });
    const windowState = createFakeWindow({ bridge });
    const builder = windowState.createBuilder([{ id: "context-node", type: "box" }]);
    bridge.getPageContext = () => ({
      page_context: { source_path: "app/Test.app/Context.spg" },
      designer: {
        getCurrentPage() {
          return {
            getBuilder() {
              return builder;
            },
          };
        },
      },
      diagnostics: [],
    });

    const script = windowState.context.__metadata_checker_page_script__;
    const result = script.installSelectionBridge();
    assert.equal(result.installed, true);
    assert.equal(result.method, "doSelectedChange");
  });

  it("uses custom bridge builder helper when page context is intentionally lightweight", () => {
    const bridge = createBridgeWithContext({
      source_path: "app/Test.app/CustomBridge.spg",
    });
    const windowState = createFakeWindow({ bridge });
    const builder = windowState.createBuilder([{ id: "custom-node", type: "input" }]);
    bridge.getSelectionBridgeBuilder = () => builder;

    const script = windowState.context.__metadata_checker_page_script__;
    const result = script.installSelectionBridge();
    assert.equal(result.installed, true);
    assert.equal(result.method, "doSelectedChange");
    assert.equal(markerValue(windowState.document, "selection-bridge"), "installed");
  });

  it("debounces rapid selection changes and emits the latest full selection only", async () => {
    const bridge = createBridgeWithContext({
      source_path: "app/Test.app/Debounce.spg",
    });
    const windowState = createFakeWindow({ bridge });
    const builder = windowState.createBuilder();
    delete builder.doSelectedChange;
    bridge.getPageContext = () => ({
      page_context: {
        source_path: "app/Test.app/Debounce.spg",
      },
      builder,
      diagnostics: [],
    });

    const script = windowState.context.__metadata_checker_page_script__;
    assert.strictEqual(script.installSelectionBridge().installed, true);

    builder.selectComponents([{ id: "first", type: "input" }]);
    builder.selectComponents([{ id: "second", type: "text" }]);
    builder.selectComponents([
      { id: "third", type: "button" },
      { id: "fourth", type: "container" },
    ]);

    assert.strictEqual(readSelectionPayloads(windowState).length, 0);
    await waitForSelectionDebounce(windowState);

    const messages = readSelectionPayloads(windowState);
    assert.strictEqual(messages.length, 1);
    assert.deepStrictEqual(messages[0].payload.selected_component_ids, ["third", "fourth"]);
    assert.deepStrictEqual(messages[0].payload.selected_component_types, ["button", "container"]);
    assert.strictEqual(messages[0].payload.active_component_id, "third");
    assert.strictEqual(messages[0].payload.selected_count, 2);
  });
});

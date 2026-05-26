/**
 * M43.2 custom bridge bootstrap smoke tests
 */

import { describe, it } from "node:test";
import assert from "node:assert";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import vm from "node:vm";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const customSourcePath = join(__dirname, "../bridge/custom-bridge.js");
const protocolSourcePath = join(__dirname, "../bridge/metadata-checker-bridge.js");

function createMockDocument() {
  const markerNodes = [];
  const listeners = new Map();

  const body = {
    children: markerNodes,
    appendChild(node) {
      markerNodes.push(node);
      return node;
    },
  };

  function createElement() {
    const attributes = new Map();
    return {
      attributes,
      style: {},
      setAttribute(name, value) {
        attributes.set(name, String(value));
      },
      getAttribute(name) {
        return attributes.get(name) ?? null;
      },
      hasAttribute(name) {
        return attributes.has(name);
      },
    };
  }

  return {
    body,
    createElement,
    querySelector(selector) {
      const match = selector.match(/^\[([^\]]+)\]$/);
      if (!match) {
        return null;
      }
      const key = match[1];
      return markerNodes.find((node) => node.hasAttribute?.(key)) ?? null;
    },
    addEventListener(type, handler) {
      const bucket = listeners.get(type) ?? [];
      bucket.push(handler);
      listeners.set(type, bucket);
    },
    dispatchEvent(event) {
      const handlers = listeners.get(event.type) ?? [];
      for (const handler of handlers) {
        handler(event);
      }
    },
  };
}

function marker(document, name) {
  const key = `data-metadata-checker-${name}`;
  return document.querySelector(`[${key}]`)?.getAttribute(key) ?? null;
}

function loadCustomBridgeWithProtocol() {
  const protocolSource = readFileSync(protocolSourcePath, "utf-8");
  const customSource = readFileSync(customSourcePath, "utf-8");
  const document = createMockDocument();

  const window = {
    __metadata_checker_bridge_protocol__: null,
    setTimeout(...args) {
      return setTimeout(...args);
    },
  };

  const context = {
    console: { log() {}, warn() {}, error() {}, },
    window,
    document,
    navigator: {},
    CustomEvent: class {
      constructor(type, init) {
        this.type = type;
        this.detail = init?.detail;
      }
    },
    define(deps, factory) {
      if (typeof deps === "function") {
        context.module = deps();
        return;
      }
      const exports = {};
      context.module = exports;
      const args = deps.map((dep) => {
        if (dep === "require") {
          return () => null;
        }
        if (dep === "exports") {
          return exports;
        }
        return null;
      });
      const returned = factory(...args);
      if (returned) {
        context.module = returned;
      }
    },
    setTimeout,
    clearTimeout,
    String,
    Array,
    Object,
    Date,
    JSON,
  };
  context.globalThis = context;

  vm.runInNewContext(protocolSource, context, { filename: protocolSourcePath });
  window.__metadata_checker_bridge_protocol__ = context.__metadata_checker_bridge_protocol__;
  vm.runInNewContext(customSource, context, { filename: customSourcePath });

  return {
    module: context.module,
    document,
    window,
  };
}

function loadCustomBridgeAfterExtensionCoreProtocol() {
  const extensionProtocolSource = readFileSync(
    join(__dirname, "../extension-core/bridge-protocol.js"),
    "utf-8",
  );
  const customSource = readFileSync(customSourcePath, "utf-8");
  const document = createMockDocument();
  const window = {};
  const context = {
    console: { log() {}, warn() {}, error() {}, },
    window,
    document,
    navigator: {},
    CustomEvent: class {
      constructor(type, init) {
        this.type = type;
        this.detail = init?.detail;
      }
    },
    define(deps, factory) {
      if (typeof deps === "function") {
        context.module = deps();
        return;
      }
      const exports = {};
      context.module = exports;
      const args = deps.map((dep) => {
        if (dep === "require") {
          return () => null;
        }
        if (dep === "exports") {
          return exports;
        }
        return null;
      });
      const returned = factory(...args);
      if (returned) {
        context.module = returned;
      }
    },
    setTimeout,
    clearTimeout,
    String,
    Array,
    Object,
    Date,
    JSON,
  };
  context.globalThis = context;
  vm.runInNewContext(extensionProtocolSource, context);
  window.__metadata_checker_bridge_protocol__ = context.__metadata_checker_bridge_protocol__;
  vm.runInNewContext(customSource, context, { filename: customSourcePath });
  return { module: context.module, document, window };
}

function makeBuilder(selectedComponents = []) {
  return {
    getSelectedComponents() {
      return selectedComponents;
    },
    getSelectedComponent() {
      return selectedComponents[0] || null;
    },
    getSelectedComponentInfo(id) {
      if (!id) {
        return null;
      }
      return {
        id,
        floatInfo: { left: 10, top: 20 },
        raw_text: "{\"raw\":\"metadata\"}",
        components: [{ id, hidden: true }],
      };
    },
  };
}

function makeDesigner(path = "/analyzer/app/M43.spg") {
  return {
    type: "superpage",
    openFileArgs: {
      path,
      id: "fid-001",
      projectName: "analyzer",
      mode: "edit",
    },
    getBuilder() {
      return makeBuilder();
    },
  };
}

describe("metadata-checker custom bridge", () => {
  it("loads AMD factory", () => {
    const { module, document } = loadCustomBridgeWithProtocol();
    assert.ok(module);
    assert.strictEqual(typeof module.onInitDesigner, "function");
    assert.strictEqual(module.default, module);
    assert.strictEqual(typeof module.CustomJS["*"].onInitDesigner, "function");
    assert.strictEqual(typeof module.CustomJS.spg.onInitDesigner, "function");
    assert.strictEqual(typeof module.CustomJS.SuperPage.onInitDesigner, "function");
    assert.strictEqual(marker(document, "bridge-module"), "loaded");
  });

  it("onInitDesigner writes bridge markers and emits light-weight ready event", () => {
    const { module, document } = loadCustomBridgeWithProtocol();
    const readyEvents = [];
    document.addEventListener("__metadata_checker_designer_ready__", (event) => {
      readyEvents.push(event);
    });

    const result = module.onInitDesigner(makeDesigner("/analyzer/app/surface.spg"), { isEditMode: true });
    const detail = result;

    assert.strictEqual(readyEvents.length, 1);
    assert.strictEqual(marker(document, "bridge"), "installed");
    assert.strictEqual(marker(document, "on-init-designer"), "called");
    assert.strictEqual(marker(document, "bridge-protocol"), "m43-protocol-v1");
    assert.strictEqual(marker(document, "bridge-status"), "ready");
    assert.deepStrictEqual(Object.keys(detail).sort(), [
      "diagnostics",
      "page_context",
      "protocol",
      "selection",
    ]);
    assert.deepStrictEqual(Object.keys(readyEvents[0].detail).sort(), [
      "diagnostics",
      "page_context",
      "protocol",
      "selection",
    ]);
  });

  it("does not crash when extension-core protocol is loaded before custom bridge", () => {
    const { module, document } = loadCustomBridgeAfterExtensionCoreProtocol();
    const readyEvents = [];
    document.addEventListener("__metadata_checker_designer_ready__", (event) => {
      readyEvents.push(event);
    });

    const result = module.onInitDesigner(makeDesigner("/analyzer/app/order.spg"), {});

    assert.strictEqual(result.protocol.version, "m43-protocol-v1");
    assert.strictEqual(readyEvents.length, 1);
    assert.strictEqual(marker(document, "bridge"), "installed");
  });

  it("onInitDesigner is idempotent and updates designer reference", () => {
    const { module, document, window } = loadCustomBridgeWithProtocol();
    const first = module.onInitDesigner(makeDesigner("/analyzer/app/first.spg"), {});
    const bridge = window.__metadata_checker_designer_bridge__;
    const second = module.onInitDesigner(makeDesigner("/analyzer/app/second.spg"), {});

    assert.strictEqual(window.__metadata_checker_designer_bridge__, bridge);
    assert.strictEqual(bridge.getPageContext().page_context.source_path, "app/second.spg");
    assert.strictEqual(marker(document, "bridge-status"), "updated");
    assert.strictEqual(first.protocol.version, "m43-protocol-v1");
    assert.strictEqual(second.protocol.version, "m43-protocol-v1");
  });

  it("getSelectionSnapshot has no-selection diagnostic when selection is empty", () => {
    const { module, window } = loadCustomBridgeWithProtocol();
    const bridge = module
      .onInitDesigner({
        type: "superpage",
        openFileArgs: {
          path: "/analyzer/app/empty.spg",
          id: "fid-empty",
          projectName: "analyzer",
        },
        getBuilder() {
          return makeBuilder([]);
        },
      });

    assert.ok(bridge);
    const snapshot = window.__metadata_checker_designer_bridge__.getSelectionSnapshot();
    assert.ok(Array.isArray(snapshot.selection.selected_component_ids));
    assert.strictEqual(snapshot.selection.selected_component_ids.length, 0);
    assert.ok(snapshot.diagnostics.some((item) => item.code === "BRIDGE_NO_SELECTION"));
  });

  it("selection snapshot and ready event block raw metadata/component JSON", () => {
    const { module, document, window } = loadCustomBridgeWithProtocol();
    const readyEvents = [];
    document.addEventListener("__metadata_checker_designer_ready__", (event) => {
      readyEvents.push(event);
    });

    const result = module.onInitDesigner({
      type: "superpage",
      openFileArgs: {
        path: "/analyzer/app/secret.spg",
        id: "fid-secret",
        projectName: "analyzer",
      },
      getBuilder() {
        return makeBuilder([{ id: "hero" }, { id: "other" }]);
      },
    });

    const snapshot = window.__metadata_checker_designer_bridge__.getSelectionSnapshot();
    const rawKeys = ["raw_text", "components", "raw_component", "rawComponent", "component_json"];
    const serialized = JSON.stringify(snapshot);

    for (const key of rawKeys) {
      assert.ok(!serialized.includes(`"${key}"`), `${key} should not leak`);
    }

    assert.strictEqual(result.selection.selected_component_ids.length, 2);
    assert.strictEqual(result.selection.selected_component_ids[0], "hero");
    assert.ok(snapshot.selection.selected_component_infos.hero.float_info);
    assert.strictEqual(readyEvents[0].detail.selection.selected_component_ids[0], "hero");
  });
});

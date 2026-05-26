import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";

const require = createRequire(import.meta.url);
const { createPanelHost } = require("../extension-core/panel-host.js");

function createElement(tagName) {
  const element = {
    tagName,
    style: {},
    children: [],
    attributes: {},
    listeners: {},
    textContent: "",
    parentNode: null,
    createTextNode(value) {
      return {
        textContent: String(value),
      };
    },
    setAttribute(name, value) {
      this.attributes[name] = String(value);
    },
    getAttribute(name) {
      return this.attributes[name] ?? null;
    },
    addEventListener(type, handler) {
      this.listeners[type] = handler;
    },
    dispatchEvent(event) {
      const handler = this.listeners[event.type];
      if (typeof handler === "function") {
        handler(event);
      }
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
      root.children = [];
      return root;
    },
  };

  return element;
}

function createDocument(hasBody = true) {
  const body = hasBody
    ? createElement("body")
    : null;

  return {
    body,
    createElement,
  };
}

function collectText(root) {
  const values = [];
  const walk = (node) => {
    if (!node) {
      return;
    }
    if (typeof node.textContent === "string") {
      values.push(node.textContent);
    }
    const childList = node.children || [];
    for (const child of childList) {
      walk(child);
    }
  };
  walk(root);
  return values.join("\n");
}

test("createPanelHost mountPanel is idempotent and marker on initial hidden panel", () => {
  const host = createPanelHost();
  const document = createDocument(true);

  const first = host.mountPanel({ rootDocument: document });
  const second = host.mountPanel({ rootDocument: document });

  assert.equal(first.mounted, true);
  assert.equal(second.mounted, true);
  assert.equal(second.alreadyMounted, true);
  assert.equal(document.body.children.length, 1);
  const root = host.getState().hostElement;
  assert.ok(root);
  assert.equal(root.getAttribute("data-metadata-checker-panel"), "hidden");
  assert.equal(root.getAttribute("data-metadata-checker-panel-trigger"), "mounted");
  assert.equal(host.getState().triggerButton.getAttribute("data-metadata-checker-panel-trigger"), "mounted");
});

test("createPanelHost mounts error state when body is missing", () => {
  const host = createPanelHost();
  const document = createDocument(false);

  const result = host.mountPanel({ rootDocument: document });

  assert.equal(result.mounted, false);
  assert.equal(result.error.status, "error");
  assert.equal(result.error.diagnostics[0].code, "PANEL_HOST_BODY_MISSING");
  assert.equal(host.getState().mounted, false);
});

test("createPanelHost toggles visible and hidden via trigger and API", () => {
  const host = createPanelHost();
  const document = createDocument(true);
  host.mountPanel({ rootDocument: document });

  const firstToggle = host.togglePanel();
  assert.equal(firstToggle.visible, true);
  const stateAfterShow = host.getState();
  assert.equal(stateAfterShow.hostElement.getAttribute("data-metadata-checker-panel"), "mounted");
  assert.equal(stateAfterShow.visible, true);
  assert.equal(stateAfterShow.panelElement.style.display, "");

  const secondToggle = host.togglePanel(false);
  assert.equal(secondToggle.visible, false);
  const stateAfterHide = host.getState();
  assert.equal(stateAfterHide.hostElement.getAttribute("data-metadata-checker-panel"), "hidden");
  assert.equal(stateAfterHide.visible, false);
  assert.equal(stateAfterHide.panelElement.style.display, "none");

  const trigger = stateAfterHide.triggerButton;
  trigger.dispatchEvent({ type: "click" });
  assert.equal(host.getState().visible, true);
});

test("createPanelHost keeps selection update while hidden and displays latest on expand", () => {
  const host = createPanelHost();
  const document = createDocument(true);
  host.mountPanel({
    rootDocument: document,
    initialState: {
      selection: {
        source_path: "first.spg",
        selected_component_ids: ["a"],
        active_component_id: "a",
      },
      envelope: {
        status: "ready",
        target: "init",
        items: [1],
      },
    },
  });

  host.updatePanel({ status: "ready", target: "analysis-v1", items: [] });
  host.updateSelection({
    source_path: "hidden.spg",
    selected_component_ids: ["x", "y", "z"],
    active_component_id: "z",
  });

  const hiddenState = host.getState();
  assert.equal(hiddenState.visible, false);
  assert.equal(hiddenState.sourcePath, "hidden.spg");
  assert.equal(hiddenState.selectionCount, 3);

  host.togglePanel(true);
  const visibleState = host.getState();
  const text = collectText(visibleState.contentElement);
  assert.match(text, /Source: hidden\.spg/);
  assert.match(text, /Selection: 3/);
  assert.match(text, /Active: z/);
});

test("createPanelHost never renders raw payload fields into DOM text", () => {
  const host = createPanelHost();
  const document = createDocument(true);
  host.mountPanel({ rootDocument: document });
  host.togglePanel(true);

  const sensitiveSelection = {
    source_path: "safe.spg",
    selected_component_ids: ["n1"],
    raw_component: { raw_text: "raw-secret" },
    raw_metadata: "should-never-leak",
    components: ["raw list"],
  };
  host.updateSelection(sensitiveSelection);
  host.updatePanel({
    status: "ready",
    target: "t1",
    diagnostics: [
      {
        code: "TEST",
        message: "ok",
      },
    ],
    raw_text: "raw-text-secret",
  });

  const text = collectText(host.getState().contentElement);
  assert.match(text, /Status: ready/);
  assert.match(text, /Source: safe.spg/);
  assert.doesNotMatch(text, /raw-secret/);
  assert.doesNotMatch(text, /should-never-leak/);
  assert.doesNotMatch(text, /raw-text-secret/);
  assert.doesNotMatch(text, /raw list/);
});

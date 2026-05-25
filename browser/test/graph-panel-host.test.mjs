import { describe, it } from "node:test";
import assert from "node:assert";
import { createGraphPanelRenderer } from "../renderer/graph-panel-renderer.mjs";
import { createGraphPanelHost } from "../renderer/graph-panel-host.mjs";

function createFakeDocument() {
  function createElement(tagName) {
    const element = {
      tagName,
      className: "",
      children: [],
      attributes: {},
      style: {},
      listeners: {},
      textContent: "",
      appendChild(child) {
        this.children.push(child);
        child.parentNode = this;
      },
      replaceChildren() {
        this.children = [];
      },
      remove() {
        if (!this.parentNode?.children) return;
        const index = this.parentNode.children.indexOf(this);
        if (index >= 0) {
          this.parentNode.children.splice(index, 1);
        }
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
        if (handler) handler(event);
      },
    };
    return element;
  }
  return { body: createElement("body"), createElement };
}

function createRenderer() {
  const calls = [];
  return {
    calls,
    async render(result) {
      calls.push({ method: "render", result });
      return { ok: true, result };
    },
    async renderError(errorEnvelope) {
      calls.push({ method: "renderError", errorEnvelope });
      return { ok: false, errorEnvelope };
    },
  };
}

describe("createGraphPanelHost", () => {
  it("mounts once and writes panel/status markers", () => {
    const document = createFakeDocument();
    const host = createGraphPanelHost({ document });

    const first = host.mount();
    const second = host.mount();

    assert.strictEqual(first.mounted, true);
    assert.strictEqual(second.alreadyMounted, true);
    assert.strictEqual(document.body.children.length, 1);
    assert.strictEqual(
      first.root.getAttribute("data-metadata-checker-graph-panel"),
      "mounted",
    );
    assert.strictEqual(
      first.root.getAttribute("data-metadata-checker-analysis-status"),
      "idle",
    );
  });

  it("collapses and expands without removing panel", () => {
    const document = createFakeDocument();
    const host = createGraphPanelHost({ document });
    const { root, body } = host.mount();

    host.setCollapsed(true);
    assert.strictEqual(body.style.display, "none");
    assert.strictEqual(
      root.getAttribute("data-metadata-checker-graph-panel"),
      "hidden",
    );
    assert.strictEqual(
      root.getAttribute("data-metadata-checker-graph-panel-collapsed"),
      "true",
    );

    host.setCollapsed(false);
    assert.strictEqual(body.style.display, "");
    assert.strictEqual(
      root.getAttribute("data-metadata-checker-graph-panel"),
      "mounted",
    );
  });

  it("delegates ready render and updates status", async () => {
    const document = createFakeDocument();
    const renderer = createRenderer();
    const host = createGraphPanelHost({ document, renderer });

    const result = await host.render({ status: "ready", target: "comp1" });

    assert.strictEqual(result.ok, true);
    assert.strictEqual(renderer.calls.length, 1);
    assert.strictEqual(renderer.calls[0].method, "render");
    assert.strictEqual(
      host.status().root.getAttribute("data-metadata-checker-analysis-status"),
      "ready",
    );
  });

  it("binds a real graph renderer to the mounted panel body", async () => {
    const document = createFakeDocument();
    const renderer = createGraphPanelRenderer({ document });
    const host = createGraphPanelHost({ document, renderer });

    const rendered = await host.render({
      status: "ready",
      target: "comp1",
      items: [
        {
          kind: "visual_graph",
          label: "Visual Graph",
          detail: {
            nodes: [
              { id: "comp1", label: "comp1", kind: "Component", metadata: { depth: 0 } },
              { id: "model1", label: "model1", kind: "Model", metadata: { depth: 1 } },
            ],
            edges: [
              { from: "comp1", to: "model1", kind: "Reads", direction: "Forward", label: "reads" },
            ],
            groups: [],
            focus_node: "comp1",
            diagnostics: [],
            truncated: false,
            source_summary: {
              total_nodes: 2,
              total_edges: 1,
              node_kinds: {},
              edge_kinds: {},
            },
          },
        },
      ],
      diagnostics: [],
    });

    const { root, body } = host.status();
    assert.strictEqual(rendered.graph.nodes.length, 2);
    assert.ok(body.children.length > 0);
    assert.strictEqual(body.getAttribute("data-metadata-checker-graph-nodes"), "2");
    assert.strictEqual(body.getAttribute("data-metadata-checker-graph-edges"), "1");
    assert.strictEqual(root.getAttribute("data-metadata-checker-analysis-status"), "ready");
  });

  it("delegates error render and updates status", async () => {
    const document = createFakeDocument();
    const renderer = createRenderer();
    const host = createGraphPanelHost({ document, renderer });

    const error = {
      status: "error",
      diagnostics: [{ severity: "error", code: "X", message: "failed" }],
    };
    const result = await host.renderError(error);

    assert.strictEqual(result.ok, false);
    assert.strictEqual(renderer.calls[0].method, "renderError");
    assert.strictEqual(
      host.status().root.getAttribute("data-metadata-checker-analysis-status"),
      "error",
    );
  });

  it("unmounts panel", () => {
    const document = createFakeDocument();
    const host = createGraphPanelHost({ document });
    host.mount();

    assert.strictEqual(document.body.children.length, 1);
    host.unmount();

    assert.strictEqual(document.body.children.length, 0);
    assert.strictEqual(host.status().mounted, false);
  });

  it("returns stable diagnostic when document is unavailable", async () => {
    const host = createGraphPanelHost({ document: null, parent: null });
    const mounted = host.mount();
    const rendered = await host.render({ status: "ready" });

    assert.strictEqual(mounted.mounted, false);
    assert.strictEqual(
      mounted.error.diagnostics[0].code,
      "GRAPH_PANEL_HOST_UNAVAILABLE",
    );
    assert.strictEqual(rendered.status, "error");
    assert.strictEqual(
      rendered.diagnostics[0].code,
      "GRAPH_PANEL_HOST_UNAVAILABLE",
    );
  });
});

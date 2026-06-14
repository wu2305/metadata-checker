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
      querySelector(selector) {
        if (!selector || !selector.startsWith(".")) {
          return null;
        }
        const className = selector.slice(1);
        const matches = collectByClass(this, className);
        return matches[0] ?? null;
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
    async setContainer(container) {
      calls.push({ method: "setContainer", container });
    },
    async render(result) {
      calls.push({ method: "render", result });
      const nodeCount = Array.isArray(result?.nodes)
        ? result.nodes.length
        : Array.isArray(result?.items?.[0]?.detail?.nodes)
          ? result.items[0].detail.nodes.length
          : 0;
      const edgeCount = Array.isArray(result?.edges)
        ? result.edges.length
        : Array.isArray(result?.items?.[0]?.detail?.edges)
          ? result.items[0].detail.edges.length
          : 0;
      return {
        status: result?.status ?? "ready",
        focus: result?.focus_node ?? result?.target,
        depth: result?.depth ?? 1,
        visibleHop: result?.visible_hop ?? 1,
        nodeCount,
        edgeCount,
        graph: {
          nodes: Array.isArray(result?.nodes)
            ? result.nodes
            : Array.isArray(result?.items?.[0]?.detail?.nodes)
              ? result.items[0].detail.nodes
              : [],
          edges: Array.isArray(result?.edges)
            ? result.edges
            : Array.isArray(result?.items?.[0]?.detail?.edges)
              ? result.items[0].detail.edges
              : [],
          focus_node: result?.focus_node ?? result?.target,
          nodeCount,
          edgeCount,
        },
      };
    },
    async renderError(errorEnvelope) {
      calls.push({ method: "renderError", errorEnvelope });
      return {
        status: "error",
        focus: null,
        depth: 0,
        visibleHop: 0,
        nodeCount: 0,
        edgeCount: 0,
      };
    },
  };
}

function collectByClass(root, className, out = []) {
  const rawClassName = typeof root.className === "string"
    ? root.className
    : root.getAttribute?.("class") ?? "";
  if ((rawClassName || "").split(" ").includes(className)) {
    out.push(root);
  }
  if (Array.isArray(root.children)) {
    root.children.forEach((child) => collectByClass(child, className, out));
  }
  return out;
}

function createCanvasRenderer(documentLike) {
  const calls = [];
  const renderer = {
    calls,
    container: null,
    async setContainer(container) {
      calls.push({ method: "setContainer", container });
      renderer.container = container;
    },
    async render(result) {
      calls.push({ method: "render", result });
      if (renderer.container && documentLike) {
        const canvas = documentLike.createElement("canvas");
        canvas.className = "pixi-canvas-surface";
        renderer.container.appendChild(canvas);
      }
      return {
        status: result?.status ?? "ready",
        focus: result?.focus_node ?? result?.target ?? "",
        depth: result?.depth ?? 0,
        visibleHop: result?.visible_hop ?? 1,
        nodeCount: result?.nodes?.length ?? 0,
        edgeCount: result?.edges?.length ?? 0,
      };
    },
    async renderError(errorEnvelope) {
      calls.push({ method: "renderError", errorEnvelope });
      return {
        status: "error",
        focus: null,
        depth: 0,
        visibleHop: 1,
        nodeCount: 0,
        edgeCount: 0,
      };
    },
  };
  return renderer;
}

function createCanvasResultFixture() {
  return {
    status: "ready",
    depth: 2,
    visible_hop: 1,
    target: "comp1",
    focus_node: "comp1",
    nodes: [
      { id: "comp1", label: "comp1", kind: "Component", metadata: { depth: 0 } },
      { id: "m1", label: "m1", kind: "Model", metadata: { depth: 1 } },
      { id: "m2", label: "m2", kind: "Model", metadata: { depth: 2 } },
    ],
    edges: [
      {
        from: "comp1",
        to: "m1",
        kind: "Reads",
        direction: "Forward",
        label: "reads",
        evidence: "order.amount",
        evidence_status: "available",
      },
      { from: "m1", to: "m2", kind: "Writes", direction: "Forward", label: "writes" },
    ],
    source_summary: {
      total_nodes: 3,
      total_edges: 2,
      node_kinds: {},
      edge_kinds: {},
    },
  };
}

function assertGraphPopupState(hostRoot, expected) {
  if (expected.focusComponent != null) {
    assert.strictEqual(hostRoot.getAttribute("data-metadata-checker-focus-component"), expected.focusComponent);
  }
  if (expected.status != null) {
    assert.strictEqual(hostRoot.getAttribute("data-metadata-checker-analysis-status"), expected.status);
  }
  if (expected.embeddedPopup != null) {
    assert.strictEqual(hostRoot.getAttribute("data-metadata-checker-embedded-popup"), expected.embeddedPopup);
  }
  if (expected.depth != null) {
    assert.strictEqual(hostRoot.getAttribute("data-metadata-checker-graph-depth"), String(expected.depth));
  }
  if (expected.visibleHop != null) {
    assert.strictEqual(
      hostRoot.getAttribute("data-metadata-checker-graph-visible-hop"),
      String(expected.visibleHop),
    );
  }
  if (expected.nodeCount != null) {
    assert.strictEqual(hostRoot.getAttribute("data-metadata-checker-graph-node-count"), String(expected.nodeCount));
  }
  if (expected.edgeCount != null) {
    assert.strictEqual(hostRoot.getAttribute("data-metadata-checker-graph-edge-count"), String(expected.edgeCount));
  }
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
    assert.strictEqual(
      first.root.getAttribute("data-metadata-checker-embedded-popup"),
      "mounted",
    );
    assert.strictEqual(
      first.root.getAttribute("data-metadata-checker-graph-depth"),
      "0",
    );
    assert.strictEqual(
      first.root.getAttribute("data-metadata-checker-graph-visible-hop"),
      "1",
    );
    assert.strictEqual(
      first.root.getAttribute("data-metadata-checker-graph-node-count"),
      "0",
    );
    assert.strictEqual(
      first.root.getAttribute("data-metadata-checker-graph-edge-count"),
      "0",
    );
  });

  it("writes all required embedded markers on render", async () => {
    const document = createFakeDocument();
    const renderer = createGraphPanelRenderer({ document });
    const host = createGraphPanelHost({ document, renderer });
    await host.render(createCanvasResultFixture());
    const { root } = host.status();

    assert.strictEqual(root.getAttribute("data-metadata-checker-embedded-popup"), "mounted");
    assert.strictEqual(root.getAttribute("data-metadata-checker-analysis-status"), "ready");
    assert.strictEqual(root.getAttribute("data-metadata-checker-focus-component"), "comp1");
    assert.strictEqual(root.getAttribute("data-metadata-checker-graph-depth"), "2");
    assert.strictEqual(root.getAttribute("data-metadata-checker-graph-visible-hop"), "1");
    assert.strictEqual(root.getAttribute("data-metadata-checker-graph-node-count"), "3");
    assert.strictEqual(root.getAttribute("data-metadata-checker-graph-edge-count"), "2");
    assert.strictEqual(root.getAttribute("data-metadata-checker-local-graph-renderer"), "html");
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
      root.getAttribute("data-metadata-checker-embedded-popup"),
      "collapsed",
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
    assert.strictEqual(
      root.getAttribute("data-metadata-checker-embedded-popup"),
      "mounted",
    );
  });

  it("delegates ready render and updates status", async () => {
    const document = createFakeDocument();
    const renderer = createRenderer();
    const host = createGraphPanelHost({ document, renderer });

    const result = await host.render(createCanvasResultFixture());

    const callLog = renderer.calls.map((entry) => entry.method);
    assert.deepStrictEqual(callLog, ["setContainer", "render"]);
    assert.strictEqual(result.status, "ready");
    assertGraphPopupState(host.status().root, {
      status: "ready",
      embeddedPopup: "mounted",
      focusComponent: "comp1",
      depth: 2,
      visibleHop: 1,
      nodeCount: 3,
      edgeCount: 2,
    });
    assert.strictEqual(host.status().root.getAttribute("data-metadata-checker-local-graph-renderer"), "fallback");
    assert.strictEqual(host.status().body.children.length, 0);
  });

  it("shows loading shell before graph render completes", () => {
    const document = createFakeDocument();
    const host = createGraphPanelHost({ document, renderer: null });
    const result = host.showShell({
      focus: "comp1",
      depth: 2,
      visibleHop: 1,
      nodeCount: 0,
      edgeCount: 0,
    });
    const { root } = host.status();

    assert.equal(result.status, "loading");
    assert.equal(result.mounted, true);
    assertGraphPopupState(root, {
      status: "loading",
      embeddedPopup: "mounted",
      focusComponent: "comp1",
      depth: 2,
      visibleHop: 1,
      nodeCount: 0,
      edgeCount: 0,
    });
  });

  it("mounts a canvas renderer and records visible graph markers", async () => {
    const document = createFakeDocument();
    const renderer = createCanvasRenderer(document);
    const host = createGraphPanelHost({ document, renderer });
    const result = await host.render(createCanvasResultFixture());

    const { root, body } = host.status();
    const canvasCount = body.children.filter((child) => child.tagName === "canvas").length;

    assert.equal(result.status, "ready");
    assert.equal(canvasCount, 1);
    assertGraphPopupState(root, {
      status: "ready",
      embeddedPopup: "mounted",
      focusComponent: "comp1",
      depth: 2,
      visibleHop: 1,
      nodeCount: 3,
      edgeCount: 2,
    });
    assert.strictEqual(root.getAttribute("data-metadata-checker-local-graph-renderer"), "fallback");
    const expectedPanelStyles = {
      position: "fixed",
      right: "16px",
      bottom: "16px",
      width: "min(280px, calc(100vw - 32px))",
      height: "min(260px, calc(100vh - 32px))",
      maxWidth: "calc(100vw - 32px)",
      maxHeight: "calc(100vh - 32px)",
      overflow: "hidden",
      zIndex: "2147483647",
    };
    Object.entries(expectedPanelStyles).forEach(([key, value]) => {
      assert.equal(root.style[key], value);
    });
    assert.equal(root.getAttribute("data-metadata-checker-graph-panel-theme"), "inspector-dark");
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
              {
                from: "comp1",
                to: "model1",
                kind: "Reads",
                direction: "Forward",
                label: "reads",
                evidence: "order.id",
                evidence_status: "available",
              },
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

    assert.strictEqual(result.status, "error");
    const callLog = renderer.calls.map((entry) => entry.method);
    assert.deepStrictEqual(callLog, ["setContainer", "renderError"]);
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

  it("passes renderer open-detail context and kind markers to host markers", async () => {
    const document = createFakeDocument();
    const renderer = createGraphPanelRenderer({ document });
    const host = createGraphPanelHost({ document, renderer });

    await host.render({
      status: "ready",
      target: "focus",
      nodes: [
        { id: "focus", label: "focus", kind: "Component", metadata: { depth: 0 } },
        { id: "token=abc", label: "token value", kind: "Model", metadata: { depth: 1 } },
      ],
      edges: [
        { id: "edge=token=xyz", from: "focus", to: "token=abc", kind: "Reads", direction: "Forward", label: "read" },
      ],
      source_summary: {
        total_nodes: 2,
        total_edges: 1,
        node_kinds: {},
        edge_kinds: {},
      },
      diagnostics: [],
      truncated: false,
      focus_node: "focus",
    });

    const { root } = host.status();
    assert.strictEqual(root.getAttribute("data-metadata-checker-graph-detail-kind"), "focus");
    assert.strictEqual(root.getAttribute("data-metadata-checker-graph-open-detail-context"), "focus:focus");

    const controller = renderer.getDomController();
    assert.ok(controller);
    controller.lockCurrentTarget({ type: "edge", id: "edge=token=xyz" });
    host.refreshRendererContext();

    assert.strictEqual(root.getAttribute("data-metadata-checker-graph-detail-kind"), "edge");
    assert.strictEqual(root.getAttribute("data-metadata-checker-graph-open-detail-context"), "edge:[sensitive]");
    controller.clearLockedTarget();
    host.refreshRendererContext();
    assert.strictEqual(root.getAttribute("data-metadata-checker-graph-detail-kind"), "focus");
  });

  it("writes visible/full node counts in header summary label", async () => {
    const document = createFakeDocument();
    const renderer = {
      async render(result) {
        return {
          status: "ready",
          focus: result.focus_node,
          renderer: "pixi",
          graph: {
            ...result,
            viewGraph: {
              visibleNodeCount: 3,
              aggregateNodeCount: 1,
              visibleEdgeCount: 2,
            },
          },
          nodeCount: result.nodes.length,
          edgeCount: result.edges.length,
        };
      },
    };
    const host = createGraphPanelHost({ document, renderer });
    await host.render({
      status: "ready",
      target: "focus",
      focus_node: "focus",
      nodes: new Array(10).fill(null).map((_, index) => ({
        id: `n${index}`,
        label: `n${index}`,
        kind: "Model",
        metadata: { depth: 1 },
      })),
      edges: new Array(8).fill(null).map((_, index) => ({
        id: `e${index}`,
        from: "focus",
        to: `n${index}`,
        kind: "Reads",
        direction: "Forward",
        label: "read",
      })),
      source_summary: { total_nodes: 10, total_edges: 8, node_kinds: {}, edge_kinds: {} },
      diagnostics: [],
      truncated: true,
      focus_node: "focus",
    });

    const counts = collectByClass(host.status().root, "metadata-checker-graph-counts");
    assert.strictEqual(counts.length, 1);
    assert.strictEqual(counts[0].textContent, "4/10");
    assert.strictEqual(counts[0].title, "4 visible / 10 total nodes");
  });

  it("toggles pin state and forwards onPinChange", async () => {
    const document = createFakeDocument();
    const pinCalls = [];
    const host = createGraphPanelHost({
      document,
      onPinChange: async ({ pinned }) => {
        pinCalls.push(Boolean(pinned));
      },
    });
    host.mount();
    const pinButtons = collectByClass(host.status().root, "metadata-checker-graph-panel-pin");
    assert.strictEqual(pinButtons.length, 1);
    assert.strictEqual(host.status().graphPinned, false);

    pinButtons[0].dispatchEvent({ type: "click" });
    assert.strictEqual(host.status().graphPinned, true);
    assert.deepStrictEqual(pinCalls, [true]);

    pinButtons[0].dispatchEvent({ type: "click" });
    assert.strictEqual(host.status().graphPinned, false);
    assert.deepStrictEqual(pinCalls, [true, false]);
  });

  it("exposes only pin, copy, and collapse footer actions", async () => {
    const document = createFakeDocument();
    const host = createGraphPanelHost({ document });
    host.mount();
    const root = host.status().root;
    assert.strictEqual(collectByClass(root, "metadata-checker-graph-panel-pin").length, 1);
    assert.strictEqual(collectByClass(root, "metadata-checker-graph-panel-copy").length, 1);
    assert.strictEqual(collectByClass(root, "metadata-checker-graph-panel-toggle").length, 1);
    assert.strictEqual(collectByClass(root, "metadata-checker-graph-panel-open-detail").length, 0);
    assert.strictEqual(collectByClass(root, "metadata-checker-graph-panel-copy-id").length, 0);
  });

  it("builds copy text from summary when renderer returns empty graph text", async () => {
    const document = createFakeDocument();
    const copyCalls = [];
    const renderer = {
      async render(result) {
        return {
          status: "ready",
          focus: result.focus_node,
          renderer: "pixi",
          graph: {
            viewGraph: { visibleNodeCount: 2, aggregateNodeCount: 0, visibleEdgeCount: 1 },
          },
          nodeCount: result.nodes.length,
          edgeCount: result.edges.length,
        };
      },
      getVisibleGraphText() {
        return "";
      },
    };
    const host = createGraphPanelHost({
      document,
      renderer,
      onCopyGraph({ text }) {
        copyCalls.push(text);
      },
    });

    await host.render({
      status: "ready",
      target: "focus",
      focus_node: "focus",
      nodes: [
        { id: "focus", label: "focus", kind: "Component", metadata: { depth: 0 } },
        { id: "m1", label: "m1", kind: "Model", metadata: { depth: 1 } },
      ],
      edges: [
        { id: "e1", from: "focus", to: "m1", kind: "Reads", direction: "Forward", label: "read" },
      ],
      source_summary: { total_nodes: 5, total_edges: 3, node_kinds: {}, edge_kinds: {} },
      diagnostics: [],
      truncated: false,
      focus_node: "focus",
    });

    const buttons = collectByClass(host.status().root, "metadata-checker-graph-panel-copy");
    buttons[0].dispatchEvent({ type: "click" });
    assert.ok(copyCalls.at(-1).includes("focus: focus"));
    assert.ok(copyCalls.at(-1).includes("visible: 2/5 nodes"));
    assert.ok(copyCalls.at(-1).includes("nodes:"));
    assert.ok(copyCalls.at(-1).includes("edges:"));
  });

  it("copies visible graph text from renderer", async () => {
    const document = createFakeDocument();
    const copyCalls = [];
    const renderer = {
      async render(result) {
        return {
          status: "ready",
          focus: result.focus_node,
          nodeCount: result.nodes.length,
          edgeCount: result.edges.length,
          graph: result,
          renderer: "svg",
        };
      },
      getVisibleGraphText() {
        return "focus: focus\nvisible: 2/2 nodes, 1/1 edges\nnodes:\n- focus\nedges:\n- focus -> m1";
      },
    };
    const host = createGraphPanelHost({
      document,
      renderer,
      onCopyGraph({ text }) {
        copyCalls.push(text);
      },
    });

    await host.render({
      status: "ready",
      target: "focus",
      nodes: [
        { id: "focus", label: "focus", kind: "Component", metadata: { depth: 0 } },
        { id: "m1", label: "model1", kind: "Model", metadata: { depth: 1 } },
      ],
      edges: [
        { id: "e1", from: "focus", to: "m1", kind: "Reads", direction: "Forward", label: "read", priority: "source" },
      ],
      source_summary: { total_nodes: 2, total_edges: 1, node_kinds: {}, edge_kinds: {} },
      diagnostics: [],
      truncated: false,
      focus_node: "focus",
    });

    const buttons = collectByClass(host.status().root, "metadata-checker-graph-panel-copy");
    assert.strictEqual(buttons.length, 1);
    buttons[0].dispatchEvent({ type: "click" });
    assert.ok(copyCalls.at(-1).includes("nodes:"));
    assert.ok(copyCalls.at(-1).includes("edges:"));
    assert.strictEqual(host.status().root.getAttribute("data-metadata-checker-graph-copy-text-length"), String(copyCalls.at(-1).length));
  });

  it("forwards keyboard accessibility events to viewport/lock handlers", async () => {
    const document = createFakeDocument();
    const renderer = createGraphPanelRenderer({ document });
    const host = createGraphPanelHost({ document, renderer });

    await host.render({
      status: "ready",
      target: "focus",
      nodes: [
        { id: "focus", label: "focus", kind: "Component", metadata: { depth: 0 } },
        { id: "node", label: "node", kind: "Model", metadata: { depth: 1 } },
      ],
      edges: [{ id: "focus->node", from: "focus", to: "node", kind: "Reads", direction: "Forward", label: "reads" }],
      source_summary: {
        total_nodes: 2,
        total_edges: 1,
        node_kinds: {},
        edge_kinds: {},
      },
      diagnostics: [],
      truncated: false,
      focus_node: "focus",
    });

    const targets = collectByClass(host.status().root, "metadata-checker-graph-a11y-target");
    assert.strictEqual(targets.length, 1);

    const controller = renderer.getDomController();
    controller.setViewportScale(1.6);
    assert.strictEqual(controller.getInteractionState().viewportScale, 1.6);

    targets[0].dispatchEvent({ type: "keydown", key: "0" });
    assert.strictEqual(controller.getInteractionState().viewportScale, 1);

    controller.lockCurrentTarget({ type: "edge", id: "focus->node" });
    host.refreshRendererContext();
    assert.strictEqual(host.status().root.getAttribute("data-metadata-checker-graph-detail-kind"), "edge");

    targets[0].dispatchEvent({ type: "keydown", key: "Escape" });
    assert.strictEqual(host.status().root.getAttribute("data-metadata-checker-graph-detail-kind"), "focus");

    targets[0].dispatchEvent({ type: "keydown", key: "Enter" });
    assert.ok(host.status().root.getAttribute("data-metadata-checker-graph-open-detail-context"));
  });
});

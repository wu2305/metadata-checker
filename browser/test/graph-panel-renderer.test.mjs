import { describe, it } from "node:test";
import assert from "node:assert";
import { createGraphPanelRenderer, renderVisualGraph } from "../renderer/graph-panel-renderer.mjs";

function createFakeDocument() {
  function createElement(tagName) {
    const element = {
      tagName,
      style: {},
      attributes: {},
      children: [],
      className: "",
      listeners: {},
      text: "",
      set textContent(value) {
        this._text = String(value);
        this.text = String(value);
      },
      get textContent() {
        return this._text ?? "";
      },
      set innerText(value) {
        this.textContent = value;
      },
      get innerText() {
        return this.textContent;
      },
      appendChild(child) {
        this.children.push(child);
      },
      replaceChildren() {
        this.children = [];
      },
      setAttribute(name, value) {
        this.attributes[name] = String(value);
      },
      getAttribute(name) {
        return this.attributes[name] ?? null;
      },
      addEventListener(type, handler) {
        if (!this.listeners[type]) {
          this.listeners[type] = [];
        }
        this.listeners[type].push(handler);
      },
      dispatchEvent(event) {
        const list = this.listeners[event.type] || [];
        list.forEach((handler) => handler(event));
      },
      querySelector(selector) {
        if (!selector) return null;
        const className = selector.startsWith(".")
          ? selector.slice(1)
          : selector;
        return this.children.find((child) =>
          (child?.className || "").split(" ").includes(className)
        );
      },
    };
    return element;
  }

  return {
    createElement,
  };
}

function findAllByTag(root, tagName) {
  const result = [];
  const walk = (node) => {
    if (node.tagName === tagName) {
      result.push(node);
    }
    if (Array.isArray(node.children)) {
      node.children.forEach(walk);
    }
  };
  walk(root);
  return result;
}

function findByClass(root, className) {
  const result = [];
  const walk = (node) => {
    if ((node.className || "").split(" ").includes(className)) {
      result.push(node);
    }
    if (Array.isArray(node.children)) {
      node.children.forEach(walk);
    }
  };
  walk(root);
  return result;
}

function walkText(root, predicate) {
  let hit = false;
  const walk = (node) => {
    if (typeof node.textContent === "string" && predicate(node.textContent)) {
      hit = true;
    }
    if (Array.isArray(node.children)) {
      node.children.forEach(walk);
    }
  };
  walk(root);
  return hit;
}

const VIZ = {
  nodes: [
    {
      id: "focus",
      label: "focus node",
      kind: "Component",
      source_path: "app/a.spg",
      metadata: { depth: 0, importance: "high" },
    },
    {
      id: "read1",
      label: "text: \"read\"",
      kind: "Model",
      source_path: "app/b.spg",
      metadata: { depth: 1 },
    },
    {
      id: "write1",
      label: "password=bad",
      kind: "Model",
      source_path: "app/c.spg",
      metadata: { depth: 2 },
    },
    {
      id: "deep1",
      label: "deep node\nline2",
      kind: "Action",
      source_path: "app/d.spg",
      metadata: { depth: 3 },
    },
    {
      id: "deep2",
      label: "collapse me",
      kind: "Action",
      source_path: "app/e.spg",
      metadata: { depth: 4, collapsed: true },
    },
  ],
  edges: [
    { from: "focus", to: "read1", kind: "Reads", direction: "Forward", label: "reads" },
    { from: "focus", to: "write1", kind: "Writes", direction: "Forward", label: "writes" },
    { from: "write1", to: "deep1", kind: "Condition", direction: "Forward", label: "condition" },
    { from: "deep1", to: "deep2", kind: "Action", direction: "Forward", label: "action" },
  ],
  diagnostics: [
    { severity: "warning", code: "SAFE", message: "demo diagnostic", location: {} },
  ],
  source_summary: { total_nodes: 5, total_edges: 4, node_kinds: {}, edge_kinds: {} },
  truncated: true,
};

function toEnvelopeFromItems(item) {
  return {
    status: "ready",
    target: "focus",
    items: [item],
    diagnostics: [],
  };
}

describe("createGraphPanelRenderer", () => {
  it("renders visual graph and emits markers for nodes/edges/focus/truncated/depth/renderer", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({
      container,
      document,
    });

    const result = await renderer.render({
      status: "ready",
      target: "focus",
      items: [],
      diagnostics: [],
      nodes: VIZ.nodes,
      edges: VIZ.edges,
      groups: [],
      focus_node: "focus",
      source_summary: VIZ.source_summary,
      truncated: true,
    });

    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-nodes"), "5");
    assert.ok(Number.parseInt(container.getAttribute("data-metadata-checker-graph-edges"), 10) >= 3);
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-focus"), "focus");
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-truncated"), "true");
    assert.strictEqual(
      container.getAttribute("data-metadata-checker-graph-renderer") ??
        container.getAttribute("data-metadata-checker-renderer"),
      "html"
    );
    assert.ok(Number.parseInt(container.getAttribute("data-metadata-checker-graph-depth"), 10) >= 1);
    assert.ok(result?.mermaid?.includes("graph TD"));
    assert.ok(Array.isArray(container.children));
  });

  it("supports analysis envelope compatible input via items entry", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({
      container,
      document,
    });
    await renderer.render(toEnvelopeFromItems({ kind: "visual_graph", detail: VIZ }));

    const nodeCount = Number.parseInt(
      container.getAttribute("data-metadata-checker-graph-nodes"),
      10
    );
    assert.ok(nodeCount >= 1);
    const focus = container.getAttribute("data-metadata-checker-graph-focus");
    assert.strictEqual(focus, "focus");
  });

  it("weakens depth-2 and depth-3 nodes, keeps focus clear", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({
      container,
      document,
      maxDepth: 4,
    });

    await renderer.render({
      nodes: VIZ.nodes,
      edges: VIZ.edges,
      groups: [],
      focus_node: "focus",
      source_summary: VIZ.source_summary,
      diagnostics: [],
      truncated: false,
      status: "ready",
    });

    const depth2Nodes = findByClass(container, "graph-node-depth-2-3");
    assert.ok(depth2Nodes.length >= 1);
    const focusNodes = findByClass(container, "graph-node-focus");
    assert.strictEqual(focusNodes.length, 1);

    const result = await renderer.render({
      nodes: VIZ.nodes,
      edges: VIZ.edges,
      groups: [],
      focus_node: "focus",
      source_summary: VIZ.source_summary,
      diagnostics: [],
      truncated: false,
      status: "ready",
    });
    const weakEdges = (result.graph.edges || []).filter((edge) => edge.edgeClass === "weak-edge");
    assert.ok(weakEdges.length >= 1);
  });

  it("renders collapsed/expandable node click as expand_requested event without inferring relations", async () => {
    const events = [];
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({
      container,
      document,
      maxDepth: 2,
      onEvent(payload) {
        events.push(payload);
      },
    });

    await renderer.render({
      nodes: VIZ.nodes,
      edges: VIZ.edges,
      groups: [],
      focus_node: "focus",
      source_summary: VIZ.source_summary,
      diagnostics: [],
      truncated: false,
      status: "ready",
    });

    const buttons = findAllByTag(container, "button");
    assert.ok(buttons.length >= 1);
    buttons[0].dispatchEvent({ type: "click" });

    assert.strictEqual(events.length, 1);
    assert.strictEqual(events[0].type, "expand_requested");
    assert.strictEqual(events[0].event, "click");
    assert.ok(typeof events[0].nodeId === "string");
    assert.ok(events[0].nodeId);
  });

  it("keeps collapsed nodes visible and emits collapse-aware expand event", async () => {
    const events = [];
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({
      container,
      document,
      maxDepth: 2,
      onEvent(payload) {
        events.push(payload);
      },
    });

    const collapsedNode = {
      id: "collapsed",
      label: "collapsed block",
      kind: "Model",
      metadata: { depth: 1, collapsed: true },
    };
    const deepNode = {
      id: "deep",
      label: "deep node",
      kind: "Model",
      metadata: { depth: 3 },
    };

    await renderer.render({
      nodes: [
        { id: "focus", label: "focus", metadata: { depth: 0 } },
        collapsedNode,
        deepNode,
      ],
      edges: [
        { from: "focus", to: "collapsed", kind: "Reads", direction: "Forward", label: "to collapsed" },
        { from: "collapsed", to: "deep", kind: "Writes", direction: "Forward", label: "to deep" },
      ],
      groups: [],
      focus_node: "focus",
      source_summary: {
        total_nodes: 3,
        total_edges: 2,
        node_kinds: {},
        edge_kinds: {},
      },
      diagnostics: [],
      truncated: false,
      status: "ready",
    });

    const collapsedRows = findByClass(container, "graph-node-collapsed");
    assert.ok(collapsedRows.length >= 1);
    const buttons = findAllByTag(collapsedRows[0], "button");
    assert.strictEqual(buttons.length, 1);
    buttons[0].dispatchEvent({ type: "click" });

    assert.strictEqual(events.length, 1);
    assert.strictEqual(events[0].type, "expand_requested");
    assert.strictEqual(events[0].nodeId, "collapsed");
    assert.strictEqual(events[0].collapsed, true);
  });

  it("caps large graphs by node/edge budget and keeps truncated status visible", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({ container, document, maxDepth: 200 });
    const nodes = Array.from({ length: 130 }, (_, index) => ({
      id: `n${index}`,
      label: `node-${index}`,
      metadata: { depth: index === 0 ? 0 : 1 },
    }));
    const edges = [];
    for (let from = 0; from < 120; from++) {
      for (let step = 1; step <= 4; step++) {
        const to = (from + step) % 120;
        edges.push({
          from: `n${from}`,
          to: `n${to}`,
          kind: "Reads",
          direction: "Forward",
          label: "rel",
        });
      }
    }

    const result = await renderer.render({
      nodes,
      edges,
      groups: [],
      focus_node: "n0",
      source_summary: {
        total_nodes: nodes.length,
        total_edges: edges.length,
        node_kinds: {},
        edge_kinds: {},
      },
      diagnostics: [],
      truncated: false,
      status: "ready",
    });

    assert.ok(result.graph.nodes.length <= 120);
    assert.ok(result.graph.edges.length <= 360);
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-truncated"), "true");
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-truncated-reason"), "max_nodes;max_edges");
    const truncatedText = walkText(container, (text) => /truncated/i.test(text));
    assert.strictEqual(truncatedText, true);
    assert.ok(Number.parseInt(container.getAttribute("data-metadata-checker-graph-nodes"), 10) <= 120);
  });

  it("accepts Rust-style truncated_reason in visual graph input", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({ container, document });

    await renderer.render({
      nodes: [{ id: "focus", label: "focus", metadata: { depth: 0 } }],
      edges: [],
      groups: [],
      focus_node: "focus",
      source_summary: {
        total_nodes: 1,
        total_edges: 0,
        node_kinds: {},
        edge_kinds: {},
      },
      diagnostics: [],
      truncated: true,
      truncated_reason: "max_nodes",
      status: "ready",
    });

    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-truncated"), "true");
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-truncated-reason"), "max_nodes");
  });

  it("keeps sensitive keywords out of rendered labels and keeps punctuation intact", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({
      container,
      document,
    });

    const result = await renderer.render({
      nodes: [
        { id: "a", label: "username: \"alice\"\ncookie=value", metadata: { depth: 0 } },
        { id: "token=abc123", label: "token=abc123", metadata: { depth: 1 }, kind: "Model", source_path: "x" },
        { id: "c", label: "cipherPassport=xyz", metadata: { depth: 2 }, kind: "Model", source_path: "y" },
      ],
      edges: [{ from: "a", to: "token=abc123", kind: "Reads", direction: "Forward", label: "read" }],
      groups: [],
      focus_node: "a",
      source_summary: {
        total_nodes: 3,
        total_edges: 1,
        node_kinds: {},
        edge_kinds: {},
      },
      diagnostics: [{ severity: "warning", code: "SAFE", message: "password=none", location: {} }],
      truncated: false,
    });

    const hasSensitive = walkText(container, (text) =>
      /token|cookie|password|cipherpassport/i.test(text),
    );
    assert.strictEqual(hasSensitive, false);
    const nodeRows = findByClass(container, "graph-node");
    assert.strictEqual(
      nodeRows.some((row) => /token|cookie|password|cipherpassport/i.test(row.getAttribute("data-node-id") ?? "")),
      false,
    );
    assert.strictEqual(
      /token|cookie|password|cipherpassport/i.test(container.getAttribute("data-metadata-checker-graph-focus") ?? ""),
      false,
    );
    assert.strictEqual(/token=abc123/i.test(result.mermaid), false);
    assert.strictEqual(
      JSON.stringify(result.echartsOption).includes("token=abc123"),
      false,
    );
    assert.ok(walkText(container, (text) => /quote|:/i.test(text)));
    assert.ok(walkText(container, (text) => /\s/.test(text)));
  });

  it("renderVisualGraph helper returns same render result as createGraphPanelRenderer", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const envelope = {
      status: "ready",
      target: "focus",
      items: [],
      diagnostics: [],
      nodes: [
        { id: "a", label: "a", metadata: { depth: 0 } },
        { id: "b", label: "b", metadata: { depth: 1 } },
      ],
      edges: [{ from: "a", to: "b", kind: "Reads", direction: "Forward" }],
      groups: [],
      focus_node: "a",
      source_summary: { total_nodes: 2, total_edges: 1, node_kinds: {}, edge_kinds: {} },
      truncated: false,
    };
    const renderResult = await renderVisualGraph(container, envelope, { document });
    assert.strictEqual(renderResult.graph.nodes.length, 2);
    assert.ok(container.getAttribute("data-metadata-checker-graph-nodes"), "2");
    assert.ok(renderResult.mermaid.includes("graph TD"));
  });
});

describe("render fallback on error envelope", () => {
  it("renders diagnostics-only graph for error envelope", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({
      container,
      document,
    });

    await renderer.renderError({
      status: "error",
      target: "focus",
      diagnostics: [{ severity: "error", code: "FAILED", message: "broken", location: {} }],
      items: [],
    });

    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-nodes"), "0");
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-truncated"), "false");
    assert.ok(walkText(container, (text) => text.includes("FAILED: broken")));
  });
});

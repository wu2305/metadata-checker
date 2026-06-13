import { describe, it } from "node:test";
import assert from "node:assert";
import { createGraphPanelRenderer, renderVisualGraph } from "../renderer/graph-panel-renderer.mjs";

function createFakeDocument() {
  function createElement(tagName, { svg = false } = {}) {
    const element = {
      tagName,
      style: {},
      attributes: {},
      children: [],
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
    if (svg) {
      Object.defineProperty(element, "className", {
        get() {
          return { baseVal: this.attributes.class ?? "" };
        },
        set() {
          throw new TypeError("SVGElement.className is read only");
        },
      });
    } else {
      element.className = "";
    }
    return element;
  }

  return {
    createElement,
    createElementNS(_namespace, tagName) {
      return createElement(tagName, { svg: true });
    },
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
    const rawClassName = typeof node.className === "string"
      ? node.className
      : node.getAttribute?.("class") ?? "";
    if (rawClassName.split(" ").includes(className)) {
      result.push(node);
    }
    if (Array.isArray(node.children)) {
      node.children.forEach(walk);
    }
  };
  walk(root);
  return result;
}

function findByTagAndAttribute(root, tagName, name, value) {
  const matches = [];
  const walk = (node) => {
    if (node.tagName === tagName) {
      if (node.getAttribute && node.getAttribute(name) === value) {
        matches.push(node);
      }
    }
    if (Array.isArray(node.children)) {
      node.children.forEach(walk);
    }
  };
  walk(root);
  return matches;
}

function findFirstByTag(root, tagName) {
  const matches = [];
  const walk = (node) => {
    if (node.tagName === tagName) {
      matches.push(node);
    }
    if (Array.isArray(node.children)) {
      node.children.forEach(walk);
    }
  };
  walk(root);
  return matches[0] ?? null;
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

function findNodeGroupById(svg, nodeId) {
  return findByTagAndAttribute(svg, "g", "data-node-id", nodeId)?.[0] ?? null;
}

function findFirstCircleFromGroup(group) {
  return group?.children?.find((child) => child.tagName === "circle") ?? null;
}

function createFakePixiRuntime() {
  class Container {
    constructor() {
      this.children = [];
    }

    addChild(child) {
      this.children.push(child);
      child.parentNode = this;
      return child;
    }
  }

  class Graphics {
    constructor() {
      this.events = {};
      this.commands = [];
      this.position = { x: 0, y: 0 };
    }

    beginFill(color, alpha) {
      this.commands.push(["beginFill", color, alpha]);
      return this;
    }

    drawCircle(x, y, radius) {
      this.commands.push(["drawCircle", x, y, radius]);
      return this;
    }

    endFill() {
      this.commands.push(["endFill"]);
      return this;
    }

    lineStyle(width, color, alpha) {
      this.commands.push(["lineStyle", width, color, alpha]);
      return this;
    }

    moveTo(x, y) {
      this.commands.push(["moveTo", x, y]);
      return this;
    }

    lineTo(x, y) {
      this.commands.push(["lineTo", x, y]);
      return this;
    }

    on(type, handler) {
      this.events[type] = handler;
      return this;
    }
  }

  class Text {
    constructor(value, options = {}) {
      this.text = String(value ?? "");
      this.options = options;
      this.position = { x: 0, y: 0 };
      this.alpha = 1;
      this.anchor = { set() {} };
      this.style = options.style || {};
    }
  }

  class Application {
    constructor(options = {}) {
      this.options = options;
      this.stage = new Container();
      this.canvas = { tagName: "canvas", options };
    }

    destroy() {}
  }

  return { Application, Container, Graphics, Text };
}

function createFakeEchartsRuntime() {
  const instances = [];
  return {
    init(dom) {
      const instance = {
        dom,
        option: null,
        handlers: {},
        setOption(option) {
          this.option = option;
        },
        on(event, handler) {
          this.handlers[event] = handler;
        },
        off(event) {
          delete this.handlers[event];
        },
        dispose() {},
        resize() {},
      };
      instances.push(instance);
      return instance;
    },
    __instances: instances,
  };
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
    {
      from: "focus",
      to: "read1",
      kind: "Reads",
      direction: "Forward",
      label: "reads",
      priority: "filter",
      summary: "Reads: filter condition",
      evidence_status: "available",
      evidence: "${read1.title}",
    },
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
  it("falls back to Pixi in auto mode when bundled ECharts is unavailable", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({
      container,
      document,
      vendorRuntimeLoader: {
        async loadEcharts() {
          return null;
        },
        async loadAll() {
          return { echarts: null, pixi: createFakePixiRuntime(), d3Force3D: null };
        },
        getLastLoadError() {
          return "test echarts missing";
        },
      },
    });

    const result = await renderer.render({
      ...VIZ,
      status: "ready",
      focus_node: "focus",
      depth: 2,
      visible_hop: 1,
    });

    assert.strictEqual(result.renderer, "pixi");
    assert.ok(findAllByTag(container, "canvas").length >= 1);
  });

  it("prefers bundled ECharts in auto mode when extension vendor runtime is available", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({
      container,
      document,
      vendorRuntimeLoader: {
        async loadEcharts() {
          return createFakeEchartsRuntime();
        },
        async loadAll() {
          return { echarts: createFakeEchartsRuntime(), pixi: createFakePixiRuntime(), d3Force3D: null };
        },
      },
    });

    const result = await renderer.render({
      ...VIZ,
      status: "ready",
      focus_node: "focus",
      depth: 2,
      visible_hop: 1,
    });

    assert.strictEqual(result.renderer, "echarts");
    assert.strictEqual(container.getAttribute("data-metadata-checker-local-graph-renderer"), "echarts");
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-renderer"), "echarts");
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-depth"), "2");
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-visible-hop"), "1");
    assert.ok(findByClass(container, "metadata-checker-echarts-mount-host").length >= 1);
  });

  it("exposes visible graph text from echarts controller after render", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({
      container,
      document,
      vendorRuntimeLoader: {
        async loadEcharts() {
          return createFakeEchartsRuntime();
        },
      },
    });

    await renderer.render({
      ...VIZ,
      status: "ready",
      focus_node: "focus",
      depth: 2,
      visible_hop: 1,
    });

    const text = renderer.getVisibleGraphText();
    assert.ok(text.includes("focus: focus"));
    assert.ok(text.includes("nodes:"));
    assert.ok(text.includes("edges:"));
    assert.ok(text.includes("visible:"));
  });

  it("exposes visible graph text from pixi controller after render", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({
      container,
      document,
      renderer: "pixi",
      vendorRuntimeLoader: {
        async loadAll() {
          return { pixi: createFakePixiRuntime(), d3Force3D: null };
        },
      },
    });

    await renderer.render({
      ...VIZ,
      status: "ready",
      focus_node: "focus",
      depth: 2,
      visible_hop: 1,
    });

    const text = renderer.getVisibleGraphText();
    assert.ok(text.includes("focus: focus"));
    assert.ok(text.includes("nodes:"));
    assert.ok(text.includes("edges:"));
    assert.ok(text.includes("visible:"));
  });

  it("falls back to last render graph when building visible graph text without pixi", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({
      container,
      document,
      renderer: "html",
    });

    await renderer.render({
      ...VIZ,
      status: "ready",
      focus_node: "focus",
      depth: 2,
      visible_hop: 1,
    });

    const text = renderer.getVisibleGraphText();
    assert.ok(text.includes("focus: focus"));
    assert.ok(text.includes("nodes:"));
    assert.ok(text.includes("- focus node (Component)"));
    assert.ok(text.includes("edges:"));
    assert.ok(text.includes("focus -> read1"));
  });

  it("forwards lock and clear to pixi controller", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({
      container,
      document,
      renderer: "pixi",
      vendorRuntimeLoader: {
        async loadAll() {
          return { pixi: createFakePixiRuntime(), d3Force3D: null };
        },
      },
    });

    const result = await renderer.render({
      nodes: [
        { id: "focus", label: "focus", kind: "Component", metadata: { depth: 0 } },
        { id: "m1", label: "model", kind: "Model", metadata: { depth: 1 } },
      ],
      edges: [
        {
          id: "focus->m1",
          from: "focus",
          to: "m1",
          kind: "Reads",
          direction: "Forward",
          label: "reads",
          priority: "source",
          evidence_status: "available",
          evidence: "${m1.name}",
        },
      ],
      focus_node: "focus",
      source_summary: { total_nodes: 2, total_edges: 1, node_kinds: {}, edge_kinds: {} },
      diagnostics: [],
      truncated: false,
      status: "ready",
      depth: 2,
      visible_hop: 1,
    });

    assert.strictEqual(result.renderer, "pixi");
    const edgeId = result.graph?.viewGraph?.edges?.[0]?.id ?? result.graph?.edges?.[0]?.id;
    assert.ok(edgeId);
    renderer.lockCurrentTarget({ type: "edge", id: edgeId });
    assert.strictEqual(
      container.getAttribute("data-metadata-checker-graph-locked-target"),
      `edge:${edgeId}`,
    );
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-detail-kind"), "edge");

    renderer.clearLockedTarget();
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-locked-target"), "none");
  });

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
    assert.strictEqual(findAllByTag(container, "svg").length, 1);
    assert.ok(findAllByTag(container, "circle").length >= 2);
    assert.ok(findAllByTag(container, "line").length >= 1);
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

  it("renders depth-aware node radii in SVG visual", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({ container, document });

    await renderer.render({
      nodes: [
        { id: "focus", label: "focus", kind: "Component", metadata: { depth: 0 } },
        { id: "one", label: "one", kind: "Model", metadata: { depth: 1 } },
        { id: "two", label: "two", kind: "Model", metadata: { depth: 2 } },
      ],
      edges: [
        { from: "focus", to: "one", kind: "Reads", direction: "Forward", label: "reads" },
        { from: "one", to: "two", kind: "Writes", direction: "Forward", label: "writes" },
      ],
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

    const surface = findByTagAndAttribute(container, "div", "data-metadata-checker-graph-surface", "svg")[0];
    const svg = findFirstByTag(surface, "svg");
    const focusNode = findNodeGroupById(svg, "focus");
    const oneNode = findNodeGroupById(svg, "one");
    const twoNode = findNodeGroupById(svg, "two");
    const focusRadius = Number.parseFloat(findFirstCircleFromGroup(focusNode)?.getAttribute("r") ?? "0");
    const oneRadius = Number.parseFloat(findFirstCircleFromGroup(oneNode)?.getAttribute("r") ?? "0");
    const twoRadius = Number.parseFloat(findFirstCircleFromGroup(twoNode)?.getAttribute("r") ?? "0");

    assert.ok(focusRadius > oneRadius);
    assert.ok(oneRadius > twoRadius);
    assert.ok(twoRadius > 0);
  });

  it("encodes edge priority in SVG visual styles", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({ container, document });

    await renderer.render({
      nodes: [
        { id: "focus", label: "focus", kind: "Component", metadata: { depth: 0 } },
        { id: "n1", kind: "Model", metadata: { depth: 1 } },
        { id: "n2", kind: "Model", metadata: { depth: 1 } },
        { id: "n3", kind: "Model", metadata: { depth: 1 } },
      ],
      edges: [
        {
          from: "focus",
          to: "n1",
          kind: "Reads",
          direction: "Forward",
          label: "reads",
          priority: "filter",
          evidence_status: "available",
          evidence: "${n1.field}",
        },
        { from: "focus", to: "n2", kind: "Condition", direction: "Forward", label: "cond", priority: "condition" },
        { from: "focus", to: "n3", kind: "Visibility", direction: "Forward", label: "visibility", priority: "visibility" },
      ],
      focus_node: "focus",
      source_summary: {
        total_nodes: 4,
        total_edges: 3,
        node_kinds: {},
        edge_kinds: {},
      },
      diagnostics: [],
      truncated: false,
      status: "ready",
    });

    const surface = findByTagAndAttribute(container, "div", "data-metadata-checker-graph-surface", "svg")[0];
    const svg = findFirstByTag(surface, "svg");

    const getLine = (edgeId) => findByTagAndAttribute(svg, "line", "data-edge-id", edgeId)[0];

    const filterLine = getLine("focus->n1");
    const conditionLine = getLine("focus->n2");
    const visibilityLine = getLine("focus->n3");

    assert.ok(filterLine && conditionLine && visibilityLine);
    assert.strictEqual(filterLine.getAttribute("stroke"), "#fbbf24");
    assert.strictEqual(conditionLine.getAttribute("stroke"), "#c084fc");
    assert.strictEqual(visibilityLine.getAttribute("stroke"), "#a78bfa");

    const filterWidth = Number.parseFloat(filterLine.getAttribute("stroke-width") || "0");
    const conditionWidth = Number.parseFloat(conditionLine.getAttribute("stroke-width") || "0");
    const visibilityWidth = Number.parseFloat(visibilityLine.getAttribute("stroke-width") || "0");
    assert.ok(filterWidth > conditionWidth);
    assert.ok(conditionWidth > visibilityWidth);
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

  it("dispatches node click/hover callbacks from SVG nodes", async () => {
    const events = [];
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({
      container,
      document,
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

    const surface = findByTagAndAttribute(container, "div", "data-metadata-checker-graph-surface", "svg")[0];
    assert.ok(surface);
    const svg = findFirstByTag(surface, "svg");
    const node = findByTagAndAttribute(svg, "g", "data-node-id", "focus")[0];
    assert.ok(node);
    node.dispatchEvent({ type: "mouseover" });
    node.dispatchEvent({ type: "click" });

    const hoverEvent = events.find((event) => event.type === "graph-node-hover");
    const clickEvent = events.find((event) => event.type === "graph-node-click");

    assert.ok(hoverEvent);
    assert.ok(clickEvent);
    assert.strictEqual(clickEvent.event, "click");
    assert.strictEqual(hoverEvent.event, "hover");
    assert.strictEqual(typeof clickEvent.nodeId, "string");
    const detailSection = findByClass(container, "metadata-checker-graph-detail-content")[0];
    assert.ok(detailSection);
    assert.ok(walkText(detailSection, (text) => text.includes("Node ·")));
    assert.ok(walkText(detailSection, (text) => text.includes("focus")));
  });

  it("dispatches edge click/hover callbacks from SVG edges and keeps edge detail", async () => {
    const events = [];
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({
      container,
      document,
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

    const surface = findByTagAndAttribute(container, "div", "data-metadata-checker-graph-surface", "svg")[0];
    assert.ok(surface);
    const svg = findFirstByTag(surface, "svg");
    const line = findFirstByTag(svg, "line");
    assert.ok(line);
    assert.strictEqual(line.getAttribute("stroke"), "#fbbf24");
    assert.strictEqual(line.getAttribute("stroke-width"), "2.2");

    line.dispatchEvent({ type: "mouseover" });
    line.dispatchEvent({ type: "click" });

    const hoverEvent = events.find((event) => event.type === "graph-edge-hover");
    const clickEvent = events.find((event) => event.type === "graph-edge-click");
    assert.ok(hoverEvent);
    assert.ok(clickEvent);
    assert.strictEqual(clickEvent.type, "graph-edge-click");
    assert.strictEqual(hoverEvent.type, "graph-edge-hover");
    assert.strictEqual(hoverEvent.event, "hover");
    assert.strictEqual(clickEvent.event, "click");
    assert.strictEqual(clickEvent.priority, "filter");
    assert.strictEqual(clickEvent.evidenceStatus, "available");
    const detailSection = findByClass(container, "metadata-checker-graph-detail-content")[0];
    assert.ok(detailSection);
    assert.ok(walkText(detailSection, (text) => text.includes("Edge ·")));
    assert.ok(walkText(detailSection, (text) => text.includes("filter")));
    assert.ok(walkText(detailSection, (text) => text.includes("title")));
  });

  it("highlights hovered SVG node neighbor links and keeps detail text compact", async () => {
    const events = [];
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({
      container,
      document,
      onEvent(payload) {
        events.push(payload);
      },
    });

    await renderer.render({
      nodes: [
        {
          id: "focus",
          label: "Focus",
          kind: "Component",
          metadata: { depth: 0 },
        },
        {
          id: "neighbor_one",
          label: "One hop",
          kind: "Model",
          metadata: { depth: 1 },
        },
        {
          id: "neighbor_two",
          label: "A very very long two-hop neighbor with deep text that should still keep compact detail area",
          kind: "Model",
          metadata: { depth: 2 },
        },
      ],
      edges: [
        {
          id: "focus->neighbor_one",
          from: "focus",
          to: "neighbor_one",
          kind: "Reads",
          direction: "Forward",
          label: "reads status",
          summary: "filter condition summary",
          priority: "filter",
          evidence_status: "available",
          evidence: "ORDER.status",
          diagnostics: [],
        },
        {
          id: "neighbor_one->neighbor_two",
          from: "neighbor_one",
          to: "neighbor_two",
          kind: "Writes",
          direction: "Forward",
          label: "writes status",
          summary: "background writes",
        },
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

    const surface = findByTagAndAttribute(container, "div", "data-metadata-checker-graph-surface", "svg")[0];
    assert.ok(surface);
    const svg = findFirstByTag(surface, "svg");
    const focusNode = findByTagAndAttribute(svg, "g", "data-node-id", "focus")[0];
    const neighborEdge = findByTagAndAttribute(svg, "line", "data-edge-id", "neighbor_one->neighbor_two")[0];
    const focusEdge = findByTagAndAttribute(svg, "line", "data-edge-id", "focus->neighbor_one")[0];
    assert.ok(focusEdge);
    assert.ok(neighborEdge);

    assert.equal(focusEdge.getAttribute("stroke"), "#fbbf24");
    const twoHopOpacityBefore = Number.parseFloat(neighborEdge.getAttribute("stroke-opacity"));
    assert.ok(twoHopOpacityBefore < Number.parseFloat(focusEdge.getAttribute("stroke-opacity")));

    neighborEdge.dispatchEvent({ type: "mouseover" });
    assert.ok(events.some((event) => event.type === "graph-edge-hover"));

    const neighborEdgeOpacityHover = Number.parseFloat(neighborEdge.getAttribute("stroke-opacity"));
    const focusEdgeOpacityHover = Number.parseFloat(focusEdge.getAttribute("stroke-opacity"));
    assert.equal(focusEdgeOpacityHover >= neighborEdgeOpacityHover, true);

    focusNode.dispatchEvent({ type: "mouseover" });
    assert.ok(events.some((event) => event.type === "graph-node-hover"));
    const focusEvent = events.find((event) => event.type === "graph-node-hover");
    assert.strictEqual(focusEvent?.event, "hover");

    focusNode.dispatchEvent({ type: "click" });
    const detailSection = findByClass(container, "metadata-checker-graph-detail-content")[0];
    assert.ok(detailSection);
    assert.ok(walkText(detailSection, (text) => text.includes("Node ·")));
    const detailRows = detailSection.children;
    assert.ok(detailRows.length >= 1);
    assert.equal(detailRows.every((row) => row.style.maxWidth === "100%"), true);
    assert.equal(detailRows.every((row) => row.style.overflowWrap === "anywhere"), true);
    assert.ok(detailRows.every((row) => row.style.wordBreak === "break-word"));
    assert.ok(walkText(container, (text) => /two-hop/i.test(text)));
  });

  it("keeps marker attributes stable under hover interaction", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({ container, document });
    const result = await renderer.render({
      nodes: [
        { id: "a", kind: "Component", metadata: { depth: 0 } },
        { id: "b", kind: "Model", metadata: { depth: 1 } },
      ],
      edges: [{ from: "a", to: "b", kind: "Reads", direction: "Forward", label: "read" }],
      groups: [],
      focus_node: "a",
      source_summary: {
        total_nodes: 2,
        total_edges: 1,
        node_kinds: {},
        edge_kinds: {},
      },
      diagnostics: [],
      truncated: false,
      status: "ready",
    });

    const markerSnapshot = {
      nodes: result.graph.nodes.length,
      edges: result.graph.edges.length,
      focus: container.getAttribute("data-metadata-checker-graph-focus"),
      truncated: container.getAttribute("data-metadata-checker-graph-truncated"),
      renderer: container.getAttribute("data-metadata-checker-graph-renderer") ?? container.getAttribute("data-metadata-checker-renderer"),
    };

    const surface = findByTagAndAttribute(container, "div", "data-metadata-checker-graph-surface", "svg")[0];
    const svg = findFirstByTag(surface, "svg");
    const node = findByTagAndAttribute(svg, "g", "data-node-id", "a")[0];
    node.dispatchEvent({ type: "mouseover" });
    node.dispatchEvent({ type: "mouseout" });
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-focus"), markerSnapshot.focus);
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-truncated"), markerSnapshot.truncated);
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-renderer") ?? container.getAttribute("data-metadata-checker-renderer"), markerSnapshot.renderer);
    assert.equal(Number.parseInt(container.getAttribute("data-metadata-checker-graph-nodes"), 10), markerSnapshot.nodes);
    assert.equal(Number.parseInt(container.getAttribute("data-metadata-checker-graph-edges"), 10), markerSnapshot.edges);
  });

  it("updates open-detail marker and context from locked targets with sanitization", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({ container, document });

    await renderer.render({
      nodes: [
        { id: "focus", label: "focus", kind: "Component", metadata: { depth: 0 } },
        { id: "secret-model", label: "secret-model", kind: "Model", metadata: { depth: 1 } },
      ],
      edges: [
        {
          id: "edge=token=xyz",
          from: "focus",
          to: "secret-model",
          kind: "Reads",
          direction: "Forward",
          label: "reads",
        },
      ],
      focus_node: "focus",
      source_summary: {
        total_nodes: 2,
        total_edges: 1,
        node_kinds: {},
        edge_kinds: {},
      },
      diagnostics: [],
      truncated: false,
      status: "ready",
    });

    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-detail-kind"), "focus");
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-open-detail-context"), "focus:focus");

    const controller = renderer.getDomController();
    assert.ok(controller);
    controller.lockCurrentTarget({ type: "edge", id: "edge=token=xyz" });
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-open-detail"), "edge");
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-detail-kind"), "edge");
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-open-detail-context"), "edge:[sensitive]");
    assert.strictEqual(renderer.getOpenDetailContext()?.kind, "edge");

    controller.clearLockedTarget();
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-detail-kind"), "focus");
    assert.strictEqual(renderer.getOpenDetailContext()?.kind, "focus");
    assert.ok(!/token=/.test(container.getAttribute("data-metadata-checker-graph-open-detail-context") ?? ""));
  });

  it("exposes viewport scale controls through dom controller", async () => {
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createGraphPanelRenderer({ container, document });
    await renderer.render({
      nodes: [
        { id: "a", kind: "Component", metadata: { depth: 0 } },
        { id: "b", kind: "Model", metadata: { depth: 1 } },
      ],
      edges: [{ from: "a", to: "b", kind: "Reads", direction: "Forward", label: "read" }],
      focus_node: "a",
      source_summary: {
        total_nodes: 2,
        total_edges: 1,
        node_kinds: {},
        edge_kinds: {},
      },
      diagnostics: [],
      truncated: false,
      status: "ready",
    });

    const controller = renderer.getDomController();
    assert.ok(controller);
    const scaled = controller.setViewportScale(1.6);
    assert.strictEqual(scaled, 1.6);
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-viewport-scale"), "1.6");
    assert.strictEqual(renderer.resetViewportScale(), 1);
    assert.strictEqual(container.getAttribute("data-metadata-checker-graph-viewport-scale"), "1");
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
    const renderer = createGraphPanelRenderer({
      container,
      document,
      maxDepth: 200,
      maxNodes: 120,
      maxEdges: 360,
    });
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
    const truncatedReason = container.getAttribute("data-metadata-checker-graph-truncated-reason") || "";
    assert.ok(truncatedReason.includes("max_edges"));
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
        { id: "secret=abc", label: "credential=abc", metadata: { depth: 2 }, kind: "Model", source_path: "z" },
        { id: "api_key=abc", label: "auth=abc", metadata: { depth: 2 }, kind: "Model", source_path: "k" },
      ],
      edges: [{ from: "a", to: "token=abc123", kind: "Reads", direction: "Forward", label: "read" }],
      groups: [],
      focus_node: "a",
      source_summary: {
        total_nodes: 5,
        total_edges: 1,
        node_kinds: {},
        edge_kinds: {},
      },
      diagnostics: [{ severity: "warning", code: "SAFE", message: "password=none", location: {} }],
      truncated: false,
    });

    const sensitivePattern = /token|cookie|password|secret|auth|credential|api_key|apikey|cipherpassport/i;
    const hasSensitive = walkText(container, (text) => sensitivePattern.test(text));
    assert.strictEqual(hasSensitive, false);
    const nodeRows = findByClass(container, "graph-node");
    assert.strictEqual(
      nodeRows.some((row) => sensitivePattern.test(row.getAttribute("data-node-id") ?? "")),
      false,
    );
    assert.strictEqual(
      sensitivePattern.test(container.getAttribute("data-metadata-checker-graph-focus") ?? ""),
      false,
    );
    assert.strictEqual(/token=abc123/i.test(result.mermaid), false);
    assert.strictEqual(/secret=abc|credential=abc|api_key=abc|auth=abc/i.test(result.mermaid), false);
    assert.strictEqual(
      JSON.stringify(result.echartsOption).includes("token=abc123"),
      false,
    );
    assert.strictEqual(
      /secret=abc|credential=abc|api_key=abc|auth=abc/i.test(JSON.stringify(result.echartsOption)),
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

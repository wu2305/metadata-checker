import { readFile } from "node:fs/promises";
import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  createPixiLocalGraphRenderer,
  edgeTouchesAggregate,
  formatVisibleGraphText,
  getCurrentDetailPayload,
  nodeSelectionTier,
} from "../renderer/pixi-local-graph-renderer.mjs";

function createFakeDocument() {
  function createElement(tagName) {
    const listeners = {};
    const element = {
      tagName: String(tagName),
      style: {},
      children: [],
      attributes: {},
      events: {},
      setAttribute(name, value) {
        this.attributes[String(name)] = String(value);
      },
      getAttribute(name) {
        return this.attributes[name] ?? null;
      },
      appendChild(child) {
        this.children.push(child);
        child.parentNode = this;
      },
      removeChild(child) {
        const index = this.children.indexOf(child);
        if (index >= 0) {
          this.children.splice(index, 1);
        }
      },
      replaceChildren() {
        this.children = [];
      },
      addEventListener(type, handler) {
        if (!listeners[type]) listeners[type] = [];
        listeners[type].push(handler);
      },
      removeEventListener(type, handler) {
        const handlers = listeners[type] || [];
        listeners[type] = handlers.filter((candidate) => candidate !== handler);
      },
      dispatchEvent(event) {
        const handlers = listeners[event.type] || [];
        handlers.forEach((handler) => handler(event));
      },
    };
    return element;
  }

  return {
    body: createElement("body"),
    createElement,
  };
}

function createFakePixiRuntime() {
  const applications = [];

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
      this.alpha = 1;
    }

    beginFill(color, alpha) {
      this.commands.push(["beginFill", color, alpha]);
    }

    drawCircle(_x, _y, radius) {
      this.commands.push(["drawCircle", _x, _y, radius]);
      this.radius = radius;
    }

    endFill() {
      this.commands.push(["endFill"]);
    }

    lineStyle(width, color, alpha) {
      this.commands.push(["lineStyle", width, color, alpha]);
      this.lineWidth = width;
      this.currentColor = color;
      this.currentAlpha = alpha;
    }

    moveTo(x, y) {
      this.commands.push(["moveTo", x, y]);
      this.cursor = { x, y };
    }

    lineTo(x, y) {
      this.commands.push(["lineTo", x, y]);
    }

    stroke(style) {
      this.commands.push(["stroke", style]);
    }

    circle(x, y, radius) {
      this.commands.push(["circle", x, y, radius]);
      this.radius = radius;
      return this;
    }

    fill(style) {
      this.commands.push(["fill", style]);
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
      this.scale = { x: 1, y: 1 };
      this.anchor = {
        set() {},
      };
      this.style = options.style || {};
      this.eventMode = "static";
    }
  }

  class Application {
    constructor(options = {}) {
      this.options = options;
      this.stage = new Container();
      this.canvas = createCanvas();
      this.applications = applications;
      applications.push(this);
    }

    destroy() {
      this.destroyed = true;
    }
  }

  function createCanvas() {
    const listeners = {};
    return {
      tagName: "canvas",
      style: {},
      attributes: {},
      addEventListener(type, handler) {
        if (!listeners[type]) listeners[type] = [];
        listeners[type].push(handler);
      },
      removeEventListener(type, handler) {
        const current = listeners[type] || [];
        listeners[type] = current.filter((candidate) => candidate !== handler);
      },
      dispatchEvent(event) {
        const handlers = listeners[event.type] || [];
        handlers.forEach((handler) => handler(event));
      },
      setAttribute(name, value) {
        this.attributes[String(name)] = String(value);
      },
      getAttribute(name) {
        return this.attributes[name] ?? null;
      },
      parentNode: null,
    };
  }

  return {
    Application,
    Container,
    Graphics,
    Text,
    applications,
  };
}

function makeCallbackLog() {
  const nodeClick = [];
  const nodeHover = [];
  const nodeFocus = [];
  const edgeClick = [];
  const edgeHover = [];

  return {
    nodeClick,
    nodeHover,
    nodeFocus,
    edgeClick,
    edgeHover,
    onNodeClick(payload) {
      nodeClick.push(payload);
    },
    onNodeHover(payload) {
      nodeHover.push(payload);
    },
    onNodeFocus(payload) {
      nodeFocus.push(payload);
    },
    onEdgeClick(payload) {
      edgeClick.push(payload);
    },
    onEdgeHover(payload) {
      edgeHover.push(payload);
    },
  };
}

function makeStateSnapshot(renderer) {
  const state = renderer.getState();
  return {
    root: state.root,
    state,
    detailKind: state.root?.getAttribute?.("data-metadata-checker-graph-detail-kind") ?? "",
    hoverTarget: state.root?.getAttribute?.("data-metadata-checker-graph-hover-target") ?? "",
    lockedTarget: state.root?.getAttribute?.("data-metadata-checker-graph-locked-target") ?? "",
    viewportScale: Number.parseFloat(state.root?.getAttribute?.("data-metadata-checker-graph-viewport-scale") || "1"),
    viewportTarget: state.root?.getAttribute?.("data-metadata-checker-graph-viewport-target") ?? "",
    detailText: getDetailHostText(state.root),
  };
}

async function loadFixture() {
  const fixturePath = new URL("./fixtures/m47-local-graph-2hop.json", import.meta.url);
  const fixtureText = await readFile(fixturePath, "utf8");
  return JSON.parse(fixtureText);
}

function makeDenseFixture(nodeCount = 40, edgeCount = 120) {
  const nodes = Array.from({ length: nodeCount }, (_, index) => ({
    id: index === 0 ? "focus" : `n${index}`,
    label: index === 0 ? "Focus Node" : `Dense ${index}`,
    kind: index % 3 === 0 ? "model" : "param",
    metadata: {
      depth: index === 0 ? 0 : index < 10 ? 1 : 2,
    },
  }));

  const edges = Array.from({ length: edgeCount }, (_, index) => {
    const fromIndex = index % nodeCount;
    const toIndex = (fromIndex + 1 + (index % 7)) % nodeCount;
    return {
      from: nodes[fromIndex].id,
      to: nodes[toIndex].id,
      kind: index % 5 === 0 ? "writes" : index % 4 === 0 ? "condition" : "reads",
      direction: "Forward",
      label: `e${index}`,
      fromDepth: fromIndex < 10 ? 1 : 2,
      toDepth: toIndex < 10 ? 1 : 2,
      priority: index % 3 === 0 ? "filter" : index % 4 === 0 ? "condition" : "other",
    };
  });

  return {
    focus_node: "focus",
    nodes,
    edges,
  };
}

function dispatchWheel(canvas, event = {}) {
  canvas.dispatchEvent({
    type: "wheel",
    deltaY: event.deltaY ?? -120,
    offsetX: event.offsetX ?? 160,
    offsetY: event.offsetY ?? 110,
    preventDefault() {},
  });
}

function dispatchPan(canvas, state, start = { x: 0, y: 0 }, end = { x: 0, y: 0 }) {
  canvas.dispatchEvent({
    type: "pointerdown",
    offsetX: start.x,
    offsetY: start.y,
  });
  canvas.dispatchEvent({
    type: "pointermove",
    offsetX: end.x,
    offsetY: end.y,
  });
  canvas.dispatchEvent({
    type: "pointerup",
    offsetX: end.x,
    offsetY: end.y,
  });
  return state.getState();
}

function dispatchDoubleClick(canvas, { x = 0, y = 0 } = {}) {
  canvas.dispatchEvent({
    type: "dblclick",
    offsetX: x,
    offsetY: y,
  });
}

function dispatchBackgroundTap(canvas, point = { x: 120, y: 110 }) {
  canvas.dispatchEvent({
    type: "pointerdown",
    offsetX: point.x,
    offsetY: point.y,
  });
  canvas.dispatchEvent({
    type: "pointerup",
    offsetX: point.x,
    offsetY: point.y,
  });
}

function getCanvas(root) {
  return root.children.find((node) => node.tagName === "canvas");
}

function getDetailHost(root) {
  return root.children.find((node) => node.getAttribute?.("data-metadata-checker-pixi-detail") === "mounted");
}

function getDetailHostText(root) {
  const detailHost = getDetailHost(root);
  const firstRow = detailHost?.children?.[0];
  return firstRow?.textContent ?? "";
}

describe("pixi local graph helpers (pure unit)", () => {
  const focusId = "comp:focus";

  it("ranks selection tiers as filter > visibility > source > downstream component", () => {
    const edges = [
      { from: focusId, to: "mdl_filter", priority: "filter" },
      { from: focusId, to: "mdl_vis", priority: "visibility" },
      { from: "mdl_source", to: focusId, priority: "source" },
      { from: focusId, to: "comp:down", priority: "action" },
      { from: focusId, to: "mdl_plain", priority: "other" },
    ];

    assert.equal(nodeSelectionTier({ id: focusId }, edges, focusId), 99);
    assert.equal(nodeSelectionTier({ id: "mdl_filter", kind: "model" }, edges, focusId), 4);
    assert.equal(nodeSelectionTier({ id: "mdl_vis", kind: "model" }, edges, focusId), 3);
    assert.equal(nodeSelectionTier({ id: "mdl_source", kind: "model" }, edges, focusId), 2);
    assert.equal(nodeSelectionTier({ id: "comp:down", kind: "component" }, edges, focusId), 1);
    assert.equal(nodeSelectionTier({ id: "mdl_plain", kind: "model" }, edges, focusId), 0);
    assert.equal(
      nodeSelectionTier({ id: "agg", aggregate: true, hiddenNodeCount: 3 }, edges, focusId),
      0,
    );
  });

  it("treats condition priority as filter tier", () => {
    const edges = [{ from: focusId, to: "mdl_cond", priority: "condition" }];
    assert.equal(nodeSelectionTier({ id: "mdl_cond", kind: "model" }, edges, focusId), 4);
  });

  it("uses max tier when node connects to multiple edge priorities", () => {
    const edges = [
      { from: focusId, to: "mdl_mix", priority: "source" },
      { from: "mdl_mix", to: "comp:child", priority: "action" },
    ];
    assert.equal(nodeSelectionTier({ id: "mdl_mix", kind: "model" }, edges, focusId), 2);
  });

  it("detects aggregate-touching edges for dashed rendering", () => {
    const viewGraph = {
      nodeById: new Map([
        ["n1", { id: "n1" }],
        ["agg", { id: "agg", aggregate: true }],
      ]),
    };
    assert.equal(edgeTouchesAggregate({ from: "n1", to: "agg" }, viewGraph), true);
    assert.equal(edgeTouchesAggregate({ from: "n1", to: "n2" }, viewGraph), false);
    assert.equal(edgeTouchesAggregate(null, viewGraph), false);
  });

  it("formats visible graph text with aggregate nodes and locked target", () => {
    const text = formatVisibleGraphText({
      designerFocusNodeId: focusId,
      lastRenderGraph: {
        nodes: new Array(10).fill(null).map((_, index) => ({ id: `n${index}` })),
        edges: new Array(20).fill(null).map((_, index) => ({ id: `e${index}` })),
      },
      lastViewGraph: {
        visibleNodeCount: 2,
        aggregateNodeCount: 1,
        visibleEdgeCount: 1,
        nodes: [
          { id: focusId, label: "Focus", kind: "component", depth: 0 },
          { id: "agg_bucket", aggregate: true, hiddenNodeCount: 5, aggregateLabel: "hidden-bucket" },
        ],
        edges: [
          {
            from: focusId,
            to: "mdl1",
            summary: "reads field",
            priority: "source",
            evidence_status: "unavailable",
          },
        ],
      },
      lockedTarget: { type: "edge", id: "e_filter" },
    });

    assert.ok(text.includes(`focus: ${focusId}`));
    assert.ok(text.includes("visible: 3/10 nodes, 1/20 edges"));
    assert.ok(text.includes("nodes:"));
    assert.ok(text.includes("- Focus (component) [0]"));
    assert.ok(text.includes("- +5 (hidden-bucket)"));
    assert.ok(text.includes("edges:"));
    assert.ok(text.includes(`- ${focusId} -> mdl1 : reads field [source/unavailable]`));
    assert.ok(text.includes("locked: edge:e_filter"));
  });

  it("formats empty visible graph text without locked line", () => {
    const text = formatVisibleGraphText({
      designerFocusNodeId: focusId,
      lastRenderGraph: { nodes: [], edges: [] },
      lastViewGraph: { visibleNodeCount: 0, aggregateNodeCount: 0, nodes: [], edges: [] },
      lockedTarget: null,
    });

    assert.equal(text, [
      `focus: ${focusId}`,
      "visible: 0/0 nodes, 0/0 edges",
      "nodes:",
      "edges:",
    ].join("\n"));
  });

  it("prefers locked target over hover for detail payload", () => {
    const edge = {
      id: "e_locked",
      from: focusId,
      to: "mdl1",
      kind: "reads",
      label: "reads",
      priority: "source",
    };
    const hoverNode = { id: "mdl_hover", label: "hover", kind: "model" };
    const state = {
      designerFocusNodeId: focusId,
      lockedTarget: { type: "edge", id: "e_locked" },
      hoveredTarget: { type: "node", id: "mdl_hover" },
      edgeById: new Map([["e_locked", edge]]),
      nodeById: new Map([["mdl_hover", hoverNode]]),
      nodeNeighborsById: new Map(),
      focusedGraphTarget: { type: "edge", id: "e_locked" },
    };

    const payload = getCurrentDetailPayload(state);
    assert.equal(payload?.type, "edge");
    assert.equal(payload?.edgeId, "e_locked");
    assert.equal(payload?.from, focusId);
  });

  it("falls back to hover then focus for detail payload", () => {
    const hoverNode = { id: "mdl_hover", label: "hover", kind: "model" };
    const focusNode = { id: focusId, label: "Focus", kind: "component" };
    const hoverOnly = {
      designerFocusNodeId: focusId,
      lockedTarget: null,
      hoveredTarget: { type: "node", id: "mdl_hover" },
      nodeById: new Map([
        ["mdl_hover", hoverNode],
        [focusId, focusNode],
      ]),
      nodeNeighborsById: new Map(),
      focusedGraphTarget: { type: "focus", id: focusId },
    };
    assert.equal(getCurrentDetailPayload(hoverOnly)?.nodeId, "mdl_hover");

    const focusOnly = {
      designerFocusNodeId: focusId,
      lockedTarget: null,
      hoveredTarget: null,
      nodeById: new Map([[focusId, focusNode]]),
      nodeNeighborsById: new Map(),
      focusedGraphTarget: { type: "focus", id: focusId },
    };
    assert.equal(getCurrentDetailPayload(focusOnly)?.nodeId, focusId);
  });
});

describe("pixi local graph renderer (m48 interaction)", () => {
  it("separates hover and lock targets, and clears lock without touching designer focus", async () => {
    const fixture = await loadFixture();
    const callbacks = makeCallbackLog();
    const document = createFakeDocument();
    const container = document.createElement("div");
    const runtime = createFakePixiRuntime();
    const renderer = createPixiLocalGraphRenderer({
      root: container,
      document,
      pixi: runtime,
      width: 320,
      height: 220,
      onNodeClick: callbacks.onNodeClick,
      onNodeHover: callbacks.onNodeHover,
      onNodeFocus: callbacks.onNodeFocus,
      onEdgeClick: callbacks.onEdgeClick,
      onEdgeHover: callbacks.onEdgeHover,
    });

    await renderer.render(fixture);

    const hoverNodeId = "mdl_order_status";
    const clickTargetId = "mdl_form_data";
    const hoverOtherId = "evt_submit";

    renderer.triggerNodeEvent(hoverNodeId, "hover");
    let snapshot = makeStateSnapshot(renderer);
    assert.equal(snapshot.state.hoveredTarget?.type, "node");
    assert.equal(snapshot.state.hoveredTarget?.id, hoverNodeId);
    assert.equal(snapshot.state.lockedTarget, null);
    assert.equal(snapshot.hoverTarget, `node:${hoverNodeId}`);
    assert.equal(snapshot.lockedTarget, "none");

    renderer.triggerNodeEvent(hoverNodeId, "hoverEnd");
    snapshot = makeStateSnapshot(renderer);
    assert.equal(snapshot.state.hoveredTarget, null);
    assert.equal(snapshot.hoverTarget, "none");

    renderer.triggerNodeEvent(clickTargetId, "click");
    snapshot = makeStateSnapshot(renderer);
    assert.equal(snapshot.state.lockedTarget?.type, "node");
    assert.equal(snapshot.state.lockedTarget?.id, clickTargetId);
    assert.equal(snapshot.state.focusedGraphTarget?.id, clickTargetId);
    assert.equal(snapshot.lockedTarget, `node:${clickTargetId}`);
    assert.equal(snapshot.state.designerFocusNodeId, fixture.focus_node);
    assert.equal(callbacks.nodeClick.length, 1);

    renderer.triggerNodeEvent(hoverOtherId, "hover");
    snapshot = makeStateSnapshot(renderer);
    assert.equal(snapshot.state.hoveredTarget?.id, hoverOtherId);
    assert.equal(snapshot.hoverTarget, `node:${hoverOtherId}`);
    assert.equal(snapshot.lockedTarget, `node:${clickTargetId}`);

    renderer.clearLockedTarget("test");
    snapshot = makeStateSnapshot(renderer);
    assert.equal(snapshot.state.lockedTarget, null);
    assert.equal(snapshot.lockedTarget, "none");
    assert.equal(snapshot.hoverTarget, `node:${hoverOtherId}`);
    assert.equal(snapshot.state.focusedGraphTarget?.type, "focus");
    assert.equal(snapshot.state.focusedGraphTarget?.id, fixture.focus_node);

    renderer.triggerNodeEvent(hoverOtherId, "hoverEnd");
    snapshot = makeStateSnapshot(renderer);
    assert.equal(snapshot.state.hoveredTarget, null);
    assert.equal(snapshot.hoverTarget, "none");
    assert.equal(snapshot.state.focusedGraphTarget?.id, fixture.focus_node);
    assert.equal(snapshot.detailKind, "node");

    assert.equal(callbacks.nodeFocus.length, 1);
  });

  it("keeps designer focus node stable when focused graph target switches", async () => {
    const fixture = await loadFixture();
    const callbacks = makeCallbackLog();
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createPixiLocalGraphRenderer({
      root: container,
      document,
      forceCanvas: false,
      onNodeFocus: callbacks.onNodeFocus,
      onEdgeClick: callbacks.onEdgeClick,
      onEdgeHover: callbacks.onEdgeHover,
    });

    const result = await renderer.render(fixture);
    const state = renderer.getState();
    const edgeId = result.graph.viewGraph.edges[0]?.id;
    const edge = state.edgeById.get(edgeId);
    assert.ok(edge);

    renderer.triggerEdgeEvent(edgeId, "click");
    const snapshot = makeStateSnapshot(renderer);
    assert.equal(snapshot.state.designerFocusNodeId, fixture.focus_node);
    assert.equal(snapshot.state.focusedGraphTarget?.type, "edge");
    assert.equal(snapshot.state.focusedGraphTarget?.id, edgeId);
    assert.equal(snapshot.state.lockedTarget?.type, "edge");
    assert.equal(snapshot.state.lockedTarget?.id, edgeId);
    assert.equal(snapshot.lockedTarget, `edge:${edgeId}`);
    assert.equal(snapshot.hoverTarget, "none");
    assert.equal(snapshot.state.root?.getAttribute("data-metadata-checker-graph-detail-kind"), "edge");

    assert.equal(callbacks.edgeClick.length, 1);
    assert.equal(callbacks.onNodeFocus.length, 1);
  });

  it("supports wheel zoom with clamp and keeps render graph stable", async () => {
    const fixture = makeDenseFixture();
    const document = createFakeDocument();
    const container = document.createElement("div");
    const callbacks = makeCallbackLog();
    const runtime = createFakePixiRuntime();
    const renderer = createPixiLocalGraphRenderer({
      root: container,
      document,
      pixi: runtime,
      width: 320,
      height: 220,
      densityProfile: "balanced",
      maxRenderedNodes: 24,
      maxRenderedEdges: 80,
      onNodeClick: callbacks.onNodeClick,
      onEdgeClick: callbacks.onEdgeClick,
    });

    const result = await renderer.render(fixture);
    const baseNodeCount = result.graph.nodes.length;
    const baseEdgeCount = result.graph.edges.length;
    const canvas = getCanvas(container);
    assert.ok(canvas);

    const initialScale = Number.parseFloat(
      container.getAttribute("data-metadata-checker-graph-viewport-scale") || "1",
    );
    assert.equal(initialScale, 1);

    dispatchWheel(canvas, { deltaY: -4000, offsetX: 120, offsetY: 110 });
    let snapshot = makeStateSnapshot(renderer);
    assert.equal(snapshot.viewportScale > 1, true);
    assert.equal(snapshot.viewportScale <= 2.4, true);

    dispatchWheel(canvas, { deltaY: 5000, offsetX: 120, offsetY: 110 });
    snapshot = makeStateSnapshot(renderer);
    assert.equal(snapshot.viewportScale >= 0.75, true);
    assert.equal(snapshot.viewportScale <= 2.4, true);

    const state = renderer.getState();
    assert.equal(state.lastRender?.graph?.nodes?.length, baseNodeCount);
    assert.equal(state.lastRender?.graph?.edges?.length, baseEdgeCount);
    assert.equal(callbacks.nodeClick.length, 0);
    assert.equal(callbacks.edgeClick.length, 0);
  });

  it("supports drag pan and resets viewport through 0 and Esc", async () => {
    const fixture = makeDenseFixture(16, 40);
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createPixiLocalGraphRenderer({
      root: container,
      document,
      pixi: createFakePixiRuntime(),
      width: 320,
      height: 220,
    });

    await renderer.render(fixture);
    const canvas = getCanvas(container);
    assert.ok(canvas);

    dispatchPan(canvas, renderer, { x: 80, y: 60 }, { x: 140, y: 130 });
    let state = renderer.getState();
    assert.notEqual(state.viewport.translateX, 0);
    assert.notEqual(state.viewport.translateY, 0);

    dispatchWheel(canvas, { deltaY: -400, offsetX: 140, offsetY: 110 });
    state = renderer.getState();
    const zoomedScale = state.viewport.scale;
    assert.equal(zoomedScale > 1, true);

    container.dispatchEvent({ type: "keydown", key: "0" });
    state = renderer.getState();
    assert.equal(state.viewport.scale, 1);
    assert.equal(state.viewport.translateX, 0);
    assert.equal(state.viewport.translateY, 0);

    dispatchPan(canvas, renderer, { x: 110, y: 110 }, { x: 200, y: 130 });
    state = renderer.getState();
    assert.notEqual(state.viewport.translateX, 0);

    container.dispatchEvent({ type: "keydown", key: "Escape" });
    state = renderer.getState();
    assert.equal(state.viewport.scale, 1);
    assert.equal(state.viewport.translateX, 0);
    assert.equal(state.viewport.translateY, 0);
    assert.equal(state.viewport.target?.type, "focus");
    assert.equal(state.viewport.target?.id, state.designerFocusNodeId);

    dispatchPan(canvas, renderer, { x: 130, y: 130 }, { x: 190, y: 180 });
    state = renderer.getState();
    assert.notEqual(state.viewport.translateX, 0);

    dispatchDoubleClick(canvas);
    state = renderer.getState();
    assert.equal(state.viewport.scale, 1);
    assert.equal(state.viewport.translateX, 0);
    assert.equal(state.viewport.translateY, 0);
    assert.equal(state.viewport.target?.type, "focus");
    assert.equal(state.viewport.target?.id, state.designerFocusNodeId);
  });

  it("frames viewport target to clicked edge and keeps endpoint highlighting", async () => {
    const fixture = await loadFixture();
    const document = createFakeDocument();
    const container = document.createElement("div");
    const runtime = createFakePixiRuntime();
    const renderer = createPixiLocalGraphRenderer({
      root: container,
      document,
      pixi: runtime,
      width: 320,
      height: 220,
    });

    const result = await renderer.render(fixture);
    const edge = result.graph.viewGraph.edges.find((item) => !item.aggregate);
    assert.ok(edge);
    const stateBefore = renderer.getState();
    const edgeViewBefore = stateBefore.edgeViews.get(edge.id);
    const beforeEdgeWidth = edgeViewBefore?.sprite?.width ?? 0;

    renderer.triggerEdgeEvent(edge.id, "click");
    const stateAfter = renderer.getState();
    const edgeViewAfter = stateAfter.edgeViews.get(edge.id);
    const afterFromView = stateAfter.nodeViews.get(edge.from);
    const afterToView = stateAfter.nodeViews.get(edge.to);

    assert.equal(stateAfter.lockedTarget?.type, "edge");
    assert.equal(stateAfter.lockedTarget?.id, edge.id);
    assert.equal(container.getAttribute("data-metadata-checker-graph-viewport-target"), `edge:${edge.id}`);
    assert.equal((stateAfter.viewport.target?.id || ""), edge.id);
    const afterEdgeWidth = edgeViewAfter?.sprite?.width || 0;
    assert.equal(afterEdgeWidth >= beforeEdgeWidth, true);
    assert.equal(afterEdgeWidth > beforeEdgeWidth, true);
    assert.equal(afterFromView?.sprite?.alpha > 0, true);
    assert.equal(afterToView?.sprite?.alpha > 0, true);
  });

  it("supports balanced and compact density profile limits and keeps same data on interaction", async () => {
    const fixture = makeDenseFixture(120, 240);
    const document = createFakeDocument();
    const compactRoot = document.createElement("div");
    const balancedRoot = document.createElement("div");

    const compactRuntime = createFakePixiRuntime();
    const balancedRuntime = createFakePixiRuntime();

    const compactRenderer = createPixiLocalGraphRenderer({
      root: compactRoot,
      document,
      pixi: compactRuntime,
      width: 320,
      height: 220,
      maxRenderedNodes: 36,
      maxRenderedEdges: 90,
    });
    const compactResult = await compactRenderer.render(fixture);
    assert.equal(
      compactResult.graph.viewGraph.visibleNodeCount <= 36,
      true,
    );
    assert.equal(
      compactResult.graph.viewGraph.visibleEdgeCount <= 90,
      true,
    );
    assert.equal(compactRoot.getAttribute("data-metadata-checker-graph-density-profile"), "compact");

    const balancedRenderer = createPixiLocalGraphRenderer({
      root: balancedRoot,
      document,
      pixi: balancedRuntime,
      width: 320,
      height: 220,
      densityProfile: "balanced",
    });
    const balancedResult = await balancedRenderer.render(fixture);
    assert.equal(
      balancedResult.graph.viewGraph.visibleNodeCount <= 48,
      true,
    );
    assert.equal(
      balancedResult.graph.viewGraph.visibleEdgeCount <= 120,
      true,
    );
    assert.equal(balancedRoot.getAttribute("data-metadata-checker-graph-density-profile"), "balanced");

    const canvas = getCanvas(balancedRoot);
    assert.ok(canvas);
    const beforeNodes = balancedResult.graph.nodes.length;
    const beforeEdges = balancedResult.graph.edges.length;

    dispatchWheel(canvas, { deltaY: -300, offsetX: 160, offsetY: 110 });
    const afterState = balancedRenderer.getState();
    assert.equal(afterState.lastRender?.graph?.nodes?.length, beforeNodes);
    assert.equal(afterState.lastRender?.graph?.edges?.length, beforeEdges);
    assert.equal(afterState.lastRender?.graph?.viewGraph?.visibleNodeCount <= 48, true);
  });

  it("locks aggregate target and keeps onNodeFocus callback unchanged", async () => {
    const fixture = makeDenseFixture(50, 120);
    const document = createFakeDocument();
    const container = document.createElement("div");
    const callbacks = makeCallbackLog();
    const runtime = createFakePixiRuntime();
    const renderer = createPixiLocalGraphRenderer({
      root: container,
      document,
      pixi: runtime,
      width: 320,
      height: 220,
      densityProfile: "compact",
      maxRenderedNodes: 8,
      maxRenderedEdges: 16,
      onNodeFocus: callbacks.onNodeFocus,
    });

    const result = await renderer.render(fixture);
    const aggregateNode = result.graph.viewGraph.aggregateNodes[0];
    assert.ok(aggregateNode, "aggregate node should be produced for dense graph");

    const initialState = renderer.getState();
    const aggregateView = initialState.nodeViews.get(aggregateNode.id);
    assert.ok(aggregateView);

    aggregateView.onPointerTap();
    const snapshot = makeStateSnapshot(renderer);

    assert.equal(snapshot.state.lockedTarget?.type, "aggregate");
    assert.equal(snapshot.state.lockedTarget?.id, aggregateNode.id);
    assert.equal(snapshot.state.focusedGraphTarget?.type, "aggregate");
    assert.equal(snapshot.state.focusedGraphTarget?.id, aggregateNode.id);

    assert.equal(snapshot.detailText.includes("Aggregate"), true);
    assert.equal(container.getAttribute("data-metadata-checker-graph-detail-kind"), "aggregate");
    assert.equal(callbacks.onNodeFocus.length, 1);
  });

  it("clears locked target on background tap and restores focus detail", async () => {
    const fixture = await loadFixture();
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createPixiLocalGraphRenderer({
      root: container,
      document,
      pixi: createFakePixiRuntime(),
      width: 320,
      height: 220,
    });

    const result = await renderer.render(fixture);
    const edgeId = result.graph.viewGraph.edges[0]?.id;
    assert.ok(edgeId);

    renderer.triggerEdgeEvent(edgeId, "click");
    let snapshot = makeStateSnapshot(renderer);
    assert.equal(snapshot.lockedTarget.startsWith("edge:"), true);
    assert.equal(snapshot.detailKind, "edge");

    const canvas = getCanvas(container);
    assert.ok(canvas);
    dispatchBackgroundTap(canvas);
    snapshot = makeStateSnapshot(renderer);
    assert.equal(snapshot.state.lockedTarget, null);
    assert.equal(snapshot.lockedTarget, "none");
    assert.equal(snapshot.state.focusedGraphTarget?.type, "focus");
    assert.equal(snapshot.state.focusedGraphTarget?.id, fixture.focus_node);
    assert.equal(snapshot.state.designerFocusNodeId, fixture.focus_node);
  });

  it("keeps locked detail while hovering another target", async () => {
    const fixture = await loadFixture();
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createPixiLocalGraphRenderer({
      root: container,
      document,
      pixi: createFakePixiRuntime(),
      width: 320,
      height: 220,
    });

    const result = await renderer.render(fixture);
    const edgeId = result.graph.viewGraph.edges[0]?.id;
    assert.ok(edgeId);

    renderer.triggerEdgeEvent(edgeId, "click");
    const lockedDetail = getDetailHostText(container);
    assert.ok(lockedDetail.includes("Edge"));

    const hoverNodeId = "mdl_order_status";
    renderer.triggerNodeEvent(hoverNodeId, "hover");
    assert.equal(getDetailHostText(container), lockedDetail);
    assert.equal(container.getAttribute("data-metadata-checker-graph-locked-target").startsWith("edge:"), true);
  });

  it("exports visible graph text for copy", async () => {
    const fixture = await loadFixture();
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createPixiLocalGraphRenderer({
      root: container,
      document,
      pixi: createFakePixiRuntime(),
      width: 320,
      height: 220,
    });

    await renderer.render(fixture);
    const text = renderer.getVisibleGraphText();
    assert.ok(text.includes("focus:"));
    assert.ok(text.includes("nodes:"));
    assert.ok(text.includes("edges:"));
    assert.ok(text.includes(`focus: ${fixture.focus_node}`));
  });

  it("draws edges with Pixi v8 stroke instead of deprecated lineStyle", async () => {
    const fixture = {
      status: "ready",
      focus_node: "n1",
      nodes: [
        { id: "n1", label: "one", kind: "component", depth: 0 },
        { id: "n2", label: "two", kind: "model", depth: 1 },
      ],
      edges: [
        { id: "e1", from: "n1", to: "n2", kind: "reads", priority: "source" },
      ],
      source_summary: { total_nodes: 2, total_edges: 1, node_kinds: {}, edge_kinds: {} },
      diagnostics: [],
    };
    const pixi = createFakePixiRuntime();
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createPixiLocalGraphRenderer({
      root: container,
      document,
      pixi,
      width: 320,
      height: 220,
    });
    await renderer.render(fixture);
    const graphics = [];
    const walk = (node) => {
      if (!node) return;
      if (Array.isArray(node.commands)) graphics.push(node);
      for (const child of node.children || []) walk(child);
    };
    walk(pixi.applications[0]?.stage);
    const stroked = graphics.filter((item) => item.commands.some((cmd) => cmd[0] === "stroke"));
    assert.ok(stroked.length > 0, "expected edge graphics to call stroke()");
    assert.equal(
      graphics.some((item) => item.commands.some((cmd) => cmd[0] === "lineStyle")),
      false,
      "deprecated lineStyle should not be used when stroke is available",
    );
  });

  it("prefers filter and source nodes over downstream components in compact view", async () => {
    const fixture = {
      status: "ready",
      focus_node: "comp:focus",
      target: "comp:focus",
      depth: 2,
      visible_hop: 1,
      nodes: [
        { id: "comp:focus", label: "focus", kind: "component", depth: 0 },
        { id: "mdl_filter", label: "filter", kind: "model", depth: 1 },
        { id: "mdl_source", label: "source", kind: "model", depth: 1 },
        { id: "comp:down1", label: "down1", kind: "component", depth: 1 },
        { id: "comp:down2", label: "down2", kind: "component", depth: 1 },
        { id: "comp:down3", label: "down3", kind: "component", depth: 1 },
      ],
      edges: [
        { id: "e_filter", from: "comp:focus", to: "mdl_filter", kind: "condition", priority: "filter" },
        { id: "e_source", from: "mdl_source", to: "comp:focus", kind: "reads", priority: "source" },
        { id: "e_down1", from: "comp:focus", to: "comp:down1", kind: "writes", priority: "action" },
        { id: "e_down2", from: "comp:focus", to: "comp:down2", kind: "writes", priority: "action" },
        { id: "e_down3", from: "comp:focus", to: "comp:down3", kind: "writes", priority: "action" },
      ],
      source_summary: { total_nodes: 6, total_edges: 5, node_kinds: {}, edge_kinds: {} },
      diagnostics: [],
    };
    const document = createFakeDocument();
    const container = document.createElement("div");
    const renderer = createPixiLocalGraphRenderer({
      root: container,
      document,
      pixi: createFakePixiRuntime(),
      width: 320,
      height: 220,
      maxRenderedNodes: 4,
      maxRenderedEdges: 8,
    });

    const result = await renderer.render(fixture);
    const visibleIds = new Set(result.graph.viewGraph.nodes.map((node) => node.id));
    assert.ok(visibleIds.has("mdl_filter"));
    assert.ok(visibleIds.has("mdl_source"));
    assert.ok(!visibleIds.has("comp:down3") || result.graph.viewGraph.aggregateNodeCount > 0);
  });
});

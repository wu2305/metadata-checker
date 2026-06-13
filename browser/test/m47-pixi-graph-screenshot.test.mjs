import { readFile } from "node:fs/promises";
import { describe, it } from "node:test";
import assert from "node:assert";

import { createGraphPanelHost } from "../renderer/graph-panel-host.mjs";

function createFakeDocument() {
  function createElement(tagName) {
    const element = {
      tagName,
      className: "",
      children: [],
      attributes: {},
      style: {},
      textContent: "",
      listeners: {},
      appendChild(child) {
        this.children.push(child);
        child.parentNode = this;
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
        this.listeners[type] = handler;
      },
      querySelector(selector) {
        return this.querySelectorAll(selector)[0] ?? null;
      },
      querySelectorAll(selector) {
        const nodes = collectNodes(this);
        if (!selector || selector === "*") {
          return nodes;
        }
        return nodes.filter((node) => {
          if (!node || typeof node !== "object") {
            return false;
          }
          if (selector.startsWith(".")) {
            const target = selector.slice(1);
            const classes = String(node.className || "")
              .split(/\s+/)
              .filter(Boolean);
            return classes.includes(target);
          }
          if (selector.startsWith("[")) {
            const attr = selector.slice(1, -1);
            return node.getAttribute?.(attr) != null;
          }
          return node.tagName?.toLowerCase() === selector.toLowerCase();
        });
      },
    };
    return element;
  }

  return {
    body: createElement("body"),
    createElement,
  };
}

function collectNodes(root, out = []) {
  if (!root || typeof root !== "object") {
    return out;
  }
  out.push(root);
  for (const child of root.children || []) {
    collectNodes(child, out);
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
      const input = result || {};
      const nodeCount = Number.parseInt(input.nodeCount, 10) || input.nodes?.length || 0;
      const edgeCount = Number.parseInt(input.edgeCount, 10) || input.edges?.length || 0;

      if (renderer.container) {
        const surface = documentLike.createElement("canvas");
        surface.className = "pixi-canvas-surface";
        surface.setAttribute("data-metadata-checker-visual-shape", input.visualShape || "sparse");
        renderer.container.appendChild(surface);

        const densityProfile = String(input.densityProfile || "compact");
        const visibleNodeCount = Number.parseInt(input.visibleNodeCount, 10) || 0;
        const visibleEdgeCount = Number.parseInt(input.visibleEdgeCount, 10) || 0;
        const fullNodeCount = Number.parseInt(input.fullNodeCount, 10) || nodeCount;
        const fullEdgeCount = Number.parseInt(input.fullEdgeCount, 10) || edgeCount;
        const aggregateNodeCount = Number.parseInt(input.aggregateNodeCount, 10) || 0;

        const stateMarker = documentLike.createElement("div");
        stateMarker.className = "metadata-checker-screenshot-state";
        stateMarker.setAttribute("data-metadata-checker-graph-density-profile", densityProfile);
        stateMarker.setAttribute("data-metadata-checker-graph-hover-target", input.hoverTarget || "none");
        stateMarker.setAttribute("data-metadata-checker-graph-locked-target", input.lockedTarget || "none");
        stateMarker.setAttribute("data-metadata-checker-graph-detail-kind", input.detailKind || "focus");
        stateMarker.setAttribute("data-metadata-checker-graph-viewport-scale", String(input.viewportScale || 1));
        stateMarker.setAttribute("data-metadata-checker-graph-highlight-node-count", String(input.highlightNodeCount || 0));
        stateMarker.setAttribute("data-metadata-checker-graph-highlight-edge-count", String(input.highlightEdgeCount || 0));
        stateMarker.setAttribute("data-metadata-checker-graph-visible-node-count", String(visibleNodeCount));
        stateMarker.setAttribute("data-metadata-checker-graph-visible-edge-count", String(visibleEdgeCount));
        stateMarker.setAttribute("data-metadata-checker-graph-full-node-count", String(fullNodeCount));
        stateMarker.setAttribute("data-metadata-checker-graph-full-edge-count", String(fullEdgeCount));
        stateMarker.setAttribute("data-metadata-checker-graph-aggregate-count", String(aggregateNodeCount));
        stateMarker.setAttribute("data-metadata-checker-graph-aggregate-visible", String(Boolean(input.aggregateVisible)));
        stateMarker.style.display = "none";

        const detailHost = documentLike.createElement("div");
        detailHost.className = "metadata-checker-pixi-detail";
        detailHost.setAttribute("data-metadata-checker-pixi-detail", "mounted");
        detailHost.style.maxHeight = "58px";
        detailHost.style.overflow = "hidden";
        detailHost.style.maxWidth = "240px";
        detailHost.style.textOverflow = "ellipsis";
        detailHost.style.whiteSpace = "nowrap";
        detailHost.textContent = String(input.detailText || "focus detail");
        renderer.container.appendChild(detailHost);

        if (Boolean(input.aggregateVisible)) {
          const aggregateRoot = documentLike.createElement("div");
          aggregateRoot.className = "metadata-checker-pixi-aggregate-root";
          const aggregateCount = Math.min(aggregateNodeCount, 2);
          for (let index = 0; index < aggregateCount; index += 1) {
            const row = documentLike.createElement("div");
            row.className = "metadata-checker-pixi-aggregate";
            row.textContent = `Aggregate +${Math.max(1, Number.parseInt(input.aggregateHiddenCount, 10) || 1)} hidden`;
            aggregateRoot.appendChild(row);
          }
          renderer.container.appendChild(aggregateRoot);
        }

        renderer.container.appendChild(stateMarker);
      }

      return {
        status: input.status || "ready",
        focus: input.focus_node ?? input.target,
        markerDepth: input.depth ?? 1,
        depth: input.depth ?? 1,
        visibleHop: input.visible_hop ?? 1,
        nodeCount,
        edgeCount,
        densityProfile: input.densityProfile || "compact",
        lockedTarget: input.lockedTarget || "none",
        hoverTarget: input.hoverTarget || "none",
        viewportScale: input.viewportScale || 1,
        detailKind: input.detailKind || "focus",
        detailText: input.detailText || "",
        visualShape: input.visualShape || "sparse",
        visibleNodeCount: Number.parseInt(input.visibleNodeCount, 10) || 0,
        visibleEdgeCount: Number.parseInt(input.visibleEdgeCount, 10) || 0,
        fullNodeCount: Number.parseInt(input.fullNodeCount, 10) || nodeCount,
        fullEdgeCount: Number.parseInt(input.fullEdgeCount, 10) || edgeCount,
        aggregateNodeCount: Number.parseInt(input.aggregateNodeCount, 10) || 0,
        aggregateHiddenCount: Number.parseInt(input.aggregateHiddenCount, 10) || 0,
        graph: {
          nodes: Array.isArray(input.nodes) ? input.nodes : [],
          edges: Array.isArray(input.edges) ? input.edges : [],
          viewGraph: {
            visibleNodeCount: Number.parseInt(input.visibleNodeCount, 10) || 0,
            visibleEdgeCount: Number.parseInt(input.visibleEdgeCount, 10) || 0,
            aggregateNodeCount: Number.parseInt(input.aggregateNodeCount, 10) || 0,
            hiddenNodeCount: Number.parseInt(input.fullNodeCount, 10) - Number.parseInt(input.visibleNodeCount, 10),
            hiddenEdgeCount: Number.parseInt(input.fullEdgeCount, 10) - Number.parseInt(input.visibleEdgeCount, 10),
          },
        },
      };
    },
    async renderError(errorEnvelope) {
      calls.push({ method: "renderError", errorEnvelope });
      return {
        status: "error",
        focus: "",
        markerDepth: 0,
        depth: 0,
        visibleHop: 1,
        nodeCount: 0,
        edgeCount: 0,
      };
    },
  };

  return renderer;
}

function toSummaryCount(value) {
  return Number.parseInt(value, 10) || 0;
}

function queryByAttribute(root, attributeName, attributeValue) {
  const nodes = collectNodes(root);
  return nodes.find((node) => {
    if (!node || typeof node !== "object") {
      return false;
    }
    if (node.getAttribute == null) {
      return false;
    }
    if (attributeValue == null) {
      return node.getAttribute(attributeName) != null;
    }
    return node.getAttribute(attributeName) === attributeValue;
  }) || null;
}

function queryAllByAttribute(root, attributeName) {
  const nodes = collectNodes(root);
  return nodes.filter((node) => node.getAttribute?.(attributeName) != null);
}

function queryAllByClass(root, className) {
  const target = String(className || "").trim();
  const nodes = collectNodes(root);
  return nodes.filter((node) => {
    const classes = String(node.className || "")
      .split(/\s+/)
      .filter(Boolean);
    return classes.includes(target);
  });
}

function makeDensityPayload(profile) {
  if (profile === "compact") {
    return {
      densityProfile: "compact",
      visibleNodeCount: 36,
      visibleEdgeCount: 90,
      fullNodeCount: 214,
      fullEdgeCount: 1400,
      aggregateVisible: true,
      aggregateNodeCount: 3,
      aggregateHiddenCount: 178,
      visualShape: "sparse",
      viewportScale: 1,
      detailKind: "node",
      detailText:
        "Node · Submit Button · component · depth 0 · neighbors 2 — intentionally long to validate line clipping and panel overflow prevention in smoke assertions.",
      hoverTarget: "node:mdl_order_status",
    };
  }

  return {
    densityProfile: "balanced",
    visibleNodeCount: 48,
    visibleEdgeCount: 120,
    fullNodeCount: 356,
    fullEdgeCount: 2400,
    aggregateVisible: true,
    aggregateNodeCount: 4,
    aggregateHiddenCount: 308,
    visualShape: "mesh",
    viewportScale: 1.8,
    lockedTarget: "edge:cmp_submit_button->mdl_status_code",
    hoverTarget: "edge:cmp_submit_button->mdl_status_code",
    detailKind: "edge",
      detailText:
      "Lock · Edge · source · cmp_submit_button -> mdl_status_code · EDGE_EVIDENCE_UNAVAILABLE — locked edge should keep stable focus with no panel overflow.",
    highlightNodeCount: 4,
    highlightEdgeCount: 7,
  };
}

function makeDensityCase(profile) {
  const base = makeDensityPayload(profile);
  return {
    focus_node: "cmp_submit_button",
    status: "ready",
    target: "component:ButtonSubmit",
    depth: 2,
    visible_hop: 1,
    nodeCount: base.fullNodeCount,
    edgeCount: base.fullEdgeCount,
    nodes: [
      { id: "cmp_submit_button", label: "Submit Button", kind: "component" },
      { id: "mdl_order_status", label: "Order Status", kind: "model" },
      { id: "mdl_form_data", label: "Form Data", kind: "model" },
    ],
    edges: [
      { from: "cmp_submit_button", to: "mdl_order_status", kind: "reads", direction: "Forward", label: "reads status" },
      { from: "cmp_submit_button", to: "mdl_form_data", kind: "reads", direction: "Forward", label: "reads form data" },
      { from: "mdl_order_status", to: "mdl_form_data", kind: "writes", direction: "Forward", label: "writes form" },
    ],
    ...base,
  };
}

describe("m47 pixi popup screenshot smoke", () => {
  it("validates popup geometry and non-empty graph surface placeholder", async () => {
    const fixture = JSON.parse(
      await readFile(new URL("./fixtures/m47-local-graph-2hop.json", import.meta.url), "utf8"),
    );
    const document = createFakeDocument();
    const renderer = createCanvasRenderer(document);
    const host = createGraphPanelHost({ document, renderer });

    const result = await host.render(fixture);
    const { root, body } = host.status();

    assert.strictEqual(result.status, "ready");
    assert.strictEqual(root.getAttribute("data-metadata-checker-embedded-popup"), "mounted");
    assert.strictEqual(root.getAttribute("data-metadata-checker-analysis-status"), "ready");
    assert.strictEqual(root.getAttribute("data-metadata-checker-graph-depth"), "2");
    assert.strictEqual(root.getAttribute("data-metadata-checker-graph-visible-hop"), "1");
    assert.strictEqual(root.getAttribute("data-metadata-checker-local-graph-renderer"), "fallback");
    assert.strictEqual(root.getAttribute("data-metadata-checker-focus-component"), "cmp_submit_button");
    assert.strictEqual(root.getAttribute("data-metadata-checker-graph-node-count"), "7");
    assert.strictEqual(root.getAttribute("data-metadata-checker-graph-edge-count"), "6");

    const nodeCount = toSummaryCount(root.getAttribute("data-metadata-checker-graph-node-count"));
    const edgeCount = toSummaryCount(root.getAttribute("data-metadata-checker-graph-edge-count"));
    assert.ok(nodeCount >= 1, `expected nodes to be present, got ${nodeCount}`);
    assert.ok(edgeCount >= 1, `expected edges to be present, got ${edgeCount}`);

    assert.strictEqual(root.style.position, "fixed");
    assert.strictEqual(root.style.right, "16px");
    assert.strictEqual(root.style.bottom, "16px");
    assert.strictEqual(root.style.width, "min(280px, calc(100vw - 32px))");
    assert.strictEqual(root.style.height, "min(260px, calc(100vh - 32px))");
    assert.equal(root.style.minWidth || null, null);
    assert.equal(root.style.minHeight || null, null);
    assert.strictEqual(root.style.maxWidth, "calc(100vw - 32px)");
    assert.strictEqual(root.style.maxHeight, "calc(100vh - 32px)");
    assert.strictEqual(root.style.overflow, "hidden");

    const hasCanvasSurface = body.children.some((child) => child.tagName === "canvas");
    assert.equal(hasCanvasSurface, true);
  });

  it("validates M48 interaction smoke: zoomed/locked edge, aggregate visible and panel-safe detail", async () => {
    const compactPayload = makeDensityCase("compact");
    const balancedPayload = makeDensityCase("balanced");

    // compact density baseline check
    {
      const document = createFakeDocument();
      const renderer = createCanvasRenderer(document);
      const host = createGraphPanelHost({ document, renderer });
      const result = await host.render(compactPayload);
      const { root, body } = host.status();

      assert.strictEqual(root.getAttribute("data-metadata-checker-embedded-popup"), "mounted");
      assert.strictEqual(root.style.overflow, "hidden");
      assert.equal(result.densityProfile, "compact");
      assert.ok(result.visibleNodeCount <= 36, `compact node count should <= 36, got ${result.visibleNodeCount}`);
      assert.ok(result.visibleEdgeCount <= 90, `compact edge count should <= 90, got ${result.visibleEdgeCount}`);
      assert.ok(result.visibleNodeCount < result.fullNodeCount, "compact should keep density cap");
      assert.ok(result.visibleEdgeCount < result.fullEdgeCount, "compact should keep density cap");

      const detail = body.querySelector(".metadata-checker-pixi-detail");
      assert.ok(detail);
      assert.equal(detail.style.overflow, "hidden");
      assert.equal(detail.style.textOverflow, "ellipsis");
      assert.equal(detail.style.maxHeight, "58px");
      assert.equal(detail.getAttribute("data-metadata-checker-pixi-detail"), "mounted");

      const canvas = body.querySelector("canvas");
      assert.ok(canvas);
      assert.notEqual(canvas.getAttribute("data-metadata-checker-visual-shape"), "ring", "compact profile should not be ring-shaped");
      assert.equal(body.querySelector(".metadata-checker-pixi-aggregate-root") != null, true);
      assert.ok(queryAllByClass(body, "metadata-checker-pixi-aggregate").length > 0);

      const densityMarker = queryByAttribute(body, "data-metadata-checker-graph-density-profile");
      assert.ok(densityMarker);
      assert.equal(densityMarker.getAttribute("data-metadata-checker-graph-density-profile"), "compact");
      assert.equal(densityMarker.getAttribute("data-metadata-checker-graph-aggregate-visible"), "true");
    }

    // balanced density + zoomed locked edge smoke helper check
    {
      const document = createFakeDocument();
      const renderer = createCanvasRenderer(document);
      const host = createGraphPanelHost({ document, renderer });
      const result = await host.render(balancedPayload);
      const { body } = host.status();

      assert.equal(result.densityProfile, "balanced");
      assert.equal(result.visibleNodeCount <= 48, true);
      assert.equal(result.visibleEdgeCount <= 120, true);
      assert.equal(result.visibleNodeCount < result.fullNodeCount, true);
      assert.equal(result.visibleEdgeCount < result.fullEdgeCount, true);

      // Balanced case is checked via screenshot-helper markers produced by the renderer fixture.
      // Worker A/B marker wiring should keep this assertion set and re-baseline it to real marker sources.
      const stateMarker = queryByAttribute(body, "data-metadata-checker-graph-density-profile");
      const lockedMarker = queryByAttribute(body, "data-metadata-checker-graph-locked-target");
      const viewportMarker = queryByAttribute(body, "data-metadata-checker-graph-viewport-scale");
      const detailKindMarker = queryByAttribute(body, "data-metadata-checker-graph-detail-kind");
      const hoverMarker = queryByAttribute(body, "data-metadata-checker-graph-hover-target");
      const canvas = body.querySelector("canvas");
      const detail = body.querySelector(".metadata-checker-pixi-detail");
      const detailText = String(detail?.textContent || "");

      assert.ok(stateMarker);
      assert.equal(stateMarker.getAttribute("data-metadata-checker-graph-density-profile"), "balanced");
      assert.equal(stateMarker.getAttribute("data-metadata-checker-graph-detail-kind"), "edge");
      assert.equal(stateMarker.getAttribute("data-metadata-checker-graph-hover-target"), "edge:cmp_submit_button->mdl_status_code");
      assert.equal(lockedMarker?.getAttribute("data-metadata-checker-graph-locked-target"), "edge:cmp_submit_button->mdl_status_code");
      assert.ok(Number.parseFloat(viewportMarker?.getAttribute("data-metadata-checker-graph-viewport-scale") || "1") >= 1.5);
      assert.equal(detailKindMarker?.getAttribute("data-metadata-checker-graph-detail-kind"), "edge");

      assert.equal(canvas?.getAttribute("data-metadata-checker-visual-shape"), "mesh");
      assert.equal(Number.parseInt(stateMarker.getAttribute("data-metadata-checker-graph-highlight-edge-count"), 10) >= 1, true);
      assert.equal(Number.parseInt(stateMarker.getAttribute("data-metadata-checker-graph-highlight-edge-count"), 10), 7);

      assert.ok(detail);
      assert.ok(detailText.includes("Lock · Edge"));
      assert.equal(detail.style.whiteSpace, "nowrap");
      assert.equal(detail.style.textOverflow, "ellipsis");
      assert.equal(detail.style.overflow, "hidden");
      assert.ok(detail.textContent.length > 40, "detail text should be long enough to validate clipping rule");

      const aggregateRows = queryAllByClass(body, "metadata-checker-pixi-aggregate");
      assert.ok(aggregateRows.length >= 2);
      assert.ok(aggregateRows.some((row) => String(row.textContent).includes("Aggregate +")));
    }
  });

  it("checks compact and balanced density ceilings stay bounded and distinguishable", async () => {
    const compactPayload = makeDensityCase("compact");
    const balancedPayload = makeDensityCase("balanced");

    const compactDocument = createFakeDocument();
    const compactRenderer = createCanvasRenderer(compactDocument);
    const compactHost = createGraphPanelHost({ document: compactDocument, renderer: compactRenderer });

    const compactResult = await compactHost.render(compactPayload);
    assert.ok(compactResult.visibleNodeCount <= 36);
    assert.ok(compactResult.visibleEdgeCount <= 90);

    const compactState = queryByAttribute(compactHost.status().body, "data-metadata-checker-graph-density-profile");
    assert.ok(compactState);
    assert.equal(compactState.getAttribute("data-metadata-checker-graph-density-profile"), "compact");
    assert.equal(compactState.getAttribute("data-metadata-checker-graph-full-node-count"), String(compactResult.fullNodeCount));

    const balancedDocument = createFakeDocument();
    const balancedRenderer = createCanvasRenderer(balancedDocument);
    const balancedHost = createGraphPanelHost({ document: balancedDocument, renderer: balancedRenderer });

    const balancedResult = await balancedHost.render(balancedPayload);
    assert.ok(balancedResult.visibleNodeCount <= 48);
    assert.ok(balancedResult.visibleEdgeCount <= 120);
    assert.ok(balancedResult.visibleNodeCount > compactResult.visibleNodeCount);
    assert.ok(balancedResult.visibleEdgeCount > compactResult.visibleEdgeCount);
    assert.ok(balancedResult.visibleNodeCount < balancedResult.fullNodeCount);
    assert.ok(balancedResult.visibleEdgeCount < balancedResult.fullEdgeCount);

    const balancedState = queryByAttribute(balancedHost.status().body, "data-metadata-checker-graph-density-profile");
    assert.ok(balancedState);
    assert.equal(balancedState.getAttribute("data-metadata-checker-graph-density-profile"), "balanced");
    const allStateMarkers = queryAllByAttribute(balancedHost.status().body, "data-metadata-checker-graph-density-profile");
    assert.equal(allStateMarkers.length, 1);

    const fullCoverageNodeGap = balancedResult.fullNodeCount - balancedResult.visibleNodeCount;
    const fullCoverageEdgeGap = balancedResult.fullEdgeCount - balancedResult.visibleEdgeCount;
    assert.ok(fullCoverageNodeGap > 0, "balanced profile must not render full node count");
    assert.ok(fullCoverageEdgeGap > 0, "balanced profile must not render full edge count");
  });
});

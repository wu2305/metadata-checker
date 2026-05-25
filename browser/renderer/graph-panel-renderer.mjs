import { layoutGraph, normalizeVisualGraphEnvelope, buildMermaidText, buildEChartsOption } from "./graph-layout.mjs";
import { renderGraphPanelDOM, setGraphMarkers } from "./graph-dom.mjs";

function createEmptyRenderModel() {
  return {
    nodes: [],
    edges: [],
    groups: [],
    focus_node: null,
    diagnostics: [],
    truncated: true,
    source_summary: {
      total_nodes: 0,
      total_edges: 0,
      node_kinds: {},
      edge_kinds: {},
    },
    depth: 1,
  };
}

function normalizeOptions(rawOptions = {}) {
  return {
    maxDepth: rawOptions.maxDepth ?? 3,
    maxNodes: rawOptions.maxNodes ?? 120,
    maxEdges: rawOptions.maxEdges ?? 360,
    renderer: rawOptions.renderer ?? "auto",
    nodeLabelMaxLength: rawOptions.nodeLabelMaxLength ?? 56,
    edgeLabelMaxLength: rawOptions.edgeLabelMaxLength ?? 48,
    onEvent: typeof rawOptions.onEvent === "function" ? rawOptions.onEvent : null,
    onDiagnostic: typeof rawOptions.onDiagnostic === "function" ? rawOptions.onDiagnostic : null,
    onRender: typeof rawOptions.onRender === "function" ? rawOptions.onRender : null,
    container: rawOptions.container ?? null,
    echarts: rawOptions.echarts ?? null,
    document: rawOptions.document ?? globalThis.document,
  };
}

function toGraphInput(value) {
  if (!value || typeof value !== "object") {
    return createEmptyRenderModel();
  }

  const normalized = normalizeVisualGraphEnvelope(value);
  if (!normalized || typeof normalized !== "object") {
    return createEmptyRenderModel();
  }
  return normalized;
}

function buildErrorGraph(result) {
  const source = (result?.diagnostics ?? []).map((diag) => ({
    severity: "error",
    code: diag?.code ?? "RENDER_ERROR",
    message: diag?.message ?? "unknown renderer error",
    location: diag?.location ?? {},
  }));
  return {
    nodes: [],
    edges: [],
    groups: [],
    focus_node: null,
    diagnostics: source,
    truncated: false,
    source_summary: {
      total_nodes: 0,
      total_edges: 0,
      node_kinds: {},
      edge_kinds: {},
    },
    depth: 1,
    status: "error",
    truncatedReason: "renderer_error",
  };
}

function makeElementHost(container, options) {
  if (!container) {
    if (options.document && typeof options.document.createElement === "function") {
      return options.document.createElement("div");
    }
    return {
      attributes: {},
      className: "",
      children: [],
      style: {},
      appendChild() {},
      setAttribute(name, value) {
        this.attributes[name] = String(value);
      },
      getAttribute(name) {
        return this.attributes[name] ?? null;
      },
      replaceChildren() {
        this.children = [];
      },
    };
  }
  return container;
}

async function renderWithEcharts(root, layout, options) {
  if (!options.echarts) {
    return false;
  }

  const optionPayload = buildEChartsOption(layout);
  try {
    let chart = options.echarts;
    if (typeof chart === "function") {
      chart = chart(root, optionPayload);
      if (chart && typeof chart.setOption === "function") {
        chart.setOption(optionPayload);
        return true;
      }
      if (chart && typeof chart === "object" && chart.then) {
        const awaited = await chart;
        if (awaited && typeof awaited.setOption === "function") {
          awaited.setOption(optionPayload);
          return true;
        }
      }
      return false;
    }

    if (chart && typeof chart.init === "function") {
      const chartInstance = chart.init(root);
      if (chartInstance && typeof chartInstance.setOption === "function") {
        chartInstance.setOption(optionPayload);
        return true;
      }
      return false;
    }

    if (chart && typeof chart.setOption === "function") {
      chart.setOption(optionPayload);
      return true;
    }
  } catch {
    return false;
  }

  return false;
}

function emitExpandEvent(callbacks, node) {
  const normalized = extractNodePayloadForExpand(node);
  const payload = {
    type: "expand_requested",
    event: "click",
    nodeId: normalized.nodeId ?? normalized.legacyNodeId,
    legacyNodeId: normalized.legacyNodeId,
    node,
    timestamp: Date.now(),
    depth: normalized.depth,
    target: normalized.target,
    expand_token: normalized.expand_token ?? normalized.expandToken,
    collapsed: normalized.collapsed,
  };
  if (typeof callbacks?.onEvent === "function") {
    callbacks.onEvent(payload);
  }
}

function extractNodePayloadForExpand(node) {
  const expandToken = node?.expand_token ?? node?.expandToken ?? node?.metadata?.expand_token ?? null;
  return {
    nodeId: node?.id ?? node?.nodeId ?? node?.node_id ?? null,
    legacyNodeId: node?.nodeId ?? null,
    depth: node?.depth ?? null,
    target: node?.target ?? null,
    expandToken,
    expand_token: expandToken,
    collapsed: Boolean(node?.collapsed),
  };
}

export function renderVisualGraph(container, input, options) {
  const renderer = createGraphPanelRenderer({ ...options, container });
  return renderer.render(input);
}

export function createGraphPanelRenderer(rawOptions = {}) {
  const options = normalizeOptions(rawOptions);
  const root = makeElementHost(options.container, options);
  let lastRender = null;

  function getRendererType(usedEcharts) {
    if (usedEcharts) return "echarts";
    return options.renderer === "svg" ? "svg" : "html";
  }

  function onExpandClick(node) {
    emitExpandEvent(options, extractNodePayloadForExpand(node));
  }

  function toMarkerDepth(graphLayout) {
    return graphLayout.depth ?? graphLayout.maxDepth ?? options.maxDepth;
  }

  function render(input) {
    let normalized = toGraphInput(input);
    if (normalized?.focus_node == null) {
      normalized.focus_node = null;
    }

    if (!Array.isArray(normalized.diagnostics) || normalized.diagnostics.length === 0) {
      normalized.diagnostics = [];
    }

    let layout;
    try {
      layout = layoutGraph(normalized, {
        maxDepth: options.maxDepth,
        maxNodes: options.maxNodes,
        maxEdges: options.maxEdges,
        nodeLabelMaxLength: options.nodeLabelMaxLength,
        edgeLabelMaxLength: options.edgeLabelMaxLength,
      });
      layout.diagnostics = normalized.diagnostics;
      layout.source_summary = normalized.source_summary;
      layout.truncatedReason = normalized.truncatedReason ?? "";
    } catch (error) {
      if (typeof options.onDiagnostic === "function") {
        options.onDiagnostic(error);
      }
      normalized = buildErrorGraph(input);
      layout = layoutGraph(normalized, options);
      layout.truncatedReason = "layout_failed";
    }

    const mermaid = buildMermaidText(layout);
    const echartsOption = buildEChartsOption(layout);

    return Promise.resolve(renderWithEcharts(root, layout, options))
      .then((usedEcharts) => {
        const renderer = getRendererType(usedEcharts);
        if (!usedEcharts) {
          renderGraphPanelDOM(root, layout, {
            document: options.document,
            renderer,
            onExpand: (payload) => {
              onExpandClick(payload.node ?? payload);
            },
          });
        } else {
          setGraphMarkers(root, {
            nodeCount: layout.nodeCount ?? 0,
            edgeCount: layout.edgeCount ?? 0,
            focus: layout.focus_node,
            truncated: Boolean(layout.truncated),
            depth: toMarkerDepth(layout),
            renderer,
          });
          root.setAttribute("data-metadata-checker-renderer", renderer);
        }

        if (typeof options.onRender === "function") {
          const summary = {
            nodes: layout.nodes?.length ?? 0,
            edges: layout.edges?.length ?? 0,
            diagnostics: layout.diagnostics?.length ?? 0,
            truncated: layout.truncated,
            depth: toMarkerDepth(layout),
          };
          options.onRender(summary);
        }

        lastRender = {
          graph: layout,
          mermaid,
          echartsOption,
          renderer,
          truncated: layout.truncated,
          focus: layout.focus_node,
        };
        return {
          ...lastRender,
          markerDepth: toMarkerDepth(layout),
          markerRenderer: renderer,
        };
      })
      .catch(() => {
        const fallbackRenderer = "html";
        setGraphMarkers(root, {
          nodeCount: 0,
          edgeCount: 0,
          focus: null,
          truncated: true,
          depth: toMarkerDepth(layout || {}),
          renderer: fallbackRenderer,
        });
        const fallbackPayload = buildErrorGraph(input);
        lastRender = { graph: fallbackPayload, mermaid: "", echartsOption: null, renderer: fallbackRenderer };
        return lastRender;
      });
  }

  function renderError(errorEnvelope) {
    return render(buildErrorGraph(errorEnvelope));
  }

  function clear() {
    if (root && typeof root.replaceChildren === "function") {
      root.replaceChildren();
    }
    setGraphMarkers(root, {
      nodeCount: 0,
      edgeCount: 0,
      focus: "",
      truncated: false,
      depth: 0,
      renderer: options.renderer,
    });
    lastRender = null;
  }

  return {
    render,
    renderAnalysis: render,
    renderError,
    clear,
    getLastRender() {
      return lastRender;
    },
  };
}

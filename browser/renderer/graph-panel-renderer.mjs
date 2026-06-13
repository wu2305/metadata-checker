import { layoutGraph, normalizeVisualGraphEnvelope, buildMermaidText, buildEChartsGraphOption } from "./graph-layout.mjs";
import { applyLocalGraphView } from "./local-graph-view.mjs";
import { renderGraphPanelDOM, setGraphMarkers } from "./graph-dom.mjs";
import { createPixiLocalGraphRenderer } from "./pixi-local-graph-renderer.mjs";
import { createEchartsLocalGraphRenderer } from "./echarts-local-graph-renderer.mjs";
import { createExtensionVendorRuntimeLoader, resolveExtensionRuntime } from "./extension-vendor-runtime-loader.mjs";

const DEFAULT_VISIBLE_HOP = 1;
const VIEWPORT_SCALE_DEFAULT = 1;

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
    visible_hop: 1,
  };
}

function parseIntLike(value, fallback = 0) {
  const parsed = Number.parseInt(value, 10);
  if (!Number.isFinite(parsed)) {
    return fallback;
  }
  return parsed;
}

function extractInputSummary(value = {}) {
  return {
    focus: value.focus_node ?? value.target ?? value.active_component_id ?? "",
    depth: parseIntLike(value.depth, 1),
    visible_hop: parseIntLike(value.visible_hop, parseIntLike(value.visibleHop, 1)),
    nodeCount: Array.isArray(value.nodes) ? value.nodes.length : 0,
    edgeCount: Array.isArray(value.edges) ? value.edges.length : 0,
  };
}

function normalizeOptions(rawOptions = {}) {
  return {
    maxDepth: rawOptions.maxDepth ?? 3,
    maxNodes: rawOptions.maxNodes ?? 36,
    maxEdges: rawOptions.maxEdges ?? 90,
    renderer: rawOptions.renderer ?? "auto",
    nodeLabelMaxLength: rawOptions.nodeLabelMaxLength ?? 56,
    edgeLabelMaxLength: rawOptions.edgeLabelMaxLength ?? 48,
    onEvent: typeof rawOptions.onEvent === "function" ? rawOptions.onEvent : null,
    onDiagnostic: typeof rawOptions.onDiagnostic === "function" ? rawOptions.onDiagnostic : null,
    onRender: typeof rawOptions.onRender === "function" ? rawOptions.onRender : null,
    container: rawOptions.container ?? null,
    width: rawOptions.width ?? 266,
    height: rawOptions.height ?? 188,
    echarts: rawOptions.echarts ?? null,
    pixi: rawOptions.pixi ?? null,
    d3Force3D: rawOptions.d3Force3D ?? rawOptions.d3Force3d ?? rawOptions.d3 ?? null,
    runtime: resolveExtensionRuntime(rawOptions),
    vendorRuntimeLoader: rawOptions.vendorRuntimeLoader
      ?? createExtensionVendorRuntimeLoader({ runtime: resolveExtensionRuntime(rawOptions) }),
    echartsRendererFactory: rawOptions.echartsRendererFactory ?? createEchartsLocalGraphRenderer,
    pixiRendererFactory: rawOptions.pixiRendererFactory ?? createPixiLocalGraphRenderer,
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
    visible_hop: 1,
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

  const optionPayload = buildEChartsGraphOption(layout);
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

function shouldAttemptBundledEcharts(options) {
  if (options.renderer === "html" || options.renderer === "svg" || options.renderer === "pixi") {
    return false;
  }
  return options.renderer === "auto" || options.renderer === "echarts";
}

function shouldAttemptPixi(options) {
  if (options.renderer === "html" || options.renderer === "svg" || options.renderer === "echarts") {
    return false;
  }
  return options.renderer === "auto" || options.renderer === "pixi";
}

function measureContainerSize(container, fallbackWidth, fallbackHeight) {
  const width = Number(container?.clientWidth) || Number(container?.offsetWidth) || fallbackWidth;
  const height = Number(container?.clientHeight) || Number(container?.offsetHeight) || fallbackHeight;
  return {
    width: Math.max(120, Number.isFinite(width) ? width : fallbackWidth),
    height: Math.max(96, Number.isFinite(height) ? height : fallbackHeight),
  };
}

async function loadPixiVendorRuntimes(options) {
  let pixi = options.pixi;
  let d3Force3D = options.d3Force3D;
  const loader = options.vendorRuntimeLoader;
  if ((!pixi || !d3Force3D) && loader && typeof loader.loadAll === "function") {
    const loaded = await loader.loadAll();
    pixi = pixi ?? loaded?.pixi ?? null;
    d3Force3D = d3Force3D ?? loaded?.d3Force3D ?? null;
  }
  return { pixi, d3Force3D };
}

async function loadEchartsRuntime(options) {
  if (options.echarts) {
    return options.echarts;
  }
  const loader = options.vendorRuntimeLoader;
  if (!loader) {
    return null;
  }
  if (typeof loader.loadEcharts === "function") {
    return loader.loadEcharts();
  }
  if (typeof loader.loadAll === "function") {
    const loaded = await loader.loadAll();
    return loaded?.echarts ?? null;
  }
  return null;
}

function clearRoot(root) {
  if (!root) return;
  if (typeof root.replaceChildren === "function") {
    root.replaceChildren();
    return;
  }
  if (Array.isArray(root.children)) {
    root.children = [];
  }
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
    legacyNodeId: node?.nodeId ?? node?.node_id ?? null,
    depth: node?.depth ?? null,
    target: node?.target ?? node?.metadata?.target ?? null,
    expandToken,
    expand_token: expandToken,
    collapsed: Boolean(node?.collapsed || node?.metadata?.collapsed === true),
  };
}

export function renderVisualGraph(container, input, options) {
  const renderer = createGraphPanelRenderer({ ...options, container });
  return renderer.render(input);
}

export function createGraphPanelRenderer(rawOptions = {}) {
  const options = normalizeOptions(rawOptions);
  let root = makeElementHost(options.container, options);
  let lastRender = null;
  let domController = null;
  let pixiController = null;
  let echartsController = null;

  async function renderWithBundledEcharts(layout, callbacks) {
    if (!shouldAttemptBundledEcharts(options)) {
      return null;
    }
    const echarts = await loadEchartsRuntime(options);
    if (!echarts) {
      if (typeof options.onDiagnostic === "function") {
        const loaderError = typeof options.vendorRuntimeLoader?.getLastLoadError === "function"
          ? options.vendorRuntimeLoader.getLastLoadError()
          : "";
        options.onDiagnostic(new Error(loaderError || "extension echarts vendor runtime is unavailable"));
      }
      if (root?.setAttribute) {
        root.setAttribute("data-metadata-checker-echarts-vendor", "missing");
        const loaderError = typeof options.vendorRuntimeLoader?.getLastLoadError === "function"
          ? options.vendorRuntimeLoader.getLastLoadError()
          : "";
        if (loaderError) {
          root.setAttribute("data-metadata-checker-echarts-vendor-error", loaderError);
        }
      }
      return null;
    }
    if (root?.setAttribute) {
      root.setAttribute("data-metadata-checker-echarts-vendor", "loaded");
    }

    const echartsRenderer = options.echartsRendererFactory({
      root,
      document: options.document,
      width: options.width,
      height: options.height,
      echarts,
      onNodeClick: (payload) => callbacks.onNodeClick(payload),
      onNodeHover: (payload) => callbacks.onNodeHover(payload),
      onEdgeClick: (payload) => callbacks.onEdgeClick(payload),
      onEdgeHover: (payload) => callbacks.onEdgeHover(payload),
    });
    const renderResult = await echartsRenderer.render(layout);
    if (renderResult?.renderer !== "echarts") {
      echartsController = null;
      if (typeof echartsRenderer.destroy === "function") {
        await echartsRenderer.destroy();
      }
      clearRoot(root);
      return null;
    }
    echartsController = echartsRenderer;
    pixiController = null;
    return renderResult;
  }

  async function renderWithPixi(graph, layout, callbacks) {
    if (!shouldAttemptPixi(options)) {
      return null;
    }
    const { pixi, d3Force3D } = await loadPixiVendorRuntimes(options);
    if (!pixi) {
      return null;
    }

    const pixiRenderer = options.pixiRendererFactory({
      root,
      document: options.document,
      pixi,
      d3Force3D,
      width: options.width,
      height: options.height,
      maxDepth: graph.maxDepth ?? graph.depth ?? layout.depth ?? options.maxDepth,
      maxNodes: options.maxNodes,
      maxEdges: options.maxEdges,
      onNodeClick: (payload) => callbacks.onNodeClick(payload),
      onNodeHover: (payload) => callbacks.onNodeHover(payload),
      onNodeFocus: (payload) => callbacks.onNodeFocus?.(payload),
      onEdgeClick: (payload) => callbacks.onEdgeClick(payload),
      onEdgeHover: (payload) => callbacks.onEdgeHover(payload),
    });
    const renderResult = await pixiRenderer.render(graph);
    if (renderResult?.renderer !== "pixi") {
      pixiController = null;
      clearRoot(root);
      return null;
    }
    pixiController = pixiRenderer;
    echartsController = null;
    return renderResult;
  }

  function getRendererType(usedEcharts) {
    if (usedEcharts) return "echarts";
    return options.renderer === "svg" ? "svg" : "html";
  }

  function emitEventIfWanted(type, payload) {
    if (typeof options.onEvent === "function") {
      options.onEvent({
        ...payload,
        type,
        event: payload?.event ?? "click",
        timestamp: Date.now(),
      });
    }
  }

  function onExpandClick(node) {
    emitExpandEvent(options, extractNodePayloadForExpand(node));
  }

  function toMarkerDepth(graphLayout) {
    return graphLayout.depth ?? graphLayout.maxDepth ?? options.maxDepth;
  }

  function toContractDepth(graph, graphLayout) {
    return parseIntLike(
      graph?.maxDepth,
      parseIntLike(graph?.depth, parseIntLike(graphLayout?.maxDepth, toMarkerDepth(graphLayout || {}))),
    );
  }

  function render(input) {
    if (root) {
      const size = measureContainerSize(root, options.width, options.height);
      options.width = size.width;
      options.height = size.height;
    }
    let normalized = toGraphInput(input);
    if (normalized?.focus_node == null) {
      normalized.focus_node = null;
    }
    let visibleHop = DEFAULT_VISIBLE_HOP;
    const inputSummary = extractInputSummary(input);
    visibleHop = parseIntLike(
      normalized.visible_hop,
      parseIntLike(normalized.visibleHop, inputSummary.visible_hop),
    );
    normalized.visible_hop = visibleHop;

    if (!Array.isArray(normalized.diagnostics) || normalized.diagnostics.length === 0) {
      normalized.diagnostics = [];
    }

    normalized = applyLocalGraphView(normalized);

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
      if (normalized.truncatedReason) {
        layout.truncatedReason = normalized.truncatedReason;
      }
    } catch (error) {
      if (typeof options.onDiagnostic === "function") {
        options.onDiagnostic(error);
      }
      normalized = buildErrorGraph(input);
      layout = layoutGraph(normalized, options);
      layout.truncatedReason = "layout_failed";
    }

    const mermaid = buildMermaidText(layout);
    const echartsOption = buildEChartsGraphOption(layout);

    const callbacks = {
      onNodeClick: (payload) => {
        emitEventIfWanted("graph-node-click", { ...payload });
      },
      onNodeHover: (payload) => {
        emitEventIfWanted("graph-node-hover", { ...payload });
      },
      onNodeFocus: (payload) => {
        emitEventIfWanted("graph-node-focus", { ...payload });
      },
      onEdgeClick: (payload) => {
        emitEventIfWanted("graph-edge-click", { ...payload });
      },
      onEdgeHover: (payload) => {
        emitEventIfWanted("graph-edge-hover", { ...payload });
      },
    };

    return Promise.resolve(renderWithBundledEcharts(layout, callbacks))
      .then((echartsResult) => {
        if (!echartsResult) {
          return null;
        }
        const renderer = "echarts";
        const nodeCount = echartsResult.graph?.nodes?.length ?? layout.nodeCount ?? 0;
        const edgeCount = echartsResult.graph?.edges?.length ?? layout.edgeCount ?? 0;
        setGraphMarkers(root, {
          nodeCount,
          edgeCount,
          focus: echartsResult.focus ?? echartsResult.graph?.focusNodeId ?? layout.focus_node,
          truncated: Boolean(layout.truncated),
          truncatedReason: layout.truncatedReason,
          depth: toContractDepth(normalized, layout),
          renderer,
          visibleHop,
        });
        root.setAttribute("data-metadata-checker-renderer", renderer);
        root.setAttribute("data-metadata-checker-local-graph-renderer", renderer);
        return {
          usedRenderer: renderer,
          renderResult: echartsResult,
        };
      })
      .then((echartsEnvelope) => {
        if (echartsEnvelope) {
          return echartsEnvelope;
        }
        return renderWithPixi(normalized, layout, callbacks).then((pixiResult) => {
          if (!pixiResult) {
            return null;
          }
          const renderer = "pixi";
          const nodeCount = pixiResult.graph?.nodes?.length ?? layout.nodeCount ?? 0;
          const edgeCount = pixiResult.graph?.edges?.length ?? layout.edgeCount ?? 0;
          setGraphMarkers(root, {
            nodeCount,
            edgeCount,
            focus: pixiResult.focus ?? pixiResult.graph?.focusNodeId ?? layout.focus_node,
            truncated: Boolean(layout.truncated),
            truncatedReason: layout.truncatedReason,
            depth: toContractDepth(normalized, layout),
            renderer,
            visibleHop,
          });
          root.setAttribute("data-metadata-checker-renderer", renderer);
          root.setAttribute("data-metadata-checker-local-graph-renderer", renderer);
          return {
            usedRenderer: renderer,
            renderResult: pixiResult,
          };
        });
      })
      .then((primaryEnvelope) => {
        if (primaryEnvelope) {
          return primaryEnvelope;
        }
        return renderWithEcharts(root, layout, options).then((usedEcharts) => ({
          usedRenderer: null,
          usedEcharts,
        }));
      })
      .then((usedEcharts) => {
        if (usedEcharts?.usedRenderer === "echarts") {
          const renderer = "echarts";
          const renderResult = usedEcharts.renderResult;
          if (typeof options.onRender === "function") {
            options.onRender({
              nodes: renderResult.graph?.nodes?.length ?? 0,
              edges: renderResult.graph?.edges?.length ?? 0,
              diagnostics: layout.diagnostics?.length ?? 0,
              truncated: layout.truncated,
              depth: toContractDepth(normalized, layout),
              visibleHop,
            });
          }
          lastRender = {
            graph: {
              ...layout,
              nodes: renderResult.graph?.nodes ?? layout.nodes,
              edges: renderResult.graph?.edges ?? layout.edges,
            },
            mermaid,
            echartsOption,
            renderer,
            truncated: layout.truncated,
            visibleHop,
            visible_hop: visibleHop,
            focus: renderResult.focus ?? renderResult.graph?.focusNodeId ?? layout.focus_node,
            nodeCount: renderResult.graph?.nodes?.length ?? layout.nodeCount ?? 0,
            edgeCount: renderResult.graph?.edges?.length ?? layout.edgeCount ?? 0,
            markerDepth: toContractDepth(normalized, layout),
            depth: toContractDepth(normalized, layout),
          };
          return {
            ...lastRender,
            markerDepth: lastRender.markerDepth,
            markerRenderer: renderer,
          };
        }

        if (usedEcharts?.usedRenderer === "pixi") {
          const renderer = "pixi";
          const renderResult = usedEcharts.renderResult;
          if (typeof options.onRender === "function") {
            options.onRender({
              nodes: renderResult.graph?.nodes?.length ?? 0,
              edges: renderResult.graph?.edges?.length ?? 0,
              diagnostics: layout.diagnostics?.length ?? 0,
              truncated: layout.truncated,
              depth: toContractDepth(normalized, layout),
              visibleHop,
            });
          }
          lastRender = {
            graph: {
              ...layout,
              nodes: renderResult.graph?.nodes ?? layout.nodes,
              edges: renderResult.graph?.edges ?? layout.edges,
            },
            mermaid,
            echartsOption,
            renderer,
            truncated: layout.truncated,
            visibleHop,
            visible_hop: visibleHop,
            focus: renderResult.focus ?? renderResult.graph?.focusNodeId ?? layout.focus_node,
            nodeCount: renderResult.graph?.nodes?.length ?? layout.nodeCount ?? 0,
            edgeCount: renderResult.graph?.edges?.length ?? layout.edgeCount ?? 0,
            markerDepth: toContractDepth(normalized, layout),
            depth: toContractDepth(normalized, layout),
          };
          return {
            ...lastRender,
            markerDepth: lastRender.markerDepth,
            markerRenderer: renderer,
          };
        }

        const renderer = getRendererType(usedEcharts?.usedEcharts);
        if (!usedEcharts?.usedEcharts) {
          domController = renderGraphPanelDOM(root, layout, {
            document: options.document,
            renderer,
            onExpand: (payload) => {
              onExpandClick(payload.node ?? payload);
            },
            onNodeClick: callbacks.onNodeClick,
            onNodeHover: callbacks.onNodeHover,
            onEdgeClick: callbacks.onEdgeClick,
            onEdgeHover: callbacks.onEdgeHover,
          });
          if (typeof options.onRender === "function") {
            options.onRender({
              nodes: layout.nodes?.length ?? 0,
              edges: layout.edges?.length ?? 0,
              diagnostics: layout.diagnostics?.length ?? 0,
              truncated: layout.truncated,
              depth: toContractDepth(normalized, layout),
              visibleHop,
            });
          }
          lastRender = {
            graph: layout,
            mermaid,
            echartsOption,
            renderer,
            truncated: layout.truncated,
            visibleHop,
            visible_hop: visibleHop,
            focus: layout.focus_node,
            nodeCount: layout.nodeCount ?? layout.nodes?.length ?? 0,
            edgeCount: layout.edgeCount ?? layout.edges?.length ?? 0,
            markerDepth: toContractDepth(normalized, layout),
            depth: toContractDepth(normalized, layout),
          };
          return {
            ...lastRender,
            markerDepth: toContractDepth(normalized, layout),
            markerRenderer: renderer,
          };
        }

        domController = null;
        root.setAttribute("data-metadata-checker-graph-open-detail-context", "");
        root.setAttribute("data-metadata-checker-graph-open-detail", "empty");
        root.setAttribute("data-metadata-checker-graph-detail-kind", "empty");
        root.setAttribute("data-metadata-checker-graph-viewport-scale", String(VIEWPORT_SCALE_DEFAULT));

        {
          setGraphMarkers(root, {
            nodeCount: layout.nodeCount ?? 0,
            edgeCount: layout.edgeCount ?? 0,
            focus: layout.focus_node,
            truncated: Boolean(layout.truncated),
            truncatedReason: layout.truncatedReason,
            depth: toContractDepth(normalized, layout),
            renderer,
            visibleHop,
          });
          root.setAttribute("data-metadata-checker-renderer", renderer);
          root.setAttribute("data-metadata-checker-local-graph-renderer", renderer);
        }
        {
          const summary = {
            nodes: layout.nodes?.length ?? 0,
            edges: layout.edges?.length ?? 0,
            diagnostics: layout.diagnostics?.length ?? 0,
            truncated: layout.truncated,
            depth: toContractDepth(normalized, layout),
            visibleHop,
          };
          if (typeof options.onRender === "function") {
            options.onRender(summary);
          }
        }

        lastRender = {
          graph: layout,
          mermaid,
          echartsOption,
          renderer,
          truncated: layout.truncated,
          visibleHop,
          visible_hop: visibleHop,
          focus: layout.focus_node,
          nodeCount: layout.nodeCount ?? layout.nodes?.length ?? 0,
          edgeCount: layout.edgeCount ?? layout.edges?.length ?? 0,
          markerDepth: toContractDepth(normalized, layout),
          depth: toContractDepth(normalized, layout),
        };
        return {
          ...lastRender,
          markerDepth: toContractDepth(normalized, layout),
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
          visibleHop,
        });
        root.setAttribute("data-metadata-checker-local-graph-renderer", fallbackRenderer);
        const fallbackPayload = buildErrorGraph(input);
        lastRender = {
          graph: fallbackPayload,
          mermaid: "",
          echartsOption: null,
          renderer: fallbackRenderer,
          visibleHop,
          visible_hop: visibleHop,
          depth: toMarkerDepth(layout || {}),
        };
        return lastRender;
      });
  }

  function renderError(errorEnvelope) {
    return render(buildErrorGraph(errorEnvelope));
  }

  function clear() {
    if (typeof echartsController?.destroy === "function") {
      void echartsController.destroy();
    }
    if (root && typeof root.replaceChildren === "function") {
      root.replaceChildren();
    }
    setGraphMarkers(root, {
      nodeCount: 0,
      edgeCount: 0,
      focus: "",
      truncated: false,
      depth: 0,
      visibleHop: DEFAULT_VISIBLE_HOP,
      renderer: options.renderer,
    });
    lastRender = null;
    domController = null;
    pixiController = null;
    echartsController = null;
  }

  function setContainer(container) {
    if (!container) {
      return { updated: false };
    }
    root = makeElementHost(container, options);
    const size = measureContainerSize(container, options.width, options.height);
    options.width = size.width;
    options.height = size.height;
    return { updated: true, ...size };
  }

  return {
    setContainer,
    render,
    renderAnalysis: render,
    renderError,
    clear,
    getDomController: () => domController,
    getInteractionState: () => domController?.getInteractionState?.() ?? null,
    getOpenDetailContext: () =>
      echartsController?.getOpenDetailContext?.()
      ?? domController?.getOpenDetailContext?.()
      ?? pixiController?.getOpenDetailContext?.()
      ?? null,
    getVisibleGraphText() {
      if (typeof echartsController?.getVisibleGraphText === "function") {
        return echartsController.getVisibleGraphText();
      }
      if (typeof pixiController?.getVisibleGraphText === "function") {
        return pixiController.getVisibleGraphText();
      }
      if (typeof domController?.getVisibleGraphText === "function") {
        return domController.getVisibleGraphText();
      }
      const graph = lastRender?.graph;
      if (!graph) return "";
      const nodes = Array.isArray(graph.nodes) ? graph.nodes : [];
      const edges = Array.isArray(graph.edges) ? graph.edges : [];
      const lines = [
        `focus: ${graph.focus_node || graph.focusNodeId || ""}`,
        `visible: ${nodes.length}/${nodes.length} nodes, ${edges.length}/${edges.length} edges`,
        "nodes:",
      ];
      for (const node of nodes) {
        lines.push(`- ${node.label || node.id} (${node.kind || "node"})`);
      }
      lines.push("edges:");
      for (const edge of edges) {
        lines.push(`- ${edge.from} -> ${edge.to} : ${edge.summary || edge.label || edge.kind || "edge"}`);
      }
      return lines.join("\n");
    },
    lockCurrentTarget(payload) {
      if (typeof pixiController?.triggerNodeEvent === "function" && payload?.type === "node" && payload?.id) {
        pixiController.triggerNodeEvent(payload.id, "click");
        return payload;
      }
      if (typeof pixiController?.triggerEdgeEvent === "function" && payload?.type === "edge" && payload?.id) {
        pixiController.triggerEdgeEvent(payload.id, "click");
        return payload;
      }
      return domController?.lockCurrentTarget?.(payload) ?? null;
    },
    clearLockedTarget() {
      if (typeof echartsController?.clearLockedTarget === "function") {
        return echartsController.clearLockedTarget();
      }
      if (typeof pixiController?.clearLockedTarget === "function") {
        return pixiController.clearLockedTarget();
      }
      if (!domController) return null;
      return domController.clearLockedTarget();
    },
    setViewportScale(value) {
      if (!domController) return VIEWPORT_SCALE_DEFAULT;
      return domController.setViewportScale?.(value) ?? VIEWPORT_SCALE_DEFAULT;
    },
    resetViewportScale() {
      if (!domController) return VIEWPORT_SCALE_DEFAULT;
      return domController.resetViewportScale?.() ?? VIEWPORT_SCALE_DEFAULT;
    },
    getLastRender() {
      return lastRender;
    },
  };
}

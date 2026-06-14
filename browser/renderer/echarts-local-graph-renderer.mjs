import { buildEChartsGraphOption, buildEchartsIdMap } from "./graph-layout.mjs";

function toStringValue(value, fallback = "") {
  if (value == null) return fallback;
  return String(value);
}

function sanitizeDetailText(value) {
  const text = toStringValue(value);
  if (!text) return text;
  return text.replace(/token|password|secret|cookie|auth|credential|cipherpassport|api_key|apikey/gi, "[sensitive]");
}

function renderInteractionDetail(host, payload) {
  if (!host || !payload) return;
  if (typeof host.replaceChildren === "function") {
    host.replaceChildren();
  } else if (Array.isArray(host.children)) {
    host.children.length = 0;
  }
  const row = host.ownerDocument?.createElement?.("div") ?? { textContent: "", style: {} };
  row.style.maxWidth = "100%";
  row.style.overflow = "hidden";
  row.style.textOverflow = "ellipsis";
  row.style.whiteSpace = "nowrap";
  const lockPrefix = payload.locked ? "Lock · " : "";
  if (payload.type === "edge") {
    const fromTo = [payload.from, payload.to].filter(Boolean).join(" -> ") || payload.edgeId || "";
    const evidenceText = payload.evidenceStatus === "available" && payload.evidence
      ? payload.evidence
      : "EDGE_EVIDENCE_UNAVAILABLE";
    row.textContent = sanitizeDetailText(
      `${lockPrefix}Edge · ${payload.priority || payload.kind || "other"} · ${fromTo} · ${evidenceText}`,
    );
  } else if (payload.kind === "aggregate" || payload.raw?.aggregate) {
    row.textContent = sanitizeDetailText(
      `${lockPrefix}Aggregate · ${payload.shortLabel || payload.raw?.label || "+0"} · ${payload.raw?.aggregateLabel || "context"} · +${payload.raw?.hiddenNodeCount ?? 0} hidden`,
    );
  } else {
    row.textContent = sanitizeDetailText(
      `${lockPrefix}Node · ${payload.shortLabel || payload.nodeId || ""} · ${payload.kind || "node"} · depth ${payload.depth ?? ""} · neighbors ${payload.neighborCount ?? 0}`,
    );
  }
  host.appendChild?.(row);
}

function buildNodePayload(node, layout, state, extras = {}) {
  const neighbors = new Set();
  for (const edge of layout.edges || []) {
    if (edge.from === node.id) neighbors.add(edge.to);
    if (edge.to === node.id) neighbors.add(edge.from);
  }
  return {
    type: "node",
    nodeId: node.id,
    shortLabel: node.visualLabel || node.label || node.id,
    kind: node.kind || "node",
    depth: node.depth ?? node.metadata?.depth ?? "",
    neighborCount: neighbors.size,
    locked: Boolean(state.lockedTarget?.id === node.id && state.lockedTarget?.type === "node"),
    raw: node,
    ...extras,
  };
}

function buildEdgePayload(edge, layout, state, extras = {}) {
  return {
    type: "edge",
    edgeId: edge.id || `${edge.from}->${edge.to}`,
    from: edge.from,
    to: edge.to,
    priority: edge.priority || "other",
    kind: edge.kind || "edge",
    evidenceStatus: edge.evidence_status || (edge.evidence == null ? "unavailable" : "available"),
    evidence: edge.evidence,
    locked: Boolean(state.lockedTarget?.id === (edge.id || `${edge.from}->${edge.to}`) && state.lockedTarget?.type === "edge"),
    raw: edge,
    ...extras,
  };
}

function formatVisibleGraphText(state, layout) {
  const focus = layout.focus_node || layout.focusNodeId || "";
  const nodes = Array.isArray(layout.nodes) ? layout.nodes : [];
  const edges = Array.isArray(layout.edges) ? layout.edges : [];
  const lines = [
    `focus: ${focus}`,
    `visible: ${nodes.length}/${nodes.length} nodes, ${edges.length}/${edges.length} edges`,
    state.lockedTarget?.id ? `locked: ${state.lockedTarget.type}:${state.lockedTarget.id}` : "",
    "nodes:",
  ].filter(Boolean);
  for (const node of nodes) {
    lines.push(`- ${node.label || node.id} (${node.kind || "node"})`);
  }
  lines.push("edges:");
  for (const edge of edges) {
    lines.push(`- ${edge.from} -> ${edge.to} : ${edge.summary || edge.label || edge.kind || "edge"}`);
  }
  return lines.join("\n");
}

const VIEWPORT_SCALE_DEFAULT = 1;
const VIEWPORT_SCALE_PRECISION = 2;

function formatTargetString(target) {
  if (!target?.id) {
    return "none";
  }
  const type = toStringValue(target.type, "node").trim() || "node";
  return `${type}:${toStringValue(target.id)}`;
}

function countHighlight(layout, target) {
  const nodes = new Set();
  const edges = new Set();
  if (!target?.id) {
    const focusId = layout?.focus_node || layout?.focusNodeId;
    if (focusId) {
      nodes.add(focusId);
    }
    return { nodeCount: nodes.size, edgeCount: edges.size };
  }
  if (target.type === "edge") {
    edges.add(target.id);
    const edge = (layout?.edges || []).find(
      (item) => (item.id || `${item.from}->${item.to}`) === target.id,
    );
    if (edge?.from) nodes.add(edge.from);
    if (edge?.to) nodes.add(edge.to);
    return { nodeCount: nodes.size, edgeCount: edges.size };
  }
  nodes.add(target.id);
  for (const edge of layout?.edges || []) {
    if (edge.from === target.id || edge.to === target.id) {
      edges.add(edge.id || `${edge.from}->${edge.to}`);
      if (edge.from) nodes.add(edge.from);
      if (edge.to) nodes.add(edge.to);
    }
  }
  return { nodeCount: nodes.size, edgeCount: edges.size };
}

function writeInteractionMarkers(root, state, layout) {
  if (!root?.setAttribute) {
    return;
  }
  const focusId = layout?.focus_node || layout?.focusNodeId || "";
  const hoverTarget = state.hoveredTarget;
  const lockedTarget = state.lockedTarget;
  const detailTarget = lockedTarget || hoverTarget;
  const highlight = countHighlight(layout, detailTarget || (focusId ? { type: "focus", id: focusId } : null));
  const viewportTarget = state.viewportTarget
    || (focusId ? { type: "focus", id: focusId } : null);
  root.setAttribute("data-metadata-checker-graph-hover-target", formatTargetString(hoverTarget));
  root.setAttribute("data-metadata-checker-graph-locked-target", formatTargetString(lockedTarget));
  root.setAttribute(
    "data-metadata-checker-graph-detail-kind",
    lockedTarget?.type || hoverTarget?.type || (focusId ? "node" : "empty"),
  );
  root.setAttribute("data-metadata-checker-graph-highlight-node-count", String(highlight.nodeCount));
  root.setAttribute("data-metadata-checker-graph-highlight-edge-count", String(highlight.edgeCount));
  root.setAttribute(
    "data-metadata-checker-graph-viewport-scale",
    String(Number((state.viewportScale ?? VIEWPORT_SCALE_DEFAULT).toFixed(VIEWPORT_SCALE_PRECISION))),
  );
  root.setAttribute("data-metadata-checker-graph-viewport-target", formatTargetString(viewportTarget));
  root.setAttribute("data-metadata-checker-graph-density-profile", "compact");
}

function getCurrentDetailPayload(state, layout) {
  if (state.lockedTarget?.id) {
    if (state.lockedTarget.type === "edge") {
      const edge = (layout.edges || []).find((item) => (item.id || `${item.from}->${item.to}`) === state.lockedTarget.id)
        || (layout.edges || []).find((item) => item.from && item.to && `${item.from}->${item.to}` === state.lockedTarget.id);
      if (edge) return buildEdgePayload(edge, layout, state, { locked: true });
    } else {
      const node = (layout.nodes || []).find((item) => item.id === state.lockedTarget.id);
      if (node) return buildNodePayload(node, layout, state, { locked: true });
    }
  }
  if (state.hoveredTarget?.id) {
    if (state.hoveredTarget.type === "edge") {
      const edge = (layout.edges || []).find((item) => (item.id || `${item.from}->${item.to}`) === state.hoveredTarget.id);
      if (edge) return buildEdgePayload(edge, layout, state);
    } else {
      const node = (layout.nodes || []).find((item) => item.id === state.hoveredTarget.id);
      if (node) return buildNodePayload(node, layout, state);
    }
  }
  const focusId = layout.focus_node || layout.focusNodeId;
  const focusNode = (layout.nodes || []).find((item) => item.id === focusId);
  if (focusNode) return buildNodePayload(focusNode, layout, state);
  return null;
}

/**
 * 在扩展/content script 上下文内直接初始化 ECharts（支持 shadow root 挂载点）。
 */
export function createEchartsLocalGraphRenderer(rawOptions = {}) {
  const options = {
    width: rawOptions.width ?? 266,
    height: rawOptions.height ?? 188,
    document: rawOptions.document ?? globalThis.document,
    echarts: rawOptions.echarts ?? null,
    onNodeClick: rawOptions.onNodeClick,
    onNodeHover: rawOptions.onNodeHover,
    onEdgeClick: rawOptions.onEdgeClick,
    onEdgeHover: rawOptions.onEdgeHover,
  };

  const state = {
    mountHost: null,
    detailHost: null,
    chart: null,
    layout: null,
    echartsIdToLayoutId: null,
    lockedTarget: null,
    hoveredTarget: null,
    viewportScale: VIEWPORT_SCALE_DEFAULT,
    viewportTarget: null,
  };

  function resolveLayoutNodeId(echartsNodeId) {
    if (!echartsNodeId) return "";
    return state.echartsIdToLayoutId?.get(echartsNodeId) || echartsNodeId;
  }

  function findLayoutEdge(detail) {
    const edges = state.layout?.edges || [];
    if (detail.edgeId) {
      return edges.find((item) => (item.id || `${item.from}->${item.to}`) === detail.edgeId);
    }
    const from = resolveLayoutNodeId(detail.from || detail.source);
    const to = resolveLayoutNodeId(detail.to || detail.target);
    if (!from || !to) return null;
    return edges.find((item) => item.from === from && item.to === to);
  }

  function ensureDetailHost(root) {
    if (state.detailHost?.parentNode === root) {
      return state.detailHost;
    }
    const detailHost = options.document?.createElement?.("section");
    if (!detailHost) return null;
    detailHost.className = "metadata-checker-graph-detail-content metadata-checker-echarts-detail";
    detailHost.setAttribute?.("data-metadata-checker-pixi-detail", "mounted");
    detailHost.style.position = "absolute";
    detailHost.style.left = "8px";
    detailHost.style.right = "8px";
    detailHost.style.bottom = "6px";
    detailHost.style.maxHeight = "24px";
    detailHost.style.overflow = "hidden";
    detailHost.style.fontSize = "10px";
    detailHost.style.lineHeight = "16px";
    detailHost.style.color = "#cbd5e1";
    detailHost.style.background = "rgba(15, 23, 42, 0.62)";
    detailHost.style.border = "1px solid rgba(148, 163, 184, 0.22)";
    detailHost.style.borderRadius = "6px";
    detailHost.style.padding = "2px 6px";
    detailHost.style.pointerEvents = "none";
    detailHost.style.boxSizing = "border-box";
    root.appendChild?.(detailHost);
    state.detailHost = detailHost;
    return detailHost;
  }

  function handleChartInteraction(params, eventType) {
    if (!params || !state.layout) return;
    if (params.dataType === "edge") {
      const edge = findLayoutEdge({
        source: params.data?.source,
        target: params.data?.target,
      });
      if (!edge) return;
      const payload = buildEdgePayload(edge, state.layout, state, { event: eventType });
      if (eventType === "click") {
        state.lockedTarget = { type: "edge", id: payload.edgeId };
        state.hoveredTarget = null;
        state.viewportTarget = { type: "edge", id: payload.edgeId };
        options.onEdgeClick?.(payload);
      } else if (eventType === "hover") {
        state.hoveredTarget = { type: "edge", id: payload.edgeId };
        options.onEdgeHover?.(payload);
      } else if (eventType === "hoverEnd") {
        if (state.hoveredTarget?.id === payload.edgeId) state.hoveredTarget = null;
        options.onEdgeHover?.({ ...payload, event: "hoverEnd" });
      }
      renderInteractionDetail(state.detailHost, getCurrentDetailPayload(state, state.layout));
      writeInteractionMarkers(rawOptions.root, state, state.layout);
      return;
    }

    const node = (state.layout.nodes || []).find((item) => item.id === resolveLayoutNodeId(params.data?.id || params.name));
    if (!node) return;
    const payload = buildNodePayload(node, state.layout, state, { event: eventType });
    if (eventType === "click") {
      state.lockedTarget = { type: "node", id: node.id };
      state.hoveredTarget = null;
      state.viewportTarget = { type: "node", id: node.id };
      options.onNodeClick?.(payload);
    } else if (eventType === "hover") {
      state.hoveredTarget = { type: "node", id: node.id };
      options.onNodeHover?.(payload);
    } else if (eventType === "hoverEnd") {
      if (state.hoveredTarget?.id === node.id) state.hoveredTarget = null;
      options.onNodeHover?.({ ...payload, event: "hoverEnd" });
    }
    renderInteractionDetail(state.detailHost, getCurrentDetailPayload(state, state.layout));
    writeInteractionMarkers(rawOptions.root, state, state.layout);
  }

  function bindChartInteractions(chart) {
    if (!chart || typeof chart.on !== "function") {
      return;
    }
    chart.off("click");
    chart.off("mouseover");
    chart.off("mouseout");
    chart.on("click", (params) => handleChartInteraction(params, "click"));
    chart.on("mouseover", (params) => handleChartInteraction(params, "hover"));
    chart.on("mouseout", (params) => handleChartInteraction(params, "hoverEnd"));
    chart.on("graphRoam", (params) => {
      const nextScale = Number(params?.zoom);
      if (Number.isFinite(nextScale) && nextScale > 0) {
        state.viewportScale = nextScale;
      }
      writeInteractionMarkers(rawOptions.root, state, state.layout);
    });
  }

  function destroyChart() {
    if (state.chart && typeof state.chart.dispose === "function") {
      state.chart.dispose();
    }
    state.chart = null;
  }

  return {
    async render(layout) {
      const root = rawOptions.root;
      const echartsRuntime = options.echarts;
      if (!root || !echartsRuntime || typeof echartsRuntime.init !== "function") {
        return { renderer: "html", ok: false };
      }

      if (typeof root.replaceChildren === "function") {
        root.replaceChildren();
      }
      destroyChart();

      const mountHost = options.document?.createElement?.("div");
      if (!mountHost) {
        return { renderer: "html", ok: false };
      }
      mountHost.className = "metadata-checker-echarts-mount-host";
      mountHost.style.width = "100%";
      mountHost.style.minHeight = "0";
      mountHost.style.position = "relative";
      mountHost.style.overflow = "hidden";
      mountHost.setAttribute("data-metadata-checker-graph-surface", "echarts");
      root.appendChild(mountHost);

      const width = Math.max(120, Number(mountHost.clientWidth) || Number(options.width) || 266);
      const height = Math.max(96, Number(mountHost.clientHeight) || Number(options.height) || 188);
      mountHost.style.height = `${height}px`;

      state.mountHost = mountHost;
      state.layout = layout;
      state.echartsIdToLayoutId = buildEchartsIdMap(layout);
      state.lockedTarget = null;
      state.hoveredTarget = null;
      state.viewportScale = VIEWPORT_SCALE_DEFAULT;
      state.viewportTarget = layout.focus_node || layout.focusNodeId
        ? { type: "focus", id: layout.focus_node || layout.focusNodeId }
        : null;

      ensureDetailHost(mountHost);
      renderInteractionDetail(state.detailHost, getCurrentDetailPayload(state, layout));
      writeInteractionMarkers(root, state, layout);

      let chart;
      try {
        chart = echartsRuntime.init(mountHost, null, {
          renderer: "canvas",
          width,
          height,
        });
        chart.setOption(buildEChartsGraphOption(layout, { width, height }), true);
        if (typeof chart.resize === "function") {
          chart.resize({ width, height });
        }
        bindChartInteractions(chart);
      } catch (error) {
        destroyChart();
        root.setAttribute?.("data-metadata-checker-echarts-canvas", "init-failed");
        root.setAttribute?.("data-metadata-checker-echarts-init-error", error?.message || "echarts init failed");
        return { renderer: "html", ok: false };
      }
      state.chart = chart;

      const canvas = typeof mountHost.querySelector === "function"
        ? mountHost.querySelector("canvas")
        : null;
      root.setAttribute?.("data-metadata-checker-echarts-canvas", canvas ? "mounted" : "missing");
      root.setAttribute?.("data-metadata-checker-local-graph-renderer", "echarts");
      root.setAttribute?.("data-metadata-checker-graph-renderer", "echarts");
      root.setAttribute?.("data-metadata-checker-echarts-source", "extension-bundle");
      return {
        renderer: "echarts",
        ok: true,
        graph: layout,
        focus: layout.focus_node || layout.focusNodeId,
        echartsSource: "extension-bundle",
      };
    },
    getVisibleGraphText() {
      return state.layout ? formatVisibleGraphText(state, state.layout) : "";
    },
    getOpenDetailContext() {
      const payload = state.layout ? getCurrentDetailPayload(state, state.layout) : null;
      if (!payload) {
        return { kind: "empty", context: "", id: "", payload: null };
      }
      return {
        kind: payload.type === "edge" ? "edge" : "node",
        context: payload.shortLabel || payload.nodeId || payload.edgeId || "",
        id: payload.nodeId || payload.edgeId || "",
        payload,
      };
    },
    clearLockedTarget() {
      state.lockedTarget = null;
      const focusId = state.layout?.focus_node || state.layout?.focusNodeId;
      state.viewportTarget = focusId ? { type: "focus", id: focusId } : null;
      if (state.layout) {
        renderInteractionDetail(state.detailHost, getCurrentDetailPayload(state, state.layout));
        writeInteractionMarkers(rawOptions.root, state, state.layout);
      }
    },
    destroy() {
      destroyChart();
      state.mountHost = null;
      state.detailHost = null;
      state.layout = null;
    },
  };
}

export const createEchartsBridgeRenderer = createEchartsLocalGraphRenderer;

import { computeLocalGraphLayout, isTranslucentDepth } from "./force-local-graph-layout.mjs";

const DEFAULT_RENDERER_OPTIONS = {
  width: 320,
  height: 220,
  nodeRadius: 4.2,
  oneHopOpacity: 0.62,
  twoHopOpacity: 0.1,
  oneHopEdgeOpacity: 0.7,
  twoHopEdgeOpacity: 0.08,
  hoverDimAlpha: 0.18,
  backgroundColor: 0x0f172a,
  maxRenderedNodes: 36,
  maxRenderedEdges: 90,
  layoutEngine: null,
  d3Force3D: null,
  pixiRuntimeLoader: null,
  pixi: null,
  forceCanvas: null,
  onNodeClick: null,
  onNodeHover: null,
  onNodeFocus: null,
  onEdgeClick: null,
  onEdgeHover: null,
};

const EDGE_KIND_STYLES = {
  reads: 0x79b8ff,
  writes: 0x7ddf7a,
  condition: 0xbf87ff,
  action: 0xf6b26b,
  dataflow: 0x6fd6d5,
  warning: 0xff8a80,
};

const EDGE_PRIORITY_STYLES = {
  filter: { color: 0xfbbf24, width: 1.9, opacity: 0.9, rank: 5 },
  condition: { color: 0xc084fc, width: 1.75, opacity: 0.84, rank: 4 },
  visibility: { color: 0xa78bfa, width: 1.5, opacity: 0.72, rank: 3 },
  source: { color: 0x38bdf8, width: 1.35, opacity: 0.62, rank: 2 },
  action: { color: 0x34d399, width: 1.15, opacity: 0.48, rank: 1 },
  other: { color: 0x64748b, width: 0.75, opacity: 0.18, rank: 0 },
};

const GRAPH_DENSITY_PROFILES = {
  compact: { maxRenderedNodes: 36, maxRenderedEdges: 90 },
  balanced: { maxRenderedNodes: 48, maxRenderedEdges: 120 },
};

const VIEWPORT_SCALE_MIN = 0.75;
const VIEWPORT_SCALE_MAX = 2.4;
const VIEWPORT_SCALE_DEFAULT = 1;
const VIEWPORT_SCALE_PRECISION = 2;
const VIEWPORT_EVENT_ZOOM_STEP = 0.12;
const VIEWPORT_EVENT_DRAG_SCALE = 1;

const TARGET_KIND_NODE = "node";
const TARGET_KIND_EDGE = "edge";
const TARGET_KIND_AGGREGATE = "aggregate";
const NODE_TIER_FILTER = 4;
const NODE_TIER_VISIBILITY = 3;
const NODE_TIER_SOURCE = 2;
const NODE_TIER_DOWNSTREAM = 1;
const NODE_TIER_OTHER = 0;
const LOCKED_EDGE_COLOR = 0xf59e0b;
const TARGET_KIND_FOCUS = "focus";
const TARGET_KIND_NONE = "none";

function toStringValue(value, fallback = "") {
  if (value == null) return fallback;
  return String(value);
}

function toPositiveInt(value, fallback) {
  const parsed = Number.parseInt(value, 10);
  if (!Number.isFinite(parsed) || parsed < 1) {
    return fallback;
  }
  return parsed;
}

function normalizeDensityProfile(value) {
  const normalized = toStringValue(value, "compact").trim().toLowerCase();
  return GRAPH_DENSITY_PROFILES[normalized] ? normalized : "compact";
}

function formatTargetString(target) {
  if (!target || !target.id) return "none";
  const type = toStringValue(target.type || TARGET_KIND_NODE).trim() || TARGET_KIND_NODE;
  return `${type}:${toStringValue(target.id)}`;
}

function normalizeTargetType(type) {
  const normalized = toStringValue(type, TARGET_KIND_NODE).trim().toLowerCase();
  if (normalized === TARGET_KIND_EDGE || normalized === TARGET_KIND_AGGREGATE || normalized === TARGET_KIND_FOCUS) {
    return normalized;
  }
  return TARGET_KIND_NODE;
}

function makeTarget(type, id) {
  const targetType = normalizeTargetType(type);
  if (id == null || id === "") return null;
  return { type: targetType, id: String(id) };
}

function isSameTarget(left, right) {
  if (!left || !right) return false;
  return left.type === right.type && toStringValue(left.id) === toStringValue(right.id);
}

function clamp(value, min, max) {
  return Math.max(min, Math.min(max, value));
}

function hasToken(value, tokens) {
  const lowered = toStringValue(value).trim().toLowerCase();
  return tokens.some((token) => lowered.includes(token));
}

function inferPriorityFromText(value) {
  if (hasToken(value, ["filter"])) return "filter";
  if (hasToken(value, ["condition"])) return "condition";
  if (hasToken(value, ["visibility", "display"])) return "visibility";
  if (hasToken(value, ["source", "value", "read", "reads", "write", "writes", "dataflow"])) return "source";
  if (hasToken(value, ["action"])) return "action";
  return "other";
}

function normalizeKind(value) {
  return toStringValue(value, "other").toLowerCase();
}

function getEdgeStyleColor(kind) {
  return EDGE_KIND_STYLES[normalizeKind(kind)] ?? 0x9aa4b2;
}

function normalizePriority(value) {
  if (value && typeof value === "object") {
    const explicit = normalizePriority(value?.priority);
    if (explicit !== "other") return explicit;
    const fromKind = inferPriorityFromText(value?.kind);
    if (fromKind !== "other") return fromKind;
    const fromSummary = inferPriorityFromText(value?.summary);
    if (fromSummary !== "other") return fromSummary;
    if (toStringValue(value?.evidence_status).toLowerCase() === "available") {
      return "source";
    }
    return "other";
  }
  const explicit = toStringValue(value, "other").toLowerCase();
  return EDGE_PRIORITY_STYLES[explicit] ? explicit : "other";
}

function getEdgePriorityStyle(edge) {
  return EDGE_PRIORITY_STYLES[normalizePriority(edge)] ?? EDGE_PRIORITY_STYLES.other;
}

function nodeVisualOpacity(nodeDepth, isFocus, isNeighborFocus) {
  if (isFocus) return 0.92;
  if (isNeighborFocus) return 0.7;
  return isTranslucentDepth(nodeDepth) ? 0.22 : 0.72;
}

function nodeVisualRadius(baseRadius, nodeDepth, isFocus, isNeighborFocus, hasPriorityConnection) {
  if (isFocus) {
    return baseRadius * 1.18;
  }
  if (isNeighborFocus) {
    return baseRadius * (hasPriorityConnection ? 1.03 : 1.0);
  }
  if (isTranslucentDepth(nodeDepth)) {
    return baseRadius * 0.85;
  }
  return baseRadius;
}

function isFocusNeighbor(node, focusNodeId, edgeById) {
  return Array.from(edgeById.values()).some((edge) =>
    (edge.from === focusNodeId && edge.to === node.id) ||
    (edge.to === focusNodeId && edge.from === node.id),
  );
}

function edgeVisualOpacity(edge, focusNodeId) {
  const translucent = isTranslucentDepth(edge.fromDepth) || isTranslucentDepth(edge.toDepth);
  if (translucent) {
    return 0.18;
  }
  const style = getEdgePriorityStyle(edge);
  const isFocusEdge = edge.from === focusNodeId || edge.to === focusNodeId;
  if (!isFocusEdge) {
    return 0.38;
  }
  return style.opacity;
}

function edgeVisualColor(edge, focusNodeId) {
  if (edge.from === focusNodeId || edge.to === focusNodeId) {
    return getEdgePriorityStyle(edge).color;
  }
  return getEdgePriorityStyle(edge).color;
}

function hasEvidence(value) {
  if (value == null) return false;
  if (typeof value === "string" && value.trim() === "") return false;
  return true;
}

function sanitizeDetailText(value) {
  const text = toStringValue(value);
  if (/token|password|secret|cookie|auth|credential|api_key|apikey|cipherpassport/i.test(text)) {
    return "[sensitive]";
  }
  return text;
}

function normalizeCallbacks(rawOptions = {}) {
  return {
    onNodeClick: typeof rawOptions.onNodeClick === "function" ? rawOptions.onNodeClick : null,
    onNodeHover: typeof rawOptions.onNodeHover === "function" ? rawOptions.onNodeHover : null,
    onNodeFocus: typeof rawOptions.onNodeFocus === "function" ? rawOptions.onNodeFocus : null,
    onEdgeClick: typeof rawOptions.onEdgeClick === "function" ? rawOptions.onEdgeClick : null,
    onEdgeHover: typeof rawOptions.onEdgeHover === "function" ? rawOptions.onEdgeHover : null,
  };
}

function normalizeRendererOptions(rawOptions = {}) {
  const densityProfile = normalizeDensityProfile(rawOptions.densityProfile ?? rawOptions.density_profile);
  const maxRenderedNodes = toPositiveInt(
    rawOptions.maxRenderedNodes,
    GRAPH_DENSITY_PROFILES[densityProfile].maxRenderedNodes,
  );
  const maxRenderedEdges = toPositiveInt(
    rawOptions.maxRenderedEdges,
    GRAPH_DENSITY_PROFILES[densityProfile].maxRenderedEdges,
  );
  return {
    ...DEFAULT_RENDERER_OPTIONS,
    ...rawOptions,
    densityProfile,
    maxRenderedNodes,
    maxRenderedEdges,
    callbacks: normalizeCallbacks(rawOptions),
  };
}

function createFakeHostElement(tagName) {
  return {
    tagName,
    children: [],
    style: {},
    attributes: {},
    listeners: {},
    setAttribute(name, value) {
      this.attributes[String(name)] = String(value);
    },
    getAttribute(name) {
      return this.attributes[name] ?? null;
    },
    appendChild(child) {
      this.children.push(child);
      if (child) {
        child.parentNode = this;
      }
    },
    removeChild(child) {
      const index = this.children.indexOf(child);
      if (index >= 0) {
        this.children.splice(index, 1);
      }
    },
    remove() {
      if (this.parentNode?.removeChild) {
        this.parentNode.removeChild(this);
      }
    },
  };
}

function ensureRootContainer(options) {
  const document = options.document || globalThis.document;
  if (options.root) return options.root;
  if (document && typeof document.createElement === "function") {
    return document.createElement("div");
  }
  return createFakeHostElement("div");
}

function createRendererElement(options, tagName) {
  const document = options.document || globalThis.document;
  if (document && typeof document.createElement === "function") {
    return document.createElement(tagName);
  }
  return createFakeHostElement(tagName);
}

function clearHostElement(element) {
  if (!element) return;
  if (typeof element.replaceChildren === "function") {
    element.replaceChildren();
  } else if (Array.isArray(element.children)) {
    element.children = [];
  }
}

function createNodeLookup(nodes) {
  const byId = new Map();
  for (const node of nodes) {
    if (node?.id) {
      byId.set(node.id, node);
    }
  }
  return byId;
}

function createEdgeLookup(edges) {
  const byId = new Map();
  const sameKeyCount = new Map();
  for (const edge of edges) {
    const fallbackId = `${edge?.from}->${edge?.to}`;
    const nextKey = edge?.id || fallbackId;
    const serial = nextKey === fallbackId ? (sameKeyCount.get(nextKey) || 0) + 1 : 0;
    const edgeId = nextKey === fallbackId ? `${nextKey}#${serial - 1}` : String(nextKey);
    sameKeyCount.set(nextKey, serial);
    edge.id = edgeId;
    byId.set(edgeId, edge);
  }
  return byId;
}

function buildAdjacencyMap(nodes, edges) {
  const map = new Map();
  for (const node of nodes) {
    if (node?.id) {
      map.set(node.id, new Set());
    }
  }
  for (const edge of edges) {
    if (!edge?.from || !edge?.to) continue;
    if (!map.has(edge.from)) {
      map.set(edge.from, new Set());
    }
    if (!map.has(edge.to)) {
      map.set(edge.to, new Set());
    }
    map.get(edge.from).add(edge.to);
    map.get(edge.to).add(edge.from);
  }
  return map;
}

function eventTimestamp() {
  return Date.now();
}

function buildNodeInteractionPayload(node, state, extras = {}) {
  const neighbors = state.nodeNeighborsById?.get(node.id) || new Set();
  const nodeType = node.aggregate ? "aggregate" : "node";
  const focusedTarget = state.focusedGraphTarget || null;
  return {
    type: nodeType,
    targetType: nodeType,
    targetId: node.id,
    target: makeTarget(nodeType, node.id),
    nodeId: node.id,
    kind: node.kind,
    label: node.label,
    shortLabel: node.visualLabel || node.label || node.id,
    sourcePath: node.sourcePath || node.source_path || "",
    focusNodeId: state.designerFocusNodeId,
    designerFocusNodeId: state.designerFocusNodeId,
    focusedGraphTarget: focusedTarget,
    depth: node.depth,
    focus: state.designerFocusNodeId === node.id,
    isFocus: state.designerFocusNodeId === node.id,
    neighborCount: neighbors.size,
    relatedPrioritySummary: node.relatedPrioritySummary || "",
    locked: isSameTarget(state.lockedTarget, makeTarget(nodeType, node.id)),
    hovered: isSameTarget(state.hoveredTarget, makeTarget(nodeType, node.id)),
    event: extras.event || "interaction",
    timestamp: eventTimestamp(),
    raw: node,
    ...extras,
  };
}

function buildEdgeInteractionPayload(edge, state, extras = {}) {
  const edgeEvidence = hasEvidence(edge?.evidence)
    ? { evidence: edge.evidence, diagnostics: [] }
    : {
        evidence: null,
        diagnostics: [
          {
            severity: "info",
            code: "EDGE_EVIDENCE_UNAVAILABLE",
            message: "Edge evidence not available",
          },
        ],
      };
  return {
    type: "edge",
    targetType: "edge",
    targetId: edge.id || `${edge.from}->${edge.to}`,
    target: makeTarget("edge", edge.id || `${edge.from}->${edge.to}`),
    edgeId: edge.id || `${edge.from}->${edge.to}`,
    from: edge.from,
    to: edge.to,
    kind: edge.kind,
    label: edge.label,
    priority: normalizePriority(edge),
    summary: edge.summary || edge.label || edge.kind || "",
    evidenceStatus: edge.evidence_status || (hasEvidence(edge?.evidence) ? "available" : "unavailable"),
    focusNodeId: state.designerFocusNodeId,
    designerFocusNodeId: state.designerFocusNodeId,
    focusedGraphTarget: state.focusedGraphTarget,
    locked: isSameTarget(state.lockedTarget, makeTarget("edge", edge.id || `${edge.from}->${edge.to}`)),
    hovered: isSameTarget(state.hoveredTarget, makeTarget("edge", edge.id || `${edge.from}->${edge.to}`)),
    fromDepth: edge.fromDepth,
    toDepth: edge.toDepth,
    direction: edge.direction,
    event: extras.event || "interaction",
    timestamp: eventTimestamp(),
    ...edgeEvidence,
    raw: edge,
    ...extras,
  };
}

function renderInteractionDetail(host, payload, options) {
  if (!host || !payload) return;
  clearHostElement(host);
  const row = createRendererElement(options, "div");
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

function createPixiInteractionPanel(root, graph, state, options) {
  const detailHost = createRendererElement(options, "section");
  detailHost.className = "metadata-checker-graph-detail-content";
  detailHost.setAttribute?.("data-metadata-checker-pixi-detail", "mounted");
  detailHost.style.position = "absolute";
  detailHost.style.left = "8px";
  detailHost.style.right = "8px";
  detailHost.style.bottom = "6px";
  detailHost.style.maxHeight = "24px";
  detailHost.style.overflowY = "hidden";
  detailHost.style.overflowX = "hidden";
  detailHost.style.fontSize = "10px";
  detailHost.style.lineHeight = "16px";
  detailHost.style.color = "#cbd5e1";
  detailHost.style.background = "rgba(15, 23, 42, 0.62)";
  detailHost.style.border = "1px solid rgba(148, 163, 184, 0.22)";
  detailHost.style.borderRadius = "6px";
  detailHost.style.padding = "2px 6px";
  detailHost.style.backdropFilter = "blur(6px)";
  detailHost.style.maxWidth = "100%";
  detailHost.style.boxSizing = "border-box";
  detailHost.style.overflowWrap = "anywhere";
  detailHost.style.wordBreak = "break-word";
  detailHost.style.pointerEvents = "none";

  const lists = createRendererElement(options, "div");
  lists.className = "metadata-checker-pixi-interaction-index";
  lists.style.display = "none";

  const nodeList = createRendererElement(options, "ul");
  const realIndexNodes = graph.nodes.filter((node) => !node?.aggregate).slice(0, 24);
  const aggregateIndexNodes = graph.nodes.filter((node) => node?.aggregate).slice(0, 4);
  for (const node of [...realIndexNodes, ...aggregateIndexNodes]) {
    const row = createRendererElement(options, "li");
    const targetKind = node.aggregate ? TARGET_KIND_AGGREGATE : TARGET_KIND_NODE;
    row.className = node.aggregate
      ? "graph-aggregate metadata-checker-graph-aggregate"
      : "graph-node metadata-checker-graph-node";
    row.setAttribute?.("data-node-id", sanitizeDetailText(node.id));
    if (node.aggregate) {
      row.setAttribute?.("data-aggregate-id", sanitizeDetailText(node.id));
    }
    row.textContent = `${sanitizeDetailText(node.label || node.id)} (${sanitizeDetailText(node.kind || "node")})`;
    row.addEventListener?.("click", () => {
      state.dispatchTargetEvent(makeTarget(targetKind, node.id), "click");
    });
    row.addEventListener?.("mouseover", () => {
      state.dispatchTargetEvent(makeTarget(targetKind, node.id), "hover");
    });
    row.addEventListener?.("mouseout", () => {
      state.dispatchTargetEvent(makeTarget(targetKind, node.id), "hoverEnd");
    });
    nodeList.appendChild?.(row);
  }

  const edgeList = createRendererElement(options, "ul");
  for (const edge of graph.edges.slice(0, 24)) {
    const row = createRendererElement(options, "li");
    row.className = "graph-edge metadata-checker-graph-edge";
    row.setAttribute?.("data-edge-id", sanitizeDetailText(edge.id));
    const evidenceStatus = edge.evidence_status || (edge.evidence ? "available" : "unavailable");
    row.textContent = `${sanitizeDetailText(edge.from)} → ${sanitizeDetailText(edge.to)} : ${sanitizeDetailText(edge.summary || edge.label || edge.kind)} [${sanitizeDetailText(edge.priority || "other")}/${sanitizeDetailText(evidenceStatus)}]`;
    row.addEventListener?.("click", () => {
      state.dispatchTargetEvent(makeTarget(TARGET_KIND_EDGE, edge.id), "click");
    });
    row.addEventListener?.("mouseover", () => {
      state.dispatchTargetEvent(makeTarget(TARGET_KIND_EDGE, edge.id), "hover");
    });
    row.addEventListener?.("mouseout", () => {
      state.dispatchTargetEvent(makeTarget(TARGET_KIND_EDGE, edge.id), "hoverEnd");
    });
    edgeList.appendChild?.(row);
  }

  lists.appendChild?.(nodeList);
  lists.appendChild?.(edgeList);
  root.appendChild?.(detailHost);
  root.appendChild?.(lists);
  state.detailHost = detailHost;
  const detailTarget = state.hoveredTarget || state.lockedTarget || makeTarget(TARGET_KIND_FOCUS, state.designerFocusNodeId);
  if (detailTarget) {
    const payload = getTargetPayload(state, detailTarget);
    if (payload) {
      const nextPayload = {
        ...payload,
        event: "focus",
      };
      renderInteractionDetail(detailHost, nextPayload, options);
    }
  }
}

function getTargetPayload(state, target) {
  if (!target || !target.id) return null;
  const targetType = normalizeTargetType(target.type);
  if (targetType === TARGET_KIND_EDGE) {
    const edge = state.viewEdgeById?.get(target.id) || state.edgeById?.get(target.id);
    if (!edge) return null;
    return buildEdgeInteractionPayload(edge, state, { event: "focus" });
  }
  const node = state.viewNodeById?.get(target.id) || state.nodeById?.get(target.id);
  if (!node) return null;
  return buildNodeInteractionPayload(node, state, { event: "focus" });
}

function getCurrentDetailPayload(state) {
  const activeTarget = state.lockedTarget || state.hoveredTarget || makeTarget(TARGET_KIND_FOCUS, state.designerFocusNodeId);
  return getTargetPayload(state, activeTarget);
}

function priorityToSelectionTier(priority) {
  const normalized = normalizePriority(priority);
  if (normalized === "filter" || normalized === "condition") return NODE_TIER_FILTER;
  if (normalized === "visibility") return NODE_TIER_VISIBILITY;
  if (normalized === "source") return NODE_TIER_SOURCE;
  return NODE_TIER_OTHER;
}

function isComponentKind(node) {
  const kind = normalizeKind(node?.kind);
  if (kind === "component" || kind === "comp") return true;
  return toStringValue(node?.id).startsWith("comp:");
}

function nodeSelectionTier(node, edges, focusNodeId) {
  if (!node || node.id === focusNodeId) return 99;
  if (node.aggregate) return NODE_TIER_OTHER;
  let maxTier = NODE_TIER_OTHER;
  for (const edge of edges) {
    if (edge?.from !== node.id && edge?.to !== node.id) continue;
    maxTier = Math.max(maxTier, priorityToSelectionTier(edge));
  }
  if (maxTier === NODE_TIER_OTHER && isComponentKind(node)) {
    return NODE_TIER_DOWNSTREAM;
  }
  return maxTier;
}

function formatVisibleGraphText(state) {
  const graph = state.lastRenderGraph || {};
  const viewGraph = state.lastViewGraph || graph.viewGraph || {};
  const nodes = Array.isArray(viewGraph.nodes) ? viewGraph.nodes : [];
  const edges = Array.isArray(viewGraph.edges) ? viewGraph.edges : [];
  const fullNodeCount = graph.nodes?.length ?? viewGraph.visibleRealNodes?.length ?? nodes.length;
  const fullEdgeCount = graph.edges?.length ?? viewGraph.visibleRealEdges?.length ?? edges.length;
  const visibleNodeCount = (viewGraph.visibleNodeCount ?? 0) + (viewGraph.aggregateNodeCount ?? 0);
  const visibleEdgeCount = edges.length;
  const lines = [
    `focus: ${state.designerFocusNodeId || ""}`,
    `visible: ${visibleNodeCount}/${fullNodeCount} nodes, ${visibleEdgeCount}/${fullEdgeCount} edges`,
    "nodes:",
  ];
  for (const node of nodes) {
    if (node.aggregate) {
      const hiddenCount = node.hiddenNodeCount ?? toStringValue(node.label).replace(/^\+/, "") ?? "0";
      lines.push(`- +${hiddenCount} (${node.aggregateLabel || node.aggregateBucket || "aggregate"})`);
      continue;
    }
    lines.push(`- ${node.label || node.id} (${node.kind || "node"}) [${nodeDepth(node)}]`);
  }
  lines.push("edges:");
  for (const edge of edges) {
    const evidence = edge.evidence_status || (edge.evidence ? "available" : "unavailable");
    lines.push(
      `- ${edge.from} -> ${edge.to} : ${edge.summary || edge.label || edge.kind || "edge"} [${normalizePriority(edge)}/${evidence}]`,
    );
  }
  if (state.lockedTarget) {
    lines.push(`locked: ${formatTargetString(state.lockedTarget)}`);
  }
  return lines.join("\n");
}

function pickPositionDepth(node, focusNodeId) {
  if (node.id === focusNodeId) return 0;
  if (Number.parseInt(node.depth, 10) >= 2) return 2;
  if (Number.parseInt(node.depth, 10) === 1) return 1;
  return Number.parseInt(node.depth, 10);
}

function buildVisualsForGraph(nodes, edges, focusNodeId, options) {
  const priorityByNode = new Map();
  for (const edge of edges) {
    const priority = normalizePriority(edge);
    for (const nodeId of [edge.from, edge.to]) {
      if (!priorityByNode.has(nodeId)) priorityByNode.set(nodeId, new Map());
      const map = priorityByNode.get(nodeId);
      map.set(priority, (map.get(priority) || 0) + 1);
    }
  }

  function summarizePriorities(nodeId) {
    const map = priorityByNode.get(nodeId);
    if (!map) return "";
    return Array.from(map.entries())
      .sort((left, right) => right[1] - left[1])
      .map(([priority, count]) => `${priority}:${count}`)
      .join(", ");
  }

  const normalizedNodes = nodes.map((node) => {
    const hopDepth = pickPositionDepth(node, focusNodeId);
    const isFocus = node.id === focusNodeId;
    const hasHighPriority =
      (priorityByNode.get(node.id)?.get("filter") || 0) > 0 ||
      (priorityByNode.get(node.id)?.get("condition") || 0) > 0 ||
      (priorityByNode.get(node.id)?.get("visibility") || 0) > 0;
    return {
      ...node,
      hopDepth,
      isFocus,
      relatedPrioritySummary: summarizePriorities(node.id),
      opacity: isFocus ? 1 : isTranslucentDepth(hopDepth) ? options.twoHopOpacity : options.oneHopOpacity,
      style: {
        fillOpacity: isFocus ? 1 : isTranslucentDepth(hopDepth) ? options.twoHopOpacity : options.oneHopOpacity,
        strokeOpacity: 1,
        radius: isFocus
          ? options.nodeRadius * 1.26
          : isTranslucentDepth(hopDepth)
            ? options.nodeRadius * 0.46
            : hasHighPriority
              ? options.nodeRadius * 0.9
              : options.nodeRadius * 0.72,
      },
    };
  });

  const normalizedEdges = edges.map((edge) => {
    const translucent = isTranslucentDepth(edge.fromDepth) || isTranslucentDepth(edge.toDepth);
    const priority = normalizePriority(edge);
    const style = getEdgePriorityStyle(edge);
    const isFocusEdge = edge.from === focusNodeId || edge.to === focusNodeId;
    const priorityRank = Number.parseInt(style.rank, 10) || (
      priority === "filter" ? 5 : priority === "condition" ? 4 : priority === "visibility" ? 3 : 2
    );
    const priorityBoost = priorityRank >= 3 && !translucent;
    return {
      ...edge,
      priority,
      priorityRank,
      opacity: translucent ? options.twoHopEdgeOpacity : isFocusEdge ? style.opacity : 0.2,
      style: {
        opacity: translucent ? options.twoHopEdgeOpacity : isFocusEdge ? style.opacity : 0.2,
        color: style.color,
        width: style.width,
      },
    };
  });

  return {
    nodes: normalizedNodes,
    edges: normalizedEdges,
  };
}

function setRendererMarkers(root, graph, state, usedRenderer) {
  if (!root || typeof root.setAttribute !== "function") return;
  const viewGraph = graph?.viewGraph || {};
  const hoverTarget = state.hoveredTarget || null;
  const lockedTarget = state.lockedTarget || null;
  const detailTarget = getCurrentDetailPayload(state);
  const highlightContext = collectInteractionContext(state) || {
    highlightNodes: new Set(),
    highlightEdges: new Set(),
  };
  const detailKind = detailTarget?.type || TARGET_KIND_NONE;
  const hoverTargetString = formatTargetString(hoverTarget);
  const lockedTargetString = formatTargetString(lockedTarget);
  const detailTargetString = formatTargetString(state.hoveredTarget || state.lockedTarget || null);
  const viewport = state.viewport || {};
  const viewportTarget = viewport.target || makeTarget(TARGET_KIND_FOCUS, state.designerFocusNodeId);
  root.setAttribute("data-metadata-checker-local-graph-ready", "true");
  root.setAttribute("data-metadata-checker-local-graph-focus", state.designerFocusNodeId ?? "");
  root.setAttribute("data-metadata-checker-local-graph-node-count", String(graph?.nodes?.length ?? 0));
  root.setAttribute("data-metadata-checker-local-graph-edge-count", String(graph?.edges?.length ?? 0));
  root.setAttribute("data-metadata-checker-local-graph-visible-node-count", String(viewGraph.visibleNodeCount ?? graph?.nodes?.length ?? 0));
  root.setAttribute("data-metadata-checker-local-graph-visible-edge-count", String(viewGraph.visibleEdgeCount ?? graph?.edges?.length ?? 0));
  root.setAttribute("data-metadata-checker-local-graph-hidden-node-count", String(viewGraph.hiddenNodeCount ?? 0));
  root.setAttribute("data-metadata-checker-local-graph-aggregate-node-count", String(viewGraph.aggregateNodeCount ?? 0));
  root.setAttribute("data-metadata-checker-graph-visible-node-count", String(viewGraph.visibleNodeCount ?? graph?.nodes?.length ?? 0));
  root.setAttribute("data-metadata-checker-graph-visible-edge-count", String(viewGraph.visibleEdgeCount ?? graph?.edges?.length ?? 0));
  root.setAttribute("data-metadata-checker-graph-hidden-node-count", String(viewGraph.hiddenNodeCount ?? 0));
  root.setAttribute("data-metadata-checker-graph-aggregate-node-count", String(viewGraph.aggregateNodeCount ?? 0));
  root.setAttribute("data-metadata-checker-graph-hover-target", hoverTargetString);
  root.setAttribute("data-metadata-checker-graph-locked-target", lockedTargetString);
  root.setAttribute("data-metadata-checker-graph-detail-kind", detailKind);
  root.setAttribute("data-metadata-checker-graph-open-detail-context", detailTargetString === TARGET_KIND_NONE ? "" : detailTargetString);
  root.setAttribute("data-metadata-checker-graph-highlight-node-count", String(highlightContext.highlightNodes.size ?? 0));
  root.setAttribute("data-metadata-checker-graph-highlight-edge-count", String(highlightContext.highlightEdges.size ?? 0));
  root.setAttribute("data-metadata-checker-graph-viewport-scale", String(Number((viewport.scale ?? VIEWPORT_SCALE_DEFAULT).toFixed(VIEWPORT_SCALE_PRECISION))));
  root.setAttribute("data-metadata-checker-graph-viewport-target", formatTargetString(viewportTarget));
  root.setAttribute("data-metadata-checker-graph-density-profile", state.options?.densityProfile || "compact");
  root.setAttribute("data-metadata-checker-local-graph-renderer", usedRenderer || "fallback");
}

async function resolvePixiRuntime(pixiHint) {
  if (pixiHint) return pixiHint.PIXI ?? pixiHint.default ?? pixiHint;
  try {
    const module = await import("pixi.js");
    return module.PIXI ?? module.default ?? module;
  } catch {
    return null;
  }
}

async function resolvePixiRuntimeFromOptions(options) {
  if (options.pixi) {
    return resolvePixiRuntime(options.pixi);
  }
  if (options.pixiRuntimeLoader && typeof options.pixiRuntimeLoader.loadPixi === "function") {
    return options.pixiRuntimeLoader.loadPixi();
  }
  return resolvePixiRuntime(null);
}

async function renderWithPixiFallback(host, graph, state, options) {
  const fallbackCanvas = createRendererElement(options, "canvas");
  fallbackCanvas.setAttribute?.("data-metadata-checker-local-graph-canvas-fallback", "true");
  fallbackCanvas.style.width = "100%";
  fallbackCanvas.style.height = "100%";
  fallbackCanvas.style.display = "block";
  fallbackCanvas.width = Number.parseFloat(options.width) || 320;
  fallbackCanvas.height = Number.parseFloat(options.height) || 220;
  const context = fallbackCanvas.getContext?.("2d");
  if (context && typeof context.fillRect === "function") {
    context.fillStyle = "#141821";
    context.fillRect(0, 0, fallbackCanvas.width, fallbackCanvas.height);
    context.strokeStyle = "#7dd3fc";
    context.lineWidth = 1.5;
    context.setLineDash([6, 5]);
    context.beginPath();
    context.moveTo(8, 8);
    context.lineTo(fallbackCanvas.width - 8, fallbackCanvas.height - 8);
    context.moveTo(fallbackCanvas.width - 8, 8);
    context.lineTo(8, fallbackCanvas.height - 8);
    context.stroke();
    context.setLineDash([]);
    context.fillStyle = "#7dd3fc";
    context.beginPath();
    context.arc(fallbackCanvas.width / 2, fallbackCanvas.height / 2, 14, 0, Math.PI * 2);
    context.fill();
    context.fillStyle = "#fff";
    context.font = "12px sans-serif";
    context.textAlign = "center";
    context.fillText("Fallback renderer", fallbackCanvas.width / 2, 16);
    context.fillText(
      `${graph?.nodes?.length ?? 0} nodes`,
      fallbackCanvas.width / 2,
      fallbackCanvas.height / 2 - 6,
    );
    context.fillText(
      `${graph?.edges?.length ?? 0} edges`,
      fallbackCanvas.width / 2,
      fallbackCanvas.height / 2 + 12,
    );
  }
  host.appendChild(fallbackCanvas);
  return false;
}

function makePixiApplicationOptions(options) {
  const dpr = typeof globalThis.devicePixelRatio === "number"
    ? Math.min(2, globalThis.devicePixelRatio)
    : 1;
  return {
    width: Number.parseFloat(options.width) || 320,
    height: Number.parseFloat(options.height) || 220,
    backgroundColor: options.backgroundColor,
    background: options.backgroundColor,
    antialias: true,
    resolution: dpr,
    autoDensity: true,
  };
}

async function createPixiApplication(runtime, options) {
  const appOptions = makePixiApplicationOptions(options);
  let app = null;

  try {
    app = new runtime.Application(appOptions);
    if (typeof app.init === "function") {
      await app.init(appOptions);
    }
  } catch {
    app = new runtime.Application();
    if (typeof app.init === "function") {
      await app.init(appOptions);
    }
  }

  return app;
}

function createPixiContainer(runtime) {
  if (typeof runtime.Container === "function") {
    return new runtime.Container();
  }
  return {
    children: [],
    addChild(child) {
      this.children.push(child);
      return child;
    },
  };
}

function createPixiGraphics(runtime) {
  if (typeof runtime.Graphics === "function") {
    return new runtime.Graphics();
  }
  return null;
}

function createPixiText(runtime, text, options = {}) {
  if (typeof runtime.Text !== "function") {
    return null;
  }
  const style = {
    fill: options.fill ?? 0xe5edf7,
    fontFamily: options.fontFamily || "Inter, ui-sans-serif, system-ui, sans-serif",
    fontSize: options.fontSize ?? 10,
    fontWeight: options.fontWeight || "600",
    align: "center",
  };
  try {
    return new runtime.Text({ text, style });
  } catch (_error) {
    try {
      return new runtime.Text(text, style);
    } catch (_nestedError) {
      return null;
    }
  }
}

function drawPixiCircle(graphics, radius, color, alpha) {
  if (!graphics) return;
  if (typeof graphics.circle === "function" && typeof graphics.fill === "function") {
    graphics.circle(0, 0, radius).fill({ color, alpha });
    return;
  }
  if (typeof graphics.beginFill === "function") {
    graphics.beginFill(color, alpha);
    graphics.drawCircle?.(0, 0, radius);
    graphics.endFill?.();
  }
}

function drawPixiStrokeCircle(graphics, radius, color, alpha, width = 1) {
  if (!graphics) return;
  if (typeof graphics.circle === "function" && typeof graphics.stroke === "function") {
    graphics.circle(0, 0, radius).stroke({ width, color, alpha });
    return;
  }
  if (typeof graphics.lineStyle === "function") {
    graphics.lineStyle(width, color, alpha);
    graphics.drawCircle?.(0, 0, radius);
  }
}

function clearPixiGraphics(graphics) {
  if (!graphics) return;
  if (typeof graphics.clear === "function") {
    graphics.clear();
  }
}

function readPixiPoint(point) {
  if (!point || typeof point !== "object") {
    return { x: 0, y: 0 };
  }
  return {
    x: Number(point.x) || 0,
    y: Number(point.y) || 0,
  };
}

function drawPixiLine(graphics, from, to, width, color, alpha) {
  if (!graphics || !from || !to) return;
  // Pixi v8 仍暴露 lineStyle，但已不绘制；必须走 moveTo/lineTo/stroke。
  if (typeof graphics.moveTo === "function" && typeof graphics.lineTo === "function" && typeof graphics.stroke === "function") {
    graphics.moveTo(from.x, from.y);
    graphics.lineTo(to.x, to.y);
    graphics.stroke({ width, color, alpha });
    return;
  }
  if (typeof graphics.lineStyle === "function") {
    graphics.lineStyle(width, color, alpha);
    graphics.moveTo?.(from.x, from.y);
    graphics.lineTo?.(to.x, to.y);
  }
}

function paintEdgeGraphics(graphics, from, to, edge, viewGraph, options) {
  if (!graphics || !from || !to || !edge) return;
  clearPixiGraphics(graphics);
  const edgeStyle = getEdgePriorityStyle(edge);
  const aggregateEdge = edgeTouchesAggregate(edge, viewGraph);
  const drawLine = aggregateEdge ? drawPixiDashedLine : drawPixiLine;
  const width = aggregateEdge ? Math.max(1, edgeStyle.width * 0.9) : edgeStyle.width;
  const opacity = aggregateEdge ? (edge.opacity ?? 1) * 0.82 : (edge.opacity ?? 1);
  drawLine(graphics, from, to, width, edgeStyle.color, opacity);
}

function syncEdgeGeometries(state, options) {
  const viewGraph = state.lastViewGraph || state.lastRenderGraph?.viewGraph;
  if (!viewGraph) return;
  for (const [edgeId, edgeView] of state.edgeViews) {
    const edge = state.viewEdgeById?.get(edgeId) || state.edgeById?.get(edgeId);
    if (!edge || !edgeView?.sprite) continue;
    const fromView = state.nodeViews.get(edge.from);
    const toView = state.nodeViews.get(edge.to);
    if (!fromView?.sprite || !toView?.sprite) continue;
    const from = readPixiPoint(fromView.sprite.position);
    const to = readPixiPoint(toView.sprite.position);
    paintEdgeGraphics(edgeView.sprite, from, to, edge, viewGraph, options);
    if (edgeView.focusHalo) {
      clearPixiGraphics(edgeView.focusHalo);
      const edgeStyle = getEdgePriorityStyle(edge);
      drawPixiLine(edgeView.focusHalo, from, to, edgeStyle.width, 0xf8fafc, 0.16);
    }
  }
}

function syncLabelPositions(state) {
  const viewport = state.viewport || {};
  const scale = viewport.scale ?? VIEWPORT_SCALE_DEFAULT;
  const tx = state.viewLayer?.x ?? viewport.translateX ?? 0;
  const ty = state.viewLayer?.y ?? viewport.translateY ?? 0;
  for (const [, nodeView] of state.nodeViews) {
    if (!nodeView?.label || !nodeView?.sprite) continue;
    const pos = readPixiPoint(nodeView.sprite.position);
    const radius = Number(nodeView.sprite.radius) || state.options?.nodeRadius || 4.2;
    nodeView.label.position = {
      x: pos.x * scale + tx + radius + 4,
      y: pos.y * scale + ty - 1,
    };
    if (nodeView.label.scale && typeof nodeView.label.scale === "object") {
      if (typeof nodeView.label.scale.set === "function") {
        nodeView.label.scale.set(1, 1);
      } else {
        nodeView.label.scale.x = 1;
        nodeView.label.scale.y = 1;
      }
    }
    if (typeof nodeView.label.resolution === "number" || nodeView.label.resolution == null) {
      const dpr = typeof globalThis.devicePixelRatio === "number" ? globalThis.devicePixelRatio : 1;
      nodeView.label.resolution = Math.min(3, dpr * 1.25);
    }
  }
}

function drawPixiDashedLine(graphics, from, to, width, color, alpha, dashLength = 5, gapLength = 4) {
  if (!graphics || !from || !to) return;
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const distance = Math.hypot(dx, dy);
  if (distance < 1) return;
  const ux = dx / distance;
  const uy = dy / distance;
  let traveled = 0;
  let drawing = true;
  let cx = from.x;
  let cy = from.y;
  while (traveled < distance - 0.01) {
    const segment = drawing ? dashLength : gapLength;
    const nextTravel = Math.min(distance, traveled + segment);
    const nx = from.x + ux * nextTravel;
    const ny = from.y + uy * nextTravel;
    if (drawing) {
      drawPixiLine(graphics, { x: cx, y: cy }, { x: nx, y: ny }, width, color, alpha);
    }
    cx = nx;
    cy = ny;
    traveled = nextTravel;
    drawing = !drawing;
  }
}

function edgeTouchesAggregate(edge, viewGraph) {
  if (!edge) return false;
  const nodeById = viewGraph?.nodeById;
  if (!(nodeById instanceof Map)) return false;
  const fromNode = nodeById.get(edge.from);
  const toNode = nodeById.get(edge.to);
  return Boolean(fromNode?.aggregate || toNode?.aggregate);
}

function edgePriorityRank(edge) {
  return Number.parseInt(getEdgePriorityStyle(edge).rank, 10) || 0;
}

function nodeDepth(node) {
  const parsed = Number.parseInt(node?.hopDepth ?? node?.depth ?? node?.metadata?.depth, 10);
  return Number.isFinite(parsed) ? Math.max(0, parsed) : 0;
}

function nodePriorityRank(nodeId, edges) {
  let rank = 0;
  for (const edge of edges) {
    if (edge?.from === nodeId || edge?.to === nodeId) {
      rank = Math.max(rank, edgePriorityRank(edge));
    }
  }
  return rank;
}

function compareNodesByVisibilityPriority(left, right, edges, focusNodeId) {
  const leftFocus = left.id === focusNodeId ? 1 : 0;
  const rightFocus = right.id === focusNodeId ? 1 : 0;
  if (leftFocus !== rightFocus) return rightFocus - leftFocus;
  const leftDepth = nodeDepth(left);
  const rightDepth = nodeDepth(right);
  if (leftDepth !== rightDepth) return leftDepth - rightDepth;
  const leftTier = nodeSelectionTier(left, edges, focusNodeId);
  const rightTier = nodeSelectionTier(right, edges, focusNodeId);
  if (leftTier !== rightTier) return rightTier - leftTier;
  const leftRank = nodePriorityRank(left.id, edges);
  const rightRank = nodePriorityRank(right.id, edges);
  if (leftRank !== rightRank) return rightRank - leftRank;
  return toStringValue(left.id).localeCompare(toStringValue(right.id));
}

function summarizePrioritiesForNodes(nodes, edges) {
  const counts = new Map();
  const nodeIds = new Set(nodes.map((node) => node.id));
  for (const edge of edges) {
    if (!nodeIds.has(edge?.from) && !nodeIds.has(edge?.to)) continue;
    const priority = normalizePriority(edge);
    counts.set(priority, (counts.get(priority) || 0) + 1);
  }
  return Array.from(counts.entries())
    .sort((left, right) => (EDGE_PRIORITY_STYLES[right[0]]?.rank ?? 0) - (EDGE_PRIORITY_STYLES[left[0]]?.rank ?? 0))
    .map(([priority, count]) => `${priority}:${count}`)
    .join(" ");
}

function aggregateBucketForNode(node, edges) {
  const depth = nodeDepth(node);
  if (depth <= 1) return "hidden-1hop";
  const rank = nodePriorityRank(node.id, edges);
  if (rank >= EDGE_PRIORITY_STYLES.condition.rank) return "hidden-2hop-filter-condition";
  const prioritySummary = summarizePrioritiesForNodes([node], edges);
  if (prioritySummary.includes("source:")) return "hidden-2hop-source";
  return "hidden-2hop-other";
}

function aggregateLabelForBucket(bucketId) {
  if (bucketId === "hidden-1hop") return "1-hop";
  if (bucketId === "hidden-2hop-filter-condition") return "filter/condition";
  if (bucketId === "hidden-2hop-source") return "source";
  return "other";
}

function averagePosition(nodes, fallback) {
  let x = 0;
  let y = 0;
  let count = 0;
  for (const node of nodes) {
    if (!node?.position) continue;
    x += Number.parseFloat(node.position.x) || 0;
    y += Number.parseFloat(node.position.y) || 0;
    count += 1;
  }
  if (count === 0) return fallback;
  return { x: x / count, y: y / count };
}

function computePixiViewportTransform(nodes, focusNodeId, options) {
  const width = Number.parseFloat(options.width) || 320;
  const height = Number.parseFloat(options.height) || 220;
  const centerX = width / 2;
  const centerY = height / 2;
  const padding = Math.max(18, Number.parseFloat(options.viewportPadding) || options.nodeRadius * 4 || 24);
  const focusNode = nodes.find((node) => node.id === focusNodeId) || nodes[0] || {};
  const focusPosition = focusNode.position || { x: centerX, y: centerY };
  let maxAbsX = 1;
  let maxAbsY = 1;

  for (const node of nodes) {
    const position = node.position || focusPosition;
    maxAbsX = Math.max(maxAbsX, Math.abs((position.x ?? centerX) - (focusPosition.x ?? centerX)));
    maxAbsY = Math.max(maxAbsY, Math.abs((position.y ?? centerY) - (focusPosition.y ?? centerY)));
  }

  const fitX = (centerX - padding) / Math.max(1, maxAbsX);
  const fitY = (centerY - padding) / Math.max(1, maxAbsY);
  const scale = Math.min(1, fitX, fitY);
  return {
    centerX,
    centerY,
    focusX: focusPosition.x ?? centerX,
    focusY: focusPosition.y ?? centerY,
    scale,
  };
}

function transformPixiPosition(position, transform) {
  return {
    x: transform.centerX + ((position?.x ?? transform.focusX) - transform.focusX) * transform.scale,
    y: transform.centerY + ((position?.y ?? transform.focusY) - transform.focusY) * transform.scale,
  };
}

function pickVisiblePixiNodes(nodes, edges, focusNodeId, maxRenderedNodes) {
  const budget = Math.max(1, Number.parseInt(maxRenderedNodes, 10) || 36);
  const focusNode = nodes.find((node) => node.id === focusNodeId) || nodes[0];
  const selected = new Map();
  if (focusNode?.id) {
    selected.set(focusNode.id, focusNode);
  }
  const focusNeighbors = new Set();
  for (const edge of edges) {
    if (edge?.from === focusNode?.id && edge.to) focusNeighbors.add(edge.to);
    if (edge?.to === focusNode?.id && edge.from) focusNeighbors.add(edge.from);
  }
  const remainingAfterFocus = Math.max(0, budget - selected.size);
  const oneHopBudget = Math.min(remainingAfterFocus, Math.max(0, Math.floor(budget * 0.66)));
  const oneHopNodes = nodes
    .filter((node) => node.id !== focusNode?.id && focusNeighbors.has(node.id))
    .sort((left, right) => compareNodesByVisibilityPriority(left, right, edges, focusNodeId))
    .slice(0, oneHopBudget);
  for (const node of oneHopNodes) {
    selected.set(node.id, node);
  }
  const selectedOneHop = new Set(oneHopNodes.map((node) => node.id));
  const contextNodes = nodes
    .filter((node) => node.id !== focusNode?.id && !selected.has(node.id))
    .filter((node) => edges.some((edge) =>
      (selectedOneHop.has(edge.from) && edge.to === node.id) ||
      (selectedOneHop.has(edge.to) && edge.from === node.id),
    ))
    .sort((left, right) => compareNodesByVisibilityPriority(left, right, edges, focusNodeId))
    .slice(0, Math.max(0, budget - selected.size));
  for (const node of contextNodes) {
    selected.set(node.id, node);
  }
  return Array.from(selected.values());
}

function pickVisiblePixiEdges(edges, focusNodeId, maxRenderedEdges) {
  const budget = Math.max(1, Number.parseInt(maxRenderedEdges, 10) || 90);
  return [...edges]
    .sort((left, right) => {
      const leftFocus = left.from === focusNodeId || left.to === focusNodeId ? 1 : 0;
      const rightFocus = right.from === focusNodeId || right.to === focusNodeId ? 1 : 0;
      if (leftFocus !== rightFocus) return rightFocus - leftFocus;
      const leftRank = edgePriorityRank(left);
      const rightRank = edgePriorityRank(right);
      if (leftRank !== rightRank) return rightRank - leftRank;
      const leftDepth = (isTranslucentDepth(left.fromDepth) || isTranslucentDepth(left.toDepth)) ? 1 : 0;
      const rightDepth = (isTranslucentDepth(right.fromDepth) || isTranslucentDepth(right.toDepth)) ? 1 : 0;
      if (leftDepth !== rightDepth) return leftDepth - rightDepth;
      return toStringValue(left.id || `${left.from}->${left.to}`).localeCompare(
        toStringValue(right.id || `${right.from}->${right.to}`),
      );
    })
    .slice(0, budget);
}

function buildAggregateNodes(hiddenNodes, edges, visibleNodeIds, focusNodeId, visibleNodeById = new Map()) {
  const buckets = new Map();
  for (const node of hiddenNodes) {
    const bucketId = aggregateBucketForNode(node, edges);
    if (!buckets.has(bucketId)) {
      buckets.set(bucketId, []);
    }
    buckets.get(bucketId).push(node);
  }

  const aggregates = [];
  const aggregateEdges = [];
  for (const [bucketId, bucketNodes] of buckets) {
    if (bucketNodes.length === 0) continue;
    const aggregateId = `aggregate:${bucketId}`;
    const prioritySummary = summarizePrioritiesForNodes(bucketNodes, edges);
    const hiddenNodeIds = new Set(bucketNodes.map((node) => node.id));
    const visibleLinks = new Map();
    let hiddenEdgeCount = 0;
    for (const edge of edges) {
      const hiddenFrom = hiddenNodeIds.has(edge?.from);
      const hiddenTo = hiddenNodeIds.has(edge?.to);
      if (!hiddenFrom && !hiddenTo) continue;
      hiddenEdgeCount += 1;
      const visibleTarget = hiddenFrom && visibleNodeIds.has(edge.to)
        ? edge.to
        : hiddenTo && visibleNodeIds.has(edge.from)
          ? edge.from
          : null;
      if (visibleTarget) {
        const current = visibleLinks.get(visibleTarget) || { count: 0, rank: 0 };
        current.count += 1;
        current.rank = Math.max(current.rank, edgePriorityRank(edge));
        visibleLinks.set(visibleTarget, current);
      }
    }

    const targetId = Array.from(visibleLinks.entries())
      .sort((left, right) => {
        if (left[1].rank !== right[1].rank) return right[1].rank - left[1].rank;
        if (left[1].count !== right[1].count) return right[1].count - left[1].count;
        if (left[0] === focusNodeId) return -1;
        if (right[0] === focusNodeId) return 1;
        return toStringValue(left[0]).localeCompare(toStringValue(right[0]));
      })[0]?.[0] || focusNodeId;

    const focusPosition = visibleNodeById.get(targetId)?.position || visibleNodeById.get(focusNodeId)?.position || { x: 0, y: 0 };
    const hiddenAverage = averagePosition(bucketNodes, focusPosition);
    const aggregatePosition = {
      x: focusPosition.x + (hiddenAverage.x - focusPosition.x) * 0.62,
      y: focusPosition.y + (hiddenAverage.y - focusPosition.y) * 0.62,
    };
    const aggregateNode = {
      id: aggregateId,
      label: `+${bucketNodes.length}`,
      visualLabel: `+${bucketNodes.length}`,
      kind: "aggregate",
      aggregate: true,
      aggregateBucket: bucketId,
      aggregateLabel: aggregateLabelForBucket(bucketId),
      hiddenNodeCount: bucketNodes.length,
      hiddenEdgeCount,
      relatedPrioritySummary: prioritySummary || "none",
      depth: bucketId === "hidden-1hop" ? 1 : 2,
      hopDepth: bucketId === "hidden-1hop" ? 1 : 2,
      opacity: bucketId === "hidden-1hop" ? 0.5 : 0.34,
      position: aggregatePosition,
      style: {
        fillOpacity: bucketId === "hidden-1hop" ? 0.5 : 0.34,
        strokeOpacity: 0.78,
        radius: 7,
      },
    };
    aggregates.push(aggregateNode);

    if (targetId) {
      aggregateEdges.push({
        id: `${aggregateId}->${targetId}`,
        from: aggregateId,
        to: targetId,
        kind: "aggregate",
        priority: "other",
        summary: `${bucketNodes.length} hidden ${aggregateNode.aggregateLabel} nodes`,
        direction: "Aggregate",
        evidence_status: "unavailable",
        aggregate: true,
        hiddenNodeCount: bucketNodes.length,
        hiddenEdgeCount,
        opacity: 0.2,
        style: {
          opacity: 0.2,
          color: EDGE_PRIORITY_STYLES.other.color,
          width: 0.8,
        },
      });
    }
  }
  return { aggregateNodes: aggregates.slice(0, 4), aggregateEdges: aggregateEdges.slice(0, 4) };
}

function buildPixiViewGraph(graph, options) {
  const allNodes = Array.isArray(graph?.nodes) ? graph.nodes : [];
  const allEdges = Array.isArray(graph?.edges) ? graph.edges : [];
  const focusNodeId = graph?.focusNodeId || graph?.focus_node || allNodes[0]?.id || null;
  if (allNodes.length === 1 && allEdges.length === 0 && allNodes[0]?.id === focusNodeId) {
    return {
      nodes: allNodes,
      edges: [],
      nodeById: createNodeLookup(allNodes),
      edgeById: createEdgeLookup([]),
      visibleRealNodes: allNodes,
      visibleRealEdges: [],
      aggregateNodes: [],
      aggregateEdges: [],
      hiddenNodes: [],
      hiddenNodeCount: 0,
      aggregateNodeCount: 0,
      visibleNodeCount: 1,
      visibleEdgeCount: 0,
    };
  }
  const visibleRealNodes = pickVisiblePixiNodes(allNodes, allEdges, focusNodeId, options.maxRenderedNodes);
  const visibleNodeIds = new Set(visibleRealNodes.map((node) => node.id));
  const visibleNodeById = createNodeLookup(visibleRealNodes);
  const realVisibleEdges = pickVisiblePixiEdges(
    allEdges.filter((edge) => visibleNodeIds.has(edge?.from) && visibleNodeIds.has(edge?.to)),
    focusNodeId,
    options.maxRenderedEdges,
  );
  const hiddenNodes = allNodes.filter((node) => !visibleNodeIds.has(node.id));
  const { aggregateNodes, aggregateEdges } = buildAggregateNodes(
    hiddenNodes,
    allEdges,
    visibleNodeIds,
    focusNodeId,
    visibleNodeById,
  );
  return {
    nodes: [...visibleRealNodes, ...aggregateNodes],
    edges: [...realVisibleEdges, ...aggregateEdges],
    nodeById: createNodeLookup([...visibleRealNodes, ...aggregateNodes]),
    edgeById: createEdgeLookup([...realVisibleEdges, ...aggregateEdges]),
    visibleRealNodes,
    visibleRealEdges: realVisibleEdges,
    aggregateNodes,
    aggregateEdges,
    hiddenNodes,
    hiddenNodeCount: hiddenNodes.length,
    aggregateNodeCount: aggregateNodes.length,
    visibleNodeCount: visibleRealNodes.length,
    visibleEdgeCount: realVisibleEdges.length,
  };
}

function nodeFillColor(node, focusNodeId, edges) {
  if (node.aggregate) return 0x94a3b8;
  if (node.id === focusNodeId) return 0x93c5fd;
  const rank = nodePriorityRank(node.id, edges);
  if (rank >= 4) return 0xdbeafe;
  if (rank >= 2) return 0x7dd3fc;
  return isTranslucentDepth(nodeDepth(node)) ? 0x31537a : 0x60a5fa;
}

function addConnectedEdgeContext(context, state, nodeId) {
  for (const [edgeId, edge] of state.edgeById?.entries() || []) {
    if (!edge?.from || !edge?.to) continue;
    if (edge.from === nodeId || edge.to === nodeId) {
      context.highlightEdges.add(edgeId);
    }
  }
}

function collectInteractionContext(state) {
  const context = {
    highlightNodes: new Set(),
    highlightEdges: new Set(),
  };

  const targets = [state.hoveredTarget, state.lockedTarget];
  for (const target of targets) {
    if (!target || !target.id) continue;
    const targetType = normalizeTargetType(target.type);
    if (targetType === TARGET_KIND_EDGE) {
      const edge = state.edgeById?.get(target.id) || state.viewEdgeById?.get(target.id);
      if (!edge) continue;
      context.highlightEdges.add(target.id);
      if (edge.from) context.highlightNodes.add(edge.from);
      if (edge.to) context.highlightNodes.add(edge.to);
      continue;
    }

    const nodeId = target.id;
    const neighbors = state.nodeNeighborsById?.get(nodeId);
    if (neighbors) {
      context.highlightNodes.add(nodeId);
      for (const nodeId of neighbors) {
        context.highlightNodes.add(nodeId);
      }
    }
    for (const edgeId of state.edgeById?.keys() || []) {
      const edge = state.edgeById.get(edgeId);
      if (!edge?.from || !edge?.to) continue;
      if (edge.from === nodeId || edge.to === nodeId) {
        context.highlightEdges.add(edgeId);
      }
    }
  }

  return context;
}

function isTargetLocked(state, target) {
  return isSameTarget(state.lockedTarget, target);
}

function isTargetHovered(state, target) {
  return isSameTarget(state.hoveredTarget, target);
}

function shouldShowNodeLabel(state, node, isLocked, isHovered) {
  if (!node) return false;
  if (node.aggregate) return true;
  const nodeId = node.id;
  if (isLocked || isHovered || nodeId === state.designerFocusNodeId) return true;
  const edges = state.lastViewGraph?.edges || Array.from(state.viewEdgeById?.values() || []);
  return nodeSelectionTier(node, edges, state.designerFocusNodeId) >= NODE_TIER_SOURCE;
}

function applyInteractionContextStyles(state, options) {
  const context = collectInteractionContext(state);
  const hasContext = context.highlightNodes.size > 0 || context.highlightEdges.size > 0;
  const dimAlpha = options.hoverDimAlpha ?? 0.22;

  for (const [nodeId, nodeView] of state.nodeViews) {
    const node = state.viewNodeById.get(nodeId) || state.nodeById.get(nodeId);
    if (!node || !nodeView?.sprite) continue;
    const target = makeTarget(node.aggregate ? TARGET_KIND_AGGREGATE : TARGET_KIND_NODE, nodeId);
    const isFocus = nodeId === state.designerFocusNodeId;
    const isLocked = isTargetLocked(state, target);
    const isHovered = isTargetHovered(state, target);
    const highlighted = context.highlightNodes.has(nodeId);
    const baseOpacity = node.style?.fillOpacity ?? options.oneHopOpacity;
    const baseRadius = node.style?.radius ?? options.nodeRadius;
    const nextAlpha = isFocus ? 1 : (highlighted || isLocked || isHovered || !hasContext ? baseOpacity : dimAlpha);
    const focusScale = isFocus ? 1.16 : highlighted ? 1.06 : 1;
    const interactionScale = isLocked || isHovered ? 1.2 : 1;
    const nextRadius = baseRadius * Math.max(focusScale, interactionScale);
    nodeView.sprite.alpha = nextAlpha;
    nodeView.sprite.radius = nextRadius;
    nodeView.sprite.strokeWidth = isLocked ? 1.8 : isHovered || highlighted ? 1.25 : 0.8;
    if (nodeView.halo) {
      const aggregateBaseAlpha = node.aggregate ? 0.3 : 0;
      nodeView.halo.alpha = isLocked ? 0.62 : isHovered ? 0.24 : aggregateBaseAlpha;
      nodeView.halo.radius = nextRadius * 1.42;
      if (typeof nodeView.halo.strokeWidth === "number") {
        nodeView.halo.strokeWidth = isLocked ? 1.6 : isHovered ? 1.1 : 1;
      }
    }
    if (nodeView.label && typeof nodeView.label === "object") {
      nodeView.label.alpha = shouldShowNodeLabel(state, node, isLocked, isHovered) ? 0.92 : 0;
      if (nodeView.label.scale && typeof nodeView.label.scale === "object") {
        if (typeof nodeView.label.scale.set === "function") {
          nodeView.label.scale.set(1, 1);
        } else {
          nodeView.label.scale.x = 1;
          nodeView.label.scale.y = 1;
        }
      }
    }
    if (typeof nodeView.sprite.tint === "number") {
      if (isLocked) {
        nodeView.sprite.tint = LOCKED_EDGE_COLOR;
      } else if (node.aggregate) {
        nodeView.sprite.tint = 0x94a3b8;
      } else if (isFocus) {
        nodeView.sprite.tint = 0xf8fafc;
      } else if (highlighted || isHovered) {
        nodeView.sprite.tint = 0xffffff;
      } else {
        nodeView.sprite.tint = 0x89a7bf;
      }
    }
  }

  for (const [edgeId, edgeView] of state.edgeViews) {
    const edge = state.viewEdgeById.get(edgeId) || state.edgeById.get(edgeId);
    if (!edge || !edgeView?.sprite) continue;
    const target = makeTarget(TARGET_KIND_EDGE, edgeId);
    const isLocked = isTargetLocked(state, target);
    const isHovered = isTargetHovered(state, target);
    const highlighted = context.highlightEdges.has(edgeId);
    const baseOpacity = edge.style?.opacity ?? options.oneHopEdgeOpacity;
    const baseWidth = edge.style?.width ?? 1;
    const baseColor = edge.style?.color ?? getEdgePriorityStyle(edge).color;
    edgeView.sprite.alpha = hasContext ? (highlighted || isLocked || isHovered ? 1 : dimAlpha) : baseOpacity;
    edgeView.sprite.width = isLocked ? baseWidth * 1.65 : highlighted || isHovered ? baseWidth * 1.22 : baseWidth;
    edgeView.sprite.strokeWidth = edgeView.sprite.width;
    edgeView.sprite.tint = isLocked ? LOCKED_EDGE_COLOR : baseColor;
    if (edgeView.focusHalo) {
      edgeView.focusHalo.alpha = isLocked ? 0.32 : isHovered ? 0.18 : 0;
      if (typeof edgeView.focusHalo.width === "number") {
        edgeView.focusHalo.width = isLocked ? baseWidth * 1.8 : baseWidth;
      }
    }
  }

  state.lastInteractionContext = {
    nodes: Array.from(context.highlightNodes),
    edges: Array.from(context.highlightEdges),
  };

  return context;
}

function computeViewportBounds(width, height) {
  return {
    centerX: width / 2,
    centerY: height / 2,
    minTranslateX: -width,
    maxTranslateX: width,
    minTranslateY: -height,
    maxTranslateY: height,
  };
}

function projectToViewport(basePoint, viewport) {
  return {
    x: (basePoint.x * (viewport.scale ?? VIEWPORT_SCALE_DEFAULT)) + (viewport.translateX ?? 0),
    y: (basePoint.y * (viewport.scale ?? VIEWPORT_SCALE_DEFAULT)) + (viewport.translateY ?? 0),
  };
}

function emitRendererEvent(root, name, detail) {
  if (!root?.dispatchEvent || typeof root.dispatchEvent !== "function") return;
  if (typeof root.ownerDocument?.defaultView?.CustomEvent === "function") {
    root.dispatchEvent(new root.ownerDocument.defaultView.CustomEvent(name, { detail }));
    return;
  }
  if (typeof globalThis.CustomEvent === "function") {
    root.dispatchEvent(new globalThis.CustomEvent(name, { detail }));
    return;
  }
  root.dispatchEvent({ type: name, detail });
}

function baseTargetPoint(state, target) {
  if (!target || !target.id) {
    return state.baseCenter || { x: 0, y: 0 };
  }
  if (target.type === TARGET_KIND_EDGE) {
    const edge = state.viewEdgeById?.get(target.id) || state.edgeById?.get(target.id);
    if (!edge) return state.baseCenter || { x: 0, y: 0 };
    const from = state.baseNodePositionById?.get(edge.from);
    const to = state.baseNodePositionById?.get(edge.to);
    if (!from || !to) return state.baseCenter || { x: 0, y: 0 };
    return {
      x: (from.x + to.x) / 2,
      y: (from.y + to.y) / 2,
    };
  }
  return state.baseNodePositionById?.get(target.id) || state.baseCenter || { x: 0, y: 0 };
}

function resetViewportState(state, graph) {
  const width = Number.parseFloat(state.options?.width) || 320;
  const height = Number.parseFloat(state.options?.height) || 220;
  const focusId = state.designerFocusNodeId;
  const baseTransform = computePixiViewportTransform(graph?.nodes || [], focusId, state.options || {});
  const baseCenter = {
    x: baseTransform.centerX,
    y: baseTransform.centerY,
  };
  state.viewport = {
    scale: VIEWPORT_SCALE_DEFAULT,
    translateX: 0,
    translateY: 0,
    bounds: computeViewportBounds(width, height),
    target: makeTarget(TARGET_KIND_FOCUS, focusId),
    panning: false,
    dragging: false,
    pointer: null,
    options: {
      minScale: VIEWPORT_SCALE_MIN,
      maxScale: VIEWPORT_SCALE_MAX,
    },
  };
  state.baseTransform = baseTransform;
  state.baseNodePositionById = new Map();
  if (graph?.nodes?.length) {
    for (const node of graph.nodes) {
      if (!node?.id) continue;
      state.baseNodePositionById.set(
        node.id,
        projectToViewport(transformPixiPosition(node.position, baseTransform), state.viewport),
      );
    }
  }
  state.baseCenter = baseCenter;
  state.viewport.bounds = computeViewportBounds(width, height);
}

function setViewportTarget(state, target) {
  state.viewport = state.viewport || {};
  state.viewport.target = target;
}

function keepViewportInBounds(viewport, bounds) {
  viewport.translateX = clamp(viewport.translateX ?? 0, bounds.minTranslateX, bounds.maxTranslateX);
  viewport.translateY = clamp(viewport.translateY ?? 0, bounds.minTranslateY, bounds.maxTranslateY);
}

function frameViewportToTarget(state, target) {
  if (!state.viewport) return;
  const focusBase = state.baseCenter || { x: state.baseTransform?.centerX ?? 0, y: state.baseTransform?.centerY ?? 0 };
  const targetBase = baseTargetPoint(state, target);
  const focusBlend = state.designerFocusNodeId && state.baseNodePositionById?.get(state.designerFocusNodeId)
    ? state.baseNodePositionById.get(state.designerFocusNodeId)
    : focusBase;
  const blendX = (focusBlend.x * 0.22) + (targetBase.x * 0.78);
  const blendY = (focusBlend.y * 0.22) + (targetBase.y * 0.78);
  const scale = state.viewport.scale ?? VIEWPORT_SCALE_DEFAULT;
  state.viewport.translateX = (state.baseCenter?.x ?? focusBase.x) - (blendX * scale);
  state.viewport.translateY = (state.baseCenter?.y ?? focusBase.y) - (blendY * scale);
  setViewportTarget(state, target);
  keepViewportInBounds(state.viewport, state.viewport.bounds || computeViewportBounds(widthHeightFromState(state).width, widthHeightFromState(state).height));
  return state.viewport;
}

function lockViewportToTarget(state, target) {
  frameViewportToTarget(state, target || null);
  state.viewport.target = target || null;
}

function clearViewportLock(state) {
  resetViewportState(state, state.lastRenderGraph || {});
  state.viewport.target = makeTarget(TARGET_KIND_FOCUS, state.designerFocusNodeId);
}

function syncViewportWithRenderers(state, options) {
  syncViewportAfterInteraction(state, options);
}

function setViewportScale(state, nextScale) {
  if (!state.viewport) return;
  const clamped = clamp(nextScale, VIEWPORT_SCALE_MIN, VIEWPORT_SCALE_MAX);
  state.viewport.scale = clamped;
  keepViewportInBounds(state.viewport, state.viewport.bounds || computeViewportBounds(widthHeightFromState(state).width, widthHeightFromState(state).height));
  return clamped;
}

function widthHeightFromState(state) {
  return {
    width: Number.parseFloat(state.options?.width) || 320,
    height: Number.parseFloat(state.options?.height) || 220,
  };
}

function applyViewportToLayer(state) {
  const viewport = state.viewport;
  if (!state.viewLayer || !viewport) return;
  const scale = viewport.scale ?? VIEWPORT_SCALE_DEFAULT;
  const layer = state.viewLayer;
  layer.x = viewport.translateX ?? 0;
  layer.y = viewport.translateY ?? 0;
  if (layer.scale && typeof layer.scale === "object") {
    if (typeof layer.scale.set === "function") {
      layer.scale.set(scale, scale);
    } else {
      layer.scale.x = scale;
      layer.scale.y = scale;
    }
  } else {
    layer.scale = { x: scale, y: scale };
  }
}

function captureViewportStateChange(state) {
  const viewport = state.viewport;
  if (!viewport) return;
  setRendererMarkers(state.root || null, state.lastRenderGraph || {}, state, state.rendererMode || "pixi");
  emitRendererEvent(state.root, "metadata-checker:local-graph-viewport-change", {
    scale: viewport.scale,
    translateX: viewport.translateX,
    translateY: viewport.translateY,
    target: formatTargetString(viewport.target || null),
  });
}

function syncViewportAfterInteraction(state, options) {
  const viewport = state.viewport;
  if (!viewport) return;
  keepViewportInBounds(viewport, computeViewportBounds(widthHeightFromState(state).width, widthHeightFromState(state).height));
  applyViewportToLayer(state);
  syncEdgeGeometries(state, options);
  syncLabelPositions(state);
  applyInteractionContextStyles(state, options);
  setRendererMarkers(state.root, state.lastRenderGraph || {}, state, state.rendererMode || "pixi");
  captureViewportStateChange(state);
}

function zoomViewportAroundPoint(state, canvasPoint, deltaScale) {
  if (!state.viewport) return;
  const width = widthHeightFromState(state).width;
  const height = widthHeightFromState(state).height;
  const basePoint = { x: canvasPoint?.x ?? width / 2, y: canvasPoint?.y ?? height / 2 };
  const beforeScale = state.viewport.scale ?? VIEWPORT_SCALE_DEFAULT;
  const beforeTranslateX = state.viewport.translateX ?? 0;
  const beforeTranslateY = state.viewport.translateY ?? 0;
  const nextScale = clamp(beforeScale * (1 + deltaScale), VIEWPORT_SCALE_MIN, VIEWPORT_SCALE_MAX);
  if (!Number.isFinite(nextScale)) return;
  const anchoredX = (basePoint.x - beforeTranslateX) / beforeScale;
  const anchoredY = (basePoint.y - beforeTranslateY) / beforeScale;
  state.viewport.translateX = basePoint.x - anchoredX * nextScale;
  state.viewport.translateY = basePoint.y - anchoredY * nextScale;
  state.viewport.scale = nextScale;
  keepViewportInBounds(state.viewport, computeViewportBounds(width, height));
  return state.viewport;
}

function panViewportBy(state, deltaX, deltaY) {
  if (!state.viewport) return;
  state.viewport.translateX = (state.viewport.translateX ?? 0) + (deltaX * VIEWPORT_EVENT_DRAG_SCALE);
  state.viewport.translateY = (state.viewport.translateY ?? 0) + (deltaY * VIEWPORT_EVENT_DRAG_SCALE);
  keepViewportInBounds(state.viewport, state.viewport.bounds || computeViewportBounds(widthHeightFromState(state).width, widthHeightFromState(state).height));
  return state.viewport;
}

function bindPixiPointerEvents(displayObject, handlers) {
  if (!displayObject) return;
  displayObject.interactive = true;
  displayObject.buttonMode = true;
  displayObject.eventMode = "static";
  displayObject.cursor = "pointer";
  if (typeof displayObject.on !== "function") {
    return;
  }
  displayObject.on("pointerover", handlers.onPointerOver);
  displayObject.on("pointerout", handlers.onPointerOut);
  displayObject.on("pointertap", handlers.onPointerTap);
  displayObject.on("click", handlers.onPointerTap);
}

function getCanvasPointFromEvent(event, state) {
  const width = Number.parseFloat(state.options?.width) || 320;
  const height = Number.parseFloat(state.options?.height) || 220;
  if (!event) {
    return { x: width / 2, y: height / 2 };
  }
  if (typeof event.offsetX === "number" && typeof event.offsetY === "number") {
    return { x: event.offsetX, y: event.offsetY };
  }
  if (typeof event.clientX === "number" && typeof event.clientY === "number") {
    return { x: event.clientX, y: event.clientY };
  }
  return { x: width / 2, y: height / 2 };
}

function bindCanvasInteraction(state, options, canvas, root) {
  if (!canvas || !state?.viewport) return;
  const listeners = state.viewportCanvasListeners || {};
  const unbind = [];
  const ensure = typeof canvas.addEventListener === "function"
    ? canvas
    : {
        listeners: {},
        addEventListener(type, handler) {
          this.listeners[type] = handler;
        },
        removeEventListener(type) {
          delete this.listeners[type];
        },
      };
  if (!canvas || !ensure) return;

  const onWheel = (event = {}) => {
    const deltaY = Number(event.deltaY) || 0;
    if (deltaY === 0) return;
    const point = getCanvasPointFromEvent(event, state);
    const deltaScale = (-deltaY / 400) * VIEWPORT_EVENT_ZOOM_STEP;
    zoomViewportAroundPoint(state, point, deltaScale);
    syncViewportAfterInteraction(state, options);
    if (typeof event.preventDefault === "function") {
      event.preventDefault();
    }
  };
  const onPointerDown = (event = {}) => {
    const point = getCanvasPointFromEvent(event, state);
    state.viewport.panning = true;
    state.viewport.pointer = { x: point.x, y: point.y };
    state.viewport.pointerMoved = false;
    state.pointerConsumedByTarget = false;
  };
  const onPointerUp = (event = {}) => {
    if (state.viewport.panning && state.viewport.pointer && !state.pointerConsumedByTarget) {
      const point = getCanvasPointFromEvent(event, state);
      const deltaX = point.x - state.viewport.pointer.x;
      const deltaY = point.y - state.viewport.pointer.y;
      if (Math.hypot(deltaX, deltaY) < 4 && state.lockedTarget) {
        state.clearLockedTarget("background_click");
        resetViewportState(state, state.lastRenderGraph || {});
        frameViewportToTarget(state, makeTarget(TARGET_KIND_FOCUS, state.designerFocusNodeId));
        syncViewportAfterInteraction(state, options);
        applyInteractionContextStyles(state, options);
        setRendererMarkers(state.root, state.lastRenderGraph || {}, state, state.rendererMode || "pixi");
      }
    }
    state.viewport.panning = false;
    state.viewport.pointer = null;
    state.pointerConsumedByTarget = false;
  };
  const onPointerMove = (event = {}) => {
    if (!state.viewport.panning || !state.viewport.pointer) return;
    const point = getCanvasPointFromEvent(event, state);
    const deltaX = point.x - state.viewport.pointer.x;
    const deltaY = point.y - state.viewport.pointer.y;
    if (Math.hypot(deltaX, deltaY) >= 4) {
      state.viewport.pointerMoved = true;
    }
    panViewportBy(state, deltaX, deltaY);
    state.viewport.pointer = point;
    syncViewportAfterInteraction(state, options);
  };
  const onDoubleClick = (event = {}) => {
    const point = getCanvasPointFromEvent(event, state);
    resetViewportState(state, state.lastRenderGraph || {});
    frameViewportToTarget(state, makeTarget(TARGET_KIND_FOCUS, state.designerFocusNodeId));
    state.lockedTarget = null;
    state.focusedGraphTarget = makeTarget(TARGET_KIND_FOCUS, state.designerFocusNodeId);
    syncViewportAfterInteraction(state, options);
    if (state.detailHost) {
      renderInteractionDetail(state.detailHost, getCurrentDetailPayload(state), options);
    }
    emitRendererEvent(state.root, "metadata-checker:local-graph-clear-lock", {
      reason: "double_click",
      scale: state.viewport.scale,
      translateX: state.viewport.translateX,
      translateY: state.viewport.translateY,
    });
  };

  ensure.addEventListener("wheel", onWheel);
  ensure.addEventListener("pointerdown", onPointerDown);
  ensure.addEventListener("pointermove", onPointerMove);
  ensure.addEventListener("pointerup", onPointerUp);
  ensure.addEventListener("pointerleave", onPointerUp);
  ensure.addEventListener("dblclick", onDoubleClick);
  if (typeof ensure.removeEventListener === "function") {
    unbind.push(() => {
      ensure.removeEventListener("wheel", onWheel);
      ensure.removeEventListener("pointerdown", onPointerDown);
      ensure.removeEventListener("pointermove", onPointerMove);
      ensure.removeEventListener("pointerup", onPointerUp);
      ensure.removeEventListener("pointerleave", onPointerUp);
      ensure.removeEventListener("dblclick", onDoubleClick);
    });
  }

  if (canvas !== ensure) {
    state.canvasElement = canvas;
  }

  const onKeyDown = (event = {}) => {
    const key = event.key === "Esc" ? "Escape" : event.key;
    if (key === "Escape") {
      const previousLocked = state.lockedTarget;
      state.lockedTarget = null;
      state.focusedGraphTarget = makeTarget(TARGET_KIND_FOCUS, state.designerFocusNodeId);
      resetViewportState(state, state.lastRenderGraph || {});
      frameViewportToTarget(state, state.focusedGraphTarget);
      syncViewportAfterInteraction(state, options);
      renderInteractionDetail(state.detailHost, getCurrentDetailPayload(state), options);
      applyInteractionContextStyles(state, options);
      setRendererMarkers(state.root, state.lastRenderGraph || {}, state, state.rendererMode || "pixi");
      emitRendererEvent(state.root, "metadata-checker:local-graph-clear-lock", {
        reason: "escape",
        target: formatTargetString(previousLocked),
        scale: state.viewport.scale,
      });
      emitRendererEvent(state.root, "metadata-checker:local-graph-viewport-reset", {
        reason: "escape",
        scale: state.viewport.scale,
        translateX: state.viewport.translateX,
        translateY: state.viewport.translateY,
      });
    }
    if (key === "0") {
      resetViewportState(state, state.lastRenderGraph || {});
      frameViewportToTarget(state, makeTarget(TARGET_KIND_FOCUS, state.designerFocusNodeId));
      syncViewportAfterInteraction(state, options);
      emitRendererEvent(state.root, "metadata-checker:local-graph-viewport-reset", {
        scale: state.viewport.scale,
      });
    }
  };
  if (root?.addEventListener) {
    if (state.viewportKeyListener) {
      root.removeEventListener?.("keydown", state.viewportKeyListener);
    }
    root.addEventListener("keydown", onKeyDown);
    state.viewportKeyListener = onKeyDown;
    unbind.push(() => root.removeEventListener?.("keydown", onKeyDown));
  }

  state.viewportCanvasListeners = {
    unbind,
    root,
  };
}

async function renderWithPixi(host, graph, state, options) {
  if (options.forceCanvas === false) {
    return false;
  }

  const runtime = await resolvePixiRuntimeFromOptions(options);
  if (!runtime || !runtime.Application) {
    host.setAttribute?.("data-metadata-checker-pixi-diagnostic-code", "PIXI_RUNTIME_UNAVAILABLE");
    return false;
  }

  try {
    const app = await createPixiApplication(runtime, options);

    state.app = app;
    state.stage = app.stage;
    state.rendererMode = "pixi";
    const canvas = app.canvas || app.view || app.renderer?.canvas || null;
    if (canvas) {
      if (!canvas.style) {
        canvas.style = {};
      }
      if (typeof canvas.addEventListener !== "function") {
        const listeners = {};
        canvas.addEventListener = (type, handler) => {
          listeners[type] = handler;
          canvas._handlers = listeners;
        };
        canvas.dispatchEvent = (event) => {
          if (event?.type && typeof listeners[event.type] === "function") {
            listeners[event.type](event);
          }
        };
      }
      canvas.className = `${canvas.className || ""} metadata-checker-graph-canvas`.trim();
      canvas.style.display = "block";
      canvas.style.width = "100%";
      canvas.style.maxWidth = "100%";
      canvas.style.height = `${Number.parseFloat(options.height) || 160}px`;
      canvas.style.maxHeight = "100%";
      canvas.style.boxSizing = "border-box";
      canvas.style.borderRadius = "7px";
      canvas.style.background = "#0f172a";
      if (!canvas.parentNode) {
        host.appendChild(canvas);
      }
      state.canvasAttached = true;
      state.canvasElement = canvas;
    }

    const nodeViews = new Map();
    const edgeViews = new Map();
    const viewGraph = graph.viewGraph || buildPixiViewGraph(graph, options);
    const displayNodes = viewGraph.nodes;
    const displayEdges = viewGraph.edges;
    const viewportTransform = computePixiViewportTransform(displayNodes, state.designerFocusNodeId, options);

    const layer = createPixiContainer(runtime);
    const edgeLayer = createPixiContainer(runtime);
    const nodeLayer = createPixiContainer(runtime);
    const labelLayer = createPixiContainer(runtime);

    if (state.stage?.addChild) {
      state.stage.addChild(layer);
      state.stage.addChild(labelLayer);
      if (typeof layer.addChild === "function") {
        layer.addChild(edgeLayer);
        layer.addChild(nodeLayer);
      }
    }

    state.viewLayer = layer;
    state.labelLayer = labelLayer;

    for (const edge of displayEdges) {
      const from = (viewGraph.nodeById || graph.nodeById || new Map()).get(edge.from)?.position;
      const to = (viewGraph.nodeById || graph.nodeById || new Map()).get(edge.to)?.position;
      if (!from || !to) {
        continue;
      }
      const edgeSprite = createPixiGraphics(runtime);
      if (!edgeSprite) {
        continue;
      }
      const lineFrom = transformPixiPosition(from, viewportTransform);
      const lineTo = transformPixiPosition(to, viewportTransform);
      paintEdgeGraphics(edgeSprite, lineFrom, lineTo, edge, viewGraph, options);
      edgeSprite.position = { x: 0, y: 0 };
      const edgeFocusHalo = createPixiGraphics(runtime);
      if (edgeFocusHalo) {
        const edgeStyle = getEdgePriorityStyle(edge);
        drawPixiLine(edgeFocusHalo, lineFrom, lineTo, edgeStyle.width, 0xf8fafc, 0.16);
        edgeFocusHalo.alpha = 0;
      }

      const handle = {
        id: edge.id,
        fromId: edge.from,
        toId: edge.to,
        edge,
        sprite: edgeSprite,
        focusHalo: edgeFocusHalo,
        onPointerOver: () => {
          state.dispatchTargetEvent(makeTarget("edge", edge.id), "hover");
        },
        onPointerOut: () => {
          state.dispatchTargetEvent(makeTarget("edge", edge.id), "hoverEnd");
        },
        onPointerTap: () => {
          state.dispatchTargetEvent(makeTarget("edge", edge.id), "click");
        },
      };
      bindPixiPointerEvents(edgeSprite, handle);
      if (typeof edgeLayer.addChild === "function") {
        edgeLayer.addChild(edgeFocusHalo);
        edgeLayer.addChild(edgeSprite);
      }
      edgeViews.set(edge.id, handle);
    }

    for (const node of displayNodes) {
      const radius = node.style.radius ?? options.nodeRadius;
      const nodeSprite = createPixiGraphics(runtime);
      if (!nodeSprite) {
        continue;
      }

      const halo = createPixiGraphics(runtime);
      const focusHaloRadius = radius + (node.aggregate ? 3 : 1.8);
      if (halo) {
        drawPixiStrokeCircle(
          halo,
          focusHaloRadius,
          node.aggregate ? 0x94a3b8 : 0x93c5fd,
          node.aggregate ? 0.42 : 0,
          node.aggregate ? 1.4 : 1.3,
        );
        halo.alpha = node.aggregate ? 0.3 : 0;
      }

      if (node.id === state.designerFocusNodeId) {
        drawPixiCircle(nodeSprite, radius + 5, 0x93c5fd, 0.16);
      }
      if (node.aggregate) {
        drawPixiStrokeCircle(nodeSprite, radius + 3, 0x94a3b8, node.style.strokeOpacity ?? 0.72, 1.1);
      }
      drawPixiCircle(nodeSprite, radius, nodeFillColor(node, state.designerFocusNodeId, graph.edges), node.style.fillOpacity ?? 1);

      const transformed = transformPixiPosition(node.position, viewportTransform);
      nodeSprite.position = transformed;
      if (halo) {
        halo.position = transformed;
      }

      if (typeof nodeLayer.addChild === "function") {
        nodeLayer.addChild(halo);
        nodeLayer.addChild(nodeSprite);
      }

      const label = createPixiText(runtime, node.visualLabel || node.label || node.id, {
        fill: node.aggregate ? 0xdbeafe : 0xf1f5f9,
        fontSize: node.aggregate ? 10 : 9,
        fontWeight: node.aggregate ? "700" : "600",
      });
      if (label && typeof labelLayer.addChild === "function") {
        label.position = {
          x: transformed.x + radius + 4,
          y: transformed.y - 1,
        };
        label.alpha = 0;
        label.eventMode = "none";
        if (label.anchor && typeof label.anchor.set === "function") {
          label.anchor.set(0, 0.5);
        }
        labelLayer.addChild(label);
      }

      const handle = {
        id: node.id,
        sprite: nodeSprite,
        halo,
        label,
        onPointerOver: () => {
          state.dispatchTargetEvent(makeTarget(node.aggregate ? TARGET_KIND_AGGREGATE : TARGET_KIND_NODE, node.id), "hover");
        },
        onPointerOut: () => {
          state.dispatchTargetEvent(makeTarget(node.aggregate ? TARGET_KIND_AGGREGATE : TARGET_KIND_NODE, node.id), "hoverEnd");
        },
        onPointerTap: () => {
          state.dispatchTargetEvent(makeTarget(node.aggregate ? TARGET_KIND_AGGREGATE : TARGET_KIND_NODE, node.id), "click");
        },
      };
      bindPixiPointerEvents(nodeSprite, handle);
      nodeViews.set(node.id, handle);
    }

    state.nodeViews = nodeViews;
    state.edgeViews = edgeViews;
    state.viewNodeById = createNodeLookup(displayNodes);
    state.viewEdgeById = createEdgeLookup(displayEdges);
    state.nodeNeighborsById = buildAdjacencyMap(displayNodes, displayEdges);
    state.lastRenderGraph = graph;

    state.lastViewGraph = viewGraph;
    resetViewportState(state, viewGraph);
    bindCanvasInteraction(state, options, canvas, host);
    frameViewportToTarget(state, makeTarget(TARGET_KIND_FOCUS, state.designerFocusNodeId));
    syncViewportAfterInteraction(state, options);
    renderInteractionDetail(state.detailHost, getCurrentDetailPayload(state), options);
    return true;
  } catch (error) {
    host.setAttribute?.("data-metadata-checker-pixi-diagnostic-code", "PIXI_RENDER_FAILED");
    host.setAttribute?.(
      "data-metadata-checker-pixi-diagnostic-message",
      String(error?.message || error || "pixi render failed").slice(0, 240),
    );
    return false;
  }
}

export function createPixiLocalGraphRenderer(rawOptions = {}) {
  const options = normalizeRendererOptions(rawOptions);
  const root = ensureRootContainer(options);
  let lastRender = null;

  const state = {
    root,
    options,
    app: null,
    stage: null,
    rendererMode: null,
    nodeById: new Map(),
    edgeById: new Map(),
    callbacks: options.callbacks,
    designerFocusNodeId: null,
    focusedGraphTarget: null,
    hoveredTarget: null,
    lockedTarget: null,
    lastRenderGraph: null,
    viewport: null,
    detailHost: null,
    canvasElement: null,
    nodeNeighborsById: new Map(),
    nodeViews: new Map(),
    edgeViews: new Map(),
    viewNodeById: new Map(),
    viewEdgeById: new Map(),
    viewportCanvasListeners: [],
    viewportKeyListener: null,
    destroy() {
      for (const unbind of state.viewportCanvasListeners || []) {
        if (typeof unbind === "function") {
          unbind();
        }
      }
      if (state.viewportKeyListener && root?.removeEventListener) {
        root.removeEventListener("keydown", state.viewportKeyListener);
      }
      state.viewportCanvasListeners = [];
      state.viewportKeyListener = null;
      state.nodeViews = new Map();
      state.edgeViews = new Map();
      state.viewNodeById = new Map();
      state.viewEdgeById = new Map();
      if (state.app?.destroy) {
        state.app.destroy(true);
      }
      state.app = null;
      state.stage = null;
      if (typeof root.replaceChildren === "function") {
        root.replaceChildren();
      } else if (Array.isArray(root.children)) {
        root.children = [];
      }
    },
    clearLockedTarget(reason = "clear") {
      if (!state.lockedTarget && !state.hoveredTarget) {
        return;
      }
      const lockedTarget = state.lockedTarget;
      state.lockedTarget = null;
      state.focusedGraphTarget = makeTarget(TARGET_KIND_FOCUS, state.designerFocusNodeId);
      renderInteractionDetail(state.detailHost, getCurrentDetailPayload(state), options);
      applyInteractionContextStyles(state, options);
      emitRendererEvent(state.root, "metadata-checker:local-graph-clear-lock", {
        reason,
        target: formatTargetString(lockedTarget),
      });
    },
    dispatchTargetEvent(target, eventType, event = {}) {
      const normalized = normalizeTargetType(target?.type) === TARGET_KIND_EDGE
        ? makeTarget(TARGET_KIND_EDGE, target?.id)
        : target;
      if (!normalized?.id) return null;
      const payloadTargetType = normalized.type;
      const data = payloadTargetType === TARGET_KIND_EDGE
        ? state.viewEdgeById.get(normalized.id) || state.edgeById.get(normalized.id)
        : state.viewNodeById.get(normalized.id) || state.nodeById.get(normalized.id);
      if (!data) return null;

      const makePayload = payloadTargetType === TARGET_KIND_EDGE
        ? buildEdgeInteractionPayload
        : buildNodeInteractionPayload;
      const payload = makePayload(data, state, {
        event: eventType,
        eventDetail: event,
      });

      if (eventType === "hover") {
        if (!isSameTarget(state.hoveredTarget, normalized)) {
          const previous = state.hoveredTarget;
          if (previous && state.callbacks.onNodeHover) {
            const previousNodeData = state.nodeById.get(previous.id) || state.viewNodeById.get(previous.id);
            if (previousNodeData && previous.type !== TARGET_KIND_EDGE) {
              state.callbacks.onNodeHover({
                ...buildNodeInteractionPayload(previousNodeData, state, {
                  event: "hoverEnd",
                  eventDetail: event,
                }),
              });
            }
          }
          if (previous && previous.type === TARGET_KIND_EDGE && state.callbacks.onEdgeHover) {
            const previousEdgeData = state.edgeById.get(previous.id) || state.viewEdgeById.get(previous.id);
            if (previousEdgeData) {
              state.callbacks.onEdgeHover({
                ...buildEdgeInteractionPayload(previousEdgeData, state, {
                  event: "hoverEnd",
                  eventDetail: event,
                }),
              });
            }
          }
          state.hoveredTarget = normalized;
        }
      } else if (eventType === "hoverEnd") {
        if (isSameTarget(state.hoveredTarget, normalized)) {
          state.hoveredTarget = null;
        }
      } else if (eventType === "click") {
        state.pointerConsumedByTarget = true;
        state.hoveredTarget = null;
        state.lockedTarget = normalized;
        state.focusedGraphTarget = normalized;
        frameViewportToTarget(state, normalized);
        emitRendererEvent(state.root, "metadata-checker:local-graph-lock", {
          kind: normalized.type,
          id: normalized.id,
          target: formatTargetString(normalized),
        });
      }

      if (eventType === "hover" || eventType === "hoverEnd") {
        if (payloadTargetType === TARGET_KIND_EDGE && state.callbacks.onEdgeHover) {
          state.callbacks.onEdgeHover(payload);
        } else if (state.callbacks.onNodeHover) {
          state.callbacks.onNodeHover(payload);
        }
      }

      if (eventType === "click") {
        if (payloadTargetType === TARGET_KIND_EDGE) {
          if (state.callbacks.onEdgeClick) {
            state.callbacks.onEdgeClick(payload);
          }
        } else if (state.callbacks.onNodeClick) {
          state.callbacks.onNodeClick(payload);
        }
      }

      if (eventType === "click" || eventType === "hover" || eventType === "hoverEnd") {
        renderInteractionDetail(state.detailHost, getCurrentDetailPayload(state), options);
        applyInteractionContextStyles(state, options);
        setRendererMarkers(state.root, state.lastRenderGraph || {}, state, state.rendererMode || "pixi");
      }
      return payload;
    },
  };

  if (options.document && root.parentNode == null && options.document.body) {
    options.document.body.appendChild(root);
  }

  async function render(input) {
    const graphInput = input || {};
    const layout = await computeLocalGraphLayout(graphInput, {
      ...options,
      layoutEngine: options.layoutEngine,
      d3Force3D: options.d3Force3D,
      iterations: options.layoutIterations ?? 72,
    });
    const focusNodeId =
      graphInput.focus_node ??
      layout.focusNodeId ??
      graphInput.focusNode ??
      graphInput.target ??
      (layout.nodes[0]?.id ?? null);

    const visualGraph = buildVisualsForGraph(
      layout.nodes || [],
      layout.edges || [],
      focusNodeId,
      options,
    );
    const nodes = visualGraph.nodes.map((node) => ({
      ...node,
      focus: node.id === focusNodeId,
    }));
    const edges = visualGraph.edges;

    state.nodeById = createNodeLookup(nodes);
    state.edgeById = createEdgeLookup(edges);
    state.nodeNeighborsById = buildAdjacencyMap(nodes, edges);
    state.designerFocusNodeId = focusNodeId;
    state.focusedGraphTarget = makeTarget(TARGET_KIND_FOCUS, focusNodeId);
    state.hoveredTarget = null;
    state.lockedTarget = null;
    state.detailHost = null;
    state.lastRenderGraph = null;
    if (state.viewportCanvasListeners?.unbind) {
      state.viewportCanvasListeners.unbind();
    }

    if (!root || typeof root.replaceChildren !== "function") {
      root.children = [];
    } else {
      root.replaceChildren();
    }
    state.lastView = createFakeHostElement("div");

    const renderedGraph = {
      ...layout,
      nodes,
      edges,
      focusNodeId,
    };
    const viewGraph = buildPixiViewGraph(renderedGraph, options);
    renderedGraph.viewGraph = viewGraph;
    state.lastViewGraph = viewGraph;

    const usedPixi = await renderWithPixi(root, { ...renderedGraph, nodeById: state.nodeById }, state, options);
    if (!usedPixi) {
      const fallbackView = createRendererElement(options, "div");
      fallbackView.setAttribute("data-metadata-checker-local-graph-render-fallback", "true");
      state.fallbackView = fallbackView;
      state.app = null;
      renderWithPixiFallback(root, renderedGraph, state, options).catch(() => {});
      root.appendChild(fallbackView);
    }
    setRendererMarkers(root, renderedGraph, focusNodeId, usedPixi ? "pixi" : "fallback");
    if (usedPixi) {
      createPixiInteractionPanel(root, { nodes: viewGraph.nodes, edges: viewGraph.edges }, state, options);
      state.lastRenderGraph = renderedGraph;
      setRendererMarkers(root, renderedGraph, state, "pixi");
      renderInteractionDetail(state.detailHost, getCurrentDetailPayload(state), options);
    }

    if (usedPixi && state.callbacks.onNodeFocus) {
      const focusNode = state.nodeById.get(focusNodeId);
      if (focusNode) {
        state.callbacks.onNodeFocus(buildNodeInteractionPayload(focusNode, state, { event: "focus" }));
      }
    }

    lastRender = {
      graph: renderedGraph,
      layout,
      ok: true,
      fallback: !usedPixi,
      renderer: usedPixi ? "pixi" : "fallback",
    };
    return lastRender;
  }

  function getState() {
    return {
      root,
      nodeById: new Map(state.nodeById),
      edgeById: new Map(state.edgeById),
      focusedNodeId: state.designerFocusNodeId,
      hoveredTarget: state.hoveredTarget,
      lockedTarget: state.lockedTarget,
      designerFocusNodeId: state.designerFocusNodeId,
      focusedGraphTarget: state.focusedGraphTarget,
      viewport: state.viewport,
      lastRender,
      options,
      nodeViews: new Map(state.nodeViews),
      edgeViews: new Map(state.edgeViews),
    };
  }

  return {
    root,
    getState,
    async render(input) {
      return render(input);
    },
    destroy() {
      state.destroy();
    },
    triggerNodeEvent(nodeId, eventType, event) {
      return state.dispatchTargetEvent({ id: nodeId, type: TARGET_KIND_NODE }, eventType, event);
    },
    triggerEdgeEvent(edgeId, eventType, event) {
      return state.dispatchTargetEvent({ id: edgeId, type: TARGET_KIND_EDGE }, eventType, event);
    },
    clearLockedTarget(reason = "clear") {
      state.clearLockedTarget(reason);
      renderInteractionDetail(state.detailHost, getCurrentDetailPayload(state), options);
      setRendererMarkers(root, state.lastRenderGraph || {}, state, state.rendererMode || "pixi");
      syncViewportAfterInteraction(state, options);
    },
    getVisibleGraphText() {
      return formatVisibleGraphText(state);
    },
    onEventHandlers() {
      return state.callbacks;
    },
    setCallbacks(nextCallbacks = {}) {
      state.callbacks = normalizeCallbacks(nextCallbacks);
      return true;
    },
  };
}

export async function renderLocalGraph(container, input, options = {}) {
  const renderer = createPixiLocalGraphRenderer({ ...options, root: container });
  return renderer.render(input);
}

export {
  DEFAULT_RENDERER_OPTIONS,
  edgeTouchesAggregate,
  formatVisibleGraphText,
  getCurrentDetailPayload,
  nodeSelectionTier,
};

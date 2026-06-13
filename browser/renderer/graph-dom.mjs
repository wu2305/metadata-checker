const MARKER_PREFIX = "data-metadata-checker-";

const TARGET_KIND_NODE = "node";
const TARGET_KIND_EDGE = "edge";
const TARGET_KIND_AGGREGATE = "aggregate";
const TARGET_KIND_FOCUS = "focus";
const TARGET_KIND_EMPTY = "empty";
const TARGET_KIND_ERROR = "error";
const TARGET_KIND_NONE = "none";
const VIEWPORT_SCALE_DEFAULT = 1;
const VIEWPORT_SCALE_PRECISION = 2;

const PIXI_TARGET_KIND_ALIAS = {
  node: TARGET_KIND_NODE,
  edge: TARGET_KIND_EDGE,
  aggregate: TARGET_KIND_AGGREGATE,
};

const EDGE_VISUAL_THEME = {
  filter: { color: "#fbbf24", width: 2.2, opacity: 0.92, rank: 4 },
  condition: { color: "#c084fc", width: 2.0, opacity: 0.88, rank: 4 },
  visibility: { color: "#a78bfa", width: 1.75, opacity: 0.80, rank: 3 },
  source: { color: "#38bdf8", width: 1.55, opacity: 0.70, rank: 2 },
  action: { color: "#34d399", width: 1.35, opacity: 0.58, rank: 1 },
  other: { color: "#64748b", width: 0.9, opacity: 0.30, rank: 0 },
};

function toString(value) {
  if (value == null) {
    return "";
  }
  return String(value);
}

function toLower(value) {
  return toString(value).trim().toLowerCase();
}

function clamp(value, min, max) {
  return Math.max(min, Math.min(max, value));
}

function normalizeTargetType(value) {
  const normalized = toLower(value);
  if (normalized === TARGET_KIND_NODE || normalized === TARGET_KIND_EDGE || normalized === TARGET_KIND_AGGREGATE || normalized === TARGET_KIND_FOCUS) {
    return normalized;
  }
  return TARGET_KIND_NODE;
}

function formatTarget(target) {
  if (!target || !target.id) {
    return TARGET_KIND_NONE;
  }
  const type = normalizeTargetType(target.type);
  return `${type}:${sanitizeLabelText(target.id)}`;
}

function parseTarget(value) {
  const raw = toString(value, "");
  const separator = raw.indexOf(":");
  if (separator <= 0) {
    return { type: TARGET_KIND_NONE, id: "" };
  }
  const type = normalizeTargetType(raw.slice(0, separator));
  const id = sanitizeLabelText(raw.slice(separator + 1));
  if (!id) {
    return { type: TARGET_KIND_NONE, id: "" };
  }
  return { type, id };
}

function isSameTarget(left, right) {
  if (!left || !right) return false;
  return left?.type === right?.type && toString(left.id) === toString(right.id);
}

function makeTarget(type, id) {
  const normalizedType = normalizeTargetType(type);
  if (id == null || id === "") return null;
  return { type: normalizedType, id: sanitizeLabelText(id) };
}

function hasToken(value, tokens) {
  const lowered = toLower(value);
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

function normalizePriorityField(value) {
  const lowered = toLower(value);
  return EDGE_VISUAL_THEME[lowered] ? lowered : "other";
}

function normalizePriority(edge) {
  const explicit = normalizePriorityField(edge?.priority);
  if (explicit !== "other") return explicit;
  const kindPriority = inferPriorityFromText(edge?.kind);
  if (kindPriority !== "other") return kindPriority;
  const summaryPriority = inferPriorityFromText(edge?.summary);
  if (summaryPriority !== "other") return summaryPriority;
  if (toLower(edge?.evidence_status) === "available") return "source";
  return "other";
}

function sanitizeLabelText(value) {
  const text = toString(value);
  if (/token|password|secret|cookie|auth|credential|api_key|apikey|cipherpassport/i.test(text)) {
    return "[sensitive]";
  }
  return text;
}

function createElementFrom(document, tag) {
  if (document && typeof document.createElement === "function") {
    return document.createElement(tag);
  }
  return {
    tagName: tag,
    style: {},
    children: [],
    attributes: {},
    className: "",
    appendChild(child) {
      this.children.push(child);
    },
    set textContent(value) {
      this._textContent = String(value);
    },
    get textContent() {
      return this._textContent ?? "";
    },
    setAttribute(name, value) {
      this.attributes[name] = String(value);
    },
    getAttribute(name) {
      return this.attributes[name] ?? null;
    },
    addEventListener() {},
    querySelector(selector) {
      if (selector === ".metadata-checker-graph-focus") {
        return this.children.find((child) =>
          toString(child.className).includes("metadata-checker-graph-focus"));
      }
      return null;
    },
    remove() {},
  };
}

function createSvgElementFrom(document, tag) {
  if (document && typeof document.createElementNS === "function") {
    return document.createElementNS("http://www.w3.org/2000/svg", tag);
  }
  return createElementFrom(document, tag);
}

function clearElement(element) {
  if (!element) return;
  if (typeof element.replaceChildren === "function") {
    element.replaceChildren();
    return;
  }
  element.children = [];
}

function createGraphDomInteractionState(layout, focusNodeId) {
  const nodeNeighborsById = new Map();
  const edgeById = new Map();
  const nodeById = new Map();
  for (const node of layout.nodes || []) {
    if (node?.id) {
      const nodeId = sanitizeLabelText(node.id);
      nodeById.set(nodeId, node);
      nodeNeighborsById.set(nodeId, new Set());
    }
  }
  for (const edge of layout.edges || []) {
    const edgeId = sanitizeLabelText(edge?.id || `${edge?.from}->${edge?.to}`);
    const from = sanitizeLabelText(edge?.from);
    const to = sanitizeLabelText(edge?.to);
    edgeById.set(edgeId, edge);
    if (!from || !to) {
      continue;
    }
    if (!nodeNeighborsById.has(from)) {
      nodeNeighborsById.set(from, new Set());
    }
    if (!nodeNeighborsById.has(to)) {
      nodeNeighborsById.set(to, new Set());
    }
    nodeNeighborsById.get(from).add(to);
    nodeNeighborsById.get(to).add(from);
  }
  return {
    focusNodeId: sanitizeLabelText(focusNodeId) || null,
    hoveredNodeId: null,
    hoveredEdgeId: null,
    lockedTarget: null,
    nodeById,
    edgeById,
    nodeNeighborsById,
    nodeElements: new Map(),
    edgeElements: new Map(),
    edgePriorityById: new Map(),
    lastOpenDetailContext: null,
    viewportScale: VIEWPORT_SCALE_DEFAULT,
  };
}

function getPayloadTargetType(payload) {
  if (!payload) return null;
  if (payload.detailType) {
    if (payload.detailType === TARGET_KIND_AGGREGATE) return TARGET_KIND_AGGREGATE;
    return payload.detailType;
  }
  if (payload.type) {
    return payload.type;
  }
  return null;
}

function getPayloadTargetId(payload) {
  if (!payload) return null;
  return payload.nodeId || payload.edgeId || payload.aggregateId || payload.from || payload.id || null;
}

function buildPayloadForTarget(state, target) {
  if (!target || !target.id) return null;
  const targetType = normalizeTargetType(target.type);
  if (targetType === TARGET_KIND_EDGE) {
    const edge = state.edgeById.get(target.id);
    if (!edge) return null;
    return buildEdgeInteractionPayload(edge);
  }
  if (targetType === TARGET_KIND_AGGREGATE) {
    const node = state.nodeById.get(target.id);
    if (!node) return null;
    return buildAggregateInteractionPayload(node);
  }
  const node = state.nodeById.get(target.id);
  if (!node) return null;
  return buildNodeInteractionPayload(node);
}

function getCurrentTargets(state) {
  const hovered = state.hoveredNodeId
    ? { type: TARGET_KIND_NODE, id: state.hoveredNodeId }
    : state.hoveredEdgeId
      ? { type: TARGET_KIND_EDGE, id: state.hoveredEdgeId }
      : null;
  const locked = state.lockedTarget;
  const focus = state.focusNodeId ? { type: TARGET_KIND_FOCUS, id: state.focusNodeId } : null;
  const active = locked || hovered || focus || null;
  return { active, hovered, locked, focus };
}

function getCurrentDetailContext(state) {
  const { active } = getCurrentTargets(state);
  if (!active) {
    return {
      kind: TARGET_KIND_EMPTY,
      context: "",
      id: "",
      kindLabel: "empty",
      type: TARGET_KIND_EMPTY,
      text: "",
    };
  }
  const payload = buildPayloadForTarget(state, active);
  if (active.type === TARGET_KIND_EDGE) {
    return {
      type: TARGET_KIND_EDGE,
      kind: TARGET_KIND_EDGE,
      id: active.id,
      text: active.id,
      context: formatTarget(active),
      payload,
      kindLabel: TARGET_KIND_EDGE,
    };
  }
  if (active.type === TARGET_KIND_NODE) {
    return {
      type: TARGET_KIND_NODE,
      kind: payload?.kind || TARGET_KIND_NODE,
      id: active.id,
      text: payload?.nodeId || active.id,
      context: formatTarget(active),
      payload,
      kindLabel: "node",
    };
  }
  if (active.type === TARGET_KIND_AGGREGATE) {
    return {
      type: TARGET_KIND_AGGREGATE,
      kind: TARGET_KIND_AGGREGATE,
      id: active.id,
      text: payload?.nodeId || active.id,
      context: formatTarget(active),
      payload,
      kindLabel: "aggregate",
    };
  }
  if (active.type === TARGET_KIND_FOCUS) {
    return {
      type: TARGET_KIND_FOCUS,
      kind: TARGET_KIND_FOCUS,
      id: active.id,
      text: active.id,
      context: `${TARGET_KIND_FOCUS}:${active.id}`,
      payload: payload || null,
      kindLabel: "focus",
    };
  }
  return {
    type: TARGET_KIND_EMPTY,
    kind: TARGET_KIND_EMPTY,
    id: "",
    text: "",
    context: "",
    payload: null,
    kindLabel: "empty",
  };
}

function getCurrentDetailKind(state) {
  const { active } = getCurrentTargets(state);
  if (!active) return TARGET_KIND_EMPTY;
  if (active.type === TARGET_KIND_FOCUS) return TARGET_KIND_FOCUS;
  if (active.type === TARGET_KIND_NODE) return TARGET_KIND_NODE;
  if (active.type === TARGET_KIND_AGGREGATE) return TARGET_KIND_AGGREGATE;
  if (active.type === TARGET_KIND_EDGE) return TARGET_KIND_EDGE;
  return TARGET_KIND_ERROR;
}

function collectInteractionContext(state) {
  const highlightNodes = new Set();
  const highlightEdges = new Set();
  if (!state) {
    return { highlightNodes, highlightEdges };
  }
  const { focusNodeId, nodeNeighborsById, edgeById, hoveredNodeId, hoveredEdgeId, lockedTarget } = state;
  const hoverTargets = [];
  if (hoveredNodeId) {
    hoverTargets.push({ type: TARGET_KIND_NODE, id: hoveredNodeId });
  }
  if (hoveredEdgeId) {
    hoverTargets.push({ type: TARGET_KIND_EDGE, id: hoveredEdgeId });
  }
  const contextTargets = [...hoverTargets];
  if (lockedTarget?.id && lockedTarget.type) {
    contextTargets.push(lockedTarget);
  }
  if (focusNodeId) {
    highlightNodes.add(focusNodeId);
    for (const edgeId of state.edgeElements.keys()) {
      const edge = edgeById.get(edgeId);
      if (!edge) continue;
      if (edge.from === focusNodeId || edge.to === focusNodeId) {
        highlightEdges.add(edgeId);
      }
    }
  }
  for (const target of contextTargets) {
    if (!target?.id) continue;
    if (target.type === TARGET_KIND_EDGE) {
      if (edgeById?.has(target.id)) {
        const edge = edgeById.get(target.id);
        highlightEdges.add(target.id);
        if (edge?.from) highlightNodes.add(sanitizeLabelText(edge.from));
        if (edge?.to) highlightNodes.add(sanitizeLabelText(edge.to));
      }
      continue;
    }
    highlightNodes.add(target.id);
    const neighbors = nodeNeighborsById?.get(target.id) || [];
    for (const neighbor of neighbors) {
      highlightNodes.add(neighbor);
    }
    for (const edgeId of state.edgeElements.keys()) {
      const edge = edgeById.get(edgeId);
      if (!edge) continue;
      const from = sanitizeLabelText(edge.from);
      const to = sanitizeLabelText(edge.to);
      if (from === target.id || to === target.id) {
        highlightEdges.add(edgeId);
      }
    }
  }
  if (contextTargets.length === 0 && state.edgeElements.size > 0 && highlightNodes.size === 0) {
    return {
      highlightNodes: new Set([focusNodeId].filter(Boolean)),
      highlightEdges,
    };
  }
  return { highlightNodes, highlightEdges };
}

function applyInteractionStyles(state, hoverDimAlpha = 0.24) {
  if (!state) {
    return;
  }
  const focusNodeId = state.focusNodeId;
  const context = collectInteractionContext(state);
  const hovering = Boolean(state.hoveredNodeId || state.hoveredEdgeId);
  const showNeighbors = Boolean(context.highlightNodes.size > 0 || context.highlightEdges.size > 0);
  for (const [nodeId, nodeElement] of state.nodeElements) {
    const circle = nodeElement.circle;
    const group = nodeElement.group;
    const isFocus = nodeId === focusNodeId;
    const isHighlighted = context.highlightNodes.has(nodeId);
    const isTwoHop = isTwoHopDepth(
      state.nodeById.get(nodeId)?.depth ?? state.nodeById.get(nodeId)?.metadata?.depth,
    );
    const baseOpacity = isFocus ? 0.96 : isTwoHop ? 0.23 : 0.74;
    const focusedOpacity = baseOpacity;
    const nextOpacity = hovering ? (isHighlighted || (showNeighbors && isFocus) ? 1 : hoverDimAlpha) : focusedOpacity;
    if (group) {
      group.setAttribute("opacity", nextOpacity.toFixed(3));
      if (isHighlighted || isFocus) {
        group.setAttribute("data-graph-node-highlight", "true");
      } else {
        group.removeAttribute?.("data-graph-node-highlight");
      }
      if (circle) {
        const nextRadius = Number.parseFloat(circle.getAttribute?.("r") ?? "0") || 0;
        const targetRadius = Number.parseFloat((isFocus ? 9.4 : isHighlighted ? Math.max(4.2, nextRadius * 1.06) : nextRadius).toFixed(3));
        circle.setAttribute("r", String(targetRadius));
      }
    }
  }

  for (const [edgeId, edgeElement] of state.edgeElements) {
    const edge = state.edgeById.get(edgeId);
    const priority = state.edgePriorityById.get(edgeId) ?? normalizePriority(edge || {});
    const theme = EDGE_VISUAL_THEME[priority] ?? EDGE_VISUAL_THEME.other;
    const from = sanitizeLabelText(edge?.from);
    const to = sanitizeLabelText(edge?.to);
    const isTwoHop = edge && (isTwoHopDepth(edge.fromDepth) || isTwoHopDepth(edge.toDepth));
    const isHighlighted = context.highlightEdges.has(edgeId);
    const isFocusChain = isFocusNeighborEdge(edge, focusNodeId);
    const baseOpacity = isTwoHop ? 0.18 : Math.max(0.22, Math.min(0.9, theme.opacity));
    const nextOpacity = hovering
      ? (isHighlighted || isFocusChain ? 1 : hoverDimAlpha)
      : baseOpacity;
    edgeElement.setAttribute("stroke-opacity", String(nextOpacity));
    edgeElement.setAttribute("stroke", theme.color);
    edgeElement.setAttribute("stroke-width", String(theme.width));
    edgeElement.setAttribute("data-edge-highlight", isHighlighted ? "true" : "false");
    if (!isHighlighted && isFocusChain) {
      edgeElement.setAttribute("data-edge-highlight", "focus-chain");
    }
  }
}

function isFocusNeighborEdge(edge, focusNodeId) {
  if (!edge || !focusNodeId) return false;
  const from = sanitizeLabelText(edge.from);
  const to = sanitizeLabelText(edge.to);
  return from === focusNodeId || to === focusNodeId;
}

function attachCallback(element, eventName, callback, payload, detailsHost, onInteractionState = null, eventNameOverride = null) {
  if (!element || typeof element.addEventListener !== "function") return;
  element.addEventListener(eventName, () => {
    const nextPayload = {
      ...payload,
      event: eventNameOverride || (eventName === "mouseover" || eventName === "mouseout" ? "hover" : "click"),
    };
    callback?.(nextPayload);
    if (typeof onInteractionState === "function") {
      onInteractionState(nextPayload, eventName);
    }
    if (eventName === "click") {
      renderDetail(detailsHost, nextPayload);
    }
  });
}

function buildNodeInteractionPayload(node) {
  const detailType = node.aggregate ? TARGET_KIND_AGGREGATE : "node";
  return {
    detailType,
    type: detailType === TARGET_KIND_AGGREGATE ? "aggregate" : "node",
    nodeId: sanitizeLabelText(node.id),
    aggregateId: detailType === TARGET_KIND_AGGREGATE ? sanitizeLabelText(node.id) : null,
    kind: node.kind,
    label: node.label,
    sourcePath: node.source_path || node.sourcePath || "",
    depth: node.depth,
    aggregate: node.aggregate === true || node.aggregate_bucket != null || node.aggregateBucket != null,
    aggregateLabel: sanitizeLabelText(node.aggregateLabel || node.aggregate_label || node.label || node.id),
    hiddenNodeCount: Number.parseInt(node.hiddenNodeCount ?? node.hidden_node_count ?? 0, 10) || 0,
    hiddenEdgeCount: Number.parseInt(node.hiddenEdgeCount ?? node.hidden_edge_count ?? 0, 10) || 0,
    isFocus: node.styleClass === "focus",
    neighborCount: node.neighborCount || 0,
    relatedPrioritySummary: node.relatedPrioritySummary || "",
  };
}

function buildEdgeInteractionPayload(edge) {
  const evidence = edge.evidence == null ? "EDGE_EVIDENCE_UNAVAILABLE" : sanitizeLabelText(edge.evidence);
  return {
    detailType: "edge",
    edgeId: sanitizeLabelText(edge.id || `${edge.from}->${edge.to}`),
    from: edge.from,
    to: edge.to,
    kind: edge.kind,
    label: edge.label,
    priority: normalizePriority(edge),
    summary: edge.summary || edge.label || edge.kind || "",
    evidenceStatus: edge.evidence_status || (edge.evidence == null ? "unavailable" : "available"),
    direction: edge.direction,
    evidence,
    diagnostics: edge.diagnostics || [],
    fromDepth: edge.fromDepth,
    toDepth: edge.toDepth,
  };
}

function buildAggregateInteractionPayload(node) {
  return {
    ...buildNodeInteractionPayload(node),
    detailType: TARGET_KIND_AGGREGATE,
    aggregate: true,
  };
}

function renderDetail(container, payload) {
  if (!container) return;
  clearElement(container);
  container.style.maxWidth = "100%";
  container.style.boxSizing = "border-box";
  container.style.overflowWrap = "anywhere";
  container.style.wordBreak = "break-word";
  container.style.overflow = "hidden";

  const row = createElementFrom(container.ownerDocument ?? container, "div");
  row.style.maxWidth = "100%";
  row.style.overflow = "hidden";
  row.style.textOverflow = "ellipsis";
  row.style.whiteSpace = "nowrap";
  row.style.overflowWrap = "anywhere";
  row.style.wordBreak = "break-word";
  const lockPrefix = payload?.locked ? "Lock · " : "";
  if (payload?.detailType === "edge") {
    const fromTo = [payload?.from, payload?.to].filter(Boolean).join(" -> ") || "";
    const evidenceText = payload?.evidenceStatus === "available" && payload?.evidence
      ? sanitizeLabelText(payload.evidence)
      : "EDGE_EVIDENCE_UNAVAILABLE";
    row.textContent = `${lockPrefix}Edge · ${sanitizeLabelText(payload?.priority || payload?.kind || "other")} · ${sanitizeLabelText(fromTo)} · ${evidenceText}`;
    container.appendChild(row);
    if (Array.isArray(payload?.diagnostics) && payload.diagnostics.length > 0) {
      const diag = createElementFrom(container.ownerDocument ?? container, "div");
      diag.textContent = sanitizeLabelText(payload.diagnostics.map((item) => item.code).join(", "));
      diag.style.display = "none";
      container.appendChild(diag);
    }
    return;
  }

  if (payload?.detailType === TARGET_KIND_AGGREGATE) {
    row.textContent = `${lockPrefix}Aggregate · ${sanitizeLabelText(payload?.shortLabel || payload?.nodeId || payload?.id || "+0")} · ${sanitizeLabelText(payload?.aggregateLabel || "context")} · +${sanitizeLabelText(payload?.hiddenNodeCount ?? 0)} hidden`;
    container.appendChild(row);
    return;
  }

  row.textContent = `${lockPrefix}Node · ${sanitizeLabelText(payload?.shortLabel || payload?.nodeId || payload?.id)} · ${sanitizeLabelText(payload?.kind || "node")} · depth ${sanitizeLabelText(payload?.depth ?? "")} · neighbors ${sanitizeLabelText(payload?.neighborCount ?? 0)}`;
  container.appendChild(row);
}

function colorForEdge(kind) {
  const value = toString(kind).toLowerCase();
  if (value.includes("read")) return "#60a5fa";
  if (value.includes("write")) return "#34d399";
  if (value.includes("condition")) return "#c084fc";
  if (value.includes("action")) return "#f59e0b";
  if (value.includes("dataflow")) return "#2dd4bf";
  return "#94a3b8";
}

function isTwoHopDepth(depth) {
  const parsed = Number.parseInt(depth, 10);
  return Number.isFinite(parsed) && parsed >= 2;
}

function extractNodeDepth(node) {
  if (node == null) return 0;
  if (node.depth != null) {
    const value = Number.parseInt(node.depth, 10);
    if (Number.isFinite(value)) {
      return value;
    }
  }
  if (node.metadata?.depth != null) {
    const value = Number.parseInt(node.metadata.depth, 10);
    if (Number.isFinite(value)) {
      return value;
    }
  }
  return 0;
}

function opacityForNodeDepth(node, focusNodeId) {
  if (node.id === focusNodeId) {
    return 0.92;
  }
  if (isTwoHopDepth(extractNodeDepth(node))) {
    return 0.18;
  }
  return 0.72;
}

function radiusForNode(node, focusNodeId) {
  if (node.id === focusNodeId) {
    return 5.8;
  }
  if (isTwoHopDepth(extractNodeDepth(node))) {
    return 3.1;
  }
  return 4.2;
}

function isTwoHopEdge(edge) {
  return isTwoHopDepth(edge.fromDepth) || isTwoHopDepth(edge.toDepth);
}

function opacityForEdge(edge, focusNodeId) {
  const theme = EDGE_VISUAL_THEME[normalizePriority(edge)] || EDGE_VISUAL_THEME.other;
  if (isTwoHopEdge(edge)) {
    const base = Number.parseFloat(theme.opacity);
    const value = Math.max(0.16, Number.parseFloat((base * 0.22).toFixed(3)));
    return Number.isFinite(value) ? value : EDGE_VISUAL_THEME.other.opacity;
  }
  if (edge.from === focusNodeId || edge.to === focusNodeId) {
    return theme.opacity;
  }
  return 0.38;
}

function colorForNode(node) {
  if (node?.styleClass === "focus") return "#f8fafc";
  const kind = toString(node?.kind).toLowerCase();
  if (kind.includes("model")) return "#60a5fa";
  if (kind.includes("component")) return "#22d3ee";
  if (kind.includes("action")) return "#f59e0b";
  return "#a78bfa";
}

function compactGraphPositions(layout, width = 360, height = 210) {
  const nodes = Array.isArray(layout.nodes) ? layout.nodes : [];
  const focusId = toString(layout.focus_node || nodes.find((node) => node.styleClass === "focus")?.id || nodes[0]?.id);
  const center = { x: width / 2, y: height / 2 };
  const byDepth = new Map();
  for (const node of nodes) {
    const depth = Math.max(0, extractNodeDepth(node));
    if (!byDepth.has(depth)) {
      byDepth.set(depth, []);
    }
    byDepth.get(depth).push(node);
  }

  const positions = new Map();
  if (focusId) {
    positions.set(focusId, center);
  }

  function placeRing(items, radiusX, radiusY, angleOffset = -Math.PI / 2) {
    const count = Math.max(1, items.length);
    items.forEach((node, index) => {
      if (positions.has(node.id)) {
        return;
      }
      const angle = angleOffset + (Math.PI * 2 * index) / count;
      positions.set(node.id, {
        x: center.x + Math.cos(angle) * radiusX,
        y: center.y + Math.sin(angle) * radiusY,
      });
    });
  }

  placeRing((byDepth.get(1) || []).filter((node) => node.id !== focusId), width * 0.23, height * 0.32);
  const outerNodes = nodes.filter((node) => !positions.has(node.id));
  placeRing(outerNodes, width * 0.42, height * 0.44, -Math.PI / 2 + Math.PI / Math.max(6, outerNodes.length || 1));
  return positions;
}

function bindSvgNodeEvents(group, node, callbacks) {
  if (!group || typeof group.addEventListener !== "function") return;
  const payload = buildNodeInteractionPayload(node);
  attachCallback(
    group,
    "click",
    callbacks.onNodeClick,
    payload,
    callbacks.detailHost,
    callbacks.onInteractionState,
    "click",
  );
  attachCallback(
    group,
    "mouseover",
    callbacks.onNodeHover,
    payload,
    callbacks.detailHost,
    callbacks.onInteractionState,
    "hover",
  );
  if (typeof callbacks.onNodeHover === "function") {
    attachCallback(
      group,
      "mouseout",
      callbacks.onNodeHover,
      payload,
      callbacks.detailHost,
      callbacks.onInteractionState,
      "hoverEnd",
    );
  }
}

function bindSvgEdgeEvents(line, edge, callbacks) {
  if (!line || typeof line.addEventListener !== "function") return;
  const payload = buildEdgeInteractionPayload(edge);
  attachCallback(
    line,
    "click",
    callbacks.onEdgeClick,
    payload,
    callbacks.detailHost,
    callbacks.onInteractionState,
    "click",
  );
  attachCallback(
    line,
    "mouseover",
    callbacks.onEdgeHover,
    payload,
    callbacks.detailHost,
    callbacks.onInteractionState,
    "hover",
  );
  if (typeof callbacks.onEdgeHover === "function") {
    attachCallback(
      line,
      "mouseout",
      callbacks.onEdgeHover,
      payload,
      callbacks.detailHost,
      callbacks.onInteractionState,
      "hoverEnd",
    );
  }
}

function buildGraphSurface(document, layout, callbacks = {}) {
  const frame = createElementFrom(document, "div");
  frame.className = "graph-panel-surface";
  frame.style.minHeight = "154px";
  frame.style.height = "100%";
  frame.style.position = "relative";
  frame.style.background = "#0f172a";
  frame.style.border = "1px solid rgba(148, 163, 184, 0.28)";
  frame.style.borderRadius = "7px";
  frame.style.overflow = "hidden";
  frame.setAttribute("data-metadata-checker-graph-surface", "svg");

  const svg = createSvgElementFrom(document, "svg");
  svg.setAttribute("class", "graph-panel-svg");
  svg.setAttribute("data-metadata-checker-graph-svg", "mounted");
  svg.setAttribute("width", "100%");
  svg.setAttribute("height", "100%");
  svg.setAttribute("role", "img");
  svg.setAttribute("aria-label", "Local metadata relationship graph");
  const viewWidth = 360;
  const viewHeight = 210;
  const positions = compactGraphPositions(layout, viewWidth, viewHeight);
  svg.setAttribute("viewBox", `0 0 ${viewWidth} ${viewHeight}`);
  svg.style.width = "100%";
  svg.style.height = "100%";
  svg.style.display = "block";

  const edgeLayer = createSvgElementFrom(document, "g");
  edgeLayer.setAttribute("class", "graph-panel-svg-edges");
  const nodeDepthById = new Map((layout.nodes || []).map((node) => [node.id, extractNodeDepth(node)]));
  for (const edge of layout.edges || []) {
    const from = positions.get(edge.from);
    const to = positions.get(edge.to);
    if (!from || !to) {
      continue;
    }
    edge.fromDepth = edge.fromDepth ?? nodeDepthById.get(edge.from);
    edge.toDepth = edge.toDepth ?? nodeDepthById.get(edge.to);
    const line = createSvgElementFrom(document, "line");
    const edgeOpacity = opacityForEdge(edge, layout.focus_node);
    const edgeId = sanitizeLabelText(edge.id || `${edge.from}->${edge.to}`);
    const edgePriority = normalizePriority(edge);
    const priorityStyle = EDGE_VISUAL_THEME[edgePriority] ?? EDGE_VISUAL_THEME.other;
    line.setAttribute("class", `graph-panel-svg-edge ${edge.edgeClass || ""}`.trim());
    line.setAttribute("data-edge-id", sanitizeLabelText(edge.id || `${edge.from}->${edge.to}`));
    line.setAttribute("data-edge-id-from", sanitizeLabelText(edge.from));
    line.setAttribute("data-edge-id-to", sanitizeLabelText(edge.to));
    line.setAttribute("data-edge-priority", edgePriority);
    line.setAttribute("data-edge-priority-rank", String(priorityStyle.rank ?? 0));
    line.setAttribute("x1", String(from.x));
    line.setAttribute("y1", String(from.y));
    line.setAttribute("x2", String(to.x));
    line.setAttribute("y2", String(to.y));
    line.setAttribute("stroke", priorityStyle.color);
    line.setAttribute("stroke-width", String(priorityStyle.width));
    line.setAttribute("stroke-opacity", edgeOpacity);
    if (callbacks.interactionState) {
      callbacks.interactionState.edgeElements.set(edgeId, line);
      callbacks.interactionState.edgePriorityById.set(edgeId, edgePriority);
    }
    if (callbacks.interactionState?.edgeById) {
      callbacks.interactionState.edgeById.set(edgeId, edge);
    }
    bindSvgEdgeEvents(line, edge, callbacks);
    edgeLayer.appendChild(line);
  }

  const nodeLayer = createSvgElementFrom(document, "g");
  nodeLayer.setAttribute("class", "graph-panel-svg-nodes");
  for (const node of layout.nodes || []) {
    const position = positions.get(node.id) || { x: viewWidth / 2, y: viewHeight / 2 };
    const nodeId = sanitizeLabelText(node.id);
    const group = createSvgElementFrom(document, "g");
    group.setAttribute("class", `graph-panel-svg-node graph-panel-svg-node-${node.styleClass || "normal"}`);
    group.setAttribute("data-node-id", nodeId);
    group.setAttribute("data-node-depth", String(extractNodeDepth(node)));
    group.setAttribute("transform", `translate(${position.x} ${position.y})`);
    group.setAttribute("opacity", opacityForNodeDepth(node, layout.focus_node));
    const circle = createSvgElementFrom(document, "circle");
    if (callbacks.interactionState) {
      callbacks.interactionState.nodeElements.set(nodeId, {
        group,
        circle,
      });
      callbacks.interactionState.nodeById.set(nodeId, node);
    }
    circle.setAttribute("r", radiusForNode(node, layout.focus_node));
    circle.setAttribute("fill", colorForNode(node));
    circle.setAttribute("stroke", node.styleClass === "focus" ? "#bae6fd" : "#0f172a");
    circle.setAttribute("stroke-width", node.styleClass === "focus" ? "1.7" : "1");
    bindSvgNodeEvents(group, node, callbacks);
    group.appendChild(circle);

    if (node.styleClass === "focus" || Number.parseInt(node.depth, 10) <= 1) {
      const label = createSvgElementFrom(document, "text");
      label.setAttribute("x", node.styleClass === "focus" ? "13" : "11");
      label.setAttribute("y", "4");
      label.setAttribute("fill", "#e5e7eb");
      label.setAttribute("font-size", "9");
      label.setAttribute("paint-order", "stroke");
      label.setAttribute("stroke", "#101520");
      label.setAttribute("stroke-width", "3");
      label.textContent = sanitizeLabelText(node.visualLabel || node.label || node.id).slice(0, 24);
      group.appendChild(label);
    }

    nodeLayer.appendChild(group);
  }

  svg.appendChild(edgeLayer);
  svg.appendChild(nodeLayer);
  frame.appendChild(svg);
  return frame;
}

function buildNodeRow(document, node, callbacks) {
  const row = createElementFrom(document, "li");
  row.className = `graph-node graph-node-${node.styleClass || "normal"}`;
  row.setAttribute("data-node-id", sanitizeLabelText(node.id));
  if (node.depth != null) {
    row.setAttribute("data-node-depth", String(node.depth));
  }
  row.textContent = `${sanitizeLabelText(node.visualLabel || node.label)} (${sanitizeLabelText(node.kind || "node")})`;
  const payload = buildNodeInteractionPayload(node);
  attachCallback(row, "click", callbacks.onNodeClick, payload, callbacks.detailHost, null, "click");
  attachCallback(
    row,
    "mouseover",
    callbacks.onNodeHover,
    payload,
    callbacks.detailHost,
    null,
    "hover",
  );
  attachCallback(
    row,
    "mouseout",
    callbacks.onNodeHover,
    payload,
    callbacks.detailHost,
    null,
    "hoverEnd",
  );
  if (callbacks.onExpand && (node?.collapsed || node?.expandable)) {
    const expandButton = createElementFrom(document, "button");
    expandButton.className = "graph-node-expand";
    expandButton.type = "button";
    expandButton.textContent = "Expand";
    expandButton.addEventListener?.("click", (event) => {
      event?.stopPropagation?.();
      callbacks.onExpand?.(node);
    });
    row.appendChild(expandButton);
  }
  return row;
}

function buildEdgeRow(document, edge, callbacks) {
  const row = createElementFrom(document, "li");
  row.className = "graph-edge";
  const payload = buildEdgeInteractionPayload(edge);
  row.textContent = `${sanitizeLabelText(edge.from)} → ${sanitizeLabelText(edge.to)} : ${sanitizeLabelText(edge.summary || edge.label || edge.kind)} [${sanitizeLabelText(payload.priority || "other")}/${sanitizeLabelText(payload.evidenceStatus || "unavailable")}]`;
  attachCallback(row, "click", callbacks.onEdgeClick, payload, callbacks.detailHost, null, "click");
  attachCallback(
    row,
    "mouseover",
    callbacks.onEdgeHover,
    payload,
    callbacks.detailHost,
    null,
    "hover",
  );
  attachCallback(
    row,
    "mouseout",
    callbacks.onEdgeHover,
    payload,
    callbacks.detailHost,
    null,
    "hoverEnd",
  );
  return row;
}

function buildList(document, titleText, items, buildItem, callbacks) {
  const block = createElementFrom(document, "details");
  block.className = "metadata-checker-graph-list";
  const title = createElementFrom(document, "summary");
  title.textContent = `${titleText} (${items.length})`;
  block.appendChild(title);
  const list = createElementFrom(document, "ul");
  for (const item of items.slice(0, 24)) {
    list.appendChild(buildItem(item, callbacks));
  }
  if (items.length > 24) {
    const more = createElementFrom(document, "li");
    more.textContent = `${items.length - 24} more hidden`;
    list.appendChild(more);
  }
  block.appendChild(list);
  return block;
}

function buildTruncatedNotice(document, layout) {
  if (!layout?.truncated) {
    return null;
  }

  const notice = createElementFrom(document, "div");
  notice.className = "graph-truncated-notice";
  const reason = sanitizeLabelText(layout.truncatedReason || "max_limits_reached");
  notice.textContent = `Truncated: ${reason}`;
  return notice;
}

function buildDiagnosticsBlock(document, layout) {
  const diagnostics = Array.isArray(layout?.diagnostics) ? layout.diagnostics : [];
  if (diagnostics.length === 0) {
    return null;
  }

  const block = createElementFrom(document, "section");
  block.className = "metadata-checker-graph-diagnostics";
  const title = createElementFrom(document, "h4");
  title.textContent = `Diagnostics (${diagnostics.length})`;
  block.appendChild(title);

  const list = createElementFrom(document, "ul");
  for (const item of diagnostics) {
    const row = createElementFrom(document, "li");
    row.className = "metadata-checker-graph-diagnostic";
    const code = sanitizeLabelText(item?.code || "DIAG");
    const message = sanitizeLabelText(item?.message || "");
    row.textContent = `${code}: ${message}`.trim();
    list.appendChild(row);
  }
  block.appendChild(list);

  return block;
}

export function setGraphMarkers(
  panel,
  {
    nodeCount,
    edgeCount,
    focus,
    truncated,
    depth,
    renderer,
    visibleHop,
    truncatedReason,
  } = {},
) {
  if (!panel || typeof panel.setAttribute !== "function") return;
  panel.setAttribute(`${MARKER_PREFIX}graph-panel`, "mounted");
  panel.setAttribute(`${MARKER_PREFIX}graph-nodes`, String(nodeCount ?? 0));
  panel.setAttribute(`${MARKER_PREFIX}graph-edges`, String(edgeCount ?? 0));
  panel.setAttribute(`${MARKER_PREFIX}graph-focus`, sanitizeLabelText(focus ?? ""));
  panel.setAttribute(
    `${MARKER_PREFIX}graph-truncated`,
    truncated ? "true" : "false",
  );
  if (truncatedReason) {
    panel.setAttribute(
      `${MARKER_PREFIX}graph-truncated-reason`,
      sanitizeLabelText(truncatedReason),
    );
  }
  panel.setAttribute(`${MARKER_PREFIX}graph-depth`, String(depth ?? 0));
  panel.setAttribute(`${MARKER_PREFIX}graph-visible-hop`, String(visibleHop ?? 1));
  panel.setAttribute(`${MARKER_PREFIX}graph-renderer`, toString(renderer ?? "html"));
  panel.setAttribute("data-metadata-checker-renderer", toString(renderer ?? "html"));
  panel.setAttribute("data-metadata-checker-local-graph-renderer", toString(renderer ?? "html"));
}

function setInteractionMarkers(panel, interactionState) {
  if (!panel || typeof panel.setAttribute !== "function" || !interactionState) return;
  const { active, hovered, locked } = getCurrentTargets(interactionState);
  const detailContext = getCurrentDetailContext(interactionState);
  const detailKind = getCurrentDetailKind(interactionState);
  const context = collectInteractionContext(interactionState);
  panel.setAttribute(
    `${MARKER_PREFIX}graph-hover-target`,
    hovered ? formatTarget(hovered) : TARGET_KIND_NONE,
  );
  panel.setAttribute(
    `${MARKER_PREFIX}graph-locked-target`,
    locked ? formatTarget(locked) : TARGET_KIND_NONE,
  );
  panel.setAttribute(`${MARKER_PREFIX}graph-open-detail-context`, detailContext.context || "");
  panel.setAttribute(`${MARKER_PREFIX}graph-detail-kind`, detailKind);
  panel.setAttribute(`${MARKER_PREFIX}graph-highlight-node-count`, String(context.highlightNodes.size ?? 0));
  panel.setAttribute(`${MARKER_PREFIX}graph-highlight-edge-count`, String(context.highlightEdges.size ?? 0));
  panel.setAttribute(
    `${MARKER_PREFIX}graph-viewport-scale`,
    String(Number((interactionState.viewportScale ?? VIEWPORT_SCALE_DEFAULT).toFixed(VIEWPORT_SCALE_PRECISION))),
  );
  panel.setAttribute("data-metadata-checker-graph-open-detail", detailKind);
}

export function renderGraphPanelDOM(root, layout, options = {}) {
  const document = options.document || globalThis.document;
  if (!document || !root) return null;

  clearElement(root);
  const hoverDimAlpha = Number.parseFloat(options.hoverDimAlpha) || 0.24;
  const interactionState = createGraphDomInteractionState(layout, layout.focus_node);

  const detailHost = createElementFrom(document, "div");

  function buildFocusPayloadFromState(state) {
    if (!state.focusNodeId) {
      return {
        detailType: TARGET_KIND_FOCUS,
        nodeId: "",
        kind: TARGET_KIND_FOCUS,
        type: TARGET_KIND_FOCUS,
        id: "",
        depth: "",
        isFocus: true,
        neighborCount: 0,
        relatedPrioritySummary: "none",
      };
    }
    return {
      detailType: TARGET_KIND_FOCUS,
      nodeId: state.focusNodeId,
      kind: TARGET_KIND_FOCUS,
      type: TARGET_KIND_FOCUS,
      id: state.focusNodeId,
      depth: "",
      isFocus: true,
      neighborCount: 0,
      relatedPrioritySummary: "none",
    };
  }

  function resolveTargetFromPayload(payload) {
    const resolvedType = normalizeTargetType(getPayloadTargetType(payload));
    const resolvedId = getPayloadTargetId(payload);
    if (!resolvedType || !resolvedId) {
      return null;
    }
    return makeTarget(resolvedType, resolvedId);
  }

  function refreshInteractionState() {
    const active = getCurrentTargets(interactionState).active;
    let payload = buildPayloadForTarget(interactionState, active);
    if (active?.type === TARGET_KIND_FOCUS || !payload) {
      payload = buildFocusPayloadFromState(interactionState);
    }
    if (payload) {
      renderDetail(detailHost, payload);
    }
    applyInteractionStyles(interactionState, hoverDimAlpha);
    interactionState.lastOpenDetailContext = getCurrentDetailContext(interactionState);
    setInteractionMarkers(root, interactionState);
    return interactionState.lastOpenDetailContext;
  }

  function handleInteractionState(payload = {}, eventName) {
    const eventType = payload?.event || (eventName === "mouseout" ? "hoverEnd" : "hover");
    const resolved = resolveTargetFromPayload(payload);
    const resolvedType = resolved?.type;
    const resolvedId = resolved?.id;

    if (eventType === "hover") {
      if (resolvedType === TARGET_KIND_EDGE) {
        interactionState.hoveredEdgeId = resolvedId;
        interactionState.hoveredNodeId = null;
      } else if (resolvedType === TARGET_KIND_NODE || resolvedType === TARGET_KIND_AGGREGATE) {
        interactionState.hoveredNodeId = resolvedId;
        interactionState.hoveredEdgeId = null;
      }
    }

    if (eventType === "hoverEnd") {
      if (resolvedType === TARGET_KIND_EDGE && interactionState.hoveredEdgeId === resolvedId) {
        interactionState.hoveredEdgeId = null;
      }
      if (
        (resolvedType === TARGET_KIND_NODE || resolvedType === TARGET_KIND_AGGREGATE)
        && interactionState.hoveredNodeId === resolvedId
      ) {
        interactionState.hoveredNodeId = null;
      }
    }

    if (eventType === "click" && resolvedType && resolvedId) {
      interactionState.lockedTarget = resolved;
    }

    refreshInteractionState();
  }

  function setViewportScale(nextScale) {
    const value = Number.parseFloat(nextScale);
    if (!Number.isFinite(value)) {
      return;
    }
    interactionState.viewportScale = clamp(value, 0.75, 2.4);
    setInteractionMarkers(root, interactionState);
    return interactionState.viewportScale;
  }

  function resetViewportScale() {
    interactionState.viewportScale = VIEWPORT_SCALE_DEFAULT;
    setInteractionMarkers(root, interactionState);
    return interactionState.viewportScale;
  }

  function lockCurrentTarget(payload) {
    const resolved = resolveTargetFromPayload(payload);
    if (resolved?.type && resolved?.id) {
      interactionState.lockedTarget = resolved;
    }
    if (!interactionState.lockedTarget) {
      const active = getCurrentTargets(interactionState).active;
      if (active?.type && active?.id) {
        interactionState.lockedTarget = active;
      }
    }
    refreshInteractionState();
    return interactionState.lockedTarget;
  }

  function clearLockedTarget() {
    interactionState.lockedTarget = null;
    refreshInteractionState();
    return null;
  }

  function getOpenDetailContext() {
    const context = getCurrentDetailContext(interactionState);
    interactionState.lastOpenDetailContext = context;
    return context;
  }

  function getInteractionState() {
    const { active, hovered, locked } = getCurrentTargets(interactionState);
    return {
      focusNodeId: interactionState.focusNodeId,
      hoveredNodeId: interactionState.hoveredNodeId,
      hoveredEdgeId: interactionState.hoveredEdgeId,
      hoveredTarget: hovered,
      lockedTarget: locked,
      activeTarget: active,
      hoverTarget: hovered ? formatTarget(hovered) : TARGET_KIND_NONE,
      lockedTargetString: locked ? formatTarget(locked) : TARGET_KIND_NONE,
      openDetailContext: getCurrentDetailContext(interactionState),
      viewportScale: interactionState.viewportScale,
    };
  }

  const callbacks = {
    onNodeClick: options.onNodeClick,
    onNodeHover: options.onNodeHover,
    onEdgeClick: options.onEdgeClick,
    onEdgeHover: options.onEdgeHover,
    onExpand: options.onExpand,
    detailHost,
    interactionState,
    onInteractionState: handleInteractionState,
  };

  setGraphMarkers(root, {
    nodeCount: layout.nodeCount ?? 0,
    edgeCount: layout.edgeCount ?? 0,
    focus: layout.focus_node,
    truncated: Boolean(layout.truncated),
    truncatedReason: layout.truncatedReason,
    depth: layout.depth ?? 0,
    renderer: options.renderer,
    visibleHop: layout.visible_hop ?? layout.visibleHop ?? 1,
  });

  const content = createElementFrom(document, "div");
  content.className = "metadata-checker-graph-content";
  content.style.position = "relative";
  content.style.width = "100%";
  content.style.height = "100%";
  content.style.minHeight = "0";
  content.style.boxSizing = "border-box";

  const graphSurface = buildGraphSurface(document, layout, callbacks);
  const lists = createElementFrom(document, "div");
  lists.className = "metadata-checker-graph-lists";
  lists.style.display = "none";

  const nodeBlock = buildList(document, "Nodes", layout.nodes || [], (node) =>
    buildNodeRow(document, node, callbacks),
    callbacks,
  );
  const edgeBlock = buildList(document, "Edges", layout.edges || [], (edge) =>
    buildEdgeRow(document, edge, callbacks),
    callbacks,
  );
  const detailBlock = createElementFrom(document, "section");
  detailBlock.className = "metadata-checker-graph-detail";
  detailHost.className = "metadata-checker-graph-detail-content";
  detailBlock.style.position = "absolute";
  detailBlock.style.left = "8px";
  detailBlock.style.right = "8px";
  detailBlock.style.bottom = "8px";
  detailBlock.style.maxHeight = "28px";
  detailBlock.style.overflow = "hidden";
  detailBlock.style.border = "1px solid rgba(125, 211, 252, 0.18)";
  detailBlock.style.borderRadius = "6px";
  detailBlock.style.background = "rgba(15, 23, 42, 0.72)";
  detailBlock.style.color = "#dbeafe";
  detailBlock.style.fontSize = "11px";
  detailBlock.style.lineHeight = "18px";
  detailBlock.style.padding = "3px 6px";
  detailBlock.style.boxSizing = "border-box";
  detailBlock.style.pointerEvents = "none";

  detailBlock.appendChild(detailHost);

  lists.appendChild(nodeBlock);
  lists.appendChild(edgeBlock);
  lists.appendChild(detailBlock);
  const truncatedNotice = buildTruncatedNotice(document, layout);
  if (truncatedNotice) {
    content.appendChild(truncatedNotice);
  }
  const diagnosticsBlock = buildDiagnosticsBlock(document, layout);
  if (diagnosticsBlock) {
    content.appendChild(diagnosticsBlock);
  }
  content.appendChild(graphSurface);
  content.appendChild(detailBlock);
  content.appendChild(lists);

  root.appendChild(content);
  refreshInteractionState();

  return {
    root,
    getInteractionState,
    getOpenDetailContext,
    lockCurrentTarget,
    clearLockedTarget,
    setViewportScale,
    resetViewportScale,
  };
}

export const DEFAULT_RENDER_OPTIONS = {
  maxDepth: 3,
  maxNodes: 120,
  maxEdges: 360,
  nodeLabelMaxLength: 56,
  edgeLabelMaxLength: 48,
};

const SENSITIVE_KEYWORDS = ["token", "cookie", "password", "cipherpassport"];

function safeToString(value) {
  if (value == null) return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function sanitizeLabel(value, maxLen) {
  let valueString = safeToString(value);
  if (containsSensitive(valueString)) {
    return "[sensitive]";
  }
  valueString = valueString.replace(/\r\n/g, " ").replace(/\n/g, " ");
  if (valueString.length <= maxLen) return valueString;
  const keep = Math.max(8, maxLen - 5);
  return `${valueString.slice(0, keep)} ...`;
}

function containsSensitive(value) {
  return SENSITIVE_KEYWORDS.some((keyword) =>
    value.toLowerCase().includes(keyword.toLowerCase())
  );
}

function hasSensitiveKey(key, value) {
  if (containsSensitive(key)) return true;
  if (containsSensitive(safeToString(value))) return true;
  return false;
}

function normalizeDepthFromMetadata(rawDepth) {
  const parsedDepth = Number.parseInt(rawDepth, 10);
  if (!Number.isFinite(parsedDepth) || parsedDepth < 0) return null;
  return parsedDepth;
}

function normalizeNodeMetadataDepth(nodeMetadata) {
  if (!nodeMetadata || typeof nodeMetadata !== "object") {
    return null;
  }
  if (nodeMetadata.depth != null) {
    return normalizeDepthFromMetadata(nodeMetadata.depth);
  }
  if (nodeMetadata.graph_depth != null) {
    return normalizeDepthFromMetadata(nodeMetadata.graph_depth);
  }
  return null;
}

function normalizeEdgeKind(kindValue) {
  const value = safeToString(kindValue).toLowerCase();
  if (value.includes("read")) return "reads";
  if (value.includes("write") || value.includes("set")) return "writes";
  if (value.includes("condition")) return "condition";
  if (value.includes("trigger") || value.includes("action")) return "action";
  if (value.includes("alias")) return "alias";
  if (value.includes("dataflow") || value.includes("data_flow")) return "dataflow";
  return "other";
}

function shouldTruncateNodeLabel(node, maxLen) {
  if (node == null || typeof node !== "object") return false;
  if (node.collapsed) return true;
  if (containsSensitive(node.id) || containsSensitive(node.label)) return true;
  return safeToString(node.label).length > maxLen;
}

function isNodeCollapsed(node, maxDepth, sourceDepth) {
  const metadata = node?.metadata || {};
  if (metadata.collapsed === true || metadata.collapsed === "true") return true;
  if (metadata.depth != null) {
    const depth = normalizeDepthFromMetadata(metadata.depth);
    if (depth != null && depth > maxDepth) return true;
  }
  if (sourceDepth != null && sourceDepth > maxDepth) return true;
  return false;
}

function isNodeExpandable(node) {
  const metadata = node?.metadata || {};
  return (
    metadata.collapsed === true ||
    metadata.collapsed === "true" ||
    metadata.expandable === true ||
    metadata.expandable === "true"
  );
}

function pickFocus(graph) {
  if (typeof graph.focus_node === "string" && graph.focus_node.trim()) return graph.focus_node;
  if (Array.isArray(graph.nodes) && graph.nodes.length > 0) {
    return safeToString(graph.nodes[0].id);
  }
  return null;
}

function parseIntSafe(value, fallback = 0) {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : fallback;
}

export function makeSafeDiagnostic(diag) {
  const safe = {
    severity: safeToString(diag?.severity) || "warning",
    code: safeToString(diag?.code),
    message: sanitizeLabel(diag?.message, 140),
    location: diag?.location ?? {},
  };

  if (safe.code && hasSensitiveKey("code", safe.code)) {
    safe.code = "[sensitive]";
  }
  if (safe.message && hasSensitiveKey("message", safe.message)) {
    safe.message = "[sensitive]";
  }

  return safe;
}

export function normalizeVisualGraphEnvelope(envelope) {
  if (!envelope || typeof envelope !== "object") {
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
      source: "diagnostic",
      metadata: { depth: 1 },
      maxDepth: 1,
    };
  }

  const visualGraph = envelopeToVisualGraph(envelope);
  const normalized = normalizeNodesEdges(visualGraph);
  const depth = computeGraphDepth(normalized);
  return {
    ...normalized,
    source: "analysis-envelope",
    metadata: {
      depth,
      rawDepth: parseIntSafe(envelope.depth, depth),
      status: safeToString(envelope.status) || "unknown",
      target: safeToString(envelope.target ?? null),
    },
    maxDepth: parseIntSafe(envelope.depth, depth),
  };
}

function envelopeToVisualGraph(envelope) {
  if (isVisualGraphLike(envelope)) {
    return envelope;
  }

  if (safeToString(envelope.status).toLowerCase() === "error") {
    return {
      nodes: [],
      edges: [],
      groups: [],
      focus_node: safeToString(envelope.target ?? null),
      diagnostics: Array.isArray(envelope.diagnostics) ? envelope.diagnostics : [],
      truncated: false,
      source_summary: {
        total_nodes: 0,
        total_edges: 0,
        node_kinds: {},
        edge_kinds: {},
      },
    };
  }

  if (Array.isArray(envelope.items)) {
    for (const item of envelope.items) {
      const itemGraph = envelopeItemToGraph(item);
      if (itemGraph) return itemGraph;
    }
  }

  return {
    nodes: [],
    edges: [],
    groups: [],
    focus_node: safeToString(envelope.target ?? null),
    diagnostics: Array.isArray(envelope.diagnostics) ? envelope.diagnostics : [],
    truncated: false,
    source_summary: {
      total_nodes: 0,
      total_edges: 0,
      node_kinds: {},
      edge_kinds: {},
    },
  };
}

function envelopeItemToGraph(item) {
  if (!item || typeof item !== "object") return null;

  const candidates = [
    item.visual_graph,
    item.graph,
    item.visualGraph,
    item.detail?.visual_graph,
    item.detail?.graph,
    item.payload?.visual_graph,
    item.payload?.graph,
    item.result?.visual_graph,
    item.result?.graph,
    item.data?.visual_graph,
    item.data?.graph,
  ];

  for (const candidate of candidates) {
    if (isVisualGraphLike(candidate)) {
      return candidate;
    }
  }

  if (isVisualGraphLike(item.detail)) {
    return item.detail;
  }
  return null;
}

function isVisualGraphLike(value) {
  return (
    value &&
    typeof value === "object" &&
    Array.isArray(value.nodes) &&
    Array.isArray(value.edges)
  );
}

function normalizeNodesEdges(visualGraph) {
  const nodes = Array.isArray(visualGraph.nodes) ? visualGraph.nodes : [];
  const edges = Array.isArray(visualGraph.edges) ? visualGraph.edges : [];
  const groups = Array.isArray(visualGraph.groups) ? visualGraph.groups : [];
  const diagnostics = Array.isArray(visualGraph.diagnostics)
    ? visualGraph.diagnostics.map(makeSafeDiagnostic)
    : [];

  return {
    nodes: nodes.map((node) => ({
      id: safeToString(node.id),
      label: safeToString(node.label),
      kind: safeToString(node.kind),
      source_path: safeToString(node.source_path),
      metadata: node?.metadata && typeof node.metadata === "object" ? node.metadata : {},
      depth: normalizeNodeMetadataDepth(node.metadata),
      collapsed: Boolean(node.metadata?.collapsed === true || node.metadata?.collapsed === "true"),
      expandable: isNodeExpandable(node),
      importance: node?.metadata?.importance,
      expand_token:
        node?.metadata?.expand_token ?? node?.metadata?.expandToken ?? node?.metadata?.next_query,
      target: node?.metadata?.target ?? null,
    })),
    edges: edges.map((edge) => ({
      from: safeToString(edge.from),
      to: safeToString(edge.to),
      kind: normalizeEdgeKind(edge.kind),
      direction: safeToString(edge.direction || "Forward"),
      label: safeToString(edge.label),
      evidence: edge.evidence == null ? null : safeToString(edge.evidence),
      metadata: edge?.metadata && typeof edge.metadata === "object" ? edge.metadata : {},
    })),
    groups,
    focus_node: pickFocus(visualGraph),
    diagnostics,
    truncated: visualGraph.truncated === true || visualGraph.truncated === "true",
    source_summary: normalizeSourceSummary(visualGraph.source_summary, nodes.length, edges.length),
    truncatedReason: safeToString(
      visualGraph.truncated_reason ?? visualGraph.truncatedReason ?? "",
    ),
  };
};

function normalizeSourceSummary(summary, nodeCount, edgeCount) {
  const input = summary && typeof summary === "object" ? summary : {};
  return {
    total_nodes: parseIntSafe(input.total_nodes, nodeCount),
    total_edges: parseIntSafe(input.total_edges, edgeCount),
    node_kinds: input.node_kinds && typeof input.node_kinds === "object" ? input.node_kinds : {},
    edge_kinds: input.edge_kinds && typeof input.edge_kinds === "object" ? input.edge_kinds : {},
  };
}

function computeGraphDepth(normalized) {
  const focus = normalized.focus_node;
  if (!focus || !Array.isArray(normalized.nodes)) return 1;
  const adjacency = new Map();
  for (const edge of normalized.edges) {
    if (!edge || edge.from == null || edge.to == null) continue;
    if (!adjacency.has(edge.from)) adjacency.set(edge.from, []);
    if (!adjacency.has(edge.to)) adjacency.set(edge.to, []);
    adjacency.get(edge.from).push({ to: edge.to, direction: edge.direction });
    if (edge.direction === "Bidirectional") {
      adjacency.get(edge.to).push({ to: edge.from, direction: edge.direction });
    }
  }

  const visited = new Map();
  const queue = [focus];
  visited.set(focus, 0);
  let head = 0;

  while (head < queue.length) {
    const current = queue[head++];
    const currentDepth = visited.get(current);
    const neighbors = adjacency.get(current) || [];
    for (const neighbor of neighbors) {
      if (visited.has(neighbor.to)) continue;
      visited.set(neighbor.to, currentDepth + 1);
      queue.push(neighbor.to);
    }
  }

  return Math.max(...Array.from(visited.values()), 0) + 1;
}

function computeNodeDepths(normalized, options) {
  const maxDepth = parseIntSafe(options?.maxDepth, DEFAULT_RENDER_OPTIONS.maxDepth);
  const focus = normalized.focus_node;
  const adjacency = new Map();
  for (const edge of normalized.edges) {
    if (!adjacency.has(edge.from)) adjacency.set(edge.from, []);
    if (!adjacency.has(edge.to)) adjacency.set(edge.to, []);
    adjacency.get(edge.from).push({ id: edge.to, direction: edge.direction });
    if (edge.direction === "Bidirectional") {
      adjacency.get(edge.to).push({ id: edge.from, direction: edge.direction });
    }
  }

  const depths = new Map();
  const parentByNode = new Map();
  const incomingEdgeByNode = new Map();

  if (focus) {
    depths.set(focus, 0);
    const queue = [focus];
    let head = 0;

    while (head < queue.length) {
      const current = queue[head++];
      const nextDepth = (depths.get(current) ?? 0) + 1;
      for (const neighbor of adjacency.get(current) || []) {
        if (depths.has(neighbor.id)) continue;
        depths.set(neighbor.id, nextDepth);
        parentByNode.set(neighbor.id, current);
        incomingEdgeByNode.set(neighbor.id, neighbor.id);
        queue.push(neighbor.id);
      }
    }
  }

  for (const node of normalized.nodes) {
    if (depths.has(node.id)) continue;
    const explicitDepth = node.depth;
    if (explicitDepth != null) {
      depths.set(node.id, explicitDepth);
      continue;
    }
    const inferred = maxDepth + 2;
    depths.set(node.id, inferred);
  }

  const collapsed = new Set();
  const renderedNodes = [];
  for (const node of normalized.nodes) {
    const depth = depths.get(node.id) ?? maxDepth + 1;
    const depthInMeta = node.depth;
    const collapsesBeyondBudget = depth > maxDepth || (depthInMeta != null && depthInMeta > maxDepth);
    const isCollapsed = isNodeCollapsed(node, maxDepth, depthInMeta) || collapsesBeyondBudget;
    const nodeDepth = depth === null ? maxDepth + 1 : depth;
    renderedNodes.push({
      ...node,
      depth: nodeDepth,
      visualLabel: sanitizeLabel(node.label, options?.nodeLabelMaxLength ?? DEFAULT_RENDER_OPTIONS.nodeLabelMaxLength),
      collapsed: isCollapsed,
      expandable:
        isExpandableNode(node, isCollapsed) || isNodeExpandable(node),
      truncated: shouldTruncateNodeLabel(node, options?.nodeLabelMaxLength ?? DEFAULT_RENDER_OPTIONS.nodeLabelMaxLength),
    });
    if (collapsesBeyondBudget) {
      collapsed.add(node.id);
    }
  }

  const retainedEdges = [];
  const hiddenGroups = new Map();
  for (const edge of normalized.edges) {
    const fromVisible = !collapsed.has(edge.from);
    const toVisible = !collapsed.has(edge.to);
    if (fromVisible && toVisible) {
      const fromNode = renderedNodes.find((candidate) => candidate.id === edge.from);
      const toNode = renderedNodes.find((candidate) => candidate.id === edge.to);
      const fromDepth = fromNode?.depth ?? 0;
      const toDepth = toNode?.depth ?? 0;
      retainedEdges.push({
        ...edge,
        label: sanitizeLabel(edge.label, options?.edgeLabelMaxLength ?? DEFAULT_RENDER_OPTIONS.edgeLabelMaxLength),
        evidence: sanitizeLabel(edge.evidence, 100),
        fromDepth,
        toDepth,
        visible: true,
      });
      continue;
    }

    const anchor = fromVisible ? edge.from : toVisible ? edge.to : (edge.to ?? edge.from);
    if (anchor == null) continue;
    const hiddenNodes = fromVisible ? edge.to : edge.from;
    const depth = depths.get(hiddenNodes) ?? maxDepth + 1;
    if (!hiddenGroups.has(depth)) {
      hiddenGroups.set(depth, {
        id: `collapsed_depth_${depth}`,
        label: `Collapsed depth ${depth}`,
        depth,
        from: anchor,
        count: 0,
        nodeIds: [],
      });
    }
    const group = hiddenGroups.get(depth);
    group.count += 1;
    group.nodeIds.push(hiddenNodes);
  }

  const collapsedGroups = [];
  for (const group of hiddenGroups.values()) {
    const groupId = safeToString(group.id);
    const metaCount = group.count;
    const groupNode = {
      id: groupId,
      label: `${group.label} (${metaCount})`,
      kind: "group",
      source_path: "",
      metadata: {
        collapsed: true,
        expandable: true,
        depth: group.depth,
        node_ids: group.nodeIds,
      },
      depth: group.depth,
      visualLabel: sanitizeLabel(
        `${group.label} (${metaCount})`,
        options?.nodeLabelMaxLength ?? DEFAULT_RENDER_OPTIONS.nodeLabelMaxLength
      ),
      collapsed: true,
      expandable: true,
      truncated: false,
      sourceSummaryCount: metaCount,
    };
    collapsedGroups.push(groupNode);
  }

  const finalNodes = renderedNodes.filter((node) => !collapsed.has(node.id));
  for (const group of collapsedGroups) {
    finalNodes.push(group);
    retainedEdges.push({
      from: group.depth > 0 ? group.from : normalized.focus_node,
      to: group.id,
      kind: "other",
      direction: "Forward",
      label: "expand",
      evidence: null,
      metadata: { collapsed_group: true, depth: group.depth },
      fromDepth: 0,
      toDepth: group.depth,
      visible: true,
      groupLink: true,
      collapsedTarget: true,
      expandableNodeId: group.id,
    });
  }

  const visibleEdges = retainedEdges.filter((edge) => edge.visible !== false);
  const sanitizedGroups = Array.isArray(normalized.groups) ? normalized.groups : [];
  const maxRenderedNodes =
    options?.maxNodes != null ? parseIntSafe(options.maxNodes, DEFAULT_RENDER_OPTIONS.maxNodes) : normalized.nodes.length;
  const maxRenderedEdges =
    options?.maxEdges != null ? parseIntSafe(options.maxEdges, DEFAULT_RENDER_OPTIONS.maxEdges) : normalized.edges.length;

  const capped = finalNodes.slice(0, maxRenderedNodes);
  const visibleIds = new Set(capped.map((node) => node.id));
  const finalEdges = [];
  for (const edge of visibleEdges) {
    if (!visibleIds.has(edge.from) || !visibleIds.has(edge.to)) continue;
    if (finalEdges.length >= maxRenderedEdges) {
      break;
    }
    finalEdges.push(edge);
  }
  const truncatedByNodeBudget = normalized.nodes.length > capped.length;
  const truncatedByEdgeBudget = visibleEdges.length > maxRenderedEdges;
  const truncatedReasons = [];
  if (normalized.truncatedReason) {
    truncatedReasons.push(normalized.truncatedReason);
  }
  if (truncatedByNodeBudget) {
    truncatedReasons.push("max_nodes");
  }
  if (truncatedByEdgeBudget) {
    truncatedReasons.push("max_edges");
  }

  return {
    nodes: capped,
    edges: finalEdges,
    groups: sanitizedGroups,
    focus_node: normalized.focus_node,
    diagnostics: normalized.diagnostics,
    truncated:
      normalized.truncated ||
      truncatedByNodeBudget ||
      truncatedByEdgeBudget,
    truncatedReason: truncatedReasons.join(";"),
    source_summary: normalized.source_summary,
    maxDepth: parseIntSafe(computeMaxDepth(finalNodes), 1),
    nodeCount: capped.length,
    edgeCount: finalEdges.length,
    depth: maxDepth,
    focusDepth: finalNodes.find((n) => n.id === normalized.focus_node)?.depth ?? 0,
  };
}

function isExpandableNode(node, isCollapsed) {
  return (
    isCollapsed ||
    node?.expandable === true ||
    isNodeExpandable(node) ||
    Boolean(node?.metadata?.expand_token) ||
    Boolean(node?.metadata?.target)
  );
}

function computeMaxDepth(nodes) {
  return nodes.reduce((acc, node) => Math.max(acc, node.depth ?? 0), 0);
}

function assignStyle(nodeDepth, isCollapsed) {
  if (isCollapsed) {
    return "collapsed";
  }
  if (nodeDepth === 0) return "focus";
  if (nodeDepth === 1) return "depth-1";
  if (nodeDepth === 2 || nodeDepth === 3) return "depth-2-3";
  return "collapsed";
}

export function layoutGraph(visualGraph, options = {}) {
  const normalized = normalizeNodesEdges(visualGraph);
  const computed = computeNodeDepths(normalized, {
    ...DEFAULT_RENDER_OPTIONS,
    ...options,
  });
  const maxDepth = computed.depth;
  const byDepth = new Map();

  for (const node of computed.nodes) {
    const depth = node.depth ?? 0;
    if (!byDepth.has(depth)) byDepth.set(depth, []);
    byDepth.get(depth).push(node);
  }

  const positions = new Map();
  const maxYPerDepth = new Map();
  for (const [depth, nodes] of byDepth.entries()) {
    const height = nodes.length;
    const baseY = (height - 1) * 36 / 2;
    nodes.forEach((node, index) => {
      positions.set(node.id, {
        x: depth * 220,
        y: index * 72 - baseY,
      });
    });
    maxYPerDepth.set(depth, height);
  }

  const positionedNodes = computed.nodes.map((node) => ({
    ...node,
    styleClass: assignStyle(node.depth ?? 0, node.collapsed),
    position: positions.get(node.id) ?? { x: 0, y: 0 },
    isGroup: node.kind === "group",
    edgeStyleClass:
      node.depth === 0
        ? "focus"
        : node.depth === 1
          ? "focus-band"
          : "faded-band",
  }));

  const positionedEdges = computed.edges.map((edge) => ({
    ...edge,
    fromPosition: positions.get(edge.from),
    toPosition: positions.get(edge.to),
    label: edge.label,
    edgeClass:
      edge.fromDepth != null && edge.toDepth != null && edge.toDepth <= 1
        ? "depth-edge"
        : "weak-edge",
    direction: edge.direction || "Forward",
  }));

  return {
    ...computed,
    nodes: positionedNodes,
    edges: positionedEdges,
    byDepth: Object.fromEntries(byDepth.entries()),
    maxYPerDepth: Object.fromEntries(maxYPerDepth.entries()),
    maxDepth,
  };
}

export function buildMermaidText(layout) {
  const nodes = Array.isArray(layout.nodes) ? layout.nodes : [];
  const edges = Array.isArray(layout.edges) ? layout.edges : [];
  const lines = ["graph TD", ""];
  const idMap = new Map();

  nodes.forEach((node, index) => {
    const rawId = safeToString(node.id);
    const safeId = containsSensitive(rawId)
      ? `sensitive_${index}`
      : rawId.replace(/[^A-Za-z0-9_]/g, "_");
    idMap.set(rawId, safeId || `node_${index}`);
  });

  for (const node of nodes) {
    const nodeId = idMap.get(safeToString(node.id)) ?? "node";
    const label = sanitizeLabel(node.label, 40);
    if (node.styleClass === "focus") {
      lines.push(`${nodeId}[${label}]`);
    } else if (node.depth <= 1) {
      lines.push(`${nodeId}[${label}]`);
    } else {
      lines.push(`${nodeId}[${label}]`);
    }
  }
  lines.push("");
  for (const edge of edges) {
    const fromId = idMap.get(safeToString(edge.from)) ?? "source";
    const toId = idMap.get(safeToString(edge.to)) ?? "target";
    const arrow = edge.direction === "Bidirectional" ? "<-->" : "-->";
    const suffix = edge.label ? ` |${sanitizeLabel(edge.label, 20)}|` : "";
    lines.push(`${fromId} ${arrow}${suffix} ${toId}`);
  }

  return lines.join("\n");
}

export function buildEChartsOption(layout) {
  const nodes = Array.isArray(layout.nodes) ? layout.nodes : [];
  const edges = Array.isArray(layout.edges) ? layout.edges : [];
  const idMap = new Map();
  nodes.forEach((node, index) => {
    const rawId = safeToString(node.id);
    idMap.set(rawId, containsSensitive(rawId) ? `sensitive_${index}` : rawId);
  });
  return {
    series: [
      {
        type: "graph",
        layout: "none",
        roam: true,
        data: nodes.map((node) => ({
          id: idMap.get(safeToString(node.id)) ?? safeToString(node.id),
          name: sanitizeLabel(node.visualLabel ?? node.label, 44),
          category: node.styleClass,
          x: node.position?.x ?? 0,
          y: node.position?.y ?? 0,
          symbolSize: node.styleClass === "focus" ? 30 : 18,
          emphasis: {
            focus: "adjacency",
          },
          label: {
            show: true,
            formatter: sanitizeLabel(node.visualLabel ?? node.label, 44),
          },
        })),
        links: edges.map((edge) => ({
          source: idMap.get(safeToString(edge.from)) ?? sanitizeLabel(edge.from, 32),
          target: idMap.get(safeToString(edge.to)) ?? sanitizeLabel(edge.to, 32),
          value: sanitizeLabel(edge.label, 24),
          lineStyle: {
            width: edge.fromDepth != null && edge.toDepth != null && edge.toDepth <= 3 ? 2 : 1,
            opacity: edge.toDepth != null && edge.toDepth <= 1 ? 0.95 : 0.45,
          },
        })),
        lineStyle: {
          opacity: 0.85,
        },
        categories: [
          { name: "focus" },
          { name: "depth-1" },
          { name: "depth-2-3" },
          { name: "collapsed" },
        ],
      },
    ],
  };
}

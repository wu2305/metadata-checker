/**
 * Local Graph 视图过滤：去掉无 evidence 的 model reads（含历史 blanket fanout）。
 */

function toStringValue(value, fallback = "") {
  if (value == null) return fallback;
  return String(value);
}

function isModelNodeId(nodeId) {
  const text = toStringValue(nodeId).toLowerCase();
  return text.includes("model:") || text.startsWith("model/");
}

function isModelNode(nodeOrId, nodesById) {
  if (nodeOrId && typeof nodeOrId === "object") {
    const kind = toStringValue(nodeOrId.kind).toLowerCase();
    if (kind === "model" || kind.includes("model")) {
      return true;
    }
    return isModelNodeId(nodeOrId.id);
  }
  const node = nodesById?.get(nodeOrId);
  if (node) {
    return isModelNode(node);
  }
  return isModelNodeId(nodeOrId);
}

function isReadsEdge(edge) {
  const kind = toStringValue(edge?.kind).toLowerCase();
  const label = toStringValue(edge?.label).toLowerCase();
  return kind.includes("read") || label === "reads";
}

function hasEdgeEvidence(edge) {
  if (edge?.evidence_status === "available") {
    return true;
  }
  const evidence = toStringValue(edge?.evidence).trim();
  return evidence.length > 0;
}

function dropWeakModelReads(edges, nodesById) {
  return edges.filter((edge) => {
    if (!isReadsEdge(edge) || !isModelNode(edge?.to, nodesById)) {
      return true;
    }
    return hasEdgeEvidence(edge);
  });
}

function keepFocusReachable(nodes, edges, focusNodeId) {
  const reachable = new Set([focusNodeId]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const edge of edges) {
      if (reachable.has(edge.from) && !reachable.has(edge.to)) {
        reachable.add(edge.to);
        changed = true;
      }
      if (reachable.has(edge.to) && !reachable.has(edge.from)) {
        reachable.add(edge.from);
        changed = true;
      }
    }
  }
  return {
    nodes: nodes.filter((node) => reachable.has(node.id)),
    edges: edges.filter((edge) => reachable.has(edge.from) && reachable.has(edge.to)),
  };
}

export function isSoloFocusGraph(graph) {
  if (!graph || typeof graph !== "object") {
    return false;
  }
  const focusNodeId = graph.focus_node || graph.focusNodeId;
  const nodes = Array.isArray(graph.nodes) ? graph.nodes : [];
  const edges = Array.isArray(graph.edges) ? graph.edges : [];
  if (!focusNodeId || nodes.length !== 1) {
    return false;
  }
  return nodes[0]?.id === focusNodeId && edges.length === 0;
}

/**
 * 对 WASM 返回的 local graph 做视图裁剪，供 ECharts / HTML / Pixi 共用。
 */
export function applyLocalGraphView(graph) {
  if (!graph || typeof graph !== "object") {
    return graph;
  }
  const nodes = Array.isArray(graph.nodes) ? graph.nodes.map((node) => ({ ...node })) : [];
  const edges = Array.isArray(graph.edges) ? graph.edges.map((edge) => ({ ...edge })) : [];
  const focusNodeId = graph.focus_node || graph.focusNodeId || nodes[0]?.id || null;
  if (!focusNodeId) {
    return { ...graph, nodes, edges };
  }

  const nodesById = new Map(nodes.map((node) => [node.id, node]));
  const filteredEdges = dropWeakModelReads(edges, nodesById);
  const reachable = keepFocusReachable(nodes, filteredEdges, focusNodeId);
  const visibleNodes = reachable.nodes;
  const visibleEdges = reachable.edges;

  const droppedEdgeCount = edges.length - visibleEdges.length;
  const droppedNodeCount = nodes.length - visibleNodes.length;
  const diagnostics = Array.isArray(graph.diagnostics) ? [...graph.diagnostics] : [];
  if (droppedEdgeCount > 0 || droppedNodeCount > 0) {
    diagnostics.push({
      severity: "info",
      code: "LOCAL_GRAPH_VIEW_FILTERED",
      message: `Filtered local graph view: ${droppedNodeCount} nodes, ${droppedEdgeCount} edges hidden.`,
      location: {},
    });
  }

  const soloFocus = isSoloFocusGraph({
    focus_node: focusNodeId,
    nodes: visibleNodes,
    edges: visibleEdges,
  });

  return {
    ...graph,
    nodes: visibleNodes,
    edges: visibleEdges,
    groups: soloFocus ? [] : graph.groups,
    status: soloFocus && !graph.truncated ? "empty" : graph.status,
    diagnostics,
    source_summary: {
      ...(graph.source_summary && typeof graph.source_summary === "object" ? graph.source_summary : {}),
      total_nodes: visibleNodes.length,
      total_edges: visibleEdges.length,
    },
  };
}

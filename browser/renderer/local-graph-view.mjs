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

function pruneDanglingNodes(nodes, edges, focusNodeId) {
  const nodeIds = new Set(nodes.map((node) => node.id));
  const connected = new Set();
  for (const edge of edges) {
    if (nodeIds.has(edge.from)) connected.add(edge.from);
    if (nodeIds.has(edge.to)) connected.add(edge.to);
  }
  return nodes.filter((node) => node.id === focusNodeId || connected.has(node.id));
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
  const visibleNodes = pruneDanglingNodes(nodes, filteredEdges, focusNodeId);

  const droppedEdgeCount = edges.length - filteredEdges.length;
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

  return {
    ...graph,
    nodes: visibleNodes,
    edges: filteredEdges,
    diagnostics,
    source_summary: {
      ...(graph.source_summary && typeof graph.source_summary === "object" ? graph.source_summary : {}),
      total_nodes: visibleNodes.length,
      total_edges: filteredEdges.length,
    },
  };
}

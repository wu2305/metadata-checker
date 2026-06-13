const DEFAULT_FORCE_LAYOUT_OPTIONS = {
  width: 320,
  height: 220,
  iterations: 220,
  linkDistance: 140,
  linkStrength: 0.55,
  chargeStrength: -220,
  fallbackBaseRadius: 95,
  fallbackDepthStep: 72,
  fallbackSeed: 13,
  twoHopRadiusOffset: 58,
  defaultDepth: 2,
};

function safeToString(value) {
  if (value == null) return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function parseDepth(value, fallback = null) {
  if (value == null) return fallback;
  const parsed = Number.parseInt(value, 10);
  if (Number.isFinite(parsed) && parsed >= 0) return parsed;
  return fallback;
}

function normalizePriority(value) {
  const priority = safeToString(value || "other").toLowerCase();
  if (priority === "filter" || priority === "condition") return "filter";
  if (priority === "visibility" || priority === "display") return "visibility";
  if (priority === "source" || priority === "value") return "source";
  if (priority === "action" || priority === "calculation") return "action";
  return "other";
}

function priorityRank(priority) {
  const ranks = {
    filter: 5,
    visibility: 4,
    source: 3,
    action: 2,
    other: 1,
  };
  return ranks[normalizePriority(priority)] ?? 1;
}

function normalizeNode(rawNode) {
  const id = safeToString(rawNode?.id || rawNode?.nodeId || rawNode?.node_id);
  return {
    id,
    label: safeToString(rawNode?.label || rawNode?.name || id),
    kind: safeToString(rawNode?.kind || rawNode?.type || "node"),
    metadata: rawNode?.metadata && typeof rawNode.metadata === "object" ? rawNode.metadata : {},
    depthHint: parseDepth(rawNode?.depth ?? rawNode?.metadata?.depth, null),
    sourcePath: safeToString(rawNode?.source_path ?? rawNode?.sourcePath ?? ""),
    raw: rawNode,
  };
}

function normalizeEdge(rawEdge) {
  const priority = normalizePriority(rawEdge?.priority);
  return {
    from: safeToString(rawEdge?.from || rawEdge?.source || rawEdge?.source_id),
    to: safeToString(rawEdge?.to || rawEdge?.target || rawEdge?.target_id),
    kind: safeToString(rawEdge?.kind || rawEdge?.relation || rawEdge?.type || "other"),
    direction: safeToString(rawEdge?.direction || "Forward"),
    label: safeToString(rawEdge?.label || ""),
    evidence: rawEdge?.evidence == null ? null : safeToString(rawEdge.evidence),
    priority,
    priorityRank: Number.parseInt(rawEdge?.priorityRank, 10) || priorityRank(priority),
    summary: safeToString(rawEdge?.summary || rawEdge?.label || rawEdge?.kind || ""),
    evidence_status: safeToString(
      rawEdge?.evidence_status ||
      (rawEdge?.evidence == null || safeToString(rawEdge.evidence).trim() === "" ? "unavailable" : "available"),
    ),
    metadata: rawEdge?.metadata && typeof rawEdge.metadata === "object" ? rawEdge.metadata : {},
    raw: rawEdge,
  };
}

function normalizeVisualGraphInput(input) {
  if (input == null || typeof input !== "object") {
    return { focusNodeId: null, nodes: [], edges: [] };
  }

  const rawNodes = Array.isArray(input.nodes)
    ? input.nodes
    : [];
  const rawEdges = Array.isArray(input.edges)
    ? input.edges
    : [];

  return {
    focusNodeId: safeToString(
      input.focus_node ??
      input.focusNode ??
      input.target ??
      (rawNodes[0]?.id ?? null),
    ),
    nodes: rawNodes.map(normalizeNode),
    edges: rawEdges.map(normalizeEdge),
    raw: input,
  };
}

function normalizeNodeDepths(input, maxDepthHint) {
  const focusNodeId = input.focusNodeId;
  const nodeById = new Map();
  for (const node of input.nodes) {
    nodeById.set(node.id, { ...node, depth: node.depthHint ?? null });
  }

  const adjacency = new Map();
  for (const edge of input.edges) {
    if (edge.from && edge.to) {
      if (!adjacency.has(edge.from)) adjacency.set(edge.from, new Set());
      if (!adjacency.has(edge.to)) adjacency.set(edge.to, new Set());
      adjacency.get(edge.from).add(edge.to);
      adjacency.get(edge.to).add(edge.from);
    }
  }

  const depth = new Map();
  if (focusNodeId && nodeById.has(focusNodeId)) {
    depth.set(focusNodeId, 0);
    const queue = [focusNodeId];
    for (let head = 0; head < queue.length; head += 1) {
      const current = queue[head];
      const currentDepth = depth.get(current);
      const neighbors = Array.from(adjacency.get(current) ?? []);
      for (const neighbor of neighbors) {
        if (depth.has(neighbor)) continue;
        const nextDepth = currentDepth + 1;
        if (maxDepthHint != null && nextDepth > maxDepthHint) continue;
        depth.set(neighbor, nextDepth);
        queue.push(neighbor);
      }
    }
  }

  for (const node of nodeById.values()) {
    if (!depth.has(node.id)) {
      depth.set(node.id, node.depthHint ?? parseDepth(node.metadata.depth, maxDepthHint + 2));
    }
  }

  return { depth, nodeById };
}

function stableHash(text, seed) {
  let hash = seed;
  const value = safeToString(text);
  for (let index = 0; index < value.length; index += 1) {
    hash = (hash * 33) ^ value.charCodeAt(index);
  }
  return (hash >>> 0) / 4294967296;
}

function deterministicRandom(seed) {
  let state = Number.isFinite(seed) ? Number.parseInt(seed, 10) : 1;
  if (!Number.isFinite(state) || state <= 0) {
    state = 1;
  }
  return () => {
    const x = Math.sin(state++) * 10000;
    return x - Math.floor(x);
  };
}

function sortByIdAndHop(nodes, depthById) {
  return nodes
    .slice()
    .sort((left, right) => {
      const leftDepth = depthById.get(left.id) ?? 999;
      const rightDepth = depthById.get(right.id) ?? 999;
      if (leftDepth !== rightDepth) return leftDepth - rightDepth;
      return left.id.localeCompare(right.id);
    });
}

function buildFallbackPositionedNodes({ input, maxDepthHint, options }) {
  const width = Number.parseFloat(options.width) || DEFAULT_FORCE_LAYOUT_OPTIONS.width;
  const height = Number.parseFloat(options.height) || DEFAULT_FORCE_LAYOUT_OPTIONS.height;
  const centerX = width / 2;
  const centerY = height / 2;

  const normalized = normalizeNodeDepths(input, maxDepthHint);
  const depthLevels = new Map();
  for (const [nodeId, nodeDepth] of normalized.depth) {
    if (!depthLevels.has(nodeDepth)) {
      depthLevels.set(nodeDepth, []);
    }
    const node = normalized.nodeById.get(nodeId);
    if (node) {
      depthLevels.get(nodeDepth).push(node);
    }
  }

  const nodes = [];
  const radiusBase = Number.parseFloat(options.fallbackBaseRadius) || DEFAULT_FORCE_LAYOUT_OPTIONS.fallbackBaseRadius;
  const depthStep = Number.parseFloat(options.fallbackDepthStep) || DEFAULT_FORCE_LAYOUT_OPTIONS.fallbackDepthStep;
  const zGap = Number.parseFloat(options.twoHopRadiusOffset) || DEFAULT_FORCE_LAYOUT_OPTIONS.twoHopRadiusOffset;

  for (const [depth, group] of depthLevels.entries()) {
    const sorted = sortByIdAndHop(group, normalized.depth);
    if (depth === 0) {
      const focus = sorted[0];
      if (focus) {
        nodes.push({
          ...focus,
          depth,
          depthHint: depth,
          position: { x: centerX, y: centerY, z: 0 },
        });
      }
      continue;
    }

    const radius = radiusBase + (depth * depthStep) + parseDepth(options.fallbackSeed, 0);
    const count = sorted.length;
    const angleStep = (Math.PI * 2) / Math.max(1, count);
    sorted.forEach((node, index) => {
      const phase = stableHash(`${node.id}-${depth}`, options.fallbackSeed || 1);
      const angle = angleStep * index + phase;
      const x = centerX + radius * Math.cos(angle);
      const y = centerY + radius * Math.sin(angle);
      const z = depth === 1 ? 0 : depth * (zGap > 0 ? zGap : 0);
      nodes.push({ ...node, depth, depthHint: depth, position: { x, y, z } });
    });
  }

  return nodes;
}

function applyEdgeDepths(nodes, edges) {
  const depthById = new Map(nodes.map((node) => [node.id, node.depth ?? 999]));
  return edges
    .filter((edge) => edge.from && edge.to)
    .map((edge, index) => ({
      ...edge,
      id: edge.id || `${edge.from}->${edge.to}#${index}`,
      fromDepth: depthById.get(edge.from) ?? 999,
      toDepth: depthById.get(edge.to) ?? 999,
      toPosition: null,
      fromPosition: null,
      positionHint: {},
    }));
}

function bindPositionsToEdges(nodes, edges) {
  const map = new Map(nodes.map((node) => [node.id, node.position]));
  for (const edge of edges) {
    edge.fromPosition = map.get(edge.from) ?? null;
    edge.toPosition = map.get(edge.to) ?? null;
    edge.positionHint = {
      dx: (edge.toPosition?.x ?? 0) - (edge.fromPosition?.x ?? 0),
      dy: (edge.toPosition?.y ?? 0) - (edge.fromPosition?.y ?? 0),
      dz: (edge.toPosition?.z ?? 0) - (edge.fromPosition?.z ?? 0),
    };
  }
}

function normalizeD3Force3DEngine(candidate) {
  if (!candidate) return null;
  return candidate.D3Force3D ?? candidate.default ?? candidate;
}

function isLoadableEngine(candidate) {
  const engine = normalizeD3Force3DEngine(candidate);
  return (
    engine &&
    typeof engine.forceSimulation === "function" &&
    typeof engine.forceLink === "function" &&
    typeof engine.forceManyBody === "function" &&
    typeof engine.forceCenter === "function"
  );
}

async function loadD3Force3DEngine(runtimeHint) {
  const hinted = normalizeD3Force3DEngine(runtimeHint);
  if (isLoadableEngine(hinted)) {
    return hinted;
  }
  try {
    const module = await import("d3-force-3d");
    const engine = normalizeD3Force3DEngine(module);
    if (isLoadableEngine(engine)) {
      return engine;
    }
    return null;
  } catch {
    return null;
  }
}

async function runDeterministicD3Layout(input, maxDepthHint, options) {
  const force3d = await loadD3Force3DEngine(options.d3Force3D ?? options.d3Force3d ?? options.d3);
  if (!force3d) {
    return null;
  }

  const normalized = normalizeVisualGraphInput(input);
  const depthById = normalizeNodeDepths(normalized, maxDepthHint);
  const links = normalized.edges
    .filter((edge) => edge.from && edge.to)
    .map((edge, index) => ({
      ...edge,
      id: edge.id || `${edge.from}->${edge.to}#${index}`,
      source: edge.from,
      target: edge.to,
    }));
  const nodes = [];
  for (const [nodeId, baseNode] of depthById.nodeById.entries()) {
    const depth = depthById.depth.get(nodeId) ?? parseDepth(baseNode.depthHint, maxDepthHint + 1);
    nodes.push({
      ...baseNode,
      depth,
      depthHint: depth,
      vx: 0,
      vy: 0,
      vz: 0,
      x: Number.parseFloat(options.width) / 2,
      y: Number.parseFloat(options.height) / 2,
      z: 0,
    });
  }

  const edges = applyEdgeDepths(nodes, normalized.edges);

  const {
    forceSimulation,
    forceLink,
    forceManyBody,
    forceCenter,
  } = force3d;
  if (
    typeof forceSimulation !== "function" ||
    typeof forceLink !== "function" ||
    typeof forceManyBody !== "function" ||
    typeof forceCenter !== "function"
  ) {
    return null;
  }

  const nodeById = new Map(nodes.map((node) => [node.id, node]));
  const simulation = forceSimulation(nodes);
  const link = forceLink(links)
    .id((item) => item.id)
    .distance(Number.parseFloat(options.linkDistance) || DEFAULT_FORCE_LAYOUT_OPTIONS.linkDistance)
    .strength(Number.parseFloat(options.linkStrength) || DEFAULT_FORCE_LAYOUT_OPTIONS.linkStrength);

  simulation
    .force("charge", forceManyBody().strength(Number.parseFloat(options.chargeStrength) || DEFAULT_FORCE_LAYOUT_OPTIONS.chargeStrength))
    .force("link", link)
    .force("center", forceCenter(Number.parseFloat(options.width) / 2, Number.parseFloat(options.height) / 2))
    .alphaMin(0.001)
    .alphaDecay(0.022)
    .velocityDecay(0.4)
    .randomSource?.(deterministicRandom(parseFloat(options.layoutSeed ?? options.fallbackSeed ?? 13)));

  if (typeof simulation.stop !== "function") {
    return null;
  }

  simulation.stop();
  const iterations = Number.parseInt(options.iterations, 10) || DEFAULT_FORCE_LAYOUT_OPTIONS.iterations;
  for (let index = 0; index < iterations; index += 1) {
    simulation.tick();
  }

  const resolvedNodes = nodes.map((node) => {
    const depth = depthById.depth.get(node.id) ?? 0;
    if (node.id === normalized.focusNodeId) {
      node.x = Number.parseFloat(options.width) / 2;
      node.y = Number.parseFloat(options.height) / 2;
      node.z = 0;
      node.vx = 0;
      node.vy = 0;
      node.vz = 0;
    }
    return {
      ...nodeById.get(node.id),
      depth,
      depthHint: depth,
      position: {
        x: Number.isFinite(node.x) ? node.x : Number.parseFloat(options.width) / 2,
        y: Number.isFinite(node.y) ? node.y : Number.parseFloat(options.height) / 2,
        z: Number.isFinite(node.z) ? node.z : 0,
      },
    };
  });

  const resolvedEdges = applyEdgeDepths(resolvedNodes, normalized.edges);
  bindPositionsToEdges(resolvedNodes, resolvedEdges);
  return {
    nodes: resolvedNodes,
    edges: resolvedEdges,
    focusNodeId: normalized.focusNodeId,
    depthById,
    layoutEngine: "d3-force-3d",
  };
}

function runDeterministicFallbackLayout(input, maxDepthHint, options) {
  const normalized = normalizeVisualGraphInput(input);
  const depthInfo = normalizeNodeDepths(normalized, maxDepthHint);
  const fallbackNodes = buildFallbackPositionedNodes({
    input: normalized,
    maxDepthHint,
    options,
  });
  const edges = applyEdgeDepths(fallbackNodes, normalized.edges);
  bindPositionsToEdges(fallbackNodes, edges);
  return {
    nodes: fallbackNodes,
    edges,
    focusNodeId: normalized.focusNodeId,
    depthById: depthInfo,
    layoutEngine: "fallback-deterministic",
  };
}

function isFunction(candidate) {
  return typeof candidate === "function";
}

export async function computeLocalGraphLayout(input, rawOptions = {}) {
  const options = { ...DEFAULT_FORCE_LAYOUT_OPTIONS, ...rawOptions };
  const depthHint = Number.parseInt(options.maxDepth, 10);
  const maxDepth = Number.isFinite(depthHint) ? depthHint : DEFAULT_FORCE_LAYOUT_OPTIONS.defaultDepth;
  const normalizedInput = normalizeVisualGraphInput(input);
  const layoutEngine = options.layoutEngine;

  if (isFunction(layoutEngine)) {
    const prepared = {
      ...normalizedInput,
      options,
    };
    const engineResult = await layoutEngine(prepared);
    if (engineResult && Array.isArray(engineResult.nodes) && Array.isArray(engineResult.edges)) {
      const nodes = engineResult.nodes.map((node) => {
        const nodeDepth = parseDepth(node.depth, maxDepth + 1);
        return {
          ...node,
          depth: nodeDepth,
          position: node.position || { x: options.width / 2, y: options.height / 2, z: 0 },
        };
      });
      const edges = engineResult.edges.map((edge, index) => ({
        ...edge,
        id: edge.id || `${edge.from}->${edge.to}#${index}`,
      }));
      const depthById = new Map(nodes.map((node) => [node.id, node.depth]));
      const resolvedEdges = applyEdgeDepths(nodes, edges);
      bindPositionsToEdges(nodes, resolvedEdges);
      return {
        ...engineResult,
        nodes,
        edges: resolvedEdges,
        focusNodeId: normalizedInput.focusNodeId,
        depthById,
        layoutEngine: "injected",
      };
    }
  }

  const deterministicEngine = await runDeterministicD3Layout(normalizedInput, maxDepth, options);
  if (deterministicEngine) {
    return deterministicEngine;
  }

  return runDeterministicFallbackLayout(normalizedInput, maxDepth, options);
}

export function resolveNodeVisualDepth(node) {
  return parseDepth(node?.depth, 999);
}

export function isTranslucentDepth(depth) {
  return Number.parseInt(depth, 10) >= 2;
}

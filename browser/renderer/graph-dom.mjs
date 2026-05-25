const MARKER_PREFIX = "data-metadata-checker-";

function toString(value) {
  if (value == null) return "";
  return String(value);
}

function sanitizeLabelText(value) {
  const text = toString(value);
  if (/token|cookie|password|cipherpassport/i.test(text)) {
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
    querySelector() {
      return null;
    },
  };
}

function clearElement(element) {
  if (!element) return;
  if (typeof element.replaceChildren === "function") {
    element.replaceChildren();
    return;
  }
  element.children = [];
}

function buildHeader(document, layout) {
  const header = createElementFrom(document, "div");
  header.className = "graph-panel-header";

  const title = createElementFrom(document, "div");
  title.className = "graph-panel-title";
  title.textContent = "Graph Relationship";

  const focusNode = createElementFrom(document, "div");
  focusNode.className = "graph-panel-focus";
  focusNode.textContent = `Focus: ${layout.focus_node ?? "unknown"}`;

  const stats = createElementFrom(document, "div");
  stats.className = "graph-panel-stats";
  stats.textContent = `Nodes: ${layout.nodeCount ?? 0}, Edges: ${layout.edgeCount ?? 0}`;

  const depth = createElementFrom(document, "div");
  depth.className = "graph-panel-depth";
  depth.textContent = `Depth: ${layout.depth ?? 0}`;

  header.appendChild(title);
  header.appendChild(focusNode);
  header.appendChild(stats);
  header.appendChild(depth);

  return header;
}

function buildDiagnostics(document, diagnostics) {
  const section = createElementFrom(document, "section");
  section.className = "graph-panel-diagnostics";
  const title = createElementFrom(document, "h4");
  title.textContent = "Diagnostics";
  section.appendChild(title);

  if (!Array.isArray(diagnostics) || diagnostics.length === 0) {
    const empty = createElementFrom(document, "div");
    empty.textContent = "No diagnostics";
    section.appendChild(empty);
    return section;
  }

  const list = createElementFrom(document, "ul");
  for (const diag of diagnostics) {
    const li = createElementFrom(document, "li");
    li.textContent = `${diag.code ? `${diag.code}: ` : ""}${diag.message ?? ""}`;
    list.appendChild(li);
  }
  section.appendChild(list);
  return section;
}

function buildNodeElement(document, node, callbacks) {
  const row = createElementFrom(document, "li");
  row.className = `graph-node graph-node-${node.styleClass || "normal"}`;
  row.setAttribute("data-node-id", toString(node.id));
  if (node.depth != null) {
    row.setAttribute("data-node-depth", String(node.depth));
  }

  const label = createElementFrom(document, "span");
  label.className = "graph-node-label";
  label.textContent = `${toString(node.visualLabel ?? node.label)}`;

  row.appendChild(label);

  const detail = createElementFrom(document, "span");
  detail.className = "graph-node-meta";
  detail.textContent = sanitizeLabelText(node.kind || "node");
  row.appendChild(detail);

  if (node.collapsed || node.expandable) {
    const action = createElementFrom(document, "button");
    action.type = "button";
    action.className = "graph-node-expand-btn";
    action.textContent = node.collapsed ? "expand" : "expand";
    action.disabled = false;
    action.addEventListener("click", () => {
      if (typeof callbacks?.onExpand === "function") {
        callbacks.onExpand({
          type: "expand_requested",
          event: "click",
          nodeId: toString(node.id ?? node.nodeId ?? node.node_id ?? ""),
          legacyNodeId: toString(node.nodeId ?? ""),
          target: node.target || node.metadata?.target || null,
          depth: node.depth ?? null,
          expand_token: node.expand_token || node.metadata?.expand_token || null,
          collapsed: node.collapsed === true,
        });
      }
    });
    row.appendChild(action);
  }

  return row;
}

function buildEdgeElement(document, edge) {
  const row = createElementFrom(document, "li");
  row.className = "graph-edge";
  const label = sanitizeLabelText(edge.label || edge.kind || "edge");
  const from = sanitizeLabelText(edge.from);
  const to = sanitizeLabelText(edge.to);
  row.textContent = `${from} → ${to} : ${label}`;
  return row;
}

function createListBlock(document, titleText, items, buildItem) {
  const block = createElementFrom(document, "section");
  const title = createElementFrom(document, "h4");
  title.textContent = titleText;
  block.appendChild(title);
  const list = createElementFrom(document, "ul");
  for (const item of items) {
    list.appendChild(buildItem(item));
  }
  block.appendChild(list);
  return block;
}
function getBounds(nodes) {
  if (nodes.length === 0) {
    return { width: 520, height: 220 };
  }
  let minX = Infinity;
  let maxX = -Infinity;
  let minY = Infinity;
  let maxY = -Infinity;
  for (const node of nodes) {
    const pos = node.position || { x: 0, y: 0 };
    minX = Math.min(minX, pos.x);
    maxX = Math.max(maxX, pos.x);
    minY = Math.min(minY, pos.y);
    maxY = Math.max(maxY, pos.y);
  }
  return {
    width: Math.max(520, maxX - minX + 240),
    height: Math.max(220, maxY - minY + 220),
    minX,
    minY,
  };
}

function renderEdgesSVG(document, svg, layout) {
  if (!svg || !Array.isArray(layout.edges)) {
    return;
  }
  const defs = createElementFrom(document, "g");
  for (const edge of layout.edges) {
    const from = edge.fromPosition || { x: 0, y: 0 };
    const to = edge.toPosition || { x: 0, y: 0 };
    const line = createElementFrom(document, "line");
    line.setAttribute("x1", String(from.x));
    line.setAttribute("y1", String(from.y));
    line.setAttribute("x2", String(to.x));
    line.setAttribute("y2", String(to.y));
    line.setAttribute("class", `graph-edge ${edge.edgeClass || "weak-edge"}`);
    defs.appendChild(line);
  }
  svg.appendChild(defs);
}

function renderNodesSVG(document, svg, layout) {
  if (!svg || !Array.isArray(layout.nodes)) {
    return;
  }
  const defs = createElementFrom(document, "g");
  for (const node of layout.nodes) {
    const position = node.position || { x: 0, y: 0 };
    const circle = createElementFrom(document, "circle");
    circle.setAttribute("cx", String(position.x + 60));
    circle.setAttribute("cy", String(position.y + 80));
    circle.setAttribute("r", String(node.styleClass === "focus" ? 18 : 14));
    circle.setAttribute("class", `graph-node ${node.styleClass || "normal"}`);
    defs.appendChild(circle);

    const text = createElementFrom(document, "text");
    text.setAttribute("x", String(position.x + 60));
    text.setAttribute("y", String(position.y + 84));
    text.textContent = toString(node.visualLabel || node.label).slice(0, 18);
    defs.appendChild(text);
  }
  svg.appendChild(defs);
}

function buildSVGCanvas(document, layout) {
  const frame = createElementFrom(document, "div");
  frame.className = "graph-svg-frame";
  const bounds = getBounds(layout.nodes || []);

  const svg = createElementFrom(document, "svg");
  svg.setAttribute("class", "graph-svg");
  svg.setAttribute("width", String(bounds.width));
  svg.setAttribute("height", String(bounds.height));
  svg.setAttribute("viewBox", `${bounds.minX} ${bounds.minY} ${bounds.width} ${bounds.height}`);

  renderEdgesSVG(document, svg, layout);
  renderNodesSVG(document, svg, layout);

  frame.appendChild(svg);
  return frame;
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
    truncatedReason,
  },
) {
  if (!panel || typeof panel.setAttribute !== "function") return;
  panel.setAttribute(`${MARKER_PREFIX}graph-panel`, "mounted");
  panel.setAttribute(`${MARKER_PREFIX}graph-nodes`, String(nodeCount ?? 0));
  panel.setAttribute(`${MARKER_PREFIX}graph-edges`, String(edgeCount ?? 0));
  panel.setAttribute(`${MARKER_PREFIX}graph-focus`, toString(focus ?? ""));
  panel.setAttribute(
    `${MARKER_PREFIX}graph-truncated`,
    truncated ? "true" : "false"
  );
  if (truncatedReason) {
    panel.setAttribute(
      `${MARKER_PREFIX}graph-truncated-reason`,
      toString(truncatedReason),
    );
  }
  panel.setAttribute(`${MARKER_PREFIX}graph-depth`, String(depth ?? 0));
  panel.setAttribute(`${MARKER_PREFIX}graph-renderer`, toString(renderer ?? "html"));
  panel.setAttribute("data-metadata-checker-renderer", toString(renderer ?? "html"));
}

export function renderGraphPanelDOM(root, layout, options = {}) {
  const document = options.document || globalThis.document;
  if (!document || !root) return null;

  clearElement(root);

  setGraphMarkers(root, {
    nodeCount: layout.nodeCount ?? 0,
    edgeCount: layout.edgeCount ?? 0,
    focus: layout.focus_node,
    truncated: Boolean(layout.truncated),
    truncatedReason: layout.truncatedReason,
    depth: layout.depth ?? 0,
    renderer: options.renderer,
  });

  const header = buildHeader(document, layout);
  const truncatedMessage = createElementFrom(document, "div");
  truncatedMessage.className = "graph-truncated";
  truncatedMessage.textContent = layout.truncated ? "Truncated" : "Not truncated";
  if (!layout.truncated) {
    truncatedMessage.textContent = "Not truncated";
  }
  if (layout.truncatedReason) {
    const reason = createElementFrom(document, "div");
    reason.className = "graph-truncated-reason";
    reason.textContent = `Reason: ${toString(layout.truncatedReason)}`;
    root.appendChild(reason);
  }
  root.appendChild(header);
  root.appendChild(truncatedMessage);

  if (layout.source_summary) {
    const summary = createElementFrom(document, "div");
    summary.className = "graph-summary-counts";
    summary.textContent = `Source Summary: nodes=${layout.source_summary.total_nodes}, edges=${layout.source_summary.total_edges}`;
    root.appendChild(summary);
  }

  const content = createElementFrom(document, "div");
  content.className = "graph-content";

  const svg = buildSVGCanvas(document, layout);
  content.appendChild(svg);

  const nodeBlock = createListBlock(document, "Nodes", layout.nodes || [], (node) =>
    buildNodeElement(document, node, options)
  );
  const edgeBlock = createListBlock(document, "Edges", layout.edges || [], (edge) =>
    buildEdgeElement(document, edge)
  );
  const diagBlock = buildDiagnostics(document, layout.diagnostics || []);

  content.appendChild(nodeBlock);
  content.appendChild(edgeBlock);
  content.appendChild(diagBlock);
  root.appendChild(content);

  return root;
}

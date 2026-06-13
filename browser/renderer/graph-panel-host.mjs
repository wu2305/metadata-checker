/**
 * 浏览器图关系面板宿主。
 *
 * 只负责 DOM 容器、收起展开、状态 marker 和委托 renderer。
 * 不读取 BI 设计器对象，不获取远程元数据，不调用 runtime。
 */

const MARKER_PREFIX = "data-metadata-checker-";
const DEFAULT_VISIBLE_HOP = 1;
const ANALYSIS_STATUSES = ["idle", "loading", "ready", "empty", "warning", "error", "pinned"];
const PANEL_WIDTH = "min(280px, calc(100vw - 32px))";
const PANEL_HEIGHT = "min(260px, calc(100vh - 32px))";
/** 右下角贴边；属性栏避让后续按实测再调 */
const PANEL_RIGHT_OFFSET = "16px";
const PANEL_BOTTOM_OFFSET = "16px";

function noop() {}

function statusDotColor(statusValue) {
  switch (String(statusValue || "").toLowerCase()) {
    case "pinned":
      return "#f59e0b";
    case "loading":
      return "#a78bfa";
    case "warning":
      return "#fbbf24";
    case "error":
      return "#f87171";
    case "ready":
      return "#38bdf8";
    case "empty":
      return "#64748b";
    default:
      return "#64748b";
  }
}

function makeLogger(logger) {
  return {
    warn: typeof logger?.warn === "function" ? logger.warn.bind(logger) : noop,
    error: typeof logger?.error === "function" ? logger.error.bind(logger) : noop,
  };
}

function setMarker(element, name, value) {
  if (element && typeof element.setAttribute === "function") {
    element.setAttribute(`${MARKER_PREFIX}${name}`, String(value));
  }
}

function parseCount(value, fallback = 0) {
  const number = Number.parseInt(value, 10);
  return Number.isFinite(number) ? Math.max(0, number) : fallback;
}

function normalizeEmbeddedState(panelState, isCollapsed, isMounted) {
  if (!isMounted) {
    return "hidden";
  }
  if (isCollapsed) {
    return "collapsed";
  }
  return panelState === "hidden" ? "hidden" : "mounted";
}

function normalizeStatus(status, fallback = "idle") {
  if (typeof status !== "string") {
    return fallback;
  }
  const normalized = status.toLowerCase().trim();
  if (!normalized) {
    return fallback;
  }
  if (normalized === "running") {
    return "loading";
  }
  return ANALYSIS_STATUSES.includes(normalized) ? normalized : fallback;
}

function normalizeText(value) {
  if (value == null) {
    return "";
  }
  return String(value);
}

function compactFocusLabel(value) {
  const text = normalizeText(value).trim();
  if (!text) return "No selection";
  const beforeCanvas = text.split("|")[0] || text;
  const parts = beforeCanvas.split("/").filter(Boolean);
  return parts.at(-1) || beforeCanvas || text;
}

function normalizeSummary(input, fallback = {}) {
  const source = input ?? {};
  const nodes = Array.isArray(source.nodes) ? source.nodes : [];
  const edges = Array.isArray(source.edges) ? source.edges : [];
  return {
    focus: normalizeText(source.focus_node ?? source.target ?? source.active_component_id ?? source.focus ?? fallback.focus),
    depth: parseCount(source.depth, parseCount(source.maxDepth, parseCount(fallback.depth, 0))),
    visibleHop: parseCount(
      source.visible_hop,
      parseCount(source.visibleHop, parseCount(fallback.visibleHop, DEFAULT_VISIBLE_HOP)),
    ),
    nodeCount: parseCount(
      source.nodeCount,
      parseCount(source.nodes?.length, parseCount(fallback.nodeCount, 0)),
    ),
    edgeCount: parseCount(
      source.edgeCount,
      parseCount(source.edges?.length, parseCount(fallback.edgeCount, 0)),
    ),
    renderer: normalizeText(source.renderer),
  };
}

function createElement(documentLike, tagName) {
  if (!documentLike || typeof documentLike.createElement !== "function") {
    return null;
  }
  return documentLike.createElement(tagName);
}

function append(parent, child) {
  if (parent && child && typeof parent.appendChild === "function") {
    parent.appendChild(child);
  }
}

function clear(element) {
  if (!element) return;
  if (typeof element.replaceChildren === "function") {
    element.replaceChildren();
    return;
  }
  if (Array.isArray(element.children)) {
    element.children.length = 0;
  }
}

function extractSummaryFromRenderResult(renderResult, source) {
  const sourceSummary = normalizeSummary(source, {
    focus: source?.focus_node ?? source?.target ?? source?.active_component_id ?? "",
    depth: parseCount(source?.depth, 0),
    visibleHop: parseCount(source?.visible_hop, parseCount(source?.visibleHop, DEFAULT_VISIBLE_HOP)),
    nodeCount: parseCount(source?.nodeCount, parseCount(source?.nodes?.length, 0)),
    edgeCount: parseCount(source?.edgeCount, parseCount(source?.edges?.length, 0)),
    renderer: normalizeText(source?.renderer),
  });
  const graph = renderResult?.graph ?? source ?? {};
  const renderer = normalizeText(
    renderResult?.markerRenderer || renderResult?.renderer || graph.renderer || sourceSummary.renderer,
  );
  return {
    focus: normalizeText(
      renderResult?.focus ??
      renderResult?.focus_node ??
      graph.focus_node ??
      sourceSummary.focus,
    ),
    depth: parseCount(
      renderResult?.markerDepth,
      parseCount(renderResult?.depth, parseCount(graph.depth, sourceSummary.depth)),
    ),
    visibleHop: parseCount(
      renderResult?.visibleHop,
      parseCount(renderResult?.visible_hop, sourceSummary.visibleHop),
    ),
    nodeCount: parseCount(
      renderResult?.nodeCount,
      parseCount(renderResult?.node_count, parseCount(graph.nodeCount, parseCount(graph.nodes?.length, sourceSummary.nodeCount))),
    ),
    edgeCount: parseCount(
      renderResult?.edgeCount,
      parseCount(renderResult?.edge_count, parseCount(graph.edgeCount, parseCount(graph.edges?.length, sourceSummary.edgeCount))),
    ),
    visibleNodeCount: parseCount(
      graph.viewGraph?.visibleNodeCount,
      parseCount(graph.nodeCount, sourceSummary.nodeCount),
    ) + parseCount(graph.viewGraph?.aggregateNodeCount, 0),
    fullNodeCount: parseCount(
      source?.source_summary?.total_nodes,
      parseCount(graph.nodes?.length, sourceSummary.nodeCount),
    ),
    visibleEdgeCount: parseCount(
      graph.viewGraph?.visibleEdgeCount,
      parseCount(graph.edgeCount, sourceSummary.edgeCount),
    ),
    fullEdgeCount: parseCount(
      source?.source_summary?.total_edges,
      parseCount(graph.edges?.length, sourceSummary.edgeCount),
    ),
    renderer: renderer || "fallback",
  };
}

function makeDiagnostic(code, message) {
  return {
    status: "error",
    target: null,
    items: [],
    diagnostics: [{ severity: "error", code, message }],
  };
}

function hasWarningDiagnostic(renderResult) {
  const diags = [];
  if (Array.isArray(renderResult?.diagnostics)) {
    diags.push(...renderResult.diagnostics);
  }
  if (Array.isArray(renderResult?.graph?.diagnostics)) {
    diags.push(...renderResult.graph.diagnostics);
  }
  return diags.some((diag) => String(diag?.severity || "").toLowerCase() === "warn" || String(diag?.code || "").includes("WARN"));
}

function sanitizeCopyText(value) {
  const text = normalizeText(value);
  if (text === "") return text;
  if (/token|password|secret|cookie|auth|credential|api_key|apikey|cipherpassport/i.test(text)) {
    return "[sensitive]";
  }
  return text;
}

function normalizeDetailKind(value) {
  if (value === "node" || value === "edge" || value === "aggregate" || value === "focus" || value === "empty" || value === "error") {
    return value;
  }
  return "empty";
}

function normalizeRendererCopyContext(context) {
  if (!context || typeof context !== "object") {
    return null;
  }
  return {
    kind: normalizeDetailKind(context.kind || context.type || "empty"),
    context: sanitizeCopyText(context.context || context.text || context.id || ""),
    id: sanitizeCopyText(context.id || context.nodeId || context.edgeId || context.from || ""),
    payload: context.payload ?? null,
  };
}

export function createGraphPanelHost(options = {}) {
  const documentLike = options.document ?? globalThis.document;
  const parent = options.parent ?? documentLike?.body ?? null;
  const renderer = options.renderer;
  const logger = makeLogger(options.logger);

  let root = null;
  let body = null;
  let mounted = false;
  let collapsed = false;
  let status = "idle";
  let renderSummary = {
    focus: "",
    depth: 0,
    visibleHop: DEFAULT_VISIBLE_HOP,
    nodeCount: 0,
    edgeCount: 0,
    renderer: "fallback",
  };

  let header = null;
  let footer = null;
  let toggleButton = null;
  let pinButton = null;
  let copyButton = null;
  let a11yTarget = null;
  let graphPinned = false;

  function resolveRendererContext() {
    if (renderer && typeof renderer.getOpenDetailContext === "function") {
      const context = renderer.getOpenDetailContext();
      return normalizeRendererCopyContext(context);
    }
    if (body && typeof body.getAttribute === "function") {
      return normalizeRendererCopyContext({
        kind: normalizeDetailKind(
          body.getAttribute("data-metadata-checker-graph-detail-kind")
            || body.getAttribute("data-metadata-checker-graph-open-detail"),
        ),
        context: body.getAttribute("data-metadata-checker-graph-open-detail-context"),
        id: renderSummary.focus,
      });
    }
    return {
      kind: renderSummary.focus ? "focus" : "empty",
      context: sanitizeCopyText(renderSummary.focus),
      id: sanitizeCopyText(renderSummary.focus),
      payload: null,
    };
  }

  function buildCopyGraphText() {
    if (renderer && typeof renderer.getVisibleGraphText === "function") {
      const text = renderer.getVisibleGraphText();
      if (typeof text === "string" && text.trim().length > 0) {
        return text;
      }
    }
    return [
      `focus: ${renderSummary.focus || ""}`,
      `visible: ${renderSummary.visibleNodeCount}/${renderSummary.fullNodeCount} nodes, ${renderSummary.visibleEdgeCount}/${renderSummary.fullEdgeCount} edges`,
      "nodes:",
      "edges:",
    ].join("\n");
  }

  function applyRendererMarkers() {
    if (!root || !mounted) {
      return;
    }

    const context = resolveRendererContext();
    const kind = normalizeDetailKind(context?.kind ?? "empty");
    const contextValue = context?.context ?? context?.id ?? "";

    setMarker(root, "graph-open-detail-context", contextValue);
    setMarker(root, "graph-detail-kind", kind);
    setMarker(root, "graph-open-detail", kind);
  }

  function forwardViewportReset() {
    if (renderer && typeof renderer.resetViewportScale === "function") {
      renderer.resetViewportScale();
      applyRendererMarkers();
    }
  }

  function forwardLock() {
    if (renderer && typeof renderer.lockCurrentTarget === "function") {
      const context = renderer.lockCurrentTarget();
      applyRendererMarkers();
      if (context) {
        setMarker(root, "graph-open-detail-context", normalizeRendererCopyContext(context)?.context ?? "");
        setMarker(root, "graph-detail-kind", normalizeRendererCopyContext(context)?.kind || "empty");
      }
      return;
    }
    applyRendererMarkers();
  }

  function forwardUnlock() {
    if (renderer && typeof renderer.clearLockedTarget === "function") {
      renderer.clearLockedTarget();
      applyRendererMarkers();
      return;
    }
    applyRendererMarkers();
  }

  function writeHostMarkers(panelState = mounted ? "mounted" : "unmounted") {
    if (!root) return;
    setMarker(root, "graph-panel", panelState);
    setMarker(root, "embedded-popup", normalizeEmbeddedState(panelState, collapsed, mounted));
    setMarker(root, "analysis-status", status);
    setMarker(root, "graph-panel-collapsed", collapsed ? "true" : "false");
    setMarker(root, "focus-component", renderSummary.focus);
    setMarker(root, "graph-depth", renderSummary.depth);
    setMarker(root, "graph-visible-hop", renderSummary.visibleHop);
    setMarker(root, "graph-node-count", renderSummary.nodeCount);
    setMarker(root, "graph-edge-count", renderSummary.edgeCount);
    setMarker(root, "local-graph-renderer", renderSummary.renderer);

    const context = resolveRendererContext();
    setMarker(root, "graph-open-detail-context", context?.context ?? "");
    setMarker(root, "graph-detail-kind", normalizeDetailKind(context?.kind));
    setMarker(root, "graph-open-detail", normalizeDetailKind(context?.kind));
  }

  function writeActionLabels() {
    if (!header) return;
    const focusLabel = root?.querySelector?.(".metadata-checker-graph-focus") || null;
    if (focusLabel) {
      focusLabel.textContent = compactFocusLabel(renderSummary.focus);
      focusLabel.title = renderSummary.focus || "";
    }
    const summaryLabel = root?.querySelector?.(".metadata-checker-graph-counts") || null;
    if (summaryLabel) {
      const visibleNodes = renderSummary.visibleNodeCount ?? renderSummary.nodeCount;
      const fullNodes = renderSummary.fullNodeCount ?? renderSummary.nodeCount;
      summaryLabel.textContent = `${visibleNodes}/${fullNodes}`;
      summaryLabel.title = `${visibleNodes} visible / ${fullNodes} total nodes`;
    }
    const statusDot = root?.querySelector?.(".metadata-checker-graph-status-dot") || null;
    if (statusDot) {
      statusDot.setAttribute("data-status", status);
      statusDot.title = status;
      if (statusDot.style) {
        const dotColor = statusDotColor(status);
        statusDot.style.background = dotColor;
        statusDot.style.boxShadow = `0 0 0 2px ${dotColor}33`;
      }
    }
    if (pinButton?.style) {
      pinButton.style.background = graphPinned ? "rgba(245, 158, 11, 0.22)" : "rgba(30, 41, 59, 0.92)";
      pinButton.style.borderColor = graphPinned ? "rgba(245, 158, 11, 0.55)" : "rgba(148, 163, 184, 0.35)";
      pinButton.style.color = graphPinned ? "#fde68a" : "#cbd5e1";
    }
  }

  function setStatus(nextStatus, summary) {
    status = normalizeStatus(nextStatus, status);
    if (summary) {
      renderSummary = {
        ...renderSummary,
        ...summary,
      };
    }
    writeHostMarkers();
    writeActionLabels();
  }

  function buildButtons(document, bodyContainer) {
    const pinWrap = createElement(document, "div");
    pinWrap.className = "metadata-checker-graph-actions";
    if (!pinWrap) return null;
    pinWrap.style.display = "flex";
    pinWrap.style.justifyContent = "flex-end";
    pinWrap.style.alignItems = "center";
    pinWrap.style.gap = "4px";
    pinWrap.style.boxSizing = "border-box";

    pinButton = createElement(document, "button");
    copyButton = createElement(document, "button");
    toggleButton = createElement(document, "button");

    pinButton.type = "button";
    copyButton.type = "button";
    toggleButton.type = "button";

    pinButton.textContent = "P";
    copyButton.textContent = "C";
    toggleButton.textContent = "−";
    pinButton.title = "Pin current graph";
    copyButton.title = "Copy visible graph text";
    toggleButton.title = "Collapse graph panel";
    pinButton.setAttribute?.("aria-label", "Pin current graph");
    copyButton.setAttribute?.("aria-label", "Copy visible graph text");
    toggleButton.setAttribute?.("aria-label", "Collapse graph panel");

    pinButton.className = "metadata-checker-graph-panel-pin";
    copyButton.className = "metadata-checker-graph-panel-copy";
    toggleButton.className = "metadata-checker-graph-panel-toggle";
    for (const button of [pinButton, copyButton, toggleButton]) {
      button.style.minWidth = "0";
      button.style.width = "22px";
      button.style.height = "22px";
      button.style.overflow = "hidden";
      button.style.textOverflow = "ellipsis";
      button.style.whiteSpace = "nowrap";
      button.style.fontSize = "11px";
      button.style.lineHeight = "18px";
      button.style.padding = "0";
      button.style.boxSizing = "border-box";
      button.style.border = "1px solid rgba(148, 163, 184, 0.35)";
      button.style.borderRadius = "5px";
      button.style.background = "rgba(30, 41, 59, 0.92)";
      button.style.color = "#cbd5e1";
      button.style.cursor = "pointer";
    }

    pinButton.addEventListener?.("click", async () => {
      graphPinned = !graphPinned;
      status = graphPinned ? "pinned" : (renderSummary.nodeCount > 0 ? "ready" : "idle");
      if (pinButton?.style) {
        pinButton.style.background = graphPinned ? "rgba(245, 158, 11, 0.22)" : "rgba(30, 41, 59, 0.92)";
        pinButton.style.borderColor = graphPinned ? "rgba(245, 158, 11, 0.55)" : "rgba(148, 163, 184, 0.35)";
        pinButton.style.color = graphPinned ? "#fde68a" : "#cbd5e1";
      }
      writeHostMarkers();
      if (typeof options.onPinChange === "function") {
        await options.onPinChange({ pinned: graphPinned });
      }
    });
    copyButton.addEventListener?.("click", async () => {
      const value = buildCopyGraphText();
      if (typeof globalThis.navigator?.clipboard?.writeText === "function") {
        await globalThis.navigator.clipboard.writeText(value);
      } else if (documentLike?.body) {
        const hidden = createElement(documentLike, "textarea");
        hidden.value = value;
        if (hidden) {
          hidden.style.position = "fixed";
          hidden.style.opacity = "0";
          append(documentLike.body, hidden);
          hidden.select?.();
        }
      }
      if (typeof options.onCopyGraph === "function") {
        options.onCopyGraph({ text: value });
      }
      setMarker(root, "graph-copy-text-length", String(value.length));
      applyRendererMarkers();
    });
    toggleButton.addEventListener?.("click", () => {
      setCollapsed(!collapsed);
    });

    append(pinWrap, pinButton);
    append(pinWrap, copyButton);
    append(pinWrap, toggleButton);
    append(bodyContainer, pinWrap);
    return pinWrap;
  }

  function ensureMounted() {
    if (mounted && root) {
      return { mounted: true, alreadyMounted: true, root, body };
    }

    if (!documentLike || !parent) {
      const error = makeDiagnostic(
        "GRAPH_PANEL_HOST_UNAVAILABLE",
        "document and parent are required to mount graph panel host",
      );
      logger.warn(error.diagnostics[0].message);
      return { mounted: false, error };
    }

    root = createElement(documentLike, "section");
    body = createElement(documentLike, "div");
    header = createElement(documentLike, "header");
    footer = createElement(documentLike, "div");

    root.className = "metadata-checker-graph-panel";
    root.style.position = "fixed";
    root.style.right = options.panelRightOffset ?? PANEL_RIGHT_OFFSET;
    root.style.bottom = options.panelBottomOffset ?? PANEL_BOTTOM_OFFSET;
    root.style.width = PANEL_WIDTH;
    root.style.height = PANEL_HEIGHT;
    root.style.maxWidth = "calc(100vw - 32px)";
    root.style.maxHeight = "calc(100vh - 32px)";
    root.style.overflow = "hidden";
    root.style.zIndex = "2147483647";
    root.style.display = "flex";
    root.style.flexDirection = "column";
    root.style.boxSizing = "border-box";
    root.style.border = "1px solid rgba(100, 116, 139, 0.42)";
    root.style.borderRadius = "10px";
    root.style.background = "rgba(15, 23, 42, 0.94)";
    root.style.boxShadow = "0 18px 42px rgba(2, 6, 23, 0.45), 0 0 0 1px rgba(148, 163, 184, 0.08)";
    root.style.fontFamily = "Inter, -apple-system, BlinkMacSystemFont, \"Segoe UI\", sans-serif";
    root.style.color = "#e2e8f0";
    root.setAttribute?.("data-metadata-checker-graph-panel-theme", "inspector-dark");

    if (header) {
      const statusDot = createElement(documentLike, "span");
      const title = createElement(documentLike, "div");
      const counts = createElement(documentLike, "div");
      statusDot.className = "metadata-checker-graph-status-dot";
      statusDot.style.width = "7px";
      statusDot.style.height = "7px";
      statusDot.style.borderRadius = "999px";
      statusDot.style.background = statusDotColor(status);
      statusDot.style.boxShadow = `0 0 0 2px ${statusDotColor(status)}33`;
      statusDot.style.flex = "0 0 auto";
      title.className = "metadata-checker-graph-panel-title";
      title.textContent = "Local Graph";
      title.style.fontSize = "11px";
      title.style.fontWeight = "600";
      title.style.lineHeight = "1";
      title.style.minWidth = "0";
      title.style.color = "#cbd5e1";
      title.style.letterSpacing = "0.01em";
      counts.className = "metadata-checker-graph-counts";
      counts.textContent = "0/0";
      counts.style.fontSize = "11px";
      counts.style.lineHeight = "1";
      counts.style.fontVariantNumeric = "tabular-nums";
      counts.style.color = "#94a3b8";
      counts.style.fontWeight = "500";
      counts.style.marginLeft = "auto";
      append(header, statusDot);
      append(header, title);
      append(header, counts);
      header.className = "metadata-checker-graph-panel-header";
      header.style.display = "flex";
      header.style.alignItems = "center";
      header.style.gap = "7px";
      header.style.height = "28px";
      header.style.flex = "0 0 28px";
      header.style.padding = "0 8px";
      header.style.borderBottom = "1px solid rgba(100, 116, 139, 0.32)";
      header.style.background = "rgba(15, 23, 42, 0.98)";
      header.style.boxSizing = "border-box";
    }

    body.className = "metadata-checker-graph-panel-body";
    body.style.flex = "1";
    body.style.minHeight = "0";
    body.style.overflowY = "hidden";
    body.style.overflowX = "hidden";
    body.style.position = "relative";
    body.style.padding = "4px";
    body.style.background = "#0f172a";
    body.style.boxSizing = "border-box";

    if (footer) {
      const focusNode = createElement(documentLike, "div");
      focusNode.className = "metadata-checker-graph-focus";
      focusNode.textContent = "No selection";
      focusNode.style.minWidth = "0";
      focusNode.style.overflow = "hidden";
      focusNode.style.textOverflow = "ellipsis";
      focusNode.style.whiteSpace = "nowrap";
      focusNode.style.fontSize = "11px";
      focusNode.style.lineHeight = "1.1";
      focusNode.style.color = "#94a3b8";
      focusNode.style.fontWeight = "500";
      append(footer, focusNode);
      footer.className = "metadata-checker-graph-panel-footer";
      footer.style.boxSizing = "border-box";
      footer.style.width = "100%";
      footer.style.height = "32px";
      footer.style.flex = "0 0 32px";
      footer.style.padding = "4px 6px";
      footer.style.overflow = "hidden";
      footer.style.display = "grid";
      footer.style.gridTemplateColumns = "minmax(0, 1fr) auto";
      footer.style.alignItems = "center";
      footer.style.gap = "6px";
      footer.style.borderTop = "1px solid rgba(100, 116, 139, 0.32)";
      footer.style.background = "rgba(15, 23, 42, 0.98)";
    }

    append(root, header);
    append(root, body);
    append(root, footer);
    append(parent, root);

    a11yTarget = createElement(documentLike, "div");
    if (a11yTarget) {
      a11yTarget.className = "metadata-checker-graph-a11y-target";
      a11yTarget.setAttribute?.("tabindex", "0");
      a11yTarget.setAttribute?.("aria-label", "Local graph interaction target");
      a11yTarget.style.position = "absolute";
      a11yTarget.style.width = "0px";
      a11yTarget.style.height = "0px";
      a11yTarget.style.opacity = "0";
      a11yTarget.style.pointerEvents = "none";
      a11yTarget.style.left = "-9999px";
      a11yTarget.style.top = "-9999px";
      a11yTarget.style.outline = "none";
      a11yTarget.addEventListener?.("keydown", (event) => {
        const key = String(event?.key || "").toLowerCase();
        if (key === "enter" || key === " " || key === "spacebar") {
          event?.preventDefault?.();
          forwardLock();
          return;
        }
        if (key === "escape") {
          event?.preventDefault?.();
          forwardUnlock();
          return;
        }
        if (key === "0") {
          event?.preventDefault?.();
          forwardViewportReset();
        }
      });
      append(root, a11yTarget);
    }

    buildButtons(documentLike, footer);

    mounted = true;
    if (renderer && typeof renderer.setContainer === "function") {
      renderer.setContainer(body);
    }
    setStatus("idle", {
      focus: "",
      depth: 0,
      visibleHop: DEFAULT_VISIBLE_HOP,
      nodeCount: 0,
      edgeCount: 0,
      renderer: "fallback",
    });
    return { mounted: true, alreadyMounted: false, root, body };
  }

  function setCollapsed(nextCollapsed) {
    collapsed = Boolean(nextCollapsed);
    if (body) {
      body.style.display = collapsed ? "none" : "";
    }
    if (footer) {
      footer.style.display = collapsed ? "none" : "";
    }
    if (toggleButton) {
      toggleButton.textContent = collapsed ? "+" : "−";
      toggleButton.title = collapsed ? "Show graph panel" : "Collapse graph panel";
    }
    writeHostMarkers(collapsed ? "hidden" : "mounted");
    return { collapsed };
  }

  function resolveStatusFromResult(result, renderResult) {
    const sourceStatus = normalizeStatus(renderResult?.status ?? result?.status, normalizeStatus(result?.status, "ready"));
    if (sourceStatus === "error") {
      return "error";
    }
    const summary = extractSummaryFromRenderResult(renderResult, result);
    const nodeCount = parseCount(summary.nodeCount, 0);
    const edgeCount = parseCount(summary.edgeCount, 0);
    if (nodeCount === 0 && edgeCount === 0) {
      return "empty";
    }
    if (hasWarningDiagnostic(renderResult) || sourceStatus === "warning") {
      return "warning";
    }
    if (sourceStatus === "pinned") {
      return "pinned";
    }
    return sourceStatus || "ready";
  }

  function showShell(summary = {}) {
    const mountResult = ensureMounted();
    if (!mountResult.mounted) {
      return mountResult.error;
    }
    const normalized = {
      focus: normalizeText(summary.focus ?? summary.focus_node ?? summary.target),
      depth: parseCount(summary.depth, renderSummary.depth),
      visibleHop: parseCount(summary.visibleHop ?? summary.visible_hop, renderSummary.visibleHop),
      nodeCount: parseCount(summary.nodeCount ?? summary.node_count, 0),
      edgeCount: parseCount(summary.edgeCount ?? summary.edge_count, 0),
      renderer: normalizeText(summary.renderer) || renderSummary.renderer,
    };
    setStatus("loading", normalized);
    writeHostMarkers();
    return { status: "loading", mounted: true, root, body };
  }

  async function render(result) {
    const mountResult = ensureMounted();
    if (!mountResult.mounted) {
      return mountResult.error;
    }
    const summary = extractSummaryFromRenderResult({ markerDepth: 0 }, result);
    setStatus("loading", { ...summary });
    try {
      let renderResult;
      if (renderer && typeof renderer.render === "function") {
        renderResult = await renderer.render(result);
      } else if (renderer && typeof renderer.renderGraph === "function") {
        renderResult = await renderer.renderGraph(result);
      }

      const resolvedSummary = extractSummaryFromRenderResult(renderResult, result);
      const nextStatus = resolveStatusFromResult(result, renderResult);
      setStatus(nextStatus, resolvedSummary);
      applyRendererMarkers();
      return renderResult ?? { status: nextStatus };
    } catch (err) {
      const error = makeDiagnostic(
        "GRAPH_PANEL_RENDER_FAILED",
        err?.message ?? String(err),
      );
      const nextSummary = extractSummaryFromRenderResult(error, result);
      setStatus("error", nextSummary);
      applyRendererMarkers();
      return error;
    }
  }

  async function renderError(errorEnvelope) {
    const mountResult = ensureMounted();
    if (!mountResult.mounted) {
      return mountResult.error;
    }
    const summary = extractSummaryFromRenderResult({ markerDepth: 0 }, errorEnvelope);
    setStatus("loading", summary);
    try {
      let renderResult;
      if (renderer && typeof renderer.renderError === "function") {
        renderResult = await renderer.renderError(errorEnvelope);
      } else if (renderer && typeof renderer.render === "function") {
        renderResult = await renderer.render(errorEnvelope);
      }
      const nextSummary = extractSummaryFromRenderResult(renderResult, errorEnvelope);
      const nextStatus = resolveStatusFromResult(errorEnvelope, renderResult);
      setStatus(nextStatus, nextSummary);
      applyRendererMarkers();
      return renderResult ?? errorEnvelope;
    } catch (err) {
      const error = makeDiagnostic(
        "GRAPH_PANEL_RENDER_FAILED",
        err?.message ?? String(err),
      );
      const nextSummary = extractSummaryFromRenderResult(error, errorEnvelope);
      setStatus("error", nextSummary);
      applyRendererMarkers();
      return error;
    }
  }

  function unmount() {
    if (!root) {
      mounted = false;
      return { mounted: false };
    }
    if (typeof root.remove === "function") {
      root.remove();
    } else if (parent && Array.isArray(parent.children)) {
      const index = parent.children.indexOf(root);
      if (index >= 0) {
        parent.children.splice(index, 1);
      }
    }
    clear(root);
    root = null;
    body = null;
    header = null;
    footer = null;
    toggleButton = null;
    pinButton = null;
    copyButton = null;
    a11yTarget = null;
    graphPinned = false;
    mounted = false;
    status = "idle";
    return { mounted: false };
  }

  return {
    mount: ensureMounted,
    showShell,
    unmount,
    render,
    renderError,
    setStatus,
    setCollapsed,
    lockCurrentTarget: forwardLock,
    clearLockedTarget: forwardUnlock,
    resetViewportScale: forwardViewportReset,
    refreshRendererContext: applyRendererMarkers,
    getOpenDetailContext: resolveRendererContext,
    getVisibleGraphText: buildCopyGraphText,
    isPinned() {
      return graphPinned;
    },
    status() {
      return { mounted, collapsed, status, root, body, graphPinned };
    },
  };
}

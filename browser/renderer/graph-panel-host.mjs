/**
 * 浏览器图关系面板宿主。
 *
 * 只负责 DOM 容器、收起展开、状态 marker 和委托 renderer。
 * 不读取 BI 设计器对象，不获取远程元数据，不调用 runtime。
 */

const MARKER_PREFIX = "data-metadata-checker-";

function noop() {}

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

function makeDiagnostic(code, message) {
  return {
    status: "error",
    target: null,
    items: [],
    diagnostics: [{ severity: "error", code, message }],
  };
}

export function createGraphPanelHost(options = {}) {
  const documentLike = options.document ?? globalThis.document;
  const parent = options.parent ?? documentLike?.body ?? null;
  const renderer = options.renderer;
  const logger = makeLogger(options.logger);

  let root = null;
  let body = null;
  let toggleButton = null;
  let mounted = false;
  let collapsed = false;
  let status = "idle";

  function writeHostMarkers(panelState = mounted ? "mounted" : "unmounted") {
    if (!root) return;
    setMarker(root, "graph-panel", panelState);
    setMarker(root, "analysis-status", status);
    setMarker(root, "graph-panel-collapsed", collapsed ? "true" : "false");
  }

  function setStatus(nextStatus) {
    status = nextStatus;
    writeHostMarkers();
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
    const header = createElement(documentLike, "header");
    const title = createElement(documentLike, "span");
    toggleButton = createElement(documentLike, "button");

    root.className = "metadata-checker-graph-panel";
    header.className = "metadata-checker-graph-panel-header";
    title.className = "metadata-checker-graph-panel-title";
    title.textContent = "Metadata Graph";
    toggleButton.type = "button";
    toggleButton.className = "metadata-checker-graph-panel-toggle";
    toggleButton.textContent = "hide";
    body.className = "metadata-checker-graph-panel-body";

    toggleButton.addEventListener?.("click", () => {
      setCollapsed(!collapsed);
    });

    append(header, title);
    append(header, toggleButton);
    append(root, header);
    append(root, body);
    append(parent, root);

    mounted = true;
    if (renderer && typeof renderer.setContainer === "function") {
      renderer.setContainer(body);
    }
    setStatus("idle");
    return { mounted: true, alreadyMounted: false, root, body };
  }

  function setCollapsed(nextCollapsed) {
    collapsed = Boolean(nextCollapsed);
    if (body?.style) {
      body.style.display = collapsed ? "none" : "";
    }
    if (toggleButton) {
      toggleButton.textContent = collapsed ? "show" : "hide";
    }
    writeHostMarkers(collapsed ? "hidden" : "mounted");
    return { collapsed };
  }

  async function render(result) {
    const mountResult = ensureMounted();
    if (!mountResult.mounted) {
      return mountResult.error;
    }
    setStatus("running");
    try {
      let renderResult;
      if (renderer && typeof renderer.render === "function") {
        renderResult = await renderer.render(result);
      } else if (renderer && typeof renderer.renderGraph === "function") {
        renderResult = await renderer.renderGraph(result);
      }
      setStatus(result?.status === "error" ? "error" : "ready");
      return renderResult ?? { status: "ready" };
    } catch (err) {
      const error = makeDiagnostic(
        "GRAPH_PANEL_RENDER_FAILED",
        err?.message ?? String(err),
      );
      setStatus("error");
      return error;
    }
  }

  async function renderError(errorEnvelope) {
    const mountResult = ensureMounted();
    if (!mountResult.mounted) {
      return mountResult.error;
    }
    setStatus("running");
    try {
      let renderResult;
      if (renderer && typeof renderer.renderError === "function") {
        renderResult = await renderer.renderError(errorEnvelope);
      } else if (renderer && typeof renderer.render === "function") {
        renderResult = await renderer.render(errorEnvelope);
      }
      setStatus("error");
      return renderResult ?? errorEnvelope;
    } catch (err) {
      const error = makeDiagnostic(
        "GRAPH_PANEL_RENDER_FAILED",
        err?.message ?? String(err),
      );
      setStatus("error");
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
    toggleButton = null;
    mounted = false;
    status = "idle";
    return { mounted: false };
  }

  return {
    mount: ensureMounted,
    unmount,
    render,
    renderError,
    setStatus,
    setCollapsed,
    status() {
      return { mounted, collapsed, status, root, body };
    },
  };
}

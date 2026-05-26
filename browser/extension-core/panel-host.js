(function (root) {
  "use strict";

  const MARKER_STATE = "data-metadata-checker-panel";
  const MARKER_TRIGGER = "data-metadata-checker-panel-trigger";
  const MARKER_SOURCE_PATH = "data-metadata-checker-panel-source-path";
  const MARKER_SELECTION_COUNT = "data-metadata-checker-panel-selection-count";
  const MARKER_LAST_STATUS = "data-metadata-checker-panel-last-status";
  const RAW_FIELD_KEYS = new Set([
    "raw_text",
    "raw_metadata",
    "component",
    "raw_component",
    "components",
  ]);

  function isObject(value) {
    return value !== null && typeof value === "object";
  }

  function asString(value) {
    return typeof value === "string" ? value : "";
  }

  function asArray(value) {
    return Array.isArray(value) ? value : [];
  }

  function createDiagnostic(code, message, severity = "error") {
    return {
      code,
      message,
      severity,
    };
  }

  function normalizeSelection(selection = {}) {
    const selected = asArray(selection.selected_component_ids);
    const sourcePath = asString(selection.source_path);
    const active = asString(selection.active_component_id);
    return {
      source_path: sourcePath,
      selected_component_ids: selected,
      active_component_id: active,
      page_type: asString(selection.page_type || "") || "",
      timestamp: typeof selection.timestamp === "number" ? selection.timestamp : Date.now(),
    };
  }

  function extractEnvelopeSummary(envelope = {}) {
    const status = asString(envelope.status) || "ready";
    const target = asString(envelope.target);
    const itemCount = asArray(envelope.items).length;
    const diagnosticCount = asArray(envelope.diagnostics).length;
    const focus = asString(envelope.focus_node || envelope.focus || envelope.target_node);

    return {
      status,
      target,
      itemCount,
      diagnosticCount,
      focus,
    };
  }

  function makePanelDiagnostic(code, message) {
    return {
      status: "error",
      target: null,
      items: [],
      diagnostics: [createDiagnostic(code, message)],
    };
  }

  function buildPanelText(envelopeSummary, selectionSummary) {
    const safeStatus = asString(envelopeSummary.status);
    const safeTarget = asString(envelopeSummary.target);
    const safeFocus = asString(envelopeSummary.focus);

    const lines = [
      `Status: ${safeStatus || "unknown"}`,
      safeTarget ? `Target: ${safeTarget}` : "Target:",
      `Selection: ${selectionSummary.selected_component_ids.length}`,
      selectionSummary.active_component_id
        ? `Active: ${selectionSummary.active_component_id}`
        : "Active:",
      `Items: ${String(envelopeSummary.itemCount)}`,
      `Diagnostics: ${String(envelopeSummary.diagnosticCount)}`,
      safeFocus ? `Focus: ${safeFocus}` : "Focus:",
      asString(selectionSummary.source_path) ? `Source: ${selectionSummary.source_path}` : "Source:",
    ];

    return lines.join("\n");
  }

  function createPanelHost() {
    let documentLike = undefined;
    let mounted = false;
    let visible = false;
    let hostElement = null;
    let shadowRoot = null;
    let triggerButton = null;
    let panelElement = null;
    let contentElement = null;
    let panelStatus = "idle";
    let lastDiagnostic = null;
    let selection = normalizeSelection({});
    let lastEnvelope = { status: "idle", diagnostics: [] };

    let selectionCount = 0;
    let sourcePath = "";

    function setMarker(name, value) {
      if (!hostElement || typeof hostElement.setAttribute !== "function") {
        return;
      }
      hostElement.setAttribute(name, value);
    }

    function renderPanel() {
      if (!mounted || !contentElement) {
        return;
      }
      const envelopeSummary = extractEnvelopeSummary(lastEnvelope);
      const selectionSummary = selection;
      const safeText = buildPanelText(envelopeSummary, selectionSummary);
      contentElement.textContent = safeText;
      panelElement.style.display = visible ? "" : "none";
    }

    function writeMarkers(state) {
      if (!hostElement) {
        return;
      }
      setMarker(MARKER_STATE, state || (visible ? "mounted" : "hidden"));
      setMarker(MARKER_TRIGGER, "mounted");
      setMarker(MARKER_SOURCE_PATH, sourcePath || "");
      setMarker(MARKER_SELECTION_COUNT, String(selectionCount));
      setMarker(MARKER_LAST_STATUS, panelStatus || "idle");
      if (triggerButton) {
        triggerButton.setAttribute(MARKER_TRIGGER, "mounted");
      }
    }

    function setHostStatus(nextStatus, stateName) {
      panelStatus = asString(nextStatus) || "idle";
      if (!hostElement) {
        return;
      }
      writeMarkers(stateName || (visible ? "mounted" : "hidden"));
      if (visible) {
        renderPanel();
      }
    }

    function sanitizeEnvelope(envelope) {
      if (!isObject(envelope)) {
        return {};
      }
      const output = {};
      for (const [key, value] of Object.entries(envelope)) {
        if (RAW_FIELD_KEYS.has(key)) {
          continue;
        }
        if (key === "target" || key === "status" || key === "focus" || key === "focus_node") {
          output[key] = value;
          continue;
        }
        if (key === "items" || key === "diagnostics") {
          output[key] = asArray(value);
          continue;
        }
        if (key === "source_summary") {
          continue;
        }
      }
      return output;
    }

    function updateSelection(nextSelection = {}) {
      if (!isObject(nextSelection)) {
        selection = normalizeSelection({});
      } else {
        selection = normalizeSelection(nextSelection);
      }
      selectionCount = selection.selected_component_ids.length;
      sourcePath = asString(selection.source_path);

      writeMarkers();
      if (mounted && visible) {
        renderPanel();
      }
      return {
        selection,
      };
    }

    function updatePanel(envelope = {}) {
      if (!isObject(envelope)) {
        lastDiagnostic = makePanelDiagnostic(
          "PANEL_HOST_INVALID_PAYLOAD",
          "updatePanel payload must be an object",
        );
        return lastDiagnostic;
      }

      lastEnvelope = sanitizeEnvelope(envelope);
      setHostStatus(asString(lastEnvelope.status) || "ready");
      lastDiagnostic = null;
      renderPanel();

      if (lastDiagnostic) {
        return lastDiagnostic;
      }
      return {
        status: lastEnvelope.status || "ready",
        target: lastEnvelope.target ?? null,
        items: asArray(lastEnvelope.items),
        diagnostics: asArray(lastEnvelope.diagnostics),
      };
    }

    function ensureHostErrorState(code, message) {
      mounted = false;
      visible = false;
      hostElement = null;
      shadowRoot = null;
      triggerButton = null;
      panelElement = null;
      contentElement = null;
      panelStatus = "error";
      lastDiagnostic = makePanelDiagnostic(code, message);
      return lastDiagnostic;
    }

    function buildHostTree(documentLike) {
      const host = documentLike.createElement("div");
      host.className = "metadata-checker-panel-host";

      if (typeof host.attachShadow !== "function") {
        return makePanelDiagnostic("PANEL_HOST_SHADOW_UNSUPPORTED", "Shadow DOM is not supported");
      }

      const shadow = host.attachShadow({ mode: "open" });
      const trigger = documentLike.createElement("button");
      const panel = documentLike.createElement("section");
      const content = documentLike.createElement("div");

      trigger.type = "button";
      trigger.className = "metadata-checker-panel-trigger";
      trigger.textContent = "Metadata";
      trigger.style.position = "fixed";
      trigger.style.right = "16px";
      trigger.style.bottom = "16px";
      trigger.style.zIndex = "2147483647";

      panel.className = "metadata-checker-panel-body";
      panel.style.position = "fixed";
      panel.style.right = "16px";
      panel.style.bottom = "56px";
      panel.style.zIndex = "2147483647";
      panel.style.width = "360px";
      panel.style.maxWidth = "calc(100vw - 32px)";
      panel.style.maxHeight = "60vh";
      panel.style.overflow = "auto";
      panel.style.padding = "12px";
      panel.style.boxSizing = "border-box";
      panel.style.background = "#ffffff";
      panel.style.color = "#202124";
      panel.style.border = "1px solid rgba(60, 64, 67, 0.24)";
      panel.style.borderRadius = "8px";
      panel.style.boxShadow = "0 8px 24px rgba(60, 64, 67, 0.24)";
      panel.style.font = "12px/1.5 -apple-system, BlinkMacSystemFont, \"Segoe UI\", sans-serif";
      panel.style.whiteSpace = "pre-wrap";
      content.className = "metadata-checker-panel-content";
      content.style.whiteSpace = "pre-wrap";

      panel.appendChild(content);
      shadow.appendChild(trigger);
      shadow.appendChild(panel);

      return {
        host,
        shadow,
        trigger,
        panel,
        content,
      };
    }

    function mountPanel(opts = {}) {
      const nextDocument = opts.rootDocument || globalThis.document;
      if (mounted && hostElement && nextDocument && documentLike === nextDocument) {
        if (opts.initialState) {
          if (opts.initialState.selection) {
            updateSelection(opts.initialState.selection);
          }
          if (opts.initialState.envelope) {
            updatePanel(opts.initialState.envelope);
          }
          if (typeof opts.initialState.status === "string") {
            setHostStatus(opts.initialState.status);
          }
        }
        writeMarkers();
        return {
          mounted: true,
          alreadyMounted: true,
          state: getState(),
        };
      }

      if (!nextDocument || typeof nextDocument.createElement !== "function") {
        return {
          mounted: false,
          error: ensureHostErrorState(
            "PANEL_HOST_DOCUMENT_INVALID",
            "rootDocument or document is required",
          ),
        };
      }

      if (!nextDocument.body || typeof nextDocument.body.appendChild !== "function") {
        return {
          mounted: false,
          error: ensureHostErrorState(
            "PANEL_HOST_BODY_MISSING",
            "document.body is unavailable for panel mount",
          ),
        };
      }

      const built = buildHostTree(nextDocument);
      if (built.diagnostics) {
        const err = built;
        return {
          mounted: false,
          error: err,
        };
      }

      documentLike = nextDocument;
      const { host, shadow, trigger, panel, content } = built;
      hostElement = host;
      shadowRoot = shadow;
      triggerButton = trigger;
      panelElement = panel;
      contentElement = content;

      triggerButton.addEventListener("click", () => {
        togglePanel();
      });

      nextDocument.body.appendChild(hostElement);
      mounted = true;
      visible = false;
      selection = normalizeSelection(opts.initialState?.selection || {});
      selectionCount = selection.selected_component_ids.length;
      sourcePath = asString(selection.source_path);
      if (opts.initialState?.envelope) {
        updatePanel(opts.initialState.envelope);
      }
      if (typeof opts.initialState?.status === "string") {
        panelStatus = opts.initialState.status;
      } else {
        panelStatus = "idle";
      }
      writeMarkers("hidden");
      renderPanel();
      panelElement.style.display = "none";

      return {
        mounted: true,
        alreadyMounted: false,
        state: getState(),
      };
    }

    function togglePanel(forceVisible) {
      if (!mounted) {
        return {
          visible: false,
          error: makePanelDiagnostic("PANEL_HOST_NOT_MOUNTED", "panel host is not mounted"),
        };
      }
      visible = typeof forceVisible === "boolean" ? forceVisible : !visible;
      writeMarkers(visible ? "mounted" : "hidden");
      renderPanel();
      return { visible };
    }

    function unmountPanel() {
      if (!mounted || !hostElement || !documentLike || !documentLike.body) {
        mounted = false;
        hostElement = null;
        shadowRoot = null;
        triggerButton = null;
        panelElement = null;
        contentElement = null;
        return { mounted: false };
      }

      if (typeof hostElement.remove === "function") {
        hostElement.remove();
      } else {
        const index = documentLike.body.children.indexOf(hostElement);
        if (index >= 0) {
          documentLike.body.children.splice(index, 1);
        }
      }
      mounted = false;
      visible = false;
      hostElement = null;
      shadowRoot = null;
      triggerButton = null;
      panelElement = null;
      contentElement = null;
      return { mounted: false };
    }

    function setPanelStatus(status) {
      const normalized = asString(status) || "idle";
      panelStatus = normalized;
      writeMarkers(visible ? "mounted" : "hidden");
      if (mounted && visible) {
        renderPanel();
      }
      return { status: panelStatus };
    }

    function getState() {
      return {
        mounted,
        visible,
        panelStatus,
        sourcePath,
        selectionCount,
        selection,
        lastEnvelope,
        lastDiagnostic,
        hostElement,
        shadowRoot,
        triggerButton,
        panelElement,
        contentElement,
      };
    }

    return {
      mountPanel,
      unmountPanel,
      updatePanel,
      setPanelStatus,
      togglePanel,
      updateSelection,
      getState,
    };
  }

  if (typeof module !== "undefined" && typeof module.exports !== "undefined") {
    module.exports = { createPanelHost };
  } else {
    root.__metadata_checker_panel_host_factory__ = createPanelHost;
  }
})(
  typeof globalThis !== "undefined" ? globalThis : this,
);

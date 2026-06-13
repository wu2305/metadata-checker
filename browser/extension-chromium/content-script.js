/* M43 Chromium content bootstrap */

(function installMetadataCheckerChromiumBridge(root) {
  "use strict";

  const runtime = typeof chrome !== "undefined" ? chrome.runtime : null;

  const PANEL_SCRIPT_PATHS = [
    "extension-core/bridge-protocol.js",
    "extension-core/runtime-adapter.js",
    "extension-core/page-script.js",
  ];

  const sharedState = root.__metadata_checker_chromium_content_state__
    || (root.__metadata_checker_chromium_content_state__ = {
      panelHost: null,
      mounted: false,
      mountResult: null,
      runtimeListenerBound: false,
      windowListenerBound: false,
    });

  const BRIDGE_READY_MARKER = "bridgeReady";
  const SELECTION_CHANGED_MARKER = "metadata-checker-selection-changed";
  const RUNTIME_MESSAGE_TIMEOUT_MS = 30000;
  const DEFAULT_GRAPH_DEPTH = 2;
  const DEFAULT_VISIBLE_HOP = 1;
  const MARKER_STATUS_IDLE = "idle";
  const MARKER_STATUS_LOADING = "loading";
  const MARKER_STATUS_READY = "ready";
  const MARKER_STATUS_EMPTY = "empty";
  const MARKER_STATUS_ERROR = "error";
  const MARKER_STATUS_PINNED = "pinned";
  const LOCAL_GRAPH_MESSAGE_TYPE = "metadata-checker-analyze-local-graph";
  const OFFSCREEN_LOCAL_GRAPH_MESSAGE_TYPE = "metadata-checker-offscreen-local-graph";
  const LOCAL_GRAPH_ARTIFACT_MISSING_CODE = "METADATA_CHECKER_LOCAL_GRAPH_ARTIFACT_MISSING";
  const LOCAL_GRAPH_RUNTIME_UNAVAILABLE_CODE = "METADATA_CHECKER_LOCAL_GRAPH_RUNTIME_UNAVAILABLE";
  const ANALYZE_LOCAL_GRAPH_UNSUPPORTED_CODE = "METADATA_CHECKER_ANALYZE_LOCAL_GRAPH_UNSUPPORTED";
  const LOCAL_GRAPH_ANALYSIS_RUNTIME_UNAVAILABLE_CODE = "LOCAL_GRAPH_ANALYSIS_RUNTIME_UNAVAILABLE";
  const RUNTIME_NOT_INITIALIZED_CODE = "RUNTIME_NOT_INITIALIZED";
  const SELECTION_ANALYSIS_DEBOUNCE_MS = 40;

  const sharedSelectionState = {
    markerSeq: 0,
    markerSeqInFlight: 0,
    debounceTimer: null,
  };

  sharedState.remoteSession = sharedState.remoteSession || null;
  sharedState.visibleIndex = sharedState.visibleIndex || null;
  sharedState.graphPanelHostPromise = sharedState.graphPanelHostPromise || null;
  sharedState.graphPinned = sharedState.graphPinned || false;

  function isObject(value) {
    return value !== null && typeof value === "object";
  }

  function asArray(value) {
    return Array.isArray(value) ? value : [];
  }

  function asString(value) {
    return typeof value === "string" ? value : "";
  }

  function asNumber(value, fallback = 0) {
    if (typeof value === "number" && Number.isFinite(value)) {
      return value;
    }
    if (typeof value === "string" && value.length > 0) {
      const parsed = Number.parseInt(value, 10);
      if (Number.isFinite(parsed)) {
        return parsed;
      }
    }
    return fallback;
  }

  function asDiagnostics(value) {
    return asArray(value).filter(
      (item) => isObject(item) && typeof item.code === "string" && typeof item.message === "string",
    );
  }

  function hasDiagnosticCode(value, code) {
    return asDiagnostics(value).some((entry) => entry.code === code);
  }

  function hasDiagnosticInResult(result, code) {
    return (
      hasDiagnosticCode(result?.diagnostics, code)
      || hasDiagnosticCode(asObject(result?.artifact)?.diagnostics, code)
      || hasDiagnosticCode(asObject(result?.artifact)?.result?.diagnostics, code)
      || hasDiagnosticCode(asObject(result?.foreground_artifact)?.diagnostics, code)
      || hasDiagnosticCode(asObject(result?.foreground_artifact)?.result?.diagnostics, code)
      || hasDiagnosticCode(asObject(result?.result)?.diagnostics, code)
    );
  }

  function asObject(value) {
    return isObject(value) ? value : null;
  }

  function coerceStatus(value) {
    if (typeof value !== "string") {
      return "";
    }
    const status = value.toLowerCase();
    return status;
  }

  function extractForegroundArtifact(result) {
    const directResult = asObject(result);
    const explicit = asObject(directResult.foreground_artifact) || asObject(directResult.artifact);
    if (explicit) {
      return explicit;
    }
    const candidate = asObject(directResult.result);
    if (!candidate) {
      return null;
    }
    if (
      typeof candidate.analysis_status === "string"
      || typeof candidate.kind === "string"
      || typeof candidate.source_path === "string"
      || typeof candidate.target === "string"
      || Array.isArray(candidate.items)
      || Array.isArray(candidate.diagnostics)
    ) {
      return candidate;
    }
    return null;
  }

  function panelStatusFromBackgroundState(background) {
    const status = coerceStatus(background?.status);
    const indexingStatus = coerceStatus(background?.indexing_status);
    if (
      status === "running"
      || status === "queued"
      || status === "indexing_current_page"
      || status === "waiting_for_metadata"
      || indexingStatus === "indexing_current_page"
      || indexingStatus === "waiting_for_metadata"
      || indexingStatus === "indexing_background"
    ) {
      return "analyzing";
    }
    if (status === "completed" || status === "idle") {
      return "ready";
    }
    return "";
  }

  function panelStatusFromAnalysisArtifact(artifact) {
    const artifactStatus = coerceStatus(artifact?.analysis_status || artifact?.status);
    if (artifactStatus === "runtime_unavailable" || artifactStatus === "error") {
      return "error";
    }
    if (artifactStatus === "ready") {
      return "ready";
    }
    if (artifactStatus) {
      return "analyzing";
    }
    const resultStatus = coerceStatus(artifact?.result?.analysis_status || artifact?.result?.status);
    if (resultStatus === "error") {
      return "error";
    }
    if (resultStatus === "ready") {
      return "ready";
    }
    if (resultStatus) {
      return "analyzing";
    }
    return "";
  }

  function resolvePanelTarget(result, selectionSourcePath, foregroundArtifact) {
    return asString(
      result?.target
      || result?.source_path
      || selectionSourcePath
      || asObject(foregroundArtifact)?.source_path
      || asObject(foregroundArtifact)?.result?.source_path
      || asObject(foregroundArtifact)?.target
      || result?.state?.visible_index?.files?.[0]?.source_path
      || result?.visible_index?.files?.[0]?.source_path,
    );
  }

  function collectForegroundItems(foregroundArtifact) {
    if (!isObject(foregroundArtifact)) {
      return [];
    }
    const directItems = asArray(foregroundArtifact.items);
    const nestedItems = asArray(foregroundArtifact.result?.items);
    if (directItems.length > 0 || nestedItems.length > 0) {
      return directItems.concat(nestedItems);
    }
    const sourcePath = asString(
      foregroundArtifact.source_path || foregroundArtifact.result?.source_path || foregroundArtifact.target,
    );
    if (!sourcePath) {
      return [];
    }
    return [
      {
        kind: "foreground_artifact",
        label: "Foreground analysis artifact",
        detail: {
          source_path: sourcePath,
          analysis_status: asString(foregroundArtifact.analysis_status || foregroundArtifact.status || ""),
        },
      },
    ];
  }

  function writeMarker(name, value) {
    const doc = root.document;
    if (!doc || typeof doc.createElement !== "function") {
      return;
    }
    const key = `data-metadata-checker-${name}`;
    const existing = typeof doc.querySelector === "function" ? doc.querySelector(`[${key}]`) : null;
    if (existing && typeof existing.setAttribute === "function") {
      existing.setAttribute(key, value);
      return;
    }
    const marker = doc.createElement("span");
    marker.setAttribute(key, value);
    marker.style.display = "none";
    doc.documentElement?.appendChild(marker);
  }

  function isPinnedSelection(payload) {
    return (
      payload?.pinned === true
      || payload?.pin === true
      || payload?.pinned === "true"
      || payload?.pin === "true"
    );
  }

  function normalizeSelectionForStatus(payload) {
    const sourcePath = asString(payload?.source_path);
    const activeComponentId = asString(payload?.active_component_id);
    const selectedComponentIds = asArray(payload?.selected_component_ids);
    return {
      sourcePath,
      activeComponentId,
      selectedCount: selectedComponentIds.length,
      isPinned: isPinnedSelection(payload),
      hasSourcePath: sourcePath.length > 0,
      hasSelection: selectedComponentIds.length > 0 || activeComponentId.length > 0,
      timestamp: asNumber(payload?.timestamp, Date.now()),
    };
  }

  function isVisualGraphLike(value) {
    return Boolean(
      value
      && typeof value === "object"
      && Array.isArray(value.nodes)
      && Array.isArray(value.edges),
    );
  }

  function findVisualGraph(value, seen = new Set()) {
    if (!value || typeof value !== "object" || seen.has(value)) {
      return null;
    }
    seen.add(value);
    if (Array.isArray(value)) {
      for (const item of value) {
        const graph = findVisualGraph(item, seen);
        if (graph) {
          return graph;
        }
      }
      return null;
    }
    if (isVisualGraphLike(value)) {
      return value;
    }
    const candidates = [
      value.graph,
      value.visual_graph,
      value.visualGraph,
      value.node_graph,
      value.result,
      value.artifact,
      value.foreground_artifact,
      value.detail,
      value.payload,
    ];
    for (const candidate of candidates) {
      const graph = findVisualGraph(candidate, seen);
      if (graph) {
        return graph;
      }
    }
    for (const item of asArray(value.items)) {
      const graph = findVisualGraph(item, seen);
      if (graph) {
        return graph;
      }
    }
    for (const child of Object.values(value)) {
      const graph = findVisualGraph(child, seen);
      if (graph) {
        return graph;
      }
    }
    return null;
  }

  function normalizeGraphSummary(result) {
    const direct = asObject(result?.foreground_artifact) ?? asObject(result?.artifact) ?? asObject(result?.result);
    const graph = findVisualGraph(result) ?? findVisualGraph(direct);
    return {
      depth: asNumber(graph?.depth ?? direct?.depth ?? direct?.maxDepth ?? result?.depth ?? direct?.depth_level, DEFAULT_GRAPH_DEPTH),
      visibleHop: asNumber(graph?.visible_hop ?? graph?.visibleHop ?? direct?.visible_hop ?? direct?.visibleHop ?? result?.visible_hop ?? result?.visibleHop, DEFAULT_VISIBLE_HOP),
      focus: asString(graph?.focus_node ?? graph?.target ?? direct?.focus_node ?? direct?.target ?? asString(result?.target) ?? ""),
      nodeCount: asNumber(asArray(graph?.nodes).length, asNumber(direct?.nodeCount, asNumber(direct?.nodes?.length, 0))),
      edgeCount: asNumber(asArray(graph?.edges).length, asNumber(direct?.edgeCount, asNumber(direct?.edges?.length, 0))),
      hasGraph: graph ? true : false,
    };
  }

  function inferEmbeddedPopupStateFromSelection(selection) {
    if (!selection.hasSourcePath) {
      return "hidden";
    }
    if (selection.hasSelection) {
      return "mounted";
    }
    return "collapsed";
  }

  function inferPopupStatusFromResult(result, selection) {
    if (!selection.hasSourcePath) {
      return MARKER_STATUS_IDLE;
    }
    if (selection.isPinned) {
      return MARKER_STATUS_PINNED;
    }
    if (!selection.hasSelection) {
      return MARKER_STATUS_EMPTY;
    }
    const artifact = asObject(result?.foreground_artifact)
      || asObject(result?.artifact)
      || asObject(result?.result)
      || asObject(result);
    if (!artifact) {
      return MARKER_STATUS_ERROR;
    }
    const status = coerceStatus(artifact?.analysis_status ?? artifact?.status);
    if (status === "running" || status === "analyzing" || status === "loading") {
      return MARKER_STATUS_LOADING;
    }
    if (status === "error") {
      return MARKER_STATUS_ERROR;
    }
    if (status === "empty" || status === "") {
      const summary = normalizeGraphSummary(artifact);
      if (summary.nodeCount === 0 && summary.edgeCount === 0) {
        return MARKER_STATUS_EMPTY;
      }
      return MARKER_STATUS_READY;
    }
    if (result?.ok === false) {
      return MARKER_STATUS_ERROR;
    }
    return MARKER_STATUS_READY;
  }

  function writeSelectionStatusMarkers(selection, status, options = {}) {
    const summary = options.summary || {};
    const focus = asString(
      options.focus ??
      selection.activeComponentId ??
      summary.focus ??
      "",
    );
    writeMarker("embedded-popup", inferEmbeddedPopupStateFromSelection(selection));
    writeMarker("analysis-status", status);
    writeMarker("focus-component", focus);
    writeMarker(
      "graph-depth",
      String(asNumber(summary.depth, DEFAULT_GRAPH_DEPTH)),
    );
    writeMarker(
      "graph-visible-hop",
      String(asNumber(summary.visibleHop, DEFAULT_VISIBLE_HOP)),
    );
    writeMarker("graph-node-count", String(asNumber(summary.nodeCount, 0)));
    writeMarker("graph-edge-count", String(asNumber(summary.edgeCount, 0)));
    writeMarker("local-graph-status-seq", String(sharedSelectionState.markerSeq));
  }

  function writeLocalGraphTimingMarkers(timings = {}) {
    const keys = [
      "ensure_runtime_ms",
      "fetch_metadata_ms",
      "init_runtime_ms",
      "load_document_ms",
      "build_graph_ms",
      "analyze_local_graph_ms",
      "layout_ms",
      "render_ms",
      "total_ms",
      "manifest_fetch_ms",
      "manifest_diff_ms",
      "changed_content_fetch_ms",
      "wasm_update_ms",
    ];
    for (const key of keys) {
      if (timings[key] !== undefined && timings[key] !== null) {
        writeMarker(`local-graph-timing-${key.replaceAll("_", "-")}`, String(timings[key]));
      }
    }
  }

  function writeManifestRefreshMarkers(result = {}) {
    const diff = asObject(result.manifest_diff) || {};
    const timings = asObject(result.timings) || {};
    const fields = {
      "manifest-added": diff.added,
      "manifest-modified": diff.modified,
      "manifest-deleted": diff.deleted,
      "manifest-unchanged": diff.unchanged,
      "content-queue-count": diff.content_queue_count,
      "local-graph-manifest-added": diff.added,
      "local-graph-manifest-modified": diff.modified,
      "local-graph-manifest-deleted": diff.deleted,
      "local-graph-manifest-unchanged": diff.unchanged,
      "local-graph-content-queue-count": diff.content_queue_count,
    };
    for (const [name, value] of Object.entries(fields)) {
      writeMarker(name, value === undefined || value === null ? "" : String(value));
    }
    writeLocalGraphTimingMarkers(timings);
    writeMarker("manifest-refresh-status", result?.ok === false ? "error" : "ready");
    writeMarker("manifest-refresh-diagnostic-code", asDiagnostics(result?.diagnostics)[0]?.code || "");
  }

  function stableDiagnostic(code, message, severity = "warning") {
    return {
      severity,
      code,
      message,
    };
  }

  function getPanelHostFactory() {
    if (typeof root.__metadata_checker_panel_host_factory__ === "function") {
      return root.__metadata_checker_panel_host_factory__;
    }
    return null;
  }

  function getPanelHost() {
    if (sharedState.panelHost) {
      return sharedState.panelHost;
    }
    const factory = getPanelHostFactory();
    const host = typeof factory === "function" ? factory() : null;
    sharedState.panelHost = host;
    return host;
  }

  async function getGraphPanelHost() {
    if (sharedState.graphPanelHostPromise) {
      return sharedState.graphPanelHostPromise;
    }
    if (!runtime || typeof runtime.getURL !== "function") {
      return null;
    }
    sharedState.graphPanelHostPromise = (async () => {
      const [
        graphHostModule,
        graphRendererModule,
        vendorLoaderModule,
      ] = await Promise.all([
        import(runtime.getURL("spike-renderer/graph-panel-host.mjs")),
        import(runtime.getURL("spike-renderer/graph-panel-renderer.mjs")),
        import(runtime.getURL("spike-renderer/extension-vendor-runtime-loader.mjs")),
      ]);
      const vendorRuntimeLoader = vendorLoaderModule.createExtensionVendorRuntimeLoader({ runtime });
      let echartsRuntime = null;
      try {
        echartsRuntime = await vendorRuntimeLoader.loadEcharts();
      } catch (error) {
        writeMarker(
          "graph-echarts-vendor-error",
          error?.message || "extension echarts vendor preload failed",
        );
      }
      writeMarker("graph-echarts-vendor", echartsRuntime ? "loaded" : "missing");
      if (!echartsRuntime && typeof vendorRuntimeLoader.getLastLoadError === "function") {
        const loadError = vendorRuntimeLoader.getLastLoadError();
        if (loadError) {
          writeMarker("graph-echarts-vendor-error", loadError);
        }
      }
      const renderer = graphRendererModule.createGraphPanelRenderer({
        document: root.document,
        renderer: "auto",
        runtime,
        vendorRuntimeLoader,
        echarts: echartsRuntime,
      });
      return graphHostModule.createGraphPanelHost({
        document: root.document,
        parent: root.document?.body,
        renderer,
        onPinChange: async ({ pinned }) => {
          sharedState.graphPinned = Boolean(pinned);
          writeMarker("graph-panel-pinned", sharedState.graphPinned ? "true" : "false");
        },
        onCopyGraph: async ({ text }) => {
          writeMarker("graph-copy-text-length", String((text || "").length));
        },
      });
    })().catch((error) => {
      sharedState.graphPanelHostPromise = null;
      writeMarker("local-graph-renderer", "error");
      writeMarker("local-graph-renderer-diagnostic-code", "GRAPH_PANEL_IMPORT_FAILED");
      writeMarker("local-graph-renderer-diagnostic-message", error?.message || "graph panel import failed");
      return null;
    });
    return sharedState.graphPanelHostPromise;
  }

  function syncLegacyPanelHostVisibility(visible) {
    const fallbackHost = getPanelHost();
    if (!fallbackHost) {
      return;
    }
    if (typeof fallbackHost.setHostVisible === "function") {
      fallbackHost.setHostVisible(Boolean(visible));
      return;
    }
    if (typeof fallbackHost.togglePanel === "function") {
      fallbackHost.togglePanel(false);
    }
    const hostElement = fallbackHost.getState?.()?.hostElement;
    if (hostElement?.style) {
      hostElement.style.display = visible ? "" : "none";
    }
  }

  async function ensureGraphPanelLoadingShell(summary = {}) {
    const host = await getGraphPanelHost();
    if (!host) {
      return null;
    }
    if (typeof host.showShell === "function") {
      return host.showShell(summary);
    }
    if (typeof host.render === "function") {
      return host.render({
        status: "loading",
        focus_node: summary.focus ?? "",
        depth: summary.depth ?? DEFAULT_GRAPH_DEPTH,
        visible_hop: summary.visibleHop ?? DEFAULT_VISIBLE_HOP,
        nodes: [],
        edges: [],
        groups: [],
        diagnostics: [],
        truncated: false,
        source_summary: {
          total_nodes: 0,
          total_edges: 0,
          node_kinds: {},
          edge_kinds: {},
        },
      });
    }
    return null;
  }

  async function renderGraphPanelIfAvailable(result) {
    const graph = findVisualGraph(result);
    if (!graph) {
      syncLegacyPanelHostVisibility(true);
      return null;
    }
    const host = await getGraphPanelHost();
    if (!host || typeof host.render !== "function") {
      syncLegacyPanelHostVisibility(true);
      return null;
    }
    const renderStartedAt = typeof performance !== "undefined" ? performance.now() : Date.now();
    const renderResult = await host.render(graph);
    const renderEndedAt = typeof performance !== "undefined" ? performance.now() : Date.now();
    writeLocalGraphTimingMarkers({
      render_ms: Math.max(0, Math.round(renderEndedAt - renderStartedAt)),
    });
    syncLegacyPanelHostVisibility(false);
    return renderResult;
  }

  function mountPanelHost() {
    const host = getPanelHost();
    if (!host || typeof host.mountPanel !== "function") {
      return {
        mounted: false,
        error: {
          status: "error",
          diagnostics: [
            stableDiagnostic("PANEL_HOST_FACTORY_MISSING", "panel host factory is unavailable"),
          ],
        },
      };
    }

    const result = host.mountPanel({
      rootDocument: root.document,
    });
    if (result?.mounted) {
      sharedState.mounted = true;
    }
    sharedState.mountResult = result;
    return result;
  }

  function writeBridgeProbeMarkers(response) {
    const payload = response?.payload || response || {};
    const pageContext = payload.page_context || {};
    const selection = payload.selection || {};
    const diagnostics = asDiagnostics(response?.diagnostics).concat(asDiagnostics(payload.diagnostics));
    writeMarker("extension-bridge-request", payload.bridge_detected === false ? "missing" : "ready");
    writeMarker("extension-bridge-source-path", pageContext.source_path || pageContext.file_id || "");
    writeMarker(
      "extension-bridge-selection-count",
      String(Array.isArray(selection.selected_component_ids) ? selection.selected_component_ids.length : 0),
    );
    writeMarker("extension-bridge-diagnostic-code", diagnostics[0]?.code || "");
  }

  function applyPanelStateFromBridge(response, requestType) {
    const host = getPanelHost();
    if (!host || typeof host.updatePanel !== "function") {
      return;
    }

    const payload = response?.payload || {};
    const diagnostics = asDiagnostics(response?.diagnostics);
    const bridgeMissing = payload.bridge_detected === false
      || diagnostics.some((item) => item.code === "METADATA_CHECKER_BRIDGE_MISSING")
      || diagnostics.some((item) => item.code === "METADATA_CHECKER_CONTENT_BRIDGE_MISSING");

    if (payload.selection && typeof host.updateSelection === "function") {
      host.updateSelection(payload.selection);
    }

    if (requestType === "analyzeCurrentSelection") {
      const status = bridgeMissing || payload.supported === false ? "error" : "ready";
      host.updatePanel({
        status,
        target: payload.target ?? payload.page_context?.source_path ?? null,
        items: asArray(payload.items),
        diagnostics,
      });
      if (typeof host.setPanelStatus === "function") {
        host.setPanelStatus(status);
      }
      return;
    }

    host.updatePanel({
      status: bridgeMissing ? "error" : "ready",
      target: payload.target ?? payload.page_context?.source_path ?? null,
      items: asArray(payload.items),
      diagnostics,
    });
    if (typeof host.setPanelStatus === "function") {
      host.setPanelStatus(bridgeMissing ? "error" : "ready");
    }
  }

  function updatePanelWithBackgroundState(result) {
    const foregroundArtifact = extractForegroundArtifact(result);
    const diagnostics = asDiagnostics(result?.diagnostics)
      .concat(asDiagnostics(foregroundArtifact?.diagnostics))
      .concat(asDiagnostics(foregroundArtifact?.result?.diagnostics));
    const backgroundStatus = asObject(result?.background);
    const hasErrorDiagnostic = diagnostics.some((entry) => entry.severity === "error");
    const artifactItems = collectForegroundItems(foregroundArtifact);

    const statusFromArtifact = panelStatusFromAnalysisArtifact(foregroundArtifact);
    const statusFromBackground = panelStatusFromBackgroundState(backgroundStatus);
    const status =
      result?.ok === false || hasErrorDiagnostic
        ? "error"
        : statusFromArtifact || statusFromBackground || "ready";

    const host = getPanelHost();
    if (!host || typeof host.updatePanel !== "function") {
      return;
    }
    if (result?.ok === false) {
      const fallbackDiagnostics = diagnostics.length > 0
        ? diagnostics
        : [
          stableDiagnostic(
            "SESSION_BOOTSTRAP_FAILED",
            "remote metadata session bootstrap failed without diagnostics",
            "error",
          ),
        ];
      const fallbackTarget = asString(
        result?.source_path
        || result?.current_source_path
        || result?.state?.background?.current_source_path
        || result?.background?.current_source_path,
      );
      host.updatePanel({
        status: "error",
        target: fallbackTarget || null,
        items: [],
        diagnostics: fallbackDiagnostics,
        background: result.background ?? null,
      });
      return;
    }
    const visibleIndex = result?.visible_index || result?.state?.visible_index || {};
    const background = backgroundStatus || result?.state?.background || {};
    const cacheStats = result?.state?.cache_stats || result?.cache_stats || {};
    const sourcePath = resolvePanelTarget(
      result,
      asString(result?.source_path),
      foregroundArtifact,
    );
    host.updatePanel({
      status,
      target: sourcePath || visibleIndex.files?.[0]?.source_path || null,
      items: [
        ...artifactItems,
        {
          kind: "background_status",
          label: "Remote Metadata Background Status",
          detail: {
            project_count: Array.isArray(visibleIndex.projects) ? visibleIndex.projects.length : 0,
            file_count: Array.isArray(visibleIndex.files) ? visibleIndex.files.length : 0,
            analyzable_count: visibleIndex.analyzable_count ?? 0,
            processed: background.processed ?? 0,
            total: background.total ?? 0,
            failed: background.failed ?? 0,
            current_source_path: background.current_source_path ?? null,
            last_processed_source_path: background.last_processed_source_path ?? null,
            last_failed_source_path: background.last_failed_source_path ?? null,
            indexing_status: background.indexing_status ?? background.status ?? "idle",
            retry_available: Boolean(background.retry_available),
            cache_hits: cacheStats.hits ?? 0,
            cache_misses: cacheStats.misses ?? 0,
          },
        },
      ],
      diagnostics,
      background,
      cache_stats: cacheStats,
    });
  }

  function forwardStatus(message) {
    if (!runtime || typeof runtime.sendMessage !== "function") {
      return;
    }
    try {
      runtime.sendMessage({
        type: "metadata-checker-bridge-status",
        payload: message,
      });
    } catch {
      // Content scripts may run before the extension service worker is ready.
    }
  }

  function sendRuntimeMessage(message) {
    if (!runtime || typeof runtime.sendMessage !== "function") {
      return Promise.resolve({ ok: false });
    }
    return new Promise((resolve) => {
      let settled = false;
      let timeoutId = null;
      const finish = (value) => {
        if (!settled) {
          settled = true;
          if (timeoutId !== null && typeof root.clearTimeout === "function") {
            root.clearTimeout(timeoutId);
          }
          resolve(value);
        }
      };
      try {
        if (typeof root.setTimeout === "function") {
          timeoutId = root.setTimeout(() => {
            finish({
              ok: false,
              diagnostics: [
                stableDiagnostic(
                  "METADATA_CHECKER_EXTENSION_BACKGROUND_TIMEOUT",
                  "extension background did not respond in time",
                  "warning",
                ),
              ],
            });
          }, RUNTIME_MESSAGE_TIMEOUT_MS);
        }
        const maybePromise = runtime.sendMessage(message, (response) => {
          finish(response || { ok: false });
        });
        if (maybePromise && typeof maybePromise.then === "function") {
          maybePromise.then(finish, (error) => {
            finish({
              ok: false,
              diagnostics: [
                stableDiagnostic(
                  "METADATA_CHECKER_EXTENSION_BACKGROUND_UNAVAILABLE",
                  error?.message || "extension background is unavailable",
                  "warning",
                ),
              ],
            });
          });
        }
      } catch (error) {
        finish({
          ok: false,
          diagnostics: [
            stableDiagnostic(
              "METADATA_CHECKER_EXTENSION_BACKGROUND_UNAVAILABLE",
              error?.message || "extension background is unavailable",
              "warning",
            ),
          ],
        });
      }
    });
  }

  async function fetchAccessTokenInContentWorld() {
    if (typeof root.fetch !== "function") {
      return {
        ok: false,
        diagnostics: [
          stableDiagnostic(
            "ACCESS_TOKEN_UNAVAILABLE",
            "native fetch is unavailable in content script",
            "error",
          ),
        ],
      };
    }
    try {
      const response = await root.fetch("/api/auth/getAccessToken", {
        method: "GET",
        credentials: "include",
      });
      if (!response || !response.ok) {
        return {
          ok: false,
          diagnostics: [
            stableDiagnostic(
              "ACCESS_TOKEN_UNAVAILABLE",
              `getAccessToken failed with HTTP ${response?.status ?? "unknown"}`,
              "error",
            ),
          ],
        };
      }
      const accessToken = String(await response.text()).trim();
      if (!accessToken) {
        return {
          ok: false,
          diagnostics: [
            stableDiagnostic(
              "ACCESS_TOKEN_UNAVAILABLE",
              "getAccessToken returned empty token",
              "error",
            ),
          ],
        };
      }
      return { ok: true, access_token: accessToken, diagnostics: [] };
    } catch (error) {
      return {
        ok: false,
        diagnostics: [
          stableDiagnostic(
            "ACCESS_TOKEN_UNAVAILABLE",
            error?.message || "getAccessToken request failed",
            "error",
          ),
        ],
      };
    }
  }

  async function bootstrapRemoteSessionFromPage() {
    const [status, tokenResponse] = await Promise.all([
      requestPageBridge("getBridgeStatus"),
      fetchAccessTokenInContentWorld(),
    ]);
    const pageContext = status?.payload?.page_context || {};
    if (!tokenResponse?.ok || typeof tokenResponse.access_token !== "string") {
      writeMarker("extension-token-source", "content-script-error");
      updatePanelWithBackgroundState({
        ok: false,
        diagnostics: asDiagnostics(tokenResponse?.diagnostics),
      });
      return tokenResponse;
    }
    writeMarker("extension-token-source", "content-script");
    const result = await sendRuntimeMessage({
      type: "metadata-checker-bootstrap-token",
      payload: {
        base_url: root.location?.origin || "",
        access_token: tokenResponse.access_token,
        project_name: pageContext.project_name || pageContext.projectName || "",
        current_source_path: pageContext.source_path || "",
        initial_limit: 0,
      },
    });
    if (result?.ok) {
      sharedState.remoteSession = {
        base_url: root.location?.origin || "",
        project_name: pageContext.project_name || pageContext.projectName || "",
      };
      sharedState.visibleIndex = result.visible_index || null;
    }
    writeMarker("extension-session", result?.ok ? "ready" : "error");
    writeMarker("extension-background-index-count", String(result?.visible_index?.files?.length ?? 0));
    const diagnosticCode = asDiagnostics(result?.diagnostics)[0]?.code || "";
    writeMarker("extension-session-diagnostic-code", diagnosticCode);
    updatePanelWithBackgroundState({
      ...(result || {}),
      source_path: pageContext.source_path || "",
    });
    return result;
  }

  function resolveSelectionFromVisibleIndex(selection = {}) {
    const sourcePath = asString(selection.source_path ?? selection.sourcePath);
    const files = asArray(sharedState.visibleIndex?.files);
    const file = files.find((item) => item?.source_path === sourcePath) || {};
    const projectName = asString(
      selection.project_name
      || file.project_name
      || sharedState.remoteSession?.project_name,
    );
    return {
      ...selection,
      project_name: projectName || selection.project_name || "",
      file_id: selection.file_id || file.file_id || "",
      revision: selection.revision ?? file.revision ?? null,
      extension: selection.extension || file.extension || "",
    };
  }

  function needsSessionRebootstrap(result) {
    const foregroundArtifact = extractForegroundArtifact(result);
    if (hasDiagnosticInResult(result, LOCAL_GRAPH_ARTIFACT_MISSING_CODE)) {
      return true;
    }
    const background = asObject(result?.background) || asObject(result?.state?.background) || {};
    const indexingStatus = coerceStatus(
      result?.indexing_status
      || background.indexing_status
      || background.status,
    );
    const artifactStatus = coerceStatus(
      foregroundArtifact?.analysis_status
      || foregroundArtifact?.status
      || result?.analysis_status
      || result?.status,
    );
    return (
      indexingStatus === "waiting_for_metadata"
      || indexingStatus === "indexing_current_page"
      || artifactStatus === "waiting_for_metadata"
    ) && result?.artifact_ready !== true;
  }

  function shouldFallbackToSelectionQueue(result) {
    return (
      hasDiagnosticInResult(result, ANALYZE_LOCAL_GRAPH_UNSUPPORTED_CODE)
      || hasDiagnosticInResult(result, LOCAL_GRAPH_RUNTIME_UNAVAILABLE_CODE)
      || hasDiagnosticInResult(result, LOCAL_GRAPH_ANALYSIS_RUNTIME_UNAVAILABLE_CODE)
    );
  }

  async function sendSelectionToLegacyBackground(selection, { rebootstrapOnMissingMetadata = true } = {}) {
    return sendRuntimeMessage({
      type: "metadata-checker-selection-changed",
      payload: selection,
    });
  }

  async function sendSelectionToLegacyBackgroundWithRetry(selection, { rebootstrapOnMissingMetadata = true } = {}) {
    const result = await sendSelectionToLegacyBackground(selection);
    if (!rebootstrapOnMissingMetadata || !needsSessionRebootstrap(result)) {
      return result;
    }
    const bootstrapped = await bootstrapRemoteSessionFromPage();
    if (bootstrapped?.ok === false) {
      return bootstrapped;
    }
    return sendRuntimeMessage({
      type: "metadata-checker-selection-changed",
      payload: selection,
    });
  }

  async function loadArtifactThenAnalyzeLocalGraph(selection) {
    writeMarker("local-graph-artifact-load", "queueing");
    const resolvedSelection = resolveSelectionFromVisibleIndex(selection);
    const ensured = await sendRuntimeMessage({
      type: "metadata-checker-ensure-offscreen-runtime",
      payload: {
        project_name: resolvedSelection.project_name || sharedState.remoteSession?.project_name || "",
      },
    });
    if (ensured?.ok === false) {
      writeMarker("local-graph-artifact-load", "queue-error");
      writeMarker("local-graph-artifact-load-diagnostic-code", asDiagnostics(ensured?.diagnostics)[0]?.code || "");
      return ensured;
    }
    writeMarker("local-graph-artifact-load", "offscreen");
    const analyzedResult = await sendRuntimeMessage({
      type: OFFSCREEN_LOCAL_GRAPH_MESSAGE_TYPE,
      payload: {
        base_url: sharedState.remoteSession?.base_url || root.location?.origin || "",
        item: {
          source_path: resolvedSelection.source_path,
          project_name: resolvedSelection.project_name || sharedState.remoteSession?.project_name || "",
          file_id: resolvedSelection.file_id || "",
          revision: resolvedSelection.revision ?? null,
          extension: resolvedSelection.extension || "",
          active_component_id: resolvedSelection.active_component_id ?? null,
          selected_component_ids: asArray(resolvedSelection.selected_component_ids),
        },
        selection: resolvedSelection,
        options: {
          depth: DEFAULT_GRAPH_DEPTH,
          visible_hop: DEFAULT_VISIBLE_HOP,
        },
      },
    });
    if (shouldFallbackToSelectionQueue(analyzedResult)) {
      writeMarker("local-graph-artifact-load", "runtime-fallback");
      return sendSelectionToLegacyBackgroundWithRetry(selection, {
        rebootstrapOnMissingMetadata: false,
      });
    }
    writeMarker("local-graph-artifact-load", analyzedResult?.ok === false ? "analyze-error" : "offscreen-analyzed");
    writeMarker("local-graph-artifact-load-diagnostic-code", asDiagnostics(analyzedResult?.diagnostics)[0]?.code || "");
    if (analyzedResult?.timings) {
      writeLocalGraphTimingMarkers(analyzedResult.timings);
      if (analyzedResult.timings.total_ms !== undefined) {
        writeMarker("local-graph-offscreen-total-ms", String(analyzedResult.timings.total_ms));
      }
    }
    if (analyzedResult?.cache) {
      writeMarker("local-graph-cache-metadata-hit", String(Boolean(analyzedResult.cache.metadata_cache_hit)));
      writeMarker("local-graph-cache-document-hit", String(Boolean(analyzedResult.cache.document_cache_hit)));
    }
    return analyzedResult;
  }

  async function sendSelectionToBackground(selection, { rebootstrapOnMissingMetadata = true } = {}) {
    const firstResult = await sendRuntimeMessage({
      type: LOCAL_GRAPH_MESSAGE_TYPE,
      payload: selection,
    });

    const shouldBootstrapAndRetry =
      rebootstrapOnMissingMetadata
      && (
        hasDiagnosticInResult(firstResult, LOCAL_GRAPH_ARTIFACT_MISSING_CODE)
        || needsSessionRebootstrap(firstResult)
      );
    if (shouldBootstrapAndRetry) {
      const bootstrapped = await bootstrapRemoteSessionFromPage();
      if (bootstrapped?.ok === false) {
        return bootstrapped;
      }
      const retriedResult = await sendRuntimeMessage({
        type: LOCAL_GRAPH_MESSAGE_TYPE,
        payload: selection,
      });
      if (shouldFallbackToSelectionQueue(retriedResult)) {
        return sendSelectionToLegacyBackgroundWithRetry(selection, { rebootstrapOnMissingMetadata });
      }
      if (hasDiagnosticInResult(retriedResult, LOCAL_GRAPH_ARTIFACT_MISSING_CODE)) {
        return loadArtifactThenAnalyzeLocalGraph(selection);
      }
      if (hasDiagnosticInResult(retriedResult, RUNTIME_NOT_INITIALIZED_CODE)) {
        return loadArtifactThenAnalyzeLocalGraph(selection);
      }
      return retriedResult;
    }

    if (shouldFallbackToSelectionQueue(firstResult)) {
      return sendSelectionToLegacyBackgroundWithRetry(selection, { rebootstrapOnMissingMetadata });
    }

    if (hasDiagnosticInResult(firstResult, RUNTIME_NOT_INITIALIZED_CODE)) {
      return loadArtifactThenAnalyzeLocalGraph(selection);
    }

    if (!rebootstrapOnMissingMetadata || !needsSessionRebootstrap(firstResult)) {
      return firstResult;
    }

    const bootstrapped = await bootstrapRemoteSessionFromPage();
    if (bootstrapped?.ok === false) {
      return bootstrapped;
    }
    return sendRuntimeMessage({
      type: LOCAL_GRAPH_MESSAGE_TYPE,
      payload: selection,
    });
  }

  async function requestPageBridge(requestType) {
    const bridge = root.__metadata_checker_content_bridge__;
    if (!bridge || typeof bridge.request !== "function") {
      const missing = {
        payload: {
          supported: false,
          bridge_detected: false,
        },
        diagnostics: [
          stableDiagnostic(
            "METADATA_CHECKER_CONTENT_BRIDGE_MISSING",
            "metadata checker content bridge is unavailable",
          ),
        ],
      };
      writeBridgeProbeMarkers(missing);
      applyPanelStateFromBridge(missing, requestType);
      return missing;
    }

    const response = await bridge.request(requestType);
    writeBridgeProbeMarkers(response);
    applyPanelStateFromBridge(response, requestType);
    return response;
  }

  function normalizeMessageFromPage(message) {
    if (!isObject(message)) {
      return false;
    }
    if (message.__metadata_checker_bridge_source !== "page-script") {
      return false;
    }
    if (message.type === SELECTION_CHANGED_MARKER) {
      return true;
    }
    if (message.type === BRIDGE_READY_MARKER) {
      return true;
    }
    if (message.__metadata_checker_bridge_direction !== "response") {
      return false;
    }
    if (typeof message.request_id !== "string") {
      return false;
    }
    return true;
  }

  function markSelectionFromPageMessage(message) {
    if (!isObject(message) || message.type !== SELECTION_CHANGED_MARKER) {
      return;
    }
    if (typeof message.payload !== "object") {
      return;
    }
    const host = getPanelHost();
    if (!isObject(message.payload)) {
      return;
    }
    const selectionPayload = message.payload;
    const selectionState = normalizeSelectionForStatus(selectionPayload);
    const markerSeq = ++sharedSelectionState.markerSeq;
    sharedSelectionState.markerSeqInFlight = markerSeq;

    const initialStatus = selectionState.hasSourcePath
      ? (selectionState.hasSelection ? MARKER_STATUS_LOADING : MARKER_STATUS_EMPTY)
      : MARKER_STATUS_IDLE;

    writeSelectionStatusMarkers(selectionState, initialStatus, {
      focus: asString(selectionPayload.active_component_id),
      summary: {
        focus: asString(selectionPayload.active_component_id),
        depth: DEFAULT_GRAPH_DEPTH,
        visibleHop: DEFAULT_VISIBLE_HOP,
        nodeCount: 0,
        edgeCount: 0,
      },
    });

    writeMarker("extension-selection-event", "received");
    writeMarker(
      "extension-selection-source",
      asString(selectionPayload.selection_source || (selectionPayload.isSingleSelection ? "single" : "")),
    );
    writeMarker(
      "extension-selection-count",
      String(selectionState.selectedCount),
    );
    writeMarker("extension-selection-active", asString(selectionPayload.active_component_id));
    writeMarker(
      "extension-selection-changed-at",
      String(selectionState.timestamp || ""),
    );

    if (host && typeof host.updateSelection === "function") {
      host.updateSelection(selectionPayload);
    }
    if (selectionState.isPinned || sharedState.graphPinned) {
      writeSelectionStatusMarkers(selectionState, MARKER_STATUS_PINNED, {
        focus: asString(selectionPayload.active_component_id),
      });
      host?.updatePanel?.({
        status: MARKER_STATUS_PINNED,
        target: selectionPayload.source_path,
        items: [],
        diagnostics: [{
          severity: "info",
          code: "ANALYSIS_SKIPPED_PINNED",
          message: "selection analysis skipped because popup is pinned",
        }],
      });
      return;
    }

    if (!selectionState.hasSourcePath) {
      host?.updatePanel?.({
        status: MARKER_STATUS_IDLE,
        target: null,
        items: [],
        diagnostics: [],
      });
      return;
    }

    if (!selectionState.hasSelection) {
      host?.updatePanel?.({
        status: MARKER_STATUS_EMPTY,
        target: selectionPayload.source_path,
        items: [],
        diagnostics: [],
      });
      return;
    }

    void ensureGraphPanelLoadingShell({
      focus: asString(selectionPayload.active_component_id),
      depth: DEFAULT_GRAPH_DEPTH,
      visibleHop: DEFAULT_VISIBLE_HOP,
      nodeCount: 0,
      edgeCount: 0,
    }).catch(() => {});

    if (sharedSelectionState.debounceTimer && typeof root.clearTimeout === "function") {
      root.clearTimeout(sharedSelectionState.debounceTimer);
      sharedSelectionState.debounceTimer = null;
    }
    writeMarker("local-graph-selection-debounce-ms", String(SELECTION_ANALYSIS_DEBOUNCE_MS));
    const runAnalysis = () => sendSelectionToBackground(selectionPayload).then((result) => {
      if (sharedSelectionState.markerSeqInFlight !== markerSeq) {
        writeMarker("local-graph-selection-stale", "ignored");
        return;
      }
      const status = inferPopupStatusFromResult(result, selectionState);
      const summary = normalizeGraphSummary(result);
      const details = {
        focus: summary.focus || asString(selectionPayload.active_component_id),
        depth: summary.depth || DEFAULT_GRAPH_DEPTH,
        visibleHop: summary.visibleHop,
        nodeCount: summary.nodeCount,
        edgeCount: summary.edgeCount,
      };
      updatePanelWithBackgroundState({
        ...(result || {}),
        source_path: asString(message?.payload?.source_path),
      });
      renderGraphPanelIfAvailable(result).catch((error) => {
        writeMarker("local-graph-renderer", "error");
        writeMarker("local-graph-renderer-diagnostic-code", "GRAPH_PANEL_RENDER_FAILED");
        writeMarker("local-graph-renderer-diagnostic-message", error?.message || "graph panel render failed");
      });
      writeSelectionStatusMarkers(selectionState, status, {
        focus: details.focus,
        summary: details,
      });
    }).catch((error) => {
      if (sharedSelectionState.markerSeqInFlight !== markerSeq) {
        writeMarker("local-graph-selection-stale", "error-ignored");
        return;
      }
      const failed = {
        ok: false,
        diagnostics: [stableDiagnostic("METADATA_CHECKER_SELECTION_ANALYSIS_FAILED", error?.message || "selection analysis failed", "error")],
      };
      updatePanelWithBackgroundState(failed);
      writeSelectionStatusMarkers(selectionState, MARKER_STATUS_ERROR, {
        focus: asString(selectionPayload.active_component_id),
      });
    });
    if (typeof root.setTimeout === "function" && SELECTION_ANALYSIS_DEBOUNCE_MS > 0) {
      sharedSelectionState.debounceTimer = root.setTimeout(runAnalysis, SELECTION_ANALYSIS_DEBOUNCE_MS);
    } else {
      runAnalysis();
    }
  }

  function normalizeAction(action) {
    if (typeof action !== "string") {
      return null;
    }
    if (action === "openPanel" || action === "open-panel" || action === "showPanel") {
      return "openPanel";
    }
    if (action === "hidePanel" || action === "hide-panel") {
      return "hidePanel";
    }
    if (action === "togglePanel" || action === "toggle-panel") {
      return "togglePanel";
    }
    if (action === "refreshBridge" || action === "refreshBridgeStatus" || action === "refresh") {
      return "refreshBridge";
    }
    if (action === "analyzeCurrentSelection" || action === "analyze") {
      return "analyzeCurrentSelection";
    }
    if (action === "retryCurrentSelection" || action === "retry-current-selection") {
      return "retryCurrentSelection";
    }
    return null;
  }

  async function retryCurrentSelectionFromBridge() {
    const response = await requestPageBridge("getBridgeStatus");
    const payload = response?.payload || {};
    const selection = payload.selection || {};
    if (!selection || !selection.source_path) {
      return {
        ok: false,
        diagnostics: [
          stableDiagnostic(
            "METADATA_CHECKER_SELECTION_EMPTY",
            "current selection has no source_path to retry",
            "warning",
          ),
        ],
      };
    }
    const result = await sendSelectionToBackground(selection);
    const selectionState = normalizeSelectionForStatus(selection);
    const status = inferPopupStatusFromResult(result, selectionState);
    const summary = normalizeGraphSummary(result);
    updatePanelWithBackgroundState({
      ...(result || {}),
      source_path: asString(selection.source_path),
    });
    renderGraphPanelIfAvailable(result).catch((error) => {
      writeMarker("local-graph-renderer", "error");
      writeMarker("local-graph-renderer-diagnostic-code", "GRAPH_PANEL_RENDER_FAILED");
      writeMarker("local-graph-renderer-diagnostic-message", error?.message || "graph panel render failed");
    });
    writeSelectionStatusMarkers(selectionState, status, {
      focus: summary.focus || asString(selection.active_component_id),
      summary: {
        focus: summary.focus || asString(selection.active_component_id),
        depth: summary.depth || DEFAULT_GRAPH_DEPTH,
        visibleHop: summary.visibleHop,
        nodeCount: summary.nodeCount,
        edgeCount: summary.edgeCount,
      },
    });
    return {
      ok: result?.ok !== false,
      bridge_status: response,
      background_result: result,
      diagnostics: asDiagnostics(response?.diagnostics).concat(asDiagnostics(result?.diagnostics)),
    };
  }

  async function refreshVisibleManifestFromBridge() {
    const response = await requestPageBridge("getBridgeStatus");
    forwardStatus(response);
    writeBridgeProbeMarkers(response);
    applyPanelStateFromBridge(response, "refreshBridge");
    const payload = response?.payload || {};
    if (payload.bridge_detected === false || payload.supported === false) {
      return {
        ok: false,
        bridge_status: response,
        diagnostics: asDiagnostics(response?.diagnostics).concat(asDiagnostics(payload.diagnostics)),
      };
    }
    if (!sharedState.remoteSession?.base_url) {
      const bootstrapped = await bootstrapRemoteSessionFromPage();
      if (bootstrapped?.ok === false) {
        writeManifestRefreshMarkers(bootstrapped);
        return {
          ok: false,
          bridge_status: response,
          background_result: bootstrapped,
          diagnostics: asDiagnostics(response?.diagnostics).concat(asDiagnostics(bootstrapped?.diagnostics)),
        };
      }
    }
    const pageContext = payload.page_context || {};
    const selection = payload.selection || {};
    const backgroundResult = await sendRuntimeMessage({
      type: "metadata-checker-refresh-visible-manifest",
      payload: {
        base_url: sharedState.remoteSession?.base_url || root.location?.origin || "",
        project_name: pageContext.project_name || pageContext.projectName || selection.project_name || "",
        current_source_path: pageContext.source_path || selection.source_path || "",
      },
    });
    if (backgroundResult?.visible_index) {
      sharedState.visibleIndex = backgroundResult.visible_index;
    }
    writeManifestRefreshMarkers(backgroundResult || {});
    updatePanelWithBackgroundState({
      ...(backgroundResult || {}),
      source_path: pageContext.source_path || selection.source_path || "",
    });
    return {
      ok: backgroundResult?.ok !== false,
      bridge_status: response,
      background_result: backgroundResult,
      manifest_diff: backgroundResult?.manifest_diff || null,
      timings: backgroundResult?.timings || null,
      diagnostics: asDiagnostics(response?.diagnostics).concat(asDiagnostics(backgroundResult?.diagnostics)),
    };
  }

  function mountIfNeeded() {
    if (!sharedState.mounted) {
      mountPanelHost();
    }
    return getPanelHost();
  }

  function handlePanelCommand(host, command) {
    if (!host) {
      return {
        ok: false,
        diagnostics: [stableDiagnostic("PANEL_HOST_NOT_AVAILABLE", "panel host is unavailable")],
      };
    }
    if (command === "openPanel") {
      const result = host.togglePanel ? host.togglePanel(true) : { visible: false };
      return { ok: result.visible === true, visible: result.visible };
    }
    if (command === "hidePanel") {
      const result = host.togglePanel ? host.togglePanel(false) : { visible: false };
      return { ok: result.visible === false, visible: result.visible };
    }
    if (command === "togglePanel") {
      const result = host.togglePanel ? host.togglePanel() : { visible: false };
      return { ok: true, visible: result.visible };
    }
    return {
      ok: false,
      diagnostics: [
        stableDiagnostic(
          "METADATA_CHECKER_PANEL_COMMAND_UNSUPPORTED",
          `unsupported panel command: ${String(command)}`,
        ),
      ],
    };
  }

  function wrapPanelCommandResponse(action, payload) {
    return {
      action,
      ...(isObject(payload) ? payload : {}),
    };
  }

  if (runtime && typeof runtime.onMessage?.addListener === "function") {
    if (!sharedState.runtimeListenerBound) {
      sharedState.runtimeListenerBound = true;
      runtime.onMessage.addListener((message, _sender, sendResponse) => {
      const requestType = message?.type === "metadata-checker-tab-request"
        ? (typeof message.request_type === "string" ? message.request_type : null)
        : normalizeAction(message?.action || message?.type);
      if (typeof requestType !== "string" || requestType.length === 0) {
        return false;
      }

      const host = mountIfNeeded();
      if (
        requestType === "openPanel"
        || requestType === "hidePanel"
        || requestType === "togglePanel"
      ) {
        sendResponse(wrapPanelCommandResponse(requestType, handlePanelCommand(host, requestType)));
        return false;
      }

      if (message?.type === "metadata-checker-tab-request") {
        if (requestType === "refreshBridge") {
          refreshVisibleManifestFromBridge().then((response) => {
            sendResponse(response);
          }).catch((error) => {
            const failed = {
              ok: false,
              diagnostics: [
                stableDiagnostic(
                  "METADATA_CHECKER_REFRESH_VISIBLE_MANIFEST_FAILED",
                  error?.message || "visible manifest refresh failed",
                  "error",
                ),
              ],
            };
            writeManifestRefreshMarkers(failed);
            sendResponse(failed);
          });
          return true;
        }
        if (requestType === "retryCurrentSelection") {
          retryCurrentSelectionFromBridge().then((response) => {
            sendResponse(response);
          }).catch((error) => {
            sendResponse({
              ok: false,
              diagnostics: [
                stableDiagnostic(
                  "METADATA_CHECKER_RETRY_CURRENT_SELECTION_FAILED",
                  error?.message || "retry current selection failed",
                  "error",
                ),
              ],
            });
          });
          return true;
        }
        requestPageBridge(requestType).then((response) => {
          forwardStatus(response);
          sendResponse(response);
        }).catch((error) => {
          const failed = {
            payload: {
              supported: false,
            },
            diagnostics: [
              stableDiagnostic(
                "METADATA_CHECKER_TAB_REQUEST_FAILED",
                error?.message || "metadata checker tab request failed",
                "error",
              ),
            ],
          };
          writeBridgeProbeMarkers(failed);
          applyPanelStateFromBridge(failed, requestType);
          sendResponse(failed);
        });
        return true;
      }

      if (requestType === "refreshBridge") {
        refreshVisibleManifestFromBridge().then((response) => {
          sendResponse(wrapPanelCommandResponse("refreshBridge", response));
        }).catch((error) => {
          const failed = {
            ok: false,
            diagnostics: [stableDiagnostic("METADATA_CHECKER_PANEL_COMMAND_FAILED", error?.message || "refresh bridge failed", "error")],
          };
          writeManifestRefreshMarkers(failed);
          sendResponse(
            wrapPanelCommandResponse("refreshBridge", failed),
          );
        });
        return true;
      }

      if (requestType === "analyzeCurrentSelection") {
        requestPageBridge("analyzeCurrentSelection").then((response) => {
          sendResponse(wrapPanelCommandResponse("analyzeCurrentSelection", response));
        }).catch((error) => {
          sendResponse(
            wrapPanelCommandResponse("analyzeCurrentSelection", {
              ok: false,
              diagnostics: [stableDiagnostic("METADATA_CHECKER_PANEL_COMMAND_FAILED", error?.message || "analyze current selection failed", "error")],
            }),
          );
        });
        return true;
      }

      if (requestType === "retryCurrentSelection") {
        retryCurrentSelectionFromBridge().then((response) => {
          sendResponse(wrapPanelCommandResponse("retryCurrentSelection", response));
        }).catch((error) => {
          sendResponse(
            wrapPanelCommandResponse("retryCurrentSelection", {
              ok: false,
              diagnostics: [stableDiagnostic("METADATA_CHECKER_PANEL_COMMAND_FAILED", error?.message || "retry current selection failed", "error")],
            }),
          );
        });
        return true;
      }

      sendResponse(wrapPanelCommandResponse(requestType, handlePanelCommand(host, requestType)));
      return false;
      });
    }
  }

  if (typeof root.addEventListener === "function") {
    if (!sharedState.windowListenerBound) {
      sharedState.windowListenerBound = true;
      const onWindowMessage = (event) => {
      const data = event?.data;
      if (!normalizeMessageFromPage(data)) {
        return;
      }

      if (data.type === SELECTION_CHANGED_MARKER) {
        markSelectionFromPageMessage(data);
        return;
      }

      if (data.type === BRIDGE_READY_MARKER) {
        writeBridgeProbeMarkers(data);
        applyPanelStateFromBridge(data, "getBridgeStatus");
        bootstrapRemoteSessionFromPage().catch((error) => {
          updatePanelWithBackgroundState({
            ok: false,
            diagnostics: [
              stableDiagnostic(
                "SESSION_BOOTSTRAP_FAILED",
                error?.message || "remote session bootstrap failed",
                "warning",
              ),
            ],
          });
        });
        return;
      }
      if (typeof data.type === "string" && typeof data.request_id === "string") {
        writeBridgeProbeMarkers(data);
        forwardStatus(data);
        applyPanelStateFromBridge(data, data.type);
      }
    };

    root.addEventListener("message", onWindowMessage);
    root.addEventListener("metadata-checker-bridge-message", onWindowMessage);
    }
  }

  writeMarker("extension-content", "loaded");
  writeMarker("extension-content-fallback-version", "artifact-load-v2");
  root.__metadata_checker_resolve_asset_url = function resolveAssetUrl(path) {
    if (runtime && typeof runtime.getURL === "function") {
      return runtime.getURL(path);
    }
    return path;
  };

  root.__metadata_checker_injected_script_paths = PANEL_SCRIPT_PATHS;
  mountPanelHost();
})(typeof globalThis === "undefined" ? undefined : globalThis);

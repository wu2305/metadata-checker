import assert from "node:assert/strict";
import test from "node:test";

import { collectPopupAcceptanceChecks, redact } from "../tools/m47-real-bi-cdp-acceptance.mjs";

const EDGE_EVIDENCE_UNAVAILABLE_TEXT = "EDGE_EVIDENCE_UNAVAILABLE";

const M48_INTERACTION_MARKER_NAMES = [
  "graph-hover-target",
  "graph-locked-target",
  "graph-detail-kind",
  "graph-highlight-node-count",
  "graph-highlight-edge-count",
  "graph-viewport-scale",
  "graph-viewport-target",
  "graph-density-profile",
];

const M48_INTERACTION_CATEGORIES = [
  "hover_node",
  "hover_edge",
  "hover_aggregate",
  "lock_node",
  "lock_edge",
  "lock_aggregate",
  "viewport_zoom",
  "viewport_focus_edge",
  "copy_graph_text",
];

function makeInteractionProbe(overrides = {}) {
  const markerValues = {
    "graph-hover-target": "node:n1",
    "graph-locked-target": "node:n1",
    "graph-detail-kind": "node",
    "graph-highlight-node-count": "1",
    "graph-highlight-edge-count": "2",
    "graph-viewport-scale": "1",
    "graph-viewport-target": "edge:e1",
    "graph-density-profile": "compact",
    "graph-open-detail-context": "node:n1",
  };

  const beforeMarkers = { ...markerValues, focusComponent: "comp-1" };
  const afterMarkers = { ...markerValues, focusComponent: "comp-1", "graph-viewport-scale": "1.2" };

  function mergeMarkers(base = {}, override = {}) {
    const baseMarkers = base?.markers || {};
    const overrideMarkers = override?.markers || {};
    return {
      ...base,
      ...override,
      markers: {
        ...baseMarkers,
        ...overrideMarkers,
      },
    };
  }

  function mergeStep(baseStep, overrideStep = {}) {
    return {
      ...baseStep,
      ...overrideStep,
      before: mergeMarkers(baseStep.before || {}, overrideStep.before || {}),
      after: mergeMarkers(baseStep.after || {}, overrideStep.after || {}),
    };
  }

  const baseProbe = {
    attempted: true,
    markerNames: M48_INTERACTION_MARKER_NAMES,
    before: {
      markers: beforeMarkers,
    },
    after: {
      markers: afterMarkers,
    },
    hover: {
      node: {
        attempted: true,
        before: {
          markers: mergeMarkers(beforeMarkers, { "graph-hover-target": "" }),
        },
        after: {
          markers: mergeMarkers(afterMarkers, { "graph-hover-target": "node:n1:5" }),
        },
      },
      edge: {
        attempted: true,
        before: {
          markers: mergeMarkers(beforeMarkers, { "graph-hover-target": "" }),
        },
        after: {
          markers: mergeMarkers(afterMarkers, { "graph-hover-target": "edge:e1:7" }),
        },
      },
      aggregate: {
        attempted: true,
        before: {
          markers: mergeMarkers(beforeMarkers, { "graph-hover-target": "" }),
        },
        after: {
          markers: mergeMarkers(afterMarkers, { "graph-hover-target": "aggregate:a1:2" }),
        },
      },
    },
    lock: {
      node: {
        attempted: true,
        before: {
          markers: mergeMarkers(beforeMarkers, { "graph-locked-target": "" }),
        },
        after: {
          markers: mergeMarkers(afterMarkers, { "graph-locked-target": "node:n1:5" }),
        },
      },
      edge: {
        attempted: true,
        before: {
          markers: mergeMarkers(beforeMarkers, { "graph-locked-target": "" }),
        },
        after: {
          markers: mergeMarkers(afterMarkers, { "graph-locked-target": "edge:e1:7" }),
        },
      },
      aggregate: {
        attempted: true,
        before: {
          markers: mergeMarkers(beforeMarkers, { "graph-locked-target": "" }),
        },
        after: {
          markers: mergeMarkers(afterMarkers, { "graph-locked-target": "aggregate:a1:2" }),
        },
      },
    },
    zoom: {
      attempted: true,
      before: {
        markers: beforeMarkers,
      },
      beforeViewport: {
        hoverTarget: beforeMarkers["graph-hover-target"],
        lockedTarget: beforeMarkers["graph-locked-target"],
        detailKind: beforeMarkers["graph-detail-kind"],
        hoverCount: beforeMarkers["graph-highlight-node-count"],
        highlightEdgeCount: beforeMarkers["graph-highlight-edge-count"],
        viewportScale: beforeMarkers["graph-viewport-scale"],
        viewportTarget: beforeMarkers["graph-viewport-target"],
      },
      beforeScroll: {
        scrollX: 0,
        scrollY: 0,
      },
      after: {
        markers: afterMarkers,
      },
      afterViewport: {
        hoverTarget: afterMarkers["graph-hover-target"],
        lockedTarget: afterMarkers["graph-locked-target"],
        detailKind: afterMarkers["graph-detail-kind"],
        hoverCount: afterMarkers["graph-highlight-node-count"],
        highlightEdgeCount: afterMarkers["graph-highlight-edge-count"],
        viewportScale: afterMarkers["graph-viewport-scale"],
        viewportTarget: afterMarkers["graph-viewport-target"],
      },
      afterScroll: {
        scrollX: 0,
        scrollY: 0,
      },
    },
    viewportFocusEdge: {
      attempted: true,
      before: {
        markers: mergeMarkers(beforeMarkers, { "graph-viewport-target": "node:n1:4" }),
      },
      after: {
        markers: mergeMarkers(afterMarkers, { "graph-viewport-target": "edge:e1:7" }),
      },
      beforeFocus: "comp-1",
      afterFocus: "comp-1",
    },
    openDetail: {
      attempted: true,
      before: {
        markers: beforeMarkers,
      },
      after: {
        markers: mergeMarkers(afterMarkers, {
          "graph-open-detail-context": "node:n1",
          "graph-detail-kind": "node",
        }),
      },
    },
    openDetailContext: {
      context: "node:n1",
      hasContext: true,
      detailKind: "node",
    },
    copy: {
      attempted: true,
      copiedText: "focus: comp-1\nvisible: 3/10 nodes, 2/20 edges\nnodes:\n- n1\nedges:\n- a -> b",
      before: {
        markers: beforeMarkers,
      },
      after: {
        markers: afterMarkers,
      },
    },
    openDetailContextAttempted: true,
  };

  const mergedProbe = {
    ...baseProbe,
    ...overrides,
    before: mergeMarkers(baseProbe.before, overrides.before || {}),
    after: mergeMarkers(baseProbe.after, overrides.after || {}),
    hover: {
      node: mergeStep(baseProbe.hover.node, overrides.hover?.node || {}),
      edge: mergeStep(baseProbe.hover.edge, overrides.hover?.edge || {}),
      aggregate: mergeStep(baseProbe.hover.aggregate, overrides.hover?.aggregate || {}),
    },
    lock: {
      node: mergeStep(baseProbe.lock.node, overrides.lock?.node || {}),
      edge: mergeStep(baseProbe.lock.edge, overrides.lock?.edge || {}),
      aggregate: mergeStep(baseProbe.lock.aggregate, overrides.lock?.aggregate || {}),
    },
    zoom: {
      ...baseProbe.zoom,
      ...overrides.zoom,
      before: mergeMarkers(baseProbe.zoom.before, overrides.zoom?.before || {}),
      after: mergeMarkers(baseProbe.zoom.after, overrides.zoom?.after || {}),
      beforeViewport: { ...baseProbe.zoom.beforeViewport, ...(overrides.zoom?.beforeViewport || {}) },
      afterViewport: { ...baseProbe.zoom.afterViewport, ...(overrides.zoom?.afterViewport || {}) },
      beforeScroll: { ...baseProbe.zoom.beforeScroll, ...(overrides.zoom?.beforeScroll || {}) },
      afterScroll: { ...baseProbe.zoom.afterScroll, ...(overrides.zoom?.afterScroll || {}) },
    },
    viewportFocusEdge: {
      ...baseProbe.viewportFocusEdge,
      ...overrides.viewportFocusEdge,
      before: mergeMarkers(baseProbe.viewportFocusEdge.before, overrides.viewportFocusEdge?.before || {}),
      after: mergeMarkers(baseProbe.viewportFocusEdge.after, overrides.viewportFocusEdge?.after || {}),
    },
    openDetail: {
      ...baseProbe.openDetail,
      ...overrides.openDetail,
      before: mergeMarkers(baseProbe.openDetail.before, overrides.openDetail?.before || {}),
      after: mergeMarkers(baseProbe.openDetail.after, overrides.openDetail?.after || {}),
    },
    copy: {
      ...baseProbe.copy,
      ...overrides.copy,
      before: mergeMarkers(baseProbe.copy.before, overrides.copy?.before || {}),
      after: mergeMarkers(baseProbe.copy.after, overrides.copy?.after || {}),
    },
    openDetailContext: {
      ...baseProbe.openDetailContext,
      ...(overrides.openDetailContext || {}),
    },
  };

  return mergedProbe;
}

function baseSnapshot(overrides = {}) {
  return {
    viewport: {
      width: 1000,
      height: 1000,
      devicePixelRatio: 1,
    },
    directMarkers: {
      "extension-content": "loaded",
      "extension-page-script": "loaded",
      "extension-runtime-adapter": "loaded",
      "extension-session": "loaded",
      "extension-selection-bridge": "loaded",
      "selection-bridge": "loaded",
      "on-init-designer": "loaded",
      "analysis-status": "ready",
      "graph-depth": "2",
      "graph-visible-hop": "1",
      "local-graph-renderer": "pixi",
      "local-graph-renderer-candidate": "pixi",
      "graph-node-count": "3",
      "graph-edge-count": "2",
      "manifest-added": "1",
      "manifest-modified": "0",
      "manifest-deleted": "0",
      "manifest-unchanged": "12",
      "content-queue-count": "3",
      "graph-hover-target": "node:n1:5",
      "graph-locked-target": "node:n1:5",
      "graph-detail-kind": "node",
      "graph-highlight-node-count": "1",
      "graph-highlight-edge-count": "2",
      "graph-viewport-scale": "1",
      "graph-viewport-target": "edge:e1:7",
      "graph-density-profile": "compact",
      "manifest-fetch-ms": "18",
      "manifest-diff-ms": "9",
      "changed-content-fetch-ms": "7",
      "wasm-update-ms": "5",
      "analyze-ms": "320",
      "local-graph-offscreen-total-ms": "1200",
      "local-graph-timing-ensure-runtime-ms": "5",
      "local-graph-timing-fetch-metadata-ms": "80",
      "local-graph-timing-init-runtime-ms": "20",
      "local-graph-timing-load-document-ms": "180",
      "local-graph-timing-build-graph-ms": "260",
      "local-graph-timing-analyze-local-graph-ms": "320",
      "local-graph-timing-layout-ms": "34",
      "local-graph-timing-render-ms": "18",
      "local-graph-timing-total-ms": "1200",
      "local-graph-cache-metadata-hit": "true",
      "local-graph-cache-document-hit": "true",
      "local-graph-selection-debounce-ms": "40",
    },
    popup: {
      exists: true,
      rect: {
        x: 800,
        y: 760,
        width: 150,
        height: 150,
        right: 0,
        bottom: 0,
      },
      markerRenderer: "pixi",
      markerDepth: "2",
      markerVisibleHop: "1",
      markerNodeCount: "3",
      markerEdgeCount: "2",
      canvaCount: 1,
      canvases: [
        {
          width: 120,
          height: 120,
          clientWidth: 120,
          clientHeight: 120,
          rect: { x: 820, y: 840, width: 120, height: 120, right: 60, bottom: 260 },
          className: "metadata-checker-graph-canvas",
        },
      ],
      actionButtons: {
        "metadata-checker-graph-panel-pin": { text: "P", disabled: false },
        "metadata-checker-graph-panel-copy": { text: "C", disabled: false },
        "metadata-checker-graph-panel-toggle": { text: "−", disabled: false },
      },
      nodeDetails: [
        {
          text: "nameA (node)",
          label: "nameA",
          kind: "node",
          className: "metadata-checker-graph-node",
          nodeId: "n1",
          edgeId: null,
          evidence: "",
        },
      ],
      edgeDetails: [
        {
          text: "n1 -> n2:related (ok)",
          label: "related",
          kind: "",
          from: "n1",
          to: "n2",
          className: "metadata-checker-graph-edge",
          nodeId: "n2",
          edgeId: "e1",
          evidence: "ok",
        },
      ],
      edgeEvidenceRows: [
        { edgeId: "e1", from: "n1", to: "n2", label: "related", evidence: "ok" },
      ],
      interactionDetailAfterNodeClick: [
        { text: "Node · focus · Component · depth 0 · neighbors 1" },
      ],
      interactionDetailAfterEdgeClick: [
        { text: "Edge · filter · n1 -> n2 · ok" },
      ],
      edgeEvidenceUnavailableCount: 0,
      edgeEvidenceAvailable: 1,
      diagnosticsTextRows: [],
    },
    interactionProbe: makeInteractionProbe(),
    selectionProbe: {
      attempted: true,
      posted: true,
      before: {
        event: "",
        status: "warning",
        seq: 0,
        active: "",
        count: 0,
      },
      loading: {
        event: "received",
        status: "loading",
        seq: 1,
        active: "canvas",
        count: 1,
      },
      final: {
        event: "received",
        status: "warning",
        seq: 1,
        active: "canvas",
        count: 1,
      },
      loadingTimedOut: false,
      finalTimedOut: false,
    },
    canvasCount: 1,
    ...overrides,
  };
}

test("collectPopupAcceptanceChecks should pass on valid hard constraints", () => {
  const result = collectPopupAcceptanceChecks(baseSnapshot(), true, "/tmp/shot.png");

  assert.equal(result.failed, false);
  assert.equal(result.checks.renderer.status, "pass");
  assert.equal(result.checks.canvas.status, "pass");
  assert.equal(result.checks.geometry.status, "pass");
  assert.equal(result.checks.selection.status, "pass");
  assert.equal(result.checks.interaction.status, "pass");
  assert.equal(result.checks.interaction.markerPresence.every((item) => item.present), true);
  for (const category of M48_INTERACTION_CATEGORIES) {
    assert.equal(result.checks.interaction.categories[category].status, "pass");
  }
  assert.equal(result.checks.performance.status, "pass");
  assert.equal(result.checks.manifest.status, "pass");
  assert.equal(result.performance_timings.render_ms, 18);
  assert.equal(result.performance_timings.total_ms, 1200);
  assert.equal(result.performance_timings.analyze_ms, 320);
  assert.equal(result.performance_timings.manifest_fetch_ms, 18);
  assert.equal(result.performance_timings.manifest_diff_ms, 9);
  assert.equal(result.performance_timings.changed_content_fetch_ms, 7);
  assert.equal(result.performance_timings.wasm_update_ms, 5);
  assert.equal(result.performance_timings.metadata_cache_hit, "true");
  assert.equal(result.performance_timings.document_cache_hit, "true");
  assert.equal(result.performance_timings.selection_debounce_ms, 40);
  assert.equal(result.checks.manifest.diff.added, 1);
  assert.equal(result.checks.manifest.diff.modified, 0);
  assert.equal(result.checks.manifest.diff.deleted, 0);
  assert.equal(result.checks.manifest.diff.unchanged, 12);
  assert.equal(result.checks.manifest.diff.content_queue_count, 3);
  assert.equal(result.checks.screenshot.status, "pass");
  assert.equal(result.failed_categories.length, 0);
  assert.equal(result.node_details.length, 1);
  assert.equal(result.edge_details.length, 1);
  assert.equal(result.screenshot_exists, true);
  assert.equal(result.passing, true);
});

test("collectPopupAcceptanceChecks should fail when click details omit node or edge contract fields", () => {
  const snapshot = baseSnapshot({
    popup: {
      ...baseSnapshot().popup,
      interactionDetailAfterNodeClick: [{ text: "Node ·" }],
      interactionDetailAfterEdgeClick: [{ text: "Edge ·" }],
    },
  });
  const result = collectPopupAcceptanceChecks(snapshot, true, "/tmp/shot.png");

  assert.equal(result.failed, true);
  assert.equal(result.checks.interaction.status, "fail");
  assert.equal(result.failed_categories.includes("interaction"), true);
  assert.equal(
    result.diagnostics.some((item) => item.code === "M47_REAL_BI_NODE_DETAIL_CONTRACT_MISSING"),
    true,
  );
  assert.equal(
    result.diagnostics.some((item) => item.code === "M47_REAL_BI_EDGE_DETAIL_CONTRACT_MISSING"),
    true,
  );
});

test("collectPopupAcceptanceChecks should fail when required M48 interaction marker is missing", () => {
  const result = collectPopupAcceptanceChecks(
    baseSnapshot({
      directMarkers: {
        ...baseSnapshot().directMarkers,
        "graph-hover-target": "",
      },
    }),
    true,
    "/tmp/shot.png",
  );

  assert.equal(result.failed, true);
  assert.equal(result.checks.interaction.status, "fail");
  assert.equal(result.failed_categories.includes("interaction"), true);
  assert.equal(
    result.diagnostics.some((item) => item.code === "M48_REAL_BI_INTERACTION_MARKER_MISSING"),
    true,
  );
  assert.equal(
    result.checks.interaction.diagnostics.some(
      (item) => item.code === "M48_REAL_BI_INTERACTION_MARKER_MISSING",
    ),
    true,
  );
});

test("collectPopupAcceptanceChecks should fail when viewport scale does not change after wheel", () => {
  const result = collectPopupAcceptanceChecks(
    baseSnapshot({
      interactionProbe: makeInteractionProbe({
        zoom: {
          before: {
            markers: {
              ...baseSnapshot().interactionProbe?.before?.markers,
              "graph-viewport-scale": "1.0",
            },
          },
          after: {
            markers: {
              ...baseSnapshot().interactionProbe?.after?.markers,
              "graph-viewport-scale": "1.0",
            },
          },
        },
      }),
    }),
    true,
    "/tmp/shot.png",
  );

  assert.equal(result.failed, true);
  assert.equal(result.checks.interaction.status, "fail");
  assert.equal(
    result.checks.interaction.categories.viewport_zoom.status,
    "fail",
  );
  assert.equal(
    result.checks.interaction.categories.viewport_zoom.diagnostics.some(
      (item) => item.code === "M48_REAL_BI_VIEWPORT_ZOOM_NOT_CHANGED",
    ),
    true,
  );
});

test("collectPopupAcceptanceChecks should fail when designer focus component changes after edge focus", () => {
  const result = collectPopupAcceptanceChecks(
    baseSnapshot({
      interactionProbe: makeInteractionProbe({
        viewportFocusEdge: {
          before: {
            markers: {
              ...(baseSnapshot().interactionProbe?.viewportFocusEdge?.before?.markers || {}),
              focusComponent: "node-component-1",
            },
          },
          after: {
            markers: {
              ...(baseSnapshot().interactionProbe?.viewportFocusEdge?.after?.markers || {}),
              focusComponent: "node-component-2",
            },
          },
        },
      }),
    }),
    true,
    "/tmp/shot.png",
  );

  assert.equal(result.failed, true);
  assert.equal(result.checks.interaction.status, "fail");
  assert.equal(result.checks.interaction.categories.viewport_focus_edge.status, "fail");
  assert.equal(
    result.checks.interaction.categories.viewport_focus_edge.diagnostics.some(
      (item) => item.code === "M48_REAL_BI_DESIGNER_FOCUS_CHANGED",
    ),
    true,
  );
});

test("collectPopupAcceptanceChecks should fail when copy output contains sensitive fields", () => {
  const result = collectPopupAcceptanceChecks(
    baseSnapshot({
      interactionProbe: makeInteractionProbe({
        copy: {
          copiedText: "focus: comp-1\nvisible: 1/1 nodes, 0/0 edges\nnodes:\n- access_token=abc123\nedges:\n",
        },
      }),
    }),
    true,
    "/tmp/shot.png",
  );

  assert.equal(result.failed, true);
  assert.equal(result.checks.interaction.status, "fail");
  assert.equal(result.checks.interaction.categories.copy_graph_text.status, "fail");
  assert.equal(
    result.checks.interaction.categories.copy_graph_text.diagnostics.some(
      (item) => item.code === "M48_REAL_BI_COPY_SENSITIVE_TEXT",
    ),
    true,
  );
});

test("collectPopupAcceptanceChecks should fail renderer/canvas/geometry/selection when constraints break", () => {
  const result = collectPopupAcceptanceChecks({
    viewport: {
      width: 300,
      height: 300,
      devicePixelRatio: 1,
    },
    directMarkers: {
      "graph-depth": "1",
      "graph-visible-hop": "3",
    },
    popup: {
      exists: true,
      rect: {
        x: 0,
        y: 0,
        width: 500,
        height: 500,
        right: 0,
        bottom: 0,
      },
      markerRenderer: "d3-force",
      markerDepth: "1",
      markerVisibleHop: "3",
      canvaCount: 1,
      canvases: [],
      actionButtons: {},
      nodeDetails: [{ text: "a" }],
      edgeDetails: [{ text: "a -> b:rel (ok)" }],
      edgeEvidenceRows: [],
      edgeEvidenceUnavailableCount: 0,
      edgeEvidenceAvailable: 0,
      diagnosticsTextRows: [],
      markerNodeCount: "1",
      markerEdgeCount: "1",
    },
    selectionProbe: {
      attempted: false,
      reason: "missing probe",
    },
    canvasCount: 0,
  }, false, "/tmp/missing-shot.png");

  assert.equal(result.failed, true);
  assert.equal(result.checks.screenshot.status, "fail");
  assert.equal(result.checks.renderer.status, "fail");
  assert.equal(result.checks.canvas.status, "fail");
  assert.equal(result.checks.geometry.status, "fail");
  assert.equal(result.checks.selection.status, "fail");
  assert.equal(result.checks.interaction.status, "fail");
  assert.equal(result.checks.performance.status, "fail");
  assert.equal(result.failed_categories.includes("renderer"), true);
  assert.equal(result.failed_categories.includes("canvas"), true);
  assert.equal(result.failed_categories.includes("geometry"), true);
  assert.equal(result.failed_categories.includes("selection"), true);
  assert.equal(result.failed_categories.includes("interaction"), true);
  assert.equal(result.failed_categories.includes("performance"), true);
  assert.equal(result.failed_categories.includes("manifest"), true);
  assert.equal(result.failed_categories.includes("selection"), true);
  assert.equal(
    result.diagnostics.some((item) => item.code === "M47_REAL_BI_SCREENSHOT_MISSING"),
    true,
  );
});

test("collectPopupAcceptanceChecks should fail when performance timings are absent", () => {
  const directMarkers = { ...baseSnapshot().directMarkers };
  for (const key of Object.keys(directMarkers)) {
    if (key.startsWith("local-graph-timing-") || key === "local-graph-offscreen-total-ms") {
      delete directMarkers[key];
    }
  }
  const result = collectPopupAcceptanceChecks(baseSnapshot({ directMarkers }), true, "/tmp/shot.png");

  assert.equal(result.failed, true);
  assert.equal(result.checks.performance.status, "fail");
  assert.equal(result.failed_categories.includes("performance"), true);
  assert.equal(
    result.diagnostics.some((item) => item.code === "M47_REAL_BI_PERFORMANCE_TIMING_MISSING"),
    true,
  );
});

test("collectPopupAcceptanceChecks should pass when manifest diff is returned by refresh command", () => {
  const snapshot = baseSnapshot({
    directMarkers: {
      ...baseSnapshot().directMarkers,
      "graph-depth": "2",
      "graph-visible-hop": "1",
      "manifest-added": "",
      "manifest-modified": "",
      "manifest-deleted": "",
      "manifest-unchanged": "",
      "content-queue-count": "",
    },
  });

  const result = collectPopupAcceptanceChecks(
    snapshot,
    true,
    "/tmp/shot.png",
    {
      requestType: "refreshBridge",
      ok: true,
      timedOut: false,
      command: {
        manifest_diff: {
          added: 2,
          modified: 0,
          deleted: 1,
          unchanged: 12,
          content_queue_count: 4,
        },
      },
    },
  );

  assert.equal(result.failed, false);
  assert.equal(result.checks.manifest.status, "pass");
  assert.equal(result.checks.manifest.diff.added, 2);
  assert.equal(result.checks.manifest.diff.content_queue_count, 4);
  assert.equal(result.failed_categories.includes("manifest"), false);
});

test("collectPopupAcceptanceChecks should fail when manifest refresh command returns error", () => {
  const result = collectPopupAcceptanceChecks(baseSnapshot({
    directMarkers: {
      ...baseSnapshot().directMarkers,
      "manifest-added": "",
      "manifest-modified": "",
      "manifest-deleted": "",
      "manifest-unchanged": "",
      "content-queue-count": "",
    },
  }), true, "/tmp/shot.png", {
    requestType: "refreshBridge",
    ok: false,
    error: "refresh failed",
  });

  assert.equal(result.failed, true);
  assert.equal(result.checks.manifest.status, "fail");
  assert.equal(result.failed_categories.includes("manifest"), true);
  assert.equal(
    result.diagnostics.some((item) => item.code === "M47_REAL_BI_MANIFEST_REFRESH_COMMAND_FAILED"),
    true,
  );
});

test("collectPopupAcceptanceChecks should fail when selection changed probe is absent", () => {
  const result = collectPopupAcceptanceChecks(baseSnapshot({ selectionProbe: null }), true, "/tmp/shot.png");

  assert.equal(result.failed, true);
  assert.equal(result.checks.selection.status, "fail");
  assert.equal(result.failed_categories.includes("selection"), true);
  assert.equal(
    result.diagnostics.some((item) => item.code === "M47_REAL_BI_SELECTION_CHANGED_NOT_VERIFIED"),
    true,
  );
});

test("collectPopupAcceptanceChecks should fail when canvas exceeds popup bounds", () => {
  const snapshot = baseSnapshot({
    popup: {
      ...baseSnapshot().popup,
      rect: { x: 100, y: 100, width: 297, height: 320, right: 0, bottom: 0 },
      canvases: [
        {
          width: 320,
          height: 160,
          clientWidth: 320,
          clientHeight: 160,
          rect: { x: 100, y: 140, width: 320, height: 160, right: 0, bottom: 0 },
          className: "metadata-checker-graph-canvas",
        },
      ],
    },
  });
  const result = collectPopupAcceptanceChecks(snapshot, true, "/tmp/shot.png");

  assert.equal(result.failed, true);
  assert.equal(result.checks.geometry.status, "fail");
  assert.equal(result.failed_categories.includes("geometry"), true);
  assert.equal(
    result.diagnostics.some((item) => item.code === "M47_REAL_BI_CANVAS_OVERFLOWS_POPUP"),
    true,
  );
});

test("collectPopupAcceptanceChecks should accept unavailable edge evidence as contract diagnostic", () => {
  const result = collectPopupAcceptanceChecks({
    ...baseSnapshot({
      directMarkers: {
        ...baseSnapshot().directMarkers,
        "analysis-status": "ready",
      },
      popup: {
        ...baseSnapshot().popup,
        edgeEvidenceRows: [
          {
            edgeId: "e1",
            from: "n1",
            to: "n2",
            label: "related",
            evidence: EDGE_EVIDENCE_UNAVAILABLE_TEXT,
          },
        ],
        edgeEvidenceUnavailableCount: 1,
        edgeEvidenceAvailable: 0,
      },
    }),
  }, true, "/tmp/shot.png");

  assert.equal(result.failed, false);
  assert.equal(result.passing, true);
  assert.equal(result.checks.interaction.status, "pass");
  assert.equal(result.checks.interaction.edgeEvidenceUnavailable, 1);
  assert.equal(
    result.checks.interaction.diagnostics.some((item) =>
      item.code === "M47_REAL_BI_EDGE_EVIDENCE_TEXT" &&
      item.details?.edgeEvidenceUnavailableCount === 1
    ),
    true,
  );
});

test("redact should handle cyclic evidence objects", () => {
  const source = { message: "token=abc123", nested: {} };
  source.self = source;
  source.nested.parent = source;

  const redacted = redact(source);

  assert.equal(redacted.message, "token=<redacted>");
  assert.equal(redacted.self, "[Circular]");
  assert.equal(redacted.nested.parent, "[Circular]");
});

test("redact should redact sensitive object keys", () => {
  const redacted = redact({
    __metadata_checker_bridge_token: "plain-bridge-token",
    nested: {
      access_token: "plain-access-token",
      message: "cookie=session-value",
    },
  });

  assert.equal(redacted.__metadata_checker_bridge_token, "[redacted]");
  assert.equal(redacted.nested.access_token, "[redacted]");
  assert.equal(redacted.nested.message, "cookie=<redacted>");
});

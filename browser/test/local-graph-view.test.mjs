import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { applyLocalGraphView, isSoloFocusGraph } from "../renderer/local-graph-view.mjs";

describe("applyLocalGraphView", () => {
  it("drops blanket model fanout without evidence from focus", () => {
    const focus = "comp:page.spg|text49";
    const graph = {
      focus_node: focus,
      nodes: [
        { id: focus, label: "text49", kind: "component" },
        ...Array.from({ length: 5 }, (_, index) => ({
          id: `model:page.spg|m${index}`,
          label: `m${index}`,
          kind: "model",
        })),
      ],
      edges: Array.from({ length: 5 }, (_, index) => ({
        from: focus,
        to: `model:page.spg|m${index}`,
        kind: "reads",
        label: "reads",
        evidence_status: "unavailable",
        evidence: null,
      })),
      diagnostics: [],
    };

    const filtered = applyLocalGraphView(graph);
    assert.equal(filtered.nodes.length, 1);
    assert.equal(filtered.edges.length, 0);
  });

  it("keeps model reads with evidence", () => {
    const focus = "comp:page.spg|label1";
    const graph = {
      focus_node: focus,
      nodes: [
        { id: focus, label: "label1", kind: "component" },
        { id: "model:page.spg|m1", label: "m1", kind: "model" },
        { id: "model:page.spg|m2", label: "m2", kind: "model" },
      ],
      edges: [
        {
          from: focus,
          to: "model:page.spg|m1",
          kind: "reads",
          label: "reads",
          evidence_status: "available",
          evidence: "${m1.name}",
        },
        {
          from: focus,
          to: "model:page.spg|m2",
          kind: "reads",
          label: "reads",
          evidence_status: "unavailable",
          evidence: null,
        },
      ],
      diagnostics: [],
    };

    const filtered = applyLocalGraphView(graph);
    assert.equal(filtered.edges.length, 1);
    assert.equal(filtered.edges[0].to, "model:page.spg|m1");
  });

  it("drops unreachable model nodes and avoids solo-focus aggregate input", () => {
    const focus = "comp:page.spg|text49";
    const graph = {
      focus_node: focus,
      status: "ready",
      nodes: [
        { id: focus, label: "text49", kind: "component" },
        { id: "model:page.spg|m1", label: "m1", kind: "model" },
        { id: "model:page.spg|m2", label: "m2", kind: "model" },
      ],
      edges: [
        {
          from: focus,
          to: "model:page.spg|m1",
          kind: "reads",
          label: "reads",
          evidence_status: "unavailable",
        },
        {
          from: focus,
          to: "model:page.spg|m2",
          kind: "reads",
          label: "reads",
          evidence_status: "unavailable",
        },
      ],
      diagnostics: [],
    };

    const filtered = applyLocalGraphView(graph);
    assert.equal(filtered.nodes.length, 1);
    assert.equal(filtered.edges.length, 0);
    assert.equal(filtered.status, "empty");
    assert.equal(isSoloFocusGraph(filtered), true);
  });
});

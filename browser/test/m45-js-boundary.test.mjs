import assert from "node:assert/strict";
import test from "node:test";

import {
  createMemoryMetadataCache,
  createM45BackgroundController,
} from "../extension-chromium/background.js";

function jsonResponse(status, body) {
  return {
    ok: status >= 200 && status < 300,
    status,
    async text() {
      return typeof body === "string" ? body : JSON.stringify(body);
    },
  };
}

function createMetadataFetchStub() {
  return async (url) => {
    const parsed = new URL(url);
    if (parsed.pathname === "/api/meta/services/getFileContent/page-foreground") {
      return jsonResponse(200, {
        raw_text: JSON.stringify({ type: "metadata", source_path: "app/Test.app/Page.spg" }),
      });
    }
    if (parsed.pathname === "/api/meta/services/getFileContent/page-background") {
      return jsonResponse(200, {
        raw_text: JSON.stringify({ type: "metadata", source_path: "app/Background.app/Page.spg" }),
      });
    }
    if (parsed.pathname === "/api/meta/services/getFileContent/xiaoshouyi/app/Test.app/Page.spg") {
      return jsonResponse(200, { raw_text: JSON.stringify({ type: "metadata", source_path: "app/Test.app/Page.spg" }) });
    }
    return jsonResponse(404, {});
  };
}

test("M45 background adapter calls high-level runtime analyze for foreground and background", async () => {
  const analyzeCalls = [];
  const adapter = {
    async analyzeSuperpageSelection(selection, options = {}) {
      analyzeCalls.push({
        selection,
        options,
      });
      return { status: "ready", target: selection.source_path };
    },
  };

  const controller = createM45BackgroundController({
    fetchImpl: createMetadataFetchStub(),
    analysisClient: adapter,
    cache: createMemoryMetadataCache(),
  });

  controller.state.session = { base_url: "https://example.test" };
  controller.state.visible_index.status = "ready";
  controller.state.visible_index.projects = [{ project_name: "xiaoshouyi" }];
  controller.seedBackgroundQueue([
    {
      project_name: "xiaoshouyi",
      source_path: "app/Test.app/Page.spg",
      file_id: "page-foreground",
      revision: "1",
      extension: "spg",
      analyzable: true,
    },
    {
      project_name: "xiaoshouyi",
      source_path: "app/Background.app/Page.spg",
      file_id: "page-background",
      revision: "1",
      extension: "spg",
      analyzable: true,
    },
  ]);

  const foregroundResult = await controller.handleMessage({
    type: "metadata-checker-selection-changed",
    payload: {
      project_name: "xiaoshouyi",
      source_path: "app/Test.app/Page.spg",
      file_id: "page-foreground",
      active_component_id: "input1",
      selected_component_ids: ["input1"],
    },
  });
  assert.equal(foregroundResult.ok, true);
  assert.equal(foregroundResult.artifact_ready, true);

  const backgroundResult = await controller.handleMessage({
    type: "metadata-checker-background-process",
    payload: { limit: 1, max_concurrency: 1, min_interval_ms: 0 },
  });
  assert.equal(Array.isArray(backgroundResult.artifacts), true);
  assert.equal(backgroundResult.artifacts.length, 1);

  assert.equal(analyzeCalls.length, 2);
  const [foregroundCall, backgroundCall] = analyzeCalls;

  assert.equal(foregroundCall.options.mode, "foreground");
  assert.equal(foregroundCall.selection.source_path, "app/Test.app/Page.spg");
  assert.deepEqual(foregroundCall.selection.selected_component_ids, ["input1"]);
  assert.equal(foregroundCall.selection.active_component_id, "input1");
  assert.equal(Object.prototype.hasOwnProperty.call(foregroundCall.selection, "raw_text"), false);
  assert.equal(Object.prototype.hasOwnProperty.call(foregroundCall.selection, "rawText"), false);
  assert.equal(Object.prototype.hasOwnProperty.call(foregroundCall.selection, "raw_metadata"), false);

  assert.equal(backgroundCall.options.mode, "background");
  assert.equal(backgroundCall.selection.source_path, "app/Background.app/Page.spg");
  assert.equal(backgroundCall.selection.active_component_id, null);
  assert.deepEqual(backgroundCall.selection.selected_component_ids, []);
});

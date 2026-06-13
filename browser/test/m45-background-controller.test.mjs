import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  createBackgroundMessageFailureResponse,
  createDefaultMetadataCache,
  createIndexedDbMetadataCache,
  createM45BackgroundController,
  createMemoryMetadataCache,
  createOffscreenWasmAnalysisClient,
  createWasmAnalysisClient,
  M45_EVENT_TYPES,
} from "../extension-chromium/background.js";

async function loadJsonFixture(name) {
  const fixtureUrl = new URL(`./fixtures/${name}`, import.meta.url);
  return JSON.parse(await readFile(fixtureUrl, "utf8"));
}

function jsonResponse(status, body) {
  return {
    ok: status >= 200 && status < 300,
    status,
    async text() {
      return typeof body === "string" ? body : JSON.stringify(body);
    },
  };
}

function createFetchStub() {
  const calls = [];
  const fetchImpl = async (url, init = {}) => {
    calls.push({ url, init });
    const parsed = new URL(url);
    if (parsed.pathname === "/api/me/whoami") {
      assert.equal(parsed.searchParams.get("access_token"), "one-shot");
      assert.equal(init.credentials, "include");
      return jsonResponse(200, { userId: "u1", userName: "User One" });
    }
    if (parsed.pathname === "/api/me/getPermissionInfo") {
      assert.equal(init.credentials, "include");
      return jsonResponse(200, {
        metaProjects: [{ projectName: "xiaoshouyi", desc: "crm" }],
      });
    }
    if (parsed.pathname === "/api/meta/services/getFileChildren/xiaoshouyi") {
      return jsonResponse(200, {
        children: [
          { name: "app", parentDir: "/xiaoshouyi", isFolder: true },
          { id: "root-js", name: "custom.js", parentDir: "/xiaoshouyi", isFolder: false },
        ],
      });
    }
    if (parsed.pathname === "/api/meta/services/getFileDescendant/xiaoshouyi/app") {
      return jsonResponse(200, {
        files: [
          {
            id: "page-1",
            name: "Page.spg",
            parentDir: "/xiaoshouyi/app/Test.app",
            revision: "7",
            isFolder: false,
          },
          {
            id: "table-1",
            name: "Data.tbl",
            parentDir: "/xiaoshouyi/data/tables",
            revision: "8",
            isFolder: false,
          },
          {
            id: "txt-1",
            name: "Readme.md",
            parentDir: "/xiaoshouyi/app/Test.app",
            isFolder: false,
          },
        ],
      });
    }
    if (
      parsed.pathname === "/api/meta/services/getFileContent/page-1" ||
      parsed.pathname === "/api/meta/services/getFileContent/table-1"
    ) {
      const syntheticComponents = Array.from({ length: 96 }, (_, index) => ({
        id: `component-${index}`,
        type: "Input",
        props: {
          defaultValue: `{{model.field_${index}}}`,
          raw_text: "literal field that must stay raw text",
        },
      }));
      return jsonResponse(200, JSON.stringify({
        type: "metadata",
        raw_text: "literal field that must stay raw text",
        components: syntheticComponents,
      }));
    }
    return jsonResponse(404, {});
  };
  return { fetchImpl, calls };
}

test("M45 background message failure response is stable and redacted", () => {
  const result = createBackgroundMessageFailureResponse(
    new Error("IndexedDB failed token=secret password=secret-password"),
  );

  assert.equal(result.ok, false);
  assert.equal(result.diagnostics.length, 1);
  assert.equal(result.diagnostics[0].severity, "error");
  assert.equal(result.diagnostics[0].code, "METADATA_CHECKER_BACKGROUND_MESSAGE_FAILED");
  assert.equal(result.diagnostics[0].message.includes("secret"), false);
});

function createFetchStubWithContentFailure() {
  const { fetchImpl, calls } = createFetchStub();
  const failingFetch = async (url, init = {}) => {
    const parsed = new URL(url);
    if (parsed.pathname === "/api/meta/services/getFileContent/page-1") {
      return jsonResponse(500, { error: "failed" });
    }
    return fetchImpl(url, init);
  };
  return { fetchImpl: failingFetch, calls };
}

function createDeferred() {
  let resolve;
  const promise = new Promise((resolveFn) => {
    resolve = resolveFn;
  });
  return { promise, resolve };
}

async function waitForCacheKey(cache, matcher) {
  while (true) {
    const keys = await cache.keys();
    if (keys.some(matcher)) {
      return;
    }
    await Promise.resolve();
  }
}

test("M45 background bootstraps with one-shot token and indexes visible metadata", async () => {
  const { fetchImpl, calls } = createFetchStub();
  const controller = createM45BackgroundController({ fetchImpl, clock: () => 1000 });

  const result = await controller.bootstrapAndIndex({
    base_url: "https://autocrm-test.xiaoshouyi.com",
    access_token: "one-shot",
    current_source_path: "app/Test.app/Page.spg",
  });

  assert.equal(result.ok, true);
  assert.equal(result.session.user_id, "u1");
  assert.equal(result.visible_index.projects.length, 1);
  assert.equal(result.visible_index.files.length, 4);
  assert.equal(result.visible_index.analyzable_count, 2);
  assert.equal(result.background.processed, 2);
  assert.equal(result.background.total, 2);
  assert.equal(result.background.failed, 0);
  assert.equal(result.background.indexing_status, "completed");
  assert.equal(result.background.retry_available, false);
  assert.equal(result.cache_stats.misses, 2);
  assert.equal(
    controller.state.events.some((event) => event.event === M45_EVENT_TYPES.SESSION_BOOTSTRAPPED),
    true,
  );
  assert.equal(
    controller.state.events.some((event) => event.event === M45_EVENT_TYPES.VISIBLE_METADATA_INDEXED),
    true,
  );
  assert.equal(
    calls.every((call) => call.init.credentials === "include"),
    true,
  );
});

test("M45 bootstrap falls back to page project name when permission info is encoded", async () => {
  const fetchImpl = async (url, init = {}) => {
    const parsed = new URL(url);
    if (parsed.pathname === "/api/me/whoami") {
      assert.equal(init.credentials, "include");
      return jsonResponse(200, { userId: "u1", userName: "User One" });
    }
    if (parsed.pathname === "/api/me/getPermissionInfo") {
      return jsonResponse(200, "encoded-permission-info");
    }
    if (parsed.pathname === "/api/meta/services/getFileChildren/xiaoshouyi") {
      return jsonResponse(200, {
        children: [{ name: "app", parentDir: "/xiaoshouyi", isFolder: true }],
      });
    }
    if (parsed.pathname === "/api/meta/services/getFileDescendant/xiaoshouyi/app") {
      return jsonResponse(200, {
        files: [
          {
            id: "page-1",
            name: "Page.spg",
            parentDir: "/xiaoshouyi/app/Test.app",
            revision: "1",
            isFolder: false,
          },
        ],
      });
    }
    if (parsed.pathname === "/api/meta/services/getFileContent/page-1") {
      return jsonResponse(200, { raw_text: JSON.stringify({ type: "metadata" }) });
    }
    return jsonResponse(404, {});
  };
  const controller = createM45BackgroundController({ fetchImpl });

  const result = await controller.bootstrapAndIndex({
    base_url: "https://autocrm-test.xiaoshouyi.com",
    access_token: "one-shot",
    project_name: "xiaoshouyi",
    current_source_path: "app/Test.app/Page.spg",
  });

  assert.equal(result.ok, true);
  assert.equal(result.visible_index.projects.length, 1);
  assert.equal(result.visible_index.projects[0].project_name, "xiaoshouyi");
  assert.equal(result.visible_index.files.length, 1);
  assert.equal(result.visible_index.files[0].file_id, "page-1");
  assert.equal(result.background.processed, 1);
  assert.equal(result.background.failed, 0);
});

test("M45 bootstrap indexes real BI encoded permission and descendant shape", async () => {
  const fixture = await loadJsonFixture("m45-real-bi-index-shape.json");
  const targetFile = fixture.descendant.files.find((file) => file.isFolder === false);
  const contentFetches = [];
  const fetchImpl = async (url, init = {}) => {
    const parsed = new URL(url);
    if (parsed.pathname === "/api/me/whoami") {
      assert.equal(init.credentials, "include");
      return jsonResponse(200, { userId: "u1", userName: "User One" });
    }
    if (parsed.pathname === "/api/me/getPermissionInfo") {
      return jsonResponse(200, "x".repeat(fixture.permissionInfo.length));
    }
    if (parsed.pathname === `/api/meta/services/getFileChildren/${fixture.projectName}`) {
      return jsonResponse(fixture.children.status, { children: fixture.children.children });
    }
    if (parsed.pathname === `/api/meta/services/getFileDescendant/${fixture.projectName}/app`) {
      return jsonResponse(fixture.descendant.status, { files: fixture.descendant.files });
    }
    if (parsed.pathname === `/api/meta/services/getFileContent/${targetFile.id}`) {
      contentFetches.push(parsed.pathname);
      return jsonResponse(200, { raw_text: JSON.stringify({ type: "metadata" }) });
    }
    return jsonResponse(404, {});
  };
  const controller = createM45BackgroundController({ fetchImpl });

  const result = await controller.bootstrapAndIndex({
    base_url: "https://autocrm-test.xiaoshouyi.com",
    access_token: "one-shot",
    project_name: fixture.projectName,
    current_source_path: fixture.targetSourcePath,
  });

  assert.equal(result.ok, true);
  assert.equal(result.visible_index.projects[0].project_name, fixture.projectName);
  assert.equal(result.visible_index.files.length, 1);
  assert.equal(result.visible_index.files[0].source_path, fixture.targetSourcePath);
  assert.equal(result.visible_index.files[0].file_id, targetFile.id);
  assert.equal(result.background.processed, 1);
  assert.equal(result.background.failed, 0);
  assert.deepEqual(contentFetches, [`/api/meta/services/getFileContent/${targetFile.id}`]);
});

test("M45 bootstrap prioritizes current source path even when it sorts later", async () => {
  const starts = [];
  const fetchImpl = async (url, init = {}) => {
    const parsed = new URL(url);
    if (parsed.pathname === "/api/me/whoami") {
      assert.equal(init.credentials, "include");
      return jsonResponse(200, { userId: "u1", userName: "User One" });
    }
    if (parsed.pathname === "/api/me/getPermissionInfo") {
      return jsonResponse(200, {
        metaProjects: [{ projectName: "xiaoshouyi" }],
      });
    }
    if (parsed.pathname === "/api/meta/services/getFileChildren/xiaoshouyi") {
      return jsonResponse(200, {
        children: [{ name: "app", parentDir: "/xiaoshouyi", isFolder: true }],
      });
    }
    if (parsed.pathname === "/api/meta/services/getFileDescendant/xiaoshouyi/app") {
      return jsonResponse(200, {
        files: [
          {
            id: "page-a",
            name: "Page.spg",
            parentDir: "/xiaoshouyi/app/A.app",
            revision: "1",
            isFolder: false,
          },
          {
            id: "page-z",
            name: "Page.spg",
            parentDir: "/xiaoshouyi/app/Z.app",
            revision: "1",
            isFolder: false,
          },
        ],
      });
    }
    if (
      parsed.pathname === "/api/meta/services/getFileContent/page-a" ||
      parsed.pathname === "/api/meta/services/getFileContent/page-z"
    ) {
      return jsonResponse(200, { raw_text: JSON.stringify({ type: "metadata" }) });
    }
    return jsonResponse(404, {});
  };
  const controller = createM45BackgroundController({
    fetchImpl,
    analysisClient: {
      async loadSuperpageDocument() {},
      async buildOrUpdateSuperpageGraph() {},
      async analyzeSuperpageSelection(selection) {
        starts.push(selection.source_path);
        return { status: "ready" };
      },
    },
  });

  const result = await controller.bootstrapAndIndex({
    base_url: "https://autocrm-test.xiaoshouyi.com",
    access_token: "one-shot",
    current_source_path: "app/Z.app/Page.spg",
    initial_limit: 1,
  });

  assert.equal(result.ok, true);
  assert.deepEqual(starts, ["app/Z.app/Page.spg"]);
  assert.equal(controller.state.background.queue[0].source_path, "app/A.app/Page.spg");
});

test("M45 background rejects anonymous whoami without leaking token", async () => {
  const controller = createM45BackgroundController({
    fetchImpl: async () => jsonResponse(200, { anonymous: true }),
  });

  const result = await controller.bootstrapWithAccessToken({
    base_url: "https://example.test",
    access_token: "secret-token-value",
  });

  assert.equal(result.ok, false);
  assert.equal(result.diagnostics[0].code, "SESSION_BOOTSTRAP_ANONYMOUS");
  assert.equal(JSON.stringify(result).includes("secret-token-value"), false);
});

test("M45 background foreground selection is queued before background items", async () => {
  const controller = createM45BackgroundController({
    fetchImpl: async () => jsonResponse(200, { userId: "u1" }),
  });
  controller.state.visible_index.status = "ready";
  controller.state.visible_index.projects = [{ project_name: "p" }];
  controller.state.session = { base_url: "https://example.test" };
  controller.seedBackgroundQueue([
    {
      project_name: "p",
      source_path: "app/Other.app/Page.spg",
      file_id: "f2",
      revision: "1",
      analyzable: true,
    },
  ]);

  const queued = controller.enqueueForegroundSelection({
    project_name: "p",
    source_path: "app/Current.app/Page.spg",
    file_id: "f1",
  });

  assert.equal(queued.queued, true);
  assert.equal(controller.state.background.queue[0].source_path, "app/Current.app/Page.spg");
});

test("M45 background queue orders dependencies before same app and same module", async () => {
  const controller = createM45BackgroundController({
    fetchImpl: async () => jsonResponse(200, { userId: "u1" }),
  });
  const queue = controller.seedBackgroundQueue(
    [
      { project_name: "p", source_path: "data/Table.tbl", file_id: "dep", revision: "1", analyzable: true },
      { project_name: "p", source_path: "app/Other.app/Page.spg", file_id: "other", revision: "1", analyzable: true },
      { project_name: "p", source_path: "app/Test.app/Child.spg", file_id: "same-app", revision: "1", analyzable: true },
      { project_name: "p", source_path: "report/Page.spg", file_id: "other-module", revision: "1", analyzable: true },
    ],
    {
      current_source_path: "app/Test.app/Main.spg",
      current_dependency_paths: ["data/Table.tbl"],
    },
  );

  assert.deepEqual(
    queue.map((item) => item.source_path),
    ["data/Table.tbl", "app/Test.app/Child.spg", "app/Other.app/Page.spg", "report/Page.spg"],
  );
});

test("M45 selection message triggers foreground processing with selection-specific cache key", async () => {
  const { fetchImpl } = createFetchStub();
  const runtimeCalls = [];
  const cache = createMemoryMetadataCache();
  const controller = createM45BackgroundController({
    fetchImpl,
    cache,
    analysisClient: {
      async loadSuperpageDocument() {},
      async buildOrUpdateSuperpageGraph() {},
      async analyzeSuperpageSelection(selection) {
        runtimeCalls.push(selection);
        return { status: "ready", target: selection.active_component_id };
      },
    },
  });
  await controller.bootstrapAndIndex({
    base_url: "https://autocrm-test.xiaoshouyi.com",
    access_token: "one-shot",
    initial_limit: 1,
  });

  const result = await controller.handleMessage({
    type: "metadata-checker-selection-changed",
    payload: {
      project_name: "xiaoshouyi",
      source_path: "app/Test.app/Page.spg",
      active_component_id: "input1",
      selected_component_ids: ["input1"],
    },
  });

  assert.equal(result.ok, true);
  assert.equal(result.artifact_ready, true);
  assert.equal(Boolean(result.artifact && result.artifact.result), true);
  assert.equal(result.artifact_key.includes("analysis-artifact|foreground|"), true);
  assert.equal(result.artifact.analysis_status, "ready");
  assert.equal(runtimeCalls.some((selection) => selection.active_component_id === "input1"), true);
  const keys = await cache.keys();
  assert.equal(keys.some((key) => key.includes("analysis-artifact|background|")), true);
  assert.equal(keys.some((key) => key.includes("analysis-artifact|foreground|")), true);
});

test("M45 local-graph analyze message forces default depth and visible_hop and normalizes limits", async () => {
  const cache = createMemoryMetadataCache();
  const localGraphCalls = [];
  const controller = createM45BackgroundController({
    cache,
    analysisClient: {
      async initRuntime() {
        throw new Error("local graph bridge should not reset runtime");
      },
      async analyzeLocalGraph(selection, options) {
        localGraphCalls.push({ method: "analyzeLocalGraph", selection, options });
        return { status: "ready", target: selection.active_component_id };
      },
    },
    fetchImpl: () => {
      throw new Error("should not fetch raw text in local graph bridge");
    },
  });

  controller.state.session = { base_url: "https://example.test" };
  controller.state.visible_index = {
    status: "ready",
    projects: [{ project_name: "p" }],
    files: [
      {
        project_name: "p",
        source_path: "app/Test.app/Page.spg",
        file_id: "page-1",
        revision: "1",
        analyzable: true,
      },
    ],
  };
  await cache.set("analysis-artifact|background|https://example.test|p|app/Test.app/Page.spg|page-1|1", {
    kind: "metadata-analysis-artifact",
    source_path: "app/Test.app/Page.spg",
    project_name: "p",
    file_id: "page-1",
    revision: "1",
    analysis_status: "ready",
  });

  const result = await controller.handleMessage({
    type: "metadata-checker-analyze-local-graph",
    payload: {
      source_path: "app/Test.app/Page.spg",
      active_component_id: "input-1",
      selected_component_ids: ["input-1"],
      options: {
        depth: 5,
        visible_hop: 2,
        max_nodes: "100",
        max_edges: "80",
      },
    },
  });

  assert.equal(result.ok, true);
  assert.equal(localGraphCalls.length, 1);
  assert.equal(localGraphCalls[0].method, "analyzeLocalGraph");
  assert.equal(localGraphCalls[0].selection.active_component_id, "input-1");
  assert.equal(localGraphCalls[0].options.depth, 2);
  assert.equal(localGraphCalls[0].options.visible_hop, 1);
  assert.equal(localGraphCalls[0].options.max_nodes, 100);
  assert.equal(localGraphCalls[0].options.max_edges, 80);
  assert.equal(result.artifact_ready, true);
  assert.equal(result.artifact.analysis_status, "ready");
});

test("M45 local-graph analyze returns unsupported diagnostic when runtime API missing", async () => {
  const cache = createMemoryMetadataCache();
  const controller = createM45BackgroundController({
    cache,
    analysisClient: {},
    fetchImpl: () => {
      throw new Error("should not fetch raw text in local graph bridge");
    },
  });

  controller.state.session = { base_url: "https://example.test" };
  controller.state.visible_index = {
    status: "ready",
    projects: [{ project_name: "p" }],
    files: [
      {
        project_name: "p",
        source_path: "app/Test.app/Page.spg",
        file_id: "page-1",
        revision: "1",
        analyzable: true,
      },
    ],
  };
  await cache.set("analysis-artifact|background|https://example.test|p|app/Test.app/Page.spg|page-1|1", {
    kind: "metadata-analysis-artifact",
    source_path: "app/Test.app/Page.spg",
    project_name: "p",
    file_id: "page-1",
    revision: "1",
    analysis_status: "ready",
  });

  const result = await controller.handleMessage({
    type: "metadata-checker-analyze-local-graph",
    payload: {
      source_path: "app/Test.app/Page.spg",
      active_component_id: "input-1",
    },
  });

  assert.equal(result.ok, false);
  assert.equal(
    result.diagnostics[0].code,
    "METADATA_CHECKER_ANALYZE_LOCAL_GRAPH_UNSUPPORTED",
  );
});

test("M45 local-graph analyze returns artifact missing diagnostic when source not loaded", async () => {
  const cache = createMemoryMetadataCache();
  const localGraphCalls = [];
  const controller = createM45BackgroundController({
    cache,
    analysisClient: {
      async analyzeLocalGraph() {
        localGraphCalls.push("called");
        return { status: "ready" };
      },
    },
    fetchImpl: () => {
      throw new Error("should not fetch raw text in local graph bridge");
    },
  });

  controller.state.session = { base_url: "https://example.test" };
  controller.state.visible_index = {
    status: "ready",
    projects: [{ project_name: "p" }],
    files: [
      {
        project_name: "p",
        source_path: "app/Test.app/Page.spg",
        file_id: "page-1",
        revision: "1",
        analyzable: true,
      },
    ],
  };

  const result = await controller.handleMessage({
    type: "metadata-checker-analyze-local-graph",
    payload: {
      source_path: "app/Test.app/Page.spg",
      active_component_id: "input-1",
    },
  });

  assert.equal(result.ok, false);
  assert.equal(
    result.diagnostics[0].code,
    "METADATA_CHECKER_LOCAL_GRAPH_ARTIFACT_MISSING",
  );
  assert.equal(localGraphCalls.length, 0);
});

test("M45 pre-bootstrap selection is replayed and prioritized after visible index ready", async () => {
  const analyzeCalls = [];
  const cache = createMemoryMetadataCache();
  const controller = createM45BackgroundController({
    fetchImpl: createFetchStub().fetchImpl,
    cache,
    analysisClient: {
      async loadSuperpageDocument() {},
      async buildOrUpdateSuperpageGraph() {},
      async analyzeSuperpageSelection(selection, options) {
        analyzeCalls.push({ selection, options });
        return { status: "ready", target: selection.active_component_id };
      },
    },
  });

  const queued = await controller.handleMessage({
    type: "metadata-checker-selection-changed",
    payload: {
      project_name: "xiaoshouyi",
      source_path: "app/Test.app/Page.spg",
      active_component_id: "input1",
      selected_component_ids: ["input1"],
    },
  });

  assert.equal(queued.queued, true);
  assert.equal(queued.pending, true);
  assert.equal(queued.indexing_status, "waiting_for_metadata");
  assert.equal(queued.retry_available, true);
  assert.equal(queued.background.indexing_status, "waiting_for_metadata");
  assert.equal(queued.background.current_source_path, "app/Test.app/Page.spg");
  assert.equal(controller.state.background.pending_foreground.length, 1);
  assert.equal(controller.state.background.queue.length, 0);

  const result = await controller.bootstrapAndIndex({
    base_url: "https://autocrm-test.xiaoshouyi.com",
    access_token: "one-shot",
    initial_limit: 3,
  });

  assert.equal(result.ok, true);
  assert.equal(controller.state.background.pending_foreground.length, 0);
  assert.equal(analyzeCalls[0].options.mode, "foreground");
  assert.equal(analyzeCalls[0].options.include_conditions, true);
  assert.equal(analyzeCalls[0].selection.source_path, "app/Test.app/Page.spg");
  const keys = await cache.keys();
  assert.equal(
    keys.some((key) => key.includes("analysis-artifact|foreground|") && key.includes("selected:input1")),
    true,
  );
});

test("M45 selection pause/resume prevents additional background items until resume", async () => {
  const deferreds = [];
  const fetchImpl = async () => jsonResponse(200, { raw_text: JSON.stringify({ type: "metadata" }) });
  const controller = createM45BackgroundController({
    fetchImpl,
    analysisClient: {
      async loadSuperpageDocument() {},
      async buildOrUpdateSuperpageGraph() {},
      async analyzeSuperpageSelection() {
        const deferred = createDeferred();
        deferreds.push(deferred);
        await deferred.promise;
        return { status: "ready" };
      },
    },
  });
  controller.state.session = { base_url: "https://example.test" };
  controller.seedBackgroundQueue([
    {
      project_name: "p",
      source_path: "app/A.app/Page.spg",
      file_id: "f1",
      revision: "1",
      analyzable: true,
    },
    {
      project_name: "p",
      source_path: "app/B.app/Page.spg",
      file_id: "f2",
      revision: "1",
      analyzable: true,
    },
  ]);

  const processing = controller.handleMessage({
    type: "metadata-checker-background-process",
    payload: { limit: 2, max_concurrency: 1 },
  });
  while (deferreds.length === 0) {
    await Promise.resolve();
  }
  assert.equal(deferreds.length, 1);
  await controller.handleMessage({ type: "metadata-checker-background-pause" });
  deferreds[0].resolve();
  await processing;
  assert.equal(deferreds.length, 1);
  assert.equal(controller.state.background.queue.length, 1);

  await controller.handleMessage({ type: "metadata-checker-background-resume" });
  const resumed = controller.handleMessage({
    type: "metadata-checker-background-process",
    payload: { limit: 1, max_concurrency: 1 },
  });
  while (deferreds.length < 2) {
    await Promise.resolve();
  }
  assert.equal(deferreds.length, 2);
  deferreds[1].resolve();
  await resumed;
});

test("M45 pause while workers reserved preserves reserved order", async () => {
  const deferreds = [];
  const starts = [];
  const fetchImpl = async () => jsonResponse(200, { raw_text: JSON.stringify({ type: "metadata" }) });
  const controller = createM45BackgroundController({
    fetchImpl,
    analysisClient: {
      async loadSuperpageDocument() {},
      async buildOrUpdateSuperpageGraph() {},
      async analyzeSuperpageSelection(selection) {
        starts.push(selection.source_path);
        if (selection.source_path === "app/A.app/Page.spg") {
          const deferred = createDeferred();
          deferreds.push(deferred);
          await deferred.promise;
        }
        return { status: "ready" };
      },
    },
  });
  controller.state.session = { base_url: "https://example.test" };
  controller.seedBackgroundQueue([
    {
      project_name: "p",
      source_path: "app/A.app/Page.spg",
      file_id: "f1",
      revision: "1",
      analyzable: true,
    },
    {
      project_name: "p",
      source_path: "app/B.app/Page.spg",
      file_id: "f2",
      revision: "1",
      analyzable: true,
    },
    {
      project_name: "p",
      source_path: "app/C.app/Page.spg",
      file_id: "f3",
      revision: "1",
      analyzable: true,
    },
    {
      project_name: "p",
      source_path: "app/D.app/Page.spg",
      file_id: "f4",
      revision: "1",
      analyzable: true,
    },
  ]);

  const processing = controller.handleMessage({
    type: "metadata-checker-background-process",
    payload: { limit: 3, max_concurrency: 3, min_interval_ms: 10 },
  });
  while (starts.length === 0) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  assert.equal(starts[0], "app/A.app/Page.spg");

  await controller.handleMessage({ type: "metadata-checker-background-pause" });
  assert.equal(starts.length, 1);

  deferreds[0].resolve();
  await processing;

  assert.deepEqual(
    controller.state.background.queue.map((item) => item.source_path),
    [
      "app/B.app/Page.spg",
      "app/C.app/Page.spg",
      "app/D.app/Page.spg",
    ],
  );
  assert.equal(controller.state.background.queue.length, 3);

  await controller.handleMessage({ type: "metadata-checker-background-resume" });
  const resumed = controller.handleMessage({
    type: "metadata-checker-background-process",
    payload: { limit: 3, max_concurrency: 3, min_interval_ms: 10 },
  });
  while (starts.length < 4) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  assert.deepEqual(starts, [
    "app/A.app/Page.spg",
    "app/B.app/Page.spg",
    "app/C.app/Page.spg",
    "app/D.app/Page.spg",
  ]);
  await resumed;
});

test("M45 background process limit reserves slots before completion under high concurrency", async () => {
  const deferreds = [];
  const fetchImpl = async () => jsonResponse(200, { raw_text: JSON.stringify({ type: "metadata" }) });
  const controller = createM45BackgroundController({
    fetchImpl,
    analysisClient: {
      async loadSuperpageDocument() {},
      async buildOrUpdateSuperpageGraph() {},
      async analyzeSuperpageSelection() {
        const deferred = createDeferred();
        deferreds.push(deferred);
        await deferred.promise;
        return { status: "ready" };
      },
    },
  });
  controller.state.session = { base_url: "https://example.test" };
  controller.seedBackgroundQueue([
    {
      project_name: "p",
      source_path: "app/A.app/Page.spg",
      file_id: "f1",
      revision: "1",
      analyzable: true,
    },
    {
      project_name: "p",
      source_path: "app/B.app/Page.spg",
      file_id: "f2",
      revision: "1",
      analyzable: true,
    },
    {
      project_name: "p",
      source_path: "app/C.app/Page.spg",
      file_id: "f3",
      revision: "1",
      analyzable: true,
    },
  ]);

  const processing = controller.handleMessage({
    type: "metadata-checker-background-process",
    payload: { limit: 1, max_concurrency: 3 },
  });

  while (deferreds.length === 0) {
    await Promise.resolve();
  }
  assert.equal(deferreds.length, 1);
  assert.equal(controller.state.background.queue.length, 2);

  deferreds[0].resolve();
  await processing;

  assert.equal(controller.state.background.queue.length, 2);
  assert.equal(controller.state.background.processed, 1);
});

test("M45 background min_interval_ms is serialized across concurrent workers", async () => {
  let now = 0;
  const starts = [];
  const sleepCalls = [];
  const fetchImpl = async () => jsonResponse(200, { raw_text: JSON.stringify({ type: "metadata" }) });
  const controller = createM45BackgroundController({
    fetchImpl,
    clock: () => now,
    sleep: (ms) => {
      sleepCalls.push(ms);
      return new Promise((resolve) => {
        globalThis.setTimeout(() => {
          now += ms;
          resolve();
        }, 0);
      });
    },
    analysisClient: {
      async loadSuperpageDocument() {},
      async buildOrUpdateSuperpageGraph() {},
      async analyzeSuperpageSelection() {
        starts.push(now);
        return { status: "ready" };
      },
    },
  });
  controller.state.session = { base_url: "https://example.test" };
  controller.seedBackgroundQueue([
    {
      project_name: "p",
      source_path: "app/A.app/Page.spg",
      file_id: "f1",
      revision: "1",
      analyzable: true,
    },
    {
      project_name: "p",
      source_path: "app/B.app/Page.spg",
      file_id: "f2",
      revision: "1",
      analyzable: true,
    },
    {
      project_name: "p",
      source_path: "app/C.app/Page.spg",
      file_id: "f3",
      revision: "1",
      analyzable: true,
    },
  ]);

  await controller.handleMessage({
    type: "metadata-checker-background-process",
    payload: { limit: 3, max_concurrency: 3, min_interval_ms: 10 },
  });

  assert.equal(starts.length, 3);
  assert.equal(starts[0], 0);
  assert.ok(starts[1] - starts[0] >= 10);
  assert.ok(starts[2] - starts[1] >= 10);
  assert.ok(starts[1] > starts[0]);
  assert.ok(starts[2] > starts[1]);
  assert.equal(sleepCalls.some((delay) => delay > 0), true);
});

test("M45 process calls are serialized across concurrent invocations", async () => {
  let now = 0;
  const starts = [];
  const deferreds = [];
  const controller = createM45BackgroundController({
    clock: () => now,
    sleep: (ms) => new Promise((resolve) => {
      globalThis.setTimeout(() => {
        now += ms;
        resolve();
      }, 0);
    }),
    fetchImpl: async () => jsonResponse(200, { raw_text: JSON.stringify({ type: "metadata" }) }),
    analysisClient: {
      async loadSuperpageDocument() {},
      async buildOrUpdateSuperpageGraph() {},
      async analyzeSuperpageSelection(selection) {
        starts.push(now);
        if (selection.source_path === "app/A.app/Page.spg") {
          const deferred = createDeferred();
          deferreds.push(deferred);
          await deferred.promise;
        }
        return { status: "ready" };
      },
    },
  });
  controller.state.session = { base_url: "https://example.test" };
  controller.seedBackgroundQueue([
    {
      project_name: "p",
      source_path: "app/A.app/Page.spg",
      file_id: "f1",
      revision: "1",
      analyzable: true,
    },
    {
      project_name: "p",
      source_path: "app/B.app/Page.spg",
      file_id: "f2",
      revision: "1",
      analyzable: true,
    },
  ]);

  const first = controller.handleMessage({
    type: "metadata-checker-background-process",
    payload: { limit: 1, max_concurrency: 1, min_interval_ms: 10 },
  });
  const second = controller.handleMessage({
    type: "metadata-checker-background-process",
    payload: { limit: 1, max_concurrency: 1, min_interval_ms: 10 },
  });

  while (starts.length === 0) {
    await Promise.resolve();
  }
  assert.equal(starts.length, 1);
  assert.equal(deferreds.length, 1);

  deferreds[0].resolve();
  await first;
  await second;

  assert.equal(starts.length, 2);
  assert.ok(starts[1] - starts[0] >= 10);
});

test("M45 foreground selection stays prioritized during running background processing", async () => {
  const starts = [];
  const deferreds = [];
  const controller = createM45BackgroundController({
    fetchImpl: async () => jsonResponse(200, { raw_text: JSON.stringify({ type: "metadata" }) }),
    analysisClient: {
      async loadSuperpageDocument() {},
      async buildOrUpdateSuperpageGraph() {},
      async analyzeSuperpageSelection(selection) {
        starts.push(selection.source_path);
        if (selection.source_path === "app/A.app/Page.spg") {
          const deferred = createDeferred();
          deferreds.push(deferred);
          await deferred.promise;
        }
        return { status: "ready" };
      },
    },
  });
  controller.state.session = { base_url: "https://example.test" };
  controller.state.visible_index.status = "ready";
  controller.state.visible_index.projects = [{ project_name: "p" }];
  controller.seedBackgroundQueue([
    {
      project_name: "p",
      source_path: "app/A.app/Page.spg",
      file_id: "f1",
      revision: "1",
      analyzable: true,
    },
    {
      project_name: "p",
      source_path: "app/B.app/Page.spg",
      file_id: "f2",
      revision: "1",
      analyzable: true,
    },
  ]);

  const backgroundProcess = controller.handleMessage({
    type: "metadata-checker-background-process",
    payload: { limit: 1, max_concurrency: 1 },
  });

  while (deferreds.length === 0) {
    await Promise.resolve();
  }
  assert.equal(starts[0], "app/A.app/Page.spg");

  const foregroundMessage = controller.handleMessage({
    type: "metadata-checker-selection-changed",
    payload: {
      project_name: "p",
      source_path: "app/Foreground.app/Page.spg",
      file_id: "f3",
      active_component_id: "fg1",
    },
  });

  assert.equal(starts.length, 1);

  deferreds[0].resolve();
  await backgroundProcess;
  await foregroundMessage;

  assert.equal(starts[0], "app/A.app/Page.spg");
  await controller.handleMessage({
    type: "metadata-checker-background-process",
    payload: { limit: 2, max_concurrency: 1 },
  });

  const foregroundIndex = starts.indexOf("app/Foreground.app/Page.spg");
  const backgroundIndex = starts.indexOf("app/B.app/Page.spg");
  assert.ok(foregroundIndex >= 0);
  assert.ok(backgroundIndex >= 0);
  assert.ok(foregroundIndex < backgroundIndex);
});

test("M45 foreground selection can return from cache while background item remains deferred", async () => {
  const starts = [];
  const deferredA = createDeferred();
  const deferredB = createDeferred();
  const cache = createMemoryMetadataCache();
  const controller = createM45BackgroundController({
    fetchImpl: async () => jsonResponse(200, { raw_text: JSON.stringify({ type: "metadata" }) }),
    cache,
    analysisClient: {
      async loadSuperpageDocument() {},
      async buildOrUpdateSuperpageGraph() {},
      async analyzeSuperpageSelection(selection) {
        starts.push(selection.source_path);
        if (selection.source_path === "app/A.app/Page.spg") {
          await deferredA.promise;
        }
        if (selection.source_path === "app/B.app/Page.spg") {
          await deferredB.promise;
        }
        return { status: "ready" };
      },
    },
  });
  const foregroundPayload = {
    project_name: "p",
    source_path: "app/Foreground.app/Page.spg",
    file_id: "f3",
    active_component_id: "fg1",
    selected_component_ids: ["fg1"],
  };

  controller.state.session = { base_url: "https://example.test" };
  controller.state.visible_index.status = "ready";
  controller.state.visible_index.projects = [{ project_name: "p" }];
  controller.seedBackgroundQueue([
    {
      project_name: "p",
      source_path: "app/A.app/Page.spg",
      file_id: "f1",
      revision: "1",
      analyzable: true,
    },
    {
      project_name: "p",
      source_path: "app/B.app/Page.spg",
      file_id: "f2",
      revision: "1",
      analyzable: true,
    },
  ]);

  const processing = controller.handleMessage({
    type: "metadata-checker-background-process",
    payload: { limit: 3, max_concurrency: 1 },
  });

  while (starts.length === 0) {
    await Promise.resolve();
  }

  const firstSelection = controller.handleMessage({
    type: "metadata-checker-selection-changed",
    payload: foregroundPayload,
  });

  deferredA.resolve();

  await waitForCacheKey(cache, (key) => key.includes("analysis-artifact|foreground|") && key.includes("selected:fg1"));

  const secondSelection = controller.handleMessage({
    type: "metadata-checker-selection-changed",
    payload: foregroundPayload,
  });

  const winner = await Promise.race([
    secondSelection.then(() => "selection"),
    deferredB.promise.then(() => "background"),
  ]);
  assert.equal(winner, "selection");

  const secondResult = await secondSelection;
  assert.equal(secondResult.ok, true);
  assert.equal(secondResult.queued, false);
  assert.equal(secondResult.artifact_ready, true);
  assert.equal(Boolean(secondResult.artifact && secondResult.artifact.source_path), true);

  deferredB.resolve({ status: "ready" });
  await firstSelection;
  await processing;

  assert.equal(starts.includes("app/A.app/Page.spg"), true);
  assert.equal(starts.includes("app/Foreground.app/Page.spg"), true);
  assert.equal(starts.includes("app/B.app/Page.spg"), true);
  assert.equal(
    starts.indexOf("app/Foreground.app/Page.spg"),
    1,
  );
  assert.ok(starts.indexOf("app/Foreground.app/Page.spg") < starts.indexOf("app/B.app/Page.spg"));
  assert.equal(controller.state.background.processed <= controller.state.background.total, true);
});

test("M45 background min_interval_ms is enforced across batches", async () => {
  let now = 0;
  const starts = [];
  const fetchImpl = async () => jsonResponse(200, { raw_text: JSON.stringify({ type: "metadata" }) });
  const controller = createM45BackgroundController({
    fetchImpl,
    clock: () => now,
    sleep: (ms) => new Promise((resolve) => {
      globalThis.setTimeout(() => {
        now += ms;
        resolve();
      }, 0);
    }),
    analysisClient: {
      async loadSuperpageDocument() {},
      async buildOrUpdateSuperpageGraph() {},
      async analyzeSuperpageSelection() {
        starts.push(now);
        return { status: "ready" };
      },
    },
  });
  controller.state.session = { base_url: "https://example.test" };
  controller.seedBackgroundQueue([
    {
      project_name: "p",
      source_path: "app/A.app/Page.spg",
      file_id: "f1",
      revision: "1",
      analyzable: true,
    },
    {
      project_name: "p",
      source_path: "app/B.app/Page.spg",
      file_id: "f2",
      revision: "1",
      analyzable: true,
    },
    {
      project_name: "p",
      source_path: "app/C.app/Page.spg",
      file_id: "f3",
      revision: "1",
      analyzable: true,
    },
    {
      project_name: "p",
      source_path: "app/D.app/Page.spg",
      file_id: "f4",
      revision: "1",
      analyzable: true,
    },
  ]);

  await controller.handleMessage({
    type: "metadata-checker-background-process",
    payload: { limit: 3, max_concurrency: 3, min_interval_ms: 10 },
  });
  await controller.handleMessage({
    type: "metadata-checker-background-process",
    payload: { limit: 1, max_concurrency: 3, min_interval_ms: 10 },
  });

  assert.equal(starts.length, 4);
  assert.ok(starts[1] - starts[0] >= 10);
  assert.ok(starts[2] - starts[1] >= 10);
  assert.ok(starts[3] - starts[2] >= 10);
});

test("M45 background pause/resume keeps pending foreground items in queue", async () => {
  const deferreds = [];
  const fetchImpl = async () => jsonResponse(200, { raw_text: JSON.stringify({ type: "metadata" }) });
  const controller = createM45BackgroundController({
    fetchImpl,
    analysisClient: {
      async loadSuperpageDocument() {},
      async buildOrUpdateSuperpageGraph() {},
      async analyzeSuperpageSelection() {
        const deferred = createDeferred();
        deferreds.push(deferred);
        await deferred.promise;
        return { status: "ready" };
      },
    },
  });
  controller.state.session = { base_url: "https://example.test" };
  controller.state.visible_index.status = "idle";
  const pending = await controller.handleMessage({
    type: "metadata-checker-selection-changed",
    payload: {
      project_name: "p",
      source_path: "app/Pending.app/Page.spg",
      file_id: "pending",
    },
  });
  assert.equal(pending.pending, true);
  assert.equal(controller.state.background.pending_foreground.length, 1);

  controller.state.visible_index.status = "ready";
  controller.state.visible_index.projects = [{ project_name: "p" }];
  controller.seedBackgroundQueue([
    {
      project_name: "p",
      source_path: "app/Background.app/Page.spg",
      file_id: "background",
      revision: "1",
      analyzable: true,
    },
  ]);

  assert.equal(controller.state.background.pending_foreground.length, 0);
  assert.equal(controller.state.background.queue[0].source_path, "app/Pending.app/Page.spg");

  const processing = controller.handleMessage({
    type: "metadata-checker-background-process",
    payload: { limit: 1, max_concurrency: 1 },
  });
  while (deferreds.length === 0) {
    await Promise.resolve();
  }
  assert.equal(controller.state.background.queue.length, 1);

  await controller.handleMessage({ type: "metadata-checker-background-pause" });
  deferreds[0].resolve();
  await processing;
  assert.equal(controller.state.background.queue.length, 1);
  assert.equal(controller.state.background.queue[0].source_path, "app/Background.app/Page.spg");

  await controller.handleMessage({ type: "metadata-checker-background-resume" });
  const resumed = controller.handleMessage({
    type: "metadata-checker-background-process",
    payload: { limit: 1, max_concurrency: 1 },
  });
  while (deferreds.length < 2) {
    await Promise.resolve();
  }
  deferreds[1].resolve();
  await resumed;
  assert.equal(controller.state.background.queue.length, 0);
});

test("M45 background fetches raw metadata and calls injected runtime analyzer", async () => {
  const { fetchImpl } = createFetchStub();
  const runtimeCalls = [];
  const controller = createM45BackgroundController({
    fetchImpl,
    clock: () => 2000,
    analysisClient: {
      async initRuntime(options) {
        runtimeCalls.push({ method: "initRuntime", options });
        return { status: "ready" };
      },
      async loadSuperpageDocument(sourcePath, rawText) {
        runtimeCalls.push({ method: "loadSuperpageDocument", sourcePath, rawText });
        return { status: "ready" };
      },
      async buildOrUpdateSuperpageGraph(sourcePath) {
        runtimeCalls.push({ method: "buildOrUpdateSuperpageGraph", sourcePath });
        return { status: "ready" };
      },
      async analyzeSuperpageSelection(selection, options) {
        runtimeCalls.push({ method: "analyzeSuperpageSelection", selection, options });
        return { status: "ready", target: selection.source_path, items: [], diagnostics: [] };
      },
    },
  });

  await controller.bootstrapAndIndex({
    base_url: "https://autocrm-test.xiaoshouyi.com",
    access_token: "one-shot",
    current_source_path: "app/Test.app/Page.spg",
    initial_limit: 1,
  });

  assert.deepEqual(
    runtimeCalls.map((call) => call.method),
    ["initRuntime", "loadSuperpageDocument", "buildOrUpdateSuperpageGraph", "analyzeSuperpageSelection"],
  );
  assert.equal(runtimeCalls[0].options.project_ref, "xiaoshouyi");
  assert.equal(runtimeCalls[1].sourcePath, "app/Test.app/Page.spg");
  const loadedRawMetadata = JSON.parse(runtimeCalls[1].rawText);
  assert.equal(loadedRawMetadata.raw_text, "literal field that must stay raw text");
  assert.equal(loadedRawMetadata.components.length, 96);
  assert.equal(runtimeCalls[1].rawText.includes("\"raw_text\":\"literal field that must stay raw text\""), true);
  assert.ok(runtimeCalls[1].rawText.length > 10000);
  assert.equal(runtimeCalls[3].selection.file_id, "page-1");
  assert.equal(runtimeCalls[3].options.mode, "background");
  assert.equal(runtimeCalls[3].options.include_priority, false);
  assert.equal(runtimeCalls[3].options.include_conditions, true);
  assert.equal(runtimeCalls[3].options.include_dataflow, false);
  assert.equal(controller.state.background.processed, 1);
});

test("M47 refresh visible manifest uses WASM diff content queue", async () => {
  let pageRevision = "7";
  const fetchCalls = [];
  const fetchImpl = async (url, init = {}) => {
    fetchCalls.push({ url, init });
    const parsed = new URL(url);
    if (parsed.pathname === "/api/me/whoami") {
      return jsonResponse(200, { userId: "u1", userName: "User One" });
    }
    if (parsed.pathname === "/api/me/getPermissionInfo") {
      return jsonResponse(200, {
        metaProjects: [{ projectName: "xiaoshouyi", desc: "crm" }],
      });
    }
    if (parsed.pathname === "/api/meta/services/getFileChildren/xiaoshouyi") {
      return jsonResponse(200, {
        children: [{ name: "app", parentDir: "/xiaoshouyi", isFolder: true }],
      });
    }
    if (parsed.pathname === "/api/meta/services/getFileDescendant/xiaoshouyi/app") {
      return jsonResponse(200, {
        files: [
          {
            id: "page-1",
            name: "Page.spg",
            parentDir: "/xiaoshouyi/app/Test.app",
            revision: pageRevision,
            isFolder: false,
          },
          {
            id: "table-1",
            name: "Data.tbl",
            parentDir: "/xiaoshouyi/data/tables",
            revision: "8",
            isFolder: false,
          },
        ],
      });
    }
    if (parsed.pathname === "/api/meta/services/getFileContent/page-1") {
      return jsonResponse(200, JSON.stringify({
        id: "page-1",
        components: [{ id: "input1", type: "Input" }],
      }));
    }
    if (parsed.pathname === "/api/meta/services/getFileContent/table-1") {
      return jsonResponse(200, JSON.stringify({ id: "table-1" }));
    }
    return jsonResponse(404, {});
  };
  const runtimeCalls = [];
  const controller = createM45BackgroundController({
    fetchImpl,
    clock: (() => {
      let now = 4000;
      return () => {
        now += 5;
        return now;
      };
    })(),
    analysisClient: {
      async initRuntime(options) {
        runtimeCalls.push({ method: "initRuntime", options });
        return { status: "ready" };
      },
      async diffVisibleManifest(previousManifestJson, visibleManifestJson, projectRef) {
        runtimeCalls.push({
          method: "diffVisibleManifest",
          previousManifestJson,
          visibleManifestJson,
          projectRef,
        });
        const previous = JSON.parse(previousManifestJson);
        const visible = JSON.parse(visibleManifestJson);
        assert.equal(projectRef, "xiaoshouyi");
        assert.equal(previous.find((entry) => entry.source_path.endsWith("Page.spg"))?.revision, "7");
        assert.equal(visible.find((entry) => entry.source_path.endsWith("Page.spg"))?.revision, "9");
        return {
          items: [
            {
              kind: "visible_manifest_diff",
              detail: {
                added: [],
                modified: [
                  {
                    source_path: "app/Test.app/Page.spg",
                    revision: "9",
                  },
                ],
                deleted: [],
                unchanged: [
                  {
                    source_path: "data/tables/Data.tbl",
                    revision: "8",
                  },
                ],
                changed_files: [
                  {
                    project_ref: "xiaoshouyi",
                    source_path: "app/Test.app/Page.spg",
                    file_id: "page-1",
                    revision: "9",
                  },
                ],
                content_queue: [
                  {
                    project_ref: "xiaoshouyi",
                    source_path: "app/Test.app/Page.spg",
                    file_id: "page-1",
                    revision: "9",
                  },
                ],
                previous_manifest_count: previous.length,
                visible_manifest_count: visible.length,
                timing: { total_ms: 6 },
                diagnostics: [],
              },
            },
          ],
        };
      },
      async loadSuperpageDocument(sourcePath, rawText) {
        runtimeCalls.push({ method: "loadSuperpageDocument", sourcePath, rawText });
        return { status: "ready" };
      },
      async buildOrUpdateSuperpageGraph(sourcePath) {
        runtimeCalls.push({ method: "buildOrUpdateSuperpageGraph", sourcePath });
        return { status: "ready" };
      },
      async analyzeSuperpageSelection(selection) {
        runtimeCalls.push({ method: "analyzeSuperpageSelection", selection });
        return { status: "ready", target: selection.source_path, items: [], diagnostics: [] };
      },
    },
  });

  await controller.bootstrapAndIndex({
    base_url: "https://autocrm-test.xiaoshouyi.com",
    access_token: "one-shot",
    project_name: "xiaoshouyi",
    initial_limit: 0,
  });

  pageRevision = "9";
  const result = await controller.refreshVisibleManifest({
    base_url: "https://autocrm-test.xiaoshouyi.com",
    project_name: "xiaoshouyi",
  });

  assert.equal(result.ok, true);
  assert.equal(result.manifest_diff.modified, 1);
  assert.equal(result.manifest_diff.content_queue_count, 1);
  assert.equal(result.timings.manifest_diff_ms, 6);
  assert.deepEqual(
    runtimeCalls.map((call) => call.method),
    [
      "diffVisibleManifest",
      "initRuntime",
      "loadSuperpageDocument",
      "buildOrUpdateSuperpageGraph",
      "analyzeSuperpageSelection",
    ],
  );
  assert.equal(runtimeCalls[2].sourcePath, "app/Test.app/Page.spg");
  assert.equal(
    fetchCalls.filter((call) => new URL(call.url).pathname === "/api/meta/services/getFileContent/page-1").length,
    1,
  );
  assert.equal(
    fetchCalls.filter((call) => new URL(call.url).pathname === "/api/meta/services/getFileContent/table-1").length,
    0,
  );
});

test("M45 bootstrap maps post-whoami 401 to session cookie diagnostic", async () => {
  const fetchImpl = async (url) => {
    const parsed = new URL(url);
    if (parsed.pathname === "/api/me/whoami") {
      return jsonResponse(200, { userId: "u1" });
    }
    if (parsed.pathname === "/api/me/getPermissionInfo") {
      return jsonResponse(401, { error: "unauthorized" });
    }
    return jsonResponse(404, {});
  };
  const controller = createM45BackgroundController({ fetchImpl });

  const result = await controller.bootstrapAndIndex({
    base_url: "https://autocrm-test.xiaoshouyi.com",
    access_token: "one-shot",
  });

  assert.equal(result.ok, false);
  assert.equal(result.diagnostics[0].code, "SESSION_COOKIE_NOT_ESTABLISHED");
});

test("M45 WASM analysis client loads extension runtime module", async () => {
  const calls = [];
  const client = createWasmAnalysisClient({
    chromeRuntime: {
      getURL(path) {
        return `chrome-extension://id/${path}`;
      },
    },
    importImpl: async (specifier) => {
      calls.push({ method: "import", specifier });
      return {
        default: async (wasmUrl) => calls.push({ method: "init", wasmUrl }),
        initRuntime: async (optionsJson) => {
          calls.push({ method: "initRuntime", optionsJson });
          return JSON.stringify({ status: "ready" });
        },
        loadSuperpageDocument: async (sourcePath, rawText) => {
          calls.push({ method: "loadSuperpageDocument", sourcePath, rawText });
          return JSON.stringify({ status: "ready" });
        },
        buildOrUpdateSuperpageGraph: async (sourcePath) => {
          calls.push({ method: "buildOrUpdateSuperpageGraph", sourcePath });
          return JSON.stringify({ status: "ready" });
        },
        analyzeSuperpageSelection: async (selectionJson, optionsJson) => {
          calls.push({ method: "analyzeSuperpageSelection", selectionJson, optionsJson });
          return JSON.stringify({ status: "ready" });
        },
      };
    },
  });

  await client.initRuntime({ project_ref: "p" });
  await client.loadSuperpageDocument("app/Page.spg", "{}");
  await client.buildOrUpdateSuperpageGraph("app/Page.spg");
  await client.analyzeSuperpageSelection({ source_path: "app/Page.spg" }, { mode: "background" });

  assert.deepEqual(
    calls.map((call) => call.method),
    [
      "import",
      "init",
      "initRuntime",
      "loadSuperpageDocument",
      "buildOrUpdateSuperpageGraph",
      "analyzeSuperpageSelection",
    ],
  );
  assert.equal(calls[0].specifier, "chrome-extension://id/metadata_checker.js");
  assert.equal(calls[1].wasmUrl, "chrome-extension://id/metadata_checker_bg.wasm");
  assert.equal(calls[2].optionsJson, JSON.stringify({ project_ref: "p" }));
});

test("M45 WASM analysis client can use statically imported module in service worker", async () => {
  const calls = [];
  const client = createWasmAnalysisClient({
    chromeRuntime: {
      getURL(path) {
        return `chrome-extension://id/${path}`;
      },
    },
    wasmInit: async (wasmUrl) => calls.push({ method: "init", wasmUrl }),
    wasmModule: {
      initRuntime: async (optionsJson) => {
        calls.push({ method: "initRuntime", optionsJson });
        return JSON.stringify({ status: "ready" });
      },
      loadSuperpageDocument: async (sourcePath, rawText) => {
        calls.push({ method: "loadSuperpageDocument", sourcePath, rawText });
        return JSON.stringify({ status: "ready" });
      },
      buildOrUpdateSuperpageGraph: async (sourcePath) => {
        calls.push({ method: "buildOrUpdateSuperpageGraph", sourcePath });
        return JSON.stringify({ status: "ready" });
      },
      analyzeSuperpageSelection: async (selectionJson, optionsJson) => {
        calls.push({ method: "analyzeSuperpageSelection", selectionJson, optionsJson });
        return JSON.stringify({ status: "ready" });
      },
    },
  });

  await client.initRuntime({ project_ref: "p" });
  await client.loadSuperpageDocument("app/Page.spg", "{}");
  await client.buildOrUpdateSuperpageGraph("app/Page.spg");
  await client.analyzeSuperpageSelection({ source_path: "app/Page.spg" }, { mode: "background" });

  assert.deepEqual(
    calls.map((call) => call.method),
    [
      "init",
      "initRuntime",
      "loadSuperpageDocument",
      "buildOrUpdateSuperpageGraph",
      "analyzeSuperpageSelection",
    ],
  );
  assert.equal(calls[0].wasmUrl, "chrome-extension://id/metadata_checker_bg.wasm");
  assert.equal(calls[1].optionsJson, JSON.stringify({ project_ref: "p" }));
});

test("M45 WASM analysis client defaults to offscreen document when available", async () => {
  const calls = [];
  const client = createWasmAnalysisClient({
    chromeApi: {
      runtime: {
        getURL(path) {
          return `chrome-extension://id/${path}`;
        },
        async getContexts(query) {
          calls.push({ method: "getContexts", query });
          return [];
        },
        sendMessage(message, callback) {
          calls.push({ method: "sendMessage", message });
          callback({
            ok: true,
            result: JSON.stringify({ status: "ready" }),
          });
        },
      },
      offscreen: {
        async createDocument(options) {
          calls.push({ method: "createDocument", options });
        },
      },
    },
  });

  await client.loadSuperpageDocument("app/Page.spg", "{}");

  assert.deepEqual(
    calls.map((call) => call.method),
    ["getContexts", "createDocument", "sendMessage"],
  );
  assert.equal(calls[1].options.url, "offscreen.html");
  assert.deepEqual(calls[1].options.reasons, ["WORKERS"]);
  assert.equal(calls[2].message.type, "metadata-checker-offscreen-wasm-call");
  assert.equal(calls[2].message.payload.method, "loadSuperpageDocument");
  assert.equal(calls[2].message.payload.wasm_url, "chrome-extension://id/metadata_checker_bg.wasm");
});

test("M45 offscreen WASM analysis client calls runtime methods through extension messages", async () => {
  const calls = [];
  const client = createOffscreenWasmAnalysisClient({
    chromeApi: {
      runtime: {
        getURL(path) {
          return `chrome-extension://id/${path}`;
        },
        async getContexts() {
          return [{ documentUrl: "chrome-extension://id/offscreen.html" }];
        },
        sendMessage(message, callback) {
          calls.push(message);
          callback({
            ok: true,
            result: JSON.stringify({ status: "ready", method: message.payload.method }),
          });
        },
      },
      offscreen: {
        async createDocument() {
          throw new Error("should not create existing offscreen document");
        },
      },
    },
  });

  const result = await client.analyzeSuperpageSelection(
    { source_path: "app/Page.spg" },
    { mode: "foreground" },
  );

  assert.equal(result.status, "ready");
  assert.equal(result.method, "analyzeSuperpageSelection");
  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0].payload.args, [
    JSON.stringify({ source_path: "app/Page.spg" }),
    JSON.stringify({ mode: "foreground" }),
  ]);
});

test("M45 offscreen WASM analysis client surfaces runtime lastError", async () => {
  const chromeApi = {
    runtime: {
      lastError: null,
      getURL(path) {
        return `chrome-extension://id/${path}`;
      },
      async getContexts() {
        return [{ documentUrl: "chrome-extension://id/offscreen.html" }];
      },
      sendMessage(_message, callback) {
        chromeApi.runtime.lastError = { message: "offscreen document is closed" };
        callback();
        chromeApi.runtime.lastError = null;
      },
    },
    offscreen: {
      async createDocument() {
        throw new Error("should not create existing offscreen document");
      },
    },
  };
  const client = createOffscreenWasmAnalysisClient({ chromeApi });

  await assert.rejects(
    () => client.loadSuperpageDocument("app/Page.spg", "{}"),
    /offscreen document is closed/,
  );
});

test("M45 offscreen WASM analysis client surfaces diagnostic response failures", async () => {
  const client = createOffscreenWasmAnalysisClient({
    chromeApi: {
      runtime: {
        getURL(path) {
          return `chrome-extension://id/${path}`;
        },
        async getContexts() {
          return [{ documentUrl: "chrome-extension://id/offscreen.html" }];
        },
        sendMessage(_message, callback) {
          callback({
            ok: false,
            diagnostic: {
              code: "OFFSCREEN_WASM_CALL_FAILED",
              message: "forced method failure",
            },
          });
        },
      },
      offscreen: {
        async createDocument() {
          throw new Error("should not create existing offscreen document");
        },
      },
    },
  });

  await assert.rejects(
    () => client.loadSuperpageDocument("app/Page.spg", "{}"),
    /forced method failure/,
  );
});

test("M45 offscreen WASM analysis client times out missing responses", async () => {
  const client = createOffscreenWasmAnalysisClient({
    requestTimeoutMs: 5,
    chromeApi: {
      runtime: {
        getURL(path) {
          return `chrome-extension://id/${path}`;
        },
        async getContexts() {
          return [{ documentUrl: "chrome-extension://id/offscreen.html" }];
        },
        sendMessage() {},
      },
      offscreen: {
        async createDocument() {
          throw new Error("should not create existing offscreen document");
        },
      },
    },
  });

  await assert.rejects(
    () => client.loadSuperpageDocument("app/Page.spg", "{}"),
    /did not respond in time/,
  );
});

test("M45 offscreen WASM analysis client serializes concurrent document creation", async () => {
  const calls = [];
  const createDocument = createDeferred();
  const chromeApi = {
    runtime: {
      getURL(path) {
        return `chrome-extension://id/${path}`;
      },
      async getContexts() {
        calls.push({ method: "getContexts" });
        return [];
      },
      sendMessage(message, callback) {
        calls.push({ method: "sendMessage", message });
        callback({
          ok: true,
          result: JSON.stringify({ status: "ready", method: message.payload.method }),
        });
      },
    },
    offscreen: {
      async createDocument(options) {
        calls.push({ method: "createDocument", options });
        await createDocument.promise;
      },
    },
  };
  const client = createOffscreenWasmAnalysisClient({ chromeApi });
  const first = client.loadSuperpageDocument("app/First.spg", "{}");
  const second = client.buildOrUpdateSuperpageGraph("app/Second.spg");

  while (!calls.some((call) => call.method === "createDocument")) {
    await Promise.resolve();
  }
  createDocument.resolve();
  const results = await Promise.all([first, second]);

  assert.equal(
    calls.filter((call) => call.method === "createDocument").length,
    1,
  );
  assert.deepEqual(results.map((result) => result.status), ["ready", "ready"]);
});

test("M45 background records per-file fetch failure and continues queue", async () => {
  const { fetchImpl } = createFetchStubWithContentFailure();
  const controller = createM45BackgroundController({ fetchImpl, clock: () => 3000 });

  const result = await controller.bootstrapAndIndex({
    base_url: "https://autocrm-test.xiaoshouyi.com",
    access_token: "one-shot",
    current_source_path: "app/Test.app/Page.spg",
    initial_limit: 2,
  });

  assert.equal(result.ok, true);
  assert.equal(result.background.processed, 2);
  assert.equal(result.background.failed, 1);
  assert.equal(result.background.last_failed_source_path, "app/Test.app/Page.spg");
  assert.equal(result.background.retry_available, true);
  assert.equal(controller.state.last_diagnostic.code, "BACKGROUND_ANALYSIS_FAILED");
  assert.equal(
    controller.state.events.some(
      (event) =>
        event.event === M45_EVENT_TYPES.BACKGROUND_ANALYSIS_PROGRESS &&
        event.payload.processed === 2,
    ),
    true,
  );
});

test("M45 background records runtime analysis failure in indexing progress", async () => {
  const { fetchImpl } = createFetchStub();
  const controller = createM45BackgroundController({
    fetchImpl,
    analysisClient: {
      async loadSuperpageDocument() {},
      async buildOrUpdateSuperpageGraph() {},
      async analyzeSuperpageSelection() {
        throw new Error("runtime analysis failed");
      },
    },
  });

  const result = await controller.bootstrapAndIndex({
    base_url: "https://autocrm-test.xiaoshouyi.com",
    access_token: "one-shot",
    current_source_path: "app/Test.app/Page.spg",
    initial_limit: 1,
  });

  assert.equal(result.ok, true);
  assert.equal(result.background.processed, 1);
  assert.equal(result.background.failed, 1);
  assert.equal(result.background.last_failed_source_path, "app/Test.app/Page.spg");
  assert.equal(result.background.retry_available, true);
  const failedArtifact = result.background.last_failed_source_path;
  assert.equal(failedArtifact, "app/Test.app/Page.spg");
});

test("M45 metadata cache rejects sensitive payloads", async () => {
  const cache = createMemoryMetadataCache();
  await assert.rejects(
    () => cache.set("k", { access_token: "secret-token-value" }),
    /sensitive keys/,
  );
  await assert.rejects(
    () => cache.set("k", { raw_text: "token=secret-token-value" }),
    /sensitive value/,
  );
});

test("M45 popup status returns stable redacted contract envelope", async () => {
  const controller = createM45BackgroundController({
    clock: () => 2000,
  });
  controller.state.session = {
    status: "ready",
    base_url: "https://autocrm-test.xiaoshouyi.com",
    user_id: "u1",
    user_name: "Test User",
    access_token: "secret-token",
    bootstrapped_at: 1000,
  };
  controller.state.visible_index = {
    status: "ready",
    projects: [{ project_name: "xiaoshouyi", project_id: "project-id" }],
    files: [
      {
        project_name: "xiaoshouyi",
        source_path: "app/Test.app/Page.spg",
        file_id: "page-1",
        extension: "spg",
        analyzable: true,
        revision: "r1",
        raw_text: "literal raw_text=should-not-leak",
        token: "secret-token",
      },
    ],
    analyzable_count: 1,
    indexed_at: 1200,
    secret: "token=abc",
  };
  controller.state.background = {
    status: "ready",
    queue: [1, 2],
    pending_foreground: [1],
    processed: 1,
    total: 2,
    active: 0,
    failed: 0,
    paused: false,
    max_concurrency: 2,
    min_interval_ms: 10,
    process_gate: Promise.resolve(),
    next_run_at: 1400,
    indexing_status: "ready",
    current_source_path: "app/Test.app/Page.spg",
    last_processed_source_path: "app/Test.app/Page.spg",
    last_failed_source_path: null,
    retry_available: false,
  };
  controller.state.cache_stats = {
    hits: 4,
    misses: 2,
    secret: "token=abc",
  };
  controller.state.last_diagnostic = {
    severity: "warning",
    code: "TEST_WARNING",
    message: "leak token=popup-token-should-not-leak cookie=popup-cookie-should-not-leak password=popup-password-should-not-leak",
  };

  const result = await controller.handleMessage({ type: "metadata-checker-popup-status" });
  assert.equal(result.ok, true);

  const popupState = result.state;
  assert.equal(popupState.runtime?.diagnostic, null);
  assert.equal(popupState.offscreen?.diagnostic, null);
  assert.equal(popupState.version?.diagnostic, null);
  assert.equal(popupState.runtime.available, false);
  assert.equal(popupState.offscreen.available, false);
  assert.equal(popupState.version.value, null);

  assert.deepEqual(Object.keys(popupState).sort(), [
    "background",
    "cache_stats",
    "last_diagnostic",
    "offscreen",
    "runtime",
    "session",
    "updated_at",
    "version",
    "visible_index",
  ]);

  const backgroundKeys = [
    "current_source_path",
    "failed",
    "indexing_status",
    "last_failed_source_path",
    "last_processed_source_path",
    "max_concurrency",
    "min_interval_ms",
    "next_run_at",
    "paused",
    "pending_foreground_count",
    "processed",
    "queue_length",
    "retry_available",
    "status",
    "total",
    "active",
  ];
  assert.deepEqual(Object.keys(popupState.background).sort(), backgroundKeys.sort());
  assert.equal(popupState.background.status, "ready");
  assert.equal(popupState.background.indexing_status, "ready");

  const visibleIndexKeys = [
    "analyzable_count",
    "files",
    "indexed_at",
    "projects",
    "status",
  ];
  assert.deepEqual(Object.keys(popupState.visible_index).sort(), visibleIndexKeys.sort());
  assert.equal(popupState.visible_index.files.length, 1);
  assert.equal(Object.prototype.hasOwnProperty.call(popupState.visible_index.files[0], "raw_text"), false);
  assert.equal(Object.prototype.hasOwnProperty.call(popupState.visible_index.files[0], "token"), false);
  assert.equal(popupState.visible_index.files[0].analyzable, true);

  assert.equal(popupState.last_diagnostic.code, "TEST_WARNING");
  assert.equal(popupState.last_diagnostic.message.includes("popup-token-should-not-leak"), false);
  assert.equal(popupState.last_diagnostic.message.includes("popup-cookie-should-not-leak"), false);
  assert.equal(popupState.last_diagnostic.message.includes("popup-password-should-not-leak"), false);
  assert.equal(popupState.last_diagnostic.message.includes("***"), true);

  const serialized = JSON.stringify(popupState);
  assert.equal(serialized.includes("secret-token"), false);
  assert.equal(serialized.includes("token=abc"), false);
  assert.equal(serialized.includes("cookie=def"), false);
  assert.equal(serialized.includes("password=ghi"), false);
  assert.equal(serialized.includes("raw_text=sensitive"), false);
});

test("M45 popup status schema stable across failed/indexing/ready states", async () => {
  const controller = createM45BackgroundController();
  const expectedBackgroundKeys = [
    "active",
    "current_source_path",
    "failed",
    "indexing_status",
    "last_failed_source_path",
    "last_processed_source_path",
    "max_concurrency",
    "min_interval_ms",
    "next_run_at",
    "paused",
    "pending_foreground_count",
    "processed",
    "queue_length",
    "retry_available",
    "status",
    "total",
  ];
  const cases = [
    { background_status: "failed", indexing_status: "failed" },
    { background_status: "queued", indexing_status: "indexing" },
    { background_status: "ready", indexing_status: "ready" },
  ];
  for (const item of cases) {
    controller.state.background.status = item.background_status;
    controller.state.background.indexing_status = item.indexing_status;
    const result = await controller.handleMessage({ type: "metadata-checker-popup-status" });
    assert.equal(result.ok, true);
    assert.equal(result.state.background.status, item.background_status);
    assert.equal(result.state.background.indexing_status, item.indexing_status);
    assert.deepEqual(Object.keys(result.state.background).sort(), expectedBackgroundKeys.sort());
  }
});

test("M45 default cache uses IndexedDB when available and never stores sensitive keys", async () => {
  const records = new Map();
  const fakeIndexedDB = {
    open() {
      const request = {};
      queueMicrotask(() => {
        const db = {
          objectStoreNames: { contains: () => true },
          createObjectStore: () => {},
          transaction() {
            return {
              objectStore() {
                return {
                  get(key) {
                    const getRequest = {};
                    queueMicrotask(() => {
                      getRequest.result = records.get(key);
                      getRequest.onsuccess?.();
                    });
                    return getRequest;
                  },
                  put(record) {
                    const putRequest = {};
                    queueMicrotask(() => {
                      records.set(record.key, record);
                      putRequest.result = record.key;
                      putRequest.onsuccess?.();
                    });
                    return putRequest;
                  },
                  getAllKeys() {
                    const keysRequest = {};
                    queueMicrotask(() => {
                      keysRequest.result = Array.from(records.keys());
                      keysRequest.onsuccess?.();
                    });
                    return keysRequest;
                  },
                };
              },
            };
          },
        };
        request.result = db;
        request.onsuccess?.();
      });
      return request;
    },
  };
  const cache = createDefaultMetadataCache({ indexedDB: fakeIndexedDB });

  await cache.set("artifact|1", { source_path: "app/Page.spg", analysis_status: "ready" });
  assert.deepEqual(await cache.keys(), ["artifact|1"]);
  assert.equal((await cache.get("artifact|1")).source_path, "app/Page.spg");
  assert.ok(records.has("artifact|1"));
  await assert.rejects(
    () => createIndexedDbMetadataCache({ indexedDB: null }).keys(),
    /IndexedDB is unavailable/,
  );
  await assert.rejects(
    () => cache.set("artifact|2", { cookie: "secret" }),
    /sensitive keys/,
  );
});

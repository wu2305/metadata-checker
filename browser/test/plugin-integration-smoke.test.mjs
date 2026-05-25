/**
 * M40.9 Plugin Integration Smoke Tests
 *
 * 覆盖 controller 完整链路：
 * - selection -> fetch metadata -> runtime load/build/analyze -> renderer render
 * - runtime failure -> renderer error
 * - metadata fetch failure -> renderer diagnostic
 * - duplicate init 不重复 runtime、listener、panel、Service Worker registration
 * - 5MB raw text 不经 selection payload，只经 provider -> runtime
 */

import { describe, it } from "node:test";
import assert from "node:assert";
import { fileURLToPath } from "node:url";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const controllerSourcePath = join(
  __dirname,
  "../integration/metadata-checker-controller.mjs",
);
const controllerSource = readFileSync(controllerSourcePath, "utf-8");

function createFakeRenderer() {
  const renderCalls = [];
  return {
    renderCalls,
    renderAnalysis(result) {
      renderCalls.push({ type: "renderAnalysis", result });
    },
    renderError(errorEnvelope) {
      renderCalls.push({ type: "renderError", errorEnvelope });
    },
  };
}

function createFakeGraphRenderer() {
  const renderCalls = [];
  const handlers = new Map();
  return {
    renderCalls,
    render(result) {
      renderCalls.push({ type: "render", result });
      return Promise.resolve({ renderer: "fake", result });
    },
    renderGraph(result) {
      renderCalls.push({ type: "renderGraph", result });
      return Promise.resolve({ renderer: "fake", result });
    },
    renderError(errorEnvelope) {
      renderCalls.push({ type: "renderError", errorEnvelope });
      return Promise.resolve({ renderer: "fake", errorEnvelope });
    },
    on(eventName, handler) {
      handlers.set(eventName, handler);
    },
    emit(eventName, payload) {
      const handler = handlers.get(eventName);
      if (handler) {
        return handler(payload);
      }
      return undefined;
    },
  };
}

function createFakeHost() {
  const events = [];
  return {
    events,
    emit(eventName, payload) {
      events.push({ eventName, payload });
    },
    getEvents(name) {
      return events.filter((e) => e.eventName === name);
    },
  };
}

function createFakeProvider(options = {}) {
  const callLog = [];
  const fixtures = options.fixtures ?? new Map();
  const shouldFailInfo = options.shouldFailInfo ?? false;
  const shouldFailContent = options.shouldFailContent ?? false;

  function log(method, args) {
    callLog.push({ method, args: Array.from(args) });
  }

  return {
    callLog,
    async getFileInfo(fileRef) {
      log("getFileInfo", arguments);
      if (shouldFailInfo) {
        return {
          status: "error",
          code: "REMOTE_FETCH_FAILED",
          message: "mock info failure",
        };
      }
      const fixture = fixtures.get(fileRef?.source_path);
      if (!fixture) {
        return {
          status: "error",
          code: "REMOTE_FETCH_NOT_FOUND",
          message: "not found",
        };
      }
      return {
        source_path: fileRef.source_path,
        file_id: fileRef.file_id ?? null,
        revision: fixture.revision ?? null,
        content_type: fixture.content_type ?? "unknown",
        updated_at: fixture.updated_at ?? null,
      };
    },
    async getFileContent(fileRef) {
      log("getFileContent", arguments);
      if (shouldFailContent) {
        return {
          status: "error",
          code: "REMOTE_FETCH_FAILED",
          message: "mock content failure",
        };
      }
      const fixture = fixtures.get(fileRef?.source_path);
      if (!fixture) {
        return {
          status: "error",
          code: "REMOTE_FETCH_NOT_FOUND",
          message: "not found",
        };
      }
      return {
        source_path: fileRef.source_path,
        file_id: fileRef.file_id ?? null,
        revision: fixture.revision ?? null,
        content_type: fixture.content_type ?? "unknown",
        raw_text: fixture.raw_text ?? "",
      };
    },
    async getRelatedFiles(fileRef) {
      log("getRelatedFiles", arguments);
      return [];
    },
  };
}

function createFakeRuntimeClient(options = {}) {
  const callLog = [];
  const shouldFail = options.shouldFail ?? false;
  const failMethod = options.failMethod ?? null;

  function log(method, args) {
    callLog.push({ method, args: Array.from(args) });
  }

  const client = {
    callLog,
    _kind: options.kind ?? "page",

    initRuntime(options) {
      log("initRuntime", arguments);
      if (shouldFail && failMethod === "initRuntime") {
        return Promise.reject(new Error("initRuntime failed"));
      }
      return Promise.resolve({ status: "ready", items: [], diagnostics: [] });
    },
    runtimeStatus() {
      log("runtimeStatus", arguments);
      if (shouldFail && failMethod === "runtimeStatus") {
        return Promise.reject(new Error("runtimeStatus failed"));
      }
      return Promise.resolve({ status: "ready", items: [], diagnostics: [] });
    },
    loadSuperpageDocument(sourcePath, rawText) {
      log("loadSuperpageDocument", arguments);
      if (shouldFail && failMethod === "loadSuperpageDocument") {
        return Promise.reject(new Error("loadSuperpageDocument failed"));
      }
      return Promise.resolve({
        status: "ready",
        target: sourcePath,
        items: [],
        diagnostics: [],
      });
    },
    buildOrUpdateSuperpageGraph(sourcePath) {
      log("buildOrUpdateSuperpageGraph", arguments);
      if (shouldFail && failMethod === "buildOrUpdateSuperpageGraph") {
        return Promise.reject(new Error("buildOrUpdateSuperpageGraph failed"));
      }
      return Promise.resolve({
        status: "ready",
        target: sourcePath,
        items: [],
        diagnostics: [],
      });
    },
    analyzeSuperpageSelection(selection, opts) {
      log("analyzeSuperpageSelection", arguments);
      if (shouldFail && failMethod === "analyzeSuperpageSelection") {
        return Promise.reject(new Error("analyzeSuperpageSelection failed"));
      }
      return Promise.resolve({
        status: "ready",
        target: selection.active_component_id ?? selection.source_path,
        items: [{ kind: "analysis", label: "Analysis", detail: { selection } }],
        diagnostics: [],
      });
    },
  };
  if (options.supportsRemoteLoad) {
    client.loadRemoteSuperpageDocument = function (fileRef, opts) {
      log("loadRemoteSuperpageDocument", arguments);
      if (shouldFail && failMethod === "loadRemoteSuperpageDocument") {
        return Promise.reject(new Error("loadRemoteSuperpageDocument failed"));
      }
      if (options.remoteLoadReturnsError) {
        return Promise.resolve({
          status: "error",
          diagnostics: [
            {
              severity: "error",
              code: "REMOTE_FETCH_FAILED",
              message: "remote runtime load failed",
            },
          ],
        });
      }
      return Promise.resolve({
        status: "ready",
        target: fileRef?.source_path,
        items: [],
        diagnostics: [],
      });
    };
  }
  return client;
}

async function loadController() {
  const { createMetadataCheckerController } = await import(
    controllerSourcePath
  );
  return createMetadataCheckerController;
}

describe("createMetadataCheckerController parameter validation", async () => {
  const createController = await loadController();

  it("throws when plugin is missing", () => {
    assert.throws(() => {
      createController({});
    }, /plugin with onSelectionChanged is required/);
  });

  it("throws when provider is missing", () => {
    assert.throws(() => {
      createController({ plugin: { onSelectionChanged() {} } });
    }, /provider with getFileContent is required/);
  });

  it("throws when runtimeClient is missing", () => {
    assert.throws(() => {
      createController({
        plugin: { onSelectionChanged() {} },
        provider: { getFileContent() {} },
      });
    }, /runtimeClient with loadSuperpageDocument or loadRemoteSuperpageDocument is required/);
  });

  it("throws when renderer is missing", () => {
    assert.throws(() => {
      createController({
        plugin: { onSelectionChanged() {} },
        provider: { getFileContent() {} },
        runtimeClient: { loadSuperpageDocument() {} },
      });
    }, /renderer with renderAnalysis is required/);
  });
});

describe("integration success path", async () => {
  const createController = await loadController();

  it("selection -> provider -> runtime -> renderer complete pipeline", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const graphRenderer = createFakeGraphRenderer();
    const provider = createFakeProvider({
      fixtures: new Map([
        [
          "pages/demo.spg",
          {
            raw_text: JSON.stringify({ components: [] }),
            content_type: "super_page",
          },
        ],
      ]),
    });
    const runtimeClient = createFakeRuntimeClient();

    // plugin core
    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      graphRenderer,
      host,
      logger: null,
    });

    const selection = {
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      selected_component_ids: ["comp-1"],
      active_component_id: "comp-1",
    };

    const result = await controller.handleSelection(selection);

    assert.strictEqual(result.status, "ready");
    assert.strictEqual(renderer.renderCalls.length, 1);
    assert.strictEqual(renderer.renderCalls[0].type, "renderAnalysis");
    assert.strictEqual(graphRenderer.renderCalls.length, 1);
    assert.strictEqual(graphRenderer.renderCalls[0].type, "renderGraph");

    // provider 被调用
    const contentCalls = provider.callLog.filter(
      (c) => c.method === "getFileContent",
    );
    assert.strictEqual(contentCalls.length, 1);
    assert.strictEqual(contentCalls[0].args[0].source_path, "pages/demo.spg");
    assert.strictEqual(contentCalls[0].args[0].file_id, "demo-123");
    assert.strictEqual(contentCalls[0].args[0].project_name, null);

    // runtime 被调用 load/build/analyze
    assert.ok(
      runtimeClient.callLog.some((c) => c.method === "loadSuperpageDocument"),
    );
    assert.ok(
      runtimeClient.callLog.some(
        (c) => c.method === "buildOrUpdateSuperpageGraph",
      ),
    );
    assert.ok(
      runtimeClient.callLog.some(
        (c) => c.method === "analyzeSuperpageSelection",
      ),
    );
  });

  it("does not put raw_text into selection payload", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const bigText = "x".repeat(6 * 1024 * 1024); // 6MB
    const provider = createFakeProvider({
      fixtures: new Map([
        [
          "pages/big.spg",
          {
            raw_text: bigText,
            content_type: "super_page",
          },
        ],
      ]),
    });
    const runtimeClient = createFakeRuntimeClient();

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      host,
      logger: null,
    });

    const selection = {
      source_path: "pages/big.spg",
      file_id: "big-123",
      selected_component_ids: ["comp-1"],
      active_component_id: "comp-1",
    };

    await controller.handleSelection(selection);

    // 验证 selection payload 中没有 raw_text
    const analyzeCalls = runtimeClient.callLog.filter(
      (c) => c.method === "analyzeSuperpageSelection",
    );
    assert.strictEqual(analyzeCalls.length, 1);
    const passedSelection = analyzeCalls[0].args[0];
    assert.strictEqual(passedSelection.raw_text, undefined);

    // 但 loadSuperpageDocument 接收到了完整 raw text
    const loadCalls = runtimeClient.callLog.filter(
      (c) => c.method === "loadSuperpageDocument",
    );
    assert.strictEqual(loadCalls.length, 1);
    assert.strictEqual(loadCalls[0].args[1].length, bigText.length);
  });

  it("passes project_name from selection to provider fileRef", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const provider = createFakeProvider({
      fixtures: new Map([
        [
          "pages/demo.spg",
          {
            raw_text: JSON.stringify({ components: [] }),
            content_type: "super_page",
          },
        ],
      ]),
    });
    const runtimeClient = createFakeRuntimeClient();

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      host,
      logger: null,
    });

    await controller.handleSelection({
      source_path: "pages/demo.spg",
      file_id: "",
      project_name: "analyzer",
      selected_component_ids: ["comp-1"],
      active_component_id: "comp-1",
    });

    const contentCall = provider.callLog.find(
      (c) => c.method === "getFileContent",
    );
    assert.strictEqual(contentCall.args[0].project_name, "analyzer");
    assert.strictEqual(contentCall.args[0].file_id, "");
  });

  it("runtime-first mode loads remote document without page provider fetch", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const provider = createFakeProvider({
      fixtures: new Map([
        [
          "pages/demo.spg",
          {
            raw_text: JSON.stringify({ components: [] }),
            content_type: "super_page",
          },
        ],
      ]),
    });
    const runtimeClient = createFakeRuntimeClient({ supportsRemoteLoad: true });

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      host,
      logger: null,
    });

    const result = await controller.handleSelection({
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      project_name: "analyzer",
      selected_component_ids: ["comp-1"],
      active_component_id: "comp-1",
    });

    assert.strictEqual(result.status, "ready");
    assert.strictEqual(
      provider.callLog.some((c) => c.method === "getFileContent"),
      false,
    );
    const remoteLoadCall = runtimeClient.callLog.find(
      (c) => c.method === "loadRemoteSuperpageDocument",
    );
    assert.ok(remoteLoadCall);
    assert.deepStrictEqual(remoteLoadCall.args[0], {
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      project_name: "analyzer",
    });
    assert.ok(
      runtimeClient.callLog.some(
        (c) => c.method === "buildOrUpdateSuperpageGraph",
      ),
    );
    assert.ok(
      runtimeClient.callLog.some(
        (c) => c.method === "analyzeSuperpageSelection",
      ),
    );
  });

  it("runtime-first mode can run without page provider when runtime remote load succeeds", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const runtimeClient = createFakeRuntimeClient({ supportsRemoteLoad: true });

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      runtimeClient,
      renderer,
      host,
      logger: null,
    });

    const result = await controller.handleSelection({
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      selected_component_ids: ["comp-1"],
      active_component_id: "comp-1",
    });

    assert.strictEqual(result.status, "ready");
    assert.ok(
      runtimeClient.callLog.some(
        (c) => c.method === "loadRemoteSuperpageDocument",
      ),
    );
  });

  it("stale analysis result does not render old graph over latest selection", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const graphRenderer = createFakeGraphRenderer();
    const provider = createFakeProvider({
      fixtures: new Map([
        [
          "pages/demo.spg",
          {
            raw_text: JSON.stringify({ components: [] }),
            content_type: "super_page",
          },
        ],
      ]),
    });
    const runtimeClient = createFakeRuntimeClient();
    runtimeClient.analyzeSuperpageSelection = function (selection) {
      this.callLog.push({ method: "analyzeSuperpageSelection", args: Array.from(arguments) });
      const delay = selection.active_component_id === "slow" ? 20 : 0;
      return new Promise((resolve) => {
        setTimeout(() => {
          resolve({
            status: "ready",
            target: selection.active_component_id,
            items: [],
            diagnostics: [],
          });
        }, delay);
      });
    };

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      graphRenderer,
      host,
      logger: null,
    });

    const slow = controller.handleSelection({
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      selected_component_ids: ["slow"],
      active_component_id: "slow",
    });
    const fast = controller.handleSelection({
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      selected_component_ids: ["fast"],
      active_component_id: "fast",
    });

    await Promise.all([slow, fast]);

    const graphTargets = graphRenderer.renderCalls.map(
      (call) => call.result?.target ?? call.errorEnvelope?.target,
    );
    assert.deepStrictEqual(graphTargets, ["fast"]);
    assert.strictEqual(host.getEvents("analysis_stale_discarded").length, 1);
  });

  it("cache miss emits cache miss event and runs provider/runtime pipeline", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const graphRenderer = createFakeGraphRenderer();
    const provider = createFakeProvider({
      fixtures: new Map([
        [
          "pages/demo.spg",
          {
            raw_text: JSON.stringify({ components: [] }),
            content_type: "super_page",
          },
        ],
      ]),
    });
    const runtimeClient = createFakeRuntimeClient();

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      graphRenderer,
      host,
      logger: null,
    });

    const first = {
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      selected_component_ids: ["comp-1"],
      active_component_id: "comp-1",
    };
    const second = {
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      selected_component_ids: ["comp-2"],
      active_component_id: "comp-2",
    };

    await controller.handleSelection(first);
    const missCountBefore = host.getEvents("analysis_cache_miss").length;

    const result = await controller.handleSelection(second);

    assert.strictEqual(result.status, "ready");
    assert.strictEqual(host.getEvents("analysis_cache_miss").length, missCountBefore + 1);
    assert.strictEqual(host.getEvents("analysis_cache_hit").length, 0);
    assert.ok(runtimeClient.callLog.some((c) => c.method === "loadSuperpageDocument"));
    assert.ok(runtimeClient.callLog.some((c) => c.method === "buildOrUpdateSuperpageGraph"));
    assert.ok(runtimeClient.callLog.some((c) => c.method === "analyzeSuperpageSelection"));
    const graphTargets = graphRenderer.renderCalls.map(
      (call) => call.result?.target ?? call.errorEnvelope?.target,
    );
    assert.deepStrictEqual(graphTargets.at(-1), "comp-2");
  });

  it("cache hit does not repeat runtime load/build/analyze", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const graphRenderer = createFakeGraphRenderer();
    const provider = createFakeProvider({
      fixtures: new Map([
        [
          "pages/demo.spg",
          {
            raw_text: JSON.stringify({ components: [] }),
            content_type: "super_page",
          },
        ],
      ]),
    });
    const runtimeClient = createFakeRuntimeClient();

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      graphRenderer,
      host,
      logger: null,
    });

    const selection = {
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      selected_component_ids: ["comp-1"],
      active_component_id: "comp-1",
    };

    await controller.handleSelection(selection);

    const providerLoadCallsBefore = provider.callLog.filter(
      (c) => c.method === "getFileContent",
    ).length;
    const runtimeLoadCallsBefore = runtimeClient.callLog.filter(
      (c) => c.method === "loadSuperpageDocument",
    ).length;
    const buildCallsBefore = runtimeClient.callLog.filter(
      (c) => c.method === "buildOrUpdateSuperpageGraph",
    ).length;
    const analyzeCallsBefore = runtimeClient.callLog.filter(
      (c) => c.method === "analyzeSuperpageSelection",
    ).length;

    const cachedResult = await controller.handleSelection(selection);

    assert.strictEqual(cachedResult.status, "ready");
    assert.strictEqual(host.getEvents("analysis_cache_hit").length, 1);
    assert.strictEqual(
      host.getEvents("analysis_cache_miss").length,
      1,
    );
    assert.strictEqual(provider.callLog.filter((c) => c.method === "getFileContent").length, providerLoadCallsBefore);
    assert.strictEqual(
      runtimeClient.callLog.filter((c) => c.method === "loadSuperpageDocument").length,
      runtimeLoadCallsBefore,
    );
    assert.strictEqual(
      runtimeClient.callLog.filter((c) => c.method === "buildOrUpdateSuperpageGraph").length,
      buildCallsBefore,
    );
    assert.strictEqual(
      runtimeClient.callLog.filter((c) => c.method === "analyzeSuperpageSelection").length,
      analyzeCallsBefore,
    );

    assert.strictEqual(renderer.renderCalls.length, 2);
    assert.strictEqual(renderer.renderCalls[1].type, "renderAnalysis");
    assert.strictEqual(graphRenderer.renderCalls[1].type, "renderGraph");
    assert.strictEqual(graphRenderer.renderCalls[1].result.target, "comp-1");
  });

  it("selection debounce only keeps the last selection result", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const graphRenderer = createFakeGraphRenderer();
    const provider = createFakeProvider({
      fixtures: new Map([
        [
          "pages/demo.spg",
          {
            raw_text: JSON.stringify({ components: [] }),
            content_type: "super_page",
          },
        ],
      ]),
    });
    const runtimeClient = createFakeRuntimeClient();
    const analyzeTargets = [];
    runtimeClient.analyzeSuperpageSelection = function (selection) {
      this.callLog.push({ method: "analyzeSuperpageSelection", args: Array.from(arguments) });
      analyzeTargets.push(selection.active_component_id);
      return new Promise((resolve) => {
        setTimeout(() => {
          resolve({
            status: "ready",
            target: selection.active_component_id,
            items: [{ kind: "analysis", label: "Analysis", detail: { selection } }],
            diagnostics: [],
          });
        }, 10);
      });
    };

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      graphRenderer,
      host,
      logger: null,
      selectionDebounceMs: 20,
    });

    const first = controller.handleSelection({
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      selected_component_ids: ["comp-1"],
      active_component_id: "comp-1",
    });
    const second = controller.handleSelection({
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      selected_component_ids: ["comp-2"],
      active_component_id: "comp-2",
    });
    const third = controller.handleSelection({
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      selected_component_ids: ["comp-3"],
      active_component_id: "comp-3",
    });

    const results = await Promise.all([first, second, third]);

    assert.deepStrictEqual(analyzeTargets, ["comp-3"]);
    assert.strictEqual(results[0].diagnostics?.[0]?.code, "ANALYSIS_STALE");
    assert.strictEqual(results[1].diagnostics?.[0]?.code, "ANALYSIS_STALE");
    assert.strictEqual(results[2].status, "ready");
    assert.deepStrictEqual(results[2].target, "comp-3");

    assert.strictEqual(graphRenderer.renderCalls.length, 1);
    assert.strictEqual(graphRenderer.renderCalls[0].result.target, "comp-3");
    assert.strictEqual(host.getEvents("analysis_stale_discarded").length, 2);
  });

  it("stale response is discarded and does not overwrite latest graph", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const graphRenderer = createFakeGraphRenderer();
    const provider = createFakeProvider({
      fixtures: new Map([
        [
          "pages/demo.spg",
          {
            raw_text: JSON.stringify({ components: [] }),
            content_type: "super_page",
          },
        ],
      ]),
    });
    const runtimeClient = createFakeRuntimeClient();
    runtimeClient.analyzeSuperpageSelection = function (selection) {
      this.callLog.push({ method: "analyzeSuperpageSelection", args: Array.from(arguments) });
      const delay = selection.active_component_id === "slow" ? 30 : 0;
      return new Promise((resolve) => {
        setTimeout(() => {
          resolve({
            status: "ready",
            target: selection.active_component_id,
            items: [],
            diagnostics: [],
          });
        }, delay);
      });
    };

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      graphRenderer,
      host,
      logger: null,
    });

    const slow = controller.handleSelection({
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      selected_component_ids: ["slow"],
      active_component_id: "slow",
    });
    const fast = controller.handleSelection({
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      selected_component_ids: ["fast"],
      active_component_id: "fast",
    });

    await Promise.all([slow, fast]);

    const graphTargets = graphRenderer.renderCalls.map(
      (call) => call.result?.target ?? call.errorEnvelope?.target,
    );
    assert.deepStrictEqual(graphTargets, ["fast"]);
    assert.strictEqual(host.getEvents("analysis_stale_discarded").length, 1);
  });

  it("runtime-first failure falls back to page provider mode", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const provider = createFakeProvider({
      fixtures: new Map([
        [
          "pages/demo.spg",
          {
            raw_text: JSON.stringify({ components: [] }),
            content_type: "super_page",
          },
        ],
      ]),
    });
    const runtimeClient = createFakeRuntimeClient({
      supportsRemoteLoad: true,
      shouldFail: true,
      failMethod: "loadRemoteSuperpageDocument",
    });

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      host,
      logger: null,
    });

    const result = await controller.handleSelection({
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      selected_component_ids: ["comp-1"],
      active_component_id: "comp-1",
    });

    assert.strictEqual(result.status, "ready");
    assert.ok(
      runtimeClient.callLog.some(
        (c) => c.method === "loadRemoteSuperpageDocument",
      ),
    );
    assert.ok(provider.callLog.some((c) => c.method === "getFileContent"));
    assert.strictEqual(host.getEvents("controller_remote_load_fallback").length, 1);
  });

  it("page-provider mode keeps existing raw text load path", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const provider = createFakeProvider({
      fixtures: new Map([
        [
          "pages/demo.spg",
          {
            raw_text: JSON.stringify({ components: [] }),
            content_type: "super_page",
          },
        ],
      ]),
    });
    const runtimeClient = createFakeRuntimeClient({ supportsRemoteLoad: true });

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      host,
      logger: null,
      remoteLoadMode: "page-provider",
    });

    await controller.handleSelection({
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      selected_component_ids: ["comp-1"],
      active_component_id: "comp-1",
    });

    assert.strictEqual(
      runtimeClient.callLog.some(
        (c) => c.method === "loadRemoteSuperpageDocument",
      ),
      false,
    );
    assert.ok(provider.callLog.some((c) => c.method === "getFileContent"));
    assert.ok(
      runtimeClient.callLog.some((c) => c.method === "loadSuperpageDocument"),
    );
  });

  it("rejects nested raw metadata or component json in selection payload", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const provider = createFakeProvider({
      fixtures: new Map([
        [
          "pages/demo.spg",
          {
            raw_text: JSON.stringify({ components: [] }),
            content_type: "super_page",
          },
        ],
      ]),
    });
    const runtimeClient = createFakeRuntimeClient();

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      host,
      logger: null,
    });

    const result = await controller.handleSelection({
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      selected_component_ids: ["comp-1"],
      active_component_id: "comp-1",
      active_component: {
        id: "comp-1",
        components: [{ id: "nested-1" }],
      },
    });

    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "INVALID_SELECTION");
    assert.match(result.diagnostics[0].message, /active_component\.components/);
    assert.strictEqual(
      provider.callLog.some((c) => c.method === "getFileContent"),
      false,
    );
    assert.strictEqual(
      runtimeClient.callLog.some((c) => c.method === "analyzeSuperpageSelection"),
      false,
    );
    assert.strictEqual(renderer.renderCalls[0].type, "renderError");
  });
});

describe("integration error paths", async () => {
  const createController = await loadController();

  it("metadata fetch failure -> renderer diagnostic", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const graphRenderer = createFakeGraphRenderer();
    const provider = createFakeProvider({ shouldFailContent: true });
    const runtimeClient = createFakeRuntimeClient();

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      graphRenderer,
      host,
      logger: null,
    });

    const selection = {
      source_path: "pages/missing.spg",
      file_id: "missing-123",
      selected_component_ids: ["comp-1"],
      active_component_id: "comp-1",
    };

    const result = await controller.handleSelection(selection);

    assert.strictEqual(result.status, "error");
    assert.strictEqual(renderer.renderCalls.length, 1);
    assert.strictEqual(renderer.renderCalls[0].type, "renderError");
    assert.strictEqual(graphRenderer.renderCalls.length, 1);
    assert.strictEqual(graphRenderer.renderCalls[0].type, "renderGraph");
    assert.strictEqual(graphRenderer.renderCalls[0].result.status, "error");
    assert.ok(
      renderer.renderCalls[0].errorEnvelope.diagnostics.some(
        (d) => d.code === "ANALYSIS_PIPELINE_FAILED",
      ),
    );
  });

  it("metadata provider diagnostic message is preserved in pipeline error", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const provider = {
      async getFileContent() {
        return {
          status: "error",
          target: null,
          items: [],
          diagnostics: [
            {
              severity: "error",
              code: "REMOTE_FETCH_FAILED",
              message: "rc getFileContent failed",
            },
          ],
        };
      },
    };
    const runtimeClient = createFakeRuntimeClient();

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      host,
      logger: null,
    });

    const result = await controller.handleSelection({
      source_path: "pages/missing.spg",
      file_id: "missing-123",
      selected_component_ids: ["comp-1"],
      active_component_id: "comp-1",
    });

    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "ANALYSIS_PIPELINE_FAILED");
    assert.strictEqual(result.diagnostics[0].message, "rc getFileContent failed");
    assert.strictEqual(renderer.renderCalls[0].type, "renderError");
  });

  it("runtime analyze failure -> renderer error", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const provider = createFakeProvider({
      fixtures: new Map([
        [
          "pages/demo.spg",
          {
            raw_text: JSON.stringify({ components: [] }),
            content_type: "super_page",
          },
        ],
      ]),
    });
    const runtimeClient = createFakeRuntimeClient({
      shouldFail: true,
      failMethod: "analyzeSuperpageSelection",
    });

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      host,
      logger: null,
    });

    const selection = {
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      selected_component_ids: ["comp-1"],
      active_component_id: "comp-1",
    };

    const result = await controller.handleSelection(selection);

    assert.strictEqual(result.status, "error");
    assert.strictEqual(renderer.renderCalls.length, 1);
    assert.strictEqual(renderer.renderCalls[0].type, "renderError");
  });

  it("graph expand request without runtime support emits diagnostic graph", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const graphRenderer = createFakeGraphRenderer();
    const provider = createFakeProvider({
      fixtures: new Map([
        [
          "pages/demo.spg",
          {
            raw_text: JSON.stringify({ components: [] }),
            content_type: "super_page",
          },
        ],
      ]),
    });
    const runtimeClient = createFakeRuntimeClient();

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      graphRenderer,
      host,
      logger: null,
    });

    const result = await graphRenderer.emit("expand_requested", {
      nodeId: "comp-2",
      depth: 3,
    });

    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "GRAPH_EXPAND_UNSUPPORTED");
    assert.strictEqual(host.getEvents("graph_expand_requested").length, 1);
    assert.strictEqual(host.getEvents("graph_expand_failed").length, 1);
    const lastGraphCall = graphRenderer.renderCalls.at(-1);
    assert.strictEqual(lastGraphCall.type, "renderGraph");
    assert.strictEqual(lastGraphCall.result.status, "error");
  });
});

describe("duplicate init and idempotency", async () => {
  const createController = await loadController();

  it("duplicate init does not reinitialize plugin or runtime", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const provider = createFakeProvider({
      fixtures: new Map([
        [
          "pages/demo.spg",
          {
            raw_text: JSON.stringify({ components: [] }),
            content_type: "super_page",
          },
        ],
      ]),
    });
    const runtimeClient = createFakeRuntimeClient();

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      host,
      logger: null,
    });

    // 第一次 init
    const r1 = await controller.init();
    assert.strictEqual(r1.ready, true);
    const initCalls1 = runtimeClient.callLog.filter(
      (c) => c.method === "initRuntime",
    ).length;

    // 第二次 init
    const r2 = await controller.init();
    assert.strictEqual(r2.ready, true);
    const initCalls2 = runtimeClient.callLog.filter(
      (c) => c.method === "initRuntime",
    ).length;

    // initRuntime 只被调用一次
    assert.strictEqual(initCalls1, initCalls2);
    assert.strictEqual(initCalls1, 1);
  });
});

describe("controller source constraints", async () => {
  it("does not reference DOM API or BI-specific platform entry points", () => {
    const forbidden = [
      "document",
      "querySelector",
      "createElement",
      ".body",
      "window",
      "SZ",
      "onInitDesigner",
      "AMD",
      "require(",
      "navigator.",
      "service-worker-runtime-launcher",
    ];
    for (const token of forbidden) {
      assert.ok(
        !controllerSource.includes(token),
        `controller source should not reference ${token}`,
      );
    }
  });

  it("emits controller state transition events through host.emit", async () => {
    const createController = await loadController();
    const host = createFakeHost();
    const renderer = createFakeRenderer();
    const provider = createFakeProvider({
      fixtures: new Map([
        [
          "pages/demo.spg",
          {
            raw_text: JSON.stringify({ components: [] }),
            content_type: "super_page",
          },
        ],
      ]),
    });
    const runtimeClient = createFakeRuntimeClient({ kind: "service-worker" });

    const { createMetadataCheckerPlugin } =
      await import("../plugin-core/metadata-checker-plugin.mjs");
    const plugin = createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: null,
    });

    const controller = createController({
      plugin,
      provider,
      runtimeClient,
      renderer,
      host,
      logger: null,
    });

    const selection = {
      source_path: "pages/demo.spg",
      file_id: "demo-123",
      selected_component_ids: ["comp-1"],
      active_component_id: "comp-1",
    };

    await controller.handleSelection(selection);

    const stateEvents = host.getEvents("controller_state_changed");
    assert.ok(stateEvents.length >= 2);
    assert.deepStrictEqual(
      stateEvents.map((entry) => entry.payload.current),
      ["initializing", "ready", "analyzing", "ready"],
    );
    assert.deepStrictEqual(
      stateEvents.map((entry) => entry.payload.previous),
      ["idle", "initializing", "ready", "analyzing"],
    );

    assert.strictEqual(host.getEvents("controller_runtime_type").length, 1);
    const runtimeTypeEvent = host.getEvents("controller_runtime_type")[0];
    assert.strictEqual(runtimeTypeEvent.payload.runtimeType, "service-worker");
  });

  it("does not manipulate DOM via marker-like APIs", () => {
    const forbidden = [
      "innerHTML",
      "outerHTML",
      "document.write",
      "document.writeln",
    ];
    for (const token of forbidden) {
      assert.ok(
        !controllerSource.includes(token),
        `controller source should not use ${token}`,
      );
    }
  });

  it("does not put raw_text references in selection handling", () => {
    assert.ok(
      !controllerSource.includes("selection.raw_text"),
      "controller should not use a top-level-only raw_text guard",
    );
    assert.ok(
      controllerSource.includes("_findForbiddenSelectionPayload"),
      "controller should recursively guard forbidden selection payload fields",
    );
  });
});

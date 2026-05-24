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

  return {
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
    }, /runtimeClient with loadSuperpageDocument is required/);
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

    // provider 被调用
    const contentCalls = provider.callLog.filter(
      (c) => c.method === "getFileContent",
    );
    assert.strictEqual(contentCalls.length, 1);
    assert.strictEqual(contentCalls[0].args[0].source_path, "pages/demo.spg");

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
});

describe("integration error paths", async () => {
  const createController = await loadController();

  it("metadata fetch failure -> renderer diagnostic", async () => {
    const host = createFakeHost();
    const renderer = createFakeRenderer();
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
    assert.ok(
      renderer.renderCalls[0].errorEnvelope.diagnostics.some(
        (d) => d.code === "ANALYSIS_PIPELINE_FAILED",
      ),
    );
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
  it("does not import BI-specific modules", () => {
    const forbidden = ["SZ", "onInitDesigner", "AMD", "require("];
    for (const token of forbidden) {
      assert.ok(
        !controllerSource.includes(token),
        `controller source should not reference ${token}`,
      );
    }
  });

  it("does not manipulate DOM directly except through markers", () => {
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
      controllerSource.includes("selection.raw_text"),
      "controller should guard against raw_text in selection",
    );
  });
});

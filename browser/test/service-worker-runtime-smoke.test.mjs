/**
 * M40.9 Service Worker Runtime Smoke Tests
 *
 * 覆盖 Service Worker 注册、ready、message、fallback、统一 runtimeClient contract。
 * 先不接真实 wasm 业务，只保证协议层面正确。
 */

import { describe, it } from "node:test";
import assert from "node:assert";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const swSourcePath = join(
  __dirname,
  "../service-worker/metadata-checker-sw.js",
);
const launcherSourcePath = join(
  __dirname,
  "../runtime-launchers/service-worker-runtime-launcher.mjs",
);

// 模拟 Service Worker 全局环境
function createMockSwGlobal(options = {}) {
  const listeners = new Map();
  let clientsMock = { claim: async () => {} };
  const self = {
    skipWaiting: () => {},
    clients: clientsMock,
    location: { href: "https://example.test/browser/service-worker/metadata-checker-sw.js" },
    importScripts(...urls) {
      if (typeof options.importScripts === "function") {
        const result = options.importScripts(...urls);
        if (typeof options.wasmBindgenFactory === "function") {
          self.wasm_bindgen = options.wasmBindgenFactory;
        }
        return result;
      }
      if (options.importScriptsError) {
        throw options.importScriptsError;
      }
      if (typeof options.wasmBindgenFactory === "function") {
        self.wasm_bindgen = options.wasmBindgenFactory;
      }
      return undefined;
    },
    addEventListener: (type, handler) => {
      listeners.set(type, handler);
    },
  };
  return { self, listeners, clientsMock };
}

// 从 SW 脚本中提取可测试函数（通过 eval 在 mock 环境中执行）
async function loadSwInMockEnvironment(options = {}) {
  const { readFileSync } = await import("node:fs");
  const swSource = readFileSync(swSourcePath, "utf-8");
  const { self, listeners } = createMockSwGlobal(options);

  // 在 mock self 环境中 eval SW 脚本
  const lexicalPrelude = options.lexicalWasmBindgenFactory
    ? "let wasm_bindgen = __lexicalWasmBindgenFactory;\n"
    : "";
  const wrapped = `(function(self, module, __lexicalWasmBindgenFactory) {\n${lexicalPrelude}${swSource}\n})`;
  const fn = eval(wrapped);
  const mod = { exports: {} };
  fn(self, mod, options.lexicalWasmBindgenFactory);

  return { self, listeners, exports: mod.exports };
}

function makeWasmHarness(options = {}) {
  const calls = {
    importScripts: 0,
    wasmBindgen: 0,
  };
  const exportCalls = {};
  const providedExports = options.exports ?? {};
  const wasmExports = {
    initRuntime: () => ({ status: "ready", target: null, items: [], diagnostics: [] }),
    runtimeStatus: () => ({ status: "ready", target: null, items: [], diagnostics: [] }),
    loadSuperpageDocument: () => ({ status: "ready", target: null, items: [], diagnostics: [] }),
    buildOrUpdateSuperpageGraph: () => ({ status: "ready", target: null, items: [], diagnostics: [] }),
    analyzeSuperpageSelection: () => ({ status: "ready", target: null, items: [], diagnostics: [] }),
    fetchRemoteFileInfo: () => ({}),
    fetchRemoteFileContent: () => "",
    loadRemoteSuperpageDocument: () => ({}),
    ...providedExports,
  };

  Object.keys(wasmExports).forEach((name) => {
    const original = wasmExports[name];
    wasmExports[name] = (...args) => {
      exportCalls[name] = (exportCalls[name] ?? 0) + 1;
      return original(...args);
    };
  });

  async function wasmBindgenFactory(wasmUrl) {
    calls.wasmBindgen += 1;
    if (options.wasmBindgenErrorOnce && calls.wasmBindgen === 1) {
      throw options.wasmBindgenErrorOnce;
    }
    if (options.wasmBindgenError) {
      throw options.wasmBindgenError;
    }
    Object.assign(wasmBindgenFactory, wasmExports);
    wasmBindgenFactory.__wasmUrl = wasmUrl;
    return wasmBindgenFactory;
  }

  return {
    calls,
    exportCalls,
    importScripts(...urls) {
      calls.importScripts += 1;
      if (options.importScriptsError) {
        throw options.importScriptsError;
      }
      return urls;
    },
    wasmBindgenFactory,
  };
}

describe("Service Worker script internal protocol", () => {
  it("handleRequest returns response with id for initRuntime", async () => {
    const wasm = makeWasmHarness();
    const { exports } = await loadSwInMockEnvironment(wasm);
    const response = await exports.handleRequest({
      id: "req-1",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });
    assert.strictEqual(response.id, "req-1");
    assert.strictEqual(response.ok, true);
    assert.strictEqual(response.result.status, "ready");
    assert.strictEqual(response.result.items[0].detail.wasm.mode, "wasm-bindgen");
    assert.strictEqual(wasm.calls.importScripts, 1);
    assert.strictEqual(wasm.calls.wasmBindgen, 1);
    assert.strictEqual(wasm.exportCalls.initRuntime, 1);

    const status = await exports.handleRequest({
      id: "req-1-status",
      method: "runtimeStatus",
      args: [],
    });
    assert.strictEqual(status.result.items[0].detail.wasm.state, "loaded");
  });

  it("loads wasm-bindgen glue exposed as a global lexical binding", async () => {
    const wasm = makeWasmHarness();
    const { exports, self } = await loadSwInMockEnvironment({
      importScripts: wasm.importScripts,
      lexicalWasmBindgenFactory: wasm.wasmBindgenFactory,
    });

    const response = await exports.handleRequest({
      id: "lexical-runtime",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });

    assert.strictEqual(response.ok, true);
    assert.strictEqual(response.result.status, "ready");
    assert.strictEqual(wasm.calls.importScripts, 0);
    assert.strictEqual(wasm.calls.wasmBindgen, 1);
    assert.strictEqual(self.wasm_bindgen, wasm.wasmBindgenFactory);
  });

  it("handleRequest returns error for unknown method", async () => {
    const { exports } = await loadSwInMockEnvironment();
    const response = await exports.handleRequest({
      id: "req-2",
      method: "unknownMethod",
      args: [],
    });
    assert.strictEqual(response.id, "req-2");
    assert.strictEqual(response.ok, false);
    assert.strictEqual(response.error.code, "UNKNOWN_METHOD");
  });

  it("fetchRemoteFileContent calls wasm export and returns content", async () => {
    const wasm = makeWasmHarness({
      exports: {
        fetchRemoteFileContent: (fileRef) => {
          assert.deepStrictEqual(fileRef, { file_id: "fid-100" });
          return '{"components":[{"id":"c-1"}]}';
        },
      },
    });
    const { exports } = await loadSwInMockEnvironment(wasm);
    await exports.handleRequest({
      id: "remote-init",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });
    const response = await exports.handleRequest({
      id: "remote-content",
      method: "fetchRemoteFileContent",
      args: [{ file_id: "fid-100" }],
    });
    assert.strictEqual(response.ok, true);
    assert.strictEqual(response.result, '{"components":[{"id":"c-1"}]}');
  });

  it("fetchRemoteFileInfo calls wasm export and returns file info", async () => {
    const wasm = makeWasmHarness({
      exports: {
        fetchRemoteFileInfo: (fileRef) => {
          assert.deepStrictEqual(fileRef, { file_id: "fid-info" });
          return { file_id: "fid-info", source_path: "pages/info.spg", revision: "9" };
        },
      },
    });
    const { exports } = await loadSwInMockEnvironment(wasm);
    await exports.handleRequest({
      id: "remote-info-init",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });
    const response = await exports.handleRequest({
      id: "remote-info",
      method: "fetchRemoteFileInfo",
      args: [{ file_id: "fid-info" }],
    });
    assert.strictEqual(response.ok, true);
    assert.deepStrictEqual(response.result, {
      file_id: "fid-info",
      source_path: "pages/info.spg",
      revision: "9",
    });
  });

  it("loadRemoteSuperpageDocument caches document so build/analyze can run", async () => {
    const wasm = makeWasmHarness({
      exports: {
        loadRemoteSuperpageDocument: (sourceOrRef, rawText) => {
          assert.strictEqual(sourceOrRef, "pages/remote.spg");
          assert.strictEqual(rawText, '{"components":[{"id":"c-1"}]}');
          return {
            source_path: "pages/remote.spg",
            raw_text: rawText,
            status: "ready",
            diagnostics: [],
          };
        },
      },
    });
    const { exports } = await loadSwInMockEnvironment(wasm);
    await exports.handleRequest({
      id: "remote-load-runtime",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });
    const loaded = await exports.handleRequest({
      id: "remote-load-doc",
      method: "loadRemoteSuperpageDocument",
      args: ["pages/remote.spg", '{"components":[{"id":"c-1"}]}'],
    });
    const built = await exports.handleRequest({
      id: "remote-build",
      method: "buildOrUpdateSuperpageGraph",
      args: ["pages/remote.spg"],
    });
    const analyzed = await exports.handleRequest({
      id: "remote-analyze",
      method: "analyzeSuperpageSelection",
      args: [
        {
          source_path: "pages/remote.spg",
          file_id: "fid-100",
          selected_component_ids: ["c-1"],
          active_component_id: "c-1",
        },
      ],
    });

    assert.strictEqual(loaded.ok, true);
    assert.strictEqual(built.ok, true);
    assert.strictEqual(analyzed.ok, true);
  });

  it("load/build/analyze calls real WASM exports when available and preserves visual_graph", async () => {
    const wasm = makeWasmHarness({
      exports: {
        loadSuperpageDocument: (sourcePath, rawText) => {
          assert.strictEqual(sourcePath, "pages/visual.spg");
          assert.strictEqual(rawText, '{"components":[{"id":"c-1"}]}');
          return JSON.stringify({
            status: "ready",
            target: sourcePath,
            items: [{ kind: "document_loaded", label: "Document Loaded", detail: { source_path: sourcePath } }],
            diagnostics: [],
          });
        },
        buildOrUpdateSuperpageGraph: (sourcePath) => {
          assert.strictEqual(sourcePath, "pages/visual.spg");
          return {
            status: "ready",
            target: sourcePath,
            items: [{ kind: "graph_built", label: "Graph Built", detail: { source_path: sourcePath } }],
            diagnostics: [],
          };
        },
        analyzeSuperpageSelection: (selectionJson, optionsJson) => {
          const selection = JSON.parse(selectionJson);
          const options = JSON.parse(optionsJson);
          assert.strictEqual(selection.active_component_id, "c-1");
          assert.deepStrictEqual(options, { include_conditions: true });
          return JSON.stringify({
            status: "ready",
            target: "c-1",
            items: [
              {
                kind: "visual_graph",
                label: "Visual Graph",
                detail: {
                  nodes: [
                    { id: "c-1", label: "c-1", kind: "Component", metadata: { depth: 0 } },
                    { id: "m-1", label: "m-1", kind: "Model", metadata: { depth: 1 } },
                  ],
                  edges: [{ from: "c-1", to: "m-1", kind: "Reads", direction: "Forward" }],
                  groups: [],
                  focus_node: "c-1",
                  diagnostics: [],
                  truncated: false,
                  source_summary: { total_nodes: 2, total_edges: 1, node_kinds: {}, edge_kinds: {} },
                },
              },
            ],
            diagnostics: [],
          });
        },
      },
    });
    const { exports } = await loadSwInMockEnvironment(wasm);
    await exports.handleRequest({
      id: "visual-init",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });
    const loaded = await exports.handleRequest({
      id: "visual-load",
      method: "loadSuperpageDocument",
      args: ["pages/visual.spg", '{"components":[{"id":"c-1"}]}'],
    });
    const built = await exports.handleRequest({
      id: "visual-build",
      method: "buildOrUpdateSuperpageGraph",
      args: ["pages/visual.spg"],
    });
    const analyzed = await exports.handleRequest({
      id: "visual-analyze",
      method: "analyzeSuperpageSelection",
      args: [
        {
          source_path: "pages/visual.spg",
          file_id: "fid-visual",
          selected_component_ids: ["c-1"],
          active_component_id: "c-1",
        },
        { include_conditions: true },
      ],
    });

    assert.strictEqual(loaded.ok, true);
    assert.strictEqual(built.ok, true);
    assert.strictEqual(analyzed.ok, true);
    assert.strictEqual(wasm.exportCalls.loadSuperpageDocument, 1);
    assert.strictEqual(wasm.exportCalls.buildOrUpdateSuperpageGraph, 1);
    assert.strictEqual(wasm.exportCalls.analyzeSuperpageSelection, 1);
    assert.strictEqual(analyzed.result.items[0].detail.nodes.length, 2);
  });

  it("fetchRemoteFileContent returns stable glue load error when runtime init fails", async () => {
    const wasm = makeWasmHarness({
      importScriptsError: new Error("glue blocked"),
    });
    const { exports } = await loadSwInMockEnvironment(wasm);

    const response = await exports.handleRequest({
      id: "remote-fetch-failed",
      method: "fetchRemoteFileContent",
      args: [{ file_id: "fid-100" }],
    });

    assert.strictEqual(response.ok, false);
    assert.strictEqual(response.error.code, "WASM_BINDGEN_GLUE_LOAD_FAILED");
    assert.match(response.error.diagnostic.code, /WASM_BINDGEN_GLUE_LOAD_FAILED/);
  });

  it("fetchRemoteFileContent supports sync or Promise export", async () => {
    const syncWasm = makeWasmHarness({
      exports: {
        fetchRemoteFileContent: () => "sync-content",
      },
    });
    const { exports } = await loadSwInMockEnvironment(syncWasm);
    await exports.handleRequest({
      id: "sync-runtime",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });
    const syncResult = await exports.handleRequest({
      id: "sync-content",
      method: "fetchRemoteFileContent",
      args: [{ file_id: "fid-sync" }],
    });
    assert.strictEqual(syncResult.ok, true);
    assert.strictEqual(syncResult.result, "sync-content");

    const promiseWasm = makeWasmHarness({
      exports: {
        fetchRemoteFileContent: () => Promise.resolve("promise-content"),
      },
    });
    const { exports: promiseExports } = await loadSwInMockEnvironment(promiseWasm);
    await promiseExports.handleRequest({
      id: "promise-runtime",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });
    const promiseResult = await promiseExports.handleRequest({
      id: "promise-content",
      method: "fetchRemoteFileContent",
      args: [{ file_id: "fid-promise" }],
    });
    assert.strictEqual(promiseResult.ok, true);
    assert.strictEqual(promiseResult.result, "promise-content");
  });

  it("remote fetch/init duplicate request still initializes once", async () => {
    const wasm = makeWasmHarness({
      exports: {
        fetchRemoteFileContent: () => "c",
        loadRemoteSuperpageDocument: () => ({ raw_text: "{}", source_path: "pages/dup.spg" }),
      },
    });
    const { exports } = await loadSwInMockEnvironment(wasm);

    const p1 = exports.handleRequest({
      id: "dup-remote-1",
      method: "fetchRemoteFileContent",
      args: [{ file_id: "fid-1" }],
    });
    const p2 = exports.handleRequest({
      id: "dup-remote-2",
      method: "loadRemoteSuperpageDocument",
      args: ["pages/dup.spg", "{}"],
    });
    const [r1, r2] = await Promise.all([p1, p2]);

    assert.strictEqual(r1.ok, true);
    assert.strictEqual(r2.ok, true);
    assert.strictEqual(wasm.calls.importScripts, 1);
    assert.strictEqual(wasm.calls.wasmBindgen, 1);
  });

  it("handleRequest returns error for missing id", async () => {
    const { exports } = await loadSwInMockEnvironment();
    const response = await exports.handleRequest({
      method: "initRuntime",
      args: [],
    });
    assert.strictEqual(response, null);
  });

  it("handleRequest rejects loadSuperpageDocument without prior document", async () => {
    const wasm = makeWasmHarness({
      exports: {
        buildOrUpdateSuperpageGraph: () => {
          throw new Error("document not loaded, call loadSuperpageDocument first");
        },
      },
    });
    const { exports } = await loadSwInMockEnvironment(wasm);
    await exports.handleRequest({
      id: "req-3-init",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });
    const response = await exports.handleRequest({
      id: "req-3",
      method: "buildOrUpdateSuperpageGraph",
      args: ["pages/test.spg"],
    });
    assert.strictEqual(response.id, "req-3");
    assert.strictEqual(response.ok, false);
    assert.strictEqual(response.error.code, "WASM_EXPORT_ERROR");
    assert.ok(response.error.message.includes("document not loaded"));
  });

  it("loadSuperpageDocument accepts large raw text from provider-runtime channel", async () => {
    const wasm = makeWasmHarness();
    const { exports } = await loadSwInMockEnvironment(wasm);
    const largeRawText = "x".repeat(6 * 1024 * 1024);

    await exports.handleRequest({
      id: "large-r0",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });
    const loaded = await exports.handleRequest({
      id: "large-r1",
      method: "loadSuperpageDocument",
      args: ["pages/large.spg", largeRawText],
    });
    const built = await exports.handleRequest({
      id: "large-r2",
      method: "buildOrUpdateSuperpageGraph",
      args: ["pages/large.spg"],
    });
    const analyzed = await exports.handleRequest({
      id: "large-r3",
      method: "analyzeSuperpageSelection",
      args: [
        {
          source_path: "pages/large.spg",
          file_id: "large-1",
          selected_component_ids: ["c1"],
          active_component_id: "c1",
        },
      ],
    });

    assert.strictEqual(loaded.ok, true);
    assert.strictEqual(built.ok, true);
    assert.strictEqual(analyzed.ok, true);
  });

  it("lazy init reuses promise for duplicate initRuntime calls", async () => {
    const wasm = makeWasmHarness();
    const { exports } = await loadSwInMockEnvironment(wasm);
    const p1 = exports.handleRequest({
      id: "dup-1",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });
    const p2 = exports.handleRequest({
      id: "dup-2",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });
    const [r1, r2] = await Promise.all([p1, p2]);
    assert.strictEqual(r1.ok, true);
    assert.strictEqual(r2.ok, true);
    assert.strictEqual(wasm.calls.importScripts, 1);
    assert.strictEqual(wasm.calls.wasmBindgen, 1);
    assert.strictEqual(wasm.exportCalls.initRuntime, 1);
  });

  it("analyzeSuperpageSelection rejects if selection contains raw metadata payload", async () => {
    const wasm = makeWasmHarness();
    const { exports } = await loadSwInMockEnvironment(wasm);
    // 先 init + load + build
    await exports.handleRequest({
      id: "r0",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });
    await exports.handleRequest({
      id: "r1",
      method: "loadSuperpageDocument",
      args: ["pages/a.spg", "{}"],
    });
    await exports.handleRequest({
      id: "r2",
      method: "buildOrUpdateSuperpageGraph",
      args: ["pages/a.spg"],
    });

    const response = await exports.handleRequest({
      id: "r3",
      method: "analyzeSuperpageSelection",
      args: [
        {
          source_path: "pages/a.spg",
          file_id: "f1",
          selected_component_ids: ["c1"],
          active_component_id: "c1",
          raw_text: "should not be here",
        },
      ],
    });
    assert.strictEqual(response.ok, false);
    assert.strictEqual(response.error.code, "INVALID_SELECTION_PAYLOAD");
    assert.ok(response.error.message.includes("raw_text"));

    const nested = await exports.handleRequest({
      id: "r4",
      method: "analyzeSuperpageSelection",
      args: [
        {
          source_path: "pages/a.spg",
          file_id: "f1",
          selected_component_ids: ["c1"],
          active_component_id: "c1",
          active_component: {
            id: "c1",
            components: [{ id: "nested-1" }],
          },
        },
      ],
    });
    assert.strictEqual(nested.ok, false);
    assert.strictEqual(nested.error.code, "INVALID_SELECTION_PAYLOAD");
    assert.ok(nested.error.message.includes("active_component.components"));
  });

  it("initRuntime returns stable diagnostic when wasm-bindgen glue load fails", async () => {
    const wasm = makeWasmHarness({
      importScriptsError: new Error("network blocked"),
    });
    const { exports } = await loadSwInMockEnvironment(wasm);

    const response = await exports.handleRequest({
      id: "fetch-failure",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });

    assert.strictEqual(response.ok, false);
    assert.strictEqual(response.error.code, "WASM_BINDGEN_GLUE_LOAD_FAILED");
    assert.strictEqual(response.error.diagnostic.code, "WASM_BINDGEN_GLUE_LOAD_FAILED");
    assert.match(response.error.diagnostic.detail.cause_message, /network blocked/);
  });

  it("initRuntime returns stable diagnostic when wasm-bindgen init fails", async () => {
    const wasm = makeWasmHarness({
      wasmBindgenError: new Error("wasm compile blocked"),
    });
    const { exports } = await loadSwInMockEnvironment(wasm);

    const response = await exports.handleRequest({
      id: "bindgen-init-failed",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });

    assert.strictEqual(response.ok, false);
    assert.strictEqual(response.error.code, "WASM_BINDGEN_INIT_FAILED");
    assert.strictEqual(response.error.diagnostic.code, "WASM_BINDGEN_INIT_FAILED");
    assert.match(response.error.diagnostic.detail.cause_message, /wasm compile blocked/);

    const status = await exports.handleRequest({
      id: "bindgen-status",
      method: "runtimeStatus",
      args: [],
    });
    assert.strictEqual(status.result.items[0].detail.wasm.state, "failed");
  });

  it("initRuntime returns stable diagnostic when wasm-bindgen glue is unavailable", async () => {
    const { exports } = await loadSwInMockEnvironment({
      importScripts: undefined,
      wasmBindgenFactory: undefined,
    });

    const response = await exports.handleRequest({
      id: "glue-unavailable",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });

    assert.strictEqual(response.ok, false);
    assert.strictEqual(response.error.code, "WASM_BINDGEN_GLUE_INVALID");
  });

  it("initRuntime clears failed promise and allows retry", async () => {
    const wasm = makeWasmHarness({
      wasmBindgenErrorOnce: new Error("first wasm bindgen failure"),
    });
    const { exports } = await loadSwInMockEnvironment(wasm);

    const failed = await exports.handleRequest({
      id: "retry-1",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });
    assert.strictEqual(failed.ok, false);
    assert.strictEqual(failed.error.code, "WASM_BINDGEN_INIT_FAILED");

    const retried = await exports.handleRequest({
      id: "retry-2",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });
    assert.strictEqual(retried.ok, true);
    assert.strictEqual(retried.result.items[0].detail.wasm.state, "loaded");
  });
});

describe("Service Worker runtime launcher with mock transport", async () => {
  const { createServiceWorkerRuntimeLauncher } = await import(
    launcherSourcePath
  );

  function makeFakeTransportWithSwProtocol(options = {}) {
    const handlers = new Set();
    let shouldFailSend = options.shouldFailSend ?? false;
    let failNext = options.failNext ?? 0;

    return {
      sentRequests: [],
      send(request) {
        this.sentRequests.push(request);
        if (shouldFailSend) {
          return Promise.reject(new Error("transport send failed"));
        }
        if (failNext > 0) {
          failNext -= 1;
          return Promise.reject(new Error("transport send failed"));
        }
        // 模拟 SW 响应
        setTimeout(() => {
          const response = {
            id: request.id,
            ok: true,
            result: null,
            error: null,
          };
          if (request.method === "initRuntime") {
            response.result = { status: "ready", items: [], diagnostics: [] };
          } else if (request.method === "runtimeStatus") {
            response.result = { status: "ready", items: [], diagnostics: [] };
          } else if (request.method === "loadSuperpageDocument") {
            response.result = {
              status: "ready",
              target: request.args[0],
              items: [],
              diagnostics: [],
            };
          } else if (request.method === "buildOrUpdateSuperpageGraph") {
            response.result = {
              status: "ready",
              target: request.args[0],
              items: [],
              diagnostics: [],
            };
          } else if (request.method === "analyzeSuperpageSelection") {
            response.result = {
              status: "ready",
              target: request.args[0]?.active_component_id,
              items: [],
              diagnostics: [],
            };
          }
          for (const h of handlers) {
            h(response);
          }
        }, 0);
      },
      onMessage(handler) {
        handlers.add(handler);
      },
      offMessage(handler) {
        handlers.delete(handler);
      },
    };
  }

  it("start returns runtimeClient with required methods", async () => {
    const transport = makeFakeTransportWithSwProtocol();
    const launcher = createServiceWorkerRuntimeLauncher({ transport });
    const client = await launcher.start();
    assert.ok(typeof client.initRuntime === "function");
    assert.ok(typeof client.runtimeStatus === "function");
    assert.ok(typeof client.loadSuperpageDocument === "function");
    assert.ok(typeof client.loadRemoteSuperpageDocument === "function");
    assert.ok(typeof client.buildOrUpdateSuperpageGraph === "function");
    assert.ok(typeof client.analyzeSuperpageSelection === "function");
  });

  it("start failure falls back to page runtime and marks fallbackUsed", async () => {
    const fallbackClient = {
      initRuntime: () => Promise.resolve({ status: "ready" }),
      runtimeStatus: () => Promise.resolve({ status: "ready" }),
      loadSuperpageDocument: () => Promise.resolve({ status: "ready" }),
      loadRemoteSuperpageDocument: () => Promise.resolve({ status: "ready" }),
      buildOrUpdateSuperpageGraph: () => Promise.resolve({ status: "ready" }),
      analyzeSuperpageSelection: () => Promise.resolve({ status: "ready" }),
    };
    const launcher = createServiceWorkerRuntimeLauncher({
      transportFactory: () => {
        throw new Error("transport unavailable");
      },
      fallbackRuntimeClient: fallbackClient,
    });
    const client = await launcher.start();
    assert.ok(client);
    const status = launcher.status();
    assert.strictEqual(status.fallbackUsed, true);
  });

  it("status includes pendingRequestCount", async () => {
    const transport = makeFakeTransportWithSwProtocol();
    const launcher = createServiceWorkerRuntimeLauncher({ transport });
    await launcher.start();
    const status = launcher.status();
    assert.strictEqual(typeof status.pendingRequestCount, "number");
  });

  it("concurrent start should dedupe", async () => {
    const transport = makeFakeTransportWithSwProtocol();
    const launcher = createServiceWorkerRuntimeLauncher({ transport });
    const p1 = launcher.start();
    const p2 = launcher.start();
    const c1 = await p1;
    const c2 = await p2;
    assert.strictEqual(c1, c2);
  });

  it("stop clears client and allows restart", async () => {
    const transport = makeFakeTransportWithSwProtocol();
    const launcher = createServiceWorkerRuntimeLauncher({ transport });
    const client1 = await launcher.start();
    launcher.stop();
    const client2 = await launcher.start();
    assert.notStrictEqual(client1, client2);
  });
});

describe("Service Worker script source constraints", () => {
  it("SW entry is JS not wasm", async () => {
    const swFileName = swSourcePath.split(/[\\/]/).pop();
    assert.ok(swFileName.endsWith(".js"), "SW entry must be .js, not .wasm");
  });

  it("SW source does not access DOM", async () => {
    const { readFileSync } = await import("node:fs");
    const swSource = readFileSync(swSourcePath, "utf-8");
    const forbidden = [
      "document.",
      "window.",
      "getElementById",
      "querySelector",
    ];
    for (const token of forbidden) {
      assert.ok(
        !swSource.includes(token),
        `SW source should not access DOM: ${token}`,
      );
    }
  });
});

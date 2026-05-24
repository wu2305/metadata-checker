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
    fetch: options.fetch,
    WebAssembly: options.WebAssembly,
    location: { href: "https://example.test/browser/service-worker/metadata-checker-sw.js" },
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
  const wrapped = `(function(self, module) {\n${swSource}\n})`;
  const fn = eval(wrapped);
  const mod = { exports: {} };
  fn(self, mod);

  return { self, listeners, exports: mod.exports };
}

function makeWasmResponse({ contentType = "application/wasm", bytes = new Uint8Array([0, 97]) } = {}) {
  return {
    ok: true,
    status: 200,
    headers: {
      get(name) {
        return name.toLowerCase() === "content-type" ? contentType : null;
      },
    },
    async arrayBuffer() {
      return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
    },
    clone() {
      return makeWasmResponse({ contentType, bytes });
    },
  };
}

function makeWasmHarness(options = {}) {
  const calls = {
    fetch: 0,
    instantiateStreaming: 0,
    instantiate: 0,
  };
  const exportCalls = {};
  const responseFactory =
    options.responseFactory ??
    (() => makeWasmResponse({ contentType: options.contentType ?? "application/wasm" }));
  const providedExports = options.exports ?? {};
  const wasmExports = {
    analyze: () => {},
    ...providedExports,
  };

  Object.keys(wasmExports).forEach((name) => {
    const original = wasmExports[name];
    wasmExports[name] = (...args) => {
      exportCalls[name] = (exportCalls[name] ?? 0) + 1;
      return original(...args);
    };
  });

  return {
    calls,
    exportCalls,
    fetch: async () => {
      calls.fetch += 1;
      if (options.fetchError) {
        throw options.fetchError;
      }
      return responseFactory();
    },
    WebAssembly: {
      instantiateStreaming: async () => {
        calls.instantiateStreaming += 1;
        if (options.streamingError) {
          throw options.streamingError;
        }
        return { instance: { exports: wasmExports }, module: {} };
      },
      instantiate: async () => {
        calls.instantiate += 1;
        if (options.instantiateError) {
          throw options.instantiateError;
        }
        return { instance: { exports: wasmExports }, module: {} };
      },
    },
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
    assert.strictEqual(response.result.items[0].detail.wasm.mode, "streaming");
    assert.strictEqual(wasm.calls.instantiateStreaming, 1);

    const status = await exports.handleRequest({
      id: "req-1-status",
      method: "runtimeStatus",
      args: [],
    });
    assert.strictEqual(status.result.items[0].detail.wasm.state, "loaded");
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
        fetch_remote_file_content: (fileRef) => {
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
        fetch_remote_file_info: (fileRef) => {
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
        load_remote_superpage_document: (sourceOrRef, rawText) => {
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

  it("fetchRemoteFileContent returns stable WASM_FETCH_FAILED when runtime init fails", async () => {
    const wasm = makeWasmHarness({
      fetchError: new Error("network blocked"),
    });
    const { exports } = await loadSwInMockEnvironment(wasm);

    const response = await exports.handleRequest({
      id: "remote-fetch-failed",
      method: "fetchRemoteFileContent",
      args: [{ file_id: "fid-100" }],
    });

    assert.strictEqual(response.ok, false);
    assert.strictEqual(response.error.code, "WASM_FETCH_FAILED");
    assert.match(response.error.diagnostic.code, /WASM_FETCH_FAILED/);
  });

  it("fetchRemoteFileContent supports sync or Promise export", async () => {
    const syncWasm = makeWasmHarness({
      exports: {
        fetch_remote_file_content: () => "sync-content",
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
        fetch_remote_file_content: () => Promise.resolve("promise-content"),
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
        fetch_remote_file_content: () => "c",
        load_remote_superpage_document: () => ({ raw_text: "{}", source_path: "pages/dup.spg" }),
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
    assert.strictEqual(wasm.calls.fetch, 1);
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
    const { exports } = await loadSwInMockEnvironment();
    // 先 reset caches
    const response = await exports.handleRequest({
      id: "req-3",
      method: "buildOrUpdateSuperpageGraph",
      args: ["pages/test.spg"],
    });
    assert.strictEqual(response.id, "req-3");
    assert.strictEqual(response.ok, false);
    assert.strictEqual(response.error.code, "RUNTIME_EXECUTION_ERROR");
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
    assert.strictEqual(wasm.calls.fetch, 1);
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

  it("initRuntime returns stable diagnostic when WASM fetch fails", async () => {
    const wasm = makeWasmHarness({
      fetchError: new Error("network blocked"),
    });
    const { exports } = await loadSwInMockEnvironment(wasm);

    const response = await exports.handleRequest({
      id: "fetch-failure",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });

    assert.strictEqual(response.ok, false);
    assert.strictEqual(response.error.code, "WASM_FETCH_FAILED");
    assert.strictEqual(response.error.diagnostic.code, "WASM_FETCH_FAILED");
    assert.match(response.error.diagnostic.detail.cause_message, /network blocked/);
  });

  it("initRuntime falls back to arrayBuffer when MIME is not application/wasm", async () => {
    const wasm = makeWasmHarness({ contentType: "application/octet-stream" });
    const { exports } = await loadSwInMockEnvironment(wasm);

    const response = await exports.handleRequest({
      id: "mime-fallback",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });

    assert.strictEqual(response.ok, true);
    assert.strictEqual(wasm.calls.instantiateStreaming, 0);
    assert.strictEqual(wasm.calls.instantiate, 1);
    assert.strictEqual(response.result.items[0].detail.wasm.mode, "arrayBuffer");
    assert.strictEqual(response.result.diagnostics[0].code, "WASM_MIME_FALLBACK");

    const status = await exports.handleRequest({
      id: "mime-status",
      method: "runtimeStatus",
      args: [],
    });
    assert.strictEqual(status.result.items[0].detail.wasm.state, "fallback");
  });

  it("initRuntime falls back to arrayBuffer when instantiateStreaming throws", async () => {
    const wasm = makeWasmHarness({
      streamingError: new Error("streaming blocked"),
    });
    const { exports } = await loadSwInMockEnvironment(wasm);

    const response = await exports.handleRequest({
      id: "streaming-fallback",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });

    assert.strictEqual(response.ok, true);
    assert.strictEqual(wasm.calls.instantiateStreaming, 1);
    assert.strictEqual(wasm.calls.instantiate, 1);
    assert.strictEqual(response.result.items[0].detail.wasm.mode, "arrayBuffer");
    assert.strictEqual(response.result.diagnostics[0].code, "WASM_STREAMING_FALLBACK");
  });

  it("initRuntime returns stable diagnostic when compile or CSP blocks fallback", async () => {
    const wasm = makeWasmHarness({
      streamingError: new Error("CSP blocked streaming compile"),
      instantiateError: new Error("CSP blocked arrayBuffer compile"),
    });
    const { exports } = await loadSwInMockEnvironment(wasm);

    const response = await exports.handleRequest({
      id: "csp-failure",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });

    assert.strictEqual(response.ok, false);
    assert.strictEqual(response.error.code, "WASM_CSP_OR_COMPILE_FAILED");
    assert.strictEqual(response.error.diagnostic.code, "WASM_CSP_OR_COMPILE_FAILED");

    const status = await exports.handleRequest({
      id: "status-after-csp",
      method: "runtimeStatus",
      args: [],
    });
    assert.strictEqual(status.result.items[0].detail.wasm.state, "failed");
  });

  it("initRuntime clears failed promise and allows retry", async () => {
    const wasm = makeWasmHarness({
      streamingError: new Error("first streaming failure"),
      instantiateError: new Error("first compile failure"),
    });
    const { exports } = await loadSwInMockEnvironment(wasm);

    const failed = await exports.handleRequest({
      id: "retry-1",
      method: "initRuntime",
      args: [{ wasmUrl: "https://example.test/runtime.wasm" }],
    });
    assert.strictEqual(failed.ok, false);
    assert.strictEqual(failed.error.code, "WASM_CSP_OR_COMPILE_FAILED");

    wasm.WebAssembly.instantiateStreaming = async () => {
      wasm.calls.instantiateStreaming += 1;
      return { instance: { exports: { analyze: () => {} } }, module: {} };
    };
    wasm.WebAssembly.instantiate = async () => {
      wasm.calls.instantiate += 1;
      return { instance: { exports: { analyze: () => {} } }, module: {} };
    };

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

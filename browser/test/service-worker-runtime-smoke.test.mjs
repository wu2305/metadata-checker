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
function createMockSwGlobal() {
  const listeners = new Map();
  let clientsMock = { claim: async () => {} };
  const self = {
    skipWaiting: () => {},
    clients: clientsMock,
    addEventListener: (type, handler) => {
      listeners.set(type, handler);
    },
  };
  return { self, listeners, clientsMock };
}

// 从 SW 脚本中提取可测试函数（通过 eval 在 mock 环境中执行）
async function loadSwInMockEnvironment() {
  const { readFileSync } = await import("node:fs");
  const swSource = readFileSync(swSourcePath, "utf-8");
  const { self, listeners } = createMockSwGlobal();

  // 在 mock self 环境中 eval SW 脚本
  const wrapped = `(function(self, module) {\n${swSource}\n})`;
  const fn = eval(wrapped);
  const mod = { exports: {} };
  fn(self, mod);

  return { self, listeners, exports: mod.exports };
}

describe("Service Worker script internal protocol", () => {
  it("handleRequest returns response with id for initRuntime", async () => {
    const { exports } = await loadSwInMockEnvironment();
    const response = await exports.handleRequest({
      id: "req-1",
      method: "initRuntime",
      args: [{}],
    });
    assert.strictEqual(response.id, "req-1");
    assert.strictEqual(response.ok, true);
    assert.strictEqual(response.result.status, "ready");
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

  it("lazy init reuses promise for duplicate initRuntime calls", async () => {
    const { exports } = await loadSwInMockEnvironment();
    const p1 = exports._ensureInit({});
    const p2 = exports._ensureInit({});
    assert.strictEqual(p1, p2);
    await p1;
  });

  it("analyzeSuperpageSelection rejects if selection contains raw_text", async () => {
    const { exports } = await loadSwInMockEnvironment();
    // 先 init + load + build
    await exports.handleRequest({
      id: "r0",
      method: "initRuntime",
      args: [{}],
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
    assert.ok(response.error.message.includes("raw_text"));
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

  it("start returns runtimeClient with five required methods", async () => {
    const transport = makeFakeTransportWithSwProtocol();
    const launcher = createServiceWorkerRuntimeLauncher({ transport });
    const client = await launcher.start();
    assert.ok(typeof client.initRuntime === "function");
    assert.ok(typeof client.runtimeStatus === "function");
    assert.ok(typeof client.loadSuperpageDocument === "function");
    assert.ok(typeof client.buildOrUpdateSuperpageGraph === "function");
    assert.ok(typeof client.analyzeSuperpageSelection === "function");
  });

  it("start failure falls back to page runtime and marks fallbackUsed", async () => {
    const fallbackClient = {
      initRuntime: () => Promise.resolve({ status: "ready" }),
      runtimeStatus: () => Promise.resolve({ status: "ready" }),
      loadSuperpageDocument: () => Promise.resolve({ status: "ready" }),
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

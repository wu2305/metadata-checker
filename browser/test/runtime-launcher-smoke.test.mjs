/**
 * M40.4 Runtime Launcher Smoke Tests
 *
 * 使用 Node 内置 node:test + assert，不引入外部依赖。
 */

import { describe, it } from "node:test";
import assert from "node:assert";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { existsSync, readFileSync } from "node:fs";

import { createFakeMessageTransport } from "./fake-message-transport.mjs";
import {
  createMessageRuntimeClient,
  MESSAGE_RUNTIME_CLIENT_CONTROL,
} from "../runtime-launchers/message-runtime-client.mjs";
import { createPageRuntimeLauncher } from "../runtime-launchers/page-runtime-launcher.mjs";
import { createRuntimeLauncher } from "../runtime-launchers/runtime-launcher.mjs";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const REQUIRED_METHODS = [
  "initRuntime",
  "runtimeStatus",
  "loadSuperpageDocument",
  "loadRemoteSuperpageDocument",
  "buildOrUpdateSuperpageGraph",
  "analyzeSuperpageSelection",
];

const LAUNCHER_KINDS = ["page", "web-worker", "service-worker", "browser-extension"];
const KIND_FILE_MAP = {
  page: "../runtime-launchers/page-runtime-launcher.mjs",
  "web-worker": "../runtime-launchers/web-worker-runtime-launcher.mjs",
  "service-worker": "../runtime-launchers/service-worker-runtime-launcher.mjs",
  "browser-extension": "../runtime-launchers/browser-extension-runtime-launcher.mjs",
};

const RUNTIME_FILES = [
  "../runtime-launchers/message-runtime-client.mjs",
  "../runtime-launchers/page-runtime-launcher.mjs",
  "../runtime-launchers/runtime-launcher.mjs",
  "../runtime-launchers/web-worker-runtime-launcher.mjs",
  "../runtime-launchers/service-worker-runtime-launcher.mjs",
  "../runtime-launchers/browser-extension-runtime-launcher.mjs",
];

function makeEnvelopeForMethod(method, args = []) {
  const selection = args[0] || {};
  const sourcePath = args[0] || null;

  if (method === "initRuntime") {
    return {
      status: "ready",
      target: null,
      items: [
        {
          kind: "runtime_ready",
          label: "Runtime Initialized",
          detail: {
            options: args[0] || {},
          },
        },
      ],
      diagnostics: [],
    };
  }

  if (method === "runtimeStatus") {
    return {
      status: "ready",
      target: null,
      items: [
        {
          kind: "runtime_status",
          label: "Runtime Status",
          detail: { initialized: true },
        },
      ],
      diagnostics: [],
    };
  }

  if (method === "loadSuperpageDocument") {
    return {
      status: "ready",
      target: sourcePath,
      items: [
        {
          kind: "document_loaded",
          label: "Document Loaded",
          detail: { source_path: sourcePath },
        },
      ],
      diagnostics: [],
    };
  }

  if (method === "loadRemoteSuperpageDocument") {
    return {
      status: "ready",
      target: selection.source_path ?? null,
      items: [
        {
          kind: "remote_document_loaded",
          label: "Remote Document Loaded",
          detail: {
            source_path: selection.source_path,
            file_id: selection.file_id,
          },
        },
      ],
      diagnostics: [],
    };
  }

  if (method === "buildOrUpdateSuperpageGraph") {
    return {
      status: "ready",
      target: sourcePath,
      items: [
        {
          kind: "graph_built",
          label: "Graph Built",
          detail: { source_path: sourcePath },
        },
      ],
      diagnostics: [],
    };
  }

  if (method === "analyzeSuperpageSelection") {
    return {
      status: "ready",
      target: selection.active_component_id ?? selection.source_path ?? null,
      items: [
        {
          kind: "analysis",
          label: "Analysis Result",
          detail: {
            source_path: selection.source_path,
            active_component_id: selection.active_component_id,
          },
        },
      ],
      diagnostics: [],
    };
  }

  return {
    status: "error",
    target: null,
    items: [],
    diagnostics: [
      {
        severity: "error",
        code: "LAUNCHER_REQUEST_FAILED",
        message: `Unknown runtime method: ${method}`,
      },
    ],
  };
}

function createStubRuntimeClient() {
  const calls = [];
  return {
    calls,
    initRuntime(...args) {
      calls.push({ method: "initRuntime", args });
      return makeEnvelopeForMethod("initRuntime", args);
    },
    runtimeStatus(...args) {
      calls.push({ method: "runtimeStatus", args });
      return makeEnvelopeForMethod("runtimeStatus", args);
    },
    loadSuperpageDocument(...args) {
      calls.push({ method: "loadSuperpageDocument", args });
      return makeEnvelopeForMethod("loadSuperpageDocument", args);
    },
    loadRemoteSuperpageDocument(...args) {
      calls.push({ method: "loadRemoteSuperpageDocument", args });
      return makeEnvelopeForMethod("loadRemoteSuperpageDocument", args);
    },
    buildOrUpdateSuperpageGraph(...args) {
      calls.push({ method: "buildOrUpdateSuperpageGraph", args });
      return makeEnvelopeForMethod("buildOrUpdateSuperpageGraph", args);
    },
    analyzeSuperpageSelection(...args) {
      calls.push({ method: "analyzeSuperpageSelection", args });
      return makeEnvelopeForMethod("analyzeSuperpageSelection", args);
    },
    getCallCount(method) {
      return calls.filter((c) => c.method === method).length;
    },
  };
}

function assertClientMethodShape(client, label = "runtime client") {
  assert.deepStrictEqual(
    Object.keys(client).sort(),
    [...REQUIRED_METHODS].sort(),
    `${label} should expose only the public runtime methods`
  );
  for (const method of REQUIRED_METHODS) {
    assert.strictEqual(
      typeof client[method],
      "function",
      `${label} should implement ${method}`
    );
  }
}

function assertNoForbiddenRuntimeSource(source, sourcePath) {
  const forbiddenTokens = [
    "metadata-checker-plugin",
    "platform-glue",
    "plugin/core",
    "plugin-core",
    "fetch(",
    "window.",
    "document.",
    "navigator.",
  ];

  for (const token of forbiddenTokens) {
    assert.strictEqual(
      source.includes(token),
      false,
      `${sourcePath} contains forbidden token "${token}"`
    );
  }
}

async function loadLauncherFactoryForKind(kind) {
  if (kind === "page") {
    return createPageRuntimeLauncher;
  }

  const modulePath = KIND_FILE_MAP[kind];
  const moduleUrl = join(__dirname, modulePath);
  if (!existsSync(moduleUrl)) {
    throw new Error(`Missing launcher module for kind "${kind}": ${moduleUrl}`);
  }

  const factoryMap = {
    "web-worker": "createWebWorkerRuntimeLauncher",
    "service-worker": "createServiceWorkerRuntimeLauncher",
    "browser-extension": "createBrowserExtensionRuntimeLauncher",
  };

  const moduleExports = await import(modulePath);
  const factoryName = factoryMap[kind];
  const factory = moduleExports?.[factoryName] ?? moduleExports?.default;
  if (typeof factory !== "function") {
    return createRuntimeLauncher;
  }
  return factory;
}

async function createLauncherByKind(kind, options = {}) {
  if (kind === "page") {
    return createPageRuntimeLauncher(options);
  }

  const factory = await loadLauncherFactoryForKind(kind);
  if (factory === createRuntimeLauncher) {
    return createRuntimeLauncher({ ...options, kind });
  }

  return factory(options);
}

function makeRequestTimeoutPromise() {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

describe("Runtime Launcher Runtime-Contract (M40.4)", () => {
  it("page launcher: start/getClient/status/stop basic lifecycle", async () => {
    const runtimeClient = createStubRuntimeClient();
    const launcher = createPageRuntimeLauncher({ runtimeClient });

    const initial = launcher.status();
    assert.strictEqual(initial.state, "idle");
    assert.strictEqual(initial.started, false);
    assert.strictEqual(initial.pendingRequestCount, 0);
    assert.strictEqual(launcher.getClient(), null);

    const client = await launcher.start();
    assertClientMethodShape(client, "page launcher client");
    assert.strictEqual(launcher.getClient(), client);

    const ready = await client.runtimeStatus();
    assert.strictEqual(ready.status, "ready");

    const status = launcher.status();
    assert.strictEqual(status.state, "ready");
    assert.strictEqual(status.started, true);
    assert.strictEqual(status.fallbackUsed, false);
    assert.strictEqual(status.pendingRequestCount, 0);

    const stopResult = launcher.stop();
    assert.deepStrictEqual(stopResult, { stopped: true });
    assert.strictEqual(launcher.getClient(), null);
    assert.strictEqual(launcher.status().state, "stopped");
    assert.strictEqual(launcher.status().pendingRequestCount, 0);

    const restarted = await launcher.start();
    assert.strictEqual(typeof restarted.runtimeStatus, "function");
    assert.deepStrictEqual(launcher.getClient(), restarted);
    assert.strictEqual(launcher.status().state, "ready");
  });

  it("page launcher: concurrent start should dedupe and ready start should reuse same client reference", async () => {
    let factoryCalls = 0;
    const delegateClient = createStubRuntimeClient();
    const launcher = createPageRuntimeLauncher({
      runtimeClientFactory: async () => {
        factoryCalls += 1;
        return delegateClient;
      },
    });

    const [c1, c2] = await Promise.all([launcher.start(), launcher.start()]);
    assert.strictEqual(c1, c2);
    assert.strictEqual(factoryCalls, 1);

    const c3 = await launcher.start();
    assert.strictEqual(c3, c1);
    assert.strictEqual(factoryCalls, 1);
  });

  it("fake web-worker launcher should return M40.3 compatible client via message transport", async () => {
    const transport = createFakeMessageTransport();
    const messageClient = createMessageRuntimeClient({ transport });
    const launcher = await createLauncherByKind("web-worker", {
      runtimeClientFactory: async () => messageClient,
      runtimeClient: messageClient,
      runtimeModule: { default: messageClient },
      transport,
      messageTransport: transport,
    });
    const client = await launcher.start();

    assertClientMethodShape(client, "web-worker launcher client");
    const ready = await client.initRuntime({ test: true });
    assert.strictEqual(ready.status, "ready");
    assert.ok(ready.items?.length > 0);
    assert.deepStrictEqual(ready.items?.[0]?.kind, "runtime_ready");
  });

  it("message transport: request id must match async response", async () => {
    const transport = createFakeMessageTransport({ autoRespond: false });
    const mismatched = [];
    const client = createMessageRuntimeClient({
      transport,
      requestTimeoutMs: 0,
      onError: (err) => mismatched.push(err),
    });

    const p1 = client.initRuntime({ mode: "single" });
    const p2 = client.runtimeStatus();
    await Promise.resolve();

    const requests = transport.getSentRequests();
    assert.strictEqual(requests.length, 2);
    const [r1, r2] = requests;
    assert.notStrictEqual(String(r1.id), String(r2.id));

    transport.emitResponse({
      id: "not-matching-id",
      ok: true,
      result: makeEnvelopeForMethod("runtimeStatus"),
      error: null,
    });
    await Promise.resolve();

    assert.strictEqual(mismatched.length, 1);
    assert.strictEqual(mismatched[0].code, "LAUNCHER_RESPONSE_MISMATCH");

    transport.emitResponse({ id: r1.id, ok: true, result: makeEnvelopeForMethod("initRuntime") });
    transport.emitResponse({ id: r2.id, ok: true, result: makeEnvelopeForMethod("runtimeStatus") });
    const [r1Result, r2Result] = await Promise.all([p1, p2]);

    assert.strictEqual(r1Result.status, "ready");
    assert.strictEqual(r2Result.status, "ready");
    assert.strictEqual(mismatched.length, 1);
  });

  it("message transport: unknown response id must not resolve wrong request and should record mismatch", async () => {
    const transport = createFakeMessageTransport({ autoRespond: false });
    const errors = [];
    const client = createMessageRuntimeClient({
      transport,
      requestTimeoutMs: 0,
      onError: (entry) => errors.push(entry),
    });

    const responsePromise = client.runtimeStatus();
    await Promise.resolve();

    const request = transport.getSentRequests().at(-1);
    assert.ok(request, "runtimeStatus request should be sent");

    transport.emitResponse({
      id: "unknown-id",
      ok: true,
      result: makeEnvelopeForMethod("runtimeStatus"),
      error: null,
    });
    await Promise.resolve();

    assert.strictEqual(errors.length, 1, "unknown id response must not resolve request");
    assert.strictEqual(errors[0].code, "LAUNCHER_RESPONSE_MISMATCH");
    assert.strictEqual(errors[0].responseId, "unknown-id");

    transport.emitResponse({
      id: request.id,
      ok: true,
      result: makeEnvelopeForMethod("runtimeStatus"),
      error: null,
    });

    const response = await responsePromise;
    assert.strictEqual(response.status, "ready");
  });

  it("request reject path: response ok=false should map to LAUNCHER_REQUEST_FAILED", async () => {
    const transport = createFakeMessageTransport({ autoRespond: true, failSend: 1 });
    const client = createMessageRuntimeClient({ transport, requestTimeoutMs: 0 });
    const failed = await client.initRuntime({});
    assert.strictEqual(failed.status, "error");
    assert.strictEqual(failed.diagnostics?.[0]?.code, "LAUNCHER_REQUEST_FAILED");
  });

  it("message transport: ok=false should preserve stable response error code", async () => {
    const transport = createFakeMessageTransport({ autoRespond: false });
    const client = createMessageRuntimeClient({ transport, requestTimeoutMs: 0 });
    const resultPromise = client.initRuntime({ wasmUrl: "/runtime.wasm" });
    await Promise.resolve();

    const request = transport.getSentRequests().at(-1);
    transport.emitResponse({
      id: request.id,
      ok: false,
      result: null,
      error: {
        code: "WASM_FETCH_FAILED",
        message: "failed to fetch runtime wasm",
      },
    });

    const failed = await resultPromise;
    assert.strictEqual(failed.status, "error");
    assert.strictEqual(failed.target, null);
    assert.deepStrictEqual(failed.items, []);
    assert.strictEqual(failed.diagnostics?.[0]?.code, "WASM_FETCH_FAILED");
    assert.strictEqual(
      failed.diagnostics?.[0]?.message,
      "failed to fetch runtime wasm"
    );
  });

  it("message transport: ok=false without response error code falls back to LAUNCHER_REQUEST_FAILED", async () => {
    const transport = createFakeMessageTransport({ autoRespond: false });
    const client = createMessageRuntimeClient({ transport, requestTimeoutMs: 0 });
    const resultPromise = client.initRuntime({});
    await Promise.resolve();

    const request = transport.getSentRequests().at(-1);
    transport.emitResponse({
      id: request.id,
      ok: false,
      result: null,
      error: {
        message: "runtime failed without stable code",
      },
    });

    const failed = await resultPromise;
    assert.strictEqual(failed.status, "error");
    assert.strictEqual(failed.target, null);
    assert.deepStrictEqual(failed.items, []);
    assert.strictEqual(failed.diagnostics?.[0]?.code, "LAUNCHER_REQUEST_FAILED");
    assert.strictEqual(
      failed.diagnostics?.[0]?.message,
      "runtime failed without stable code"
    );
  });

  it("request reject path: transport send throw should map to LAUNCHER_REQUEST_FAILED", async () => {
    const transport = createFakeMessageTransport({ throwOnSend: true, autoRespond: false });
    const client = createMessageRuntimeClient({ transport, requestTimeoutMs: 0 });
    const failed = await client.runtimeStatus();
    assert.strictEqual(failed.status, "error");
    assert.strictEqual(failed.diagnostics?.[0]?.code, "LAUNCHER_REQUEST_FAILED");
    assert.strictEqual(failed.diagnostics?.[0]?.message.includes("Fake transport send failure"), true);
  });

  it("page launcher request throw should map to LAUNCHER_REQUEST_FAILED envelope", async () => {
    const throwingClient = {
      ...createStubRuntimeClient(),
      runtimeStatus() {
        throw new Error("direct page runtime failed");
      },
    };
    const launcher = createPageRuntimeLauncher({ runtimeClient: throwingClient });
    const client = await launcher.start();
    const failed = await client.runtimeStatus();
    assert.strictEqual(failed.status, "error");
    assert.strictEqual(failed.diagnostics?.[0]?.code, "LAUNCHER_REQUEST_FAILED");
    assert.strictEqual(
      failed.diagnostics?.[0]?.message.includes("direct page runtime failed"),
      true
    );
  });

  it("requestTimeoutMs should return LAUNCHER_REQUEST_TIMEOUT", async () => {
    const transport = createFakeMessageTransport({ autoRespond: false });
    const client = createMessageRuntimeClient({ transport, requestTimeoutMs: 5 });
    const timedOut = await client.loadSuperpageDocument("app/test.spg");
    assert.strictEqual(timedOut.status, "error");
    assert.strictEqual(timedOut.diagnostics?.[0]?.code, "LAUNCHER_REQUEST_TIMEOUT");
    await makeRequestTimeoutPromise();
    const control = client[MESSAGE_RUNTIME_CLIENT_CONTROL];
    assert.strictEqual(control.getPendingRequestCount(), 0);
  });

  it("service-worker launcher start failure should fallback and mark fallbackUsed=true", async () => {
    const fallbackClient = createStubRuntimeClient();
    let primaryCalled = 0;
    const launcher = await createLauncherByKind("service-worker", {
      transportFactory: () => {
        primaryCalled += 1;
        throw new Error("SW unavailable");
      },
      fallbackRuntimeClient: fallbackClient,
    });

    const client = await launcher.start();
    const status = launcher.status();
    assert.strictEqual(status.fallbackUsed, true, "service-worker start failure should use fallback");
    assert.strictEqual(primaryCalled, 1);
    assertClientMethodShape(client, "service-worker fallback client");
  });

  it("page launcher stop should clear client and pendingRequestCount, then allow restart", async () => {
    const pending = { resolve: null };
    const slowClient = {
      initRuntime: () => Promise.resolve(makeEnvelopeForMethod("initRuntime")),
      runtimeStatus: () => new Promise((resolve) => (pending.resolve = resolve)),
      loadSuperpageDocument: (sourcePath) =>
        makeEnvelopeForMethod("loadSuperpageDocument", [sourcePath]),
      loadRemoteSuperpageDocument: (fileRef) =>
        makeEnvelopeForMethod("loadRemoteSuperpageDocument", [fileRef]),
      buildOrUpdateSuperpageGraph: (sourcePath) =>
        makeEnvelopeForMethod("buildOrUpdateSuperpageGraph", [sourcePath]),
      analyzeSuperpageSelection: (selection) =>
        makeEnvelopeForMethod("analyzeSuperpageSelection", [selection]),
    };

    const launcher = createPageRuntimeLauncher({ runtimeClient: slowClient });
    await launcher.start();

    const pendingRequest = launcher.getClient().runtimeStatus();
    await Promise.resolve();
    assert.ok(launcher.status().pendingRequestCount > 0);

    const stop = launcher.stop();
    assert.strictEqual(stop.stopped, true);
    assert.strictEqual(launcher.getClient(), null);
    assert.strictEqual(launcher.status().pendingRequestCount, 0);

    const restarted = await launcher.start();
    assertClientMethodShape(restarted, "restarted page launcher client");
    assert.strictEqual(launcher.status().state, "ready");

    const restartReady = await restarted.initRuntime({ test: true });
    assert.strictEqual(restartReady.status, "ready");

    pending.resolve?.(makeEnvelopeForMethod("runtimeStatus"));
    const stoppedResult = await pendingRequest;
    assert.strictEqual(stoppedResult.status, "error");
    assert.strictEqual(stoppedResult.diagnostics?.[0]?.code, "LAUNCHER_REQUEST_FAILED");
  });

  it("web-worker launcher stop/restart with same transport should not leak old listener and should keep new request healthy", async () => {
    const transport = createFakeMessageTransport({ autoRespond: false });
    let requestIdCounter = 0;
    const launcher = await createLauncherByKind("web-worker", {
      transport,
      idGenerator: () => `msg-${++requestIdCounter}`,
    });

    const firstClient = await launcher.start();
    const firstRequest = firstClient.runtimeStatus();
    await Promise.resolve();
    const firstRequestRecord = transport.getSentRequests().at(-1);
    assert.ok(firstRequestRecord);

    launcher.stop();
    assert.strictEqual(launcher.status().state, "stopped");
    assert.strictEqual(launcher.status().lastError, null);

    const secondClient = await launcher.start();
    const secondRequest = secondClient.runtimeStatus();
    await Promise.resolve();
    const secondRequestRecord = transport.getSentRequests().at(-1);
    assert.ok(secondRequestRecord);
    assert.notStrictEqual(String(firstRequestRecord.id), String(secondRequestRecord.id));

    transport.emitResponse({
      id: firstRequestRecord.id,
      ok: true,
      result: makeEnvelopeForMethod("runtimeStatus"),
      error: null,
    });
    await Promise.resolve();

    transport.emitResponse({
      id: secondRequestRecord.id,
      ok: true,
      result: makeEnvelopeForMethod("runtimeStatus"),
      error: null,
    });

    const firstResult = await firstRequest;
    const secondResult = await secondRequest;
    assert.strictEqual(firstResult.status, "error");
    assert.strictEqual(firstResult.diagnostics?.[0]?.code, "LAUNCHER_REQUEST_FAILED");
    assert.strictEqual(secondResult.status, "ready");
    assert.strictEqual(launcher.status().lastError, null);
  });

  it("all launcher kinds should expose the same runtime client methods", async () => {
    for (const kind of LAUNCHER_KINDS) {
      const isPage = kind === "page";
      const runtimeClient = isPage
        ? createStubRuntimeClient()
        : createMessageRuntimeClient({
            transport: createFakeMessageTransport(),
            requestTimeoutMs: 0,
          });
      const launcher = await createLauncherByKind(kind, {
        runtimeClient,
        runtimeClientFactory: async () => runtimeClient,
        runtimeModule: { default: runtimeClient },
        transport: createFakeMessageTransport(),
      });

      const client = await launcher.start();
      assertClientMethodShape(client, `${kind} runtime client`);
      const status = launcher.status();
      assert.strictEqual(
        status.kind,
        kind,
        `${kind} launcher should keep kind in status`
      );
      launcher.stop();
    }
  });

  it("runtime-launcher source files should only reference allowed runtime-only deps", () => {
    for (const relativePath of RUNTIME_FILES) {
      const absolutePath = join(__dirname, relativePath);
      assert.ok(existsSync(absolutePath), `Expected runtime file exists: ${relativePath}`);
      const source = readFileSync(absolutePath, "utf-8");
      assertNoForbiddenRuntimeSource(source, relativePath);
    }
  });

  it("runtime launcher smoke suite should run as ESM with node:test (no package-type warning path)", () => {
    assert.ok(import.meta.url.endsWith(".mjs"));
    const runtimeSuite = join(__dirname, "../runtime-launchers/page-runtime-launcher.mjs");
    const transportSource = join(__dirname, "./fake-message-transport.mjs");
    const testSource = join(__dirname, "./runtime-launcher-smoke.test.mjs");
    assert.ok(runtimeSuite.endsWith(".mjs"));
    assert.ok(transportSource.endsWith(".mjs"));
    assert.ok(testSource.endsWith(".mjs"));
  });
});

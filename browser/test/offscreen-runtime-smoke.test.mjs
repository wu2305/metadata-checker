import assert from "node:assert/strict";
import { copyFile, mkdtemp, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import test from "node:test";
import { pathToFileURL } from "node:url";

const OFFSCREEN_MESSAGE_TYPE = "metadata-checker-offscreen-wasm-call";

async function importOffscreenRuntime({ failInit = false } = {}) {
  const root = await mkdtemp(join(tmpdir(), "metadata-checker-offscreen-runtime-"));
  await copyFile(
    new URL("../extension-chromium/offscreen-runtime.js", import.meta.url),
    join(root, "offscreen-runtime.js"),
  );
  await writeFile(
    join(root, "metadata_checker.js"),
    `
const calls = globalThis.__m45OffscreenRuntimeCalls;

export default async function initWasmRuntime(wasmUrl) {
  calls.push({ method: "init", wasmUrl });
  if (globalThis.__m45OffscreenFailInit) {
    throw new Error("forced wasm init failure");
  }
}

export async function loadSuperpageDocument(sourcePath, rawText) {
  calls.push({ method: "loadSuperpageDocument", sourcePath, rawText });
  return JSON.stringify({ status: "ready", sourcePath, rawText });
}
`,
    "utf8",
  );

  const previousChrome = globalThis.chrome;
  const previousCalls = globalThis.__m45OffscreenRuntimeCalls;
  const previousFailInit = globalThis.__m45OffscreenFailInit;
  const listeners = [];
  globalThis.__m45OffscreenRuntimeCalls = [];
  globalThis.__m45OffscreenFailInit = failInit;
  globalThis.chrome = {
    runtime: {
      getURL(path) {
        return `chrome-extension://id/${path}`;
      },
      onMessage: {
        addListener(listener) {
          listeners.push(listener);
        },
      },
    },
  };

  await import(`${pathToFileURL(join(root, "offscreen-runtime.js")).href}?case=${Math.random()}`);
  assert.equal(listeners.length, 1);

  async function dispatch(payload) {
    return new Promise((resolve) => {
      const keepAlive = listeners[0](payload, {}, resolve);
      assert.equal(keepAlive, true);
    });
  }

  async function cleanup() {
    globalThis.chrome = previousChrome;
    globalThis.__m45OffscreenRuntimeCalls = previousCalls;
    globalThis.__m45OffscreenFailInit = previousFailInit;
    await rm(root, { recursive: true, force: true });
  }

  return {
    calls: globalThis.__m45OffscreenRuntimeCalls,
    dispatch,
    cleanup,
  };
}

test("M45 offscreen runtime host initializes WASM and dispatches method calls", async () => {
  const runtime = await importOffscreenRuntime();
  try {
    const response = await runtime.dispatch({
      type: OFFSCREEN_MESSAGE_TYPE,
      request_id: "req-1",
      payload: {
        method: "loadSuperpageDocument",
        args: ["app/Page.spg", "{\"raw_text\":\"literal\"}"],
        wasm_url: "chrome-extension://id/custom.wasm",
      },
    });

    assert.equal(response.ok, true);
    assert.equal(response.request_id, "req-1");
    assert.equal(
      response.result,
      JSON.stringify({
        status: "ready",
        sourcePath: "app/Page.spg",
        rawText: "{\"raw_text\":\"literal\"}",
      }),
    );
    assert.deepEqual(
      runtime.calls.map((call) => call.method),
      ["init", "loadSuperpageDocument"],
    );
    assert.equal(runtime.calls[0].wasmUrl, "chrome-extension://id/custom.wasm");
  } finally {
    await runtime.cleanup();
  }
});

test("M45 offscreen runtime host returns stable diagnostic for missing WASM method", async () => {
  const runtime = await importOffscreenRuntime();
  try {
    const response = await runtime.dispatch({
      type: OFFSCREEN_MESSAGE_TYPE,
      request_id: "req-missing",
      payload: {
        method: "missingRuntimeMethod",
        args: [],
      },
    });

    assert.equal(response.ok, false);
    assert.equal(response.request_id, "req-missing");
    assert.equal(response.diagnostic.code, "OFFSCREEN_WASM_METHOD_MISSING");
    assert.equal(response.diagnostic.severity, "error");
    assert.equal(response.diagnostic.message.includes("missingRuntimeMethod"), true);
  } finally {
    await runtime.cleanup();
  }
});

test("M45 offscreen runtime host returns stable diagnostic for init failure", async () => {
  const runtime = await importOffscreenRuntime({ failInit: true });
  try {
    const response = await runtime.dispatch({
      type: OFFSCREEN_MESSAGE_TYPE,
      request_id: "req-init-failed",
      payload: {
        method: "loadSuperpageDocument",
        args: ["app/Page.spg", "{}"],
      },
    });

    assert.equal(response.ok, false);
    assert.equal(response.request_id, "req-init-failed");
    assert.equal(response.diagnostic.code, "OFFSCREEN_WASM_CALL_FAILED");
    assert.equal(response.diagnostic.severity, "error");
    assert.equal(response.diagnostic.message, "forced wasm init failure");
  } finally {
    await runtime.cleanup();
  }
});

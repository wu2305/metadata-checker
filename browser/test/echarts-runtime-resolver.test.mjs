/**
 * M42.3 BI ECharts Runtime Resolver Tests
 */

import { describe, it } from "node:test";
import assert from "node:assert";

import { resolveEchartsRuntime } from "../renderer/echarts-runtime-resolver.mjs";

function createFakeRequire(handler = {}) {
  const calls = [];
  const records = [];
  const fn = (...args) => {
    const [deps, onSuccess, onError] = args;
    const moduleName = Array.isArray(deps) ? deps[0] : deps;
    calls.push({ args });
    records.push(moduleName);

    if (typeof onSuccess !== "function") {
      throw new TypeError("requireLike requires success callback");
    }

    const result = handler[moduleName];
    if (result && typeof result === "object" && result.async === true) {
      const timer = setTimeout(() => onSuccess(result.value), 0);
      timer.unref?.();
      return undefined;
    }

    if (result && typeof result === "object" && result.promise === true) {
      return Promise.resolve(result.value);
    }

    if (result && Object.prototype.hasOwnProperty.call(result, "error")) {
      onError(result.error);
      return undefined;
    }

    if (typeof result === "undefined") {
      onSuccess(null);
      return undefined;
    }

    if (result && typeof result === "object" && "value" in result) {
      onSuccess(result.value);
      return undefined;
    }

    onSuccess(result);
    return undefined;
  };

  fn.calls = calls;
  fn.records = records;
  return fn;
}

describe("resolveEchartsRuntime", () => {
  it("resolves BI AMD module commons/echarts/echarts-ext via getEcharts()", async () => {
    const echartsExt = {
      getEcharts() {
        return {
          version: "4.x-resolver-test",
          rendering: "resolved-from-ext",
        };
      },
    };

    const requireLike = createFakeRequire({
      "commons/echarts/echarts-ext": {
        value: echartsExt,
      },
      echarts: { value: { version: "should-not-be-used" } },
    });

    const result = await resolveEchartsRuntime({
      requireLike,
      timeoutMs: 200,
    });

    assert.strictEqual(result.source, "commons/echarts/echarts-ext");
    assert.strictEqual(
      result.echarts.rendering,
      "resolved-from-ext"
    );
    assert.strictEqual(Array.isArray(result.diagnostics), true);
    assert.strictEqual(result.diagnostics.length, 0);
    assert.deepStrictEqual(requireLike.records, [
      "commons/echarts/echarts-ext",
    ]);
  });

  it("falls back to AMD module echarts when commons/echarts/echarts-ext fails", async () => {
    const requireLike = createFakeRequire({
      "commons/echarts/echarts-ext": { error: new Error("not found") },
      echarts: {
        value: { version: "4.x-fallback", from: "amd-echarts" },
      },
    });

    const result = await resolveEchartsRuntime({ requireLike, timeoutMs: 200 });

    assert.strictEqual(result.source, "echarts");
    assert.strictEqual(result.echarts.from, "amd-echarts");
    assert.strictEqual(result.diagnostics.length, 0);
    assert.deepStrictEqual(requireLike.records, [
      "commons/echarts/echarts-ext",
      "echarts",
    ]);
  });

  it("supports Promise wrapped AMD resolution", async () => {
    const requireLike = createFakeRequire({
      "commons/echarts/echarts-ext": {
        promise: true,
        value: {
          getEcharts: async () => ({ version: "4.x-promise", marker: "amd-promise" }),
        },
      },
      echarts: { value: { version: "should-not-be-used" } },
    });

    const result = await resolveEchartsRuntime({ requireLike, timeoutMs: 200 });

    assert.strictEqual(result.source, "commons/echarts/echarts-ext");
    assert.strictEqual(result.echarts.marker, "amd-promise");
    assert.strictEqual(result.diagnostics.length, 0);
  });

  it("falls back to window.echarts when AMD modules are unavailable", async () => {
    const requireLike = createFakeRequire({
      "commons/echarts/echarts-ext": { value: null },
      echarts: undefined,
    });
    const globalThisLike = {
      window: {
        echarts: { version: "4.x-window", marker: "from-window" },
      },
    };

    const result = await resolveEchartsRuntime({ requireLike, globalThisLike });

    assert.strictEqual(result.source, "window.echarts");
    assert.strictEqual(result.echarts.marker, "from-window");
    assert.strictEqual(result.diagnostics.length, 0);
  });

  it("returns null + stable diagnostic when no runtime is available", async () => {
    const requireLike = createFakeRequire({
      "commons/echarts/echarts-ext": { value: null },
      echarts: { value: null },
    });
    const result = await resolveEchartsRuntime({ requireLike });

    assert.strictEqual(result.echarts, null);
    assert.strictEqual(result.source, "fallback");
    assert.strictEqual(Array.isArray(result.diagnostics), true);
    assert.ok(result.diagnostics.length >= 1);
    const diagnostic = result.diagnostics[0];
    assert.strictEqual(diagnostic.code, "ECHARTS_RESOLVER_UNAVAILABLE");
    assert.strictEqual(diagnostic.severity, "warning");
    assert.ok(typeof diagnostic.message === "string");
  });

  it("runs in Node-like environments without window", async () => {
    const result = await resolveEchartsRuntime({
      requireLike: undefined,
      globalThisLike: {},
    });

    assert.strictEqual(result.echarts, null);
    assert.strictEqual(result.source, "fallback");
    assert.ok(Array.isArray(result.diagnostics));
    assert.ok(result.diagnostics.length > 0);
  });
});

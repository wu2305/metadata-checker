/**
 * M40.3 Plugin Core Smoke Tests
 *
 * 使用 Node 内置 node:test + assert。
 */

import { describe, it } from "node:test";
import assert from "node:assert";
import { createFakeHost } from "./fake-host.mjs";
import { createFakeRuntimeClient } from "./fake-runtime-client.mjs";
import { createMetadataCheckerPlugin } from "../plugin-core/metadata-checker-plugin.mjs";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const pluginSourcePath = join(__dirname, "../plugin-core/metadata-checker-plugin.mjs");

function createPlugin(options = {}) {
  const host = options.host ?? createFakeHost();
  const runtimeClient = options.runtimeClient ?? createFakeRuntimeClient();
  return {
    plugin: createMetadataCheckerPlugin({
      runtimeClient,
      host,
      logger: options.logger,
      clock: options.clock ?? (() => 12345),
      defaultAnalysisOptions: options.defaultAnalysisOptions,
    }),
    host,
    runtimeClient,
  };
}

const validSelection = {
  source_path: "app/test.spg",
  file_id: "f1",
  selected_component_ids: ["btn1"],
  active_component_id: "btn1",
};

describe("createMetadataCheckerPlugin parameter validation", () => {
  it("throws when host lacks emit", () => {
    assert.throws(
      () => createMetadataCheckerPlugin({ runtimeClient: createFakeRuntimeClient(), host: {} }),
      /emit/
    );
  });

  it("throws when runtimeClient lacks initRuntime", () => {
    const client = createFakeRuntimeClient();
    delete client.initRuntime;
    assert.throws(
      () => createMetadataCheckerPlugin({ runtimeClient: client, host: createFakeHost() }),
      /initRuntime/
    );
  });

  it("throws when runtimeClient lacks analyzeSuperpageSelection", () => {
    const client = createFakeRuntimeClient();
    delete client.analyzeSuperpageSelection;
    assert.throws(
      () => createMetadataCheckerPlugin({ runtimeClient: client, host: createFakeHost() }),
      /analyzeSuperpageSelection/
    );
  });
});

describe("activate", () => {
  it("calls initRuntime and runtimeStatus, emits plugin_activated and runtime_ready", async () => {
    const { plugin, host, runtimeClient } = createPlugin();
    const result = await plugin.activate();
    assert.strictEqual(result.status, "ready");
    assert.strictEqual(plugin.status().state, "ready");
    assert.strictEqual(plugin.status().activated, true);

    const initCalls = runtimeClient.callLog.filter((c) => c.method === "initRuntime");
    const statusCalls = runtimeClient.callLog.filter((c) => c.method === "runtimeStatus");
    assert.strictEqual(initCalls.length, 1);
    assert.strictEqual(statusCalls.length, 1);

    const activatedEvents = host.getEvents("plugin_activated");
    const readyEvents = host.getEvents("runtime_ready");
    assert.strictEqual(activatedEvents.length, 1);
    assert.strictEqual(readyEvents.length, 1);
  });

  it("is idempotent: second activate does not re-init runtime, re-emit, or change return", async () => {
    const { plugin, host, runtimeClient } = createPlugin();
    const r1 = await plugin.activate();
    const eventCountBefore = host.events.length;
    const r2 = await plugin.activate();

    const initCalls = runtimeClient.callLog.filter((c) => c.method === "initRuntime");
    assert.strictEqual(initCalls.length, 1);
    assert.strictEqual(plugin.status().state, "ready");

    const eventCountAfter = host.events.length;
    assert.strictEqual(eventCountAfter, eventCountBefore, "second activate must not emit new events");
    assert.deepStrictEqual(r1, r2);
  });

  it("allows retry from error state with same plugin instance", async () => {
    const host = createFakeHost();
    const retryClient = createFakeRuntimeClient({ returnType: "promise" });
    let initCalls = 0;

    retryClient.initRuntime = () => {
      initCalls += 1;
      retryClient.callLog.push({ method: "initRuntime", args: [] });
      if (initCalls === 1) {
        return Promise.reject(new Error("initRuntime failed"));
      }
      return Promise.resolve({
        status: "ready",
        target: null,
        items: [],
        diagnostics: [],
      });
    };

    const { plugin, runtimeClient } = createPlugin({ host, runtimeClient: retryClient });
    const r1 = await plugin.activate();
    assert.strictEqual(r1.status, "error");
    assert.strictEqual(plugin.status().state, "error");
    assert.strictEqual(initCalls, 1);
    assert.strictEqual(runtimeClient.callLog.filter((c) => c.method === "runtimeStatus").length, 0);

    const r2 = await plugin.activate();
    assert.strictEqual(r2.status, "ready");
    assert.strictEqual(plugin.status().state, "ready");
    assert.strictEqual(initCalls, 2);
    assert.strictEqual(runtimeClient.callLog.filter((c) => c.method === "runtimeStatus").length, 1);
    assert.strictEqual(runtimeClient.callLog.filter((c) => c.method === "initRuntime").length, 2);
  });

  it("keeps first analysisOptions after repeated activate in ready state", async () => {
    const { plugin, runtimeClient } = createPlugin({
      runtimeClient: createFakeRuntimeClient({ returnType: "promise" }),
    });
    await plugin.activate({ analysisOptions: { include_conditions: true } });
    await plugin.activate({ analysisOptions: { include_conditions: false, include_priority: true } });
    await plugin.analyze(validSelection);

    const analyzeCalls = runtimeClient.callLog.filter(
      (c) => c.method === "analyzeSuperpageSelection"
    );
    assert.strictEqual(analyzeCalls.length, 1);
    assert.strictEqual(analyzeCalls[0].args[1].include_conditions, true);
    assert.strictEqual(analyzeCalls[0].args[1].include_priority, undefined);
  });
});

describe("deactivate", () => {
  it("clears state and emits plugin_deactivated", async () => {
    const { plugin, host } = createPlugin();
    await plugin.activate();
    const result = plugin.deactivate();
    assert.deepStrictEqual(result, { deactivated: true });
    assert.strictEqual(plugin.status().state, "inactive");
    assert.strictEqual(plugin.status().lastSelection, null);
    assert.strictEqual(plugin.status().lastResult, null);
    assert.strictEqual(host.getEvents("plugin_deactivated").length, 1);
  });
});

describe("status deep clone", () => {
  it("modifying returned status does not affect internal state", async () => {
    const { plugin } = createPlugin();
    await plugin.activate();
    const s1 = plugin.status();
    s1.state = "tampered";
    s1.lastResult = { tampered: true };
    assert.strictEqual(plugin.status().state, "ready");
    assert.notDeepStrictEqual(plugin.status().lastResult, { tampered: true });
  });
});

describe("onSelectionChanged", () => {
  it("saves valid selection and emits selection_changed", async () => {
    const { plugin, host } = createPlugin();
    const result = plugin.onSelectionChanged(validSelection);
    assert.deepStrictEqual(result, { handled: true });
    assert.deepStrictEqual(plugin.status().lastSelection, validSelection);
    assert.strictEqual(host.getEvents("selection_changed").length, 1);
  });

  it("event payload does not contain raw metadata", async () => {
    const { plugin, host } = createPlugin();
    plugin.onSelectionChanged(validSelection);
    const evt = host.getEvents("selection_changed")[0];
    const payload = JSON.stringify(evt.payload);
    assert.strictEqual(payload.includes("raw_text"), false);
    assert.strictEqual(payload.includes("components"), false);
    assert.strictEqual(payload.includes("html"), false);
  });

  it("does not auto-analyze", async () => {
    const { plugin, runtimeClient } = createPlugin();
    await plugin.activate();
    plugin.onSelectionChanged(validSelection);
    const analyzeCalls = runtimeClient.callLog.filter(
      (c) => c.method === "analyzeSuperpageSelection"
    );
    assert.strictEqual(analyzeCalls.length, 0);
    const loadCalls = runtimeClient.callLog.filter(
      (c) => c.method === "loadSuperpageDocument"
    );
    assert.strictEqual(loadCalls.length, 0);
    const buildCalls = runtimeClient.callLog.filter(
      (c) => c.method === "buildOrUpdateSuperpageGraph"
    );
    assert.strictEqual(buildCalls.length, 0);
  });

  it("does not retain references to external selection object", async () => {
    const { plugin, runtimeClient } = createPlugin();
    const externalSelection = {
      source_path: "app/test.spg",
      file_id: "f1",
      selected_component_ids: ["btn1"],
      active_component_id: "btn1",
    };
    assert.strictEqual(plugin.status().lastSelection, null);

    await plugin.activate();
    plugin.onSelectionChanged(externalSelection);
    externalSelection.source_path = "app/other.spg";
    externalSelection.file_id = "other";
    externalSelection.selected_component_ids.push("btn2");
    externalSelection.active_component_id = "btn2";

    assert.deepStrictEqual(plugin.status().lastSelection, {
      source_path: "app/test.spg",
      file_id: "f1",
      selected_component_ids: ["btn1"],
      active_component_id: "btn1",
    });

    await plugin.analyze();
    const analyzeCalls = runtimeClient.callLog.filter(
      (c) => c.method === "analyzeSuperpageSelection"
    );
    assert.strictEqual(analyzeCalls.length, 1);
    assert.deepStrictEqual(analyzeCalls[0].args[0], {
      source_path: "app/test.spg",
      file_id: "f1",
      selected_component_ids: ["btn1"],
      active_component_id: "btn1",
    });
  });
});

describe("selection schema validation", () => {
  it("rejects missing file_id", () => {
    const { plugin } = createPlugin();
    const result = plugin.onSelectionChanged({
      source_path: "app/test.spg",
      selected_component_ids: ["btn1"],
      active_component_id: "btn1",
    });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "INVALID_SELECTION");
    assert.ok(result.diagnostics[0].message.includes("file_id"));
  });

  it("rejects missing selected_component_ids", () => {
    const { plugin } = createPlugin();
    const result = plugin.onSelectionChanged({
      source_path: "app/test.spg",
      file_id: "f1",
      active_component_id: "btn1",
    });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "INVALID_SELECTION");
    assert.ok(result.diagnostics[0].message.includes("selected_component_ids"));
  });

  it("rejects missing active_component_id", () => {
    const { plugin } = createPlugin();
    const result = plugin.onSelectionChanged({
      source_path: "app/test.spg",
      file_id: "f1",
      selected_component_ids: ["btn1"],
    });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "INVALID_SELECTION");
    assert.ok(result.diagnostics[0].message.includes("active_component_id"));
  });

  it("rejects file_id that is not a string", () => {
    const { plugin } = createPlugin();
    const result = plugin.onSelectionChanged({
      source_path: "app/test.spg",
      file_id: 123,
    });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "INVALID_SELECTION");
    assert.ok(result.diagnostics[0].message.includes("file_id"));
  });

  it("rejects selected_component_ids that is not an array", () => {
    const { plugin } = createPlugin();
    const result = plugin.onSelectionChanged({
      source_path: "app/test.spg",
      file_id: "f1",
      selected_component_ids: "btn1",
    });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "INVALID_SELECTION");
    assert.ok(result.diagnostics[0].message.includes("selected_component_ids"));
  });

  it("rejects selected_component_ids containing non-strings", () => {
    const { plugin } = createPlugin();
    const result = plugin.onSelectionChanged({
      source_path: "app/test.spg",
      file_id: "f1",
      selected_component_ids: ["btn1", 123],
    });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "INVALID_SELECTION");
    assert.ok(result.diagnostics[0].message.includes("selected_component_ids"));
  });

  it("rejects active_component_id that is not string or null", () => {
    const { plugin } = createPlugin();
    const result = plugin.onSelectionChanged({
      source_path: "app/test.spg",
      file_id: "f1",
      selected_component_ids: ["btn1"],
      active_component_id: 123,
    });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "INVALID_SELECTION");
    assert.ok(result.diagnostics[0].message.includes("active_component_id"));
  });

  it("accepts active_component_id as null", () => {
    const { plugin } = createPlugin();
    const result = plugin.onSelectionChanged({
      source_path: "app/test.spg",
      file_id: "f1",
      selected_component_ids: ["btn1"],
      active_component_id: null,
    });
    assert.deepStrictEqual(result, { handled: true });
  });

  it("analyze also validates selection schema", async () => {
    const { plugin } = createPlugin();
    await plugin.activate();
    const result = await plugin.analyze({
      source_path: "app/test.spg",
      file_id: "f1",
      selected_component_ids: "not-array",
      active_component_id: "btn1",
    });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "INVALID_SELECTION");
    assert.ok(result.diagnostics[0].message.includes("selected_component_ids"));
  });

  it("analyze returns INVALID_SELECTION when file_id missing and does not call runtime analyze", async () => {
    const { plugin, runtimeClient } = createPlugin();
    await plugin.activate();
    const result = await plugin.analyze({
      source_path: "app/test.spg",
      selected_component_ids: ["btn1"],
      active_component_id: "btn1",
    });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "INVALID_SELECTION");
    assert.strictEqual(
      runtimeClient.callLog.filter((c) => c.method === "analyzeSuperpageSelection").length,
      0
    );
  });

  it("analyze returns INVALID_SELECTION when selected_component_ids missing and does not call runtime analyze", async () => {
    const { plugin, runtimeClient } = createPlugin();
    await plugin.activate();
    const result = await plugin.analyze({
      source_path: "app/test.spg",
      file_id: "f1",
      active_component_id: "btn1",
    });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "INVALID_SELECTION");
    assert.strictEqual(
      runtimeClient.callLog.filter((c) => c.method === "analyzeSuperpageSelection").length,
      0
    );
  });

  it("analyze returns INVALID_SELECTION when active_component_id missing and does not call runtime analyze", async () => {
    const { plugin, runtimeClient } = createPlugin();
    await plugin.activate();
    const result = await plugin.analyze({
      source_path: "app/test.spg",
      file_id: "f1",
      selected_component_ids: ["btn1"],
    });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "INVALID_SELECTION");
    assert.strictEqual(
      runtimeClient.callLog.filter((c) => c.method === "analyzeSuperpageSelection").length,
      0
    );
  });
});

describe("analyze", () => {
  it("calls runtime analyze, saves result, emits analysis_completed, calls renderAnalysis", async () => {
    const { plugin, host, runtimeClient } = createPlugin();
    await plugin.activate();
    const result = await plugin.analyze(validSelection, { include_conditions: true });
    assert.strictEqual(result.status, "ready");
    assert.strictEqual(plugin.status().lastResult.status, "ready");

    const analyzeCalls = runtimeClient.callLog.filter(
      (c) => c.method === "analyzeSuperpageSelection"
    );
    assert.strictEqual(analyzeCalls.length, 1);

    assert.strictEqual(host.getEvents("analysis_completed").length, 1);
    const renderCalls = host.renderCalls.filter((c) => c.type === "renderAnalysis");
    assert.strictEqual(renderCalls.length, 1);
  });

  it("uses lastSelection when called without arguments", async () => {
    const { plugin, runtimeClient } = createPlugin();
    await plugin.activate();
    plugin.onSelectionChanged(validSelection);
    await plugin.analyze();
    const analyzeCalls = runtimeClient.callLog.filter(
      (c) => c.method === "analyzeSuperpageSelection"
    );
    assert.strictEqual(analyzeCalls.length, 1);
    assert.deepStrictEqual(analyzeCalls[0].args[0], validSelection);
  });

  it("merges options in correct order", async () => {
    const { plugin, runtimeClient } = createPlugin({
      defaultAnalysisOptions: { include_conditions: false, include_priority: false },
    });
    await plugin.activate({ analysisOptions: { include_conditions: true } });
    await plugin.analyze(validSelection, { include_dataflow: true });
    const analyzeCalls = runtimeClient.callLog.filter(
      (c) => c.method === "analyzeSuperpageSelection"
    );
    const opts = analyzeCalls[0].args[1];
    assert.strictEqual(opts.include_conditions, true);
    assert.strictEqual(opts.include_priority, false);
    assert.strictEqual(opts.include_dataflow, true);
  });

  it("works with sync runtimeClient", async () => {
    const { plugin, host } = createPlugin({
      runtimeClient: createFakeRuntimeClient({ returnType: "sync" }),
    });
    await plugin.activate();
    const result = await plugin.analyze(validSelection);
    assert.strictEqual(result.status, "ready");
    assert.strictEqual(host.getEvents("analysis_completed").length, 1);
  });
});

describe("error cases", () => {
  it("returns PLUGIN_NOT_ACTIVATED when analyze called before activate", async () => {
    const { plugin } = createPlugin();
    const result = await plugin.analyze(validSelection);
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "PLUGIN_NOT_ACTIVATED");
  });

  it("returns INVALID_SELECTION when source_path missing", async () => {
    const { plugin } = createPlugin();
    const result = plugin.onSelectionChanged({ file_id: "f1" });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "INVALID_SELECTION");
  });

  it("returns INVALID_SELECTION for absolute path /foo/bar.spg", async () => {
    const { plugin } = createPlugin();
    const result = plugin.onSelectionChanged({ source_path: "/foo/bar.spg" });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "INVALID_SELECTION");
  });

  it("returns INVALID_SELECTION for URL https://example.com/a.spg", async () => {
    const { plugin } = createPlugin();
    const result = plugin.onSelectionChanged({
      source_path: "https://example.com/a.spg",
    });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "INVALID_SELECTION");
  });

  it("returns INVALID_SELECTION for path containing ..", async () => {
    const { plugin } = createPlugin();
    const result = plugin.onSelectionChanged({ source_path: "app/../secret.spg" });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "INVALID_SELECTION");
  });

  it("returns INVALID_SELECTION for Windows drive path C:\\foo.spg", async () => {
    const { plugin } = createPlugin();
    const result = plugin.onSelectionChanged({ source_path: "C:\\foo.spg" });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "INVALID_SELECTION");
  });

  it("returns RUNTIME_CLIENT_ERROR when analyze rejects", async () => {
    const { plugin, host } = createPlugin({
      runtimeClient: createFakeRuntimeClient({
        returnType: "promise",
        analyzeShouldReject: true,
      }),
    });
    await plugin.activate();
    const result = await plugin.analyze(validSelection);
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "RUNTIME_CLIENT_ERROR");
    assert.strictEqual(host.getEvents("analysis_failed").length, 1);
    const renderCalls = host.renderCalls.filter((c) => c.type === "renderError");
    assert.strictEqual(renderCalls.length, 1);
    assert.strictEqual(plugin.status().state, "error");
  });

  it("returns RUNTIME_CLIENT_ERROR when analyze throws synchronously", async () => {
    const { plugin, host } = createPlugin({
      runtimeClient: createFakeRuntimeClient({
        returnType: "sync",
        analyzeShouldThrow: true,
      }),
    });
    await plugin.activate();
    const result = await plugin.analyze(validSelection);
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "RUNTIME_CLIENT_ERROR");
    assert.strictEqual(host.getEvents("analysis_failed").length, 1);
  });
});

describe("static dependency check", () => {
  it("has no import statements referencing forbidden modules", () => {
    const source = readFileSync(pluginSourcePath, "utf-8");
    const lines = source.split("\n");
    for (const line of lines) {
      const trimmed = line.trim();
      if (trimmed.startsWith("import ") || trimmed.startsWith("export ")) {
        assert.strictEqual(
          trimmed.includes("runtime-launcher"),
          false,
          `Import must not reference runtime-launcher: ${trimmed}`
        );
        assert.strictEqual(
          trimmed.includes("platform-glue"),
          false,
          `Import must not reference platform-glue: ${trimmed}`
        );
        assert.strictEqual(
          trimmed.includes("service-worker"),
          false,
          `Import must not reference service-worker: ${trimmed}`
        );
      }
    }
  });

  it("has no calls to forbidden globals", () => {
    const source = readFileSync(pluginSourcePath, "utf-8");
    const callPatterns = [
      { pattern: /\bfetch\s*\(/, name: "fetch()" },
      { pattern: /\bwindow\.SZ\.rc\b/, name: "window.SZ.rc" },
      { pattern: /\bwindow\.SZ\.rc1\b/, name: "window.SZ.rc1" },
      { pattern: /\bdocument\.createElement\s*\(/, name: "document.createElement()" },
      { pattern: /\bdocument\.appendChild\s*\(/, name: "document.appendChild()" },
      { pattern: /\bnavigator\b/, name: "navigator" },
    ];
    for (const { pattern, name } of callPatterns) {
      assert.strictEqual(
        pattern.test(source),
        false,
        `Plugin source must not contain ${name}`
      );
    }
  });

  it("has no DOM manipulation strings", () => {
    const source = readFileSync(pluginSourcePath, "utf-8");
    const forbidden = ["innerHTML", "outerHTML", "insertAdjacentHTML"];
    for (const word of forbidden) {
      assert.strictEqual(
        source.includes(word),
        false,
        `Plugin source must not contain "${word}"`
      );
    }
  });
});

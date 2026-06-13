import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import test from "node:test";
import vm from "node:vm";

const ROOT = new URL("..", import.meta.url);

class FakeElement {
  constructor() {
    this.textContent = "";
    this.title = "";
    this.disabled = false;
    this.className = "";
    this.style = {};
    this.dataset = {};
    this._handlers = new Map();
    this.innerHTML = "";
  }

  addEventListener(type, handler) {
    const list = this._handlers.get(type) || [];
    list.push(handler);
    this._handlers.set(type, list);
  }

  dispatchEvent(event) {
    const list = this._handlers.get(event.type) || [];
    for (const handler of list) {
      handler(event);
    }
  }

  click() {
    this.dispatchEvent({ type: "click" });
  }
}

class FakeClipboard {
  constructor() {
    this.lastText = null;
  }

  async writeText(text) {
    this.lastText = text;
    return undefined;
  }
}

function createPopupContext({
  queryResult = [{ id: 1 }],
  sendMessage = async () => ({}),
  runtimeSendMessage = async () => ({ ok: true, state: {} }),
  navigatorOverrides = {},
} = {}) {
  const calls = [];
  const clipboard = new FakeClipboard();

  const fieldIds = [
    "session-status",
    "session-state",
    "runtime-status",
    "offscreen-status",
    "source-path",
    "source-file-id",
    "source-revision",
    "indexing-status",
    "indexing-progress",
    "indexing-progress-bar",
    "indexing-current-source",
    "cache-hits",
    "cache-misses",
    "artifact-readiness",
    "diagnostic",
    "action-message",
  ];

  const buttonIds = [
    "open-settings",
    "refresh-status",
    "sync-metadata",
    "pause-background",
    "resume-background",
    "copy-diagnostic",
  ];

  const nodes = {};
  for (const fieldId of fieldIds) {
    nodes[`[data-field="${fieldId}"]`] = new FakeElement();
  }
  for (const buttonId of buttonIds) {
    nodes[`[data-action="${buttonId}"]`] = new FakeElement();
  }

  const context = {
    console,
    setTimeout,
    clearTimeout,
    document: {
      querySelector(selector) {
        return nodes[selector] ?? null;
      },
      createElement() {
        return {};
      },
    },
    chrome: {
      tabs: {
        async query() {
          return queryResult;
        },
        async sendMessage(tabId, message) {
          calls.push({ tabId, message, source: "tabs.sendMessage" });
          if (typeof sendMessage === "function") {
            return sendMessage(tabId, message);
          }
          return {};
        },
      },
      runtime: {
        async sendMessage(message) {
          calls.push({ message, source: "runtime.sendMessage" });
          if (typeof runtimeSendMessage === "function") {
            return runtimeSendMessage(message);
          }
          return {};
        },
      },
    },
    navigator: {
      clipboard,
      ...navigatorOverrides,
    },
    __calls: calls,
    __clipboard: clipboard,
  };

  return context;
}

async function loadPopup(context) {
  const source = await readFile(join(ROOT.pathname, "extension-chromium", "popup.js"), "utf8");
  vm.runInNewContext(source, context, {
    filename: join(ROOT.pathname, "extension-chromium", "popup.js"),
  });
}

function flush() {
  return new Promise((resolve) => {
    setTimeout(resolve, 0);
  });
}

function parseDiagnostic(context) {
  const raw = context.document.querySelector('[data-field="diagnostic"]').textContent;
  return raw ? JSON.parse(raw) : null;
}

function popupStatusFixture({
  sourcePath = "projects/tenant/page.spg",
  fileId = "file-001",
  revision = "rev-1",
  selectedComponentIds = ["comp-a", "comp-b", "comp-c", "comp-d"],
  activeComponentId = "comp-a",
  sessionStatus = "ready",
  sessionBaseUrl = "https://autocrm-test.xiaoshouyi.com",
  visibleIndex = { status: "ready", analyzable_count: 3, files: [{ source_path: "projects/tenant/page.spg" }] },
  background = {
    status: "running",
    indexing_status: "indexing_current_page",
    processed: 2,
    total: 6,
    failed: 1,
    active: 1,
    current_source_path: "projects/tenant/page.spg",
    last_processed_source_path: "projects/tenant/page.spg",
  },
  cacheStats = { hits: 7, misses: 5 },
  runtime = { available: true },
  offscreen = { available: true },
  version = { value: "1.2.3" },
  lastDiagnostic = null,
} = {}) {
  const bridge = {
    ok: true,
    payload: {
      page_context: { source_path: sourcePath, file_id: fileId, revision },
      selection: {
        source_path: sourcePath,
        file_id: fileId,
        revision,
        selected_component_ids: selectedComponentIds,
        active_component_id: activeComponentId,
      },
      diagnostics: lastDiagnostic ? [lastDiagnostic] : [],
    },
    page_context: { source_path: sourcePath, file_id: fileId, revision },
    selection: {
      source_path: sourcePath,
      file_id: fileId,
      revision,
      selected_component_ids: selectedComponentIds,
      active_component_id: activeComponentId,
    },
  };
  const state = {
    session: { status: sessionStatus, base_url: sessionBaseUrl },
    visible_index: visibleIndex,
    background,
    cache_stats: cacheStats,
    runtime,
    offscreen,
    version,
    last_diagnostic: lastDiagnostic || null,
    updated_at: 1700000000000,
  };
  return { bridge, state };
}

test("chromium popup renders global status fields and hides current-selection analyze controls", async () => {
  const context = createPopupContext({
    sendMessage: async () => popupStatusFixture().bridge,
    runtimeSendMessage: async (message) => {
      if (message.type === "metadata-checker-popup-status") {
        return popupStatusFixture().state;
      }
      if (message.type === "metadata-checker-selection-changed") {
        return {
          ok: true,
          artifact_ready: true,
          artifact: {
            analysis_status: "ready",
          },
          background: { last_processed_source_path: "projects/tenant/page.spg" },
        };
      }
      return { ok: true };
    },
  });
  await loadPopup(context);
  await flush();

  assert.equal(context.document.querySelector('[data-field="session-status"]').textContent, "ready");
  assert.equal(context.document.querySelector('[data-field="runtime-status"]').textContent, "ready");
  assert.equal(context.document.querySelector('[data-field="offscreen-status"]').textContent, "ready");
  assert.match(context.document.querySelector('[data-field="session-state"]').textContent, /v1/);
  assert.equal(
    context.document.querySelector('[data-field="source-path"]').textContent,
    "projects/tenant/page.spg",
  );
  assert.equal(context.document.querySelector('[data-action="process-current"]'), null);
  assert.equal(context.document.querySelector('[data-action="retry-current-selection"]'), null);
  const diag = parseDiagnostic(context);
  assert.equal(diag.code, "METADATA_CHECKER_IDLE");
});

test("chromium popup source stays a settings and status surface, not graph analysis UI", async () => {
  const [html, script] = await Promise.all([
    readFile(join(ROOT.pathname, "extension-chromium", "popup.html"), "utf8"),
    readFile(join(ROOT.pathname, "extension-chromium", "popup.js"), "utf8"),
  ]);
  const source = `${html}\n${script}`;

  assert.doesNotMatch(source, /data-action=["']process-current["']/);
  assert.doesNotMatch(source, /data-action=["']retry-current-selection["']/);
  assert.doesNotMatch(source, /Analyze current selection/i);
  assert.doesNotMatch(source, /Graph quick/i);
  assert.doesNotMatch(source, /metadata-checker-graph-panel|graph-surface|pixi/i);
});

test("chromium popup shows indexing status and progress ratio", async () => {
  const status = popupStatusFixture({
    sourcePath: "projects/tenant/index.spg",
    selectedComponentIds: [],
    background: {
      status: "running",
      indexing_status: "running",
      processed: 1,
      total: 4,
      failed: 0,
      active: 3,
    },
    visibleIndex: { status: "ready", analyzable_count: 8, files: [] },
    cacheStats: { hits: 0, misses: 9 },
    runtime: { available: true },
    offscreen: { available: false },
  });
  const context = createPopupContext({
    sendMessage: async () => status.bridge,
    runtimeSendMessage: async (message) => {
      if (message.type === "metadata-checker-popup-status") {
        return status.state;
      }
      if (message.type === "metadata-checker-selection-changed") {
        return {
          ok: true,
          artifact_ready: false,
        };
      }
      return { ok: true };
    },
  });
  await loadPopup(context);
  await flush();

  assert.equal(context.document.querySelector('[data-field="indexing-progress"]').textContent, "1/4 processed, 0 failed, 8 discovered, 3 active");
  const width = context.document.querySelector('[data-field="indexing-progress-bar"]').style.width;
  assert.equal(width, "25%");
});

test("chromium popup surfaces active tab missing diagnostic", async () => {
  const context = createPopupContext({
    queryResult: [],
    sendMessage: async () => popupStatusFixture().bridge,
    runtimeSendMessage: async (message) => {
      if (message.type === "metadata-checker-popup-status") {
        return popupStatusFixture().state;
      }
      if (message.type === "metadata-checker-selection-changed") {
        return { ok: true, artifact_ready: false };
      }
      return { ok: true };
    },
  });
  await loadPopup(context);
  await flush();

  const diag = parseDiagnostic(context);
  assert.equal(diag.code, "METADATA_CHECKER_ACTIVE_TAB_MISSING");
});

test("chromium popup truncates long source path with tooltip", async () => {
  const longPath = "projects/enterprise/apps/autocrm/prod/modules/marketing/campaigns/2026/rev-very-long-source-path/page-home-overview-dashboard.spg";
  const context = createPopupContext({
    sendMessage: async () => popupStatusFixture({ sourcePath: longPath }).bridge,
    runtimeSendMessage: async (message) => {
      if (message.type === "metadata-checker-popup-status") {
        return popupStatusFixture({ sourcePath: longPath }).state;
      }
      if (message.type === "metadata-checker-selection-changed") {
        return {
          ok: true,
          artifact_ready: false,
        };
      }
      return { ok: true };
    },
  });
  await loadPopup(context);
  await flush();

  const sourcePathNode = context.document.querySelector('[data-field="source-path"]');
  assert.notEqual(sourcePathNode.textContent, longPath);
  assert.match(sourcePathNode.textContent, /…/);
  assert.equal(sourcePathNode.title, longPath);
});

test("chromium popup executes global actions and shows button messages", async () => {
  const context = createPopupContext({
    sendMessage: async (_tabId, message) => ({ ok: true, payload: {}, action: message.request_type }),
    runtimeSendMessage: async (message) => {
      if (message.type === "metadata-checker-popup-status") {
        return popupStatusFixture({
          lastDiagnostic: { code: "METADATA_CHECKER_SESSION_OK", message: "session ready" },
        }).state;
      }
      if (message.type === "metadata-checker-selection-changed") {
        return { ok: true, artifact_ready: false };
      }
      return { ok: true };
    },
  });

  await loadPopup(context);
  await flush();

  const actionMessage = context.document.querySelector('[data-field="action-message"]');
  const openSettings = context.document.querySelector('[data-action="open-settings"]');
  const refresh = context.document.querySelector('[data-action="refresh-status"]');
  const sync = context.document.querySelector('[data-action="sync-metadata"]');
  const pause = context.document.querySelector('[data-action="pause-background"]');
  const resume = context.document.querySelector('[data-action="resume-background"]');

  openSettings.click();
  assert.equal(actionMessage.textContent, "open settings");
  await flush();
  assert.equal(context.__calls.some((item) => item.source === "tabs.sendMessage" && item.message.request_type === "openPanel"), true);

  refresh.click();
  assert.equal(actionMessage.textContent, "refresh status");
  await flush();

  sync.click();
  await flush();
  assert.equal(context.__calls.some((item) => item.source === "runtime.sendMessage" && item.message.type === "metadata-checker-background-process"), true);

  pause.click();
  await flush();
  assert.equal(context.__calls.some((item) => item.source === "runtime.sendMessage" && item.message.type === "metadata-checker-background-pause"), true);

  resume.click();
  await flush();
  assert.equal(context.__calls.some((item) => item.source === "runtime.sendMessage" && item.message.type === "metadata-checker-background-resume"), true);
});

test("chromium popup copies redacted diagnostic", async () => {
  const context = createPopupContext({
    runtimeSendMessage: async (message) => {
      if (message.type === "metadata-checker-popup-status") {
        return popupStatusFixture({
          lastDiagnostic: { code: "TOKEN_LEAK_CHECK", message: "token=secret-token cookie=secret-cookie password=secret-password" },
        }).state;
      }
      if (message.type === "metadata-checker-selection-changed") {
        return {
          ok: true,
          artifact_ready: false,
        };
      }
      return { ok: true };
    },
    sendMessage: async () => popupStatusFixture().bridge,
  });

  await loadPopup(context);
  await flush();

  const copyDiagnostic = context.document.querySelector('[data-action="copy-diagnostic"]');
  copyDiagnostic.click();
  await flush();

  const copied = context.__clipboard.lastText;
  assert.ok(typeof copied === "string");
  assert.equal(copied.includes("secret-token"), false);
  assert.equal(copied.includes("secret-cookie"), false);
  assert.equal(copied.includes("secret-password"), false);
  assert.equal(copied.includes("***"), true);
  const actionMessage = context.document.querySelector('[data-field="action-message"]');
  assert.equal(actionMessage.textContent, "diagnostic copied");
});

test("chromium popup marks button failure with stable diagnostic", async () => {
  const context = createPopupContext({
    sendMessage: async () => popupStatusFixture().bridge,
    runtimeSendMessage: async (message) => {
      if (message.type === "metadata-checker-popup-status") {
        return popupStatusFixture({}).state;
      }
      if (message.type === "metadata-checker-selection-changed") {
        return { ok: true, artifact_ready: false };
      }
      if (message.type === "metadata-checker-background-process") {
        throw new Error("sync failed");
      }
      return { ok: true };
    },
  });
  await loadPopup(context);
  await flush();

  const sync = context.document.querySelector('[data-action="sync-metadata"]');
  sync.click();
  await flush();

  const diag = parseDiagnostic(context);
  assert.equal(diag.code, "METADATA_CHECKER_ACTION_FAILED");
  assert.match(context.document.querySelector('[data-field="action-message"]').textContent, /METADATA_CHECKER_ACTION_FAILED/);
});

test("chromium popup uses plain text for button error labels", async () => {
  const maliciousCode = "<img src=x onerror=alert(1)>";
  const context = createPopupContext({
    sendMessage: async () => popupStatusFixture().bridge,
    runtimeSendMessage: async (message) => {
      if (message.type === "metadata-checker-popup-status") {
        return popupStatusFixture({}).state;
      }
      if (message.type === "metadata-checker-selection-changed") {
        return { ok: true, artifact_ready: false };
      }
      if (message.type === "metadata-checker-background-process") {
        throw { code: maliciousCode, message: "fail" };
      }
      return { ok: true };
    },
  });

  await loadPopup(context);
  await flush();

  const sync = context.document.querySelector('[data-action="sync-metadata"]');
  sync.click();
  await flush();

  assert.equal(sync.textContent, maliciousCode);
  assert.equal(sync.innerHTML.includes("<img"), false);
  assert.equal(parseDiagnostic(context)?.code, maliciousCode);
  assert.match(context.document.querySelector('[data-field="action-message"]').textContent, new RegExp(maliciousCode.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
});

test("chromium popup does not use chrome.scripting.executeScript", async () => {
  const source = await readFile(join(ROOT.pathname, "extension-chromium", "popup.js"), "utf8");
  assert.doesNotMatch(source, /chrome\\.scripting\\.executeScript/);
});

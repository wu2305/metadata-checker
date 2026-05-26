import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import test from "node:test";
import vm from "node:vm";

const ROOT = new URL("..", import.meta.url);

class FakeElement {
  constructor() {
    this.textContent = "";
    this._handlers = new Map();
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

function createPopupContext({
  queryResult = [{ id: 1 }],
  sendMessage = async () => ({}),
} = {}) {
  const calls = [];

  const nodes = {
    '[data-field="bridge"]': new FakeElement(),
    '[data-field="source"]': new FakeElement(),
    '[data-field="selection"]': new FakeElement(),
    '[data-field="diagnostic"]': new FakeElement(),
    '[data-action="open-panel"]': new FakeElement(),
    '[data-action="hide-panel"]': new FakeElement(),
    '[data-action="refresh-bridge"]': new FakeElement(),
    '[data-action="analyze"]': new FakeElement(),
  };

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
          calls.push({ tabId, message });
          return sendMessage(tabId, message);
        },
      },
    },
    __calls: calls,
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

function assertDiagnostic(context, expectedCode) {
  const raw = context.document.querySelector('[data-field="diagnostic"]').textContent;
  if (!raw) {
    assert.fail("diagnostic field is empty");
  }
  const parsed = JSON.parse(raw);
  assert.equal(parsed.code, expectedCode);
}

test("chromium popup sends open/hide/refresh/analyze requests via chrome.tabs.sendMessage", async () => {
  const context = createPopupContext();
  await loadPopup(context);
  await flush();

  context.document.querySelector('[data-action="open-panel"]').click();
  context.document.querySelector('[data-action="hide-panel"]').click();
  context.document.querySelector('[data-action="refresh-bridge"]').click();
  context.document.querySelector('[data-action="analyze"]').click();
  await flush();

  const requestTypes = context.__calls.map((item) => item.message.request_type);
  assert.equal(requestTypes.length, 5);
  assert.deepEqual(requestTypes.slice(1), [
    "openPanel",
    "hidePanel",
    "refreshBridge",
    "analyzeCurrentSelection",
  ]);
});

test("chromium popup shows stable diagnostic when active tab is missing", async () => {
  const context = createPopupContext({
    queryResult: [],
    sendMessage: async () => {
      throw new Error("should not be called");
    },
  });
  await loadPopup(context);
  await flush();
  assert.equal(context.__calls.length, 0);
  assertDiagnostic(context, "METADATA_CHECKER_ACTIVE_TAB_MISSING");
});

test("chromium popup handles sendMessage failure with stable diagnostic", async () => {
  const errors = [];
  const onRejection = (error) => {
    errors.push(error);
  };
  process.on("unhandledRejection", onRejection);
  try {
    const context = createPopupContext({
      sendMessage: async (_tabId, message) => {
        if (message.request_type === "analyzeCurrentSelection") {
          throw new Error("content bridge send failed");
        }
        return { diagnostics: [] };
      },
    });
    await loadPopup(context);
    await flush();

    context.document.querySelector('[data-action="analyze"]').click();
    await flush();
    assertDiagnostic(context, "METADATA_CHECKER_POPUP_ANALYZE_FAILED");
    assert.equal(context.__calls.some((item) => item.message.request_type === "analyzeCurrentSelection"), true);
    assert.equal(errors.length, 0);
  } finally {
    process.off("unhandledRejection", onRejection);
  }
});

test("chromium popup shows bridge missing diagnostic when content script returns diagnostics", async () => {
  const context = createPopupContext({
    sendMessage: async () => ({
      diagnostics: [
        {
          severity: "warning",
          code: "METADATA_CHECKER_BRIDGE_MISSING",
          message: "bridge missing",
        },
      ],
    }),
  });
  await loadPopup(context);
  await flush();

  context.document.querySelector('[data-action="refresh-bridge"]').click();
  await flush();
  assertDiagnostic(context, "METADATA_CHECKER_BRIDGE_MISSING");
});

test("chromium popup does not use chrome.scripting.executeScript", async () => {
  const source = await readFile(join(ROOT.pathname, "extension-chromium", "popup.js"), "utf8");
  assert.doesNotMatch(source, /chrome\.scripting\.executeScript/);
});

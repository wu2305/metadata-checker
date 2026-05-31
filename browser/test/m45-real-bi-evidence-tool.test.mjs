import assert from "node:assert/strict";
import test from "node:test";

import {
  parseArgs,
  redact,
  selectPageTarget,
} from "../tools/m45-real-bi-collect-evidence.mjs";

test("M45 real BI evidence tool parses reproducible collection options", () => {
  const args = parseArgs([
    "--debugging-url",
    "http://127.0.0.1:9333",
    "--page-url-contains",
    "autocrm-test.xiaoshouyi.com",
    "--out-dir",
    "browser/artifacts/m45-real-bi-evidence",
    "--observe-ms",
    "100",
  ]);

  assert.equal(args.debuggingUrl, "http://127.0.0.1:9333");
  assert.equal(args.pageUrlContains, "autocrm-test.xiaoshouyi.com");
  assert.equal(args.outDir, "browser/artifacts/m45-real-bi-evidence");
  assert.equal(args.observeMs, 100);
});

test("M45 real BI evidence tool selects the authenticated BI page target", () => {
  const target = selectPageTarget(
    [
      { type: "service_worker", url: "chrome-extension://id/background.js" },
      { type: "page", url: "https://example.test", webSocketDebuggerUrl: "ws://example" },
      {
        type: "page",
        url: "https://autocrm-test.xiaoshouyi.com/xiaoshouyi/app/%E4%BB%B7%E5%AE%A1.app",
        webSocketDebuggerUrl: "ws://bi",
      },
    ],
    "autocrm-test.xiaoshouyi.com",
  );

  assert.equal(target.webSocketDebuggerUrl, "ws://bi");
});

test("M45 real BI evidence tool redacts sensitive console and marker values", () => {
  const result = redact({
    page_snapshot: {
      markers: {
        "data-metadata-checker-runtime": "ready token=secret-token",
      },
    },
    console_events: [{ args: ["password=secret-password"] }],
  });

  const serialized = JSON.stringify(result);
  assert.equal(serialized.includes("secret-token"), false);
  assert.equal(serialized.includes("secret-password"), false);
  assert.match(serialized, /token=<redacted>/);
  assert.match(serialized, /password=<redacted>/);
});

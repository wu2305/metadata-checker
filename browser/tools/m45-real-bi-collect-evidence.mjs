import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const DEFAULT_DEBUGGING_URL = "http://127.0.0.1:9222";
const DEFAULT_PAGE_URL_CONTAINS = "autocrm-test.xiaoshouyi.com";
const DEFAULT_OUT_DIR = "browser/artifacts/m45-real-bi-evidence";
const SENSITIVE_PAIR_PATTERN =
  /\b(token|cookie|password|secret|auth|credential|cipherpassport)\b\s*[:=]\s*[^&\s,;}"']+/gi;

function parseArgs(argv) {
  const args = {
    debuggingUrl: DEFAULT_DEBUGGING_URL,
    pageUrlContains: DEFAULT_PAGE_URL_CONTAINS,
    outDir: DEFAULT_OUT_DIR,
    observeMs: 0,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (!token.startsWith("--")) {
      throw new Error(`unexpected argument: ${token}`);
    }
    const value = argv[index + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new Error(`missing value for ${token}`);
    }
    if (token === "--debugging-url") {
      args.debuggingUrl = value;
    } else if (token === "--page-url-contains") {
      args.pageUrlContains = value;
    } else if (token === "--out-dir") {
      args.outDir = value;
    } else if (token === "--observe-ms") {
      args.observeMs = Number(value);
      if (!Number.isFinite(args.observeMs) || args.observeMs < 0) {
        throw new Error("--observe-ms must be a non-negative number");
      }
    } else {
      throw new Error(`unknown argument: ${token}`);
    }
    index += 1;
  }
  return args;
}

function redact(value) {
  if (typeof value === "string") {
    return value.replace(SENSITIVE_PAIR_PATTERN, "$1=<redacted>");
  }
  if (Array.isArray(value)) {
    return value.map((item) => redact(item));
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [key, redact(item)]),
    );
  }
  return value;
}

async function listTargets(debuggingUrl) {
  const response = await fetch(`${debuggingUrl.replace(/\/$/, "")}/json/list`);
  if (!response.ok) {
    throw new Error(`CDP target list failed: HTTP ${response.status}`);
  }
  return response.json();
}

function selectPageTarget(targets, pageUrlContains) {
  const page = targets.find(
    (target) =>
      target.type === "page" &&
      typeof target.url === "string" &&
      target.url.includes(pageUrlContains) &&
      target.webSocketDebuggerUrl,
  );
  if (!page) {
    throw new Error(`cannot find page target containing: ${pageUrlContains}`);
  }
  return page;
}

function createCdpClient(webSocketDebuggerUrl) {
  const socket = new WebSocket(webSocketDebuggerUrl);
  let nextId = 1;
  const pending = new Map();
  const events = [];

  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) {
        reject(new Error(message.error.message || "CDP command failed"));
      } else {
        resolve(message.result || {});
      }
      return;
    }
    if (message.method === "Runtime.consoleAPICalled") {
      events.push({
        method: message.method,
        type: message.params?.type,
        args: (message.params?.args || []).map((arg) => arg.value ?? arg.description ?? ""),
      });
    } else if (message.method === "Runtime.exceptionThrown") {
      events.push({
        method: message.method,
        text: message.params?.exceptionDetails?.text || "",
      });
    }
  });

  return {
    async ready() {
      if (socket.readyState === WebSocket.OPEN) {
        return;
      }
      await new Promise((resolve, reject) => {
        socket.addEventListener("open", resolve, { once: true });
        socket.addEventListener("error", reject, { once: true });
      });
    },
    send(method, params = {}) {
      const id = nextId;
      nextId += 1;
      socket.send(JSON.stringify({ id, method, params }));
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
      });
    },
    events,
    close() {
      socket.close();
    },
  };
}

async function collectPageSnapshot(client, observeMs) {
  await client.send("Runtime.enable");
  await client.send("Page.enable");
  if (observeMs > 0) {
    await new Promise((resolve) => setTimeout(resolve, observeMs));
  }
  const snapshot = await client.send("Runtime.evaluate", {
    returnByValue: true,
    expression: `(() => {
      const root = document.documentElement;
      const markers = {};
      for (const name of root.getAttributeNames()) {
        if (name.startsWith("data-metadata-checker")) {
          markers[name] = root.getAttribute(name);
        }
      }
      const panelTexts = [];
      for (const element of document.querySelectorAll("*")) {
        if (element.shadowRoot) {
          const text = element.shadowRoot.innerText || element.shadowRoot.textContent || "";
          if (/metadata checker|background progress|cache hits|retry/i.test(text)) {
            panelTexts.push(text.trim().replace(/\\s+/g, " ").slice(0, 4000));
          }
        }
      }
      return {
        url: location.href,
        title: document.title,
        markers,
        panel_text: panelTexts.join("\\n---\\n"),
        collected_at: new Date().toISOString(),
      };
    })()`,
  });
  const screenshot = await client.send("Page.captureScreenshot", {
    format: "png",
    captureBeyondViewport: false,
  });
  return {
    snapshot: snapshot.result?.value || {},
    screenshotBase64: screenshot.data,
  };
}

async function collectEvidence(options = {}) {
  const targets = await listTargets(options.debuggingUrl);
  const pageTarget = selectPageTarget(targets, options.pageUrlContains);
  const extensionTargets = targets
    .filter((target) =>
      target.type === "service_worker" ||
      target.type === "background_page" ||
      String(target.url || "").startsWith("chrome-extension://"))
    .map((target) => ({
      type: target.type,
      title: target.title,
      url: target.url,
    }));

  const client = createCdpClient(pageTarget.webSocketDebuggerUrl);
  await client.ready();
  try {
    const { snapshot, screenshotBase64 } = await collectPageSnapshot(client, options.observeMs);
    await mkdir(options.outDir, { recursive: true });
    const screenshotPath = join(options.outDir, "m45-real-bi-panel.png");
    const evidencePath = join(options.outDir, "m45-real-bi-evidence.json");
    await writeFile(screenshotPath, Buffer.from(screenshotBase64, "base64"));
    const evidence = redact({
      page_target: {
        type: pageTarget.type,
        title: pageTarget.title,
        url: pageTarget.url,
      },
      extension_targets: extensionTargets,
      page_snapshot: snapshot,
      console_events: client.events,
      screenshot_path: screenshotPath,
    });
    await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, "utf8");
    return {
      evidence_path: evidencePath,
      screenshot_path: screenshotPath,
      marker_count: Object.keys(evidence.page_snapshot.markers || {}).length,
      extension_target_count: extensionTargets.length,
    };
  } finally {
    client.close();
  }
}

async function main(argv = process.argv.slice(2)) {
  const result = await collectEvidence(parseArgs(argv));
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}

export {
  collectEvidence,
  parseArgs,
  redact,
  selectPageTarget,
};

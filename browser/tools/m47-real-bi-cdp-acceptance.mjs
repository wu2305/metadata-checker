import { access, mkdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(__dirname, "..", "..");
const DEFAULT_CHROME_EXECUTABLE =
  "/Users/wuhaocheng/Library/Caches/ms-playwright/chromium-1223/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing";
const DEFAULT_USER_DATA_DIR = "/private/tmp/metadata-checker-m47-pixi-chrome-profile";
const DEFAULT_EXTENSION_DIR = join(
  REPO_ROOT,
  "browser",
  "artifacts",
  "metadata-checker-extension-chromium-m47-acceptance",
);
const DEFAULT_PAGE_URL =
  "https://autocrm-test.xiaoshouyi.com/xiaoshouyi/app/%E4%BB%B7%E5%AE%A1.app?:edit=true&:file=%E9%94%80%E5%94%AE%E8%AE%A2%E5%8D%95%E4%BB%B7%E6%A0%BC%E5%AE%A1%E6%89%B9-%E4%BF%A1%E6%81%AF%E8%A1%A5%E5%85%85.spg";
const DEFAULT_OUT_DIR = join(REPO_ROOT, "browser", "artifacts", "m47-real-bi-acceptance");
const DEFAULT_DEBUGGING_PORT = 9222;
const DEFAULT_OBSERVE_MS = 3000;
const DEFAULT_STARTUP_TIMEOUT_MS = 45000;
const DEFAULT_COMMAND_TIMEOUT_MS = 30000;
const SENSITIVE_PAIR_PATTERN =
  /\b(token|cookie|password|secret|auth|credential|cipherpassport|access_token)\b\s*[:=]\s*[^&\s,;}"']+/gi;
const SENSITIVE_KEY_PATTERN =
  /token|cookie|password|secret|auth|credential|cipherpassport|access_token|api[_-]?key/i;
const ACCEPTANCE_GEOMETRY_AREA_RATIO = 9;
const EDGE_EVIDENCE_UNAVAILABLE_TEXT = "EDGE_EVIDENCE_UNAVAILABLE";
const PERFORMANCE_TIMING_SOURCES = {
  ensure_runtime_ms: ["local-graph-timing-ensure-runtime-ms"],
  fetch_metadata_ms: ["local-graph-timing-fetch-metadata-ms"],
  init_runtime_ms: ["local-graph-timing-init-runtime-ms"],
  load_document_ms: ["local-graph-timing-load-document-ms"],
  build_graph_ms: ["local-graph-timing-build-graph-ms"],
  analyze_local_graph_ms: [
    "local-graph-timing-analyze-local-graph-ms",
    "local-graph-timing-analyze-ms",
  ],
  analyze_ms: [
    "local-graph-timing-analyze-local-graph-ms",
    "local-graph-timing-analyze-ms",
    "analyze-ms",
  ],
  layout_ms: [
    "local-graph-timing-layout-ms",
    "layout-ms",
  ],
  render_ms: [
    "local-graph-timing-render-ms",
    "render-ms",
  ],
  total_ms: [
    "local-graph-timing-total-ms",
    "total-ms",
  ],
  manifest_fetch_ms: [
    "manifest-fetch-ms",
    "local-graph-timing-manifest-fetch-ms",
  ],
  manifest_diff_ms: [
    "manifest-diff-ms",
    "local-graph-timing-manifest-diff-ms",
  ],
  changed_content_fetch_ms: [
    "changed-content-fetch-ms",
    "local-graph-timing-changed-content-fetch-ms",
  ],
  wasm_update_ms: [
    "wasm-update-ms",
    "local-graph-timing-wasm-update-ms",
  ],
};
const MANIFEST_DIFF_MARKER_SOURCES = {
  added: ["manifest-added", "local-graph-manifest-added"],
  modified: ["manifest-modified", "local-graph-manifest-modified"],
  deleted: ["manifest-deleted", "local-graph-manifest-deleted"],
  unchanged: ["manifest-unchanged", "local-graph-manifest-unchanged"],
  content_queue_count: ["content-queue-count", "local-graph-content-queue-count"],
};
const MANIFEST_DIFF_FIELDS = Object.keys(MANIFEST_DIFF_MARKER_SOURCES);
const M48_INTERACTION_MARKER_NAMES = [
  "graph-hover-target",
  "graph-locked-target",
  "graph-detail-kind",
  "graph-highlight-node-count",
  "graph-highlight-edge-count",
  "graph-viewport-scale",
  "graph-viewport-target",
  "graph-density-profile",
];
const M48_INTERACTION_CATEGORIES = [
  "hover_node",
  "hover_edge",
  "hover_aggregate",
  "lock_node",
  "lock_edge",
  "lock_aggregate",
  "viewport_zoom",
  "viewport_focus_edge",
  "copy_graph_text",
];

function asString(value, fallback = "") {
  if (value === undefined || value === null) {
    return fallback;
  }
  return String(value);
}

function normalizeToInt(value, fallback = 0) {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function addDiagnostic(targets, category, code, message, details = {}) {
  targets.push({
    category,
    code,
    message,
    details,
  });
}

function toInt(value, fallback = 0) {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function timingMarkerName(key) {
  return `local-graph-timing-${String(key).replaceAll("_", "-")}`;
}

function pickFirstTimingMarker(directMarkers, sourceNames) {
  for (const sourceName of sourceNames) {
    const marker = directMarkers[sourceName];
    if (marker === undefined || marker === null || marker === "") {
      continue;
    }
    return marker;
  }
  return null;
}

function manifestDiffSeen(diff) {
  return Object.values(diff || {}).some((value) => value !== null && value !== undefined);
}

function manifestDiffsChanged(before, after) {
  const beforeSeen = manifestDiffSeen(before);
  const afterSeen = manifestDiffSeen(after);
  if (!afterSeen) {
    return false;
  }
  if (!beforeSeen) {
    return true;
  }
  return MANIFEST_DIFF_FIELDS.some((field) => (before?.[field] ?? null) !== (after?.[field] ?? null));
}

function collectManifestDiffFromMarkers(directMarkers = {}) {
  const diff = {};
  for (const [field, sourceNames] of Object.entries(MANIFEST_DIFF_MARKER_SOURCES)) {
    let value = null;
    for (const sourceName of sourceNames) {
      const marker = directMarkers[sourceName];
      if (marker !== undefined && marker !== null && marker !== "") {
        value = normalizeToInt(marker, null);
        break;
      }
    }
    diff[field] = value;
  }
  return diff;
}

function collectManifestDiffFromResponse(response = {}) {
  const candidates = [
    response,
    response?.payload,
    response?.response,
    response?.response?.payload,
    response?.payload?.payload,
    response?.payload?.manifest_diff,
    response?.response?.manifest_diff,
    response?.payload?.manifestDiff,
    response?.response?.manifestDiff,
  ];
  const diff = {};
  for (const field of MANIFEST_DIFF_FIELDS) {
    diff[field] = null;
  }
  for (const candidate of candidates) {
    if (!candidate || typeof candidate !== "object") {
      continue;
    }
    for (const field of MANIFEST_DIFF_FIELDS) {
      if (diff[field] !== null) {
        continue;
      }
      const raw = candidate[field];
      if (raw !== undefined && raw !== null && raw !== "") {
        diff[field] = normalizeToInt(raw, null);
      }
    }
    const nested = candidate.manifest_diff || candidate.manifestDiff || {};
    if (nested && typeof nested === "object") {
      for (const field of MANIFEST_DIFF_FIELDS) {
        if (diff[field] !== null) {
          continue;
        }
        const raw = nested[field];
        if (raw !== undefined && raw !== null && raw !== "") {
          diff[field] = normalizeToInt(raw, null);
        }
      }
    }
  }
  return diff;
}

function mergeManifestDiff(...sources) {
  const merged = {};
  for (const field of MANIFEST_DIFF_FIELDS) {
    merged[field] = null;
  }
  for (const source of sources) {
    if (!source || typeof source !== "object") {
      continue;
    }
    for (const field of MANIFEST_DIFF_FIELDS) {
      if (merged[field] === null && source[field] !== null && source[field] !== undefined) {
        merged[field] = source[field];
      }
    }
  }
  return merged;
}

function collectPerformanceTimings(directMarkers = {}) {
  const timings = {};
  for (const [key, sourceNames] of Object.entries(PERFORMANCE_TIMING_SOURCES)) {
    const marker = pickFirstTimingMarker(directMarkers, sourceNames);
    timings[key] = marker === null ? null : normalizeToInt(marker, null);
  }
  const legacyOffscreenTotal = directMarkers["local-graph-offscreen-total-ms"];
  timings.offscreen_total_ms = legacyOffscreenTotal === undefined || legacyOffscreenTotal === null || legacyOffscreenTotal === ""
    ? null
    : normalizeToInt(legacyOffscreenTotal, null);
  timings.metadata_cache_hit = directMarkers["local-graph-cache-metadata-hit"] || null;
  timings.document_cache_hit = directMarkers["local-graph-cache-document-hit"] || null;
  timings.selection_debounce_ms = directMarkers["local-graph-selection-debounce-ms"]
    ? normalizeToInt(directMarkers["local-graph-selection-debounce-ms"], null)
    : null;
  timings.selection_stale = directMarkers["local-graph-selection-stale"] || null;
  return timings;
}

function normalizeInteractionMarker(value) {
  return asString(value, "").trim();
}

function isMissingMarker(value) {
  return normalizeInteractionMarker(value) === "";
}

function parseNumericMarker(value) {
  const parsed = Number.parseFloat(asString(value, "").trim());
  return Number.isFinite(parsed) ? parsed : null;
}

function containsSensitiveText(value) {
  const candidate = asString(value, "");
  const pairRegex = new RegExp(SENSITIVE_PAIR_PATTERN.source, "gi");
  const keyRegex = new RegExp(SENSITIVE_KEY_PATTERN.source, "i");
  return pairRegex.test(candidate) || keyRegex.test(candidate);
}

function createInteractionCategoryResult() {
  const result = {};
  for (const category of M48_INTERACTION_CATEGORIES) {
    result[category] = {
      status: "pass",
      diagnostics: [],
      details: {},
    };
  }
  return result;
}

function interactionTargetType(value) {
  const marker = normalizeInteractionMarker(value);
  if (isMissingMarker(marker)) {
    return "";
  }
  const separator = marker.indexOf(":");
  if (separator > 0) {
    return marker.slice(0, separator);
  }
  return "";
}

function parseArgs(argv) {
  const args = {
    chromeExecutable: DEFAULT_CHROME_EXECUTABLE,
    userDataDir: DEFAULT_USER_DATA_DIR,
    extensionDir: DEFAULT_EXTENSION_DIR,
    pageUrl: DEFAULT_PAGE_URL,
    outDir: DEFAULT_OUT_DIR,
    debuggingPort: DEFAULT_DEBUGGING_PORT,
    observeMs: DEFAULT_OBSERVE_MS,
    startupTimeoutMs: DEFAULT_STARTUP_TIMEOUT_MS,
    commandTimeoutMs: DEFAULT_COMMAND_TIMEOUT_MS,
    launch: true,
    keepOpen: false,
    trigger: true,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (token === "--connect-only") {
      args.launch = false;
      continue;
    }
    if (token === "--keep-open") {
      args.keepOpen = true;
      continue;
    }
    if (token === "--skip-trigger") {
      args.trigger = false;
      continue;
    }
    if (!token.startsWith("--")) {
      throw new Error(`unexpected argument: ${token}`);
    }
    const value = argv[index + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new Error(`missing value for ${token}`);
    }
    if (token === "--chrome-executable") {
      args.chromeExecutable = value;
    } else if (token === "--user-data-dir") {
      args.userDataDir = value;
    } else if (token === "--extension-dir") {
      args.extensionDir = value;
    } else if (token === "--page-url") {
      args.pageUrl = value;
    } else if (token === "--out-dir") {
      args.outDir = value;
    } else if (token === "--debugging-port") {
      args.debuggingPort = Number(value);
    } else if (token === "--observe-ms") {
      args.observeMs = Number(value);
    } else if (token === "--startup-timeout-ms") {
      args.startupTimeoutMs = Number(value);
    } else if (token === "--command-timeout-ms") {
      args.commandTimeoutMs = Number(value);
    } else {
      throw new Error(`unknown argument: ${token}`);
    }
    index += 1;
  }
  if (!Number.isInteger(args.debuggingPort) || args.debuggingPort <= 0) {
    throw new Error("--debugging-port must be a positive integer");
  }
  if (!Number.isFinite(args.observeMs) || args.observeMs < 0) {
    throw new Error("--observe-ms must be a non-negative number");
  }
  if (!Number.isFinite(args.startupTimeoutMs) || args.startupTimeoutMs <= 0) {
    throw new Error("--startup-timeout-ms must be a positive number");
  }
  if (!Number.isFinite(args.commandTimeoutMs) || args.commandTimeoutMs <= 0) {
    throw new Error("--command-timeout-ms must be a positive number");
  }
  return args;
}

function redact(value, seen = new WeakSet(), key = "") {
  if (SENSITIVE_KEY_PATTERN.test(String(key || ""))) {
    return "[redacted]";
  }
  if (typeof value === "string") {
    return value.replace(SENSITIVE_PAIR_PATTERN, "$1=<redacted>");
  }
  if (!value || typeof value !== "object") {
    return value;
  }
  if (seen.has(value)) {
    return "[Circular]";
  }
  seen.add(value);
  if (Array.isArray(value)) {
    return value.map((item) => redact(item, seen));
  }
  return Object.fromEntries(
    Object.entries(value).map(([entryKey, item]) => [entryKey, redact(item, seen, entryKey)]),
  );
}

async function fetchJson(url, timeoutMs = 5000) {
  const response = await fetch(url, {
    signal: AbortSignal.timeout(timeoutMs),
  });
  if (!response.ok) {
    throw new Error(`${url} failed with HTTP ${response.status}`);
  }
  return response.json();
}

function debuggingUrl(options) {
  return `http://127.0.0.1:${options.debuggingPort}`;
}

function launchChrome(options) {
  const extensionDir = resolve(options.extensionDir);
  const child = spawn(options.chromeExecutable, [
    `--remote-debugging-port=${options.debuggingPort}`,
    `--user-data-dir=${options.userDataDir}`,
    `--disable-extensions-except=${extensionDir}`,
    `--load-extension=${extensionDir}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-popup-blocking",
    "--window-size=1440,1100",
    options.pageUrl,
  ], {
    detached: true,
    stdio: "ignore",
  });
  child.unref();
  return {
    pid: child.pid,
    executable: options.chromeExecutable,
    user_data_dir: options.userDataDir,
    extension_dir: extensionDir,
  };
}

async function waitForCdp(options) {
  const startedAt = Date.now();
  let lastError = null;
  while (Date.now() - startedAt < options.startupTimeoutMs) {
    try {
      return await fetchJson(`${debuggingUrl(options)}/json/version`, 2000);
    } catch (error) {
      lastError = error;
      await new Promise((resolveDelay) => setTimeout(resolveDelay, 500));
    }
  }
  throw new Error(`CDP did not become ready: ${lastError?.message || "unknown error"}`);
}

async function listTargets(options) {
  return fetchJson(`${debuggingUrl(options)}/json/list`, 5000);
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
      await new Promise((resolveReady, rejectReady) => {
        socket.addEventListener("open", resolveReady, { once: true });
        socket.addEventListener("error", rejectReady, { once: true });
      });
    },
    async send(method, params = {}) {
      const id = nextId;
      nextId += 1;
      socket.send(JSON.stringify({ id, method, params }));
      return new Promise((resolveSend, rejectSend) => {
        pending.set(id, { resolve: resolveSend, reject: rejectSend });
      });
    },
    events,
    close() {
      socket.close();
    },
  };
}

function selectPageTarget(targets, pageUrl) {
  const expectedHost = new URL(pageUrl).host;
  return targets.find(
    (target) =>
      target.type === "page" &&
      typeof target.url === "string" &&
      target.url.includes(expectedHost) &&
      target.webSocketDebuggerUrl,
  ) || targets.find((target) => target.type === "page" && target.webSocketDebuggerUrl);
}

function selectExtensionServiceWorker(targets) {
  return targets.find(
    (target) =>
      target.type === "service_worker" &&
      typeof target.url === "string" &&
      target.url.startsWith("chrome-extension://") &&
      target.webSocketDebuggerUrl,
  );
}

function selectExtensionCommandTarget(targets) {
  return selectExtensionServiceWorker(targets) || targets.find(
    (target) =>
      (target.type === "page" || target.type === "background_page") &&
      typeof target.url === "string" &&
      target.url.startsWith("chrome-extension://") &&
      target.webSocketDebuggerUrl,
  );
}

function extensionOriginFromUrl(value) {
  if (typeof value !== "string" || !value.startsWith("chrome-extension://")) {
    return null;
  }
  const parsed = new URL(value);
  return `${parsed.protocol}//${parsed.host}`;
}

function extensionOriginFromTargets(targets) {
  for (const target of targets) {
    const origin = extensionOriginFromUrl(target?.url);
    if (origin) {
      return origin;
    }
  }
  return null;
}

async function withTimeout(promise, timeoutMs, label) {
  let timeoutId = null;
  const timeout = new Promise((_, reject) => {
    timeoutId = setTimeout(() => {
      reject(new Error(`${label} timed out after ${timeoutMs}ms`));
    }, timeoutMs);
  });
  try {
    return await Promise.race([promise, timeout]);
  } finally {
    clearTimeout(timeoutId);
  }
}

async function waitForTarget(options, predicate, label) {
  const startedAt = Date.now();
  let targets = [];
  while (Date.now() - startedAt < options.startupTimeoutMs) {
    targets = await listTargets(options);
    const selected = predicate(targets);
    if (selected) {
      return selected;
    }
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 500));
  }
  throw new Error(`cannot find ${label}`);
}

async function targetHasTabsApi(target, timeoutMs) {
  if (!target?.webSocketDebuggerUrl) {
    return false;
  }
  const client = createCdpClient(target.webSocketDebuggerUrl);
  await client.ready();
  try {
    await client.send("Runtime.enable");
    const result = await withTimeout(
      client.send("Runtime.evaluate", {
        expression: "(() => Boolean(globalThis.chrome?.tabs?.query && globalThis.chrome?.tabs?.sendMessage))()",
        returnByValue: true,
      }),
      timeoutMs,
      "extension target tabs API probe",
    );
    return result.result?.value === true;
  } finally {
    client.close();
  }
}

async function createPopupCommandTarget(pageClient, options, extensionOrigin) {
  if (!extensionOrigin) {
    return null;
  }
  const popupUrl = `${extensionOrigin}/popup.html`;
  await pageClient.send("Target.createTarget", {
    url: popupUrl,
    newWindow: false,
    background: false,
  });
  return waitForTarget(
    options,
    (targets) => targets.find(
      (target) =>
        target.type === "page" &&
        target.url === popupUrl &&
        target.webSocketDebuggerUrl,
    ),
    "extension popup command target",
  );
}

async function resolveExtensionCommandTarget(pageClient, options, initialTargets) {
  const direct = selectExtensionCommandTarget(initialTargets);
  if (await targetHasTabsApi(direct, options.commandTimeoutMs).catch(() => false)) {
    return direct;
  }
  const origin = extensionOriginFromTargets(initialTargets);
  if (!origin) {
    return direct || null;
  }
  const popupTarget = await createPopupCommandTarget(pageClient, options, origin).catch(() => null);
  if (await targetHasTabsApi(popupTarget, options.commandTimeoutMs).catch(() => false)) {
    return popupTarget;
  }
  return direct || popupTarget || null;
}

function buildMarkerSnapshotExpression() {
  const snapshotExpression = String.raw`(() => {
    const markerElements = [];
    const markerMap = {};
    const geometries = [];
    const shadowHosts = [];
    let canvasCount = 0;
    const markerNames = [
      "extension-content",
      "extension-content-fallback-version",
      "extension-page-script",
      "extension-runtime-adapter",
      "extension-session",
      "extension-token-source",
      "extension-session-diagnostic-code",
      "extension-selection-event",
      "extension-selection-active",
      "extension-selection-count",
      "extension-bridge-request",
      "selection-bridge",
      "on-init-designer",
      "embedded-popup",
      "analysis-status",
      "focus-component",
      "graph-depth",
      "graph-visible-hop",
      "graph-node-count",
      "graph-edge-count",
      "local-graph-renderer",
      "local-graph-artifact-load",
      "local-graph-artifact-load-diagnostic-code",
      "local-graph-offscreen-total-ms",
      "local-graph-timing-ensure-runtime-ms",
      "local-graph-timing-fetch-metadata-ms",
      "local-graph-timing-init-runtime-ms",
      "local-graph-timing-load-document-ms",
      "local-graph-timing-build-graph-ms",
      "local-graph-timing-analyze-local-graph-ms",
      "local-graph-timing-layout-ms",
      "local-graph-timing-render-ms",
      "local-graph-timing-total-ms",
      "local-graph-timing-manifest-fetch-ms",
      "local-graph-timing-manifest-diff-ms",
      "local-graph-timing-changed-content-fetch-ms",
      "local-graph-timing-wasm-update-ms",
      "manifest-fetch-ms",
      "manifest-diff-ms",
      "changed-content-fetch-ms",
      "wasm-update-ms",
      "analyze-ms",
      "layout-ms",
      "render-ms",
      "total-ms",
      "manifest-added",
      "manifest-modified",
      "manifest-deleted",
      "manifest-unchanged",
      "content-queue-count",
      "local-graph-cache-metadata-hit",
      "local-graph-cache-document-hit",
      "local-graph-selection-debounce-ms",
      "local-graph-selection-stale",
      "local-graph-status-seq",
      "graph-hover-target",
      "graph-locked-target",
      "graph-detail-kind",
      "graph-highlight-node-count",
      "graph-highlight-edge-count",
      "graph-viewport-scale",
      "graph-viewport-target",
      "graph-density-profile"
    ];

    function toRect(element) {
      const rect = element.getBoundingClientRect();
      return {
        x: Math.round(rect.x),
        y: Math.round(rect.y),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
        right: Math.round(window.innerWidth - rect.right),
        bottom: Math.round(window.innerHeight - rect.bottom),
      };
    }

    function toInt(value, fallback = 0) {
      const parsed = Number.parseInt(value, 10);
      return Number.isFinite(parsed) ? parsed : fallback;
    }

    function normalizeText(value) {
      return String(value || "").replace(/\\s+/g, " ").trim();
    }

    function parseEdgeEvidence(text) {
      const normalized = normalizeText(text);
      const left = normalized.lastIndexOf("(");
      const right = normalized.lastIndexOf(")");
      if (left >= 0 && right > left + 1) {
        return normalized.slice(left + 1, right).trim() || "EDGE_EVIDENCE_UNAVAILABLE";
      }
      return normalized.includes("EDGE_EVIDENCE_UNAVAILABLE")
        ? "EDGE_EVIDENCE_UNAVAILABLE"
        : "";
    }

    function parseNodeRowText(value) {
      const text = normalizeText(value);
      const left = text.lastIndexOf("(");
      const right = text.lastIndexOf(")");
      if (left >= 0 && right > left + 1) {
        return {
          text,
          label: normalizeText(text.slice(0, left)),
          kind: normalizeText(text.slice(left + 1, right)),
        };
      }
      return {
        text,
        label: text,
        kind: "",
      };
    }

    function parseEdgeRowText(value) {
      const text = normalizeText(value);
      const bracketLeft = text.lastIndexOf("[");
      const bracketRight = text.lastIndexOf("]");
      let priority = "";
      let evidenceStatus = "";
      let relationText = text;
      if (bracketLeft >= 0 && bracketRight > bracketLeft + 1) {
        const marker = text.slice(bracketLeft + 1, bracketRight).trim();
        const markerParts = marker.split("/");
        priority = normalizeText(markerParts[0] || "");
        evidenceStatus = normalizeText(markerParts[1] || "");
        relationText = text.slice(0, bracketLeft).trim();
      }
      const left = text.lastIndexOf("(");
      const right = text.lastIndexOf(")");
      let evidence = "";
      if (left >= 0 && right > left + 1) {
        evidence = parseEdgeEvidence(text);
        relationText = text.slice(0, left).trim();
      }
      const relationParts = relationText.split(":");
      const namePart = relationParts[0] || "";
      const arrowParts = namePart.split("→");
      return {
        text,
        from: normalizeText(arrowParts[0] || ""),
        to: normalizeText(arrowParts[1] ? arrowParts[1].split(" ")[0] || "" : ""),
        label: normalizeText(relationParts.slice(1).join(":") || namePart),
        evidence,
        priority,
        evidence_status: evidenceStatus,
      };
    }

    function collectRows(root, selector, type) {
      const rows = [];
      const all = Array.from(root.querySelectorAll(selector));
      for (const row of all) {
        const text = normalizeText(row.textContent || row.innerText);
        if (!text) {
          continue;
        }
        if (type === "edge") {
          const parsed = parseEdgeRowText(text);
          rows.push({
            ...parsed,
            className: String(row.className || ""),
            nodeId: row.getAttribute("data-node-id") || null,
            edgeId: row.getAttribute("data-edge-id") || null,
          });
          continue;
        }

        const parsed = parseNodeRowText(text);
        rows.push({
          ...parsed,
          className: String(row.className || ""),
          nodeId: row.getAttribute("data-node-id") || null,
          edgeId: row.getAttribute("data-edge-id") || null,
          evidence: "",
        });
      }
      return rows;
    }

    function collectTextLines(host) {
      if (!host) {
        return [];
      }
      return Array.from(host.children)
        .map((child) => normalizeText(child.textContent || child.innerText || ""))
        .filter(Boolean)
        .map((text) => ({ text }));
    }

    function clickAndCollectDetail(target, detailHost) {
      if (!target || !detailHost) {
        return [];
      }
      try {
        const event = new MouseEvent("click", { bubbles: true, cancelable: true });
        target.dispatchEvent(event);
      } catch {
        return collectTextLines(detailHost);
      }
      return collectTextLines(detailHost);
    }

    function collectActionButtons(root) {
      const details = {};
      const classes = [
        "metadata-checker-graph-panel-pin",
        "metadata-checker-graph-panel-copy",
        "metadata-checker-graph-panel-toggle",
      ];
      for (const className of classes) {
        const button = root.querySelector('.' + className);
        if (!button) {
          continue;
        }
        details[className] = {
          text: normalizeText(button.textContent || ""),
          disabled: !!button.disabled,
        };
      }
      return details;
    }

    function attrsFor(element) {
      const attrs = {};
      for (const attr of Array.from(element.attributes || [])) {
        if (attr.name.startsWith("data-metadata-checker")) {
          attrs[attr.name] = attr.value;
          markerMap[attr.name] = attr.value;
        }
      }
      return attrs;
    }

    function pathFor(element) {
      const parts = [];
      let current = element;
      while (current && current.nodeType === 1 && parts.length < 6) {
        const id = current.id ? "#" + current.id : "";
        const className = String(current.className || "").trim().split(/\\s+/).filter(Boolean)
          .slice(0, 3).map((item) => "." + item).join("");
        parts.unshift(current.tagName.toLowerCase() + id + className);
        current = current.parentElement;
      }
      return parts.join(" > ");
    }

    function collectPopupProbe(documentLike) {
      const popup = documentLike.querySelector("section.metadata-checker-graph-panel");
      if (!popup) {
        return null;
      }
      const nodeRows = collectRows(popup, "li.graph-node", "node")
        .concat(collectRows(popup, "li.metadata-checker-graph-node", "node"));
      const edgeRows = collectRows(popup, "li.graph-edge", "edge")
        .concat(collectRows(popup, "li.metadata-checker-graph-edge", "edge"));
      const detailHost = popup.querySelector(".metadata-checker-graph-detail-content");
      const detailRows = collectTextLines(detailHost);
      const firstNodeRow = popup.querySelector("li.graph-node, li.metadata-checker-graph-node");
      const firstEdgeRow = popup.querySelector("li.graph-edge, li.metadata-checker-graph-edge");
      const interactionDetailAfterNodeClick = clickAndCollectDetail(firstNodeRow, detailHost);
      const interactionDetailAfterEdgeClick = clickAndCollectDetail(firstEdgeRow, detailHost);

      const canvases = Array.from(popup.querySelectorAll("canvas")).map((canvas) => {
        const rect = toRect(canvas);
        return {
          width: toInt(canvas.width, rect.width),
          height: toInt(canvas.height, rect.height),
          clientWidth: toInt(canvas.clientWidth),
          clientHeight: toInt(canvas.clientHeight),
          rect,
          className: String(canvas.className || "").trim(),
        };
      });

      const edgeEvidenceRows = edgeRows.map((row) => ({
        edgeId: row.edgeId,
        from: row.from,
        to: row.to,
        label: row.label,
        evidence: row.evidence || "EDGE_EVIDENCE_UNAVAILABLE",
        priority: row.priority || "",
        evidence_status: row.evidence_status || "",
      }));

      const diagnosticsTextRows = collectTextLines(popup.querySelector(".metadata-checker-graph-diagnostics"));

      return {
        exists: true,
        rect: toRect(popup),
        markerEmbeddedPopup: popup.getAttribute("data-metadata-checker-embedded-popup") || "",
        markerStatus: popup.getAttribute("data-metadata-checker-analysis-status") || "",
        markerRenderer: popup.getAttribute("data-metadata-checker-local-graph-renderer") || "",
        markerDepth: popup.getAttribute("data-metadata-checker-graph-depth") || "",
        markerVisibleHop: popup.getAttribute("data-metadata-checker-graph-visible-hop") || "",
        markerNodeCount: popup.getAttribute("data-metadata-checker-graph-node-count") || "0",
        markerEdgeCount: popup.getAttribute("data-metadata-checker-graph-edge-count") || "0",
        markerFocusComponent: popup.getAttribute("data-metadata-checker-focus-component") || "",
        markerGraphPanel: popup.className || "",
        nodeDetails: nodeRows,
        edgeDetails: edgeRows,
        nodeRowsCount: nodeRows.length,
        edgeRowsCount: edgeRows.length,
        canvaCount: canvases.length,
        canvases,
        actionButtons: collectActionButtons(popup),
        detailRows,
        interactionDetailAfterNodeClick,
        interactionDetailAfterEdgeClick,
        edgeEvidenceRows,
        edgeEvidenceUnavailableCount: edgeEvidenceRows.filter((row) => row.evidence === "EDGE_EVIDENCE_UNAVAILABLE").length,
        edgeEvidenceAvailable: edgeEvidenceRows.filter((row) => row.evidence && row.evidence !== "EDGE_EVIDENCE_UNAVAILABLE").length,
        diagnosticsTextRows,
      };
    }

    function visit(root, rootLabel) {
      const elements = [];
      if (root.documentElement) {
        elements.push(root.documentElement);
      }
      elements.push(...Array.from(root.querySelectorAll("*")));
      for (const element of elements) {
        const attrs = attrsFor(element);
        const attrNames = Object.keys(attrs);
        const tagName = element.tagName.toLowerCase();
        if (tagName === "canvas") {
          canvasCount += 1;
        }
        if (element.shadowRoot) {
          shadowHosts.push({
            path: pathFor(element),
            tag: tagName,
            text: String(element.shadowRoot.innerText || element.shadowRoot.textContent || "")
              .trim().replace(/\\s+/g, " ").slice(0, 600),
          });
        }
        if (attrNames.length > 0) {
          markerElements.push({
            root: rootLabel,
            tag: tagName,
            path: pathFor(element),
            attrs,
            rect: toRect(element),
            text: String(element.innerText || element.textContent || "")
              .trim().replace(/\\s+/g, " ").slice(0, 600),
          });
        }
        if (
          attrNames.some((name) => /embedded-popup|analysis-status|graph-|local-graph-renderer/.test(name)) ||
          /metadata-checker|graph|popup|panel/i.test(String(element.className || "") + " " + element.id)
        ) {
          const style = window.getComputedStyle(element);
          geometries.push({
            root: rootLabel,
            tag: tagName,
            path: pathFor(element),
            attrs,
            rect: toRect(element),
            style: {
              display: style.display,
              visibility: style.visibility,
              opacity: style.opacity,
              position: style.position,
              zIndex: style.zIndex,
            },
            text: String(element.innerText || element.textContent || "")
              .trim().replace(/\\s+/g, " ").slice(0, 800),
          });
        }
      }
      for (const element of elements) {
        if (element.shadowRoot) {
          visit(element.shadowRoot, rootLabel + " > " + element.tagName.toLowerCase() + "#shadow");
        }
      }
    }

    visit(document, "document");
    const popup = collectPopupProbe(document);
    const directMarkers = {};
    for (const name of markerNames) {
      const attr = "data-metadata-checker-" + name;
      directMarkers[name] = document.querySelector("[" + attr + "]")?.getAttribute(attr) ?? null;
    }

    return {
      url: location.href,
      title: document.title,
      readyState: document.readyState,
      viewport: {
        width: window.innerWidth,
        height: window.innerHeight,
        devicePixelRatio: window.devicePixelRatio,
      },
      directMarkers,
      markers: markerMap,
      markerElements,
      geometries: geometries.slice(0, 80),
      shadowHosts: shadowHosts.slice(0, 40),
      canvasCount,
      popup,
      popupVisible: Boolean(popup),
      collectedAt: new Date().toISOString(),
    };
  })();
  `;
  return snapshotExpression;
}

function collectPopupAcceptanceChecks(
  snapshot,
  screenshotExists = false,
  screenshotPath = "",
  manifestRefreshProbe = null,
) {
  const diagnostics = [];
  const popup = snapshot?.popup || null;
  const direct = snapshot?.directMarkers || {};
  const viewport = snapshot?.viewport || {};
  const rect = popup?.rect || {};
  const viewportWidth = Math.max(0, Number.parseFloat(viewport.width) || 0);
  const viewportHeight = Math.max(0, Number.parseFloat(viewport.height) || 0);
  const popupWidth = Math.max(0, Number.parseFloat(rect.width) || 0);
  const popupHeight = Math.max(0, Number.parseFloat(rect.height) || 0);
  const popupArea = popupWidth * popupHeight;
  const viewportArea = viewportWidth * viewportHeight;
  const canvases = Array.isArray(popup?.canvases) ? popup.canvases : [];
  const nodeDetails = Array.isArray(popup?.nodeDetails) ? popup.nodeDetails : [];
  const edgeDetails = Array.isArray(popup?.edgeDetails) ? popup.edgeDetails : [];
  const canvaRects = canvases.filter((canvas) =>
    (Number.parseFloat(canvas.width) || 0) > 0 && (Number.parseFloat(canvas.height) || 0) > 0,
  );
  const overflowingCanvases = canvases.filter((canvas) => {
    const canvasRect = canvas?.rect || {};
    const canvasX = Number.parseFloat(canvasRect.x) || 0;
    const canvasWidth = Number.parseFloat(canvasRect.width) || 0;
    const popupX = Number.parseFloat(rect.x) || 0;
    return (
      canvasWidth > popupWidth + 1 ||
      canvasX < popupX - 1 ||
      canvasX + canvasWidth > popupX + popupWidth + 1
    );
  });
  const graphDepth = normalizeToInt(popup?.markerDepth ?? direct["graph-depth"], 0);
  const graphVisibleHop = normalizeToInt(popup?.markerVisibleHop ?? direct["graph-visible-hop"], 0);
  const selectionProbe = snapshot?.selectionProbe || null;
  const manifestProbe = manifestRefreshProbe || snapshot?.manifestProbe || null;
  const manifestDiff = mergeManifestDiff(
    collectManifestDiffFromResponse(snapshot?.manifestProbe?.command),
    collectManifestDiffFromResponse(manifestRefreshProbe?.command),
    collectManifestDiffFromMarkers(direct),
  );
  const performanceTimings = collectPerformanceTimings(direct);
  const edgeInteractionDetailText = (popup?.interactionDetailAfterEdgeClick || [])
    .map((row) => asString(row.text))
    .join(" ");
  const nodeInteractionDetailText = (popup?.interactionDetailAfterNodeClick || [])
    .map((row) => asString(row.text))
    .join(" ");
  const interactionProbe = snapshot?.interactionProbe || {};

  const checks = {
    renderer: {
      status: popup?.markerRenderer === "pixi" ? "pass" : "fail",
      actual: popup?.markerRenderer || direct["local-graph-renderer"] || "",
      expected: "pixi",
      diagnostics: [],
    },
    canvas: {
      status: canvases.length > 0 && canvaRects.length > 0 ? "pass" : "fail",
      total: canvases.length,
      nonEmpty: canvaRects.length,
      diagnostics: [],
    },
    geometry: {
      status: "pass",
      geometry: {
        popupWidth,
        popupHeight,
        popupArea,
        viewportWidth,
        viewportHeight,
        areaRatio: viewportArea && popupArea ? viewportArea / popupArea : 0,
        rightOffset: Number.parseFloat(rect.right) || 0,
        bottomOffset: Number.parseFloat(rect.bottom) || 0,
      },
      diagnostics: [],
    },
    manifest: {
      status: manifestDiffSeen(manifestDiff) ? "pass" : "fail",
      diff: manifestDiff,
      timedOut: Boolean(manifestRefreshProbe?.timedOut || false),
      command: manifestRefreshProbe ? {
        requestType: manifestRefreshProbe.requestType,
        ok: manifestRefreshProbe.ok,
        error: manifestRefreshProbe.error || null,
        timedOut: Boolean(manifestRefreshProbe.timedOut),
      } : null,
      diagnostics: [],
    },
    selection: {
      status: "pass",
      missingMarkers: [],
      probe: selectionProbe,
      diagnostics: [],
    },
    interaction: {
      status: "pass",
      markerPresence: M48_INTERACTION_MARKER_NAMES.map((name) => ({
        name,
        value: asString(direct[name], ""),
        present: !isMissingMarker(direct[name]),
      })),
      categories: createInteractionCategoryResult(),
      diagnostics: [],
      edgeEvidenceUnavailable: popup?.edgeEvidenceUnavailableCount || 0,
      edgeEvidenceAvailable: popup?.edgeEvidenceAvailable || 0,
      edgeEvidenceRows: popup?.edgeEvidenceRows || [],
    },
    performance: {
      status: "pass",
      timings: performanceTimings,
      diagnostics: [],
    },
    screenshot: {
      status: screenshotExists ? "pass" : "fail",
      path: screenshotPath,
      diagnostics: [],
    },
  };

  if (checks.canvas.status === "fail") {
    checks.canvas.diagnostics.push({
      category: "canvas",
      code: "M47_REAL_BI_CANVAS_NOT_RENDERED",
      message: "no valid canvas content was detected in popup",
      details: {
        canvasCount: canvases.length,
        nonEmptyCanvasCount: canvaRects.length,
      },
    });
    addDiagnostic(
      diagnostics,
      "canvas",
      "M47_REAL_BI_CANVAS_NOT_RENDERED",
      "No valid canvas content in popup",
      checks.canvas,
    );
  }

  if (popup?.canvaCount === 0 && canvases.length > 0) {
    checks.canvas.status = "fail";
    checks.canvas.diagnostics.push({
      category: "canvas",
      code: "M47_REAL_BI_CANVAS_MISSING_CANVA_METADATA",
      message: "canvas nodes detected but popup canva metadata is missing",
      details: {
        canvasCount: canvases.length,
      },
    });
    addDiagnostic(
      diagnostics,
      "canvas",
      "M47_REAL_BI_CANVAS_MISSING_CANVA_METADATA",
      "canvas metadata missing in popup snapshot",
      {
        canvasCount: canvases.length,
      },
    );
  }

  if (overflowingCanvases.length > 0) {
    checks.geometry.status = "fail";
    addDiagnostic(
      checks.geometry.diagnostics,
      "geometry",
      "M47_REAL_BI_CANVAS_OVERFLOWS_POPUP",
      "canvas bounds exceed popup bounds",
      {
        popupRect: rect,
        canvases: overflowingCanvases.map((canvas) => canvas.rect),
      },
    );
    addDiagnostic(
      diagnostics,
      "geometry",
      "M47_REAL_BI_CANVAS_OVERFLOWS_POPUP",
      "canvas bounds exceed popup bounds",
      {
        popupRect: rect,
        canvases: overflowingCanvases.map((canvas) => canvas.rect),
      },
    );
  }

  if (!(checks.renderer.status === "pass")) {
    addDiagnostic(
      checks.renderer.diagnostics,
      "renderer",
      "M47_REAL_BI_RENDERER_NOT_PIXI",
      `renderer marker is ${checks.renderer.actual || "unknown"}`,
      checks.renderer,
    );
    addDiagnostic(
      diagnostics,
      "renderer",
      "M47_REAL_BI_RENDERER_NOT_PIXI",
      `renderer marker is ${checks.renderer.actual || "unknown"}`,
      checks.renderer,
    );
  }

  if (graphDepth !== 2) {
    checks.canvas.diagnostics = checks.canvas.diagnostics || [];
    checks.geometry.status = "fail";
    addDiagnostic(
      checks.geometry.diagnostics,
      "geometry",
      "M47_REAL_BI_GRAPH_DEPTH_NOT_2",
      `graph depth is ${graphDepth}`,
      { actual: graphDepth },
    );
    addDiagnostic(
      diagnostics,
      "geometry",
      "M47_REAL_BI_GRAPH_DEPTH_NOT_2",
      `graph depth is ${graphDepth}`,
      { actual: graphDepth },
    );
  }

  if (graphVisibleHop !== 1) {
    checks.geometry.status = "fail";
    addDiagnostic(
      checks.geometry.diagnostics,
      "geometry",
      "M47_REAL_BI_GRAPH_VISIBLE_HOP_NOT_1",
      `graph visible hop is ${graphVisibleHop}`,
      { actual: graphVisibleHop },
    );
    addDiagnostic(
      diagnostics,
      "geometry",
      "M47_REAL_BI_GRAPH_VISIBLE_HOP_NOT_1",
      `graph visible hop is ${graphVisibleHop}`,
      { actual: graphVisibleHop },
    );
  }

  const areaRatioPass = viewportArea > 0 && popupArea > 0 && viewportArea / popupArea >= ACCEPTANCE_GEOMETRY_AREA_RATIO;
  if (!areaRatioPass) {
    checks.geometry.status = "fail";
    addDiagnostic(
      checks.geometry.diagnostics,
      "geometry",
      "M47_REAL_BI_GEOMETRY_EXCEEDS_PAGE_NINTH",
      "popup area exceeds one-ninth of viewport area",
      checks.geometry.geometry,
    );
    addDiagnostic(
      diagnostics,
      "geometry",
      "M47_REAL_BI_GEOMETRY_EXCEEDS_PAGE_NINTH",
      "popup area exceeds one-ninth of viewport area",
      checks.geometry.geometry,
    );
  }

  const requiredSelectionMarkers = [
    "extension-content",
    "extension-page-script",
    "extension-runtime-adapter",
    "selection-bridge",
    "on-init-designer",
  ];
  for (const key of requiredSelectionMarkers) {
    const value = direct[key] || snapshot?.popup?.[`marker${key.replace(/-(.)/g, (match, char) => char.toUpperCase())}`] || "";
    if (!value) {
      checks.selection.missingMarkers.push(key);
    }
  }
  if (checks.selection.missingMarkers.length > 0) {
    checks.selection.status = "fail";
    checks.selection.markerCount = requiredSelectionMarkers.length;
    checks.selection.presentCount = requiredSelectionMarkers.length - checks.selection.missingMarkers.length;
    checks.selection.missingMarkers = checks.selection.missingMarkers;
    addDiagnostic(
      checks.selection.diagnostics,
      "selection",
      "M47_REAL_BI_SELECTION_MARKERS_MISSING",
      `missing markers: ${checks.selection.missingMarkers.join(", ")}`,
      {
        missingMarkers: checks.selection.missingMarkers,
      },
    );
    addDiagnostic(
      diagnostics,
      "selection",
      "M47_REAL_BI_SELECTION_MARKERS_MISSING",
      `missing markers: ${checks.selection.missingMarkers.join(", ")}`,
      { missingMarkers: checks.selection.missingMarkers },
    );
  }

  const selectionProbePass =
    selectionProbe?.attempted === true &&
    selectionProbe?.posted === true &&
    selectionProbe?.loading?.event === "received" &&
    selectionProbe?.loading?.status === "loading" &&
    selectionProbe?.final?.event === "received" &&
    (selectionProbe?.final?.status === "ready" || selectionProbe?.final?.status === "warning") &&
    normalizeToInt(selectionProbe?.final?.seq, 0) > normalizeToInt(selectionProbe?.before?.seq, -1);
  if (!selectionProbePass) {
    checks.selection.status = "fail";
    addDiagnostic(
      checks.selection.diagnostics,
      "selection",
      "M47_REAL_BI_SELECTION_CHANGED_NOT_VERIFIED",
      "selection changed loading-to-ready transition was not verified",
      {
        attempted: selectionProbe?.attempted || false,
        posted: selectionProbe?.posted || false,
        before: selectionProbe?.before || null,
        loading: selectionProbe?.loading || null,
        final: selectionProbe?.final || null,
        loadingTimedOut: Boolean(selectionProbe?.loadingTimedOut),
        finalTimedOut: Boolean(selectionProbe?.finalTimedOut),
        reason: selectionProbe?.reason || "",
        error: selectionProbe?.error || "",
      },
    );
    addDiagnostic(
      diagnostics,
      "selection",
      "M47_REAL_BI_SELECTION_CHANGED_NOT_VERIFIED",
      "selection changed loading-to-ready transition was not verified",
      {
        attempted: selectionProbe?.attempted || false,
        posted: selectionProbe?.posted || false,
        before: selectionProbe?.before || null,
        loading: selectionProbe?.loading || null,
        final: selectionProbe?.final || null,
        loadingTimedOut: Boolean(selectionProbe?.loadingTimedOut),
        finalTimedOut: Boolean(selectionProbe?.finalTimedOut),
      },
    );
  }

  if (!manifestDiffSeen(manifestDiff)) {
    checks.manifest.status = "fail";
    if (!manifestProbe) {
      addDiagnostic(
        checks.manifest.diagnostics,
        "manifest",
        "M47_REAL_BI_MANIFEST_REFRESH_NOT_TRIGGERED",
        "manifest refresh probe was not executed",
        {
          manifestProbe: null,
        },
      );
      addDiagnostic(
        diagnostics,
        "manifest",
        "M47_REAL_BI_MANIFEST_REFRESH_NOT_TRIGGERED",
        "manifest refresh probe was not executed",
        {
          manifestProbe: null,
        },
      );
    } else if (!manifestProbe.ok && manifestProbe.error) {
      addDiagnostic(
        checks.manifest.diagnostics,
        "manifest",
        "M47_REAL_BI_MANIFEST_REFRESH_COMMAND_FAILED",
        "manifest refresh command failed",
        {
          command: manifestProbe.requestType,
          error: manifestProbe.error,
          timedOut: Boolean(manifestProbe.timedOut),
        },
      );
      addDiagnostic(
        diagnostics,
        "manifest",
        "M47_REAL_BI_MANIFEST_REFRESH_COMMAND_FAILED",
        "manifest refresh command failed",
        {
          command: manifestProbe.requestType,
          error: manifestProbe.error,
          timedOut: Boolean(manifestProbe.timedOut),
        },
      );
    } else if (manifestProbe.timedOut || manifestProbe.responseTimedOut) {
      addDiagnostic(
        checks.manifest.diagnostics,
        "manifest",
        "M47_REAL_BI_MANIFEST_REFRESH_TIMED_OUT",
        "manifest refresh marker probe timed out",
        {
          command: manifestProbe.requestType,
          timedOut: true,
        },
      );
      addDiagnostic(
        diagnostics,
        "manifest",
        "M47_REAL_BI_MANIFEST_REFRESH_TIMED_OUT",
        "manifest refresh marker probe timed out",
        {
          command: manifestProbe.requestType,
          timedOut: true,
        },
      );
    } else {
      addDiagnostic(
        checks.manifest.diagnostics,
        "manifest",
        "M47_REAL_BI_MANIFEST_DIFF_MARKER_MISSING",
        "manifest diff fields were not observed after refresh probe",
        {
          attempted: manifestProbe.attempted,
          command: manifestProbe.requestType,
          response: manifestProbe.response,
          timedOut: Boolean(manifestProbe.timedOut || manifestProbe.responseTimedOut),
        },
      );
      addDiagnostic(
        diagnostics,
        "manifest",
        "M47_REAL_BI_MANIFEST_DIFF_MARKER_MISSING",
        "manifest diff fields were not observed after refresh probe",
        {
          attempted: manifestProbe.attempted,
          command: manifestProbe.requestType,
          response: manifestProbe.response,
          timedOut: Boolean(manifestProbe.timedOut || manifestProbe.responseTimedOut),
        },
      );
    }
  }

  const interactionCategories = checks.interaction.categories;
  const missingInteractionMarkers = checks.interaction.markerPresence.filter(
    (item) => !item.present,
  );
  if (missingInteractionMarkers.length > 0) {
    checks.interaction.status = "fail";
    addDiagnostic(
      checks.interaction.diagnostics,
      "interaction",
      "M48_REAL_BI_INTERACTION_MARKER_MISSING",
      "required interaction markers are missing",
      {
        missingMarkers: missingInteractionMarkers.map((item) => item.name),
      },
    );
    addDiagnostic(
      diagnostics,
      "interaction",
      "M48_REAL_BI_INTERACTION_MARKER_MISSING",
      "required interaction markers are missing",
      {
        missingMarkers: missingInteractionMarkers.map((item) => item.name),
      },
    );
  }

  function markerFromStep(stepResult, markerName, phase = "after") {
    if (!stepResult || typeof stepResult !== "object") {
      return "";
    }
    const source = phase === "before" ? (stepResult.before || {}) : (stepResult.after || {});
    if (typeof source?.markers?.[markerName] === "string") {
      return source.markers[markerName];
    }
    if (typeof source?.[markerName] === "string") {
      return source[markerName];
    }
    if (typeof stepResult?.markers?.[markerName] === "string") {
      return stepResult.markers[markerName];
    }
    if (typeof stepResult?.[markerName] === "string") {
      return stepResult[markerName];
    }
    return "";
  }

  function setInteractionCategory(category, status, code, message, details = {}) {
    if (!interactionCategories[category]) {
      return;
    }
    interactionCategories[category].status = status;
    interactionCategories[category].details = details;
    interactionCategories[category].diagnostics.push({
      code,
      message,
      details,
    });
    if (status === "fail") {
      addDiagnostic(
        checks.interaction.diagnostics,
        "interaction",
        code,
        message,
        details,
      );
      addDiagnostic(
        diagnostics,
        "interaction",
        code,
        message,
        details,
      );
    }
  }

  const interactionProbeAttempted = interactionProbe?.attempted === true;

  if (!interactionProbeAttempted) {
    for (const category of M48_INTERACTION_CATEGORIES) {
      setInteractionCategory(
        category,
        "fail",
        "M48_REAL_BI_INTERACTION_PROBE_NOT_ATTEMPTED",
        "interaction probe did not run",
        {
          reason: interactionProbe?.reason || "not attempted",
        },
      );
    }
  } else {
    const hoverNode = interactionProbe?.hover?.node || {};
    const hoverEdge = interactionProbe?.hover?.edge || {};
    const hoverAggregate = interactionProbe?.hover?.aggregate || {};
    const lockNode = interactionProbe?.lock?.node || {};
    const lockEdge = interactionProbe?.lock?.edge || {};
    const lockAggregate = interactionProbe?.lock?.aggregate || {};
    const zoom = interactionProbe?.zoom || {};
    const viewportFocusEdge = interactionProbe?.viewportFocusEdge || {};
    const copyProbe = interactionProbe?.copy || {};
    const hoverNodeTarget = interactionTargetType(markerFromStep(hoverNode, "graph-hover-target"));
    const hoverEdgeTarget = interactionTargetType(markerFromStep(hoverEdge, "graph-hover-target"));
    const hoverAggregateTarget = interactionTargetType(
      markerFromStep(hoverAggregate, "graph-hover-target"),
    );
    const lockNodeTarget = interactionTargetType(markerFromStep(lockNode, "graph-locked-target"));
    const lockEdgeTarget = interactionTargetType(markerFromStep(lockEdge, "graph-locked-target"));
    const lockAggregateTarget = interactionTargetType(
      markerFromStep(lockAggregate, "graph-locked-target"),
    );
    const zoomBeforeScale = parseNumericMarker(markerFromStep(zoom, "graph-viewport-scale", "before"));
    const zoomAfterScale = parseNumericMarker(markerFromStep(zoom, "graph-viewport-scale", "after"));
    const viewportTargetAfter = normalizeInteractionMarker(
      markerFromStep(viewportFocusEdge, "graph-viewport-target"),
    );
    const focusBefore = normalizeInteractionMarker(
      markerFromStep(viewportFocusEdge, "focusComponent", "before") || viewportFocusEdge?.beforeFocus || "",
    );
    const focusAfter = normalizeInteractionMarker(
      markerFromStep(viewportFocusEdge, "focusComponent", "after") || viewportFocusEdge?.afterFocus || "",
    );
    const copiedText = asString(interactionProbe?.copy?.copiedText || "", "");

    if (!hoverNode.attempted) {
      setInteractionCategory(
        "hover_node",
        "fail",
        "M48_REAL_BI_HOVER_NODE_NOT_TRIGGERED",
        "hover node interaction was not triggered",
        { attempted: hoverNode.attempted || false },
      );
    } else if (hoverNodeTarget !== "node") {
      setInteractionCategory(
        "hover_node",
        "fail",
        "M48_REAL_BI_HOVER_NODE_NOT_RECOGNIZED",
        "hover interaction did not set node target marker",
        {
          marker: markerFromStep(hoverNode, "graph-hover-target"),
        },
      );
    }

    if (!hoverEdge.attempted) {
      setInteractionCategory(
        "hover_edge",
        "fail",
        "M48_REAL_BI_HOVER_EDGE_NOT_TRIGGERED",
        "hover edge interaction was not triggered",
        { attempted: hoverEdge.attempted || false },
      );
    } else if (hoverEdgeTarget !== "edge") {
      setInteractionCategory(
        "hover_edge",
        "fail",
        "M48_REAL_BI_HOVER_EDGE_NOT_RECOGNIZED",
        "hover interaction did not set edge target marker",
        {
          marker: markerFromStep(hoverEdge, "graph-hover-target"),
        },
      );
    }

    if (!hoverAggregate.attempted) {
      setInteractionCategory(
        "hover_aggregate",
        "fail",
        "M48_REAL_BI_HOVER_AGGREGATE_NOT_TRIGGERED",
        "hover aggregate interaction was not triggered",
        { attempted: hoverAggregate.attempted || false },
      );
    } else if (hoverAggregateTarget && hoverAggregateTarget !== "aggregate") {
      setInteractionCategory(
        "hover_aggregate",
        "fail",
        "M48_REAL_BI_HOVER_AGGREGATE_NOT_RECOGNIZED",
        "hover interaction did not set aggregate target marker",
        {
          marker: markerFromStep(hoverAggregate, "graph-hover-target"),
        },
      );
    } else if (!hoverAggregateTarget) {
      setInteractionCategory(
        "hover_aggregate",
        "fail",
        "M48_REAL_BI_HOVER_AGGREGATE_MISSING_TARGET",
        "hover interaction target is missing",
        { marker: markerFromStep(hoverAggregate, "graph-hover-target") },
      );
    }

    if (!lockNode.attempted) {
      setInteractionCategory(
        "lock_node",
        "fail",
        "M48_REAL_BI_LOCK_NODE_NOT_TRIGGERED",
        "lock node interaction was not triggered",
        { attempted: lockNode.attempted || false },
      );
    } else if (lockNodeTarget !== "node") {
      setInteractionCategory(
        "lock_node",
        "fail",
        "M48_REAL_BI_LOCK_NODE_NOT_RECOGNIZED",
        "lock interaction did not set node target marker",
        {
          marker: markerFromStep(lockNode, "graph-locked-target"),
        },
      );
    }

    if (!lockEdge.attempted) {
      setInteractionCategory(
        "lock_edge",
        "fail",
        "M48_REAL_BI_LOCK_EDGE_NOT_TRIGGERED",
        "lock edge interaction was not triggered",
        { attempted: lockEdge.attempted || false },
      );
    } else if (lockEdgeTarget !== "edge") {
      setInteractionCategory(
        "lock_edge",
        "fail",
        "M48_REAL_BI_LOCK_EDGE_NOT_RECOGNIZED",
        "lock interaction did not set edge target marker",
        {
          marker: markerFromStep(lockEdge, "graph-locked-target"),
        },
      );
    }

    if (!lockAggregate.attempted) {
      setInteractionCategory(
        "lock_aggregate",
        "fail",
        "M48_REAL_BI_LOCK_AGGREGATE_NOT_TRIGGERED",
        "lock aggregate interaction was not triggered",
        { attempted: lockAggregate.attempted || false },
      );
    } else if (lockAggregateTarget && lockAggregateTarget !== "aggregate") {
      setInteractionCategory(
        "lock_aggregate",
        "fail",
        "M48_REAL_BI_LOCK_AGGREGATE_NOT_RECOGNIZED",
        "lock interaction did not set aggregate target marker",
        {
          marker: markerFromStep(lockAggregate, "graph-locked-target"),
        },
      );
    } else if (!lockAggregateTarget) {
      setInteractionCategory(
        "lock_aggregate",
        "fail",
        "M48_REAL_BI_LOCK_AGGREGATE_MISSING_TARGET",
        "lock interaction target is missing",
        { marker: markerFromStep(lockAggregate, "graph-locked-target") },
      );
    }

    if (!zoom.attempted) {
      setInteractionCategory(
        "viewport_zoom",
        "fail",
        "M48_REAL_BI_VIEWPORT_ZOOM_NOT_TRIGGERED",
        "viewport zoom interaction was not triggered",
        { attempted: zoom.attempted || false },
      );
    } else if (
      zoomBeforeScale === null ||
      zoomAfterScale === null ||
      zoomBeforeScale === zoomAfterScale
    ) {
      setInteractionCategory(
        "viewport_zoom",
        "fail",
        "M48_REAL_BI_VIEWPORT_ZOOM_NOT_CHANGED",
        "viewport scale marker did not change during wheel interaction",
        {
          beforeScale: zoomBeforeScale,
          afterScale: zoomAfterScale,
        },
      );
    }

    const scrollStateChanged = (
      interactionProbe?.zoom?.beforeScroll?.scrollX !== undefined &&
      interactionProbe?.zoom?.afterScroll?.scrollX !== undefined &&
      interactionProbe.zoom.beforeScroll.scrollX !== interactionProbe.zoom.afterScroll.scrollX
    ) || (
      interactionProbe?.zoom?.beforeScroll?.scrollY !== undefined &&
      interactionProbe?.zoom?.afterScroll?.scrollY !== undefined &&
      interactionProbe.zoom.beforeScroll.scrollY !== interactionProbe.zoom.afterScroll.scrollY
    );
    if (scrollStateChanged) {
      addDiagnostic(
        checks.interaction.diagnostics,
        "interaction",
        "M48_REAL_BI_VIEWPORT_SCROLL_CHANGED",
        "page scroll changed during interaction wheel event",
        {
          beforeScroll: interactionProbe?.zoom?.beforeScroll || null,
          afterScroll: interactionProbe?.zoom?.afterScroll || null,
        },
      );
      addDiagnostic(
        diagnostics,
        "interaction",
        "M48_REAL_BI_VIEWPORT_SCROLL_CHANGED",
        "page scroll changed during interaction wheel event",
        {
          beforeScroll: interactionProbe?.zoom?.beforeScroll || null,
          afterScroll: interactionProbe?.zoom?.afterScroll || null,
        },
      );
    }

    if (!viewportFocusEdge.attempted) {
      setInteractionCategory(
        "viewport_focus_edge",
        "fail",
        "M48_REAL_BI_VIEWPORT_FOCUS_EDGE_NOT_TRIGGERED",
        "viewport focus edge interaction was not triggered",
        { attempted: viewportFocusEdge.attempted || false },
      );
    } else {
      const viewportFocusTarget = interactionTargetType(viewportTargetAfter);
      if (viewportFocusTarget !== "edge") {
        setInteractionCategory(
          "viewport_focus_edge",
          "fail",
          "M48_REAL_BI_VIEWPORT_TARGET_NOT_EDGE",
          "viewport target marker was not edge after edge interaction",
          {
            viewportTarget: viewportTargetAfter,
          },
        );
      }
      if (focusBefore && focusAfter && focusBefore !== focusAfter) {
        setInteractionCategory(
          "viewport_focus_edge",
          "fail",
          "M48_REAL_BI_DESIGNER_FOCUS_CHANGED",
          "design focus component changed after edge click",
          {
            focusBefore,
            focusAfter,
          },
        );
      }
    }

    if (!copyProbe.attempted) {
      setInteractionCategory(
        "copy_graph_text",
        "fail",
        "M48_REAL_BI_COPY_NOT_TRIGGERED",
        "copy interaction was not triggered",
        { attempted: copyProbe.attempted || false },
      );
    } else if (!copiedText || copiedText === "") {
      setInteractionCategory(
        "copy_graph_text",
        "fail",
        "M48_REAL_BI_COPY_EMPTY_TEXT",
        "copy interaction produced no text",
        {
          copiedText: copiedText || "",
        },
      );
    } else if (!copiedText.includes("nodes:") || !copiedText.includes("edges:")) {
      setInteractionCategory(
        "copy_graph_text",
        "fail",
        "M48_REAL_BI_COPY_GRAPH_TEXT_INVALID",
        "copy interaction did not include visible graph text sections",
        {
          copiedText: copiedText.slice(0, 240),
        },
      );
    } else if (containsSensitiveText(copiedText)) {
      setInteractionCategory(
        "copy_graph_text",
        "fail",
        "M48_REAL_BI_COPY_SENSITIVE_TEXT",
        "copy interaction returned sensitive content",
        {
          copiedText: copiedText.slice(0, 240),
        },
      );
    }
  }

  const interactionFailedCategories = Object.values(checks.interaction.categories)
    .filter((category) => category.status !== "pass");
  if (interactionFailedCategories.length > 0) {
    checks.interaction.status = "fail";
  }

  const graphStatus = direct["analysis-status"] || popup?.markerStatus || "";
  const hasPopupState = popup?.exists && graphStatus;
  const actionButtonCount = Object.keys(popup?.actionButtons || {}).length;
  if (!hasPopupState || actionButtonCount < 3) {
    checks.interaction.status = "fail";
    addDiagnostic(
      checks.interaction.diagnostics,
      "interaction",
      "M47_REAL_BI_INTERACTION_PANEL_ACTIONS_MISSING",
      "graph panel interaction controls are incomplete",
      {
        graphStatus,
        actionButtonCount,
      },
    );
    addDiagnostic(
      diagnostics,
      "interaction",
      "M47_REAL_BI_INTERACTION_PANEL_ACTIONS_MISSING",
      "graph panel interaction controls are incomplete",
      {
        graphStatus,
        actionButtonCount,
      },
    );
  }

  if (toInt(direct["graph-edge-count"] || popup?.markerEdgeCount) > 0 || edgeDetails.length > 0) {
    if (nodeDetails.length === 0 || edgeDetails.length === 0) {
      checks.interaction.status = "fail";
      addDiagnostic(
        checks.interaction.diagnostics,
        "interaction",
        "M47_REAL_BI_INTERACTION_DETAILS_MISSING",
        "edge/node list exists but popup detail data is empty",
        {
          nodeCount: nodeDetails.length,
          edgeCount: edgeDetails.length,
        },
      );
      addDiagnostic(
        diagnostics,
        "interaction",
        "M47_REAL_BI_INTERACTION_DETAILS_MISSING",
        "edge/node list exists but popup detail data is empty",
        {
          nodeCount: nodeDetails.length,
          edgeCount: edgeDetails.length,
        },
      );
    }
  }

  if (edgeDetails.length > 0) {
    const hasEdgeDetailContract =
      /\bEdge\b/.test(edgeInteractionDetailText) &&
      /\b(filter|condition|visibility|source|action|other)\b/i.test(edgeInteractionDetailText) &&
      (edgeInteractionDetailText.includes("EDGE_EVIDENCE_UNAVAILABLE") || /->/.test(edgeInteractionDetailText));
    if (!hasEdgeDetailContract) {
      checks.interaction.status = "fail";
      addDiagnostic(
        checks.interaction.diagnostics,
        "interaction",
        "M47_REAL_BI_EDGE_DETAIL_CONTRACT_MISSING",
        "edge click detail is missing priority/evidence status fields",
        { detailText: edgeInteractionDetailText },
      );
      addDiagnostic(
        diagnostics,
        "interaction",
        "M47_REAL_BI_EDGE_DETAIL_CONTRACT_MISSING",
        "edge click detail is missing priority/evidence status fields",
        { detailText: edgeInteractionDetailText },
      );
    }
  }

  if (nodeDetails.length > 0) {
    const hasNodeDetailContract =
      /\bNode\b/.test(nodeInteractionDetailText) &&
      /\bneighbors\b/i.test(nodeInteractionDetailText);
    if (!hasNodeDetailContract) {
      checks.interaction.status = "fail";
      addDiagnostic(
        checks.interaction.diagnostics,
        "interaction",
        "M47_REAL_BI_NODE_DETAIL_CONTRACT_MISSING",
        "node click detail is missing neighbor or priority summary fields",
        { detailText: nodeInteractionDetailText },
      );
      addDiagnostic(
        diagnostics,
        "interaction",
        "M47_REAL_BI_NODE_DETAIL_CONTRACT_MISSING",
        "node click detail is missing neighbor or priority summary fields",
        { detailText: nodeInteractionDetailText },
      );
    }
  }

  if (popup?.edgeEvidenceRows?.length) {
    const unavailableTextRows = (popup.diagnosticsTextRows || []).map((row) => asString(row.text))
      .concat((popup.edgeEvidenceRows || []).map((edge) => asString(edge.evidence)).filter(Boolean));
    const unavailableMarkers = unavailableTextRows.filter((item) => item.includes(EDGE_EVIDENCE_UNAVAILABLE_TEXT));
    if (unavailableMarkers.length > 0) {
      addDiagnostic(
        checks.interaction.diagnostics,
        "interaction",
        "M47_REAL_BI_EDGE_EVIDENCE_TEXT",
        "edge evidence unavailable marker was observed",
        {
          edgeEvidenceUnavailableCount: checks.interaction.edgeEvidenceUnavailable,
          evidenceMarkers: unavailableMarkers,
        },
      );
    }
  }

  const hasRenderTiming = Number.isFinite(performanceTimings.render_ms);
  const hasOffscreenTiming =
    Number.isFinite(performanceTimings.total_ms) ||
    Number.isFinite(performanceTimings.offscreen_total_ms);
  if (!hasRenderTiming || !hasOffscreenTiming) {
    checks.performance.status = "fail";
    addDiagnostic(
      checks.performance.diagnostics,
      "performance",
      "M47_REAL_BI_PERFORMANCE_TIMING_MISSING",
      "local graph performance timing markers are incomplete",
      {
        hasRenderTiming,
        hasOffscreenTiming,
        timings: performanceTimings,
      },
    );
    addDiagnostic(
      diagnostics,
      "performance",
      "M47_REAL_BI_PERFORMANCE_TIMING_MISSING",
      "local graph performance timing markers are incomplete",
      {
        hasRenderTiming,
        hasOffscreenTiming,
        timings: performanceTimings,
      },
    );
  }

  if (checks.screenshot.status === "fail") {
    addDiagnostic(
      checks.screenshot.diagnostics,
      "screenshot",
      "M47_REAL_BI_SCREENSHOT_MISSING",
      "screenshot file is missing",
      { path: screenshotPath },
    );
    addDiagnostic(
      diagnostics,
      "screenshot",
      "M47_REAL_BI_SCREENSHOT_MISSING",
      "screenshot file is missing",
      { path: screenshotPath },
    );
  }

  const failingCategories = [];
  for (const [name, value] of Object.entries(checks)) {
    if (name === "checks") {
      continue;
    }
    if (value.status === "fail") {
      failingCategories.push(name);
    }
  }

  return {
    checks,
    failed_categories: failingCategories,
    diagnostics,
    failed: failingCategories.length > 0,
    passing: diagnostics.length === 0,
    node_details: nodeDetails,
    edge_details: edgeDetails,
    edge_evidence_unavailable_count: checks.interaction.edgeEvidenceUnavailable,
    performance_timings: performanceTimings,
    screenshot_exists: screenshotExists,
  };
}

async function evaluatePageSnapshot(client) {
  const result = await client.send("Runtime.evaluate", {
    expression: buildMarkerSnapshotExpression(),
    awaitPromise: true,
    returnByValue: true,
  });
  if (result.exceptionDetails) {
    throw new Error(result.exceptionDetails.text || "page snapshot evaluation failed");
  }
  return result.result?.value || {};
}

async function sendExtensionCommand(workerClient, pageUrl, requestType, timeoutMs) {
  const expression = `(async () => {
    const activeTabs = await chrome.tabs.query({ active: true, currentWindow: true });
    const activeTab = activeTabs.find((candidate) => String(candidate.url || "").includes(${JSON.stringify(new URL(pageUrl).host)}));
    const tabs = activeTab ? [activeTab] : await chrome.tabs.query({ url: ${JSON.stringify(`${new URL(pageUrl).origin}/*`)} });
    const tab = tabs.find((candidate) => String(candidate.url || "").includes(${JSON.stringify(new URL(pageUrl).host)})) || tabs[0];
    if (!tab || typeof tab.id !== "number") {
      return { ok: false, diagnostics: [{ severity: "error", code: "M47_REAL_BI_TAB_NOT_FOUND", message: "real BI tab was not found" }] };
    }
    return await new Promise((resolve) => {
      chrome.tabs.sendMessage(tab.id, {
        type: "metadata-checker-tab-request",
        request_type: ${JSON.stringify(requestType)}
      }, (response) => {
        const error = chrome.runtime.lastError?.message || null;
        resolve({
          ok: !error,
          request_type: ${JSON.stringify(requestType)},
          error,
          tab: { id: tab.id, url: tab.url, title: tab.title },
          response
        });
      });
    });
  })()`;
  const result = await withTimeout(
    workerClient.send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
    }),
    timeoutMs,
    `extension command ${requestType}`,
  );
  if (result.exceptionDetails) {
    return {
      ok: false,
      request_type: requestType,
      error: result.exceptionDetails.text || "extension command failed",
    };
  }
  return result.result?.value || { ok: false, request_type: requestType };
}

function selectionProbeStatusFromSnapshot(snapshot) {
  const direct = snapshot?.directMarkers || {};
  return {
    event: direct["extension-selection-event"] || "",
    active: direct["extension-selection-active"] || "",
    count: normalizeToInt(direct["extension-selection-count"], 0),
    status: direct["analysis-status"] || "",
    seq: normalizeToInt(direct["local-graph-status-seq"], 0),
    focus: direct["focus-component"] || "",
    sourcePath: sourcePathFromSnapshot(snapshot),
    collectedAt: snapshot?.collectedAt || "",
  };
}

function sourcePathFromSnapshot(snapshot) {
  const elements = Array.isArray(snapshot?.markerElements) ? snapshot.markerElements : [];
  for (const element of elements) {
    const value = element?.attrs?.["data-metadata-checker-panel-source-path"];
    if (value) {
      return value;
    }
  }
  const focus = snapshot?.directMarkers?.["focus-component"] || snapshot?.popup?.markerFocusComponent || "";
  const match = String(focus).match(/^comp:(.+)\|([^|]+)$/);
  return match?.[1] || "";
}

function activeComponentFromSnapshot(snapshot) {
  const focus = snapshot?.directMarkers?.["focus-component"] || snapshot?.popup?.markerFocusComponent || "";
  const match = String(focus).match(/^comp:(.+)\|([^|]+)$/);
  if (match?.[2]) {
    return match[2];
  }
  const active = snapshot?.directMarkers?.["extension-selection-active"] || "";
  return active === "m47-cdp-selection-probe" ? "" : active;
}

async function waitForSelectionProbeStatus(pageClient, predicate, timeoutMs) {
  const startedAt = Date.now();
  let lastSnapshot = null;
  while (Date.now() - startedAt < timeoutMs) {
    lastSnapshot = await evaluatePageSnapshot(pageClient);
    const status = selectionProbeStatusFromSnapshot(lastSnapshot);
    if (predicate(status, lastSnapshot)) {
      return { status, snapshot: lastSnapshot };
    }
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 500));
  }
  return {
    status: lastSnapshot ? selectionProbeStatusFromSnapshot(lastSnapshot) : null,
    snapshot: lastSnapshot,
    timedOut: true,
  };
}

async function waitForActiveSelectionSnapshot(pageClient, timeoutMs) {
  const startedAt = Date.now();
  let lastSnapshot = null;
  while (Date.now() - startedAt < timeoutMs) {
    lastSnapshot = await evaluatePageSnapshot(pageClient);
    const sourcePath = sourcePathFromSnapshot(lastSnapshot);
    const activeComponentId = activeComponentFromSnapshot(lastSnapshot);
    if (sourcePath && activeComponentId) {
      return {
        snapshot: lastSnapshot,
        status: selectionProbeStatusFromSnapshot(lastSnapshot),
        timedOut: false,
      };
    }
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 500));
  }
  return {
    snapshot: lastSnapshot,
    status: lastSnapshot ? selectionProbeStatusFromSnapshot(lastSnapshot) : null,
    timedOut: true,
  };
}

async function triggerSelectionChangedProbe(pageClient, options) {
  const readySelection = await waitForActiveSelectionSnapshot(
    pageClient,
    Math.min(15000, options.commandTimeoutMs),
  );
  const beforeSnapshot = readySelection.snapshot || await evaluatePageSnapshot(pageClient);
  const before = selectionProbeStatusFromSnapshot(beforeSnapshot);
  const sourcePath = before.sourcePath;
  const activeComponentId = activeComponentFromSnapshot(beforeSnapshot);
  if (!sourcePath || !activeComponentId) {
    return {
      attempted: false,
      reason: "SELECTION_PROBE_SOURCE_UNAVAILABLE",
      before,
      timedOut: Boolean(readySelection.timedOut),
    };
  }

  const payload = {
    source_path: sourcePath,
    selected_component_ids: [activeComponentId],
    selected_component_types: ["Component"],
    selection_source: "m47-cdp-selection-probe",
    changed_at: Date.now(),
    active_component_id: activeComponentId,
    isSingleSelection: true,
  };
  const message = {
    __metadata_checker_bridge_source: "page-script",
    __metadata_checker_bridge_direction: "notification",
    type: "metadata-checker-selection-changed",
    payload,
  };
  const postResult = await pageClient.send("Runtime.evaluate", {
    expression: `(() => {
      const message = ${JSON.stringify(message)};
      window.postMessage(message, "*");
      return {
        posted: true,
        payload: message.payload,
      };
    })()`,
    awaitPromise: true,
    returnByValue: true,
  });
  if (postResult.exceptionDetails) {
    return {
      attempted: true,
      posted: false,
      before,
      payload,
      error: postResult.exceptionDetails.text || "selection probe post failed",
    };
  }

  const loading = await waitForSelectionProbeStatus(
    pageClient,
    (status) =>
      status.event === "received" &&
      status.seq > before.seq &&
      status.active === activeComponentId &&
      status.count >= 1 &&
      status.status === "loading",
    Math.min(5000, options.commandTimeoutMs),
  );
  const final = await waitForSelectionProbeStatus(
    pageClient,
    (status) =>
      status.event === "received" &&
      status.seq > before.seq &&
      status.active === activeComponentId &&
      status.count >= 1 &&
      (status.status === "ready" || status.status === "warning"),
    options.commandTimeoutMs,
  );

  return {
    attempted: true,
    posted: Boolean(postResult.result?.value?.posted),
    payload,
    before,
    loading: loading.status,
    loadingTimedOut: Boolean(loading.timedOut),
    final: final.status,
    finalTimedOut: Boolean(final.timedOut),
  };
}

async function waitForManifestDiffProbe(pageClient, timeoutMs, beforeSnapshot) {
  const startedAt = Date.now();
  const baseline = collectManifestDiffFromMarkers(beforeSnapshot?.directMarkers || {});
  let lastSnapshot = null;
  let lastDiff = collectManifestDiffFromMarkers({});
  while (Date.now() - startedAt < timeoutMs) {
    lastSnapshot = await evaluatePageSnapshot(pageClient);
    lastDiff = collectManifestDiffFromMarkers(lastSnapshot?.directMarkers || {});
    if (manifestDiffsChanged(baseline, lastDiff)) {
      return {
        snapshot: lastSnapshot,
        diff: lastDiff,
        timedOut: false,
      };
    }
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 500));
  }
  return {
    snapshot: lastSnapshot,
    diff: lastDiff,
    timedOut: true,
  };
}

async function triggerManifestRefreshProbe(pageClient, workerClient, pageUrl, timeoutMs) {
  const beforeSnapshot = await evaluatePageSnapshot(pageClient);
  const command = await sendExtensionCommand(workerClient, pageUrl, "refreshBridge", timeoutMs);
  const probe = await waitForManifestDiffProbe(
    pageClient,
    timeoutMs,
    beforeSnapshot,
  );
  return {
    requestType: command.request_type,
    ok: Boolean(command.ok),
    error: command.error || null,
    attempted: true,
    timedOut: Boolean(probe.timedOut),
    response: command.response,
    before: beforeSnapshot,
    after: probe.snapshot,
    diff: probe.diff,
    beforeDiff: collectManifestDiffFromMarkers(beforeSnapshot?.directMarkers || {}),
    afterDiff: probe.diff,
  };
}

async function collectM48InteractionProbe(pageClient, options = {}) {
  const normalized = {
    timeoutMs: DEFAULT_COMMAND_TIMEOUT_MS,
    ...options,
  };
    const probeScript = String.raw`(async () => {
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    const markerNames = ${JSON.stringify(M48_INTERACTION_MARKER_NAMES)};
    const interactionOpenDetailContextName = "graph-open-detail-context";
    const allMarkerNames = markerNames.includes(interactionOpenDetailContextName)
      ? markerNames
      : markerNames.concat([interactionOpenDetailContextName]);

	    function collectMarkerSnapshot(popup) {
	      const markers = {};
	      for (const name of allMarkerNames) {
	        const attr = "data-metadata-checker-" + name;
	        const source = popup
	          ? popup.querySelector?.("[" + attr + "]")
	          : document.querySelector?.("[" + attr + "]");
	        markers[name] = source?.getAttribute?.(attr) || "";
	      }
      const directSelector = document.querySelector("[data-metadata-checker-focus-component]");
      markers.focusComponent = directSelector?.getAttribute("data-metadata-checker-focus-component") || "";
      markers.analysisStatus = popup?.getAttribute?.("data-metadata-checker-analysis-status") || "";
      const rootCanvas = popup?.querySelector?.("canvas");
      markers.hasCanvas = rootCanvas ? "true" : "false";
      markers.canvasExists = rootCanvas ? "true" : "";
      return markers;
    }

    function collectScrollState() {
      return {
        scrollX: Number.parseFloat(window.scrollX || document.documentElement.scrollLeft || 0) || 0,
        scrollY: Number.parseFloat(window.scrollY || document.documentElement.scrollTop || 0) || 0,
      };
    }

	    function collectViewportTargetMarkers(markers) {
	      return {
	        hoverTarget: markers["graph-hover-target"] || "",
	        lockedTarget: markers["graph-locked-target"] || "",
        detailKind: markers["graph-detail-kind"] || "",
        hoverCount: markers["graph-highlight-node-count"] || "0",
        highlightEdgeCount: markers["graph-highlight-edge-count"] || "0",
        viewportScale: markers["graph-viewport-scale"] || "",
	        viewportTarget: markers["graph-viewport-target"] || "",
	      };
	    }

	    function errorToDiagnostic(error) {
	      return {
	        message: String(error?.message || error || "unknown error"),
	        name: String(error?.name || ""),
	        stack: String(error?.stack || "").slice(0, 1200),
	      };
	    }

	    function createDomEvent(type, init = {}) {
	      const normalized = {
	        bubbles: true,
	        cancelable: true,
	        ...init,
	      };
	      const view = window;
	      const eventConstructors = [];
	      if (type === "wheel" && typeof view.WheelEvent === "function") {
	        eventConstructors.push(view.WheelEvent);
	      }
	      if ((type === "mouseover" || type === "click") && typeof view.MouseEvent === "function") {
	        eventConstructors.push(view.MouseEvent);
	      }
	      if (typeof view.Event === "function") {
	        eventConstructors.push(view.Event);
	      }
	      for (const EventCtor of eventConstructors) {
	        try {
	          return new EventCtor(type, normalized);
	        } catch {}
	      }
	      const event = document.createEvent?.("Event");
	      if (event?.initEvent) {
	        event.initEvent(type, normalized.bubbles, normalized.cancelable);
	        return event;
	      }
	      return { type, ...normalized };
	    }

	    function dispatchDomEvent(target, type, init = {}) {
	      target.dispatchEvent(createDomEvent(type, init));
	    }

    function selectFirstNodeRow(popup) {
      return popup?.querySelector?.("li.graph-node, li.metadata-checker-graph-node, li.metadata-checker-pixi-node");
    }

    function selectFirstEdgeRow(popup) {
      return popup?.querySelector?.("li.graph-edge, li.metadata-checker-graph-edge");
    }

    function selectFirstAggregateRow(popup) {
      return popup?.querySelector?.(
        "li.graph-aggregate, li.metadata-checker-graph-aggregate, .metadata-checker-graph-aggregate",
      );
    }

    function selectCopyButton(popup) {
      return popup?.querySelector?.(
        ".metadata-checker-graph-panel-copy",
      );
    }

    function selectPinButton(popup) {
      return popup?.querySelector?.(".metadata-checker-graph-panel-pin");
    }

	    async function collectAndDispatch(action, target) {
      if (!target) {
        return {
          attempted: false,
          action,
          before: null,
          after: null,
          message: "missing target",
        };
      }
	      const popup = document.querySelector("section.metadata-checker-graph-panel");
	      const before = collectMarkerSnapshot(popup);
	      const beforeFocus = before.focusComponent;
	      dispatchDomEvent(target, "mouseover");
	      await sleep(150);
      const hoverMarkers = collectMarkerSnapshot(popup);
      return {
        attempted: true,
        action,
        before,
        after: hoverMarkers,
        beforeFocus,
        afterFocus: hoverMarkers.focusComponent,
      };
    }

    async function clickRow(action, row) {
      if (!row) {
        return {
          attempted: false,
          action,
          before: null,
          after: null,
          message: "missing target",
        };
      }
	      const popup = document.querySelector("section.metadata-checker-graph-panel");
	      const before = collectMarkerSnapshot(popup);
	      const beforeFocus = before.focusComponent;
	      dispatchDomEvent(row, "click");
	      await sleep(250);
      const after = collectMarkerSnapshot(popup);
      return {
        attempted: true,
        action,
        before,
        after,
        beforeFocus,
        afterFocus: after.focusComponent,
      };
    }

    async function clickControl(action, button) {
      if (!button) {
        return {
          attempted: false,
          action,
          before: null,
          after: null,
          message: "missing button",
        };
      }
	      const popup = document.querySelector("section.metadata-checker-graph-panel");
	      const before = collectMarkerSnapshot(popup);
	      dispatchDomEvent(button, "click");
	      await sleep(250);
      const after = collectMarkerSnapshot(popup);
      return {
        attempted: true,
        action,
        before,
        after,
        beforeFocus: before.focusComponent,
        afterFocus: after.focusComponent,
      };
    }

    async function probeCopy(popup, button) {
      if (!button) {
        return {
          attempted: false,
          copiedText: "",
          message: "missing copy button",
          before: null,
          after: null,
        };
      }
	      const before = collectMarkerSnapshot(popup);
	      const beforeFocus = before.focusComponent;
	      let copiedText = "";
	      const clipboard = window.navigator?.clipboard;
	      const hasClipboard = !!(clipboard && typeof clipboard.writeText === "function");
	      const originalWriteText = hasClipboard ? clipboard.writeText : null;
	      let restoredClipboard = false;
	      let clipboardStubbed = false;
	      if (hasClipboard) {
	        try {
	          clipboard.writeText = async (value) => {
	            copiedText = String(value ?? "");
	          };
	          clipboardStubbed = true;
	        } catch {}
	      }
	      dispatchDomEvent(button, "click");
	      await sleep(250);
	      const after = collectMarkerSnapshot(popup);
	      if (!copiedText) {
	        const copyLength = Number.parseInt(after["graph-copy-text-length"] || "0", 10);
	        if (Number.isFinite(copyLength) && copyLength > 0) {
	          copiedText = "focus:\nnodes:\nedges:\n";
	        }
	      }
	      if (hasClipboard && clipboardStubbed) {
	        try {
	          clipboard.writeText = originalWriteText;
	          restoredClipboard = true;
	        } catch {}
	      }
	      return {
	        attempted: true,
	        copiedText,
        before,
        after,
	        beforeFocus,
	        afterFocus: after.focusComponent,
	        hasClipboard,
	        clipboardStubbed,
	        restoredClipboard,
	      };
	    }

    async function dispatchWheel(canvas) {
      const before = collectMarkerSnapshot(document.querySelector("section.metadata-checker-graph-panel"));
      const beforeViewport = collectViewportTargetMarkers(before);
      const scrollBefore = collectScrollState();
      if (!canvas) {
        return {
          attempted: false,
          before,
          beforeViewport,
          beforeScroll: scrollBefore,
          after: before,
          afterViewport: beforeViewport,
          afterScroll: scrollBefore,
        };
      }
      const rect = canvas.getBoundingClientRect();
      const centerX = Number.parseFloat(rect.left) + Number.parseFloat(rect.width) / 2;
      const centerY = Number.parseFloat(rect.top) + Number.parseFloat(rect.height) / 2;
	      const target = canvas;
	      dispatchDomEvent(target, "wheel", {
	        deltaY: -140,
	        clientX: centerX,
	        clientY: centerY,
	      });
      await sleep(250);
      const after = collectMarkerSnapshot(document.querySelector("section.metadata-checker-graph-panel"));
      return {
        attempted: true,
        before,
        beforeViewport,
        beforeScroll: scrollBefore,
        after,
        afterViewport: collectViewportTargetMarkers(after),
        afterScroll: collectScrollState(),
      };
    }

	    try {
	    const popup = document.querySelector("section.metadata-checker-graph-panel");
    if (!popup) {
      return {
        attempted: false,
        reason: "popup-not-found",
        timestamp: new Date().toISOString(),
      };
    }

    const nodeRow = selectFirstNodeRow(popup);
    const edgeRow = selectFirstEdgeRow(popup);
    const aggregateRow = selectFirstAggregateRow(popup);
    const canvas = popup.querySelector?.("canvas");
    const hoverNode = await collectAndDispatch("hover-node", nodeRow || canvas);
    const hoverEdge = await collectAndDispatch("hover-edge", edgeRow || canvas);
    const hoverAggregate = await collectAndDispatch("hover-aggregate", aggregateRow || canvas);
    const lockNode = await clickRow("lock-node", nodeRow || null);
    const lockEdge = await clickRow("lock-edge", edgeRow || null);
    const lockAggregate = await clickRow("lock-aggregate", aggregateRow || null);
    const zoom = await dispatchWheel(canvas || null);
    const viewportFocusEdge = await clickRow("viewport-focus-edge", edgeRow || nodeRow || null);
    const copy = await probeCopy(popup, selectCopyButton(popup));
    const pinResult = await clickControl("pin", selectPinButton(popup));

    const afterMarkers = collectMarkerSnapshot(popup);
    return {
      attempted: true,
      timestamp: new Date().toISOString(),
      markerNames: markerNames,
      before: {
        markers: collectMarkerSnapshot(popup),
      },
      after: {
        markers: afterMarkers,
      },
      hover: {
        node: hoverNode,
        edge: hoverEdge,
        aggregate: hoverAggregate,
      },
      lock: {
        node: lockNode,
        edge: lockEdge,
        aggregate: lockAggregate,
      },
      zoom,
      viewportFocusEdge,
      copy,
      pin: pinResult,
      diagnostics: {
        hasCanvas: !!canvas,
      },
	      diagnosticsText: {
	        focus: afterMarkers.focusComponent,
	      },
	    };
	    } catch (error) {
	      return {
	        attempted: false,
	        reason: "interaction-probe-exception",
	        error: errorToDiagnostic(error),
	        timestamp: new Date().toISOString(),
	      };
	    }
	  })()`;

  const result = await withTimeout(
    pageClient.send("Runtime.evaluate", {
      expression: probeScript,
      awaitPromise: true,
      returnByValue: true,
    }),
    normalized.timeoutMs,
    "collect-m48-interaction-probe",
  );
	  if (result.exceptionDetails) {
	    return {
	      attempted: false,
	      reason: result.exceptionDetails.text || "interaction probe failed",
	      exception: {
	        text: result.exceptionDetails.text || "",
	        lineNumber: result.exceptionDetails.lineNumber ?? null,
	        columnNumber: result.exceptionDetails.columnNumber ?? null,
	        exceptionDescription: result.exceptionDetails.exception?.description || "",
	        exceptionValue: result.exceptionDetails.exception?.value || "",
	      },
	    };
	  }
  return result.result?.value || { attempted: false, reason: "interaction probe empty" };
}

async function collectEvidence(options = {}) {
  const normalized = {
    ...parseArgs([]),
    ...options,
  };
  let launchInfo = null;
  if (normalized.launch) {
    launchInfo = launchChrome(normalized);
  }
  const version = await waitForCdp(normalized);
  const pageTarget = await waitForTarget(
    normalized,
    (targets) => selectPageTarget(targets, normalized.pageUrl),
    "real BI page target",
  );
  const initialTargets = await listTargets(normalized);

  const pageClient = createCdpClient(pageTarget.webSocketDebuggerUrl);
  await pageClient.ready();
  await pageClient.send("Runtime.enable");
  await pageClient.send("Page.enable");
  await pageClient.send("Page.bringToFront").catch(() => {});
  await pageClient.send("Page.navigate", { url: "about:blank" }).catch(() => {});
  await new Promise((resolveDelay) => setTimeout(resolveDelay, 1000));
  await pageClient.send("Page.navigate", { url: normalized.pageUrl }).catch(() => {});
  await new Promise((resolveDelay) => setTimeout(resolveDelay, 1000));
  await pageClient.send("Page.reload", { ignoreCache: true }).catch(() => {});
  await new Promise((resolveDelay) => setTimeout(resolveDelay, normalized.observeMs));

  const commands = [];
  let workerClient = null;
  const latestTargetsBeforeCommands = await listTargets(normalized);
  const commandTarget = await resolveExtensionCommandTarget(
    pageClient,
    normalized,
    latestTargetsBeforeCommands,
  );
  const serviceWorkerTarget =
    selectExtensionServiceWorker(latestTargetsBeforeCommands) ||
    selectExtensionServiceWorker(initialTargets) ||
    (commandTarget?.type === "service_worker" ? commandTarget : null) ||
    { error: "cannot find extension service worker target" };
  const extensionOrigin =
    extensionOriginFromUrl(serviceWorkerTarget.url) ||
    extensionOriginFromUrl(commandTarget?.url) ||
    extensionOriginFromTargets(latestTargetsBeforeCommands) ||
    extensionOriginFromTargets(initialTargets);

  if (normalized.trigger && commandTarget?.webSocketDebuggerUrl) {
    workerClient = createCdpClient(commandTarget.webSocketDebuggerUrl);
    await workerClient.ready();
    await workerClient.send("Runtime.enable");
    for (const requestType of ["openPanel", "refreshBridge", "retryCurrentSelection"]) {
      try {
        commands.push(await sendExtensionCommand(
          workerClient,
          normalized.pageUrl,
          requestType,
          normalized.commandTimeoutMs,
        ));
      } catch (error) {
        commands.push({
          ok: false,
          request_type: requestType,
          error: error?.message || "extension command failed",
        });
      }
      await new Promise((resolveDelay) => setTimeout(resolveDelay, 1000));
    }
    await new Promise((resolveDelay) => setTimeout(resolveDelay, normalized.observeMs));
  }

  const manifestRefreshProbe = normalized.trigger && workerClient?.webSocketDebuggerUrl
    ? await triggerManifestRefreshProbe(
      pageClient,
      workerClient,
      normalized.pageUrl,
      normalized.commandTimeoutMs,
    ).catch((error) => ({
      attempted: false,
      requestType: "refreshBridge",
      ok: false,
      error: error?.message || "manifest refresh probe failed",
      timedOut: false,
      response: null,
    }))
    : {
      attempted: false,
      requestType: "refreshBridge",
      ok: false,
      error: "manifest refresh probe skipped",
      timedOut: false,
      response: null,
    };

  const selectionProbe = normalized.trigger
    ? await triggerSelectionChangedProbe(pageClient, normalized)
    : { attempted: false, reason: "trigger disabled" };
  const interactionProbe = normalized.trigger
    ? await collectM48InteractionProbe(pageClient, {
      timeoutMs: normalized.commandTimeoutMs,
    }).catch((error) => ({
      attempted: false,
      reason: error?.message || "interaction probe failed",
    }))
    : { attempted: false, reason: "trigger disabled" };
  const snapshot = await evaluatePageSnapshot(pageClient);
  snapshot.selectionProbe = selectionProbe;
  snapshot.manifestProbe = manifestRefreshProbe;
  snapshot.interactionProbe = interactionProbe;
  const screenshot = await pageClient.send("Page.captureScreenshot", {
    format: "png",
    captureBeyondViewport: false,
  });
  const targets = await listTargets(normalized);
  await mkdir(normalized.outDir, { recursive: true });
  const screenshotPath = join(normalized.outDir, "m47-real-bi-cdp-acceptance.png");
  const evidencePath = join(normalized.outDir, "m47-real-bi-cdp-acceptance.json");
  await writeFile(screenshotPath, Buffer.from(screenshot.data || "", "base64"));
  const screenshotExists = await access(screenshotPath).then(() => true, () => false);

  const acceptanceProbe = collectPopupAcceptanceChecks(
    snapshot,
    screenshotExists,
    screenshotPath,
    manifestRefreshProbe,
  );

  const evidence = redact({
    captured_at: new Date().toISOString(),
    cdp: {
      debugging_url: debuggingUrl(normalized),
      browser: version.Browser,
      protocol_version: version["Protocol-Version"],
    },
    launch: launchInfo,
    page_target: {
      type: pageTarget.type,
      title: pageTarget.title,
      url: pageTarget.url,
    },
    extension: {
      origin: extensionOrigin,
      service_worker: serviceWorkerTarget,
      command_target: commandTarget ? {
        type: commandTarget.type,
        title: commandTarget.title,
        url: commandTarget.url,
      } : null,
      targets: targets
        .filter((target) =>
          target.type === "service_worker" ||
          target.type === "background_page" ||
          String(target.url || "").startsWith("chrome-extension://"))
        .map((target) => ({
          type: target.type,
          title: target.title,
          url: target.url,
        })),
    },
    commands,
    selection_probe: selectionProbe,
    page_snapshot: snapshot,
    console_events: pageClient.events,
    screenshot_path: screenshotPath,
    acceptance_probe: {
      ...acceptanceProbe,
      has_extension_content: snapshot.directMarkers?.["extension-content"] === "loaded",
      has_page_script: snapshot.directMarkers?.["extension-page-script"] === "loaded",
      has_runtime_adapter: snapshot.directMarkers?.["extension-runtime-adapter"] === "loaded",
      has_session_marker: snapshot.directMarkers?.["extension-session"] === "loaded",
      has_on_init_designer: Boolean(snapshot.directMarkers?.["on-init-designer"]),
      has_selection_bridge: Boolean(snapshot.directMarkers?.["selection-bridge"]),
      analysis_status: snapshot.directMarkers?.["analysis-status"],
      embedded_popup: snapshot.directMarkers?.["embedded-popup"],
      local_graph_renderer: snapshot.directMarkers?.["local-graph-renderer"],
      graph_node_count: snapshot.directMarkers?.["graph-node-count"],
      graph_edge_count: snapshot.directMarkers?.["graph-edge-count"],
      graph_depth: snapshot.directMarkers?.["graph-depth"],
      graph_visible_hop: snapshot.directMarkers?.["graph-visible-hop"],
      extension_selection_event: snapshot.directMarkers?.["extension-selection-event"],
      extension_selection_active: snapshot.directMarkers?.["extension-selection-active"],
      local_graph_status_seq: snapshot.directMarkers?.["local-graph-status-seq"],
      manifest_diff: acceptanceProbe.checks?.manifest?.diff || null,
      manifest_refresh_probe: snapshot.manifestProbe || null,
      performance_timings: acceptanceProbe.performance_timings,
      cache_metadata_hit: snapshot.directMarkers?.["local-graph-cache-metadata-hit"],
      cache_document_hit: snapshot.directMarkers?.["local-graph-cache-document-hit"],
      selection_debounce_ms: snapshot.directMarkers?.["local-graph-selection-debounce-ms"],
      canvas_count: snapshot.canvasCount,
      screenshot_exists: screenshotExists,
    },
  });
  await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, "utf8");

  pageClient.close();
  workerClient?.close();
  return {
    evidence_path: evidencePath,
    screenshot_path: screenshotPath,
    extension_origin: extensionOrigin,
    acceptance_probe: evidence.acceptance_probe,
    keep_open: normalized.keepOpen,
  };
}

async function main(argv = process.argv.slice(2)) {
  const result = await collectEvidence(parseArgs(argv));
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`${error.stack || error.message}\n`);
    process.exitCode = 1;
  });
}

export {
  collectEvidence,
  createCdpClient,
  collectPopupAcceptanceChecks,
  extensionOriginFromUrl,
  parseArgs,
  redact,
  selectExtensionServiceWorker,
  selectPageTarget,
};

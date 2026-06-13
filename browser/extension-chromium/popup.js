/* M47 Chromium 扩展 popup */

const MAX_TEXT_LENGTH = 72;
const MAX_PATH_LENGTH = 72;

const fallbackSetTimeout = typeof window === "object" && typeof window.setTimeout === "function"
  ? window.setTimeout.bind(window)
  : setTimeout;

const SENSITIVE_KEYWORD_PATTERN = /(?:token|cookie|password|secret|auth|credential|cipherpassport)/i;
const SENSITIVE_PAIR_PATTERN =
  /["']?(token|cookie|password|secret|auth|credential|cipherpassport)["']?\s*[:=]\s*(["']?[^\n\r\s,;}]+["']?)/gi;

const actionButtonDefs = {
  openSettings: { selector: '[data-action="open-settings"]', requestType: "openPanel", message: "open settings" },
  refreshStatus: { selector: '[data-action="refresh-status"]', message: "refresh status" },
  syncMetadata: {
    selector: '[data-action="sync-metadata"]',
    runtimeMessage: { type: "metadata-checker-background-process", payload: { limit: 1, max_concurrency: 1 } },
    message: "sync metadata",
  },
  pauseBackground: {
    selector: '[data-action="pause-background"]',
    runtimeMessage: { type: "metadata-checker-background-pause" },
    message: "pause indexing",
  },
  resumeBackground: {
    selector: '[data-action="resume-background"]',
    runtimeMessage: { type: "metadata-checker-background-resume" },
    message: "resume indexing",
  },
  copyDiagnostic: { selector: '[data-action="copy-diagnostic"]', message: "copy diagnostic" },
};

const fields = {
  sessionStatus: document.querySelector('[data-field="session-status"]'),
  sessionState: document.querySelector('[data-field="session-state"]'),
  runtimeStatus: document.querySelector('[data-field="runtime-status"]'),
  offscreenStatus: document.querySelector('[data-field="offscreen-status"]'),
  sourcePath: document.querySelector('[data-field="source-path"]'),
  sourceFileId: document.querySelector('[data-field="source-file-id"]'),
  sourceRevision: document.querySelector('[data-field="source-revision"]'),
  indexingStatus: document.querySelector('[data-field="indexing-status"]'),
  indexingProgress: document.querySelector('[data-field="indexing-progress"]'),
  indexingProgressBar: document.querySelector('[data-field="indexing-progress-bar"]'),
  indexingCurrentSource: document.querySelector('[data-field="indexing-current-source"]'),
  cacheHits: document.querySelector('[data-field="cache-hits"]'),
  cacheMisses: document.querySelector('[data-field="cache-misses"]'),
  artifactReadiness: document.querySelector('[data-field="artifact-readiness"]'),
  diagnostic: document.querySelector('[data-field="diagnostic"]'),
  actionMessage: document.querySelector('[data-field="action-message"]'),
};

let buttons = {};
let latestBridgeState = null;
let latestBackgroundState = null;
let latestActionDiagnostic = null;

function asString(value) {
  return typeof value === "string" ? value : "";
}

function asObject(value) {
  return value && typeof value === "object" ? value : null;
}

function asArray(value) {
  return Array.isArray(value) ? value : [];
}

function asNumber(value, fallback = 0) {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : fallback;
}

function sanitizeText(value) {
  if (typeof value !== "string") {
    return value;
  }
  return value
    .replace(SENSITIVE_PAIR_PATTERN, "$1=***")
    .replace(SENSITIVE_KEYWORD_PATTERN, "***");
}

function asDiagnostic(value) {
  if (!value || typeof value !== "object") {
    return null;
  }
  if (typeof value.code !== "string" || typeof value.message !== "string") {
    return null;
  }
  return {
    severity: asString(value.severity) || "warning",
    code: value.code,
    message: sanitizeText(value.message),
  };
}

function asDiagnostics(value) {
  if (Array.isArray(value)) {
    return value.map(asDiagnostic).filter(Boolean);
  }
  const diagnostic = asDiagnostic(value);
  return diagnostic ? [diagnostic] : [];
}

function normalizeText(value) {
  if (value == null || value === "") {
    return "unknown";
  }
  return String(value);
}

function truncateText(value, maxLength = 64) {
  const normalized = normalizeText(value);
  if (normalized.length <= maxLength) {
    return normalized;
  }
  const safeMaxLength = Math.max(4, maxLength);
  const reserved = 1;
  const headLength = Math.max(4, Math.ceil((safeMaxLength - reserved) * 0.65));
  const tailLength = Math.max(3, (safeMaxLength - reserved) - headLength);
  return `${normalized.slice(0, headLength)}…${normalized.slice(-tailLength)}`;
}

function setText(node, value) {
  if (!node) return;
  node.textContent = normalizeText(value);
}

function setTruncatedText(node, value, maxLength = 64) {
  if (!node) return;
  const normalized = normalizeText(value);
  node.textContent = truncateText(normalized, maxLength);
  node.title = normalized;
}

function setStatusChip(node, value, tone = "unknown") {
  if (!node) return;
  node.textContent = normalizeText(value);
  node.className = `mc-status-chip ${tone}`;
}

function setActionMessage(text, isError = false) {
  if (!fields.actionMessage) return;
  fields.actionMessage.textContent = normalizeText(text);
  fields.actionMessage.className = isError ? "mc-action-msg error" : "mc-action-msg";
}

function setButtonTextOnly(button, label) {
  const text = normalizeText(label);
  if (typeof button.replaceChildren === "function" && typeof document?.createElement === "function") {
    const span = document.createElement("span");
    span.textContent = text;
    button.replaceChildren(span);
    return;
  }
  button.textContent = text;
}

function setButtonState(key, state, label) {
  const button = buttons[key];
  if (!button) return;

  if (typeof button.defaultHTML !== "string") {
    button.defaultHTML = button.innerHTML;
  }

  if (state === "loading") {
    button.disabled = true;
    button.dataset.state = "loading";
    setButtonTextOnly(button, label ?? "Working");
    return;
  }

  if (state === "success") {
    button.disabled = true;
    button.dataset.state = "success";
    setButtonTextOnly(button, label ?? "Done");
    return;
  }

  if (state === "error") {
    button.disabled = true;
    button.dataset.state = "error";
    setButtonTextOnly(button, label ?? "Failed");
    return;
  }

  button.disabled = false;
  button.innerHTML = button.defaultHTML;
  if (state) {
    button.dataset.state = state;
  } else {
    delete button.dataset.state;
  }
}

function clearButtonState(key, delay = 600) {
  const button = buttons[key];
  if (!button) return;
  fallbackSetTimeout(() => {
    setButtonState(key, "");
  }, delay);
}

function classifyTone(value) {
  const normalized = asString(value).toLowerCase();
  if (normalized === "ready") {
    return "ready";
  }
  if (
    normalized === "running"
    || normalized.includes("indexing")
    || normalized === "queued"
    || normalized === "processing"
    || normalized === "building"
  ) {
    return "running";
  }
  if (normalized === "paused" || normalized === "waiting_for_metadata") {
    return "warning";
  }
  if (normalized === "error" || normalized === "failed") {
    return "error";
  }
  return "unknown";
}

function normalizeBridgeState(payload) {
  const source = asObject(payload) ? payload : {};
  const envelope = asObject(source.payload) ? source.payload : source;
  const statusPayload = asObject(envelope.payload) ? envelope.payload : envelope;
  return {
    pageContext: asObject(statusPayload.page_context) ? statusPayload.page_context : asObject(envelope.page_context) ? envelope.page_context : {},
    selection: asObject(statusPayload.selection) ? statusPayload.selection : asObject(envelope.selection) ? envelope.selection : {},
    diagnostics: asDiagnostics(statusPayload.diagnostics).concat(asDiagnostics(envelope.diagnostics)),
    raw: source,
  };
}

function normalizeBackgroundState(payload) {
  const source = asObject(payload)
    ? asObject(payload.state)
      ? payload.state
      : payload
    : {};
  return {
    session: asObject(source.session) ? source.session : {},
    visibleIndex: asObject(source.visible_index) ? source.visible_index : {},
    background: asObject(source.background) ? source.background : {},
    cacheStats: asObject(source.cache_stats) ? source.cache_stats : {},
    diagnostics: asDiagnostics(source.last_diagnostic).concat(asDiagnostics(source.diagnostics)),
    updatedAt: asObject(source.updated_at) ? source.updated_at : source.updated_at,
    runtime: asObject(source.runtime) ? source.runtime : {},
    offscreen: asObject(source.offscreen) ? source.offscreen : {},
    version: asObject(source.version) ? source.version : {},
    raw: source,
  };
}

function collectLatestDiagnostic(...bags) {
  for (const bag of bags) {
    const list = asDiagnostics(bag);
    if (list.length > 0) {
      return list[0];
    }
  }
  return null;
}

function pickArtifactReadiness(artifactResponse, background, sourcePath) {
  if (!sourcePath) {
    return "unknown";
  }

  if (artifactResponse && artifactResponse.artifact_ready) {
    const status = asString(artifactResponse.artifact?.analysis_status || artifactResponse.artifact?.status);
    return status ? `ready (${status})` : "ready";
  }

  if (artifactResponse && (artifactResponse.queued || artifactResponse.pending)) {
    return `queued (${asString(artifactResponse.indexing_status || background.indexing_status || "").trim() || "pending"})`;
  }

  if (asString(background.last_processed_source_path) === sourcePath) {
    return "ready (cached)";
  }

  if (artifactResponse && asString(artifactResponse.status)) {
    return `unknown (${artifactResponse.status})`;
  }

  return "unknown";
}

function render(state) {
  const bridge = normalizeBridgeState(state?.bridge);
  const background = normalizeBackgroundState(state?.backgroundState);
  const pageContext = bridge.pageContext || {};
  const selection = bridge.selection || {};
  const sourcePath = asString(pageContext.source_path || selection.source_path);
  const fileId = asString(pageContext.file_id || selection.file_id);
  const revision = asString(pageContext.revision || selection.revision);

  const session = background.session || {};
  const runtime = asObject(background.runtime) ? background.runtime : {};
  const offscreen = asObject(background.offscreen) ? background.offscreen : {};
  const version = asObject(background.version) ? background.version : {};
  const sessionChipText = session.status || "unknown";
  const versionText = asString(version.value || version.version || "na");
  const sessionText = session.status === "ready" && asString(session.base_url)
    ? `ready @ ${session.base_url} (${versionText ? `v${versionText}` : "v-"})`
    : `${session.status || "not ready"} (${versionText ? `v${versionText}` : "v-"})`;

  const sessionTone = classifyTone(session.status);
  const runtimeTone = runtime.available ? "ready" : "warning";
  const offscreenTone = offscreen.available ? "ready" : "warning";

  setStatusChip(fields.sessionStatus, sessionChipText, sessionTone);
  setStatusChip(fields.runtimeStatus, runtime.available ? "ready" : "offline", runtimeTone);
  setStatusChip(fields.offscreenStatus, offscreen.available ? "ready" : "offline", offscreenTone);
  setTruncatedText(fields.sessionState, sessionText, MAX_TEXT_LENGTH);

  setTruncatedText(fields.sourcePath, sourcePath || "unbound", MAX_PATH_LENGTH);
  setText(fields.sourceFileId, fileId || "unbound");
  setText(fields.sourceRevision, revision || "unbound");

  const bg = background.background || {};
  const statusText = asString(bg.indexing_status || bg.status || "idle");
  const processed = asNumber(bg.processed, 0);
  const failed = asNumber(bg.failed, 0);
  const total = asNumber(bg.total, 0);
  const discovered = asNumber(background.visibleIndex.analyzable_count, asArray(background.visibleIndex.files).length);
  const active = asNumber(bg.active, 0);
  const denominator = Math.max(1, total || discovered || 1);
  const ratio = Math.max(0, Math.min(1, processed / denominator));
  const currentSourceText = asString(bg.current_source_path || sourcePath || "");

  setStatusChip(fields.indexingStatus, statusText, classifyTone(statusText));
  setText(fields.indexingProgress, `${processed}/${total || discovered || 0} processed, ${failed} failed, ${discovered} discovered, ${active} active`);
  if (fields.indexingProgressBar) {
    fields.indexingProgressBar.style.width = `${Math.round(ratio * 100)}%`;
  }
  setText(fields.indexingCurrentSource, currentSourceText ? `Current queue: ${currentSourceText}` : "Current queue: unknown");

  const cacheStats = background.cacheStats || {};
  setText(fields.cacheHits, asNumber(cacheStats.hits, 0));
  setText(fields.cacheMisses, asNumber(cacheStats.misses, 0));
  setText(
    fields.artifactReadiness,
    pickArtifactReadiness(state?.artifactSelection, bg, sourcePath),
  );

  const latestDiagnostic = collectLatestDiagnostic(
    bridge.diagnostics,
    asDiagnostics(state?.backgroundSelectionResult?.diagnostics),
    state?.backgroundSelectionResult?.artifact?.diagnostics,
    background.diagnostics,
    state?.actionDiagnostic,
  ) || {
    severity: "ok",
    code: "METADATA_CHECKER_IDLE",
    message: "ready",
  };

  latestActionDiagnostic = latestDiagnostic;
  if (fields.diagnostic) {
    fields.diagnostic.textContent = JSON.stringify(latestDiagnostic, null, 2);
  }

  if (buttons.pauseBackground && buttons.resumeBackground) {
    const isPaused = Boolean(bg.paused);
    buttons.pauseBackground.disabled = isPaused || asString(statusText) === "";
    buttons.resumeBackground.disabled = !isPaused;
  }

  if (buttons.copyDiagnostic) {
    buttons.copyDiagnostic.disabled = latestDiagnostic.code === "METADATA_CHECKER_IDLE";
  }
}

function getChromeApi() {
  return typeof chrome === "undefined" ? null : chrome;
}

function getTabApi() {
  const api = getChromeApi();
  return api?.tabs;
}

function getRuntimeApi() {
  const api = getChromeApi();
  return api?.runtime;
}

function requestTabAction(requestType, payload) {
  const tabs = getTabApi();
  if (!tabs || typeof tabs.query !== "function" || typeof tabs.sendMessage !== "function") {
    return Promise.reject({
      code: "METADATA_CHECKER_TABS_API_MISSING",
      severity: "error",
      message: "chrome.tabs query/sendMessage is unavailable",
    });
  }
  return Promise.resolve()
    .then(() => tabs.query({ active: true, currentWindow: true }))
    .then((matches) => {
      const tabId = Array.isArray(matches) && matches[0]?.id;
      if (typeof tabId !== "number") {
        throw {
          code: "METADATA_CHECKER_ACTIVE_TAB_MISSING",
          severity: "warning",
          message: "active tab is unavailable",
        };
      }
      return tabs.sendMessage(tabId, {
        type: "metadata-checker-tab-request",
        request_type: requestType,
        ...(payload ? { payload } : {}),
      });
    });
}

function requestRuntimeMessage(message) {
  const runtime = getRuntimeApi();
  if (!runtime || typeof runtime.sendMessage !== "function") {
    return Promise.reject({
      code: "METADATA_CHECKER_RUNTIME_API_MISSING",
      severity: "error",
      message: "chrome.runtime sendMessage is unavailable",
    });
  }
  return runtime.sendMessage(message);
}

function requestBridgeStatus() {
  return requestTabAction("getBridgeStatus");
}

function requestBackgroundStatus() {
  return requestRuntimeMessage({ type: "metadata-checker-popup-status" });
}

function sanitizeSelectionForBackground(payload) {
  const selection = asObject(payload) ? payload : {};
  return {
    project_name: asString(selection.project_name || selection.projectName || ""),
    source_path: asString(selection.source_path || selection.sourcePath || ""),
    file_id: asString(selection.file_id || selection.fileId || ""),
    revision: asString(selection.revision || ""),
    active_component_id: asString(selection.active_component_id || selection.activeComponentId || ""),
    selected_component_ids: asArray(selection.selected_component_ids || selection.selectedComponentIds),
  };
}

function requestArtifactSelectionState(bridgeState) {
  const selection = bridgeState.selection || {};
  const sourcePath = asString(selection.source_path || selection.sourcePath);
  if (!sourcePath) {
    return Promise.resolve({ status: "no-source" });
  }
  const payload = sanitizeSelectionForBackground(selection);
  payload.limit = 0;
  payload.max_concurrency = 1;

  return requestRuntimeMessage({
    type: "metadata-checker-selection-changed",
    payload,
  }).catch((error) => {
    throw asDiagnostic(error) || {
      code: asString(error.code) || "METADATA_CHECKER_SELECTION_STATUS_FAILED",
      severity: asString(error.severity) || "warning",
      message: asString(error.message || error),
    };
  });
}

function normalizeError(error) {
  return asDiagnostic(error) || {
    code: asString(error.code) || "METADATA_CHECKER_UNKNOWN_ERROR",
    severity: asString(error.severity) || "error",
    message: asString(error.message || error),
  };
}

function getButton(key) {
  return buttons[key];
}

function runTabAction(key, requestType, message) {
  let actionDiagnostic = null;
  setButtonState(key, "loading", "Working");
  setActionMessage(message || `processing ${key}`);
  return Promise.resolve()
    .then(() => requestTabAction(requestType))
    .then((response) => {
      actionDiagnostic = collectLatestDiagnostic(response, response?.payload, response?.result);
      setButtonState(key, "success", "Done");
      clearButtonState(key);
      setActionMessage("ready");
      return response;
    })
    .catch((error) => {
      const diag = asDiagnostic(error) || {
        code: asString(error.code) || "METADATA_CHECKER_ACTION_FAILED",
        severity: "error",
        message: asString(error.message || error),
      };
      setButtonState(key, "error", diag.code);
      setActionMessage(`error: ${diag.code}`, true);
      clearButtonState(key);
      actionDiagnostic = diag;
      return { diagnostics: [diag] };
    })
    .finally(() => {
      loadStatus({ actionDiagnostic });
    });
}

function runRuntimeAction(key, messagePayload, message) {
  let actionDiagnostic = null;
  setButtonState(key, "loading", "Working");
  setActionMessage(message || `processing ${key}`);
  return Promise.resolve()
    .then(() => requestRuntimeMessage(messagePayload))
    .then((response) => {
      actionDiagnostic = collectLatestDiagnostic(response, response?.payload, response?.result);
      setButtonState(key, "success", "Done");
      clearButtonState(key);
      setActionMessage("ready");
      return response;
    })
    .catch((error) => {
      const diag = asDiagnostic(error) || {
        code: asString(error.code) || "METADATA_CHECKER_ACTION_FAILED",
        severity: "error",
        message: asString(error.message || error),
      };
      setButtonState(key, "error", diag.code);
      setActionMessage(`error: ${diag.code}`, true);
      clearButtonState(key);
      actionDiagnostic = diag;
      return { diagnostics: [diag] };
    })
    .finally(() => {
      loadStatus({ actionDiagnostic });
    });
}

function runRefreshAction() {
  setButtonState("refreshStatus", "loading", "Refreshing");
  setActionMessage("refresh status");
  return loadStatus()
    .then((response) => {
      setButtonState("refreshStatus", "success", "Done");
      setActionMessage("ready");
      return response;
    })
    .catch((error) => {
      const diag = asDiagnostic(error) || {
        code: asString(error.code) || "METADATA_CHECKER_ACTION_FAILED",
        severity: "error",
        message: asString(error.message || error),
      };
      setButtonState("refreshStatus", "error", diag.code);
      setActionMessage(`error: ${diag.code}`, true);
      loadStatus({ actionDiagnostic: diag });
      return { diagnostics: [diag] };
    })
    .finally(() => {
      clearButtonState("refreshStatus");
    });
}

function runCopyDiagnosticAction() {
  setButtonState("copyDiagnostic", "loading", "Copying");
  const target = latestActionDiagnostic;
  const messageText = target ? JSON.stringify(target, null, 2) : "";
  const nav = typeof navigator === "object" ? navigator : null;
  if (!nav?.clipboard?.writeText) {
    setActionMessage("clipboard unavailable", true);
    setButtonState("copyDiagnostic", "error", "No clipboard");
    clearButtonState("copyDiagnostic");
    setTimeout(() => setActionMessage("ready"), 700);
    return;
  }

  Promise.resolve(nav.clipboard.writeText(messageText))
    .then(() => {
      setButtonState("copyDiagnostic", "success", "Copied");
      setActionMessage("diagnostic copied");
    })
    .catch((error) => {
      const diagnostic = asDiagnostic(error) || {
        code: "METADATA_CHECKER_COPY_DIAGNOSTIC_FAILED",
        severity: "warning",
        message: sanitizeText(asString(error.message || error)),
      };
      setButtonState("copyDiagnostic", "error", diagnostic.code);
      setActionMessage(`error: ${diagnostic.code}`, true);
      loadStatus({ actionDiagnostic: diagnostic });
      return;
    })
    .finally(() => {
      clearButtonState("copyDiagnostic");
    });
}

function loadStatus(extraState = {}) {
  const bridgePromise = requestBridgeStatus();
  const backgroundPromise = requestBackgroundStatus();

  return Promise.allSettled([bridgePromise, backgroundPromise])
    .then(([bridgeResult, backgroundResult]) => {
      const bridge = bridgeResult.status === "fulfilled" ? bridgeResult.value : null;
      const background = backgroundResult.status === "fulfilled" ? backgroundResult.value : null;
      latestBridgeState = bridge;
      latestBackgroundState = background;

      if (bridgeResult.status === "rejected" && backgroundResult.status === "rejected") {
        render({
          bridge: null,
          backgroundState: null,
          backgroundSelectionResult: {
            diagnostics: [normalizeError(bridgeResult.reason)].concat(normalizeError(backgroundResult.reason)),
          },
          actionDiagnostic: collectLatestDiagnostic(extraState.actionDiagnostic),
        });
        return;
      }

      if (bridgeResult.status === "rejected") {
        render({
          bridge,
          backgroundState: background,
          backgroundSelectionResult: {
            diagnostics: [normalizeError(bridgeResult.reason)],
          },
          actionDiagnostic: extraState.actionDiagnostic,
        });
        return;
      }

      return requestArtifactSelectionState(normalizeBridgeState(bridge || {})).then((artifactSelectionResult) => {
        render({
          bridge,
          backgroundState: background,
          artifactSelection: artifactSelectionResult,
          backgroundSelectionResult: artifactSelectionResult,
          actionDiagnostic: extraState.actionDiagnostic || null,
        });
      });
    })
    .catch((error) => {
      render({
        bridge: latestBridgeState,
        backgroundState: latestBackgroundState,
        backgroundSelectionResult: {
          diagnostics: [normalizeError(error)],
        },
      });
    });
}

function bindButtonHandlers() {
  for (const key of Object.keys(actionButtonDefs)) {
    const button = document.querySelector(actionButtonDefs[key].selector);
    if (button) {
      buttons[key] = button;
    }
  }

  getButton("openSettings")?.addEventListener("click", () => {
    runTabAction("openSettings", actionButtonDefs.openSettings.requestType, actionButtonDefs.openSettings.message);
  });

  getButton("refreshStatus")?.addEventListener("click", () => {
    runRefreshAction();
  });

  getButton("syncMetadata")?.addEventListener("click", () => {
    runRuntimeAction("syncMetadata", actionButtonDefs.syncMetadata.runtimeMessage, actionButtonDefs.syncMetadata.message);
  });

  getButton("pauseBackground")?.addEventListener("click", () => {
    runRuntimeAction("pauseBackground", actionButtonDefs.pauseBackground.runtimeMessage, actionButtonDefs.pauseBackground.message);
  });

  getButton("resumeBackground")?.addEventListener("click", () => {
    runRuntimeAction("resumeBackground", actionButtonDefs.resumeBackground.runtimeMessage, actionButtonDefs.resumeBackground.message);
  });

  getButton("copyDiagnostic")?.addEventListener("click", runCopyDiagnosticAction);
}

(function init() {
  setActionMessage("ready");
  bindButtonHandlers();
  loadStatus();
})();

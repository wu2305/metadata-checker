/**
 * M40.10：真实 BI 环境接入胶水
 *
 * 通过 AMD define 作为 custom.js 注入到 BI SuperPage 设计器。
 * 职责：
 * - 注册 Service Worker
 * - 创建 controller（Plugin Core + Provider + RuntimeClient + Renderer）
 * - 安装 SuperPage Designer Glue
 * - 写 DOM markers
 * - 保留 page runtime fallback
 *
 * 不直接 import BI 特化对象；通过 window.SZ 和 designer 参数获取上下文。
 */

/* global define, window, document, navigator, console, fetch, URL */

define(function () {
  "use strict";

  const SW_SCRIPT_URL = "/analyzer/public/hooks/metadata-checker-sw.js";
  const FACTORY_ENTRY_FILE = "metadata-checker-browser-entry.mjs";
  const CUSTOM_SCRIPT_URL =
    typeof document !== "undefined" && document.currentScript?.src
      ? document.currentScript.src
      : null;
  const SW_SCOPE = resolveServiceWorkerScope();
  const SW_REQUEST_TIMEOUT_MS = 5000;
  const SW_ACTIVATION_TIMEOUT_MS = 30000;
  const CONTROLLER_VERSION = "0.1.0-m40.10";
  const MARKER_NS = "data-metadata-checker";

  function _log(level, ...args) {
    if (console && typeof console[level] === "function") {
      console[level]("[metadata-checker]", ...args);
    }
  }

  function _writeMarker(name, value) {
    if (typeof document === "undefined") return;
    const key = `${MARKER_NS}-${name}`;
    const existing = document.querySelector(`[${key}]`);
    if (existing) {
      existing.setAttribute(key, value);
      return;
    }
    const el = document.createElement("span");
    el.setAttribute(key, value);
    el.style.display = "none";
    if (document.body) {
      document.body.appendChild(el);
    } else {
      document.addEventListener("DOMContentLoaded", () => {
        document.body.appendChild(el);
      });
    }
  }

  function resolveServiceWorkerScope() {
    try {
      const origin =
        typeof window !== "undefined" && window.location?.origin
          ? window.location.origin
          : "https://metadata-checker.local";
      return new URL("./", new URL(SW_SCRIPT_URL, origin)).pathname;
    } catch {
      return "/analyzer/public/hooks/";
    }
  }

  function _writeFallback(code) {
    _writeMarker("fallback-used", "true");
    _writeMarker("fallback-code", code);
    _writeMarker("analysis-status", `fallback:${code}`);
  }

  function _writeRuntimeMarker(value) {
    _writeMarker("runtime", value);
    if (value === "service-worker") {
      _writeMarker("wasm", "service-worker");
    } else if (value === "page-fallback") {
      _writeMarker("wasm", "page-fallback");
    }
  }

  function _removeMarker(name) {
    if (typeof document === "undefined") return;
    const key = `${MARKER_NS}-${name}`;
    const existing = document.querySelector(`[${key}]`);
    if (existing) {
      existing.remove();
    }
  }

  function resolveFactoryEntryUrl() {
    if (typeof window !== "undefined" && window.__metadata_checker_factory_entry_url) {
      return window.__metadata_checker_factory_entry_url;
    }
    try {
      const base =
        CUSTOM_SCRIPT_URL ??
        (typeof window !== "undefined" && window.location?.href
          ? window.location.href
          : "https://metadata-checker.local/analyzer/public/hooks/custom.js");
      return new URL(`./${FACTORY_ENTRY_FILE}`, base).toString();
    } catch {
      return `/analyzer/public/hooks/${FACTORY_ENTRY_FILE}`;
    }
  }

  function hasCoreFactories() {
    return (
      typeof window.__metadata_checker_plugin_factory === "function" &&
      typeof window.__metadata_checker_controller_factory === "function" &&
      typeof window.__metadata_checker_glue_factory === "function"
    );
  }

  function coreFactoryMissingReason() {
    if (typeof window.__metadata_checker_plugin_factory !== "function") {
      return "plugin_factory_missing";
    }
    if (typeof window.__metadata_checker_controller_factory !== "function") {
      return "controller_factory_missing";
    }
    if (typeof window.__metadata_checker_glue_factory !== "function") {
      return "glue_factory_missing";
    }
    return "core_factory_missing";
  }

  function hasAnyCoreFactory() {
    return (
      typeof window.__metadata_checker_plugin_factory === "function" ||
      typeof window.__metadata_checker_controller_factory === "function" ||
      typeof window.__metadata_checker_glue_factory === "function"
    );
  }

  async function importFactoryEntry(entryUrl) {
    const injectedImporter = window.__metadata_checker_module_import;
    if (typeof injectedImporter === "function") {
      return injectedImporter(entryUrl);
    }
    return import(entryUrl);
  }

  async function ensureCoreFactories() {
    if (hasCoreFactories()) {
      _writeMarker("factories", "preinstalled");
      return { installed: true, source: "preinstalled" };
    }

    if (hasAnyCoreFactory()) {
      const reason = coreFactoryMissingReason();
      _writeMarker("factories", "partial");
      _writeMarker("factory-diagnostic", reason.toUpperCase());
      return { installed: false, source: "preinstalled", reason };
    }

    const installer = window.__metadata_checker_core_factory_installer ?? null;
    if (typeof installer === "function") {
      try {
        _writeMarker("factory-installer", "attempted");
        await Promise.resolve(installer({ window, document, logger: console, marker: _writeMarker }));
        if (hasCoreFactories()) {
          _writeMarker("factory-installer", "installed");
          _writeMarker("factories", "installed");
          return { installed: true, source: "installer" };
        }
      } catch (err) {
        _log("warn", "Core factory installer failed:", err);
        _writeMarker("factory-installer", "failed");
      }
    }

    const entryUrl = resolveFactoryEntryUrl();
    try {
      _writeMarker("factory-entry", entryUrl);
      const module = await importFactoryEntry(entryUrl);
      const moduleInstaller =
        module?.installMetadataCheckerBrowserFactories ?? module?.default ?? null;
      if (typeof moduleInstaller === "function") {
        await Promise.resolve(moduleInstaller({ window, document, logger: console, marker: _writeMarker }));
      }
      if (hasCoreFactories()) {
        _writeMarker("real-bundle", "loaded");
        _writeMarker("factories", "installed");
        return { installed: true, source: "module" };
      }
      _writeMarker("factories", "missing");
      _writeMarker("factory-diagnostic", "CORE_FACTORY_MISSING");
      return { installed: false, source: "module", reason: "core_factory_missing" };
    } catch (err) {
      _log("warn", "Core factory entry failed:", err);
      _writeMarker("factories", "failed");
      _writeMarker("factory-diagnostic", "CORE_FACTORY_IMPORT_FAILED");
      return { installed: false, source: "module", reason: "core_factory_import_failed" };
    }
  }

  function _makeErrorEnvelope(code, message) {
    return {
      status: "error",
      target: null,
      items: [],
      diagnostics: [{ severity: "error", code, message }],
    };
  }

  function _makeRuntimeError(code, message) {
    const error = new Error(message);
    error.code = code;
    return error;
  }

  function _isErrorEnvelope(value) {
    return value && typeof value === "object" && value.status === "error";
  }

  function _throwIfErrorEnvelope(value) {
    if (!_isErrorEnvelope(value)) {
      return value;
    }
    const diagnostic = value.diagnostics?.[0] ?? {};
    throw _makeRuntimeError(
      diagnostic.code ?? "RUNTIME_ERROR_ENVELOPE",
      diagnostic.message ?? "runtime returned an error envelope",
    );
  }

  // ---- Service Worker 注册 ----

  async function registerServiceWorker() {
    if (!("serviceWorker" in navigator)) {
      _log("warn", "Service Worker not supported in this browser");
      _writeMarker("sw", "unsupported");
      return { registered: false, reason: "unsupported", code: "SW_UNSUPPORTED", controller: null };
    }

    try {
      const registration = await navigator.serviceWorker.register(SW_SCRIPT_URL, {
        scope: SW_SCOPE,
      });
      _log("log", "Service Worker registered:", registration.scope);
      _writeMarker("sw", "registered");

      // 等待激活
      if (registration.installing) {
        await waitForState(registration.installing, "activated");
      } else if (registration.waiting) {
        await waitForState(registration.waiting, "activated");
      } else if (registration.active) {
        _writeMarker("sw", "active");
      }

      return { registered: true, registration, controller: registration.active };
    } catch (err) {
      _log("error", "Service Worker registration failed:", err);
      _writeMarker("sw", "failed");
      return {
        registered: false,
        reason: "registration_failed",
        code: "SW_REGISTRATION_FAILED",
        error: err?.message ?? String(err),
      };
    }
  }

  function waitForState(worker, state) {
    return new Promise((resolve, reject) => {
      if (worker.state === state) {
        resolve();
        return;
      }
      let timeoutId = null;
      const onStateChange = () => {
        if (worker.state === state) {
          cleanup();
          resolve();
        }
      };
      function cleanup() {
        if (timeoutId !== null) {
          clearTimeout(timeoutId);
          timeoutId = null;
        }
        worker.removeEventListener("statechange", onStateChange);
      }
      worker.addEventListener("statechange", onStateChange);

      // 超时保护
      timeoutId = setTimeout(() => {
        cleanup();
        reject(new Error(`Service Worker state did not reach ${state} in time`));
      }, SW_ACTIVATION_TIMEOUT_MS);
    });
  }

  // ---- 与 SW 通信的 transport ----

  function createSwMessageTransport(worker) {
    const handlers = new Set();

    function onMessage(event) {
      if (!event.data || typeof event.data !== "object") return;
      for (const handler of handlers) {
        handler(event.data);
      }
    }

    navigator.serviceWorker.addEventListener("message", onMessage);

    return {
      send(request) {
        const target = navigator.serviceWorker.controller ?? worker;
        if (!target || typeof target.postMessage !== "function") {
          return Promise.reject(
            _makeRuntimeError(
              "SW_CONTROLLER_UNAVAILABLE",
              "Service Worker controller not available",
            ),
          );
        }
        target.postMessage(request);
        return undefined;
      },
      onMessage(handler) {
        handlers.add(handler);
      },
      offMessage(handler) {
        handlers.delete(handler);
      },
      dispose() {
        navigator.serviceWorker.removeEventListener("message", onMessage);
        handlers.clear();
      },
    };
  }

  // ---- Page Runtime Fallback ----

  async function createPageFallbackRuntimeClient() {
    // 这里未来可以通过 import 或 script 标签加载 page runtime
    // 现在返回一个 mock runtimeClient，用于 fallback 场景
    return {
      _kind: "page-fallback",
      async initRuntime(options) {
        try {
          const result = { status: "ready", items: [], diagnostics: [] };
          _writeMarker("wasm", "page-fallback-ready");
          return result;
        } catch (err) {
          _writeMarker("wasm", "failed");
          _writeFallback("PAGE_RUNTIME_WASM_INIT_FAILED");
          throw err;
        }
      },
      async runtimeStatus() {
        return { status: "ready", items: [], diagnostics: [] };
      },
      async loadSuperpageDocument(sourcePath, rawText) {
        return { status: "ready", target: sourcePath, items: [], diagnostics: [] };
      },
      async loadRemoteSuperpageDocument(fileRef) {
        return {
          status: "ready",
          target: fileRef && fileRef.source_path,
          items: [],
          diagnostics: [],
        };
      },
      async buildOrUpdateSuperpageGraph(sourcePath) {
        return _makeErrorEnvelope(
          "PAGE_RUNTIME_WASM_UNAVAILABLE",
          "page fallback runtime cannot build a graph without the WASM runtime",
        );
      },
      async analyzeSuperpageSelection(selection, options) {
        return _makeErrorEnvelope(
          "PAGE_RUNTIME_WASM_UNAVAILABLE",
          "page fallback runtime cannot analyze a selection without the WASM runtime",
        );
      },
    };
  }

  function createMarkerRuntimeClient(runtimeClient, options) {
    let currentClient = runtimeClient;
    let currentKind = options?.runtimeKind ?? runtimeClient?._kind ?? "unknown";
    const onFallback = options?.onFallback ?? (() => {});

    async function switchToPageFallback(code) {
      onFallback(code);
      _writeFallback(code);
      _writeRuntimeMarker("page-fallback");
      currentClient = await createPageFallbackRuntimeClient();
      currentKind = "page-fallback";
      return currentClient;
    }

    async function call(method, args) {
      if (!currentClient || typeof currentClient[method] !== "function") {
        throw new Error(`runtimeClient.${method} is not available`);
      }
      return currentClient[method](...args);
    }

    return {
      get _kind() {
        return currentKind;
      },
      async initRuntime(...args) {
        _writeMarker("wasm", `${currentKind}-initializing`);
        try {
          const result = await call("initRuntime", args);
          _throwIfErrorEnvelope(result);
          _writeMarker("wasm", `${currentKind}-ready`);
          return result;
        } catch (err) {
          _writeMarker("wasm", "failed");
          if (currentKind !== "page-fallback") {
            const fallbackClient = await switchToPageFallback(
              err?.code ?? "WASM_INIT_FAILED",
            );
            const result = await fallbackClient.initRuntime(...args);
            _writeMarker("wasm", "page-fallback-ready");
            return result;
          }
          _writeFallback("PAGE_RUNTIME_WASM_INIT_FAILED");
          throw err;
        }
      },
      runtimeStatus(...args) {
        return call("runtimeStatus", args);
      },
      loadSuperpageDocument(...args) {
        return call("loadSuperpageDocument", args);
      },
      loadRemoteSuperpageDocument(...args) {
        return call("loadRemoteSuperpageDocument", args);
      },
      buildOrUpdateSuperpageGraph(...args) {
        return call("buildOrUpdateSuperpageGraph", args);
      },
      analyzeSuperpageSelection(...args) {
        return call("analyzeSuperpageSelection", args);
      },
    };
  }

  async function ensureGraphPanelFactories() {
    const installer = window.__metadata_checker_graph_factory_installer ?? null;
    if (typeof installer !== "function") {
      return;
    }

    try {
      _writeMarker("graph-factory-installer", "attempted");
      await Promise.resolve(
        installer({
          window,
          document,
          logger: console,
          marker: _writeMarker,
        }),
      );
      _writeMarker("graph-factory-installer", "installed");
    } catch (err) {
      _log("warn", "Graph factory installer failed:", err);
      _writeMarker("graph-factory-installer", "failed");
      _writeMarker("graph-factory-diagnostic", "GRAPH_PANEL_FACTORY_INSTALL_FAILED");
    }
  }

  // ---- Renderer ----

  function createDomRenderer() {
    return {
      renderAnalysis(result) {
        _log("log", "renderAnalysis:", result);
        _writeMarker("last-render", "analysis");
        _writeMarker("last-render-status", result.status ?? "unknown");
        _removeMarker("last-render-error-code");
        _removeMarker("last-render-error-message");
      },
      renderError(errorEnvelope) {
        _log("error", "renderError:", errorEnvelope);
        _writeMarker("last-render", "error");
        if (errorEnvelope?.diagnostics?.[0]?.code) {
          _writeMarker("last-render-error-code", errorEnvelope.diagnostics[0].code);
        }
        if (errorEnvelope?.diagnostics?.[0]?.message) {
          _writeMarker("last-render-error-message", errorEnvelope.diagnostics[0].message);
        }
      },
    };
  }

  async function createGraphRenderer() {
    const rendererFactory =
      window.__metadata_checker_graph_renderer_factory ?? null;
    const hostFactory =
      window.__metadata_checker_graph_panel_host_factory ?? null;
    if (!rendererFactory || !hostFactory) {
      await ensureGraphPanelFactories();
    }

    const finalRendererFactory =
      window.__metadata_checker_graph_renderer_factory ?? rendererFactory;
    const finalHostFactory =
      window.__metadata_checker_graph_panel_host_factory ?? hostFactory;

    if (!finalRendererFactory || !finalHostFactory) {
      if (!finalRendererFactory) {
        _writeMarker("graph-factory-renderer", "missing");
      }
      if (!finalHostFactory) {
        _writeMarker("graph-factory-host", "missing");
      }
      _writeMarker(
        "graph-factory",
        !finalRendererFactory && !finalHostFactory
          ? "missing_renderer_host"
          : "missing_" +
              (!finalRendererFactory
                ? "renderer"
                : "panel_host"),
      );
      _writeMarker("graph-factory-diagnostic", "GRAPH_PANEL_FACTORY_MISSING");
      _writeMarker("graph-panel", "unavailable");
      _writeMarker("graph-renderer", "missing");
      return null;
    }

    if (!rendererFactory || !hostFactory) {
      _writeMarker("graph-factory", "installed");
    }

    const rendererFactoryToUse = finalRendererFactory;
    const hostFactoryToUse = finalHostFactory;

    let echarts = null;
    const resolverFactory =
      window.__metadata_checker_echarts_resolver_factory ?? null;
    if (resolverFactory) {
      try {
        const resolved = await resolverFactory({
          globalThisLike: window,
          requireLike: typeof window.require === "function" ? window.require : undefined,
          logger: console,
        });
        echarts = resolved?.echarts ?? null;
        _writeMarker("graph-renderer", echarts ? "echarts" : "html");
        _writeMarker("graph-echarts-resolver", "installed");
      } catch (err) {
        _log("warn", "ECharts resolver failed, falling back to HTML renderer:", err);
        _writeMarker("graph-echarts-resolver", "failed");
        _writeMarker("graph-renderer", "html");
        _writeMarker("graph-factory-diagnostic", "GRAPH_ECHARTS_RESOLVER_FAILED");
      }
    } else {
      _writeMarker("graph-echarts-resolver", "missing");
      _writeMarker("graph-renderer", "html");
    }

    const graphRenderer = rendererFactoryToUse({
      document,
      echarts,
      onEvent(event) {
        if (event?.type === "expand_requested") {
          _writeMarker("graph-last-event", "expand_requested");
        }
      },
    });
    const panelHost = hostFactoryToUse({
      document,
      parent: document.body,
      renderer: graphRenderer,
      logger: console,
    });
    const mounted = panelHost.mount?.();
    _writeMarker("graph-panel", mounted?.mounted ? "mounted" : "error");
    return panelHost;
  }

  // ---- Provider ----

  function createPageRcProvider() {
    // 复用已有的 page-rc-metadata-provider 逻辑
    // 由于 AMD 环境不能动态 import ESM，这里内联最小实现
    const rc = typeof window !== "undefined" ? window.SZ?.rc : undefined;
    const rc1 = typeof window !== "undefined" ? window.SZ?.rc1 : undefined;

    function encodeMetaPath(value) {
      return encodeURIComponent(value).replaceAll("%2F", "/");
    }

    function normalizeProjectName(projectName) {
      if (typeof projectName !== "string") return "";
      return projectName.replace(/^\/+|\/+$/g, "");
    }

    function resolveIdOrProjectPath(fileRef) {
      if (typeof fileRef.file_id === "string" && fileRef.file_id !== "") {
        return fileRef.file_id;
      }
      const sourcePath = String(fileRef.source_path ?? "").replace(/^\/+/, "");
      const projectName = normalizeProjectName(fileRef.project_name);
      return projectName ? `${projectName}/${sourcePath}` : sourcePath;
    }

    function getFileInfoUrl(fileRef) {
      return `/api/meta/services/getFileInfo/${encodeMetaPath(resolveIdOrProjectPath(fileRef))}`;
    }

    function getFileContentUrl(fileRef) {
      return `/api/meta/services/getFileContent/${encodeMetaPath(resolveIdOrProjectPath(fileRef))}`;
    }

    async function callRc(request) {
      if (typeof rc === "function") return rc(request);
      if (typeof rc1 === "function") return rc1(request);
      return null;
    }

    function inferContentType(sourcePath) {
      if (typeof sourcePath !== "string") return "unknown";
      if (sourcePath.endsWith(".spg")) return "super_page";
      if (sourcePath.endsWith(".tbl")) return "table";
      return "unknown";
    }

    return {
      async getFileInfo(fileRef) {
        if (!rc && !rc1) {
          return _makeErrorEnvelope("PAGE_RC_UNAVAILABLE", "window.SZ.rc / rc1 is not available");
        }
        try {
          const result = await callRc({ url: getFileInfoUrl(fileRef) });
          if (!result) {
            return _makeErrorEnvelope("REMOTE_RESPONSE_INVALID", "rc returned null");
          }
          const parsed = typeof result === "string" ? JSON.parse(result) : result;
          return {
            source_path: fileRef.source_path,
            file_id: fileRef.file_id ?? null,
            revision: parsed.revision ?? null,
            content_type: parsed.content_type ?? inferContentType(fileRef.source_path),
            updated_at: parsed.updated_at ?? null,
          };
        } catch (err) {
          return _makeErrorEnvelope("REMOTE_FETCH_FAILED", "rc getFileInfo failed");
        }
      },
      async getFileContent(fileRef) {
        if (!rc && !rc1) {
          return _makeErrorEnvelope("PAGE_RC_UNAVAILABLE", "window.SZ.rc / rc1 is not available");
        }
        try {
          const result = await callRc({
            url: getFileContentUrl(fileRef),
            dataType: "text",
          });
          if (!result) {
            return _makeErrorEnvelope("REMOTE_RESPONSE_INVALID", "rc returned null");
          }
          const rawText = typeof result === "string" ? result : JSON.stringify(result);
          return {
            source_path: fileRef.source_path,
            file_id: fileRef.file_id ?? null,
            revision: null,
            content_type: inferContentType(fileRef.source_path),
            raw_text: rawText,
          };
        } catch (err) {
          return _makeErrorEnvelope("REMOTE_FETCH_FAILED", "rc getFileContent failed");
        }
      },
      async getRelatedFiles(fileRef) {
        return [];
      },
    };
  }

  // ---- 主入口 ----

  let _controller = null;
  let _glueInstalled = false;

  async function onInitDesigner(designer, args) {
    _log("log", "onInitDesigner loaded, version:", CONTROLLER_VERSION);
    _writeMarker("on-init-designer", "called");

    if (_glueInstalled) {
      _log("warn", "glue already installed, skipping duplicate init");
      _writeMarker("on-init-designer", "already-installed");
      return { installed: true, alreadyInstalled: true };
    }

    // 1. 注册 Service Worker
    const swResult = await registerServiceWorker();

    // 2. 决定 runtime launcher 类型
    let runtimeClient;
    let fallbackUsed = false;
    if (swResult.registered && swResult.controller) {
      try {
        const transport = createSwMessageTransport(swResult.controller);
        // 动态导入 launcher（AMD 环境下通过 require 或全局 script）
        // 这里假设 runtime launcher 已通过 script 标签预加载到 window
        const launcherFactory =
          window.__metadata_checker_service_worker_launcher ??
          createFallbackLauncher;
        const launcher = launcherFactory({
          transport,
          requestTimeoutMs: SW_REQUEST_TIMEOUT_MS,
          fallbackPageLauncher: {
            start: async () => {
              fallbackUsed = true;
              _writeFallback("SW_RUNTIME_FALLBACK_REQUESTED");
              _writeRuntimeMarker("page-fallback");
              return createPageFallbackRuntimeClient();
            },
            stop() {},
          },
        });
        runtimeClient = await launcher.start();
        _writeRuntimeMarker("service-worker");
      } catch (err) {
        _log("error", "SW runtime start failed, using fallback:", err);
        fallbackUsed = true;
        runtimeClient = await createPageFallbackRuntimeClient();
        _writeFallback("SW_RUNTIME_START_FAILED");
        _writeRuntimeMarker("page-fallback");
      }
    } else {
      _log("warn", "SW not available, using page runtime fallback");
      fallbackUsed = true;
      runtimeClient = await createPageFallbackRuntimeClient();
      _writeFallback(swResult.code ?? "SW_UNAVAILABLE");
      _writeRuntimeMarker("page-fallback");
    }

    _writeMarker("fallback-used", String(fallbackUsed));
    if (!fallbackUsed) {
      _writeMarker("fallback-code", "none");
      _writeMarker("analysis-status", "initializing");
    }

    runtimeClient = createMarkerRuntimeClient(runtimeClient, {
      runtimeKind: fallbackUsed ? "page-fallback" : "service-worker",
      onFallback() {
        fallbackUsed = true;
      },
    });

    // 3. 创建 host
    const host = {
      events: [],
      emit(eventName, payload) {
        this.events.push({ eventName, payload });
      },
    };

    // 4. 创建 plugin core
    const factoryResult = await ensureCoreFactories();
    if (!factoryResult.installed) {
      if (factoryResult.reason === "controller_factory_missing") {
        _writeMarker("controller", "missing");
      } else if (factoryResult.reason === "glue_factory_missing") {
        _writeMarker("glue", "missing");
      } else {
        _writeMarker("plugin", "missing");
      }
      _writeMarker("analysis-status", factoryResult.reason ?? "core_factory_missing");
      return { installed: false, reason: factoryResult.reason ?? "core_factory_missing" };
    }

    const pluginFactory =
      window.__metadata_checker_plugin_factory ?? null;
    if (!pluginFactory) {
      _log("error", "Plugin factory not found on window");
      _writeMarker("plugin", "missing");
      _writeMarker("analysis-status", "plugin_factory_missing");
      return { installed: false, reason: "plugin_factory_missing" };
    }
    const plugin = pluginFactory({
      runtimeClient,
      host,
      logger: console,
    });

    // 5. 创建 provider
    const provider = createPageRcProvider();

    // 6. 创建 renderer
    const renderer = createDomRenderer();
    const graphRenderer = await createGraphRenderer();

    // 7. 创建 controller
    // 这里假设 controller 已通过 script 标签预加载到 window
    const controllerFactory =
      window.__metadata_checker_controller_factory ?? null;
    if (!controllerFactory) {
      _log("error", "Controller factory not found on window");
      _writeMarker("controller", "missing");
      _writeMarker("analysis-status", "controller_factory_missing");
      return { installed: false, reason: "controller_factory_missing" };
    }
    _controller = controllerFactory({
      plugin,
      provider,
      runtimeClient,
      renderer,
      graphRenderer,
      host,
      logger: console,
      selectionDebounceMs: 50,
    });

    await _controller.init();

    // 8. 安装 designer glue
    // 这里假设 glue 已通过 script 标签预加载到 window
    const glueFactory =
      window.__metadata_checker_glue_factory ?? null;
    if (!glueFactory) {
      _log("error", "Glue factory not found on window");
      _writeMarker("glue", "missing");
      _writeMarker("analysis-status", "glue_factory_missing");
      return { installed: false, reason: "glue_factory_missing" };
    }
    const glueResult = glueFactory(designer, args, plugin, { host, logger: console });
    if (glueResult && glueResult.installed) {
      _glueInstalled = true;
      _writeMarker("glue", "installed");
    } else {
      _writeMarker("glue", "failed");
      _writeMarker("analysis-status", "glue_install_failed");
      return { installed: false, reason: "glue_install_failed", detail: glueResult };
    }

    _writeMarker("version", CONTROLLER_VERSION);
    _writeMarker("analysis-status", "ready");

    return { installed: true, sw: swResult, fallbackUsed };
  }

  function createFallbackLauncher(options) {
    // 最小 fallback launcher，模拟 service-worker-runtime-launcher 的接口
    const transport = options.transport;
    const requestTimeoutMs = options.requestTimeoutMs ?? SW_REQUEST_TIMEOUT_MS;
    let started = false;
    let _client = null;
    let nextRequestId = 0;

    function makeRequest(method, args) {
      const id = `req-${++nextRequestId}`;
      return new Promise((resolve, reject) => {
        let settled = false;
        const timeoutId = setTimeout(() => {
          settleReject(
            _makeRuntimeError(
              "SW_RUNTIME_REQUEST_TIMEOUT",
              `Service Worker request ${id} timed out`,
            ),
          );
        }, requestTimeoutMs);

        function cleanup() {
          clearTimeout(timeoutId);
          transport.offMessage(handler);
        }

        function settleResolve(value) {
          if (settled) return;
          settled = true;
          cleanup();
          resolve(value);
        }

        function settleReject(error) {
          if (settled) return;
          settled = true;
          cleanup();
          reject(error);
        }

        function handler(response) {
          if (response.id !== id) return;
          if (response.ok) {
            settleResolve(response.result);
            return;
          }
          settleReject(
            _makeRuntimeError(
              response.error?.code ?? "SW_RUNTIME_REQUEST_FAILED",
              response.error?.message ?? "request failed",
            ),
          );
        }

        transport.onMessage(handler);
        try {
          const sendResult = transport.send({ id, method, args });
          if (sendResult && typeof sendResult.then === "function") {
            sendResult.catch((err) => {
              settleReject(
                err?.code
                  ? err
                  : _makeRuntimeError(
                      "SW_RUNTIME_REQUEST_FAILED",
                      err?.message ?? String(err),
                    ),
              );
            });
          }
        } catch (err) {
          settleReject(
            err?.code
              ? err
              : _makeRuntimeError(
                  "SW_RUNTIME_REQUEST_FAILED",
                  err?.message ?? String(err),
                ),
          );
        }
      });
    }

    return {
      async start() {
        if (started && _client) return _client;
        started = true;

        // 尝试通过 transport 连接 SW
        try {
          const messageClient = {
            initRuntime(opts) {
              return makeRequest("initRuntime", [opts ?? {}]);
            },
            runtimeStatus() {
              return makeRequest("runtimeStatus", []);
            },
            loadSuperpageDocument(sourcePath, rawText) {
              return makeRequest("loadSuperpageDocument", [sourcePath, rawText]);
            },
            loadRemoteSuperpageDocument(fileRef, opts) {
              return makeRequest("loadRemoteSuperpageDocument", [fileRef, opts]);
            },
            buildOrUpdateSuperpageGraph(sourcePath) {
              return makeRequest("buildOrUpdateSuperpageGraph", [sourcePath]);
            },
            analyzeSuperpageSelection(selection, opts) {
              return makeRequest("analyzeSuperpageSelection", [selection, opts]);
            },
          };
          _client = messageClient;
          return _client;
        } catch (err) {
          started = false;
          throw err;
        }
      },
      stop() {
        started = false;
        _client = null;
        return { stopped: true };
      },
      getClient() {
        return _client;
      },
      status() {
        return { kind: "service-worker", state: started ? "ready" : "idle", started, fallbackUsed: false };
      },
    };
  }

  // 导出供 AMD 模块系统使用
  return {
    version: CONTROLLER_VERSION,
    onInitDesigner,
    status() {
      return {
        controller: _controller ? _controller.status() : null,
        glueInstalled: _glueInstalled,
      };
    },
  };
});

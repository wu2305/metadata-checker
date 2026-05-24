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
  const SW_SCOPE = "/analyzer/";
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

  function _removeMarker(name) {
    if (typeof document === "undefined") return;
    const key = `${MARKER_NS}-${name}`;
    const existing = document.querySelector(`[${key}]`);
    if (existing) {
      existing.remove();
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

  // ---- Service Worker 注册 ----

  async function registerServiceWorker() {
    if (!("serviceWorker" in navigator)) {
      _log("warn", "Service Worker not supported in this browser");
      _writeMarker("sw", "unsupported");
      return { registered: false, reason: "unsupported", controller: null };
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
      const onStateChange = () => {
        if (worker.state === state) {
          worker.removeEventListener("statechange", onStateChange);
          resolve();
        }
      };
      worker.addEventListener("statechange", onStateChange);

      // 超时保护
      setTimeout(() => {
        worker.removeEventListener("statechange", onStateChange);
        reject(new Error(`Service Worker state did not reach ${state} in time`));
      }, 30000);
    });
  }

  // ---- 与 SW 通信的 transport ----

  function createSwMessageTransport() {
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
        if (navigator.serviceWorker.controller) {
          navigator.serviceWorker.controller.postMessage(request);
        } else {
          return Promise.reject(new Error("Service Worker controller not available"));
        }
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
        return { status: "ready", items: [], diagnostics: [] };
      },
      async runtimeStatus() {
        return { status: "ready", items: [], diagnostics: [] };
      },
      async loadSuperpageDocument(sourcePath, rawText) {
        return { status: "ready", target: sourcePath, items: [], diagnostics: [] };
      },
      async buildOrUpdateSuperpageGraph(sourcePath) {
        return { status: "ready", target: sourcePath, items: [], diagnostics: [] };
      },
      async analyzeSuperpageSelection(selection, options) {
        return {
          status: "ready",
          target: selection.active_component_id ?? selection.source_path,
          items: [{ kind: "analysis", label: "Fallback Analysis", detail: { selection } }],
          diagnostics: [],
        };
      },
    };
  }

  // ---- Renderer ----

  function createDomRenderer() {
    return {
      renderAnalysis(result) {
        _log("log", "renderAnalysis:", result);
        _writeMarker("last-render", "analysis");
        _writeMarker("last-render-status", result.status ?? "unknown");
      },
      renderError(errorEnvelope) {
        _log("error", "renderError:", errorEnvelope);
        _writeMarker("last-render", "error");
        if (errorEnvelope?.diagnostics?.[0]?.code) {
          _writeMarker("last-render-error-code", errorEnvelope.diagnostics[0].code);
        }
      },
    };
  }

  // ---- Provider ----

  function createPageRcProvider() {
    // 复用已有的 page-rc-metadata-provider 逻辑
    // 由于 AMD 环境不能动态 import ESM，这里内联最小实现
    const rc = typeof window !== "undefined" ? window.SZ?.rc : undefined;
    const rc1 = typeof window !== "undefined" ? window.SZ?.rc1 : undefined;

    async function callRc(method, args) {
      if (typeof rc === "function") return rc(method, ...args);
      if (typeof rc1 === "function") return rc1(method, ...args);
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
          const result = await callRc("getFileInfo", [fileRef.file_id, fileRef.source_path]);
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
          const result = await callRc("getFileContent", [fileRef.file_id, fileRef.source_path]);
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
    const swResult = await registerServiceWatcher();

    // 2. 决定 runtime launcher 类型
    let runtimeClient;
    let fallbackUsed = false;
    if (swResult.registered && swResult.controller) {
      try {
        const transport = createSwMessageTransport();
        // 动态导入 launcher（AMD 环境下通过 require 或全局 script）
        // 这里假设 runtime launcher 已通过 script 标签预加载到 window
        const launcherFactory =
          window.__metadata_checker_service_worker_launcher ??
          createFallbackLauncher;
        const launcher = launcherFactory({
          transport,
          fallbackPageLauncher: {
            start: async () => {
              fallbackUsed = true;
              _writeMarker("runtime", "page-fallback");
              return createPageFallbackRuntimeClient();
            },
            stop() {},
          },
        });
        runtimeClient = await launcher.start();
        _writeMarker("runtime", "service-worker");
      } catch (err) {
        _log("error", "SW runtime start failed, using fallback:", err);
        fallbackUsed = true;
        runtimeClient = await createPageFallbackRuntimeClient();
        _writeMarker("runtime", "page-fallback");
      }
    } else {
      _log("warn", "SW not available, using page runtime fallback");
      fallbackUsed = true;
      runtimeClient = await createPageFallbackRuntimeClient();
      _writeMarker("runtime", "page-fallback");
    }

    _writeMarker("fallback-used", String(fallbackUsed));

    // 3. 创建 host
    const host = {
      events: [],
      emit(eventName, payload) {
        this.events.push({ eventName, payload });
      },
    };

    // 4. 创建 plugin core
    // 这里假设 plugin core 已通过 script 标签预加载到 window
    const pluginFactory =
      window.__metadata_checker_plugin_factory ?? null;
    if (!pluginFactory) {
      _log("error", "Plugin factory not found on window");
      _writeMarker("plugin", "missing");
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

    // 7. 创建 controller
    // 这里假设 controller 已通过 script 标签预加载到 window
    const controllerFactory =
      window.__metadata_checker_controller_factory ?? null;
    if (!controllerFactory) {
      _log("error", "Controller factory not found on window");
      _writeMarker("controller", "missing");
      return { installed: false, reason: "controller_factory_missing" };
    }
    _controller = controllerFactory({
      plugin,
      provider,
      runtimeClient,
      renderer,
      host,
      logger: console,
    });

    await _controller.init();

    // 8. 安装 designer glue
    // 这里假设 glue 已通过 script 标签预加载到 window
    const glueFactory =
      window.__metadata_checker_glue_factory ?? null;
    if (!glueFactory) {
      _log("error", "Glue factory not found on window");
      _writeMarker("glue", "missing");
      return { installed: false, reason: "glue_factory_missing" };
    }
    const glueResult = glueFactory(designer, args, plugin, { host, logger: console });
    if (glueResult && glueResult.installed) {
      _glueInstalled = true;
      _writeMarker("glue", "installed");
    } else {
      _writeMarker("glue", "failed");
      return { installed: false, reason: "glue_install_failed", detail: glueResult };
    }

    _writeMarker("version", CONTROLLER_VERSION);
    _writeMarker("analysis-status", "ready");

    return { installed: true, sw: swResult, fallbackUsed };
  }

  function createFallbackLauncher(options) {
    // 最小 fallback launcher，模拟 service-worker-runtime-launcher 的接口
    const transport = options.transport;
    let started = false;
    let _client = null;

    return {
      async start() {
        if (started && _client) return _client;
        started = true;

        // 尝试通过 transport 连接 SW
        try {
          const messageClient = {
            initRuntime(opts) {
              return new Promise((resolve, reject) => {
                const id = `req-${Date.now()}`;
                const handler = (response) => {
                  if (response.id === id) {
                    transport.offMessage(handler);
                    if (response.ok) resolve(response.result);
                    else reject(new Error(response.error?.message ?? "request failed"));
                  }
                };
                transport.onMessage(handler);
                transport.send({ id, method: "initRuntime", args: [opts ?? {}] });
              });
            },
            runtimeStatus() {
              return new Promise((resolve, reject) => {
                const id = `req-${Date.now()}`;
                const handler = (response) => {
                  if (response.id === id) {
                    transport.offMessage(handler);
                    if (response.ok) resolve(response.result);
                    else reject(new Error(response.error?.message ?? "request failed"));
                  }
                };
                transport.onMessage(handler);
                transport.send({ id, method: "runtimeStatus", args: [] });
              });
            },
            loadSuperpageDocument(sourcePath, rawText) {
              return new Promise((resolve, reject) => {
                const id = `req-${Date.now()}`;
                const handler = (response) => {
                  if (response.id === id) {
                    transport.offMessage(handler);
                    if (response.ok) resolve(response.result);
                    else reject(new Error(response.error?.message ?? "request failed"));
                  }
                };
                transport.onMessage(handler);
                transport.send({ id, method: "loadSuperpageDocument", args: [sourcePath, rawText] });
              });
            },
            buildOrUpdateSuperpageGraph(sourcePath) {
              return new Promise((resolve, reject) => {
                const id = `req-${Date.now()}`;
                const handler = (response) => {
                  if (response.id === id) {
                    transport.offMessage(handler);
                    if (response.ok) resolve(response.result);
                    else reject(new Error(response.error?.message ?? "request failed"));
                  }
                };
                transport.onMessage(handler);
                transport.send({ id, method: "buildOrUpdateSuperpageGraph", args: [sourcePath] });
              });
            },
            analyzeSuperpageSelection(selection, opts) {
              return new Promise((resolve, reject) => {
                const id = `req-${Date.now()}`;
                const handler = (response) => {
                  if (response.id === id) {
                    transport.offMessage(handler);
                    if (response.ok) resolve(response.result);
                    else reject(new Error(response.error?.message ?? "request failed"));
                  }
                };
                transport.onMessage(handler);
                transport.send({ id, method: "analyzeSuperpageSelection", args: [selection, opts] });
              });
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

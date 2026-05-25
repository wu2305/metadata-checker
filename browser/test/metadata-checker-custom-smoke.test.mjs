/**
 * M40.10 custom.js real entry smoke tests
 *
 * 通过 mock AMD/window/document/navigator 执行 custom.js factory 和 onInitDesigner。
 */

import { describe, it } from "node:test";
import assert from "node:assert";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import vm from "node:vm";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const customSourcePath = join(__dirname, "../tools/metadata-checker-custom.js");

function createMockDocument() {
  const children = [];

  function createElement(tagName) {
    const attributes = new Map();
    const elementChildren = [];
    return {
      tagName,
      children: elementChildren,
      style: {},
      className: "",
      textContent: "",
      appendChild(child) {
        elementChildren.push(child);
        child.parentNode = this;
      },
      replaceChildren(...nextChildren) {
        elementChildren.length = 0;
        elementChildren.push(...nextChildren);
      },
      addEventListener() {},
      setAttribute(name, value) {
        attributes.set(name, String(value));
      },
      getAttribute(name) {
        return attributes.get(name) ?? null;
      },
      hasAttribute(name) {
        return attributes.has(name);
      },
      remove() {
        const index = children.indexOf(this);
        if (index !== -1) {
          children.splice(index, 1);
        }
      },
    };
  }

  const document = {
    body: {
      children,
      appendChild(element) {
        children.push(element);
      },
    },
    createElement,
    addEventListener() {},
    querySelector(selector) {
      const match = selector.match(/^\[([^\]]+)\]$/);
      if (!match) return null;
      const attribute = match[1];
      return children.find((child) => child.hasAttribute(attribute)) ?? null;
    },
  };

  return document;
}

function marker(document, name) {
  const key = `data-metadata-checker-${name}`;
  return document.querySelector(`[${key}]`)?.getAttribute(key) ?? null;
}

function createFakeTimers() {
  let nextId = 0;
  const timers = new Map();
  const cleared = [];
  return {
    timers,
    cleared,
    setTimeout(callback, delay) {
      const id = ++nextId;
      timers.set(id, { callback, delay });
      return id;
    },
    clearTimeout(id) {
      cleared.push(id);
      timers.delete(id);
    },
    runNext() {
      const [id, timer] = timers.entries().next().value ?? [];
      if (!timer) return false;
      timers.delete(id);
      timer.callback();
      return true;
    },
  };
}

async function waitUntil(predicate, message) {
  for (let i = 0; i < 20; i += 1) {
    if (predicate()) return;
    await Promise.resolve();
  }
  assert.ok(predicate(), message);
}

function createWorkerMock(options = {}) {
  const listeners = new Map();
  const worker = {
    state: options.state ?? "activated",
    postMessage(request) {
      if (options.postMessageThrow) {
        throw new Error(options.postMessageThrow);
      }
      if (typeof options.onRequest === "function") {
        options.onRequest(request);
      }
    },
    addEventListener(type, handler) {
      listeners.set(type, handler);
    },
    removeEventListener(type, handler) {
      if (listeners.get(type) === handler) {
        listeners.delete(type);
      }
    },
    activate() {
      worker.state = "activated";
      listeners.get("statechange")?.();
    },
    listenerCount() {
      return listeners.size;
    },
  };
  return worker;
}

function createServiceWorkerMock(options = {}) {
  const listeners = new Map();
  const registerCalls = [];
  const activeWorker = createWorkerMock({
    onRequest(request) {
      if (typeof options.onWorkerRequest === "function") {
        options.onWorkerRequest(request, (response) => {
          const handler = listeners.get("message");
          if (handler) {
            handler({ data: response });
          }
        });
      }
    },
    postMessageThrow: options.workerPostMessageThrow,
  });
  const installingWorker = options.installing
    ? createWorkerMock({ state: "installing" })
    : null;
  const waitingWorker = options.waiting
    ? createWorkerMock({ state: "installed" })
    : null;
  const serviceWorker = {
    controller: options.controller ?? activeWorker,
    activeWorker,
    installingWorker,
    waitingWorker,
    registerCalls,
    addEventListener(type, handler) {
      listeners.set(type, handler);
    },
    removeEventListener(type) {
      listeners.delete(type);
    },
    async register(scriptUrl, registerOptions) {
      registerCalls.push({ scriptUrl, registerOptions });
      if (options.registerReject) {
        throw new Error(options.registerReject);
      }
      return {
        scope: registerOptions?.scope ?? "/analyzer/",
        active: options.activeWorker === null ? null : activeWorker,
        installing: installingWorker,
        waiting: waitingWorker,
      };
    },
  };
  return serviceWorker;
}

function createRequiredFactories(options = {}) {
  return {
    __metadata_checker_plugin_factory:
      options.pluginFactory ??
      (() => ({
        activate: async () => ({ status: "ready" }),
        status: () => ({ state: "ready" }),
      })),
    __metadata_checker_controller_factory:
      options.controllerFactory ??
      (() => ({
        init: async () => ({ status: "ready" }),
        status: () => ({ state: "ready" }),
      })),
    __metadata_checker_glue_factory:
      options.glueFactory ??
      (() => ({
        installed: true,
      })),
  };
}

function loadCustomModule(options = {}) {
  const source = readFileSync(customSourcePath, "utf-8");
  const document = createMockDocument();
  if (Object.prototype.hasOwnProperty.call(options, "currentScript")) {
    document.currentScript = options.currentScript;
  }
  const serviceWorker = options.serviceWorker ?? createServiceWorkerMock();
  const windowObject = {
    SZ: {},
    location: options.location ?? {
      origin: "https://autocrm-test.xiaoshouyi.com",
      pathname: "/analyzer/app/M40HookSmoke.app",
      href: "https://autocrm-test.xiaoshouyi.com/analyzer/app/M40HookSmoke.app?:edit=true",
    },
    ...createRequiredFactories(options),
    ...(options.windowOverrides ?? {}),
  };
  const navigatorObject =
    options.noServiceWorker === true ? {} : { serviceWorker };

  let amdModule = null;
  const context = {
    define(factory) {
      amdModule = factory();
    },
    window: windowObject,
    document,
    navigator: navigatorObject,
    console: {
      log() {},
      warn() {},
      error() {},
    },
    setTimeout: options.setTimeout ?? setTimeout,
    clearTimeout: options.clearTimeout ?? clearTimeout,
    URL,
    fetch: async () => {
      throw new Error("fetch should not be called in custom.js smoke");
    },
  };

  vm.runInNewContext(source, context, { filename: customSourcePath });
  return { module: amdModule, document, window: windowObject, serviceWorker };
}

describe("metadata-checker custom.js AMD entry", () => {
  it("loads AMD factory", () => {
    const { module } = loadCustomModule();
    assert.ok(module, "AMD factory should return module exports");
    assert.strictEqual(typeof module.onInitDesigner, "function");
  });

  it("onInitDesigner calls registerServiceWorker without ReferenceError", async () => {
    const { module } = loadCustomModule();
    const result = await module.onInitDesigner({}, {});
    assert.strictEqual(result.installed, true);
    assert.strictEqual(result.fallbackUsed, false);
  });

  it("loads core factories from sidecar module when custom.js is uploaded alone", async () => {
    let importedUrl = null;
    const fakeDesigner = {
      openFileArgs: {
        path: "/analyzer/app/M42Smoke.app/M42Smoke.spg",
        id: "fid-m42",
        projectName: "analyzer",
      },
      getBuilder() {
        return {
          getSelectedComponents: () => [],
          getSelectedComponent: () => null,
          getSelectedComponentInfo: () => null,
          selectComponents() {},
          doSelectedChange() {},
        };
      },
    };
    const { module, document } = loadCustomModule({
      noServiceWorker: true,
      windowOverrides: {
        __metadata_checker_plugin_factory: null,
        __metadata_checker_controller_factory: null,
        __metadata_checker_glue_factory: null,
        __metadata_checker_factory_entry_url:
          "https://autocrm-test.xiaoshouyi.com/analyzer/public/hooks/metadata-checker-browser-entry.mjs",
        __metadata_checker_module_import: async (url) => {
          importedUrl = url;
          return import("../metadata-checker-browser-entry.mjs");
        },
      },
    });

    const result = await module.onInitDesigner(fakeDesigner, {});

    assert.strictEqual(result.installed, true);
    assert.strictEqual(
      importedUrl,
      "https://autocrm-test.xiaoshouyi.com/analyzer/public/hooks/metadata-checker-browser-entry.mjs",
    );
    assert.strictEqual(marker(document, "real-bundle"), "loaded");
    assert.strictEqual(marker(document, "factories"), "installed");
    assert.strictEqual(marker(document, "graph-panel"), "mounted");
    assert.strictEqual(marker(document, "analysis-status"), "ready");
  });

  it("resolves sidecar module from project hook path when document.currentScript is unavailable", async () => {
    let importedUrl = null;
    const fakeDesigner = {
      openFileArgs: {
        path: "/analyzer/app/M42Smoke.app/M42Smoke.spg",
        id: "fid-m42",
        projectName: "analyzer",
      },
      getBuilder() {
        return {
          getSelectedComponents: () => [],
          getSelectedComponent: () => null,
          getSelectedComponentInfo: () => null,
          selectComponents() {},
          doSelectedChange() {},
        };
      },
    };
    const { module, document } = loadCustomModule({
      currentScript: null,
      noServiceWorker: true,
      location: {
        origin: "https://autocrm-test.xiaoshouyi.com",
        pathname: "/analyzer/app/M42Smoke.app/M42Smoke.spg",
        href: "https://autocrm-test.xiaoshouyi.com/analyzer/app/M42Smoke.app/M42Smoke.spg?:edit=true",
      },
      windowOverrides: {
        __metadata_checker_plugin_factory: null,
        __metadata_checker_controller_factory: null,
        __metadata_checker_glue_factory: null,
        __metadata_checker_module_import: async (url) => {
          importedUrl = url;
          return import("../metadata-checker-browser-entry.mjs");
        },
      },
    });

    const result = await module.onInitDesigner(fakeDesigner, {});

    assert.strictEqual(result.installed, true);
    assert.strictEqual(
      importedUrl,
      "https://autocrm-test.xiaoshouyi.com/analyzer/public/hooks/metadata-checker-browser-entry.mjs",
    );
    assert.strictEqual(marker(document, "factory-entry"), importedUrl);
    assert.strictEqual(marker(document, "factories"), "installed");
  });

  it("Service Worker register success writes stable markers", async () => {
    const { module, document } = loadCustomModule();
    const result = await module.onInitDesigner({}, {});

    assert.strictEqual(result.installed, true);
    assert.strictEqual(marker(document, "sw"), "active");
    assert.strictEqual(marker(document, "runtime"), "service-worker");
    assert.strictEqual(marker(document, "wasm"), "service-worker");
    assert.strictEqual(marker(document, "fallback-used"), "false");
    assert.strictEqual(marker(document, "fallback-code"), "none");
    assert.strictEqual(marker(document, "analysis-status"), "ready");
    assert.strictEqual(marker(document, "graph-panel"), "unavailable");
  });

  it("records stable graph factory diagnostics when graph factories are missing", async () => {
    const { module, document } = loadCustomModule();
    const result = await module.onInitDesigner({}, {});

    assert.strictEqual(result.installed, true);
    assert.strictEqual(marker(document, "graph-factory"), "missing_renderer_host");
    assert.strictEqual(marker(document, "graph-factory-renderer"), "missing");
    assert.strictEqual(marker(document, "graph-factory-host"), "missing");
    assert.strictEqual(marker(document, "graph-factory-diagnostic"), "GRAPH_PANEL_FACTORY_MISSING");
    assert.strictEqual(marker(document, "graph-factory-installer"), null);
    assert.strictEqual(marker(document, "graph-renderer"), "missing");
  });

  it("optional graph factory installer can materialize panel factories", async () => {
    let installerCalled = 0;
    const { module, document } = loadCustomModule({
      windowOverrides: {
        __metadata_checker_graph_factory_installer: ({ window: sandboxWindow }) => {
          installerCalled += 1;
          sandboxWindow.__metadata_checker_echarts_resolver_factory = async () => ({
            echarts: { version: "4.x" },
            source: "commons/echarts/echarts-ext",
            diagnostics: [],
          });
          sandboxWindow.__metadata_checker_graph_renderer_factory = ({ echarts }) => ({
            echarts,
            render: async () => ({ renderer: echarts ? "echarts" : "html" }),
            renderError: async () => ({ renderer: "error" }),
          });
          sandboxWindow.__metadata_checker_graph_panel_host_factory = ({ renderer }) => ({
            renderer,
            mount() {
              return { mounted: true };
            },
            render(result) {
              return renderer.render(result);
            },
            renderError(error) {
              return renderer.renderError(error);
            },
          });
        },
      },
    });

    const result = await module.onInitDesigner({}, {});

    assert.strictEqual(installerCalled, 1);
    assert.strictEqual(result.installed, true);
    assert.strictEqual(marker(document, "graph-factory-installer"), "installed");
    assert.strictEqual(marker(document, "graph-panel"), "mounted");
    assert.strictEqual(marker(document, "graph-factory"), "installed");
    assert.strictEqual(marker(document, "graph-renderer"), "echarts");
    assert.strictEqual(marker(document, "graph-echarts-resolver"), "installed");
  });

  it("wires optional graph renderer and panel host into controller", async () => {
    let capturedControllerArgs = null;
    let panelMounted = false;
    const { module, document } = loadCustomModule({
      windowOverrides: {
        __metadata_checker_echarts_resolver_factory: async () => ({
          echarts: { version: "4.x" },
          source: "commons/echarts/echarts-ext",
          diagnostics: [],
        }),
        __metadata_checker_graph_renderer_factory: ({ echarts }) => ({
          echarts,
          render: async () => ({ renderer: echarts ? "echarts" : "html" }),
          renderError: async () => ({ renderer: "error" }),
        }),
        __metadata_checker_graph_panel_host_factory: ({ renderer }) => ({
          renderer,
          mount() {
            panelMounted = true;
            return { mounted: true };
          },
          render(result) {
            return renderer.render(result);
          },
          renderError(error) {
            return renderer.renderError(error);
          },
        }),
        __metadata_checker_controller_factory: (args) => {
          capturedControllerArgs = args;
          return {
            async init() {
              return { status: "ready" };
            },
            status: () => ({ state: "ready" }),
          };
        },
      },
    });

    const result = await module.onInitDesigner({}, {});

    assert.strictEqual(result.installed, true);
    assert.strictEqual(panelMounted, true);
    assert.strictEqual(marker(document, "graph-panel"), "mounted");
    assert.strictEqual(marker(document, "graph-renderer"), "echarts");
    assert.ok(capturedControllerArgs.graphRenderer);
    assert.strictEqual(capturedControllerArgs.selectionDebounceMs, 50);
  });

  it("derives Service Worker scope from script directory", async () => {
    const serviceWorker = createServiceWorkerMock();
    const { module } = loadCustomModule({ serviceWorker });

    const result = await module.onInitDesigner({}, {});

    assert.strictEqual(result.installed, true);
    assert.strictEqual(serviceWorker.registerCalls.length, 1);
    assert.deepStrictEqual(JSON.parse(JSON.stringify(serviceWorker.registerCalls[0])), {
      scriptUrl: "/analyzer/public/hooks/metadata-checker-sw.js",
      registerOptions: { scope: "/analyzer/public/hooks/" },
    });
  });

  it("inline page rc provider calls BI rc object-url contract", async () => {
    const rcCalls = [];
    let contentResult = null;
    let infoResult = null;
    const { module } = loadCustomModule({
      windowOverrides: {
        SZ: {
          async rc(request) {
            rcCalls.push(request);
            if (request.url.startsWith("/api/meta/services/getFileContent/")) {
              return '{"components":[]}';
            }
            if (request.url.startsWith("/api/meta/services/getFileInfo/")) {
              return { revision: "9", updated_at: "2026-05-24" };
            }
            throw new Error(`unexpected rc url: ${request.url}`);
          },
        },
        __metadata_checker_controller_factory: ({ provider }) => ({
          async init() {
            contentResult = await provider.getFileContent({
              source_path: "app/M40HookSmoke.app/M40HookDesign.spg",
              file_id: "EwLEBjaYNhLFYTaK6rxMNC",
              project_name: "analyzer",
            });
            infoResult = await provider.getFileInfo({
              source_path: "app/M40HookSmoke.app/M40HookDesign.spg",
              file_id: "",
              project_name: "analyzer",
            });
            return { status: "ready" };
          },
          status: () => ({ state: "ready" }),
        }),
      },
    });

    const result = await module.onInitDesigner({}, {});

    assert.strictEqual(result.installed, true);
    assert.strictEqual(contentResult.raw_text, '{"components":[]}');
    assert.strictEqual(infoResult.revision, "9");
    assert.deepStrictEqual(JSON.parse(JSON.stringify(rcCalls)), [
      {
        url: "/api/meta/services/getFileContent/EwLEBjaYNhLFYTaK6rxMNC",
        dataType: "text",
      },
      {
        url: "/api/meta/services/getFileInfo/analyzer/app/M40HookSmoke.app/M40HookDesign.spg",
      },
    ]);
  });

  it("DOM renderer exposes provider error message marker", async () => {
    const { module, document } = loadCustomModule({
      windowOverrides: {
        SZ: {
          async rc() {
            throw new Error("mock rc failure");
          },
        },
        __metadata_checker_controller_factory: ({ provider, renderer }) => ({
          async init() {
            const result = await provider.getFileContent({
              source_path: "app/M40HookSmoke.app/M40HookDesign.spg",
              file_id: "fid1",
              project_name: "analyzer",
            });
            renderer.renderError(result);
            return { status: "ready" };
          },
          status: () => ({ state: "ready" }),
        }),
      },
    });

    const result = await module.onInitDesigner({}, {});

    assert.strictEqual(result.installed, true);
    assert.strictEqual(marker(document, "last-render"), "error");
    assert.strictEqual(marker(document, "last-render-error-code"), "REMOTE_FETCH_FAILED");
    assert.strictEqual(marker(document, "last-render-error-message"), "rc getFileContent failed");
  });

  it("real service-worker launcher receives explicit requestTimeoutMs", async () => {
    let capturedOptions = null;
    const { module } = loadCustomModule({
      windowOverrides: {
        __metadata_checker_service_worker_launcher: (options) => {
          capturedOptions = options;
          return {
            start: async () => ({
              _kind: "service-worker",
              initRuntime: async () => ({ status: "ready" }),
              runtimeStatus: async () => ({ status: "ready" }),
              loadSuperpageDocument: async () => ({ status: "ready" }),
              loadRemoteSuperpageDocument: async () => ({ status: "ready" }),
              buildOrUpdateSuperpageGraph: async () => ({ status: "ready" }),
              analyzeSuperpageSelection: async () => ({ status: "ready" }),
            }),
            stop() {},
          };
        },
      },
    });

    const result = await module.onInitDesigner({}, {});

    assert.strictEqual(result.installed, true);
    assert.strictEqual(capturedOptions.requestTimeoutMs, 5000);
  });

  it("installing worker activation clears waitForState timeout", async () => {
    const timers = createFakeTimers();
    const serviceWorker = createServiceWorkerMock({ installing: true });
    const { module } = loadCustomModule({
      serviceWorker,
      setTimeout: timers.setTimeout,
      clearTimeout: timers.clearTimeout,
    });

    const initPromise = module.onInitDesigner({}, {});
    await waitUntil(
      () => timers.timers.size === 1,
      "activation wait timeout should be scheduled",
    );

    assert.strictEqual(timers.timers.size, 1, "activation wait timeout should be scheduled");
    const timeoutId = Array.from(timers.timers.keys())[0];
    serviceWorker.installingWorker.activate();
    const result = await initPromise;

    assert.strictEqual(result.installed, true);
    assert.ok(timers.cleared.includes(timeoutId), "activation timeout should be cleared");
    assert.strictEqual(timers.timers.size, 0);
    assert.strictEqual(serviceWorker.installingWorker.listenerCount(), 0);
  });

  it("uses registration.active when navigator.serviceWorker.controller is not ready", async () => {
    const serviceWorker = createServiceWorkerMock({
      controller: null,
      onWorkerRequest(request, respond) {
        respond({
          id: request.id,
          ok: true,
          result: { status: "ready", items: [], diagnostics: [] },
        });
      },
    });
    const { module, document } = loadCustomModule({
      serviceWorker,
      windowOverrides: {
        __metadata_checker_plugin_factory: ({ runtimeClient }) => ({
          async activate() {
            return runtimeClient.initRuntime({});
          },
          status: () => ({ state: "ready" }),
        }),
        __metadata_checker_controller_factory: ({ plugin }) => ({
          async init() {
            return plugin.activate();
          },
          status: () => ({ state: "ready" }),
        }),
      },
    });

    const result = await module.onInitDesigner({}, {});

    assert.strictEqual(result.installed, true);
    assert.strictEqual(result.fallbackUsed, false);
    assert.strictEqual(marker(document, "runtime"), "service-worker");
    assert.strictEqual(marker(document, "fallback-used"), "false");
    assert.strictEqual(marker(document, "fallback-code"), "none");
  });

  it("Service Worker register failure falls back and records fallback code", async () => {
    const { module, document } = loadCustomModule({
      serviceWorker: createServiceWorkerMock({ registerReject: "mock register failure" }),
    });
    const result = await module.onInitDesigner({}, {});

    assert.strictEqual(result.installed, true);
    assert.strictEqual(result.fallbackUsed, true);
    assert.strictEqual(marker(document, "sw"), "failed");
    assert.strictEqual(marker(document, "runtime"), "page-fallback");
    assert.strictEqual(marker(document, "wasm"), "page-fallback");
    assert.strictEqual(marker(document, "fallback-used"), "true");
    assert.strictEqual(marker(document, "fallback-code"), "SW_REGISTRATION_FAILED");
    assert.strictEqual(marker(document, "analysis-status"), "ready");
  });

  it("Service Worker runtime start failure falls back and records fallback code", async () => {
    const { module, document } = loadCustomModule({
      windowOverrides: {
        __metadata_checker_service_worker_launcher: () => ({
          start: async () => {
            throw new Error("mock runtime start failed");
          },
          stop() {},
        }),
      },
    });
    const result = await module.onInitDesigner({}, {});

    assert.strictEqual(result.installed, true);
    assert.strictEqual(result.fallbackUsed, true);
    assert.strictEqual(marker(document, "runtime"), "page-fallback");
    assert.strictEqual(marker(document, "fallback-used"), "true");
    assert.strictEqual(marker(document, "fallback-code"), "SW_RUNTIME_START_FAILED");
    assert.strictEqual(marker(document, "analysis-status"), "ready");
  });

  it("runtime wasm init failure switches to page fallback and records code", async () => {
    const { module, document } = loadCustomModule({
      windowOverrides: {
        __metadata_checker_service_worker_launcher: () => ({
          start: async () => ({
            _kind: "service-worker",
            initRuntime: async () => {
              throw new Error("mock wasm init failed");
            },
            runtimeStatus: async () => ({ status: "ready" }),
            loadSuperpageDocument: async () => ({ status: "ready" }),
            loadRemoteSuperpageDocument: async () => ({ status: "ready" }),
            buildOrUpdateSuperpageGraph: async () => ({ status: "ready" }),
            analyzeSuperpageSelection: async () => ({ status: "ready" }),
          }),
          stop() {},
        }),
        __metadata_checker_plugin_factory: ({ runtimeClient }) => ({
          async activate() {
            return runtimeClient.initRuntime({});
          },
          status: () => ({ state: "ready" }),
        }),
        __metadata_checker_controller_factory: ({ plugin }) => ({
          async init() {
            return plugin.activate();
          },
          status: () => ({ state: "ready" }),
        }),
      },
    });
    const result = await module.onInitDesigner({}, {});

    assert.strictEqual(result.installed, true);
    assert.strictEqual(result.fallbackUsed, true);
    assert.strictEqual(marker(document, "runtime"), "page-fallback");
    assert.strictEqual(marker(document, "wasm"), "page-fallback-ready");
    assert.strictEqual(marker(document, "fallback-used"), "true");
    assert.strictEqual(marker(document, "fallback-code"), "WASM_INIT_FAILED");
    assert.strictEqual(marker(document, "analysis-status"), "ready");
  });

  it("Service Worker request failure falls back with stable code instead of hanging", async () => {
    const { module, document } = loadCustomModule({
      serviceWorker: createServiceWorkerMock({
        controller: null,
        workerPostMessageThrow: "mock postMessage failure",
      }),
      windowOverrides: {
        __metadata_checker_plugin_factory: ({ runtimeClient }) => ({
          async activate() {
            return runtimeClient.initRuntime({});
          },
          status: () => ({ state: "ready" }),
        }),
        __metadata_checker_controller_factory: ({ plugin }) => ({
          async init() {
            return plugin.activate();
          },
          status: () => ({ state: "ready" }),
        }),
      },
    });

    const result = await module.onInitDesigner({}, {});

    assert.strictEqual(result.installed, true);
    assert.strictEqual(result.fallbackUsed, true);
    assert.strictEqual(marker(document, "runtime"), "page-fallback");
    assert.strictEqual(marker(document, "wasm"), "page-fallback-ready");
    assert.strictEqual(marker(document, "fallback-used"), "true");
    assert.strictEqual(marker(document, "fallback-code"), "SW_RUNTIME_REQUEST_FAILED");
    assert.strictEqual(marker(document, "analysis-status"), "ready");
  });

  it("Service Worker request timeout falls back instead of hanging", async () => {
    const timers = createFakeTimers();
    const serviceWorker = createServiceWorkerMock({
      controller: null,
      onWorkerRequest() {
        // postMessage succeeds, but the SW never responds.
      },
    });
    const { module, document } = loadCustomModule({
      serviceWorker,
      setTimeout: timers.setTimeout,
      clearTimeout: timers.clearTimeout,
      windowOverrides: {
        __metadata_checker_plugin_factory: ({ runtimeClient }) => ({
          async activate() {
            return runtimeClient.initRuntime({});
          },
          status: () => ({ state: "ready" }),
        }),
        __metadata_checker_controller_factory: ({ plugin }) => ({
          async init() {
            return plugin.activate();
          },
          status: () => ({ state: "ready" }),
        }),
      },
    });

    const initPromise = module.onInitDesigner({}, {});
    await waitUntil(
      () => timers.timers.size === 1,
      "request timeout should be scheduled",
    );

    assert.strictEqual(timers.timers.size, 1, "request timeout should be scheduled");
    timers.runNext();
    const result = await initPromise;

    assert.strictEqual(result.installed, true);
    assert.strictEqual(result.fallbackUsed, true);
    assert.strictEqual(marker(document, "runtime"), "page-fallback");
    assert.strictEqual(marker(document, "wasm"), "page-fallback-ready");
    assert.strictEqual(marker(document, "fallback-used"), "true");
    assert.strictEqual(marker(document, "fallback-code"), "SW_RUNTIME_REQUEST_TIMEOUT");
    assert.strictEqual(marker(document, "analysis-status"), "ready");
  });

  it("plugin/controller/glue missing returns stable result without uncaught throw", async () => {
    const missingPlugin = loadCustomModule({
      windowOverrides: {
        __metadata_checker_plugin_factory: null,
      },
    });
    const pluginResult = await missingPlugin.module.onInitDesigner({}, {});
    assert.strictEqual(pluginResult.installed, false);
    assert.strictEqual(pluginResult.reason, "plugin_factory_missing");
    assert.strictEqual(marker(missingPlugin.document, "plugin"), "missing");
    assert.strictEqual(
      marker(missingPlugin.document, "analysis-status"),
      "plugin_factory_missing",
    );

    const missingController = loadCustomModule({
      windowOverrides: {
        __metadata_checker_controller_factory: null,
      },
    });
    const controllerResult = await missingController.module.onInitDesigner({}, {});
    assert.strictEqual(controllerResult.installed, false);
    assert.strictEqual(controllerResult.reason, "controller_factory_missing");
    assert.strictEqual(marker(missingController.document, "controller"), "missing");

    const missingGlue = loadCustomModule({
      windowOverrides: {
        __metadata_checker_glue_factory: null,
      },
    });
    const glueResult = await missingGlue.module.onInitDesigner({}, {});
    assert.strictEqual(glueResult.installed, false);
    assert.strictEqual(glueResult.reason, "glue_factory_missing");
    assert.strictEqual(marker(missingGlue.document, "glue"), "missing");
  });
});

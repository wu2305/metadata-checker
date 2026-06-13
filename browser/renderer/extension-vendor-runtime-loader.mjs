/**
 * M47 扩展内 vendor runtime loader。
 *
 * 只负责从 extension web_accessible_resources 加载 Pixi/d3 bundle，不承载解析、
 * 图查询或业务推理。
 */

const DEFAULT_PIXI_BUNDLE = "spike-vendor/pixi-bundle.mjs";
const DEFAULT_D3_FORCE_BUNDLE = "spike-vendor/d3-force-3d-bundle.mjs";
const DEFAULT_ECHARTS_BUNDLE = "spike-vendor/echarts-bundle.mjs";

function resolveExtensionRuntime(options = {}) {
  if (options.runtime && typeof options.runtime.getURL === "function") {
    return options.runtime;
  }
  if (typeof globalThis.chrome !== "undefined" && globalThis.chrome?.runtime) {
    return globalThis.chrome.runtime;
  }
  if (typeof chrome !== "undefined" && chrome?.runtime) {
    return chrome.runtime;
  }
  return null;
}

function hasRuntimeGetURL(runtime) {
  return runtime && typeof runtime.getURL === "function";
}

function normalizePixiRuntime(module) {
  if (!module) return null;
  return module.PIXI ?? module.default ?? module;
}

function normalizeD3Runtime(module) {
  if (!module) return null;
  return module.D3Force3D ?? module.default ?? module;
}

function normalizeEchartsRuntime(module) {
  if (!module) return null;
  return module.echarts ?? module.default ?? module;
}

export function createExtensionVendorRuntimeLoader(options = {}) {
  const runtime = resolveExtensionRuntime(options);
  const importer = typeof options.importer === "function"
    ? options.importer
    : (specifier) => import(specifier);
  let lastLoadError = "";
  const pixiBundle = options.pixiBundle ?? DEFAULT_PIXI_BUNDLE;
  const d3ForceBundle = options.d3ForceBundle ?? DEFAULT_D3_FORCE_BUNDLE;
  const echartsBundle = options.echartsBundle ?? DEFAULT_ECHARTS_BUNDLE;

  const cache = {
    pixi: undefined,
    d3Force3D: undefined,
    echarts: undefined,
  };

  async function importRuntimeBundle(path) {
    if (!hasRuntimeGetURL(runtime)) {
      lastLoadError = "extension runtime.getURL is unavailable";
      return null;
    }
    try {
      return await importer(runtime.getURL(path));
    } catch (error) {
      lastLoadError = error?.message ? String(error.message) : `failed to import ${path}`;
      return null;
    }
  }

  async function loadPixi() {
    if (cache.pixi !== undefined) {
      return cache.pixi;
    }
    cache.pixi = normalizePixiRuntime(await importRuntimeBundle(pixiBundle));
    return cache.pixi;
  }

  async function loadD3Force3D() {
    if (cache.d3Force3D !== undefined) {
      return cache.d3Force3D;
    }
    cache.d3Force3D = normalizeD3Runtime(await importRuntimeBundle(d3ForceBundle));
    return cache.d3Force3D;
  }

  async function loadEcharts() {
    if (cache.echarts !== undefined) {
      return cache.echarts;
    }
    cache.echarts = normalizeEchartsRuntime(await importRuntimeBundle(echartsBundle));
    return cache.echarts;
  }

  async function loadAll() {
    const [pixi, d3Force3D, echarts] = await Promise.all([
      loadPixi(),
      loadD3Force3D(),
      loadEcharts(),
    ]);
    return { pixi, d3Force3D, echarts };
  }

  return {
    loadPixi,
    loadD3Force3D,
    loadEcharts,
    loadAll,
    getLastLoadError() {
      return lastLoadError;
    },
  };
}

export {
  DEFAULT_PIXI_BUNDLE,
  DEFAULT_D3_FORCE_BUNDLE,
  DEFAULT_ECHARTS_BUNDLE,
  normalizePixiRuntime,
  normalizeD3Runtime,
  normalizeEchartsRuntime,
  resolveExtensionRuntime,
};

import { createMetadataCheckerController } from "./integration/metadata-checker-controller.mjs";
import { createMetadataCheckerPlugin } from "./plugin-core/metadata-checker-plugin.mjs";
import { installSuperPageDesignerGlue } from "./platform-glue/superpage-designer-glue.mjs";
import { resolveEchartsRuntime } from "./renderer/echarts-runtime-resolver.mjs";
import { createGraphPanelHost } from "./renderer/graph-panel-host.mjs";
import { createGraphPanelRenderer } from "./renderer/graph-panel-renderer.mjs";

const MARKER_PREFIX = "data-metadata-checker-";

function writeMarker(documentLike, name, value) {
  if (!documentLike || typeof documentLike.createElement !== "function") return;
  const key = `${MARKER_PREFIX}${name}`;
  const existing = documentLike.querySelector?.(`[${key}]`);
  if (existing) {
    existing.setAttribute(key, value);
    return;
  }
  const marker = documentLike.createElement("span");
  marker.setAttribute(key, value);
  marker.style.display = "none";
  if (documentLike.body) {
    documentLike.body.appendChild(marker);
  }
}

export function installMetadataCheckerBrowserFactories(options = {}) {
  const windowLike = options.window ?? globalThis.window ?? globalThis;
  const documentLike = options.document ?? windowLike.document ?? globalThis.document;

  windowLike.__metadata_checker_plugin_factory = createMetadataCheckerPlugin;
  windowLike.__metadata_checker_controller_factory = createMetadataCheckerController;
  windowLike.__metadata_checker_glue_factory = installSuperPageDesignerGlue;
  windowLike.__metadata_checker_echarts_resolver_factory = resolveEchartsRuntime;
  windowLike.__metadata_checker_graph_renderer_factory = createGraphPanelRenderer;
  windowLike.__metadata_checker_graph_panel_host_factory = createGraphPanelHost;

  writeMarker(documentLike, "real-bundle", "loaded");
  writeMarker(documentLike, "factories", "installed");
  return {
    installed: true,
    factories: [
      "plugin",
      "controller",
      "glue",
      "echarts_resolver",
      "graph_renderer",
      "graph_panel_host",
    ],
  };
}

export default installMetadataCheckerBrowserFactories;

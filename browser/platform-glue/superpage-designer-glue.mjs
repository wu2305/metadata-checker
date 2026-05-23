/**
 * M40.5：SuperPage Designer Platform Glue
 *
 * 把 BI SuperPage 设计器特化逻辑限制在 Glue JS 中，
 * 将设计器选择态转换为 Plugin Core 的 snake_case selection contract。
 */

function _makeErrorEnvelope(code, message, extra = {}) {
  return {
    status: "error",
    target: null,
    items: [],
    diagnostics: [
      {
        severity: "error",
        code,
        message,
        ...extra,
      },
    ],
  };
}

function _debounce(fn, ms = 50) {
  let timer = null;
  return function (...args) {
    if (timer) {
      clearTimeout(timer);
    }
    timer = setTimeout(() => {
      timer = null;
      fn.apply(this, args);
    }, ms);
  };
}

function _extractSourcePath(designer) {
  const path = designer?.openFileArgs?.path;
  if (typeof path !== "string") {
    return null;
  }
  // 去掉 /analyzer/ 前缀，转为项目内逻辑路径
  if (path.startsWith("/analyzer/")) {
    return path.slice("/analyzer/".length);
  }
  if (path.startsWith("/")) {
    // 其他绝对路径，去掉首个 /
    return path.slice(1);
  }
  return path;
}

function _extractFileId(designer, host, logger) {
  const id =
    designer?.openFileArgs?.id ??
    designer?.getFileInfo?.()?.id ??
    designer?.metaFileInfo?.id ??
    "";
  if (id === "" && host && typeof host.emit === "function") {
    host.emit("file_id_missing", {
      message: "file_id not found in designer, using empty fallback",
      timestamp: Date.now(),
    });
  }
  return id;
}

function _buildSelection(designer) {
  const builder = designer?.getBuilder?.();
  if (!builder) {
    return null;
  }

  const sourcePath = _extractSourcePath(designer);
  if (!sourcePath) {
    return null;
  }

  const fileId = _extractFileId(designer);

  const components = builder.getSelectedComponents?.() ?? [];
  const selectedComponentIds = [];
  const selectionInfos = {};

  for (const comp of components) {
    const id = typeof comp?.getId === "function" ? comp.getId() : comp?.id;
    if (typeof id === "string") {
      selectedComponentIds.push(id);
      const info = builder.getSelectedComponentInfo?.(id);
      if (info && typeof info === "object") {
        selectionInfos[id] = { id: info.id ?? id };
        if (info.floatInfo !== undefined) {
          selectionInfos[id].floatInfo = info.floatInfo;
        }
      }
    }
  }

  let activeComponentId = null;
  const activeComp = builder.getSelectedComponent?.();
  if (activeComp) {
    activeComponentId =
      typeof activeComp?.getId === "function"
        ? activeComp.getId()
        : activeComp?.id ?? null;
  }
  if (activeComponentId === null && selectedComponentIds.length > 0) {
    activeComponentId = selectedComponentIds[0];
  }

  const projectName =
    designer?.openFileArgs?.projectName ??
    (sourcePath ? sourcePath.split("/")[0] : "");

  return {
    source_path: sourcePath,
    file_id: fileId,
    selected_component_ids: selectedComponentIds,
    active_component_id: activeComponentId,
    timestamp: Date.now(),
    designer_kind: "superpage",
    project_name: projectName,
    selection_infos: selectionInfos,
  };
}

function _patchBuilderSelectComponents(builder, plugin, host, logger) {
  const original = builder.selectComponents;
  builder.selectComponents = function (infos, clearOthers) {
    const result = original.apply(this, arguments);
    try {
      const selection = _buildSelection(builder.__designerRef);
      if (selection) {
        const pluginResult = plugin.onSelectionChanged(selection);
        if (pluginResult && pluginResult.status === "error") {
          const log = logger ?? console;
          log.warn?.("[glue] plugin.onSelectionChanged returned error:", pluginResult);
          if (host && typeof host.emit === "function") {
            host.emit("glue_selection_failed", {
              error: pluginResult,
              timestamp: Date.now(),
            });
          }
        }
      }
    } catch (err) {
      const log = logger ?? console;
      log.error?.("[glue] error in patched selectComponents:", err);
      if (host && typeof host.emit === "function") {
        host.emit("glue_selection_failed", {
          error: _makeErrorEnvelope("GLUE_ERROR", err?.message ?? String(err)),
          timestamp: Date.now(),
        });
      }
    }
    return result;
  };
  builder.__originalSelectComponents = original;
}

function _patchBuilderDoSelectedChange(builder, plugin, host, logger) {
  const original = builder.doSelectedChange;
  builder.doSelectedChange = function () {
    const result = original.apply(this, arguments);
    try {
      const selection = _buildSelection(builder.__designerRef);
      if (selection) {
        const pluginResult = plugin.onSelectionChanged(selection);
        if (pluginResult && pluginResult.status === "error") {
          const log = logger ?? console;
          log.warn?.("[glue] plugin.onSelectionChanged returned error:", pluginResult);
          if (host && typeof host.emit === "function") {
            host.emit("glue_selection_failed", {
              error: pluginResult,
              timestamp: Date.now(),
            });
          }
        }
      }
    } catch (err) {
      const log = logger ?? console;
      log.error?.("[glue] error in patched doSelectedChange:", err);
      if (host && typeof host.emit === "function") {
        host.emit("glue_selection_failed", {
          error: _makeErrorEnvelope("GLUE_ERROR", err?.message ?? String(err)),
          timestamp: Date.now(),
        });
      }
    }
    return result;
  };
  builder.__originalDoSelectedChange = original;
}

export { _extractSourcePath, _extractFileId, _buildSelection };

export function installSuperPageDesignerGlue(designer, args, plugin, options = {}) {
  const host = options.host;
  const logger = options.logger;

  if (!designer || typeof designer !== "object") {
    const envelope = _makeErrorEnvelope(
      "GLUE_INSTALL_FAILED",
      "designer is required and must be an object"
    );
    if (host && typeof host.emit === "function") {
      host.emit("glue_install_failed", { error: envelope, timestamp: Date.now() });
    }
    return envelope;
  }

  if (!plugin || typeof plugin.onSelectionChanged !== "function") {
    const envelope = _makeErrorEnvelope(
      "GLUE_INSTALL_FAILED",
      "plugin.onSelectionChanged is required"
    );
    if (host && typeof host.emit === "function") {
      host.emit("glue_install_failed", { error: envelope, timestamp: Date.now() });
    }
    return envelope;
  }

  if (designer.__metadataCheckerGlueInstalled) {
    return { installed: true, alreadyInstalled: true };
  }

  if (!designer.openFileArgs?.path) {
    const envelope = _makeErrorEnvelope(
      "GLUE_INSTALL_FAILED",
      "designer.openFileArgs.path is required"
    );
    if (host && typeof host.emit === "function") {
      host.emit("glue_install_failed", { error: envelope, timestamp: Date.now() });
    }
    return envelope;
  }

  const builder = designer.getBuilder?.();
  if (!builder) {
    const envelope = _makeErrorEnvelope(
      "GLUE_INSTALL_FAILED",
      "designer.getBuilder() returned null or undefined"
    );
    if (host && typeof host.emit === "function") {
      host.emit("glue_install_failed", { error: envelope, timestamp: Date.now() });
    }
    return envelope;
  }

  // 让 builder 能反向访问 designer 以构建 selection
  builder.__designerRef = designer;

  _patchBuilderSelectComponents(builder, plugin, host, logger);
  _patchBuilderDoSelectedChange(builder, plugin, host, logger);

  designer.__metadataCheckerGlueInstalled = true;

  return { installed: true };
}

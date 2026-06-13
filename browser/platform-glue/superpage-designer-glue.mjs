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

function _emitHost(host, eventName, payload) {
  if (host && typeof host.emit === "function") {
    host.emit(eventName, payload);
  }
}

function _log(logger, level, ...args) {
  const log = logger ?? console;
  log?.[level]?.(...args);
}

function _emitInstallFailed(host, code, message) {
  const envelope = _makeErrorEnvelope(code, message);
  _emitHost(host, "glue_install_failed", { error: envelope, timestamp: Date.now() });
  return envelope;
}

function _extractSourcePath(designer) {
  const path = designer?.openFileArgs?.path;
  if (typeof path !== "string") {
    return null;
  }

  const normalizedPath = path.replace(/^\/+/, "");
  if (normalizedPath === "") {
    return null;
  }

  const projectName = designer?.openFileArgs?.projectName;
  if (
    typeof projectName === "string" &&
    projectName !== "" &&
    normalizedPath.startsWith(`${projectName}/`)
  ) {
    return normalizedPath.slice(projectName.length + 1);
  }

  if (path.startsWith("/")) {
    const slashIndex = normalizedPath.indexOf("/");
    return slashIndex >= 0 ? normalizedPath.slice(slashIndex + 1) : normalizedPath;
  }

  return normalizedPath;
}

function _extractProjectName(designer) {
  const projectName = designer?.openFileArgs?.projectName;
  if (typeof projectName === "string" && projectName !== "") {
    return projectName;
  }

  const path = designer?.openFileArgs?.path;
  if (typeof path !== "string" || !path.startsWith("/")) {
    return "";
  }

  const normalizedPath = path.replace(/^\/+/, "");
  const slashIndex = normalizedPath.indexOf("/");
  return slashIndex > 0 ? normalizedPath.slice(0, slashIndex) : "";
}

function _extractFileId(designer, host, logger) {
  const id =
    designer?.openFileArgs?.id ??
    designer?.getFileInfo?.()?.id ??
    designer?.metaFileInfo?.id ??
    "";
  if (id === "") {
    const payload = {
      message: "file_id not found in designer, using empty fallback",
      timestamp: Date.now(),
    };
    _emitHost(host, "file_id_missing", payload);
    if (logger) {
      _log(logger, "warn", "[glue] file_id not found in designer, using empty fallback");
    }
  }
  return id;
}

function _extractRevision(designer, host, logger) {
  const revision =
    designer?.openFileArgs?.revision ??
    designer?.getFileInfo?.()?.revision ??
    designer?.metaFileInfo?.revision ??
    designer?.fileInfo?.revision ??
    "";
  if (typeof revision === "string" && revision !== "") {
    return revision;
  }
  if (typeof revision === "number" && Number.isFinite(revision)) {
    return String(revision);
  }
  if (logger) {
    _log(logger, "warn", "[glue] revision not available in designer, using empty fallback");
  }
  if (typeof host?.emit === "function") {
    _emitHost(host, "revision_missing", {
      message: "revision not found in designer, using null fallback",
      timestamp: Date.now(),
    });
  }
  return null;
}

function _buildSelection(designer, options = {}) {
  const host = options?.host ?? (typeof options?.emit === "function" ? options : null);
  const logger = options?.logger;
  const builder = designer?.getBuilder?.();
  if (!builder) {
    return null;
  }

  const sourcePath = _extractSourcePath(designer);
  if (!sourcePath) {
    return null;
  }

  const fileId = _extractFileId(designer, host, logger);
  const revision = _extractRevision(designer, host, logger);

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

  const projectName = _extractProjectName(designer);

  return {
    source_path: sourcePath,
    file_id: fileId,
    revision,
    selected_component_ids: selectedComponentIds,
    active_component_id: activeComponentId,
    timestamp: Date.now(),
    designer_kind: "superpage",
    project_name: projectName,
    selection_infos: selectionInfos,
  };
}

function _dispatchSelection(builder, plugin, host, logger, sourceMethod) {
  try {
    const selection = _buildSelection(builder.__designerRef, { host, logger });
    if (!selection) {
      return;
    }

    const pluginResult = plugin.onSelectionChanged(selection);
    if (pluginResult && pluginResult.status === "error") {
      _log(logger, "warn", "[glue] plugin.onSelectionChanged returned error:", pluginResult);
      _emitHost(host, "glue_selection_failed", {
        error: pluginResult,
        timestamp: Date.now(),
      });
    }
  } catch (err) {
    _log(logger, "error", `[glue] error in patched ${sourceMethod}:`, err);
    _emitHost(host, "glue_selection_failed", {
      error: _makeErrorEnvelope("GLUE_ERROR", err?.message ?? String(err)),
      timestamp: Date.now(),
    });
  }
}

function _patchBuilderSelectComponents(builder, plugin, host, logger) {
  const original = builder.selectComponents;
  builder.selectComponents = function (infos, clearOthers) {
    builder.__metadataCheckerSelectDepth = (builder.__metadataCheckerSelectDepth ?? 0) + 1;
    let result;
    try {
      result = original.apply(this, arguments);
    } finally {
      builder.__metadataCheckerSelectDepth -= 1;
    }
    _dispatchSelection(builder, plugin, host, logger, "selectComponents");
    return result;
  };
  builder.__originalSelectComponents = original;
}

function _patchBuilderDoSelectedChange(builder, plugin, host, logger) {
  const original = builder.doSelectedChange;
  builder.doSelectedChange = function () {
    const result = original.apply(this, arguments);
    if ((builder.__metadataCheckerSelectDepth ?? 0) === 0) {
      _dispatchSelection(builder, plugin, host, logger, "doSelectedChange");
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
    return _emitInstallFailed(
      host,
      "GLUE_INSTALL_FAILED",
      "designer is required and must be an object"
    );
  }

  if (!plugin || typeof plugin.onSelectionChanged !== "function") {
    return _emitInstallFailed(
      host,
      "GLUE_INSTALL_FAILED",
      "plugin.onSelectionChanged is required"
    );
  }

  if (designer.__metadataCheckerGlueInstalled) {
    return { installed: true, alreadyInstalled: true };
  }

  if (!designer.openFileArgs?.path) {
    return _emitInstallFailed(
      host,
      "GLUE_INSTALL_FAILED",
      "designer.openFileArgs.path is required"
    );
  }

  const builder = designer.getBuilder?.();
  if (!builder) {
    return _emitInstallFailed(
      host,
      "GLUE_INSTALL_FAILED",
      "designer.getBuilder() returned null or undefined"
    );
  }

  const hasSelectComponents = typeof builder.selectComponents === "function";
  const hasDoSelectedChange = typeof builder.doSelectedChange === "function";
  if (!hasSelectComponents && !hasDoSelectedChange) {
    return _emitInstallFailed(
      host,
      "GLUE_INSTALL_FAILED",
      "builder must provide selectComponents or doSelectedChange"
    );
  }

  // 让 builder 能反向访问 designer 以构建 selection
  builder.__designerRef = designer;

  if (hasSelectComponents) {
    _patchBuilderSelectComponents(builder, plugin, host, logger);
  }
  if (hasDoSelectedChange) {
    _patchBuilderDoSelectedChange(builder, plugin, host, logger);
  }

  designer.__metadataCheckerGlueInstalled = true;

  return { installed: true };
}

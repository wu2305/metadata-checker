(function defineMetadataCheckerBridgeProtocol(root) {
  "use strict";

  const PROTOCOL_NAME = "metadata_checker_designer_bridge";
  const PROTOCOL_VERSION = "m43-protocol-v1";
  const READY_EVENT = "__metadata_checker_designer_ready__";
  const SELECTION_REQUESTED_EVENT = "__metadata_checker_selection_requested__";
  const SELECTION_RESPONSE_EVENT = "__metadata_checker_selection_response__";

  const BRIDGE_PROTOCOL = {
    name: PROTOCOL_NAME,
    version: PROTOCOL_VERSION,
    events: {
      ready: READY_EVENT,
      selection_requested: SELECTION_REQUESTED_EVENT,
      selection_response: SELECTION_RESPONSE_EVENT,
    },
  };

  function _coerceString(value, fallback = "") {
    return typeof value === "string" ? value : fallback;
  }

  function _coerceArray(value) {
    return Array.isArray(value) ? value : [];
  }

  function _coerceObject(value) {
    return value && typeof value === "object" ? value : {};
  }

  function _makeDiagnostic(code, message, severity = "info") {
    return {
      severity,
      code,
      message,
    };
  }

  function _extractProjectName(designer) {
    const openFileArgs = _coerceObject(designer?.openFileArgs);
    const fromArgs = _coerceString(openFileArgs.projectName);
    if (fromArgs !== "") {
      return fromArgs;
    }

    const path = _coerceString(openFileArgs.path);
    if (path === "") {
      return "";
    }
    const normalized = path.startsWith("/") ? path.slice(1) : path;
    const firstSlash = normalized.indexOf("/");
    return firstSlash > 0 ? normalized.slice(0, firstSlash) : "";
  }

  function _extractSourcePath(designer) {
    const openFileArgs = _coerceObject(designer?.openFileArgs);
    const rawPath = _coerceString(openFileArgs.path);
    if (rawPath === "") {
      return null;
    }

    const normalized = rawPath.startsWith("/") ? rawPath.slice(1) : rawPath;
    const projectName = _coerceString(openFileArgs.projectName);
    if (projectName !== "" && normalized.startsWith(`${projectName}/`)) {
      return normalized.slice(projectName.length + 1);
    }

    return normalized || null;
  }

  function _extractFileId(designer) {
    const openFileArgs = _coerceObject(designer?.openFileArgs);
    return _coerceString(openFileArgs.file_id) || _coerceString(openFileArgs.id);
  }

  function _inferPageKindFromPath(sourcePath) {
    if (typeof sourcePath !== "string") {
      return "unknown";
    }
    if (sourcePath.endsWith(".spg")) {
      return "superpage";
    }
    if (sourcePath.endsWith(".tbl")) {
      return "table";
    }
    return "unknown";
  }

  function getPageContext(designer, args = {}) {
    const sourcePath = _extractSourcePath(designer);
    const projectName = _extractProjectName(designer);
    const fileId = _extractFileId(designer);
    const openFileArgs = _coerceObject(designer?.openFileArgs);

    const contextDiagnostics = [];
    if (sourcePath === null) {
      contextDiagnostics.push(
        _makeDiagnostic("BRIDGE_NO_SOURCE_PATH", "designer open file path is missing"),
      );
    }

    const pageKind = _inferPageKindFromPath(sourcePath ?? "");
    return {
      project_name: projectName,
      source_path: sourcePath,
      page_type: pageKind,
      file_id: fileId,
      is_edit_mode: Boolean(args?.isEditMode ?? openFileArgs?.mode === "edit"),
      designer_type: designer?.type === "table" ? "table" : "superpage",
      diagnostics: contextDiagnostics,
    };
  }

  function _extractComponentId(component) {
    if (!component) {
      return null;
    }
    if (typeof component.getId === "function") {
      const fromGetter = component.getId();
      if (typeof fromGetter === "string") {
        return fromGetter;
      }
    }
    if (typeof component.id === "string") {
      return component.id;
    }
    return null;
  }

  function _snapshotSelectionInfo(componentInfo) {
    const info = _coerceObject(componentInfo);
    const id = _coerceString(info.id);
    const result = {
      id,
    };
    if (Object.prototype.hasOwnProperty.call(info, "floatInfo")) {
      result.float_info = info.floatInfo;
    }
    return result;
  }

  function getSelectionSnapshot(designer) {
    const pageContext = getPageContext(designer);
    const builder = designer?.getBuilder?.();
    if (!builder || typeof builder.getSelectedComponents !== "function") {
      return {
        protocol: BRIDGE_PROTOCOL,
        page_context: pageContext,
        selection: {
          source_path: pageContext.source_path,
          file_id: pageContext.file_id,
          selected_component_ids: [],
          active_component_id: null,
          selected_component_infos: {},
          project_name: pageContext.project_name,
          timestamp: Date.now(),
          page_type: pageContext.page_type,
        },
        diagnostics: [
          _makeDiagnostic(
            "BRIDGE_NO_SELECTION",
            "designer has no selectable components",
            "warn",
          ),
        ],
      };
    }

    const selectedComponents = _coerceArray(builder.getSelectedComponents());
    const selectedIds = [];
    const selectedComponentInfos = {};

    for (const component of selectedComponents) {
      const id = _extractComponentId(component);
      if (!id) {
        continue;
      }
      selectedIds.push(id);
      if (typeof builder.getSelectedComponentInfo === "function") {
        selectedComponentInfos[id] = _snapshotSelectionInfo(
          builder.getSelectedComponentInfo(id),
        );
      } else {
        selectedComponentInfos[id] = { id };
      }
    }

    let activeComponentId = null;
    if (typeof builder.getSelectedComponent === "function") {
      const active = builder.getSelectedComponent();
      activeComponentId = _extractComponentId(active);
    }
    if (activeComponentId === null && selectedIds.length > 0) {
      activeComponentId = selectedIds[0];
    }

    const selectionDiagnostics = [];
    if (selectedIds.length === 0) {
      selectionDiagnostics.push(
        _makeDiagnostic("BRIDGE_NO_SELECTION", "designer selection is empty", "warn"),
      );
    }

    return {
      page_context: pageContext,
      selection: {
        source_path: pageContext.source_path,
        file_id: pageContext.file_id,
        selected_component_ids: selectedIds,
        active_component_id: activeComponentId,
        selected_component_infos: selectedComponentInfos,
        project_name: pageContext.project_name,
        timestamp: Date.now(),
        page_type: pageContext.page_type,
      },
      diagnostics: selectionDiagnostics,
    };
  }

  function createBridgePayload(designer, args) {
    const pageContext = getPageContext(designer, args);
    const selectionResult = getSelectionSnapshot(designer);
    const diagnostics = [
      ...(pageContext.diagnostics ?? []),
      ...(selectionResult.diagnostics ?? []),
    ];

    return {
      protocol: BRIDGE_PROTOCOL,
      page_context: pageContext,
      selection: selectionResult.selection,
      diagnostics,
    };
  }

  const exports = {
    BRIDGE_PROTOCOL_NAME: PROTOCOL_NAME,
    BRIDGE_PROTOCOL_VERSION: PROTOCOL_VERSION,
    BRIDGE_PROTOCOL_EVENT_READY: READY_EVENT,
    BRIDGE_PROTOCOL,
    getPageContext,
    getSelectionSnapshot,
    createBridgePayload,
  };

  root.__metadata_checker_bridge_protocol__ = exports;
})(typeof globalThis !== "undefined" ? globalThis : window);

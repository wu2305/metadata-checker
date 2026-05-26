define(function () {
  "use strict";

  const protocolExports = typeof window !== "undefined"
    ? window.__metadata_checker_bridge_protocol__
    : null;

  const protocol = protocolExports?.BRIDGE_PROTOCOL ?? {
    name: "metadata_checker_designer_bridge",
    version: "m43-protocol-v1",
    events: {
      ready: "__metadata_checker_designer_ready__",
      selection_requested: "__metadata_checker_selection_requested__",
      selection_response: "__metadata_checker_selection_response__",
    },
  };
  const getPageContext = protocolExports?.getPageContext ?? defaultGetPageContext;
  const getSelectionSnapshot = protocolExports?.getSelectionSnapshot ?? defaultGetSelectionSnapshot;

  let bridgeInstance = null;
  let lastDesigner = null;
  let lastArgs = {};
  let initState = "uninitialized";

  function _writeMarker(name, value) {
    if (typeof document === "undefined") {
      return;
    }
    const key = `data-metadata-checker-${name}`;
    const existing = document.querySelector(`[${key}]`);
    if (existing && typeof existing.setAttribute === "function") {
      existing.setAttribute(key, value);
      return;
    }

    const marker = document.createElement
      ? document.createElement("span")
      : {
          setAttribute() {},
          style: {},
        };
    if (typeof marker.setAttribute === "function") {
      marker.setAttribute(key, value);
      marker.style.display = "none";
      if (document.body && typeof document.body.appendChild === "function") {
        document.body.appendChild(marker);
      }
    }
  }

  function buildBridgeStatus() {
    const pageContext = getPageContext(lastDesigner, lastArgs);
    const selectionResult = getSelectionSnapshot(lastDesigner);
    const diagnostics = [
      ...(pageContext.diagnostics ?? []),
      ...(selectionResult.diagnostics ?? []),
    ];
    return {
      protocol,
      page_context: pageContext,
      selection: selectionResult.selection ?? null,
      diagnostics,
    };
  }

  function getBridgeStatus() {
    return buildBridgeStatus();
  }

  function getCurrentPageContext() {
    const pageContext = getPageContext(lastDesigner, lastArgs);
    return {
      protocol,
      page_context: pageContext,
      selection: null,
      diagnostics: pageContext.diagnostics ?? [],
    };
  }

  function getCurrentSelectionSnapshot() {
    const selection = getSelectionSnapshot(lastDesigner);
    return {
      protocol,
      page_context: selection.page_context,
      selection: selection.selection,
      diagnostics: selection.diagnostics ?? [],
    };
  }

  function ensureBridge() {
    if (!bridgeInstance) {
      bridgeInstance = {
        getBridgeStatus,
        getPageContext: getCurrentPageContext,
        getSelectionSnapshot: getCurrentSelectionSnapshot,
      };
      if (typeof window !== "undefined") {
        window.__metadata_checker_designer_bridge__ = bridgeInstance;
      }
    }
    return bridgeInstance;
  }

  function dispatchReadyEvent() {
    if (typeof document === "undefined" || typeof document.dispatchEvent !== "function") {
      return;
    }
    const detail = getBridgeStatus();
    const event = typeof CustomEvent === "function"
      ? new CustomEvent(protocol.events.ready, {
        bubbles: true,
        detail,
      })
      : {
        type: protocol.events.ready,
        detail,
        bubbles: true,
      };
    if (typeof event === "object" && event && typeof document.dispatchEvent === "function") {
      document.dispatchEvent(event);
    }
  }

  function normalizeDesignerRef(designer) {
    if (!designer || typeof designer !== "object") {
      return null;
    }
    return designer;
  }

  function onInitDesigner(designer, args = {}) {
    lastDesigner = normalizeDesignerRef(designer);
    lastArgs = args ?? {};
    const previousState = initState;
    initState = previousState === "uninitialized" ? "ready" : "updated";

    const bridge = ensureBridge();
    if (previousState === "uninitialized") {
      _writeMarker("bridge", "installed");
    } else {
      _writeMarker("bridge", "updated");
    }
    _writeMarker("bridge-protocol", protocol.version);
    _writeMarker("bridge-status", initState);

    dispatchReadyEvent();

    return bridge.getBridgeStatus();
  }

  return {
    onInitDesigner,
  };

  function defaultGetPageContext(designer, options = {}) {
    const openFileArgs = designer?.openFileArgs ?? {};
    const rawPath = typeof openFileArgs.path === "string" ? openFileArgs.path : "";
    const normalizedPath = rawPath.startsWith("/") ? rawPath.slice(1) : rawPath;
    const projectName = typeof openFileArgs.projectName === "string"
      ? openFileArgs.projectName
      : "";
    const sourcePath = projectName !== "" && normalizedPath.startsWith(`${projectName}/`)
      ? normalizedPath.slice(projectName.length + 1)
      : normalizedPath;
    return {
      project_name:
        typeof openFileArgs.projectName === "string" ? openFileArgs.projectName : "",
      source_path: sourcePath === "" ? null : sourcePath,
      page_type: sourcePath.endsWith(".spg")
        ? "superpage"
        : sourcePath.endsWith(".tbl")
          ? "table"
          : "unknown",
      file_id:
        typeof openFileArgs.file_id === "string"
          ? openFileArgs.file_id
          : typeof openFileArgs.id === "string"
            ? openFileArgs.id
            : "",
      is_edit_mode:
        options.isEditMode === true || openFileArgs.mode === "edit",
      designer_type: designer?.type === "table" ? "table" : "superpage",
      diagnostics: normalizedPath === ""
        ? [{ severity: "warn", code: "BRIDGE_NO_SOURCE_PATH", message: "designer open file path is missing" }]
        : [],
    };
  }

  function defaultGetSelectionSnapshot(designer) {
    const pageContext = defaultGetPageContext(designer);
    const builder = designer?.getBuilder?.();
    if (!builder || typeof builder.getSelectedComponents !== "function") {
      return {
        protocol,
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
          {
            severity: "warn",
            code: "BRIDGE_NO_SELECTION",
            message: "designer has no selectable components",
          },
        ],
      };
    }

    const selectedComponents = builder.getSelectedComponents() || [];
    const selectedIds = [];
    const selectedComponentInfos = {};

    for (const component of selectedComponents) {
      const id = typeof component?.getId === "function"
        ? component.getId()
        : component?.id;
      if (typeof id !== "string" || id === "") {
        continue;
      }
      selectedIds.push(id);
      if (typeof builder.getSelectedComponentInfo === "function") {
        const info = builder.getSelectedComponentInfo(id) ?? {};
        selectedComponentInfos[id] = {
          id: typeof info.id === "string" ? info.id : id,
        };
        if (Object.prototype.hasOwnProperty.call(info, "floatInfo")) {
          selectedComponentInfos[id].float_info = info.floatInfo;
        }
      } else {
        selectedComponentInfos[id] = { id };
      }
    }

    const active = typeof builder.getSelectedComponent === "function"
      ? builder.getSelectedComponent()
      : null;
    const activeId = typeof active?.getId === "function"
      ? active.getId()
      : active?.id ?? null;

    return {
      protocol,
      page_context: pageContext,
      selection: {
        source_path: pageContext.source_path,
        file_id: pageContext.file_id,
        selected_component_ids: selectedIds,
        active_component_id: activeId ?? (selectedIds[0] ?? null),
        selected_component_infos: selectedComponentInfos,
        project_name: pageContext.project_name,
        timestamp: Date.now(),
        page_type: pageContext.page_type,
      },
      diagnostics: selectedIds.length === 0
        ? [
            {
              severity: "warn",
              code: "BRIDGE_NO_SELECTION",
              message: "designer selection is empty",
            },
          ]
        : [],
    };
  }
});

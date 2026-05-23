/**
 * Fake BI SuperPage Designer for testing
 */

export function createFakeDesigner(options = {}) {
  let selectedComponents = options.selectedComponents ?? [
    { getId: () => "webview1", id: "webview1" },
  ];
  const componentInfos = options.componentInfos ?? {};
  const fileId = options.missingFileId ? undefined : options.fileId ?? "fid1";
  const projectName = options.missingProjectName ? undefined : options.projectName ?? "Test";
  const path = options.path ?? "/analyzer/app/Test.app/Page.spg";

  const callLog = [];
  const patchedMethods = new Set();

  function logCall(method, args) {
    callLog.push({ method, args: Array.from(args) });
  }

  const builder = {
    getSelectedComponents() {
      logCall("getSelectedComponents", []);
      return selectedComponents;
    },

    getSelectedComponent() {
      logCall("getSelectedComponent", []);
      if (options.activeComponentNull) {
        return null;
      }
      return selectedComponents[0] ?? null;
    },

    getSelectedComponentInfo(componentId) {
      logCall("getSelectedComponentInfo", [componentId]);
      if (componentInfos[componentId] !== undefined) {
        return componentInfos[componentId];
      }
      if (options.floatInfo !== undefined) {
        return { id: componentId, floatInfo: options.floatInfo };
      }
      return { id: componentId };
    },
  };

  if (!options.missingSelectComponents) {
    builder.selectComponents = function (infos, clearOthers) {
      logCall("selectComponents", [infos, clearOthers]);
      if (Array.isArray(infos)) {
        selectedComponents = infos.map((info) => ({
          ...info,
          getId:
            typeof info.getId === "function"
              ? info.getId
              : () => info.id ?? info.componentId,
        }));
      }

      if (options.selectComponentsCallsDoSelectedChange) {
        const selectIds = selectedComponents
          .map((info) => (typeof info.getId === "function" ? info.getId() : info.id))
          .filter((id) => typeof id === "string");
        this.doSelectedChange?.(selectIds, []);
      }
    };
  }

  if (!options.missingDoSelectedChange) {
    builder.doSelectedChange = function (selectIds = [], deselectIds = []) {
      logCall("doSelectedChange", [selectIds, deselectIds]);
    };
  }

  const designer = {
    openFileArgs: options.missingPath
      ? { id: fileId, projectName }
      : { path, id: fileId, projectName },

    getBuilder: options.missingBuilder
      ? undefined
      : () => builder,

    getSelectedInfo() {
      logCall("getSelectedInfo", []);
      return options.selectedInfo ?? null;
    },

    getFileInfo: options.missingFileId
      ? undefined
      : function () {
          logCall("getFileInfo", []);
          return { id: fileId };
        },

    metaFileInfo: options.missingFileId ? undefined : { id: fileId },

    notifyStateChange(stateName) {
      logCall("notifyStateChange", [stateName]);
    },

    _callLog: callLog,
    _patchedMethods: patchedMethods,
  };

  return designer;
}

/**
 * Fake BI SuperPage Designer for testing
 */

export function createFakeDesigner(options = {}) {
  const selectedComponents = options.selectedComponents ?? [
    { getId: () => "webview1", id: "webview1" },
  ];

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
      return selectedComponents[0] ?? null;
    },

    getSelectedComponentInfo(componentId) {
      logCall("getSelectedComponentInfo", [componentId]);
      return { id: componentId };
    },

    selectComponents(infos, clearOthers) {
      logCall("selectComponents", [infos, clearOthers]);
    },

    doSelectedChange() {
      logCall("doSelectedChange", []);
    },
  };

  const designer = {
    openFileArgs: options.missingPath
      ? { id: "fid1", projectName: "Test" }
      : { path: "/analyzer/app/Test.app/Page.spg", id: "fid1", projectName: "Test" },

    getBuilder: options.missingBuilder
      ? undefined
      : () => builder,

    getSelectedInfo() {
      logCall("getSelectedInfo", []);
      return null;
    },

    getFileInfo() {
      logCall("getFileInfo", []);
      return { id: "fid1" };
    },

    metaFileInfo: { id: "fid1" },

    notifyStateChange(stateName) {
      logCall("notifyStateChange", [stateName]);
    },

    _callLog: callLog,
    _patchedMethods: patchedMethods,
  };

  return designer;
}

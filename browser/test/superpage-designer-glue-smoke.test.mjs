/**
 * M40.5 SuperPage Designer Glue Smoke Tests
 */

import { describe, it } from "node:test";
import assert from "node:assert";
import { createFakeDesigner } from "./fake-designer.mjs";
import { createFakeHost } from "./fake-host.mjs";
import { installSuperPageDesignerGlue, _extractSourcePath, _extractFileId, _buildSelection } from "../platform-glue/superpage-designer-glue.mjs";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const glueSourcePath = join(__dirname, "../platform-glue/superpage-designer-glue.mjs");

function createPlugin(options = {}) {
  const host = options.host ?? createFakeHost();
  const selections = [];
  return {
    host,
    plugin: {
      onSelectionChanged(selection) {
        selections.push(selection);
        if (options.returnError) {
          return {
            status: "error",
            target: null,
            items: [],
            diagnostics: [{ severity: "error", code: "INVALID_SELECTION", message: "test error" }],
          };
        }
        if (options.shouldThrow) {
          throw new Error("plugin throw");
        }
        return { handled: true };
      },
      _selections: selections,
    },
  };
}

describe("installSuperPageDesignerGlue", () => {
  it("returns { installed: true } on success", () => {
    const designer = createFakeDesigner();
    const { plugin, host } = createPlugin();
    const result = installSuperPageDesignerGlue(designer, {}, plugin, { host });
    assert.deepStrictEqual(result, { installed: true });
    assert.strictEqual(designer.__metadataCheckerGlueInstalled, true);
  });

  it("does not double-patch on repeated install", () => {
    const designer = createFakeDesigner();
    const { plugin, host } = createPlugin();
    installSuperPageDesignerGlue(designer, {}, plugin, { host });
    const result2 = installSuperPageDesignerGlue(designer, {}, plugin, { host });
    assert.strictEqual(result2.alreadyInstalled, true);
  });

  it("returns error when designer lacks getBuilder", () => {
    const designer = createFakeDesigner({ missingBuilder: true });
    const { plugin, host } = createPlugin();
    const result = installSuperPageDesignerGlue(designer, {}, plugin, { host });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "GLUE_INSTALL_FAILED");
    assert.strictEqual(host.getEvents("glue_install_failed").length, 1);
  });

  it("returns error when designer.openFileArgs.path is missing", () => {
    const designer = createFakeDesigner({ missingPath: true });
    const { plugin, host } = createPlugin();
    const result = installSuperPageDesignerGlue(designer, {}, plugin, { host });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "GLUE_INSTALL_FAILED");
    assert.strictEqual(host.getEvents("glue_install_failed").length, 1);
  });

  it("emits file_id_missing through installed glue when file_id cannot be resolved", () => {
    const designer = createFakeDesigner({ missingFileId: true });
    const { plugin, host } = createPlugin();
    const result = installSuperPageDesignerGlue(designer, {}, plugin, { host });
    assert.deepStrictEqual(result, { installed: true });
    const builder = designer.getBuilder();
    builder.doSelectedChange(["webview1"], []);

    assert.strictEqual(host.getEvents("file_id_missing").length, 1);
    assert.strictEqual(plugin._selections.length, 1);
    assert.strictEqual(plugin._selections[0].file_id, "");
  });

  it("returns error when builder lacks both selection APIs", () => {
    const designer = createFakeDesigner({
      missingSelectComponents: true,
      missingDoSelectedChange: true,
    });
    const { plugin, host } = createPlugin();
    const result = installSuperPageDesignerGlue(designer, {}, plugin, { host });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.diagnostics[0].code, "GLUE_INSTALL_FAILED");
    assert.strictEqual(host.getEvents("glue_install_failed").length, 1);
  });

  it("installs and observes selection when only doSelectedChange exists", () => {
    const designer = createFakeDesigner({ missingSelectComponents: true });
    const { plugin, host } = createPlugin();
    const result = installSuperPageDesignerGlue(designer, {}, plugin, { host });
    assert.deepStrictEqual(result, { installed: true });

    const builder = designer.getBuilder();
    builder.doSelectedChange(["webview1"], []);

    assert.strictEqual(plugin._selections.length, 1);
    assert.deepStrictEqual(plugin._selections[0].selected_component_ids, ["webview1"]);
  });
});

describe("path extraction", () => {
  it("converts /analyzer/app/Test.app/Page.spg to app/Test.app/Page.spg", () => {
    const designer = createFakeDesigner();
    const path = _extractSourcePath(designer);
    assert.strictEqual(path, "app/Test.app/Page.spg");
  });

  it("converts arbitrary project absolute paths to project-relative app paths", () => {
    const designer = createFakeDesigner({
      path: "/xiaoshouyi/app/售后.app/Page.spg",
      projectName: "xiaoshouyi",
    });
    const path = _extractSourcePath(designer);
    assert.strictEqual(path, "app/售后.app/Page.spg");
  });

  it("converts absolute paths by stripping the leading project segment when projectName is missing", () => {
    const designer = createFakeDesigner({
      path: "/xiaoshouyi/app/售后.app/Page.spg",
      missingProjectName: true,
    });
    const path = _extractSourcePath(designer);
    assert.strictEqual(path, "app/售后.app/Page.spg");
  });

  it("keeps project-internal relative paths unchanged", () => {
    const designer = createFakeDesigner({
      path: "app/售后.app/Page.spg",
      missingProjectName: true,
    });
    const path = _extractSourcePath(designer);
    assert.strictEqual(path, "app/售后.app/Page.spg");
  });

  it("returns null when path is missing", () => {
    const designer = createFakeDesigner({ missingPath: true });
    const path = _extractSourcePath(designer);
    assert.strictEqual(path, null);
  });
});

describe("file_id extraction", () => {
  it("extracts file_id from openFileArgs.id", () => {
    const designer = createFakeDesigner();
    const id = _extractFileId(designer);
    assert.strictEqual(id, "fid1");
  });

  it("falls back to empty string and emits warning when missing", () => {
    const designer = createFakeDesigner();
    designer.openFileArgs = { path: "/analyzer/app/Test.app/Page.spg" };
    designer.metaFileInfo = undefined;
    designer.getFileInfo = undefined;
    const host = createFakeHost();
    const id = _extractFileId(designer, host);
    assert.strictEqual(id, "");
    assert.strictEqual(host.getEvents("file_id_missing").length, 1);
  });
});

describe("selection building", () => {
  it("builds correct selection from designer state", () => {
    const designer = createFakeDesigner();
    const selection = _buildSelection(designer);
    assert.strictEqual(selection.source_path, "app/Test.app/Page.spg");
    assert.strictEqual(selection.file_id, "fid1");
    assert.deepStrictEqual(selection.selected_component_ids, ["webview1"]);
    assert.strictEqual(selection.active_component_id, "webview1");
    assert.strictEqual(selection.designer_kind, "superpage");
    assert.strictEqual(selection.project_name, "Test");
    assert.ok(typeof selection.timestamp === "number");
  });

  it("derives project_name from absolute remote path before stripping source_path", () => {
    const designer = createFakeDesigner({
      path: "/xiaoshouyi/app/售后.app/Page.spg",
      missingProjectName: true,
    });
    const selection = _buildSelection(designer);
    assert.strictEqual(selection.source_path, "app/售后.app/Page.spg");
    assert.strictEqual(selection.project_name, "xiaoshouyi");
  });

  it("does not infer project_name from already project-internal relative paths", () => {
    const designer = createFakeDesigner({
      path: "app/售后.app/Page.spg",
      missingProjectName: true,
    });
    const selection = _buildSelection(designer);
    assert.strictEqual(selection.source_path, "app/售后.app/Page.spg");
    assert.strictEqual(selection.project_name, "");
  });

  it("handles canvas selection correctly", () => {
    const designer = createFakeDesigner({
      selectedComponents: [{ getId: () => "canvas", id: "canvas" }],
    });
    const selection = _buildSelection(designer);
    assert.deepStrictEqual(selection.selected_component_ids, ["canvas"]);
    assert.strictEqual(selection.active_component_id, "canvas");
  });

  it("does not include raw_text, components, or html", () => {
    const designer = createFakeDesigner();
    const selection = _buildSelection(designer);
    assert.strictEqual("raw_text" in selection, false);
    assert.strictEqual("components" in selection, false);
    assert.strictEqual("html" in selection, false);
  });

  it("uses first selected component when active component is null", () => {
    const designer = createFakeDesigner({
      activeComponentNull: true,
      selectedComponents: [
        { getId: () => "input1", id: "input1" },
        { getId: () => "button1", id: "button1" },
      ],
    });
    const selection = _buildSelection(designer);
    assert.deepStrictEqual(selection.selected_component_ids, ["input1", "button1"]);
    assert.strictEqual(selection.active_component_id, "input1");
  });

  it("keeps floatInfo in selection_infos but strips raw payload and object references", () => {
    const builderRef = { kind: "builder" };
    const componentRef = { kind: "component" };
    const designer = createFakeDesigner({
      selectedComponents: [{ getId: () => "float1", id: "float1" }],
      componentInfos: {
        float1: {
          id: "float1",
          floatInfo: { top: 10, left: 20 },
          raw_text: "large raw text",
          components: [{ id: "nested" }],
          html: "<div>raw</div>",
          builder: builderRef,
          component: componentRef,
        },
      },
    });
    const selection = _buildSelection(designer);

    assert.deepStrictEqual(selection.selection_infos.float1, {
      id: "float1",
      floatInfo: { top: 10, left: 20 },
    });
    assert.strictEqual("raw_text" in selection, false);
    assert.strictEqual("components" in selection, false);
    assert.strictEqual("html" in selection, false);
    assert.strictEqual("builder" in selection.selection_infos.float1, false);
    assert.strictEqual("component" in selection.selection_infos.float1, false);
  });

  it("uses builder selection when designer.getSelectedInfo returns null", () => {
    const designer = createFakeDesigner({ selectedInfo: null });
    const selection = _buildSelection(designer);
    assert.deepStrictEqual(selection.selected_component_ids, ["webview1"]);
    assert.strictEqual(
      designer._callLog.some((entry) => entry.method === "getSelectedComponents"),
      true
    );
  });

  it("returns null when builder is missing", () => {
    const designer = createFakeDesigner({ missingBuilder: true });
    const selection = _buildSelection(designer);
    assert.strictEqual(selection, null);
  });
});

describe("patched builder methods", () => {
  it("calls original selectComponents and triggers plugin.onSelectionChanged", () => {
    const designer = createFakeDesigner();
    const { plugin, host } = createPlugin();
    installSuperPageDesignerGlue(designer, {}, plugin, { host });

    const builder = designer.getBuilder();
    builder.selectComponents([{ id: "webview1" }], true);

    assert.strictEqual(designer._callLog.some((c) => c.method === "selectComponents"), true);
    assert.strictEqual(plugin._selections.length, 1);
    assert.deepStrictEqual(plugin._selections[0].selected_component_ids, ["webview1"]);
  });

  it("does not emit duplicate selection when selectComponents calls doSelectedChange internally", () => {
    const designer = createFakeDesigner({ selectComponentsCallsDoSelectedChange: true });
    const { plugin, host } = createPlugin();
    installSuperPageDesignerGlue(designer, {}, plugin, { host });

    const builder = designer.getBuilder();
    builder.selectComponents([{ id: "webview1" }], true);

    assert.strictEqual(
      designer._callLog.filter((entry) => entry.method === "doSelectedChange").length,
      1
    );
    assert.strictEqual(plugin._selections.length, 1);
    assert.deepStrictEqual(plugin._selections[0].selected_component_ids, ["webview1"]);
  });

  it("calls original doSelectedChange and triggers plugin.onSelectionChanged", () => {
    const designer = createFakeDesigner();
    const { plugin, host } = createPlugin();
    installSuperPageDesignerGlue(designer, {}, plugin, { host });

    const builder = designer.getBuilder();
    builder.doSelectedChange();

    assert.strictEqual(designer._callLog.some((c) => c.method === "doSelectedChange"), true);
    assert.strictEqual(plugin._selections.length, 1);
  });

  it("records glue_selection_failed when plugin returns error envelope", () => {
    const designer = createFakeDesigner();
    const { plugin, host } = createPlugin({ returnError: true });
    installSuperPageDesignerGlue(designer, {}, plugin, { host });

    const builder = designer.getBuilder();
    builder.selectComponents([{ id: "webview1" }]);

    assert.strictEqual(host.getEvents("glue_selection_failed").length, 1);
  });

  it("records glue_selection_failed when plugin throws", () => {
    const designer = createFakeDesigner();
    const { plugin, host } = createPlugin({ shouldThrow: true });
    installSuperPageDesignerGlue(designer, {}, plugin, { host });

    const builder = designer.getBuilder();
    builder.doSelectedChange();

    assert.strictEqual(host.getEvents("glue_selection_failed").length, 1);
  });
});

describe("static dependency check", () => {
  it("glue source does not contain forbidden dependencies", () => {
    const source = readFileSync(glueSourcePath, "utf-8");
    const forbidden = [
      "window",
      "document",
      "navigator",
      "fetch",
      "createElement",
      "appendChild",
      "innerHTML",
    ];
    for (const word of forbidden) {
      assert.strictEqual(
        source.includes(word),
        false,
        `Glue source must not contain "${word}"`
      );
    }
  });

  it("glue source does not import external modules", () => {
    const source = readFileSync(glueSourcePath, "utf-8");
    const importRegex = /import\s+.*?\s+from\s+["'][^"']+["']/g;
    const imports = source.match(importRegex) ?? [];
    assert.strictEqual(imports.length, 0, "Glue should not have external imports");
  });
});

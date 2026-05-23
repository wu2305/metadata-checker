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
});

describe("path extraction", () => {
  it("converts /analyzer/app/Test.app/Page.spg to app/Test.app/Page.spg", () => {
    const designer = createFakeDesigner();
    const path = _extractSourcePath(designer);
    assert.strictEqual(path, "app/Test.app/Page.spg");
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

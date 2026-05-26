/**
 * M43.1 bridge protocol smoke tests
 */

import { describe, it } from "node:test";
import assert from "node:assert";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import vm from "node:vm";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const protocolSourcePath = join(__dirname, "../bridge/metadata-checker-bridge.js");

function loadProtocolModule() {
  const source = readFileSync(protocolSourcePath, "utf-8");
  const context = {
    globalThis: null,
    console: {
      log() {},
      warn() {},
      error() {},
    },
    Date,
    Object,
    Array,
    JSON,
    String,
    Number,
    Boolean,
  };
  context.globalThis = context;

  vm.runInNewContext(source, context, { filename: protocolSourcePath });
  return context.__metadata_checker_bridge_protocol__;
}

function makeDesigner() {
  return {
    type: "superpage",
    openFileArgs: {
      path: "/analyzer/app/M43Smoke.app/M43Smoke.spg",
      id: "fid-123",
      projectName: "analyzer",
    },
    getBuilder() {
      return {
        getSelectedComponents() {
          return [{ getId: () => "comp-1" }];
        },
        getSelectedComponent() {
          return { getId: () => "comp-1" };
        },
        getSelectedComponentInfo(id) {
          if (id === "comp-1") {
            return {
              id: "comp-1",
              floatInfo: {
                left: 0,
                top: 10,
              },
              raw_text: JSON.stringify({ raw: "payload" }),
              components: [{ id: "comp-1", extra: true }],
            };
          }
          return null;
        },
      };
    },
  };
}

describe("metadata-checker bridge protocol", () => {
  it("registers global protocol object", () => {
    const protocol = loadProtocolModule();
    assert.ok(protocol);
    assert.strictEqual(protocol.BRIDGE_PROTOCOL_NAME, "metadata_checker_designer_bridge");
    assert.strictEqual(protocol.BRIDGE_PROTOCOL.version, "m43-protocol-v1");
    assert.strictEqual(protocol.BRIDGE_PROTOCOL_EVENT_READY, "__metadata_checker_designer_ready__");
    assert.strictEqual(protocol.BRIDGE_PROTOCOL.events.ready, "__metadata_checker_designer_ready__");
    assert.strictEqual(
      protocol.BRIDGE_PROTOCOL.events.selection_requested,
      "__metadata_checker_selection_requested__",
    );
    assert.strictEqual(
      protocol.BRIDGE_PROTOCOL.events.selection_response,
      "__metadata_checker_selection_response__",
    );
  });

  it("buildBridgePayload returns protocol page_context selection diagnostics contract", () => {
    const protocol = loadProtocolModule();
    const designer = makeDesigner();
    const payload = protocol.createBridgePayload(designer, { isEditMode: true });

    assert.deepStrictEqual(Object.keys(payload).sort(), [
      "diagnostics",
      "page_context",
      "protocol",
      "selection",
    ]);
    assert.strictEqual(payload.protocol.version, "m43-protocol-v1");
    assert.strictEqual(payload.page_context.project_name, "analyzer");
    assert.strictEqual(payload.page_context.source_path, "app/M43Smoke.app/M43Smoke.spg");
    assert.ok(Array.isArray(payload.selection.selected_component_ids));
    assert.strictEqual(payload.selection.selected_component_ids[0], "comp-1");
    assert.ok(payload.selection.selected_component_infos["comp-1"].id === "comp-1");
  });

  it("returns warning diagnostic when there is no selection", () => {
    const protocol = loadProtocolModule();
    const payload = protocol.createBridgePayload({
      openFileArgs: { path: "/app/only.spg", projectName: "analyzer" },
      getBuilder() {
        return {
          getSelectedComponents() {
            return [];
          },
          getSelectedComponent() {
            return null;
          },
        };
      },
    });

    assert.ok(payload.diagnostics.some((item) => item.code === "BRIDGE_NO_SELECTION"));
    assert.strictEqual(payload.selection.selected_component_ids.length, 0);
  });

  it("selection snapshot does not expose raw metadata", () => {
    const protocol = loadProtocolModule();
    const payload = protocol.createBridgePayload(makeDesigner());

    const selectedInfo = payload.selection.selected_component_infos?.["comp-1"];
    assert.ok(selectedInfo);
    assert.strictEqual(selectedInfo.id, "comp-1");
    assert.ok(!("raw_text" in selectedInfo));
    assert.ok(!("components" in selectedInfo));
    assert.strictEqual(payload.selection.selected_component_infos["comp-1"].float_info.left, 0);
  });
});

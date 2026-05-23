/**
 * M40.6 Remote Metadata Provider Smoke Tests
 */

import { describe, it } from "node:test";
import assert from "node:assert";
import { createFakeHost } from "./fake-host.mjs";
import { createFakeRemoteMetadataProvider } from "../providers/fake-remote-metadata-provider.mjs";
import { createPageRcMetadataProvider } from "../providers/page-rc-metadata-provider.mjs";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const pageRcSourcePath = join(__dirname, "../providers/page-rc-metadata-provider.mjs");

const validFileRef = {
  project_ref: "Test",
  source_path: "app/Test.app/Page.spg",
  file_id: "fid1",
};

function makeFixtures() {
  return new Map([
    [
      "app/Test.app/Page.spg",
      {
        raw_text: '{"components":[]}',
        revision: "3",
        content_type: "super_page",
      },
    ],
  ]);
}

describe("FakeRemoteMetadataProvider", () => {
  it("returns fixture content", async () => {
    const provider = createFakeRemoteMetadataProvider({ fixtures: makeFixtures() });
    const result = await provider.getFileContent(validFileRef);
    assert.strictEqual(result.raw_text, '{"components":[]}');
    assert.strictEqual(result.revision, "3");
    assert.strictEqual(result.content_type, "super_page");
  });

  it("returns fixture info", async () => {
    const provider = createFakeRemoteMetadataProvider({ fixtures: makeFixtures() });
    const result = await provider.getFileInfo(validFileRef);
    assert.strictEqual(result.source_path, "app/Test.app/Page.spg");
    assert.strictEqual(result.revision, "3");
  });

  it("records callLog", async () => {
    const provider = createFakeRemoteMetadataProvider({ fixtures: makeFixtures() });
    await provider.getFileContent(validFileRef);
    await provider.getFileInfo(validFileRef);
    assert.strictEqual(provider.callLog.length, 2);
    assert.strictEqual(provider.callLog[0].method, "getFileContent");
    assert.strictEqual(provider.callLog[1].method, "getFileInfo");
  });

  it("returns error for missing file", async () => {
    const provider = createFakeRemoteMetadataProvider();
    const result = await provider.getFileContent({ source_path: "app/Missing.app/Page.spg" });
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.code, "REMOTE_FETCH_NOT_FOUND");
  });

  it("handles reject gracefully", async () => {
    const provider = createFakeRemoteMetadataProvider({
      getFileContentShouldReject: true,
    });
    try {
      await provider.getFileContent(validFileRef);
      assert.fail("should have rejected");
    } catch (err) {
      assert.strictEqual(err.code, "REMOTE_FETCH_FAILED");
    }
  });

  it("handles throw gracefully", async () => {
    const provider = createFakeRemoteMetadataProvider({
      getFileContentShouldThrow: true,
    });
    try {
      await provider.getFileContent(validFileRef);
      assert.fail("should have thrown");
    } catch (err) {
      assert.strictEqual(err.message, "mock getFileContent throw");
    }
  });

  it("returns empty related files", async () => {
    const provider = createFakeRemoteMetadataProvider();
    const result = await provider.getRelatedFiles(validFileRef);
    assert.deepStrictEqual(result, []);
  });

  it("sync returnType works", () => {
    const provider = createFakeRemoteMetadataProvider({
      fixtures: makeFixtures(),
      returnType: "sync",
    });
    const result = provider.getFileContent(validFileRef);
    assert.strictEqual(result.raw_text, '{"components":[]}');
  });
});

describe("PageRcMetadataProvider", () => {
  it("returns PAGE_RC_UNAVAILABLE when rc is missing", async () => {
    const host = createFakeHost();
    const provider = createPageRcMetadataProvider({ host, rc: undefined, rc1: undefined });
    const result = await provider.getFileInfo(validFileRef);
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.code, "PAGE_RC_UNAVAILABLE");
    assert.strictEqual(host.getEvents("metadata_fetch_failed").length, 1);
  });

  it("calls mock rc and returns info", async () => {
    const rc = async (method, fileId, sourcePath) => {
      return { revision: "5", updated_at: "2024-01-01" };
    };
    const provider = createPageRcMetadataProvider({ rc });
    const result = await provider.getFileInfo(validFileRef);
    assert.strictEqual(result.revision, "5");
    assert.strictEqual(result.content_type, "super_page");
    assert.strictEqual(result.updated_at, "2024-01-01");
  });

  it("calls mock rc and returns content", async () => {
    const rc = async (method, fileId, sourcePath) => {
      return '{"components":[]}';
    };
    const provider = createPageRcMetadataProvider({ rc });
    const result = await provider.getFileContent(validFileRef);
    assert.strictEqual(result.raw_text, '{"components":[]}');
    assert.strictEqual(result.content_type, "super_page");
  });

  it("maps invalid rc response to REMOTE_RESPONSE_INVALID", async () => {
    const rc = async () => null;
    const host = createFakeHost();
    const provider = createPageRcMetadataProvider({ rc, host });
    const result = await provider.getFileContent(validFileRef);
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.code, "REMOTE_RESPONSE_INVALID");
    assert.strictEqual(host.getEvents("metadata_fetch_failed").length, 1);
  });

  it("maps rc throw to REMOTE_FETCH_FAILED", async () => {
    const rc = async () => {
      throw new Error("rc error");
    };
    const host = createFakeHost();
    const provider = createPageRcMetadataProvider({ rc, host });
    const result = await provider.getFileInfo(validFileRef);
    assert.strictEqual(result.status, "error");
    assert.strictEqual(result.code, "REMOTE_FETCH_FAILED");
    assert.strictEqual(host.getEvents("metadata_fetch_failed").length, 1);
  });

  it("getRelatedFiles returns empty array", async () => {
    const provider = createPageRcMetadataProvider({});
    const result = await provider.getRelatedFiles(validFileRef);
    assert.deepStrictEqual(result, []);
  });

  it("source_path remains logical path", async () => {
    const rc = async (method, fileId, sourcePath) => {
      return '{"components":[]}';
    };
    const provider = createPageRcMetadataProvider({ rc });
    const result = await provider.getFileContent(validFileRef);
    assert.strictEqual(result.source_path, "app/Test.app/Page.spg");
    assert.strictEqual(result.source_path.includes("http"), false);
    assert.strictEqual(result.source_path.startsWith("/"), false);
  });
});

describe("static dependency check", () => {
  it("page-rc source does not import external modules", () => {
    const source = readFileSync(pageRcSourcePath, "utf-8");
    const importRegex = /import\s+.*?\s+from\s+["'][^"']+["']/g;
    const imports = source.match(importRegex) ?? [];
    assert.strictEqual(imports.length, 0, "page-rc provider should not have external imports");
  });

  it("page-rc source does not contain forbidden globals", () => {
    const source = readFileSync(pageRcSourcePath, "utf-8");
    const forbidden = ["document", "navigator", "createElement", "appendChild", "innerHTML"];
    for (const word of forbidden) {
      assert.strictEqual(
        source.includes(word),
        false,
        `page-rc source must not contain "${word}"`
      );
    }
    // fetch must not appear as a standalone call/global access
    const fetchCallRegex = /\bfetch\s*\(/;
    assert.strictEqual(
      fetchCallRegex.test(source),
      false,
      "page-rc source must not contain fetch() calls"
    );
  });
});

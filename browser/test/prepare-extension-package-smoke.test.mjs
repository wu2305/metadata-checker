import assert from "node:assert/strict";
import { access, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import test from "node:test";

import {
  parseArgs,
  prepareExtensionPackage,
} from "../tools/prepare-extension-package.mjs";
import {
  parseArgs as parseSafariArgs,
  prepareSafariExtensionPackage,
} from "../tools/prepare-safari-extension-package.mjs";

const ABS_REPO_ROOT = "/Users/wuhaocheng/Documents/repos/metadata-checker";
const SENSITIVE_MARKERS = ["test-account", "password=", "secret-password"];

async function createChromiumFixture() {
  const fixtureRoot = await mkdtemp(join(tmpdir(), "metadata-checker-chromium-fixture-"));
  const extensionCoreDir = join(fixtureRoot, "extension-core");
  const extensionChromiumDir = join(fixtureRoot, "extension-chromium");
  await mkdir(extensionCoreDir, { recursive: true });
  await mkdir(extensionChromiumDir, { recursive: true });
  await writeFile(join(extensionCoreDir, "core.js"), "window.__metadataCheckerCore = true;\n");
  await writeFile(
    join(extensionChromiumDir, "manifest.json"),
    JSON.stringify(
      {
        manifest_version: 3,
        name: "metadata-checker",
        version: "0.0.1",
        host_permissions: ["https://old-host.example/*"],
        content_scripts: [
          { js: ["content.js"], matches: ["https://old-host.example/*"] },
          { js: ["content2.js"], matches: ["https://old-host.example/page/*"] },
        ],
        web_accessible_resources: [
          {
            resources: ["extension-core/page-script.js"],
            matches: ["https://old-host.example/*"],
          },
        ],
      },
      null,
      2,
    ),
  );
  await writeFile(join(extensionChromiumDir, "content.js"), "console.log('content');\n");
  return { fixtureRoot, extensionCoreDir, extensionChromiumDir };
}

async function createSafariFixture(withTemplate = true) {
  const fixtureRoot = await mkdtemp(join(tmpdir(), "metadata-checker-safari-fixture-"));
  const extensionCoreDir = join(fixtureRoot, "extension-core");
  const extensionSafariDir = join(fixtureRoot, "extension-safari");
  await mkdir(extensionCoreDir, { recursive: true });
  await writeFile(join(extensionCoreDir, "core.js"), "window.__metadataCheckerCore = true;\n");
  if (withTemplate) {
    await mkdir(extensionSafariDir, { recursive: true });
    await writeFile(
      join(extensionSafariDir, "manifest.template.json"),
      JSON.stringify(
        {
          manifest_version: 3,
          name: "metadata-checker-safari",
          version: "0.0.1",
          host_permissions: ["https://template-placeholder/*"],
          content_scripts: [{ js: ["content.js"], matches: ["https://template-placeholder/*"] }],
        },
        null,
        2,
      ),
    );
    await writeFile(
      join(extensionSafariDir, "safari-notes.md"),
      "# Safari template notes\n",
      "utf8",
    );
  }
  return { fixtureRoot, extensionCoreDir, extensionSafariDir };
}

function assertNoSecretOrPathLeak(value) {
  const serialized = typeof value === "string" ? value : JSON.stringify(value);
  assert.equal(serialized.includes(ABS_REPO_ROOT), false, "repo absolute path leaked");
  for (const marker of SENSITIVE_MARKERS) {
    assert.equal(serialized.includes(marker), false, `sensitive marker leaked: ${marker}`);
  }
}

async function listTextFiles(root) {
  const items = await readdir(root, { withFileTypes: true });
  const result = [];
  for (const item of items) {
    const full = join(root, item.name);
    if (item.isDirectory()) {
      const child = await listTextFiles(full);
      result.push(...child.map((value) => join(item.name, value)));
      continue;
    }
    if (item.name.endsWith(".json") || item.name.endsWith(".js") || item.name.endsWith(".md")) {
      result.push(item.name);
    }
  }
  return result;
}

test("parseArgs supports required extension package options", () => {
  const args = parseArgs([
    "--out-dir",
    "/tmp/out",
    "--version",
    "1.2.3",
    "--host-match",
    "https://host.example/*",
    "--wasm-bindgen-js",
    "/tmp/meta.js",
    "--wasm-file",
    "/tmp/meta.wasm",
  ]);
  assert.equal(args.out_dir, "/tmp/out");
  assert.equal(args.version, "1.2.3");
  assert.equal(args.host_match, "https://host.example/*");
  assert.equal(args.wasm_bindgen_js, "/tmp/meta.js");
  assert.equal(args.wasm_file, "/tmp/meta.wasm");
});

test("parseArgs supports required safari package options", () => {
  const args = parseSafariArgs([
    "--out-dir",
    "/tmp/safari-out",
    "--version",
    "1.2.3",
    "--host-match",
    "https://host.example/*",
  ]);
  assert.equal(args.out_dir, "/tmp/safari-out");
  assert.equal(args.version, "1.2.3");
  assert.equal(args.host_match, "https://host.example/*");
});

test("prepareExtensionPackage builds Chromium extension and rewrites manifest", async () => {
  const fixture = await createChromiumFixture();
  const outRoot = await mkdtemp(join(tmpdir(), "metadata-checker-ext-out-"));
  const wasmInput = join(outRoot, "metadata_checker_bg.wasm");
  const glueInput = join(outRoot, "metadata_checker.js");
  const outDir = join(outRoot, "package");
  try {
    await writeFile(glueInput, "const wasm = {};\nlet wasm_bindgen = function(){};\n", "utf8");
    await writeFile(wasmInput, Buffer.from([0x00, 0x61, 0x73, 0x6d, 0x02, 0x00, 0x00, 0x00]));

    const result = await prepareExtensionPackage({
      outDir,
      version: "3.1.0",
      hostMatch: "https://host.test/*,https://host2.test/*",
      wasmBindgenJs: glueInput,
      wasmFile: wasmInput,
      extensionCoreDir: fixture.extensionCoreDir,
      extensionChromiumDir: fixture.extensionChromiumDir,
      clean: true,
    });

    const manifest = JSON.parse(await readFile(join(outDir, "manifest.json"), "utf8"));
    assert.equal(manifest.version, "3.1.0");
    assert.deepStrictEqual(manifest.host_permissions, [
      "https://host.test/*",
      "https://host2.test/*",
    ]);
    assert.deepStrictEqual(manifest.content_scripts[0].matches, [
      "https://host.test/*",
      "https://host2.test/*",
    ]);
    assert.deepStrictEqual(manifest.content_scripts[1].matches, [
      "https://host.test/*",
      "https://host2.test/*",
    ]);
    assert.deepStrictEqual(manifest.web_accessible_resources[0].matches, [
      "https://host.test/*",
      "https://host2.test/*",
    ]);

    const copiedWasm = await readFile(join(outDir, "metadata_checker_bg.wasm"));
    assert.deepStrictEqual(
      copiedWasm,
      Buffer.from([0x00, 0x61, 0x73, 0x6d, 0x02, 0x00, 0x00, 0x00]),
    );

    const copiedGlue = await readFile(join(outDir, "metadata_checker.js"), "utf8");
    assert.match(copiedGlue, /wasm_bindgen/);

    const accessCore = await access(join(outDir, "extension-core", "core.js")).then(() => true, () => false);
    assert.equal(accessCore, true);
    const accessContent = await access(join(outDir, "content.js")).then(() => true, () => false);
    assert.equal(accessContent, true);

    assert.equal(result.zip.status, "skipped");
    assert.equal(result.diagnostics.some((item) => item.code === "ZIP_PACKAGING_NOT_SUPPORTED"), true);
    assertNoSecretOrPathLeak(result);

    const textFiles = await listTextFiles(outDir);
    for (const file of textFiles) {
      const content = await readFile(join(outDir, file), "utf8");
      assertNoSecretOrPathLeak(content);
    }
  } finally {
    await rm(fixture.fixtureRoot, { recursive: true, force: true });
    await rm(outRoot, { recursive: true, force: true });
  }
});

test("Chromium manifest loads shared panel host before content script", async () => {
  const manifest = JSON.parse(
    await readFile(new URL("../extension-chromium/manifest.json", import.meta.url), "utf8"),
  );
  const scripts = manifest.content_scripts?.[0]?.js || [];
  const panelHostIndex = scripts.indexOf("extension-core/panel-host.js");
  const contentScriptIndex = scripts.indexOf("content-script.js");

  assert.notEqual(panelHostIndex, -1);
  assert.notEqual(contentScriptIndex, -1);
  assert.ok(panelHostIndex < contentScriptIndex);
});

test("prepareExtensionPackage writes .wasm as binary", async () => {
  const fixture = await createChromiumFixture();
  const outRoot = await mkdtemp(join(tmpdir(), "metadata-checker-ext-out-bin-"));
  const outDir = join(outRoot, "package");
  const wasmInput = join(outRoot, "metadata_checker_bg.wasm");
  try {
    const bytes = Buffer.from([0xff, 0xfe, 0xfd, 0x01, 0x00, 0x31, 0x00, 0x00]);
    await writeFile(wasmInput, bytes);
    await prepareExtensionPackage({
      outDir,
      wasmFile: wasmInput,
      extensionCoreDir: fixture.extensionCoreDir,
      extensionChromiumDir: fixture.extensionChromiumDir,
      clean: true,
    });
    const copiedWasm = await readFile(join(outDir, "metadata_checker_bg.wasm"));
    assert.deepStrictEqual(copiedWasm, bytes);
  } finally {
    await rm(fixture.fixtureRoot, { recursive: true, force: true });
    await rm(outRoot, { recursive: true, force: true });
  }
});

test("prepareSafariExtensionPackage builds staging package with template manifest", async () => {
  const fixture = await createSafariFixture(true);
  const outRoot = await mkdtemp(join(tmpdir(), "metadata-checker-safari-out-"));
  const outDir = join(outRoot, "package");
  try {
    await prepareSafariExtensionPackage({
      outDir,
      version: "4.0.0",
      hostMatch: "https://safari.example/*",
      extensionCoreDir: fixture.extensionCoreDir,
      extensionSafariDir: fixture.extensionSafariDir,
      extensionChromiumDir: fixture.extensionChromiumDir ?? fixture.extensionSafariDir,
      clean: true,
    });

    const manifest = JSON.parse(await readFile(join(outDir, "manifest.json"), "utf8"));
    assert.equal(manifest.version, "4.0.0");
    assert.deepStrictEqual(manifest.host_permissions, ["https://safari.example/*"]);
    assert.equal(JSON.stringify(manifest).includes("{{HOST_MATCHES}}"), false);
    const notes = await readFile(join(outDir, "safari-notes.md"), "utf8");
    assert.equal(notes.includes("# Safari template notes"), true);
    const accessCore = await access(join(outDir, "extension-core", "core.js")).then(() => true, () => false);
    assert.equal(accessCore, true);
    assertNoSecretOrPathLeak(await readFile(join(outDir, "manifest.json"), "utf8"));
  } finally {
    await rm(fixture.fixtureRoot, { recursive: true, force: true });
    await rm(outRoot, { recursive: true, force: true });
  }
});

test("prepareSafariExtensionPackage creates safari-notes.md when safari template is missing", async () => {
  const fixture = await createSafariFixture(false);
  const outRoot = await mkdtemp(join(tmpdir(), "metadata-checker-safari-missing-"));
  const outDir = join(outRoot, "package");
  try {
    const missingDir = join(fixture.fixtureRoot, "extension-safari-missing");
    const result = await prepareSafariExtensionPackage({
      outDir,
      extensionCoreDir: fixture.extensionCoreDir,
      extensionSafariDir: missingDir,
      clean: true,
    });

    assert.equal(result.diagnostics.some((item) => item.code === "SAFARI_TEMPLATE_MISSING"), true);
    const notes = await readFile(join(outDir, "safari-notes.md"), "utf8");
    assert.equal(notes.includes("Safari staging package notes"), true);
    assert.equal(result.manifest_path, null);
    assert.equal(result.files.includes("safari-notes.md"), true);
    assertNoSecretOrPathLeak(notes);
  } finally {
    await rm(fixture.fixtureRoot, { recursive: true, force: true });
    await rm(outRoot, { recursive: true, force: true });
  }
});

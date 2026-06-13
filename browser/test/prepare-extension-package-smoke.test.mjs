import assert from "node:assert/strict";
import { access, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import test from "node:test";

import {
  parseArgs,
  prepareExtensionPackage,
} from "../tools/prepare-extension-package.mjs";
import { buildSpikeVendorBundles } from "../tools/build-spike-vendor-bundle.mjs";
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
  await writeFile(
    join(extensionChromiumDir, "offscreen.html"),
    "<!doctype html><script type=\"module\" src=\"offscreen-runtime.js\"></script>\n",
  );
  await writeFile(
    join(extensionChromiumDir, "offscreen-runtime.js"),
    "globalThis.__metadataCheckerOffscreenRuntime = true;\n",
  );
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
  assert.equal(args.spike_renderer_artifacts, undefined);
  assert.equal(args.spike_vendor_artifacts, undefined);
});

test("parseArgs supports spike renderer artifact folder option", () => {
  const args = parseArgs([
    "--spike-renderer-artifacts",
    "/tmp/spike-artifacts",
  ]);

  assert.equal(args.spike_renderer_artifacts, "/tmp/spike-artifacts");
});

test("parseArgs supports spike vendor artifact folder option", () => {
  const args = parseArgs([
    "--spike-vendor-artifacts",
    "/tmp/spike-vendor",
  ]);

  assert.equal(args.spike_vendor_artifacts, "/tmp/spike-vendor");
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
    const copiedOffscreenHtml = await readFile(join(outDir, "offscreen.html"), "utf8");
    assert.match(copiedOffscreenHtml, /offscreen-runtime\.js/);
    const copiedOffscreenRuntime = await readFile(join(outDir, "offscreen-runtime.js"), "utf8");
    assert.match(copiedOffscreenRuntime, /__metadataCheckerOffscreenRuntime/);

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

test("prepareExtensionPackage keeps acceptance marker files stable", async () => {
  const fixture = await createChromiumFixture();
  const outRoot = await mkdtemp(join(tmpdir(), "metadata-checker-ext-out-acceptance-marker-"));
  const outDir = join(outRoot, "package");
  const acceptanceMarkerFile = join(
    fixture.extensionChromiumDir,
    "m47-real-bi-cdp-acceptance.marker.js",
  );
  const markerContent =
    "export const acceptanceManifest = { added: 1, modified: 0, deleted: 0, unchanged: 8, contentQueueCount: 2 };\n";
  try {
    await writeFile(acceptanceMarkerFile, markerContent, "utf8");
    const result = await prepareExtensionPackage({
      outDir,
      extensionCoreDir: fixture.extensionCoreDir,
      extensionChromiumDir: fixture.extensionChromiumDir,
      clean: true,
    });

    const copiedMarker = await readFile(
      join(outDir, "m47-real-bi-cdp-acceptance.marker.js"),
      "utf8",
    );
    assert.match(copiedMarker, /acceptanceManifest/);
    assert.equal(result.files.includes("m47-real-bi-cdp-acceptance.marker.js"), true);
    assertNoSecretOrPathLeak(result);
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

test("Chromium manifest allows WASM compilation in extension pages", async () => {
  const manifest = JSON.parse(
    await readFile(new URL("../extension-chromium/manifest.json", import.meta.url), "utf8"),
  );

  assert.ok(manifest.permissions?.includes("offscreen"));
  assert.match(
    manifest.content_security_policy?.extension_pages || "",
    /'wasm-unsafe-eval'/,
  );
});

test("Chromium extension includes offscreen runtime host files", async () => {
  const html = await readFile(new URL("../extension-chromium/offscreen.html", import.meta.url), "utf8");
  assert.match(html, /offscreen-runtime\.js/);
  await access(new URL("../extension-chromium/offscreen-runtime.js", import.meta.url));
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

test("prepareExtensionPackage can package spike renderer artifacts", async () => {
  const fixture = await createChromiumFixture();
  const outRoot = await mkdtemp(join(tmpdir(), "metadata-checker-ext-out-spike-"));
  const outDir = join(outRoot, "package");
  const artifactRoot = await mkdtemp(join(tmpdir(), "metadata-checker-spike-artifacts-"));
  const artifactDir = join(artifactRoot, "pixi-force-assets");
  const vendorPixi = join(artifactDir, "pixi.min.js");
  const vendorForce = join(artifactDir, "d3-force-3d.js");
  try {
    await mkdir(artifactDir, { recursive: true });
    await writeFile(vendorPixi, "window.PIXI=\"ok\";\n");
    await writeFile(vendorForce, "window.d3Force3d=\"ok\";\n");

    const result = await prepareExtensionPackage({
      outDir,
      extensionCoreDir: fixture.extensionCoreDir,
      extensionChromiumDir: fixture.extensionChromiumDir,
      spikeRendererArtifacts: artifactDir,
      clean: true,
    });

    const copiedPixi = await readFile(join(outDir, "spike-renderer", "pixi.min.js"), "utf8");
    const copiedForce = await readFile(join(outDir, "spike-renderer", "d3-force-3d.js"), "utf8");
    const manifest = JSON.parse(await readFile(join(outDir, "manifest.json"), "utf8"));
    assert.equal(copiedPixi, "window.PIXI=\"ok\";\n");
    assert.equal(copiedForce, "window.d3Force3d=\"ok\";\n");
    assert.ok(
      manifest.web_accessible_resources[0].resources.includes("spike-renderer/*.js"),
    );
    assert.equal(result.spike_renderer_artifacts.length > 0, true);
    assert.equal(
      result.spike_renderer_artifacts.some((item) => item.endsWith("spike-renderer")),
      true,
    );
  } finally {
    await rm(fixture.fixtureRoot, { recursive: true, force: true });
    await rm(outRoot, { recursive: true, force: true });
    await rm(artifactRoot, { recursive: true, force: true });
  }
});

test("prepareExtensionPackage can package spike vendor ESM artifacts", async () => {
  const fixture = await createChromiumFixture();
  const outRoot = await mkdtemp(join(tmpdir(), "metadata-checker-ext-out-vendor-"));
  const outDir = join(outRoot, "package");
  const artifactRoot = await mkdtemp(join(tmpdir(), "metadata-checker-spike-vendor-"));
  const vendorPixi = join(artifactRoot, "pixi-bundle.mjs");
  const vendorForce = join(artifactRoot, "d3-force-3d-bundle.mjs");
  try {
    await writeFile(vendorPixi, "export const PIXI = {};\n");
    await writeFile(vendorForce, "export const D3Force3D = {};\n");

    const result = await prepareExtensionPackage({
      outDir,
      extensionCoreDir: fixture.extensionCoreDir,
      extensionChromiumDir: fixture.extensionChromiumDir,
      spikeVendorArtifacts: artifactRoot,
      clean: true,
    });
    const copiedPixi = await readFile(join(outDir, "spike-vendor", "pixi-bundle.mjs"), "utf8");
    const copiedForce = await readFile(join(outDir, "spike-vendor", "d3-force-3d-bundle.mjs"), "utf8");
    const manifest = JSON.parse(await readFile(join(outDir, "manifest.json"), "utf8"));

    assert.equal(copiedPixi, "export const PIXI = {};\n");
    assert.equal(copiedForce, "export const D3Force3D = {};\n");
    assert.ok(
      manifest.web_accessible_resources[0].resources.includes("spike-vendor/*.mjs"),
    );
    assert.ok(
      manifest.web_accessible_resources[0].resources.includes("spike-vendor/*.js"),
    );
    assert.equal(result.spike_vendor_artifacts.length > 0, true);
    assert.equal(
      result.spike_vendor_artifacts.some((item) => item.endsWith("spike-vendor")),
      true,
    );

    await writeFile(vendorPixi, "export const PIXI2 = {};\n");
    await writeFile(vendorForce, "export const D3Force3D_V2 = {};\n");
    const repackageResult = await prepareExtensionPackage({
      outDir,
      extensionCoreDir: fixture.extensionCoreDir,
      extensionChromiumDir: fixture.extensionChromiumDir,
      spikeVendorArtifacts: artifactRoot,
      clean: true,
    });

    const copiedPixiV2 = await readFile(join(outDir, "spike-vendor", "pixi-bundle.mjs"), "utf8");
    const copiedForceV2 = await readFile(join(outDir, "spike-vendor", "d3-force-3d-bundle.mjs"), "utf8");
    assert.equal(copiedPixiV2, "export const PIXI2 = {};\n");
    assert.equal(copiedForceV2, "export const D3Force3D_V2 = {};\n");
    assert.equal(
      repackageResult.spike_vendor_artifacts.some((item) => item.endsWith("spike-vendor")),
      true,
    );
  } finally {
    await rm(fixture.fixtureRoot, { recursive: true, force: true });
    await rm(outRoot, { recursive: true, force: true });
    await rm(artifactRoot, { recursive: true, force: true });
  }
});

test("buildSpikeVendorBundles can rerun and overwrite outputs", async () => {
  const outRoot = await mkdtemp(join(tmpdir(), "metadata-checker-spike-vendor-build-"));
  const outDir = join(outRoot, "spike-vendor");
  const pixiBundlePath = join(outDir, "pixi-bundle.mjs");
  const forceBundlePath = join(outDir, "d3-force-3d-bundle.mjs");
  try {
    const first = await buildSpikeVendorBundles({ outDir });
    const firstBundle = await readFile(pixiBundlePath, "utf8");
    assert.equal(first.files.includes("pixi-bundle.mjs"), true);
    assert.equal(first.outDir, outDir);

    await writeFile(pixiBundlePath, "export const MUTATED = true;");
    await writeFile(forceBundlePath, "export const MUTATED_FORCE = true;");

    const second = await buildSpikeVendorBundles({ outDir });
    assert.equal(second.outDir, outDir);
    assert.equal(second.files.includes("pixi-bundle.mjs"), true);
    assert.equal(second.files.includes("d3-force-3d-bundle.mjs"), true);

    const secondPixi = await readFile(pixiBundlePath, "utf8");
    const secondForce = await readFile(forceBundlePath, "utf8");
    assert.equal(secondPixi.includes("MUTATED"), false);
    assert.equal(secondForce.includes("MUTATED_FORCE"), false);
  } finally {
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

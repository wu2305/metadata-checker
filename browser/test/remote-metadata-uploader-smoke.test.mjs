import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  makeLoginBody,
  metadataInfoUrl,
  normalizeRemotePath,
  parseArgs,
  uploadMetadataFile,
} from '../tools/remote-metadata-uploader.mjs';

function jsonResponse(status, body, headers = {}) {
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: {
      get(name) {
        return headers[name.toLowerCase()] ?? null;
      },
      getSetCookie() {
        const value = headers['set-cookie'];
        return value ? [value] : [];
      },
    },
    async text() {
      return JSON.stringify(body);
    },
  };
}

async function makeTempFile(content) {
  const dir = await mkdtemp(join(tmpdir(), 'metadata-checker-upload-'));
  const file = join(dir, 'custom.js');
  await writeFile(file, content, 'utf8');
  return file;
}

test('metadataInfoUrl preserves slash-separated logical path', () => {
  assert.equal(
    metadataInfoUrl('/analyzer/public/hooks/custom.js'),
    '/api/meta/services/getFileInfo/analyzer/public/hooks/custom.js',
  );
});

test('normalizeRemotePath rejects unsafe file path shapes', () => {
  assert.throws(() => normalizeRemotePath('analyzer/public/hooks/custom.js'), /must start/);
  assert.throws(() => normalizeRemotePath('/analyzer/public/hooks/'), /file path/);
});

test('makeLoginBody wraps credentials in cipherPassport', () => {
  const body = makeLoginBody({
    username: 'u',
    password: 'p',
    userDirectory: 'sys',
  });
  const decoded = JSON.parse(Buffer.from(body.cipherPassport, 'base64').toString('utf8'));
  assert.deepEqual(decoded, {
    user: 'u',
    password: 'p',
    remember: false,
    userDirectory: 'sys',
  });
});

test('parseArgs supports env-backed CLI shape and dry-run flag', () => {
  assert.deepEqual(parseArgs([
    '--base-url',
    'https://example.test',
    '--file',
    '/tmp/custom.js',
    '--remote-path',
    '/analyzer/public/hooks/custom.js',
    '--dry-run',
  ]), {
    base_url: 'https://example.test',
    file: '/tmp/custom.js',
    remote_path: '/analyzer/public/hooks/custom.js',
    dryRun: true,
  });
});

test('uploadMetadataFile recursively creates missing folders then creates file', async () => {
  const content = 'define(["exports"], function(exports) { exports.CustomJS = {}; });';
  const localFile = await makeTempFile(content);
  const calls = [];
  const existing = new Map([
    ['/analyzer', { name: 'analyzer', isFolder: true, parentDir: '/', revision: 0 }],
  ]);

  const fetchImpl = async (url, init = {}) => {
    const parsed = new URL(url);
    calls.push({ path: parsed.pathname, method: init.method ?? 'GET', body: init.body && JSON.parse(init.body), cookie: init.headers?.Cookie });

    if (parsed.pathname === '/api/auth/signin') {
      return jsonResponse(200, { result: true }, { 'set-cookie': 'JSESSIONID=abc; Path=/; HttpOnly' });
    }

    if (parsed.pathname === '/api/meta/file/createFile') {
      const body = JSON.parse(init.body);
      const path = `${body.parentDir === '/' ? '' : body.parentDir}/${body.name}`;
      existing.set(path, {
        id: `id-${body.name}`,
        name: body.name,
        type: body.type ?? 'fold',
        parentDir: body.parentDir,
        isFolder: body.isFolder,
        revision: 0,
        content: body.content,
      });
      return jsonResponse(200, { created: [existing.get(path)], modified: [], deleted: [] });
    }

    if (parsed.pathname.startsWith('/api/meta/services/getFileInfo/')) {
      const logicalPath = `/${decodeURIComponent(parsed.pathname.slice('/api/meta/services/getFileInfo/'.length))}`;
      const item = existing.get(logicalPath);
      return item ? jsonResponse(200, item) : jsonResponse(404, { errorCode: 'notfound' });
    }

    throw new Error(`unexpected request: ${parsed.pathname}`);
  };

  const result = await uploadMetadataFile({
    baseUrl: 'https://example.test',
    username: 'user',
    password: 'pass',
    localFile,
    remotePath: '/analyzer/public/hooks/custom.js',
    fetchImpl,
  });

  assert.equal(result.status, 'ok');
  assert.equal(result.action, 'created');
  assert.equal(existing.get('/analyzer/public/hooks/custom.js').content, content);
  assert.ok(calls.some(call => call.path === '/api/meta/file/createFile' && call.body.name === 'public'));
  assert.ok(calls.some(call => call.path === '/api/meta/file/createFile' && call.body.name === 'hooks'));
  assert.ok(calls.every(call => call.path === '/api/auth/signin' || call.cookie === 'JSESSIONID=abc'));
});

test('uploadMetadataFile modifies existing remote file and verifies content', async () => {
  const content = 'define(["exports"], function(exports) { exports.CustomJS = { "*": {} }; });';
  const localFile = await makeTempFile(content);
  const existing = {
    id: 'custom-id',
    name: 'custom.js',
    type: 'js',
    parentDir: '/analyzer/public/hooks',
    isFolder: false,
    revision: 3,
    content: 'old',
  };
  const calls = [];

  const fetchImpl = async (url, init = {}) => {
    const parsed = new URL(url);
    calls.push({ path: parsed.pathname, method: init.method ?? 'GET', body: init.body && JSON.parse(init.body) });

    if (parsed.pathname === '/api/auth/signin') {
      return jsonResponse(200, { result: true }, { 'set-cookie': 'JSESSIONID=abc; Path=/; HttpOnly' });
    }
    if (parsed.pathname === '/api/meta/file/modifyFile') {
      const body = JSON.parse(init.body);
      assert.equal(body.revision, 3);
      existing.content = body.content;
      existing.revision += 1;
      return jsonResponse(200, { created: [], modified: [existing], deleted: [] });
    }
    if (parsed.pathname.startsWith('/api/meta/services/getFileInfo/')) {
      const logicalPath = `/${decodeURIComponent(parsed.pathname.slice('/api/meta/services/getFileInfo/'.length))}`;
      if (logicalPath === '/analyzer' || logicalPath === '/analyzer/public' || logicalPath === '/analyzer/public/hooks') {
        return jsonResponse(200, { name: logicalPath.split('/').at(-1), isFolder: true, parentDir: '/' });
      }
      if (logicalPath === '/analyzer/public/hooks/custom.js') {
        return jsonResponse(200, existing);
      }
      return jsonResponse(404, { errorCode: 'notfound' });
    }
    throw new Error(`unexpected request: ${parsed.pathname}`);
  };

  const result = await uploadMetadataFile({
    baseUrl: 'https://example.test',
    username: 'user',
    password: 'pass',
    localFile,
    remotePath: '/analyzer/public/hooks/custom.js',
    fetchImpl,
  });

  assert.equal(result.action, 'modified');
  assert.equal(result.file.revision, 4);
  assert.equal(existing.content, content);
  assert.ok(calls.some(call => call.path === '/api/meta/file/modifyFile'));
});

test('uploadMetadataFile can login with prebuilt cipherPassport body', async () => {
  const content = 'define(["exports"], function(exports) { exports.CustomJS = {}; });';
  const localFile = await makeTempFile(content);
  let loginBody;

  const fetchImpl = async (url, init = {}) => {
    const parsed = new URL(url);
    if (parsed.pathname === '/api/auth/signin') {
      loginBody = JSON.parse(init.body);
      return jsonResponse(200, { result: true }, { 'set-cookie': 'JSESSIONID=abc; Path=/; HttpOnly' });
    }
    if (parsed.pathname === '/api/meta/file/createFile') {
      const body = JSON.parse(init.body);
      return jsonResponse(200, {
        created: [{
          id: `id-${body.name}`,
          name: body.name,
          type: body.type ?? 'fold',
          parentDir: body.parentDir,
          isFolder: body.isFolder,
          revision: 0,
          content: body.content,
        }],
        modified: [],
        deleted: [],
      });
    }
    if (parsed.pathname === '/api/meta/file/modifyFile') {
      const body = JSON.parse(init.body);
      return jsonResponse(200, {
        created: [],
        modified: [{
          id: 'custom-id',
          name: 'custom.js',
          type: 'js',
          parentDir: '/analyzer/public/hooks',
          isFolder: false,
          revision: 1,
          content: body.content,
        }],
        deleted: [],
      });
    }
    if (parsed.pathname.startsWith('/api/meta/services/getFileInfo/')) {
      const logicalPath = `/${decodeURIComponent(parsed.pathname.slice('/api/meta/services/getFileInfo/'.length))}`;
      if (logicalPath === '/analyzer/public/hooks/custom.js') {
        return jsonResponse(200, {
          id: 'custom-id',
          name: 'custom.js',
          type: 'js',
          parentDir: '/analyzer/public/hooks',
          isFolder: false,
          revision: 0,
          content,
        });
      }
      if (logicalPath === '/analyzer' || logicalPath === '/analyzer/public' || logicalPath === '/analyzer/public/hooks') {
        return jsonResponse(200, { name: logicalPath.split('/').at(-1), isFolder: true, parentDir: '/' });
      }
      return jsonResponse(404, { errorCode: 'notfound' });
    }
    throw new Error(`unexpected request: ${parsed.pathname}`);
  };

  await uploadMetadataFile({
    baseUrl: 'https://example.test',
    loginBody: { cipherPassport: 'prebuilt' },
    localFile,
    remotePath: '/analyzer/public/hooks/custom.js',
    fetchImpl,
  });

  assert.deepEqual(loginBody, { cipherPassport: 'prebuilt' });
});

test('uploadMetadataFile refuses to create missing project root implicitly', async () => {
  const localFile = await makeTempFile('define(["exports"], function(exports) {});');
  const fetchImpl = async (url) => {
    const parsed = new URL(url);
    if (parsed.pathname === '/api/auth/signin') {
      return jsonResponse(200, { result: true }, { 'set-cookie': 'JSESSIONID=abc; Path=/; HttpOnly' });
    }
    if (parsed.pathname.startsWith('/api/meta/services/getFileInfo/')) {
      return jsonResponse(404, { errorCode: 'notfound' });
    }
    throw new Error(`unexpected request: ${parsed.pathname}`);
  };

  await assert.rejects(
    () => uploadMetadataFile({
      baseUrl: 'https://example.test',
      loginBody: { cipherPassport: 'prebuilt' },
      localFile,
      remotePath: '/missing/public/hooks/custom.js',
      fetchImpl,
    }),
    /remote project root does not exist/,
  );
});

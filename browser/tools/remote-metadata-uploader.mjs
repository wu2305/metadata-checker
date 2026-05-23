import { readFile } from 'node:fs/promises';
import { basename, dirname } from 'node:path/posix';
import { Buffer } from 'node:buffer';
import { fileURLToPath } from 'node:url';

const DEFAULT_USER_DIRECTORY = 'sys';

class CookieJar {
  constructor() {
    this.cookies = new Map();
  }

  addFromHeader(setCookieHeader) {
    if (!setCookieHeader) {
      return;
    }
    const values = Array.isArray(setCookieHeader) ? setCookieHeader : [setCookieHeader];
    for (const value of values) {
      const first = String(value).split(';')[0];
      const eq = first.indexOf('=');
      if (eq > 0) {
        this.cookies.set(first.slice(0, eq).trim(), first.slice(eq + 1).trim());
      }
    }
  }

  header() {
    return Array.from(this.cookies.entries()).map(([key, value]) => `${key}=${value}`).join('; ');
  }
}

function getSetCookieHeaders(headers) {
  if (typeof headers.getSetCookie === 'function') {
    return headers.getSetCookie();
  }
  const value = headers.get?.('set-cookie');
  return value ? [value] : [];
}

function normalizeBaseUrl(baseUrl) {
  if (!baseUrl || typeof baseUrl !== 'string') {
    throw new Error('baseUrl is required');
  }
  return baseUrl.replace(/\/+$/, '');
}

function normalizeRemotePath(remotePath) {
  if (!remotePath || typeof remotePath !== 'string') {
    throw new Error('remotePath is required');
  }
  if (!remotePath.startsWith('/')) {
    throw new Error(`remotePath must start with "/": ${remotePath}`);
  }
  if (remotePath.endsWith('/')) {
    throw new Error(`remotePath must be a file path, not a directory: ${remotePath}`);
  }
  return remotePath.replace(/\/+/g, '/');
}

function metadataInfoUrl(remotePath) {
  const normalized = normalizeRemotePath(remotePath);
  return `/api/meta/services/getFileInfo/${encodeURIComponent(normalized.slice(1)).replaceAll('%2F', '/')}`;
}

function inferTypeFromName(name) {
  const dot = name.lastIndexOf('.');
  return dot === -1 ? undefined : name.slice(dot + 1);
}

function makeLoginBody({ username, password, userDirectory = DEFAULT_USER_DIRECTORY }) {
  if (!username) {
    throw new Error('username is required');
  }
  if (!password) {
    throw new Error('password is required');
  }
  const loginArgs = {
    user: username,
    password,
    remember: false,
    userDirectory,
  };
  return {
    cipherPassport: Buffer.from(JSON.stringify(loginArgs), 'utf8').toString('base64'),
  };
}

function safeJsonParse(text) {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

async function requestJson(client, path, { method = 'GET', body } = {}) {
  const response = await client.fetch(`${client.baseUrl}${path}`, {
    method,
    headers: {
      ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
      ...(client.cookieJar.header() ? { Cookie: client.cookieJar.header() } : {}),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });

  client.cookieJar.addFromHeader(getSetCookieHeaders(response.headers));
  const text = await response.text();
  const json = safeJsonParse(text);
  return {
    ok: response.ok,
    status: response.status,
    text,
    json,
  };
}

async function login(client, { username, password, userDirectory, loginBody }) {
  const result = await requestJson(client, '/api/auth/signin', {
    method: 'POST',
    body: loginBody ?? makeLoginBody({ username, password, userDirectory }),
  });
  if (!result.ok || result.json?.result !== true) {
    throw new Error(`login failed: HTTP ${result.status}`);
  }
  return result.json;
}

async function getFileInfo(client, remotePath, { downloadContent = false } = {}) {
  const suffix = downloadContent ? '?downloadContent=true' : '';
  const result = await requestJson(client, `${metadataInfoUrl(remotePath)}${suffix}`);
  if (result.status === 404) {
    return null;
  }
  if (!result.ok) {
    throw new Error(`getFileInfo failed for ${remotePath}: HTTP ${result.status}`);
  }
  return result.json;
}

async function createFile(client, { parentDir, name, isFolder = false, content, type }) {
  const result = await requestJson(client, '/api/meta/file/createFile', {
    method: 'POST',
    body: {
      parentDir,
      name,
      isFolder,
      ...(content === undefined ? {} : { content }),
      ...(type === undefined ? {} : { type }),
    },
  });
  if (!result.ok) {
    throw new Error(`createFile failed for ${parentDir}/${name}: HTTP ${result.status}`);
  }
  return result.json;
}

async function modifyFile(client, { remotePath, content, revision }) {
  const result = await requestJson(client, '/api/meta/file/modifyFile', {
    method: 'POST',
    body: {
      idOrPath: remotePath,
      content,
      ...(revision === undefined ? {} : { revision }),
    },
  });
  if (!result.ok) {
    throw new Error(`modifyFile failed for ${remotePath}: HTTP ${result.status}`);
  }
  return result.json;
}

async function ensureDirectory(client, remoteDir) {
  const normalized = normalizeRemotePath(`${remoteDir}/placeholder`).replace(/\/placeholder$/, '');
  if (normalized === '') {
    return;
  }
  const segments = normalized.split('/').filter(Boolean);
  let current = '';
  for (let index = 0; index < segments.length; index += 1) {
    const segment = segments[index];
    const parentDir = current || '/';
    current = `${current}/${segment}`;
    const existing = await getFileInfo(client, current);
    if (existing) {
      if (!existing.isFolder) {
        throw new Error(`remote path exists but is not a folder: ${current}`);
      }
      continue;
    }
    if (index === 0) {
      throw new Error(`remote project root does not exist: ${current}`);
    }
    await createFile(client, {
      parentDir,
      name: segment,
      isFolder: true,
    });
  }
}

async function uploadMetadataFile(options) {
  const {
    baseUrl,
    username,
    password,
    userDirectory = DEFAULT_USER_DIRECTORY,
    loginBody,
    localFile,
    remotePath,
    fetchImpl = globalThis.fetch,
    dryRun = false,
  } = options;

  if (typeof fetchImpl !== 'function') {
    throw new Error('fetch implementation is required');
  }
  if (!localFile) {
    throw new Error('localFile is required');
  }

  const normalizedRemotePath = normalizeRemotePath(remotePath);
  const content = await readFile(localFile, 'utf8');
  const client = {
    baseUrl: normalizeBaseUrl(baseUrl),
    cookieJar: new CookieJar(),
    fetch: fetchImpl,
  };

  await login(client, { username, password, userDirectory, loginBody });
  const parentDir = dirname(normalizedRemotePath);
  const name = basename(normalizedRemotePath);

  if (dryRun) {
    const existing = await getFileInfo(client, normalizedRemotePath);
    return {
      status: 'dry_run',
      remotePath: normalizedRemotePath,
      action: existing ? 'modify' : 'create',
      bytes: Buffer.byteLength(content),
    };
  }

  await ensureDirectory(client, parentDir);
  const existing = await getFileInfo(client, normalizedRemotePath);
  let update;
  let action;
  if (existing) {
    update = await modifyFile(client, {
      remotePath: normalizedRemotePath,
      content,
      revision: existing.revision,
    });
    action = 'modified';
  } else {
    update = await createFile(client, {
      parentDir,
      name,
      isFolder: false,
      content,
      type: inferTypeFromName(name),
    });
    action = 'created';
  }

  const verified = await getFileInfo(client, normalizedRemotePath, { downloadContent: true });
  if (!verified || verified.content !== content) {
    throw new Error(`verification failed for ${normalizedRemotePath}`);
  }

  return {
    status: 'ok',
    action,
    remotePath: normalizedRemotePath,
    bytes: Buffer.byteLength(content),
    file: {
      id: verified.id,
      name: verified.name,
      type: verified.type,
      parentDir: verified.parentDir,
      revision: verified.revision,
      modifyTime: verified.modifyTime,
    },
    update,
  };
}

function parseArgs(argv) {
  const args = {};
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    if (!key.startsWith('--')) {
      throw new Error(`unexpected argument: ${key}`);
    }
    if (key === '--dry-run') {
      args.dryRun = true;
      continue;
    }
    const value = argv[index + 1];
    if (value === undefined || value.startsWith('--')) {
      throw new Error(`missing value for ${key}`);
    }
    index += 1;
    args[key.slice(2).replaceAll('-', '_')] = value;
  }
  return args;
}

function envOrArg(args, key, envName) {
  return args[key] ?? process.env[envName];
}

async function main(argv = process.argv.slice(2)) {
  const args = parseArgs(argv);
  const loginBody = args.login_body_file
    ? JSON.parse(await readFile(args.login_body_file, 'utf8'))
    : undefined;
  const result = await uploadMetadataFile({
    baseUrl: envOrArg(args, 'base_url', 'MC_REMOTE_BASE_URL'),
    username: envOrArg(args, 'username', 'MC_REMOTE_USERNAME'),
    password: envOrArg(args, 'password', 'MC_REMOTE_PASSWORD'),
    userDirectory: envOrArg(args, 'user_directory', 'MC_REMOTE_USER_DIRECTORY') ?? DEFAULT_USER_DIRECTORY,
    loginBody,
    localFile: args.file,
    remotePath: args.remote_path,
    dryRun: args.dryRun ?? false,
  });
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch(error => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}

export {
  CookieJar,
  ensureDirectory,
  getFileInfo,
  makeLoginBody,
  metadataInfoUrl,
  normalizeRemotePath,
  parseArgs,
  uploadMetadataFile,
};

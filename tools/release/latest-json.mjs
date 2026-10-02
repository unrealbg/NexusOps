import { readFile } from 'node:fs/promises';
import { basename } from 'node:path';
import { PRODUCT_NAME, assertSemver, exactKeys } from './release-common.mjs';
import { inspectUpdaterSignature } from './updater-signature.mjs';

export const LATEST_JSON_NAME = 'latest.json';
export const MAX_LATEST_JSON_BYTES = 32 * 1024;
export const RELEASE_TARGETS = Object.freeze([
  Object.freeze({ target: 'darwin-aarch64', platform: 'macos', architecture: 'aarch64' }),
  Object.freeze({ target: 'linux-x86_64', platform: 'linux', architecture: 'x86_64' }),
  Object.freeze({ target: 'windows-x86_64', platform: 'windows', architecture: 'x86_64' }),
]);

function fail(message) {
  throw new Error(message);
}

export function releaseArtifactBasename(target, version) {
  assertSemver(version, 'release version');
  const names = {
    'darwin-aarch64': `${PRODUCT_NAME}.app.tar.gz`,
    'linux-x86_64': `${PRODUCT_NAME}_${version}_amd64.AppImage`,
    'windows-x86_64': `${PRODUCT_NAME}_${version}_x64-setup.exe`,
  };
  if (!Object.hasOwn(names, target)) fail('unsupported release target');
  return names[target];
}

export function releaseAssetAllowlist(version) {
  const assets = RELEASE_TARGETS.flatMap(({ target }) => {
    const payload = releaseArtifactBasename(target, version);
    return [payload, `${payload}.sig`];
  });
  return [...assets, LATEST_JSON_NAME].sort();
}

export function immutableArtifactUrl(version, target) {
  const tag = `v${assertSemver(version, 'release version')}`;
  const artifact = releaseArtifactBasename(target, version);
  return `https://github.com/unrealbg/NexusOps/releases/download/${tag}/${artifact}`;
}

export function validateArtifactUrl(value, version, target) {
  if (typeof value !== 'string' || /[\x00-\x20\x7f]/.test(value)) fail('artifact URL is invalid');
  if (/^https:\/\/[^/]+:\d+(?:\/|$)/.test(value)) fail('artifact URL has an explicit port');
  let url;
  try {
    url = new URL(value);
  } catch {
    fail('artifact URL is malformed');
  }
  if (
    url.protocol !== 'https:' ||
    url.hostname !== 'github.com' ||
    url.port !== '' ||
    url.username !== '' ||
    url.password !== '' ||
    url.search !== '' ||
    url.hash !== ''
  ) {
    fail('artifact URL violates the production origin policy');
  }
  if (/%|\\|\.\./i.test(url.pathname)) fail('artifact URL path is ambiguous');
  const expected = immutableArtifactUrl(version, target);
  if (url.href !== expected) fail('artifact URL differs from the immutable release URL');
  if (basename(url.pathname) !== releaseArtifactBasename(target, version))
    fail('artifact URL basename differs');
  return value;
}

function signatureText(value, target, version) {
  if (typeof value !== 'string') fail(`${target} signature must be text`);
  const artifact = releaseArtifactBasename(target, version);
  inspectUpdaterSignature(Buffer.from(value, 'utf8'), artifact, version);
  return value;
}

export function canonicalLatestJson(manifest) {
  const platforms = {};
  for (const { target } of RELEASE_TARGETS) {
    const entry = manifest.platforms?.[target];
    platforms[target] = { url: entry?.url, signature: entry?.signature };
  }
  return `${JSON.stringify({ version: manifest.version, platforms }, null, 2)}\n`;
}

export function createLatestManifest(version, signatures) {
  assertSemver(version, 'release version');
  exactKeys(signatures, RELEASE_TARGETS.map(({ target }) => target), 'release signatures');
  const platforms = {};
  for (const { target } of RELEASE_TARGETS) {
    platforms[target] = {
      url: immutableArtifactUrl(version, target),
      signature: signatureText(signatures[target], target, version),
    };
  }
  return { version, platforms };
}

export function createLatestJson(version, signatures) {
  const bytes = Buffer.from(canonicalLatestJson(createLatestManifest(version, signatures)), 'utf8');
  if (bytes.length > MAX_LATEST_JSON_BYTES) fail('latest.json exceeds 32 KiB');
  return bytes;
}

export function verifyLatestJson(bytes, expectedVersion, expectedSignatures) {
  if (!Buffer.isBuffer(bytes) || bytes.length === 0 || bytes.length > MAX_LATEST_JSON_BYTES)
    fail('latest.json size is invalid');
  if (bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) fail('latest.json has a BOM');
  const text = bytes.toString('utf8');
  if (!Buffer.from(text, 'utf8').equals(bytes)) fail('latest.json is not UTF-8');
  let manifest;
  try {
    manifest = JSON.parse(text);
  } catch {
    fail('latest.json is malformed JSON');
  }
  exactKeys(manifest, ['version', 'platforms'], 'latest.json');
  assertSemver(manifest.version, 'latest.json version');
  if (manifest.version !== expectedVersion) fail('latest.json version differs from release version');
  exactKeys(manifest.platforms, RELEASE_TARGETS.map(({ target }) => target), 'latest.json platforms');
  if (expectedSignatures !== undefined)
    exactKeys(expectedSignatures, RELEASE_TARGETS.map(({ target }) => target), 'expected signatures');
  for (const { target } of RELEASE_TARGETS) {
    const entry = manifest.platforms[target];
    exactKeys(entry, ['url', 'signature'], `${target} entry`);
    validateArtifactUrl(entry.url, expectedVersion, target);
    signatureText(entry.signature, target, expectedVersion);
    if (expectedSignatures !== undefined && entry.signature !== expectedSignatures[target])
      fail(`${target} signature differs from verified candidate`);
  }
  if (canonicalLatestJson(manifest) !== text) fail('latest.json is not canonical');
  return manifest;
}

export async function readReleaseSignatures(directory, version) {
  const signatures = {};
  for (const { target } of RELEASE_TARGETS) {
    const artifact = releaseArtifactBasename(target, version);
    signatures[target] = await readFile(`${directory}/${artifact}.sig`, 'utf8').catch(() =>
      fail(`${target} signature is missing or unreadable`),
    );
  }
  return signatures;
}

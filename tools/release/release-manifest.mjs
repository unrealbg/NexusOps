import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import {
  MANIFEST_NAME,
  MANIFEST_SCHEMA_VERSION,
  PRODUCT_NAME,
  assertSemver,
  exactKeys,
  expectedExecutable,
  hashOrdinaryFile,
  stagedFileNames,
} from './release-common.mjs';

function fail(message) {
  throw new Error(message);
}

function validateIdentity(identity) {
  exactKeys(
    identity,
    ['productVersion', 'sourceCommit', 'platform', 'architecture'],
    'release identity',
  );
  assertSemver(identity.productVersion, 'product version');
  if (!/^[0-9a-f]{40}$/.test(identity.sourceCommit)) fail('source commit must be a full SHA');
  if (!['windows', 'linux', 'macos'].includes(identity.platform))
    fail('unsupported release platform');
  if (!['x86_64', 'aarch64'].includes(identity.architecture))
    fail('unsupported release architecture');
  return expectedExecutable(identity.platform);
}

export function canonicalManifest(manifest) {
  return `${JSON.stringify(manifest, null, 2)}\n`;
}

export function validateManifest(manifest, identity) {
  const executableName = validateIdentity(identity);
  exactKeys(
    manifest,
    [
      'schemaVersion',
      'productName',
      'productVersion',
      'sourceCommit',
      'platform',
      'architecture',
      'artifacts',
    ],
    'release manifest',
  );
  if (manifest.schemaVersion !== MANIFEST_SCHEMA_VERSION || manifest.productName !== PRODUCT_NAME)
    fail('release manifest schema or product differs');
  for (const key of ['productVersion', 'sourceCommit', 'platform', 'architecture']) {
    if (manifest[key] !== identity[key])
      fail(`release manifest ${key} differs from checked-out source`);
  }
  if (!Array.isArray(manifest.artifacts) || manifest.artifacts.length === 0)
    fail('release manifest has no artifacts');
  const seen = new Set();
  let previous = '';
  for (const entry of manifest.artifacts) {
    exactKeys(entry, ['basename', 'bytes', 'sha256'], 'artifact entry');
    if (
      typeof entry.basename !== 'string' ||
      !/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(entry.basename)
    ) {
      fail('artifact basename is unsafe');
    }
    if (seen.has(entry.basename)) fail('duplicate artifact basename');
    seen.add(entry.basename);
    if (previous && previous >= entry.basename) fail('artifact entries are not in lexical order');
    previous = entry.basename;
    if (!Number.isSafeInteger(entry.bytes) || entry.bytes <= 0)
      fail('artifact byte length is invalid');
    if (typeof entry.sha256 !== 'string' || !/^[0-9a-f]{64}$/.test(entry.sha256))
      fail('artifact SHA-256 is invalid');
  }
  if (manifest.artifacts.length !== 1 || manifest.artifacts[0].basename !== executableName) {
    fail('artifact set is not the Goal 04B executable allowlist');
  }
  return executableName;
}

export async function createReleaseManifest(stage, identity) {
  const executableName = validateIdentity(identity);
  const names = await stagedFileNames(stage);
  if (names.length !== 1 || names[0] !== executableName)
    fail('staging must contain only the allowlisted executable');
  const digest = await hashOrdinaryFile(join(stage, executableName));
  const manifest = {
    schemaVersion: MANIFEST_SCHEMA_VERSION,
    productName: PRODUCT_NAME,
    productVersion: identity.productVersion,
    sourceCommit: identity.sourceCommit,
    platform: identity.platform,
    architecture: identity.architecture,
    artifacts: [{ basename: executableName, bytes: digest.bytes, sha256: digest.sha256 }],
  };
  validateManifest(manifest, identity);
  await writeFile(join(stage, MANIFEST_NAME), canonicalManifest(manifest), {
    encoding: 'utf8',
    flag: 'wx',
  });
  return manifest;
}

export async function verifyReleaseManifest(stage, identity) {
  const executableName = validateIdentity(identity);
  const names = await stagedFileNames(stage);
  if (names.length !== 2 || !names.includes(executableName) || !names.includes(MANIFEST_NAME)) {
    fail('staging has missing or unmanifested entries');
  }
  let source;
  try {
    source = await readFile(join(stage, MANIFEST_NAME), 'utf8');
  } catch {
    fail('release manifest is missing or unreadable');
  }
  let manifest;
  try {
    manifest = JSON.parse(source);
  } catch {
    fail('release manifest is malformed JSON');
  }
  validateManifest(manifest, identity);
  if (canonicalManifest(manifest) !== source)
    fail('release manifest formatting is not deterministic');
  const actual = await hashOrdinaryFile(join(stage, executableName));
  const recorded = manifest.artifacts[0];
  if (recorded.bytes !== actual.bytes || recorded.sha256 !== actual.sha256)
    fail('artifact size or SHA-256 differs from manifest');
  return manifest;
}

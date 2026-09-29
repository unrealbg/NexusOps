import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import {
  PRODUCT_NAME,
  assertOrdinaryPath,
  assertSemver,
  exactKeys,
  hashOrdinaryFile,
  stagedFileNames,
} from './release-common.mjs';
import { inspectUpdaterSignature } from './updater-signature.mjs';

export const SIGNED_CANDIDATE_NAME = 'signed-updater-candidate.json';
export const SIGNED_CANDIDATE_SCHEMA_VERSION = 1;

function fail(message) {
  throw new Error(message);
}

function validateIdentity(identity) {
  exactKeys(identity, ['productVersion', 'sourceCommit', 'platform', 'architecture'], 'identity');
  assertSemver(identity.productVersion, 'product version');
  if (!/^[0-9a-f]{40}$/.test(identity.sourceCommit)) fail('source commit is not a full SHA');
  if (!['windows', 'linux', 'macos'].includes(identity.platform))
    fail('unsupported updater platform');
  if (!['x86_64', 'aarch64'].includes(identity.architecture))
    fail('unsupported updater architecture');
}

export function updaterArtifactBasename(name, identity) {
  validateIdentity(identity);
  if (typeof name !== 'string' || !/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(name))
    fail('updater artifact basename is unsafe');
  // Tauri 2.12's macOS archive uses the .app basename without a version.
  // Version and architecture remain bound in the trusted comment and metadata.
  const expected = {
    windows: `${PRODUCT_NAME}_${identity.productVersion}_${identity.architecture === 'x86_64' ? 'x64' : 'arm64'}-setup.exe`,
    linux: `${PRODUCT_NAME}_${identity.productVersion}_${identity.architecture === 'x86_64' ? 'amd64' : 'aarch64'}.AppImage`,
    macos: `${PRODUCT_NAME}.app.tar.gz`,
  }[identity.platform];
  if (name !== expected) fail('updater artifact basename differs from exact Tauri output');
  return name;
}

export function canonicalSignedCandidate(metadata) {
  return `${JSON.stringify(metadata, null, 2)}\n`;
}

export function validateSignedCandidate(metadata, identity, publicKeySha256) {
  validateIdentity(identity);
  if (typeof publicKeySha256 !== 'string' || !/^[0-9a-f]{64}$/.test(publicKeySha256))
    fail('public key SHA-256 is invalid');
  exactKeys(
    metadata,
    [
      'schemaVersion',
      'productName',
      'productVersion',
      'sourceCommit',
      'platform',
      'architecture',
      'artifact',
      'signature',
      'publicKeySha256',
    ],
    'signed candidate',
  );
  if (metadata.schemaVersion !== SIGNED_CANDIDATE_SCHEMA_VERSION || metadata.productName !== PRODUCT_NAME)
    fail('signed candidate schema or product differs');
  for (const key of ['productVersion', 'sourceCommit', 'platform', 'architecture']) {
    if (metadata[key] !== identity[key]) fail(`signed candidate ${key} differs from source`);
  }
  if (metadata.publicKeySha256 !== publicKeySha256)
    fail('signed candidate public key SHA-256 differs from committed key');
  exactKeys(metadata.artifact, ['basename', 'bytes', 'sha256'], 'updater artifact');
  exactKeys(metadata.signature, ['basename', 'sha256'], 'updater signature');
  const artifactName = updaterArtifactBasename(metadata.artifact.basename, identity);
  if (
    !Number.isSafeInteger(metadata.artifact.bytes) ||
    metadata.artifact.bytes <= 0 ||
    !/^[0-9a-f]{64}$/.test(metadata.artifact.sha256)
  ) {
    fail('updater artifact size or SHA-256 is invalid');
  }
  if (
    metadata.signature.basename !== `${artifactName}.sig` ||
    !/^[0-9a-f]{64}$/.test(metadata.signature.sha256)
  ) {
    fail('updater signature name or SHA-256 is invalid');
  }
  return artifactName;
}

async function inspectStage(stage, identity) {
  validateIdentity(identity);
  const names = await stagedFileNames(stage);
  const artifacts = names.filter((name) => {
    try {
      updaterArtifactBasename(name, identity);
      return true;
    } catch {
      return false;
    }
  });
  if (artifacts.length !== 1) fail('stage does not contain exactly one updater artifact');
  const artifactName = artifacts[0];
  const signatureName = `${artifactName}.sig`;
  await assertOrdinaryPath(join(stage, artifactName), 'file');
  await assertOrdinaryPath(join(stage, signatureName), 'file');
  const artifact = await hashOrdinaryFile(join(stage, artifactName));
  const signature = inspectUpdaterSignature(
    await readFile(join(stage, signatureName)),
    artifactName,
    identity.productVersion,
  );
  return { names, artifactName, signatureName, artifact, signature };
}

export async function createSignedCandidate(stage, identity, publicKeySha256) {
  const contents = await inspectStage(stage, identity);
  if (
    contents.names.length !== 2 ||
    !contents.names.includes(contents.signatureName)
  ) {
    fail('stage has missing or unexpected pre-metadata entries');
  }
  const metadata = {
    schemaVersion: SIGNED_CANDIDATE_SCHEMA_VERSION,
    productName: PRODUCT_NAME,
    productVersion: identity.productVersion,
    sourceCommit: identity.sourceCommit,
    platform: identity.platform,
    architecture: identity.architecture,
    artifact: {
      basename: contents.artifactName,
      bytes: contents.artifact.bytes,
      sha256: contents.artifact.sha256,
    },
    signature: {
      basename: contents.signatureName,
      sha256: contents.signature.sha256,
    },
    publicKeySha256,
  };
  validateSignedCandidate(metadata, identity, publicKeySha256);
  await writeFile(join(stage, SIGNED_CANDIDATE_NAME), canonicalSignedCandidate(metadata), {
    encoding: 'utf8',
    flag: 'wx',
  });
  return metadata;
}

export async function verifySignedCandidate(stage, identity, publicKeySha256) {
  const contents = await inspectStage(stage, identity);
  if (
    contents.names.length !== 3 ||
    !contents.names.includes(contents.signatureName) ||
    !contents.names.includes(SIGNED_CANDIDATE_NAME)
  ) {
    fail('stage has missing or unexpected entries');
  }
  const text = await readFile(join(stage, SIGNED_CANDIDATE_NAME), 'utf8');
  let metadata;
  try {
    metadata = JSON.parse(text);
  } catch {
    fail('signed candidate metadata is malformed JSON');
  }
  validateSignedCandidate(metadata, identity, publicKeySha256);
  if (canonicalSignedCandidate(metadata) !== text)
    fail('signed candidate metadata is not canonical');
  if (
    metadata.artifact.basename !== contents.artifactName ||
    metadata.artifact.bytes !== contents.artifact.bytes ||
    metadata.artifact.sha256 !== contents.artifact.sha256
  ) {
    fail('updater artifact differs from signed candidate metadata');
  }
  if (
    metadata.signature.basename !== contents.signatureName ||
    metadata.signature.sha256 !== contents.signature.sha256
  ) {
    fail('updater signature differs from signed candidate metadata');
  }
  return metadata;
}

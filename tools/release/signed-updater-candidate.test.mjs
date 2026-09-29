import assert from 'node:assert/strict';
import { mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import {
  canonicalSignedCandidate,
  createSignedCandidate,
  updaterArtifactBasename,
  verifySignedCandidate,
} from './signed-updater-candidate.mjs';
import { validateUpdaterPublicKey } from './updater-public-key.mjs';
import { inspectUpdaterSignature } from './updater-signature.mjs';
import { requireUpdaterSigningEnvironment } from './signing-environment.mjs';

const IDENTITY = {
  productVersion: '0.1.0',
  sourceCommit: 'a'.repeat(40),
  platform: 'windows',
  architecture: 'x86_64',
};
const ARTIFACT = 'NexusOps_0.1.0_x64-setup.exe';
const SIGNATURE = `${ARTIFACT}.sig`;
const PUBLIC_KEY_SHA = 'b'.repeat(64);

test('signing precondition fails without either secret and never returns key material', () => {
  assert.throws(() => requireUpdaterSigningEnvironment({}), /unavailable/);
  assert.throws(
    () => requireUpdaterSigningEnvironment({ TAURI_SIGNING_PRIVATE_KEY: 'synthetic' }),
    /unavailable/,
  );
  assert.throws(
    () => requireUpdaterSigningEnvironment({ TAURI_SIGNING_PRIVATE_KEY_PASSWORD: 'synthetic' }),
    /unavailable/,
  );
  assert.equal(
    requireUpdaterSigningEnvironment({
      TAURI_SIGNING_PRIVATE_KEY: 'synthetic',
      TAURI_SIGNING_PRIVATE_KEY_PASSWORD: 'synthetic',
    }),
    undefined,
  );
});

function syntheticPublicKey() {
  const key = Buffer.alloc(42, 1);
  key.write('Ed', 0, 'ascii');
  const keyId = Buffer.from(key.subarray(2, 10)).reverse().toString('hex').toUpperCase();
  return Buffer.from(
    Buffer.from(
      `untrusted comment: minisign public key: ${keyId}\n${key.toString('base64')}\n`,
    ).toString('base64'),
  );
}

function syntheticSignature(artifact = ARTIFACT, version = '0.1.0', signatureByte = 3) {
  const signed = Buffer.alloc(74, signatureByte);
  signed.write('Ed', 0, 'ascii');
  const global = Buffer.alloc(64, 5);
  const lines = [
    'untrusted comment: signature from tauri secret key',
    signed.toString('base64'),
    `trusted comment: timestamp:1700000000\tfile:${artifact}\tversion:${version}`,
    global.toString('base64'),
    '',
  ];
  return Buffer.from(Buffer.from(lines.join('\n')).toString('base64'));
}

async function stage(t) {
  const dir = await mkdtemp(join(await realpath(tmpdir()), 'nexusops-signed-candidate-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  await writeFile(join(dir, ARTIFACT), 'synthetic installer bytes');
  await writeFile(join(dir, SIGNATURE), syntheticSignature());
  return dir;
}

async function modifyMetadata(dir, change) {
  const path = join(dir, 'signed-updater-candidate.json');
  const metadata = JSON.parse(await readFile(path, 'utf8'));
  change(metadata);
  await writeFile(path, canonicalSignedCandidate(metadata));
}

test('one canonical synthetic minisign public key passes', () => {
  const result = validateUpdaterPublicKey(syntheticPublicKey());
  assert.match(result.sha256, /^[0-9a-f]{64}$/);
});

test('malformed public key and private-key-looking material fail closed', () => {
  assert.throws(() => validateUpdaterPublicKey(Buffer.from('not base64')));
  const secretLooking = Buffer.from(
    Buffer.from('untrusted comment: rsign encrypted secret key\nRWQx\n').toString('base64'),
  );
  assert.throws(() => validateUpdaterPublicKey(secretLooking));
  assert.throws(() => validateUpdaterPublicKey(Buffer.concat([syntheticPublicKey(), Buffer.from([0])])));
  assert.throws(() => validateUpdaterPublicKey(Buffer.alloc(1025, 65)));
});

test('synthetic signature has an exact artifact and version trusted comment', () => {
  assert.equal(inspectUpdaterSignature(syntheticSignature(), ARTIFACT, '0.1.0').productVersion, '0.1.0');
  assert.throws(() => inspectUpdaterSignature(syntheticSignature(), ARTIFACT, '0.2.0'));
  assert.throws(() => inspectUpdaterSignature(syntheticSignature(), 'other-setup.exe', '0.1.0'));
});

test('signature rejects empty, oversized, missing and ambiguous version binding', () => {
  assert.throws(() => inspectUpdaterSignature(Buffer.alloc(0), ARTIFACT, '0.1.0'));
  assert.throws(() => inspectUpdaterSignature(Buffer.alloc(4097, 65), ARTIFACT, '0.1.0'));
  const missing = syntheticSignature().toString('utf8');
  const decoded = Buffer.from(missing, 'base64').toString('utf8');
  const withoutVersion = Buffer.from(
    decoded.replace('\tversion:0.1.0', ''),
  ).toString('base64');
  assert.throws(() => inspectUpdaterSignature(Buffer.from(withoutVersion), ARTIFACT, '0.1.0'));
  const duplicate = Buffer.from(
    decoded.replace('\tversion:0.1.0', '\tversion:0.1.0\tversion:0.1.0'),
  ).toString('base64');
  assert.throws(() => inspectUpdaterSignature(Buffer.from(duplicate), ARTIFACT, '0.1.0'));
  assert.throws(() => inspectUpdaterSignature(syntheticSignature(ARTIFACT, '0.2.0'), ARTIFACT, '0.1.0'));
});

test('candidate metadata is deterministic and verifies the exact synthetic pair', async (t) => {
  const dir = await stage(t);
  const created = await createSignedCandidate(dir, IDENTITY, PUBLIC_KEY_SHA);
  const verified = await verifySignedCandidate(dir, IDENTITY, PUBLIC_KEY_SHA);
  assert.deepEqual(verified, created);
  assert.equal(verified.artifact.basename, ARTIFACT);
  assert.equal(verified.signature.basename, SIGNATURE);
  assert.equal(verified.publicKeySha256, PUBLIC_KEY_SHA);
});

for (const [label, mutate] of [
  ['artifact byte change', async (dir) => writeFile(join(dir, ARTIFACT), 'synthetic installer byteX')],
  ['artifact truncation', async (dir) => writeFile(join(dir, ARTIFACT), '')],
  ['artifact size mismatch', async (dir) => modifyMetadata(dir, (m) => { m.artifact.bytes += 1; })],
  ['artifact SHA mismatch', async (dir) => modifyMetadata(dir, (m) => { m.artifact.sha256 = 'c'.repeat(64); })],
  ['signature byte change', async (dir) => writeFile(join(dir, SIGNATURE), syntheticSignature(ARTIFACT, '0.1.0', 7))],
  ['signature SHA mismatch', async (dir) => modifyMetadata(dir, (m) => { m.signature.sha256 = 'c'.repeat(64); })],
  ['missing signature', async (dir) => rm(join(dir, SIGNATURE))],
  ['unexpected file', async (dir) => writeFile(join(dir, 'extra.txt'), 'not allowlisted')],
  ['path traversal', async (dir) => modifyMetadata(dir, (m) => { m.artifact.basename = '../evil-setup.exe'; })],
  ['absolute path', async (dir) => modifyMetadata(dir, (m) => { m.artifact.basename = 'C:\\evil-setup.exe'; })],
]) {
  test(`candidate verification rejects ${label}`, async (t) => {
    const dir = await stage(t);
    await createSignedCandidate(dir, IDENTITY, PUBLIC_KEY_SHA);
    await mutate(dir);
    await assert.rejects(verifySignedCandidate(dir, IDENTITY, PUBLIC_KEY_SHA));
  });
}

test('candidate identity cannot be rebound to another source, version, platform or architecture', async (t) => {
  const dir = await stage(t);
  await createSignedCandidate(dir, IDENTITY, PUBLIC_KEY_SHA);
  for (const identity of [
    { ...IDENTITY, sourceCommit: 'c'.repeat(40) },
    { ...IDENTITY, productVersion: '0.2.0' },
    { ...IDENTITY, platform: 'linux' },
    { ...IDENTITY, architecture: 'aarch64' },
  ]) {
    await assert.rejects(verifySignedCandidate(dir, identity, PUBLIC_KEY_SHA));
  }
  await assert.rejects(verifySignedCandidate(dir, IDENTITY, 'c'.repeat(64)));
});

test('candidate artifact basename enforces one platform bundle format', () => {
  assert.equal(updaterArtifactBasename(ARTIFACT, IDENTITY), ARTIFACT);
  assert.throws(() => updaterArtifactBasename('NexusOps_0.1.0_x64.msi', IDENTITY));
  assert.throws(() => updaterArtifactBasename('NexusOps_10.1.0_x64-setup.exe', IDENTITY));
  assert.throws(() => updaterArtifactBasename('../NexusOps_0.1.0_x64-setup.exe', IDENTITY));
  assert.throws(() => updaterArtifactBasename('D:\\NexusOps_0.1.0_x64-setup.exe', IDENTITY));
  assert.throws(() => updaterArtifactBasename('NexusOps_0.1.0_arm64-setup.exe', IDENTITY));
  assert.equal(updaterArtifactBasename('NexusOps_0.1.0_amd64.AppImage', {
    ...IDENTITY, platform: 'linux',
  }), 'NexusOps_0.1.0_amd64.AppImage');
  assert.equal(updaterArtifactBasename('NexusOps.app.tar.gz', {
    ...IDENTITY, platform: 'macos',
  }), 'NexusOps.app.tar.gz');
  assert.throws(() => updaterArtifactBasename('Other.app.tar.gz', {
    ...IDENTITY, platform: 'macos',
  }));
});

import assert from 'node:assert/strict';
import {
  mkdtemp,
  mkdir,
  readFile,
  realpath,
  rename,
  rm,
  unlink,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import {
  createReleaseManifest,
  canonicalManifest,
  verifyReleaseManifest,
} from './release-manifest.mjs';

const identity = {
  productVersion: '0.1.0',
  sourceCommit: 'a'.repeat(40),
  platform: 'windows',
  architecture: 'x86_64',
};
const binary = 'nexus-desktop.exe';
const original = Buffer.from('NexusOps synthetic executable bytes');

async function fixture(t) {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'nexusops-release-manifest-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const stage = join(root, 'stage');
  await mkdir(stage);
  await writeFile(join(stage, binary), original);
  await createReleaseManifest(stage, identity);
  return stage;
}

async function editManifest(stage, change) {
  const path = join(stage, 'release-manifest.json');
  const manifest = JSON.parse(await readFile(path, 'utf8'));
  change(manifest);
  await writeFile(path, canonicalManifest(manifest));
}

test('valid executable and deterministic manifest verify', async (t) => {
  const stage = await fixture(t);
  const manifest = await verifyReleaseManifest(stage, identity);
  assert.equal(manifest.artifacts[0].bytes, original.length);
  assert.match(manifest.artifacts[0].sha256, /^[0-9a-f]{64}$/);
  assert.equal(manifest.sourceCommit, identity.sourceCommit);
  assert.equal(manifest.productVersion, identity.productVersion);
  assert.equal(
    (await readFile(join(stage, 'release-manifest.json'), 'utf8')).includes(stage),
    false,
  );
});

test('identical input metadata produces identical manifest bytes', async (t) => {
  const first = await fixture(t);
  const second = await fixture(t);
  assert.deepEqual(
    await readFile(join(first, 'release-manifest.json')),
    await readFile(join(second, 'release-manifest.json')),
  );
});

for (const [name, tamper] of [
  [
    'one-byte content change',
    (stage) =>
      writeFile(join(stage, binary), Buffer.concat([Buffer.from('X'), original.subarray(1)])),
  ],
  [
    'artifact truncation',
    (stage) => writeFile(join(stage, binary), original.subarray(0, original.length - 1)),
  ],
  [
    'artifact replacement',
    (stage) => writeFile(join(stage, binary), Buffer.alloc(original.length, 0x5a)),
  ],
  ['renamed artifact', (stage) => rename(join(stage, binary), join(stage, 'renamed.exe'))],
  ['extra unmanifested artifact', (stage) => writeFile(join(stage, 'unexpected.txt'), 'x')],
  ['missing artifact', (stage) => unlink(join(stage, binary))],
  [
    'manifest SHA tamper',
    (stage) =>
      editManifest(stage, (data) => {
        data.artifacts[0].sha256 = '0'.repeat(64);
      }),
  ],
  [
    'manifest size tamper',
    (stage) =>
      editManifest(stage, (data) => {
        data.artifacts[0].bytes += 1;
      }),
  ],
  [
    'manifest path traversal',
    (stage) =>
      editManifest(stage, (data) => {
        data.artifacts[0].basename = '../escape.exe';
      }),
  ],
  [
    'manifest absolute Unix path',
    (stage) =>
      editManifest(stage, (data) => {
        data.artifacts[0].basename = '/tmp/escape.exe';
      }),
  ],
  [
    'manifest absolute Windows path',
    (stage) =>
      editManifest(stage, (data) => {
        data.artifacts[0].basename = 'C:\\escape.exe';
      }),
  ],
  [
    'duplicate basename',
    (stage) =>
      editManifest(stage, (data) => {
        data.artifacts.push({ ...data.artifacts[0] });
      }),
  ],
  [
    'wrong source SHA',
    (stage) =>
      editManifest(stage, (data) => {
        data.sourceCommit = 'b'.repeat(40);
      }),
  ],
  ['unexpected directory', (stage) => mkdir(join(stage, 'directory'))],
]) {
  test(`${name} fails closed`, async (t) => {
    const stage = await fixture(t);
    await tamper(stage);
    await assert.rejects(verifyReleaseManifest(stage, identity));
  });
}

test('generator rejects an extra file before writing a manifest', async (t) => {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'nexusops-release-staging-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(join(root, binary), original);
  await writeFile(join(root, 'secret.key'), 'synthetic');
  await assert.rejects(createReleaseManifest(root, identity), /allowlisted executable/);
});

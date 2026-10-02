import assert from 'node:assert/strict';
import { mkdtemp, mkdir, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { RELEASE_TARGETS, releaseArtifactBasename } from './latest-json.mjs';
import { verifyReleaseCryptography } from './updater-crypto.mjs';

test('cryptographic helper receives each exact allowlisted payload/signature pair', async (t) => {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'nexusops-release-crypto-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(root, { recursive: true });
  for (const { target } of RELEASE_TARGETS) {
    const artifact = join(root, releaseArtifactBasename(target, '0.1.1'));
    await writeFile(artifact, 'payload');
    await writeFile(`${artifact}.sig`, 'signature');
  }
  const calls = [];
  const result = await verifyReleaseCryptography(root, '0.1.1', (args) => calls.push(args));
  assert.deepEqual(result, RELEASE_TARGETS.map(({ target }) => target));
  assert.equal(calls.length, 3);
  for (const [index, { target }] of RELEASE_TARGETS.entries()) {
    const artifact = join(root, releaseArtifactBasename(target, '0.1.1'));
    assert.deepEqual(calls[index].slice(0, 10), [
      'run', '--offline', '-q', '-p', 'nexus-core', '--example', 'verify_updater_signature', '--locked', '--', artifact,
    ]);
    assert.equal(calls[index][10], `${artifact}.sig`);
    assert.match(calls[index][11], /nexusops-updater\.pub$/);
    assert.equal(calls[index][12], '0.1.1');
  }
});

test('missing payload or signature fails before invoking crypto', async (t) => {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'nexusops-release-crypto-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  let called = false;
  await assert.rejects(
    verifyReleaseCryptography(root, '0.1.1', () => {
      called = true;
    }),
  );
  assert.equal(called, false);
});

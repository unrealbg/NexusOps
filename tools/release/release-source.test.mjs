import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { releaseArtifactName, verifyReleaseSource } from './release-common.mjs';

function git(root, args) {
  return execFileSync('git', args, {
    cwd: root,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
}

test('source identity requires exact HEAD, matching CI SHA and clean tracked/untracked inputs', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'nexusops-release-source-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  git(root, ['init', '-q']);
  await writeFile(join(root, 'source.txt'), 'committed source\n');
  git(root, ['add', 'source.txt']);
  git(root, [
    '-c',
    'user.name=NexusOps Test',
    '-c',
    'user.email=test@example.invalid',
    'commit',
    '-qm',
    'synthetic source',
  ]);
  const head = git(root, ['rev-parse', 'HEAD']);
  assert.equal(verifyReleaseSource(root, { GITHUB_SHA: head }), head);
  assert.throws(
    () => verifyReleaseSource(root, { GITHUB_SHA: 'b'.repeat(40) }),
    /GITHUB_SHA does not match/,
  );
  await writeFile(join(root, 'source.txt'), 'modified source\n');
  assert.throws(() => verifyReleaseSource(root, { GITHUB_SHA: head }), /working tree is dirty/);
  git(root, ['restore', 'source.txt']);
  await writeFile(join(root, 'untracked-input.mjs'), 'export default 1;\n');
  assert.throws(() => verifyReleaseSource(root, { GITHUB_SHA: head }), /working tree is dirty/);
});

test('release artifact name is derived only from verified metadata', () => {
  assert.equal(
    releaseArtifactName({
      productVersion: '0.1.0',
      platform: 'windows',
      architecture: 'x86_64',
      sourceCommit: 'a'.repeat(40),
    }),
    'NexusOps-0.1.0-windows-x86_64-aaaaaaaa',
  );
  assert.throws(
    () =>
      releaseArtifactName({
        productVersion: '0.1.0',
        platform: 'windows',
        architecture: '../evil',
        sourceCommit: 'a'.repeat(40),
      }),
    /invalid release artifact/,
  );
});

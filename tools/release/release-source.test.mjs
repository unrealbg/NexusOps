import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import {
  REPOSITORY_ROOT,
  releaseArtifactName,
  updaterCandidateArtifactName,
  verifyReleaseSource,
} from './release-common.mjs';

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

test('ordinary and signed updater workflow artifact namespaces stay distinct', () => {
  const identity = {
    productVersion: '0.1.1',
    platform: 'windows',
    architecture: 'x86_64',
    sourceCommit: '62fd0832ca252448b6bb13b58af347c50e697a53',
  };
  assert.equal(
    releaseArtifactName(identity),
    'NexusOps-0.1.1-windows-x86_64-62fd0832',
  );
  assert.equal(
    updaterCandidateArtifactName(identity),
    'NexusOps-updater-0.1.1-windows-x86_64-62fd0832',
  );
});

for (const [label, identity] of [
  ['invalid version', { productVersion: 'v0.1.1' }],
  ['invalid platform', { platform: 'freebsd' }],
  ['invalid architecture', { architecture: 'armv7' }],
  ['short source SHA', { sourceCommit: '62fd0832' }],
  ['uppercase source SHA', { sourceCommit: 'A'.repeat(40) }],
]) {
  test(`signed updater workflow artifact name rejects ${label}`, () => {
    assert.throws(() =>
      updaterCandidateArtifactName({
        productVersion: '0.1.1',
        platform: 'linux',
        architecture: 'x86_64',
        sourceCommit: 'a'.repeat(40),
        ...identity,
      }),
    );
  });
}

test('manual release workflow fails closed without preinstalled rustup and has no live bootstrap', async () => {
  const workflow = await readFile(
    join(REPOSITORY_ROOT, '.github/workflows/release-candidate.yml'),
    'utf8',
  );
  assert.match(workflow, /command -v rustup\b/);
  assert.match(workflow, /rustup toolchain install 1\.98\.1[^\r\n]*--no-self-update/);
  assert.doesNotMatch(workflow, /actions-rust-lang\/setup-rust-toolchain@/);
  assert.doesNotMatch(workflow, /sh\.rustup\.rs/i);
  assert.doesNotMatch(workflow, /\b(?:curl|wget)\b[^\r\n]*\|\s*(?:sh|bash)\b/i);
  assert.doesNotMatch(
    workflow,
    /\bInvoke-WebRequest\b[^\r\n]*\|\s*(?:Invoke-Expression|iex|sh|bash)\b/i,
  );
  const actionRefs = [...workflow.matchAll(/^\s*-\s+uses:\s+(\S+)/gm)].map((match) => match[1]);
  assert.deepEqual(
    actionRefs.map((reference) => reference.split('@')[0]),
    ['actions/checkout', 'actions/setup-node', 'actions/attest', 'actions/upload-artifact'],
  );
  for (const reference of actionRefs) assert.match(reference, /@[0-9a-f]{40}$/);
});

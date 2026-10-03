import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { createLatestJson, releaseAssetAllowlist } from './latest-json.mjs';
import { REPOSITORY_ROOT } from './release-common.mjs';
import { createDraftReleaseRequest } from './release-request.mjs';
import {
  comparePublicationDirectories,
  expectedCandidateArtifacts,
  validateCandidateRun,
  validateImmutableReleaseStatus,
  validateNoConflictingReleaseState,
  validateReleaseAssets,
  validateReleaseState,
  validateTagRef,
} from './release-publication-common.mjs';

const SHA = 'a'.repeat(40);
const PRODUCTION_SHAPE_SHA = '62fd0832ca252448b6bb13b58af347c50e697a53';
const VERSION = '0.1.1';
const RUN_ID = 123;

test('committed 0.1.2 notes produce the exact deterministic draft request', async () => {
  const sourceCommit = 'b'.repeat(40);
  const body = await readFile(join(REPOSITORY_ROOT, 'docs/release/notes/v0.1.2.md'), 'utf8');
  assert.deepEqual(await createDraftReleaseRequest('0.1.2', sourceCommit), {
    tag_name: 'v0.1.2',
    target_commitish: sourceCommit,
    name: 'NexusOps v0.1.2',
    body,
    draft: true,
    prerelease: false,
    make_latest: 'false',
  });
});

test('draft request fails closed when committed release notes are missing', async (t) => {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'nexusops-release-notes-missing-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await assert.rejects(createDraftReleaseRequest('0.1.2', 'b'.repeat(40), root), /missing or unreadable/);
});

function run() {
  return {
    id: RUN_ID,
    path: '.github/workflows/signed-updater-candidate.yml',
    event: 'workflow_dispatch',
    status: 'completed',
    conclusion: 'success',
    head_sha: SHA,
  };
}

function artifacts() {
  return {
    artifacts: expectedCandidateArtifacts(VERSION, SHA).map((name, index) => ({
      id: index + 1,
      name,
      expired: false,
      size_in_bytes: 100,
      digest: `sha256:${String(index + 1).repeat(64).slice(0, 64)}`,
    })),
  };
}

const expectation = { runId: RUN_ID, sourceCommit: SHA, version: VERSION };

test('candidate run binds exact workflow, result, source and three artifacts', () => {
  assert.deepEqual(validateCandidateRun(run(), artifacts(), expectation), expectedCandidateArtifacts(VERSION, SHA));
});

test('candidate artifact names match the exact production workflow envelope', () => {
  assert.deepEqual(expectedCandidateArtifacts('0.1.1', PRODUCTION_SHAPE_SHA), [
    'NexusOps-updater-0.1.1-linux-x86_64-62fd0832',
    'NexusOps-updater-0.1.1-macos-aarch64-62fd0832',
    'NexusOps-updater-0.1.1-windows-x86_64-62fd0832',
  ]);
});

test('candidate run cannot be rebound to a different expected version', () => {
  assert.throws(() => validateCandidateRun(run(), artifacts(), { ...expectation, version: '0.1.2' }));
});

for (const [label, mutate] of [
  ['wrong workflow', (r) => { r.path = '.github/workflows/other.yml'; }],
  ['wrong event', (r) => { r.event = 'push'; }],
  ['failed result', (r) => { r.conclusion = 'failure'; }],
  ['wrong SHA', (r) => { r.head_sha = 'b'.repeat(40); }],
]) {
  test(`candidate run rejects ${label}`, () => {
    const value = run();
    mutate(value);
    assert.throws(() => validateCandidateRun(value, artifacts(), expectation));
  });
}

for (const [label, mutate] of [
  ['missing artifact', (a) => { a.artifacts.pop(); }],
  ['extra artifact', (a) => { a.artifacts.push({ ...a.artifacts[0], id: 99, name: 'extra' }); }],
  ['duplicate platform artifact', (a) => { a.artifacts[1].name = a.artifacts[0].name; }],
  ['missing updater prefix', (a) => { a.artifacts[0].name = a.artifacts[0].name.replace('NexusOps-updater-', 'NexusOps-'); }],
  ['generic NexusOps artifact name', (a) => { a.artifacts[0].name = 'NexusOps-0.1.1-linux-x86_64-aaaaaaaa'; }],
  ['wrong artifact version', (a) => { a.artifacts[0].name = a.artifacts[0].name.replace('0.1.1', '0.1.2'); }],
  ['wrong artifact platform', (a) => { a.artifacts[0].name = a.artifacts[0].name.replace('-linux-', '-freebsd-'); }],
  ['wrong artifact architecture', (a) => { a.artifacts[0].name = a.artifacts[0].name.replace('-x86_64-', '-armv7-'); }],
  ['wrong artifact short SHA', (a) => { a.artifacts[0].name = a.artifacts[0].name.replace('-aaaaaaaa', '-bbbbbbbb'); }],
  ['expired artifact', (a) => { a.artifacts[0].expired = true; }],
  ['bad digest', (a) => { a.artifacts[0].digest = 'sha256:no'; }],
]) {
  test(`candidate run rejects ${label}`, () => {
    const value = artifacts();
    mutate(value);
    assert.throws(() => validateCandidateRun(run(), value, expectation));
  });
}

test('immutability is a mandatory production gate', () => {
  assert.equal(validateImmutableReleaseStatus({ enabled: true }), true);
  assert.throws(() => validateImmutableReleaseStatus({ enabled: false }), /not enabled/);
  assert.throws(() => validateImmutableReleaseStatus({}), /not enabled/);
});

test('an existing lightweight tag is reusable only at the exact release source', () => {
  const tag = { ref: 'refs/tags/v0.1.1', object: { type: 'commit', sha: SHA } };
  assert.equal(validateTagRef(tag, 'v0.1.1', SHA), true);
  assert.throws(() => validateTagRef(tag, 'v0.1.1', 'b'.repeat(40)), /different source/);
  assert.throws(
    () => validateTagRef({ ...tag, object: { type: 'tag', sha: SHA } }, 'v0.1.1', SHA),
    /lightweight/,
  );
});

function release() {
  return {
    id: 42,
    tag_name: 'v0.1.1',
    target_commitish: SHA,
    draft: true,
    prerelease: false,
  };
}

const releaseExpectation = { releaseId: 42, tag: 'v0.1.1', sourceCommit: SHA, version: VERSION };

test('release state binds ID, tag, source and draft/published phase', () => {
  assert.equal(validateReleaseState(release(), releaseExpectation, 'draft'), true);
  const published = { ...release(), draft: false };
  assert.equal(validateReleaseState(published, releaseExpectation, 'published'), true);
});

test('first-release activation rejects another published release', () => {
  assert.equal(validateNoConflictingReleaseState([release()], 42, 'v0.1.1'), true);
  assert.throws(
    () => validateNoConflictingReleaseState([
      release(),
      { ...release(), id: 41, tag_name: 'v0.1.0', draft: false },
    ], 42, 'v0.1.1'),
    /another published release/,
  );
});

for (const [label, mutate] of [
  ['wrong release ID', (r) => { r.id = 43; }],
  ['wrong draft tag', (r) => { r.tag_name = 'v0.1.2'; }],
  ['wrong source', (r) => { r.target_commitish = 'b'.repeat(40); }],
  ['prerelease', (r) => { r.prerelease = true; }],
  ['published instead of draft', (r) => { r.draft = false; }],
]) {
  test(`draft release rejects ${label}`, () => {
    const value = release();
    mutate(value);
    assert.throws(() => validateReleaseState(value, releaseExpectation, 'draft'));
  });
}

function releaseAssets() {
  return releaseAssetAllowlist(VERSION).map((name, index) => ({
    id: index + 1,
    name,
    size: 10,
    state: 'uploaded',
    digest: `sha256:${String(index + 1).repeat(64).slice(0, 64)}`,
  }));
}

test('release assets require the exact seven-name allowlist', () => {
  assert.deepEqual(validateReleaseAssets(releaseAssets(), VERSION), releaseAssetAllowlist(VERSION));
  const missing = releaseAssets();
  missing.pop();
  assert.throws(() => validateReleaseAssets(missing, VERSION));
  const extra = releaseAssets();
  extra.push({ id: 99, name: 'release-manifest.json', size: 1, state: 'uploaded' });
  assert.throws(() => validateReleaseAssets(extra, VERSION));
  const existingBad = releaseAssets();
  existingBad[0].state = 'new';
  assert.throws(() => validateReleaseAssets(existingBad, VERSION));
});

test('downloaded draft/public assets must equal staged bytes', async (t) => {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'nexusops-publication-compare-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const left = join(root, 'left');
  const right = join(root, 'right');
  await mkdir(left);
  await mkdir(right);
  for (const name of releaseAssetAllowlist(VERSION)) {
    const content = name === 'latest.json' ? createLatestJson(VERSION, signatureMap()) : Buffer.from(name);
    await writeFile(join(left, name), content);
    await writeFile(join(right, name), content);
  }
  await comparePublicationDirectories(left, right, VERSION);
  await writeFile(join(right, 'NexusOps_0.1.1_x64-setup.exe'), 'changed');
  await assert.rejects(comparePublicationDirectories(left, right, VERSION), /differ/);
});

function signatureMap() {
  const result = {};
  for (const target of ['darwin-aarch64', 'linux-x86_64', 'windows-x86_64']) {
    const artifact = {
      'darwin-aarch64': 'NexusOps.app.tar.gz',
      'linux-x86_64': 'NexusOps_0.1.1_amd64.AppImage',
      'windows-x86_64': 'NexusOps_0.1.1_x64-setup.exe',
    }[target];
    const signed = Buffer.alloc(74, 3);
    signed.write('Ed', 0, 'ascii');
    const global = Buffer.alloc(64, 5);
    result[target] = Buffer.from([
      'untrusted comment: signature from tauri secret key', signed.toString('base64'),
      `trusted comment: timestamp:1700000000\tfile:${artifact}\tversion:0.1.1`, global.toString('base64'), '',
    ].join('\n')).toString('base64');
  }
  return result;
}

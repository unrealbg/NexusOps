import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { preparePublicationSet } from './prepare-publication-set.mjs';
import { expectedCandidateArtifacts } from './release-publication-common.mjs';
import { createSignedCandidate } from './signed-updater-candidate.mjs';
import { verifyUpdaterPublicKey } from './updater-public-key.mjs';
import { verifyLatestJson } from './latest-json.mjs';
import { REPOSITORY_ROOT } from './release-common.mjs';

const SHA = 'a'.repeat(40);
const VERSION = '0.1.1';
const RUN_ID = 123;

function signature(artifact) {
  const signed = Buffer.alloc(74, 3);
  signed.write('Ed', 0, 'ascii');
  const global = Buffer.alloc(64, 5);
  return Buffer.from(Buffer.from([
    'untrusted comment: signature from tauri secret key', signed.toString('base64'),
    `trusted comment: timestamp:1700000000\tfile:${artifact}\tversion:${VERSION}`,
    global.toString('base64'), '',
  ].join('\n')).toString('base64'));
}

async function fixture(t) {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'nexusops-publication-set-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const candidates = join(root, 'candidates');
  const output = join(root, 'publication');
  await mkdir(candidates);
  const key = await verifyUpdaterPublicKey(REPOSITORY_ROOT);
  const identities = [
    { platform: 'macos', architecture: 'aarch64', artifact: 'NexusOps.app.tar.gz' },
    { platform: 'linux', architecture: 'x86_64', artifact: 'NexusOps_0.1.1_amd64.AppImage' },
    { platform: 'windows', architecture: 'x86_64', artifact: 'NexusOps_0.1.1_x64-setup.exe' },
  ];
  const names = expectedCandidateArtifacts(VERSION, SHA);
  for (const identity of identities) {
    const name = names.find((value) => value.includes(`-${identity.platform}-${identity.architecture}-`));
    const directory = join(candidates, name);
    await mkdir(directory);
    await writeFile(join(directory, identity.artifact), `payload:${identity.platform}`);
    await writeFile(join(directory, `${identity.artifact}.sig`), signature(identity.artifact));
    await createSignedCandidate(
      directory,
      {
        productVersion: VERSION,
        sourceCommit: SHA,
        platform: identity.platform,
        architecture: identity.architecture,
      },
      key.sha256,
    );
  }
  const runJson = join(root, 'run.json');
  const artifactsJson = join(root, 'artifacts.json');
  await writeFile(runJson, JSON.stringify({
    id: RUN_ID,
    path: '.github/workflows/signed-updater-candidate.yml',
    event: 'workflow_dispatch',
    status: 'completed',
    conclusion: 'success',
    head_sha: SHA,
  }));
  await writeFile(artifactsJson, JSON.stringify({
    artifacts: names.map((name, index) => ({
      id: index + 1, name, expired: false, size_in_bytes: 100,
      digest: `sha256:${String(index + 1).repeat(64).slice(0, 64)}`,
    })),
  }));
  return {
    values: {
      'candidate-root': candidates,
      output,
      'run-json': runJson,
      'artifacts-json': artifactsJson,
      'run-id': String(RUN_ID),
      'expected-source-sha': SHA,
      'expected-version': VERSION,
    },
    output,
    artifactsJson,
  };
}

test('one exact successful candidate run becomes seven verified publication files without rebuild', async (t) => {
  const { values, output } = await fixture(t);
  let cryptoCalls = 0;
  const result = await preparePublicationSet(values, () => { cryptoCalls += 1; });
  assert.equal(cryptoCalls, 3);
  assert.equal(result.assets.length, 7);
  verifyLatestJson(await readFile(join(output, 'latest.json')), VERSION);
});

test('candidate artifact substitution is rejected before publication staging', async (t) => {
  const { values, artifactsJson } = await fixture(t);
  const data = JSON.parse(await readFile(artifactsJson, 'utf8'));
  data.artifacts[0].name = 'substituted';
  await writeFile(artifactsJson, JSON.stringify(data));
  await assert.rejects(preparePublicationSet(values, () => {}), /artifacts/);
});

test('cryptographic failure rejects the publication set', async (t) => {
  const { values } = await fixture(t);
  await assert.rejects(
    preparePublicationSet(values, () => { throw new Error('cryptographic signature verification failed'); }),
    /cryptographic signature verification failed/,
  );
});

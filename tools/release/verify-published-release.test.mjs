import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { createLatestJson, releaseAssetAllowlist } from './latest-json.mjs';
import { verifyPublishedRelease } from './verify-published-release.mjs';

const VERSION = '0.1.1';
const SHA = 'a'.repeat(40);
const TAG = 'v0.1.1';

function signature(artifact) {
  const signed = Buffer.alloc(74, 3);
  signed.write('Ed', 0, 'ascii');
  const global = Buffer.alloc(64, 5);
  return Buffer.from([
    'untrusted comment: signature from tauri secret key', signed.toString('base64'),
    `trusted comment: timestamp:1700000000\tfile:${artifact}\tversion:${VERSION}`,
    global.toString('base64'), '',
  ].join('\n')).toString('base64');
}

async function fixture(t) {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'nexusops-published-release-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const stage = join(root, 'stage');
  await mkdir(stage);
  const artifacts = {
    'NexusOps.app.tar.gz': Buffer.from('mac payload'),
    'NexusOps_0.1.1_amd64.AppImage': Buffer.from('linux payload'),
    'NexusOps_0.1.1_x64-setup.exe': Buffer.from('windows payload'),
  };
  const signatures = {
    'darwin-aarch64': signature('NexusOps.app.tar.gz'),
    'linux-x86_64': signature('NexusOps_0.1.1_amd64.AppImage'),
    'windows-x86_64': signature('NexusOps_0.1.1_x64-setup.exe'),
  };
  for (const [name, data] of Object.entries(artifacts)) {
    await writeFile(join(stage, name), data);
    const target = name.endsWith('.exe') ? 'windows-x86_64' : name.endsWith('.AppImage') ? 'linux-x86_64' : 'darwin-aarch64';
    await writeFile(join(stage, `${name}.sig`), signatures[target]);
  }
  await writeFile(join(stage, 'latest.json'), createLatestJson(VERSION, signatures));
  const assetRecords = [];
  for (const [index, name] of releaseAssetAllowlist(VERSION).entries()) {
    assetRecords.push({
      id: index + 1,
      name,
      size: (await readFile(join(stage, name))).length,
      state: 'uploaded',
      digest: null,
      browser_download_url: `https://github.com/unrealbg/NexusOps/releases/download/${TAG}/${name}`,
    });
  }
  const release = {
    id: 42,
    tag_name: TAG,
    target_commitish: SHA,
    draft: false,
    prerelease: false,
    immutable: true,
    assets: assetRecords,
  };
  return { stage, release };
}

function response(url, body, isJson = true) {
  return {
    ok: true,
    status: 200,
    url,
    async json() { return body; },
    async arrayBuffer() {
      const value = Buffer.isBuffer(body) ? body : Buffer.from(body);
      return value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength);
    },
  };
}

function fetcher(stage, release, latestOverride) {
  return async (url) => {
    if (url.endsWith('/releases/42') || url.endsWith('/releases/latest')) return response(url, release);
    if (url.endsWith(`/git/ref/tags/${TAG}`))
      return response(url, { ref: `refs/tags/${TAG}`, object: { type: 'commit', sha: SHA } });
    if (url.endsWith('/immutable-releases')) throw new Error('admin-only immutable settings endpoint was called');
    if (url.endsWith('/releases/latest/download/latest.json'))
      return response(url, latestOverride ?? await readFile(join(stage, 'latest.json')), false);
    const asset = release.assets.find((entry) => entry.browser_download_url === url);
    if (asset) return response(url, await readFile(join(stage, asset.name)), false);
    throw new Error(`unexpected URL ${url}`);
  };
}

test('post-publication verifier rechecks exact public release, manifest and all seven bytes', async (t) => {
  const { stage, release } = await fixture(t);
  const result = await verifyPublishedRelease(
    { stage, releaseId: 42, expectedTag: TAG, expectedSourceSha: SHA, expectedVersion: VERSION },
    { fetcher: fetcher(stage, release), token: 'synthetic' },
  );
  assert.equal(result.assets.length, 7);
});

test('post-publication manifest mismatch stops without repair', async (t) => {
  const { stage, release } = await fixture(t);
  const changed = Buffer.from((await readFile(join(stage, 'latest.json'), 'utf8')).replace('0.1.1', '0.1.2'));
  await assert.rejects(
    verifyPublishedRelease(
      { stage, releaseId: 42, expectedTag: TAG, expectedSourceSha: SHA, expectedVersion: VERSION },
      { fetcher: fetcher(stage, release, changed) },
    ),
  );
});

test('post-publication verifier fails unless the exact release reports immutable true', async (t) => {
  const { stage, release } = await fixture(t);
  const mutableRelease = { ...release };
  delete mutableRelease.immutable;
  await assert.rejects(
    verifyPublishedRelease(
      { stage, releaseId: 42, expectedTag: TAG, expectedSourceSha: SHA, expectedVersion: VERSION },
      { fetcher: fetcher(stage, mutableRelease) },
    ),
    /does not report immutable state/,
  );
});

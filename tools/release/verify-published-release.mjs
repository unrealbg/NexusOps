import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readReleaseSignatures, releaseAssetAllowlist, verifyLatestJson } from './latest-json.mjs';
import {
  REPOSITORY,
  hashPublicationDirectory,
  validateReleaseAssets,
  validateReleaseState,
  validateTagRef,
} from './release-publication-common.mjs';

const MAX_ATTEMPTS = 4;
const OVERALL_TIMEOUT_MS = 300_000;
const REDIRECT_HOSTS = new Set([
  'api.github.com',
  'github.com',
  'release-assets.githubusercontent.com',
  'objects.githubusercontent.com',
]);

function fail(message) {
  throw new Error(message);
}

function parse(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    if (!argv[index]?.startsWith('--') || argv[index + 1] === undefined) fail('invalid arguments');
    values[argv[index].slice(2)] = argv[index + 1];
  }
  for (const key of ['stage', 'release-id', 'expected-tag', 'expected-source-sha', 'expected-version'])
    if (!values[key]) fail(`missing --${key}`);
  return values;
}

async function fetchWithBound(url, options, deadline, fetcher) {
  let last;
  for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt += 1) {
    const remaining = deadline - Date.now();
    if (remaining <= 0) break;
    try {
      const response = await fetcher(url, {
        ...options,
        redirect: 'follow',
        signal: AbortSignal.timeout(Math.min(60_000, remaining)),
      });
      if (response.ok) {
        const final = new URL(response.url);
        if (!REDIRECT_HOSTS.has(final.hostname)) fail('public download redirected outside GitHub asset hosts');
        return response;
      }
      last = new Error(`HTTP ${response.status}`);
    } catch (error) {
      last = error;
    }
    if (attempt < MAX_ATTEMPTS) await new Promise((done) => setTimeout(done, 3_000));
  }
  fail(`bounded public verification request failed: ${last?.message ?? 'timeout'}`);
}

async function json(url, headers, deadline, fetcher) {
  const response = await fetchWithBound(url, { headers }, deadline, fetcher);
  return response.json();
}

export async function verifyPublishedRelease(
  { stage, releaseId, expectedTag, expectedSourceSha, expectedVersion },
  { fetcher = fetch, token = process.env.GH_TOKEN } = {},
) {
  if (!Number.isSafeInteger(releaseId) || releaseId <= 0) fail('release ID is invalid');
  if (expectedTag !== `v${expectedVersion}`) fail('expected tag differs from version');
  const headers = { Accept: 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28' };
  if (token) headers.Authorization = `Bearer ${token}`;
  const deadline = Date.now() + OVERALL_TIMEOUT_MS;
  const api = 'https://api.github.com/repos/unrealbg/NexusOps';
  const [release, latest, tagRef] = await Promise.all([
    json(`${api}/releases/${releaseId}`, headers, deadline, fetcher),
    json(`${api}/releases/latest`, headers, deadline, fetcher),
    json(`${api}/git/ref/tags/${expectedTag}`, headers, deadline, fetcher),
  ]);
  const expected = { releaseId, tag: expectedTag, sourceCommit: expectedSourceSha, version: expectedVersion };
  validateReleaseState(release, expected, 'published');
  validateReleaseState(latest, expected, 'published');
  validateTagRef(tagRef, expectedTag, expectedSourceSha);
  if (release.immutable !== true)
    fail('published release does not report immutable state');
  validateReleaseAssets(release.assets, expectedVersion);

  const local = await hashPublicationDirectory(resolve(stage), expectedVersion);
  const signatures = await readReleaseSignatures(resolve(stage), expectedVersion);
  const latestResponse = await fetchWithBound(
    'https://github.com/unrealbg/NexusOps/releases/latest/download/latest.json',
    {},
    deadline,
    fetcher,
  );
  const latestBytes = Buffer.from(await latestResponse.arrayBuffer());
  verifyLatestJson(latestBytes, expectedVersion, signatures);
  if (!latestBytes.equals(await readFile(join(stage, 'latest.json'))))
    fail('public latest.json bytes differ from verified draft bytes');

  for (const name of releaseAssetAllowlist(expectedVersion)) {
    const asset = release.assets.find((entry) => entry.name === name);
    const response = await fetchWithBound(asset.browser_download_url, {}, deadline, fetcher);
    const data = Buffer.from(await response.arrayBuffer());
    if (data.length !== local[name].bytes) fail(`public ${name} size differs`);
    if (createHash('sha256').update(data).digest('hex') !== local[name].sha256)
      fail(`public ${name} SHA-256 differs`);
  }
  return { repository: REPOSITORY, releaseId, tag: expectedTag, assets: releaseAssetAllowlist(expectedVersion) };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const values = parse(process.argv.slice(2));
    const result = await verifyPublishedRelease({
      stage: values.stage,
      releaseId: Number(values['release-id']),
      expectedTag: values['expected-tag'],
      expectedSourceSha: values['expected-source-sha'],
      expectedVersion: values['expected-version'],
    });
    console.log(`Verified published updater release: ${JSON.stringify(result)}`);
  } catch (error) {
    console.error(`Post-publication verification failed: ${error.message}`);
    process.exitCode = 1;
  }
}

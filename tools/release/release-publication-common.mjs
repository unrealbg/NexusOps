import { readFile, readdir } from 'node:fs/promises';
import { join } from 'node:path';
import {
  PRODUCT_NAME,
  assertOrdinaryPath,
  assertSemver,
  exactKeys,
  hashOrdinaryFile,
  releaseArtifactName,
  stagedFileNames,
} from './release-common.mjs';
import { RELEASE_TARGETS, releaseAssetAllowlist, verifyLatestJson } from './latest-json.mjs';

export const REPOSITORY = 'unrealbg/NexusOps';
export const SIGNED_WORKFLOW_PATH = '.github/workflows/signed-updater-candidate.yml';

function fail(message) {
  throw new Error(message);
}

export function expectedCandidateArtifacts(version, sourceCommit) {
  assertSemver(version, 'expected version');
  if (!/^[0-9a-f]{40}$/.test(sourceCommit)) fail('expected source SHA is invalid');
  return RELEASE_TARGETS.map(({ platform, architecture }) =>
    releaseArtifactName({ productVersion: version, platform, architecture, sourceCommit }),
  ).sort();
}

export function validateCandidateRun(run, artifacts, expected) {
  exactKeys(expected, ['runId', 'sourceCommit', 'version'], 'candidate expectation');
  if (!Number.isSafeInteger(expected.runId) || expected.runId <= 0) fail('candidate run ID is invalid');
  assertSemver(expected.version, 'expected version');
  if (!/^[0-9a-f]{40}$/.test(expected.sourceCommit)) fail('expected source SHA is invalid');
  if (!run || typeof run !== 'object' || Array.isArray(run)) fail('candidate run is invalid');
  if (
    run.id !== expected.runId ||
    run.path !== SIGNED_WORKFLOW_PATH ||
    run.event !== 'workflow_dispatch' ||
    run.status !== 'completed' ||
    run.conclusion !== 'success' ||
    run.head_sha !== expected.sourceCommit
  ) {
    fail('candidate workflow run identity or result differs');
  }
  const list = artifacts?.artifacts;
  if (!Array.isArray(list)) fail('candidate artifact response is invalid');
  const expectedNames = expectedCandidateArtifacts(expected.version, expected.sourceCommit);
  const names = list.map((entry) => entry?.name).sort();
  if (
    names.length !== expectedNames.length ||
    names.some((name, index) => name !== expectedNames[index])
  ) {
    fail('candidate workflow artifacts are missing, duplicated or unexpected');
  }
  for (const entry of list) {
    if (
      entry.expired !== false ||
      !Number.isSafeInteger(entry.id) ||
      entry.id <= 0 ||
      !Number.isSafeInteger(entry.size_in_bytes) ||
      entry.size_in_bytes <= 0
    ) {
      fail('candidate workflow artifact metadata is invalid');
    }
    if (entry.digest !== undefined && !/^sha256:[0-9a-f]{64}$/.test(entry.digest))
      fail('candidate workflow artifact digest is invalid');
  }
  return expectedNames;
}

export function validateImmutableReleaseStatus(status) {
  if (!status || status.enabled !== true) fail('repository immutable releases are not enabled');
  return true;
}

export function validateTagRef(tagRef, expectedTag, expectedSourceCommit) {
  if (tagRef?.ref !== `refs/tags/${expectedTag}` || tagRef?.object?.type !== 'commit')
    fail('release tag is missing or is not a lightweight commit ref');
  if (tagRef.object.sha !== expectedSourceCommit) fail('release tag points to a different source commit');
  return true;
}

export function validateNoConflictingReleaseState(releases, expectedReleaseId, expectedTag) {
  if (!Array.isArray(releases)) fail('repository release list is invalid');
  const sameTag = releases.filter((release) => release?.tag_name === expectedTag);
  if (sameTag.length !== 1 || sameTag[0].id !== expectedReleaseId)
    fail('release tag is missing, duplicated or bound to another release');
  if (releases.some((release) => release?.id !== expectedReleaseId && release?.draft === false))
    fail('another published release makes first-release activation ambiguous');
  return true;
}

export function validateReleaseState(release, expected, phase) {
  exactKeys(expected, ['releaseId', 'tag', 'sourceCommit', 'version'], 'release expectation');
  if (!Number.isSafeInteger(expected.releaseId) || expected.releaseId <= 0) fail('release ID is invalid');
  if (expected.tag !== `v${expected.version}`) fail('release tag differs from expected version');
  if (!/^[0-9a-f]{40}$/.test(expected.sourceCommit)) fail('release source SHA is invalid');
  if (!release || typeof release !== 'object' || Array.isArray(release)) fail('release state is invalid');
  if (
    release.id !== expected.releaseId ||
    release.tag_name !== expected.tag ||
    release.target_commitish !== expected.sourceCommit ||
    release.prerelease !== false
  ) {
    fail('release identity differs');
  }
  if (phase === 'draft' && release.draft !== true) fail('release is not the expected draft');
  if (phase === 'published' && release.draft !== false) fail('release is not published');
  if (!['draft', 'published'].includes(phase)) fail('release phase is invalid');
  return true;
}

export function validateReleaseAssets(assets, version) {
  if (!Array.isArray(assets)) fail('release assets are invalid');
  const expected = releaseAssetAllowlist(version);
  const names = assets.map((asset) => asset?.name).sort();
  if (names.length !== expected.length || names.some((name, index) => name !== expected[index]))
    fail('release assets differ from the exact allowlist');
  for (const asset of assets) {
    if (
      !Number.isSafeInteger(asset.id) ||
      asset.id <= 0 ||
      !Number.isSafeInteger(asset.size) ||
      asset.size <= 0 ||
      asset.state !== 'uploaded'
    ) {
      fail('release asset metadata is invalid');
    }
    if (asset.digest !== undefined && asset.digest !== null && !/^sha256:[0-9a-f]{64}$/.test(asset.digest))
      fail('release asset digest is invalid');
  }
  return expected;
}

export async function hashPublicationDirectory(directory, version) {
  const names = await stagedFileNames(directory);
  const expected = releaseAssetAllowlist(version);
  if (names.length !== expected.length || names.some((name, index) => name !== expected[index]))
    fail('publication directory differs from exact release asset allowlist');
  const result = {};
  for (const name of names) result[name] = await hashOrdinaryFile(join(directory, name));
  return result;
}

export async function comparePublicationDirectories(expectedDirectory, actualDirectory, version) {
  const left = await hashPublicationDirectory(expectedDirectory, version);
  const right = await hashPublicationDirectory(actualDirectory, version);
  if (JSON.stringify(left) !== JSON.stringify(right)) fail('downloaded release assets differ from staged bytes');
  return left;
}

export async function validateReleaseNotes(root, version, releaseBody) {
  const path = join(root, 'docs/release/notes', `v${version}.md`);
  await assertOrdinaryPath(path, 'file');
  const expected = await readFile(path, 'utf8');
  if (releaseBody !== undefined && releaseBody !== expected) fail('release body differs from committed notes');
  return expected;
}

export async function verifyPublicationDirectory(directory, version, expectedSignatures) {
  const hashes = await hashPublicationDirectory(directory, version);
  verifyLatestJson(await readFile(join(directory, 'latest.json')), version, expectedSignatures);
  return hashes;
}

export async function assertExactCandidateDirectories(root, expectedNames) {
  const entries = (await readdir(root, { withFileTypes: true })).sort((a, b) =>
    a.name.localeCompare(b.name),
  );
  if (
    entries.length !== expectedNames.length ||
    entries.some((entry, index) => !entry.isDirectory() || entry.name !== expectedNames[index])
  ) {
    fail('downloaded candidate directories differ from the verified workflow artifacts');
  }
}

export function deterministicReleaseTitle(version) {
  assertSemver(version, 'release version');
  return `${PRODUCT_NAME} v${version}`;
}

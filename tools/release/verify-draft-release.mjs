import { readFile } from 'node:fs/promises';
import { isAbsolute, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { REPOSITORY_ROOT } from './release-common.mjs';
import { readReleaseSignatures } from './latest-json.mjs';
import {
  comparePublicationDirectories,
  validateImmutableReleaseStatus,
  validateNoConflictingReleaseState,
  validateReleaseAssets,
  validateReleaseNotes,
  validateReleaseState,
  validateTagRef,
  verifyPublicationDirectory,
} from './release-publication-common.mjs';
import { verifyReleaseCryptography } from './updater-crypto.mjs';

function parse(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    if (!argv[index]?.startsWith('--') || argv[index + 1] === undefined) throw new Error('invalid arguments');
    values[argv[index].slice(2)] = argv[index + 1];
  }
  const required = ['release-json', 'tag-json', 'releases-json', 'downloaded', 'release-id', 'expected-source-sha', 'expected-version'];
  if (required.some((key) => !values[key])) throw new Error('missing draft verification argument');
  for (const key of ['release-json', 'tag-json', 'releases-json', 'downloaded'])
    if (!isAbsolute(values[key])) throw new Error(`${key} must be absolute`);
  if (values.stage && !isAbsolute(values.stage)) throw new Error('stage must be absolute');
  if (values['immutable-json'] && !isAbsolute(values['immutable-json']))
    throw new Error('immutable-json must be absolute');
  return values;
}

export async function verifyDraftRelease(values, cryptoRunner) {
  const version = values['expected-version'];
  const sourceCommit = values['expected-source-sha'];
  const releaseId = Number(values['release-id']);
  const tag = `v${version}`;
  const release = JSON.parse(await readFile(values['release-json'], 'utf8'));
  const tagRef = JSON.parse(await readFile(values['tag-json'], 'utf8'));
  const releases = JSON.parse(await readFile(values['releases-json'], 'utf8'));
  validateReleaseState(release, { releaseId, tag, sourceCommit, version }, 'draft');
  validateTagRef(tagRef, tag, sourceCommit);
  validateNoConflictingReleaseState(releases, releaseId, tag);
  validateReleaseAssets(release.assets, version);
  await validateReleaseNotes(REPOSITORY_ROOT, version, release.body);
  const downloaded = resolve(values.downloaded);
  const signatures = await readReleaseSignatures(downloaded, version);
  await verifyPublicationDirectory(downloaded, version, signatures);
  await verifyReleaseCryptography(downloaded, version, cryptoRunner);
  if (values.stage) await comparePublicationDirectories(resolve(values.stage), downloaded, version);
  if (values['immutable-json'])
    validateImmutableReleaseStatus(JSON.parse(await readFile(values['immutable-json'], 'utf8')));
  return { releaseId, tag, sourceCommit, version };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const result = await verifyDraftRelease(parse(process.argv.slice(2)));
    console.log(`Verified updater release draft: ${JSON.stringify(result)}`);
  } catch (error) {
    console.error(`Draft release verification failed: ${error.message}`);
    process.exitCode = 1;
  }
}

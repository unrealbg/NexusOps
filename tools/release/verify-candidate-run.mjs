import { readFile } from 'node:fs/promises';
import { isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { REPOSITORY_ROOT, hashOrdinaryFile } from './release-common.mjs';
import { RELEASE_TARGETS, readReleaseSignatures, releaseArtifactBasename, verifyLatestJson } from './latest-json.mjs';
import {
  assertExactCandidateDirectories,
  expectedCandidateArtifacts,
  validateCandidateRun,
  verifyPublicationDirectory,
} from './release-publication-common.mjs';
import { verifySignedCandidate } from './signed-updater-candidate.mjs';
import { verifyUpdaterPublicKey } from './updater-public-key.mjs';
import { verifyUpdaterCryptography } from './updater-crypto.mjs';

function parse(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    if (!argv[index]?.startsWith('--') || argv[index + 1] === undefined) throw new Error('invalid arguments');
    values[argv[index].slice(2)] = argv[index + 1];
  }
  const required = ['candidate-root', 'publication', 'run-json', 'artifacts-json', 'run-id', 'expected-source-sha', 'expected-version'];
  if (required.some((key) => !values[key])) throw new Error('missing candidate-run verification argument');
  for (const key of ['candidate-root', 'publication', 'run-json', 'artifacts-json'])
    if (!isAbsolute(values[key])) throw new Error(`${key} must be absolute`);
  return values;
}

export async function verifyCandidateRun(values, cryptoRunner) {
  const candidateRoot = resolve(values['candidate-root']);
  const publication = resolve(values.publication);
  const runId = Number(values['run-id']);
  const sourceCommit = values['expected-source-sha'];
  const version = values['expected-version'];
  const run = JSON.parse(await readFile(values['run-json'], 'utf8'));
  const artifacts = JSON.parse(await readFile(values['artifacts-json'], 'utf8'));
  const names = validateCandidateRun(run, artifacts, { runId, sourceCommit, version });
  await assertExactCandidateDirectories(candidateRoot, names);
  const key = await verifyUpdaterPublicKey(REPOSITORY_ROOT);
  const signatures = {};
  for (const identity of RELEASE_TARGETS) {
    const candidateName = expectedCandidateArtifacts(version, sourceCommit).find((name) =>
      name.includes(`-${identity.platform}-${identity.architecture}-`),
    );
    const directory = join(candidateRoot, candidateName);
    const expectedIdentity = {
      productVersion: version,
      sourceCommit,
      platform: identity.platform,
      architecture: identity.architecture,
    };
    const metadata = await verifySignedCandidate(directory, expectedIdentity, key.sha256);
    const artifactName = releaseArtifactBasename(identity.target, version);
    const artifact = join(directory, artifactName);
    const signature = `${artifact}.sig`;
    const publishedArtifact = await hashOrdinaryFile(join(publication, artifactName));
    const publishedSignature = await hashOrdinaryFile(join(publication, `${artifactName}.sig`));
    if (
      metadata.artifact.sha256 !== publishedArtifact.sha256 ||
      metadata.artifact.bytes !== publishedArtifact.bytes ||
      metadata.signature.sha256 !== publishedSignature.sha256
    ) {
      throw new Error('draft bytes differ from signed candidate metadata');
    }
    signatures[identity.target] = await readFile(signature, 'utf8');
    await verifyUpdaterCryptography(artifact, signature, version, cryptoRunner);
  }
  await verifyPublicationDirectory(publication, version, signatures);
  verifyLatestJson(await readFile(join(publication, 'latest.json')), version, signatures);
  const flatSignatures = await readReleaseSignatures(publication, version);
  if (JSON.stringify(flatSignatures) !== JSON.stringify(signatures))
    throw new Error('draft signatures differ from signed candidate bytes');
  return { runId, sourceCommit, version };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const result = await verifyCandidateRun(parse(process.argv.slice(2)));
    console.log(`Verified signed candidate run against draft bytes: ${JSON.stringify(result)}`);
  } catch (error) {
    console.error(`Candidate-run verification failed: ${error.message}`);
    process.exitCode = 1;
  }
}

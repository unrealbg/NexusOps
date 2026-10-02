import { copyFile, mkdir, readFile, writeFile } from 'node:fs/promises';
import { constants } from 'node:fs';
import { isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { REPOSITORY_ROOT, assertOrdinaryPath, stagedFileNames } from './release-common.mjs';
import { createLatestJson, readReleaseSignatures, releaseArtifactBasename } from './latest-json.mjs';
import {
  assertExactCandidateDirectories,
  expectedCandidateArtifacts,
  validateCandidateRun,
  verifyPublicationDirectory,
} from './release-publication-common.mjs';
import { verifySignedCandidate } from './signed-updater-candidate.mjs';
import { verifyUpdaterPublicKey } from './updater-public-key.mjs';
import { verifyReleaseCryptography } from './updater-crypto.mjs';

function parse(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    if (!argv[index]?.startsWith('--') || argv[index + 1] === undefined) throw new Error('invalid arguments');
    values[argv[index].slice(2)] = argv[index + 1];
  }
  const required = ['candidate-root', 'output', 'run-json', 'artifacts-json', 'run-id', 'expected-source-sha', 'expected-version'];
  if (Object.keys(values).length !== required.length || required.some((key) => !values[key]))
    throw new Error(`required arguments: ${required.map((key) => `--${key}`).join(' ')}`);
  for (const key of ['candidate-root', 'output', 'run-json', 'artifacts-json'])
    if (!isAbsolute(values[key])) throw new Error(`${key} must be absolute`);
  return values;
}

export async function preparePublicationSet(values, cryptoRunner) {
  const candidateRoot = resolve(values['candidate-root']);
  const output = resolve(values.output);
  await assertOrdinaryPath(candidateRoot, 'directory');
  await mkdir(output, { recursive: false });
  await assertOrdinaryPath(output, 'directory');
  if ((await stagedFileNames(output)).length !== 0) throw new Error('publication output must start empty');
  const runId = Number(values['run-id']);
  const sourceCommit = values['expected-source-sha'];
  const version = values['expected-version'];
  const run = JSON.parse(await readFile(values['run-json'], 'utf8'));
  const artifacts = JSON.parse(await readFile(values['artifacts-json'], 'utf8'));
  const expectedNames = validateCandidateRun(run, artifacts, { runId, sourceCommit, version });
  await assertExactCandidateDirectories(candidateRoot, expectedNames);
  const key = await verifyUpdaterPublicKey(REPOSITORY_ROOT);
  const identities = [
    { platform: 'macos', architecture: 'aarch64', target: 'darwin-aarch64' },
    { platform: 'linux', architecture: 'x86_64', target: 'linux-x86_64' },
    { platform: 'windows', architecture: 'x86_64', target: 'windows-x86_64' },
  ];
  for (const identity of identities) {
    const candidateName = expectedCandidateArtifacts(version, sourceCommit).find((name) =>
      name.includes(`-${identity.platform}-${identity.architecture}-`),
    );
    const candidateDirectory = join(candidateRoot, candidateName);
    const metadata = await verifySignedCandidate(
      candidateDirectory,
      { productVersion: version, sourceCommit, platform: identity.platform, architecture: identity.architecture },
      key.sha256,
    );
    const payload = releaseArtifactBasename(identity.target, version);
    if (metadata.artifact.basename !== payload) throw new Error('candidate payload differs from target mapping');
    await copyFile(join(candidateDirectory, payload), join(output, payload), constants.COPYFILE_EXCL);
    await copyFile(join(candidateDirectory, `${payload}.sig`), join(output, `${payload}.sig`), constants.COPYFILE_EXCL);
  }
  const signatures = await readReleaseSignatures(output, version);
  const latest = createLatestJson(version, signatures);
  await writeFile(join(output, 'latest.json'), latest, { flag: 'wx' });
  await verifyPublicationDirectory(output, version, signatures);
  await verifyReleaseCryptography(output, version, cryptoRunner);
  return { version, sourceCommit, assets: await stagedFileNames(output) };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const result = await preparePublicationSet(parse(process.argv.slice(2)));
    console.log(`Prepared verified publication set: ${JSON.stringify(result)}`);
  } catch (error) {
    console.error(`Publication-set preparation failed: ${error.message}`);
    process.exitCode = 1;
  }
}

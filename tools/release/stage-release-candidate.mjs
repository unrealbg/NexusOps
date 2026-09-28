import { constants } from 'node:fs';
import { appendFile, copyFile, mkdir } from 'node:fs/promises';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  REPOSITORY_ROOT,
  assertOrdinaryPath,
  detectPlatformArchitecture,
  expectedExecutable,
  hashOrdinaryFile,
  parseStageArgument,
  releaseArtifactName,
  stagedFileNames,
  verifyReleaseSource,
  verifyReleaseVersion,
} from './release-common.mjs';

function fail(message) {
  throw new Error(message);
}

export async function stageReleaseCandidate(
  stage,
  root = REPOSITORY_ROOT,
  environment = process.env,
) {
  if (!isAbsolute(stage)) fail('staging directory must be absolute');
  const sourceCommit = verifyReleaseSource(root, environment);
  const { productVersion } = await verifyReleaseVersion(root);
  const { platform, architecture } = detectPlatformArchitecture();
  const executableName = expectedExecutable(platform);
  const targetRoot = environment.CARGO_TARGET_DIR
    ? resolve(root, environment.CARGO_TARGET_DIR)
    : join(root, 'target');
  const buildPath = join(targetRoot, 'release', executableName);
  await assertOrdinaryPath(buildPath, 'file');
  await assertOrdinaryPath(dirname(stage), 'directory');
  await mkdir(stage, { recursive: false });
  await assertOrdinaryPath(stage, 'directory');
  const stagedPath = join(stage, executableName);
  await copyFile(buildPath, stagedPath, constants.COPYFILE_EXCL);
  const [sourceDigest, stagedDigest] = await Promise.all([
    hashOrdinaryFile(buildPath),
    hashOrdinaryFile(stagedPath),
  ]);
  if (sourceDigest.bytes !== stagedDigest.bytes || sourceDigest.sha256 !== stagedDigest.sha256) {
    fail('staged executable differs from production build');
  }
  const names = await stagedFileNames(stage);
  if (names.length !== 1 || names[0] !== executableName)
    fail('staging contains an unexpected entry');
  const artifactName = releaseArtifactName({
    productVersion,
    platform,
    architecture,
    sourceCommit,
  });
  return {
    stage,
    stagedPath,
    executableName,
    artifactName,
    platform,
    architecture,
    productVersion,
    sourceCommit,
  };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const stage = parseStageArgument(process.argv.slice(2));
    const result = await stageReleaseCandidate(stage);
    if (process.env.GITHUB_OUTPUT) {
      for (const value of [result.stage, result.stagedPath, result.artifactName]) {
        if (/[\r\n]/.test(value)) fail('workflow output contains a newline');
      }
      await appendFile(
        process.env.GITHUB_OUTPUT,
        [
          `stage_dir=${result.stage}`,
          `binary_path=${result.stagedPath}`,
          `artifact_name=${result.artifactName}`,
          '',
        ].join('\n'),
      );
    }
    console.log(`Staged one production executable: ${result.artifactName}`);
  } catch (error) {
    console.error(`Release staging failed: ${error.message}`);
    process.exitCode = 1;
  }
}

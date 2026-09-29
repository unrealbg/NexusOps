import { constants } from 'node:fs';
import { appendFile, copyFile, mkdir, readFile, readdir } from 'node:fs/promises';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  REPOSITORY_ROOT,
  assertOrdinaryPath,
  detectPlatformArchitecture,
  hashOrdinaryFile,
  parseStageArgument,
  stagedFileNames,
  verifyReleaseSource,
  verifyReleaseVersion,
} from './release-common.mjs';
import { updaterArtifactBasename } from './signed-updater-candidate.mjs';
import { inspectUpdaterSignature } from './updater-signature.mjs';

function fail(message) {
  throw new Error(message);
}

export async function stageSignedUpdaterCandidate(
  stage,
  root = REPOSITORY_ROOT,
  environment = process.env,
) {
  if (!isAbsolute(stage)) fail('staging directory must be absolute');
  const sourceCommit = verifyReleaseSource(root, environment);
  const { productVersion } = await verifyReleaseVersion(root);
  const { platform, architecture } = detectPlatformArchitecture();
  const identity = { productVersion, sourceCommit, platform, architecture };
  const targetRoot = environment.CARGO_TARGET_DIR
    ? resolve(root, environment.CARGO_TARGET_DIR)
    : join(root, 'target');
  const bundleFolder = { windows: 'nsis', linux: 'appimage', macos: 'macos' }[platform];
  const bundleDir = join(targetRoot, 'release', 'bundle', bundleFolder);
  await assertOrdinaryPath(bundleDir, 'directory');
  const artifacts = (await readdir(bundleDir)).filter((name) => {
    try {
      updaterArtifactBasename(name, identity);
      return true;
    } catch {
      return false;
    }
  });
  if (artifacts.length !== 1) fail('bundle output has zero or multiple updater artifacts');
  const artifactName = artifacts[0];
  const signatureName = `${artifactName}.sig`;
  const artifactPath = join(bundleDir, artifactName);
  const signaturePath = join(bundleDir, signatureName);
  await assertOrdinaryPath(artifactPath, 'file');
  await assertOrdinaryPath(signaturePath, 'file');
  inspectUpdaterSignature(await readFile(signaturePath), artifactName, productVersion);

  await assertOrdinaryPath(dirname(stage), 'directory');
  await mkdir(stage, { recursive: false });
  await assertOrdinaryPath(stage, 'directory');
  const stagedArtifact = join(stage, artifactName);
  const stagedSignature = join(stage, signatureName);
  await copyFile(artifactPath, stagedArtifact, constants.COPYFILE_EXCL);
  await copyFile(signaturePath, stagedSignature, constants.COPYFILE_EXCL);
  for (const [source, destination] of [
    [artifactPath, stagedArtifact],
    [signaturePath, stagedSignature],
  ]) {
    const [before, after] = await Promise.all([
      hashOrdinaryFile(source),
      hashOrdinaryFile(destination),
    ]);
    if (before.bytes !== after.bytes || before.sha256 !== after.sha256)
      fail('staged updater file differs from bundle output');
  }
  const names = await stagedFileNames(stage);
  if (
    names.length !== 2 ||
    !names.includes(artifactName) ||
    !names.includes(signatureName)
  ) {
    fail('signed updater stage contains unexpected entries');
  }
  return {
    stage,
    stagedArtifact,
    stagedSignature,
    artifactName: `NexusOps-updater-${productVersion}-${platform}-${architecture}-${sourceCommit.slice(0, 8)}`,
  };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const stage = parseStageArgument(process.argv.slice(2));
    const result = await stageSignedUpdaterCandidate(stage);
    if (process.env.GITHUB_OUTPUT) {
      for (const value of [
        result.stage,
        result.stagedArtifact,
        result.stagedSignature,
        result.artifactName,
      ]) {
        if (/[\r\n]/.test(value)) fail('workflow output contains a newline');
      }
      await appendFile(
        process.env.GITHUB_OUTPUT,
        [
          `stage_dir=${result.stage}`,
          `artifact_path=${result.stagedArtifact}`,
          `signature_path=${result.stagedSignature}`,
          `artifact_name=${result.artifactName}`,
          '',
        ].join('\n'),
      );
    }
    console.log(`Staged one signed updater candidate: ${result.artifactName}`);
  } catch (error) {
    console.error(`Signed updater staging failed: ${error.message}`);
    process.exitCode = 1;
  }
}

import { constants } from 'node:fs';
import { appendFile, copyFile, mkdir, readdir } from 'node:fs/promises';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  REPOSITORY_ROOT,
  assertOrdinaryPath,
  detectPlatformArchitecture,
  hashOrdinaryFile,
  parseStageArgument,
  verifyReleaseSource,
  verifyReleaseVersion,
} from './release-common.mjs';
import { inspectUnsignedUpdaterStage, updaterArtifactBasename } from './signed-updater-candidate.mjs';

function fail(message) {
  throw new Error(message);
}

export async function stageUpdaterArtifact(
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
  const suffix = { windows: '-setup.exe', linux: '.AppImage', macos: '.app.tar.gz' }[platform];
  const entries = await readdir(bundleDir, { withFileTypes: true });
  if (entries.some((entry) => entry.name.endsWith('.sig')))
    fail('bundle output unexpectedly contains a pre-signing signature');
  const candidates = entries.filter((entry) => entry.name.endsWith(suffix));
  if (candidates.length !== 1) fail('bundle output has zero or multiple updater payloads');
  const artifactName = updaterArtifactBasename(candidates[0].name, identity);
  const artifactPath = join(bundleDir, artifactName);
  await assertOrdinaryPath(artifactPath, 'file');

  await assertOrdinaryPath(dirname(stage), 'directory');
  await mkdir(stage, { recursive: false });
  await assertOrdinaryPath(stage, 'directory');
  const stagedArtifact = join(stage, artifactName);
  await copyFile(artifactPath, stagedArtifact, constants.COPYFILE_EXCL);
  const [before, after] = await Promise.all([
    hashOrdinaryFile(artifactPath),
    hashOrdinaryFile(stagedArtifact),
  ]);
  if (before.bytes !== after.bytes || before.sha256 !== after.sha256)
    fail('staged updater payload differs from bundle output');
  await inspectUnsignedUpdaterStage(stage, identity);
  return {
    stage,
    stagedArtifact,
    stagedSignature: `${stagedArtifact}.sig`,
    artifactName: `NexusOps-updater-${productVersion}-${platform}-${architecture}-${sourceCommit.slice(0, 8)}`,
  };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const stage = parseStageArgument(process.argv.slice(2));
    const result = await stageUpdaterArtifact(stage);
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
    console.log(`Staged one unsigned updater payload: ${result.artifactName}`);
  } catch (error) {
    console.error(`Unsigned updater staging failed: ${error.message}`);
    process.exitCode = 1;
  }
}

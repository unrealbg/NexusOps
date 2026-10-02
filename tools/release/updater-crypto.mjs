import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { REPOSITORY_ROOT, assertOrdinaryPath } from './release-common.mjs';
import { RELEASE_TARGETS, releaseArtifactBasename } from './latest-json.mjs';
import { UPDATER_PUBLIC_KEY } from './updater-public-key.mjs';

export async function verifyUpdaterCryptography(
  artifact,
  signature,
  version,
  runner = (args) =>
    execFileSync('cargo', args, {
      cwd: REPOSITORY_ROOT,
      stdio: ['ignore', 'ignore', 'pipe'],
      windowsHide: true,
    }),
) {
  const key = join(REPOSITORY_ROOT, UPDATER_PUBLIC_KEY);
  await assertOrdinaryPath(key, 'file');
  await assertOrdinaryPath(artifact, 'file');
  await assertOrdinaryPath(signature, 'file');
  runner([
    'run', '--offline', '-q', '-p', 'nexus-core', '--example', 'verify_updater_signature', '--locked', '--',
    artifact, signature, key, version,
  ]);
}

export async function verifyReleaseCryptography(
  directory,
  version,
  runner = (args) =>
    execFileSync('cargo', args, {
      cwd: REPOSITORY_ROOT,
      stdio: ['ignore', 'ignore', 'pipe'],
      windowsHide: true,
    }),
) {
  const verified = [];
  for (const { target } of RELEASE_TARGETS) {
    const artifact = join(directory, releaseArtifactBasename(target, version));
    const signature = `${artifact}.sig`;
    await verifyUpdaterCryptography(artifact, signature, version, runner);
    verified.push(target);
  }
  return verified;
}

import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { REPOSITORY_ROOT, verifyReleaseVersion } from './release-common.mjs';
import { verifyTauriGeneration } from './tauri-generation.mjs';
import { verifyUpdaterPublicKey } from './updater-public-key.mjs';
import { verifyLifecycleSource } from './lifecycle-source.mjs';

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 2) throw new Error('verify-release-version accepts no arguments');
  try {
    const result = await verifyReleaseVersion(REPOSITORY_ROOT);
    const tauri = await verifyTauriGeneration(REPOSITORY_ROOT);
    const key = await verifyUpdaterPublicKey(REPOSITORY_ROOT);
    const lifecycle = await verifyLifecycleSource(REPOSITORY_ROOT);
    console.log(
      `Release version consistency: ${result.productName} ${result.productVersion}; Tauri ${tauri.tauri}; CLI ${tauri.cli}; updater public key SHA-256 ${key.sha256}; lifecycle-guarded commands ${lifecycle.commandCount}`,
    );
  } catch (error) {
    console.error(`Release version verification failed: ${error.message}`);
    process.exitCode = 1;
  }
}

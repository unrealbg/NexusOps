import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { REPOSITORY_ROOT, verifyReleaseVersion } from './release-common.mjs';

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 2) throw new Error('verify-release-version accepts no arguments');
  try {
    const result = await verifyReleaseVersion(REPOSITORY_ROOT);
    console.log(`Release version consistency: ${result.productName} ${result.productVersion}`);
  } catch (error) {
    console.error(`Release version verification failed: ${error.message}`);
    process.exitCode = 1;
  }
}

import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { REPOSITORY_ROOT, verifyReleaseSource } from './release-common.mjs';

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 2) throw new Error('verify-release-source accepts no arguments');
  try {
    console.log(`Clean release source: ${verifyReleaseSource(REPOSITORY_ROOT)}`);
  } catch (error) {
    console.error(`Release source verification failed: ${error.message}`);
    process.exitCode = 1;
  }
}

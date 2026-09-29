import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  detectPlatformArchitecture,
  parseStageArgument,
  verifyReleaseSource,
  verifyReleaseVersion,
} from './release-common.mjs';
import { verifySignedCandidate } from './signed-updater-candidate.mjs';
import { verifyUpdaterPublicKey } from './updater-public-key.mjs';

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const stage = parseStageArgument(process.argv.slice(2));
    const sourceCommit = verifyReleaseSource();
    const [{ productVersion }, { sha256: publicKeySha256 }] = await Promise.all([
      verifyReleaseVersion(),
      verifyUpdaterPublicKey(),
    ]);
    const identity = { productVersion, sourceCommit, ...detectPlatformArchitecture() };
    await verifySignedCandidate(stage, identity, publicKeySha256);
    console.log('Signed updater candidate metadata and files: PASS');
  } catch (error) {
    console.error(`Signed updater candidate verification failed: ${error.message}`);
    process.exitCode = 1;
  }
}

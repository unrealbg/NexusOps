import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  REPOSITORY_ROOT,
  detectPlatformArchitecture,
  parseStageArgument,
  verifyReleaseSource,
  verifyReleaseVersion,
} from './release-common.mjs';
import { createReleaseManifest } from './release-manifest.mjs';

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const stage = parseStageArgument(process.argv.slice(2));
    const sourceCommit = verifyReleaseSource(REPOSITORY_ROOT);
    const { productVersion } = await verifyReleaseVersion(REPOSITORY_ROOT);
    const { platform, architecture } = detectPlatformArchitecture();
    const manifest = await createReleaseManifest(stage, {
      productVersion,
      sourceCommit,
      platform,
      architecture,
    });
    console.log(
      `Created deterministic manifest for ${manifest.productVersion} ${platform}/${architecture}`,
    );
  } catch (error) {
    console.error(`Release manifest creation failed: ${error.message}`);
    process.exitCode = 1;
  }
}

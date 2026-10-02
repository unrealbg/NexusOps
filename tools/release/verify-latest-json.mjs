import { readFile } from 'node:fs/promises';
import { isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { assertOrdinaryPath } from './release-common.mjs';
import { readReleaseSignatures, verifyLatestJson } from './latest-json.mjs';

function argumentsFrom(argv) {
  if (argv.length !== 4 || argv[0] !== '--stage' || argv[2] !== '--version' || !isAbsolute(argv[1]))
    throw new Error('expected --stage <absolute-directory> --version <version>');
  return { stage: resolve(argv[1]), version: argv[3] };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const { stage, version } = argumentsFrom(process.argv.slice(2));
    await assertOrdinaryPath(stage, 'directory');
    const signatures = await readReleaseSignatures(stage, version);
    verifyLatestJson(await readFile(join(stage, 'latest.json')), version, signatures);
    console.log(`Verified canonical latest.json for ${version}`);
  } catch (error) {
    console.error(`latest.json verification failed: ${error.message}`);
    process.exitCode = 1;
  }
}

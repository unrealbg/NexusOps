import { appendFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  REPOSITORY_ROOT,
  assertSemver,
  verifyReleaseSource,
  verifyReleaseVersion,
} from './release-common.mjs';

function fail(message) {
  throw new Error(message);
}

function assertion(value, label) {
  if (typeof value !== 'string' || value.length === 0 || /[\x00-\x1f\x7f]/.test(value))
    fail(`${label} is missing or contains control characters`);
  return value;
}

export async function deriveReleaseIdentity({
  root = REPOSITORY_ROOT,
  environment = process.env,
  expectedSourceSha,
  expectedVersion,
  sourceVerifier = verifyReleaseSource,
  versionVerifier = verifyReleaseVersion,
} = {}) {
  expectedSourceSha = assertion(expectedSourceSha, 'expected source SHA');
  expectedVersion = assertion(expectedVersion, 'expected version');
  if (!/^[0-9a-f]{40}$/.test(expectedSourceSha)) fail('expected source SHA is not a full lowercase SHA');
  assertSemver(expectedVersion, 'expected version');
  if (expectedVersion.startsWith('v')) fail('expected version must not have a leading v');

  const sourceCommit = sourceVerifier(root, environment);
  const { productName, productVersion } = await versionVerifier(root);
  if (sourceCommit !== expectedSourceSha) fail('expected source SHA does not match repository source');
  if (environment.GITHUB_SHA !== undefined && environment.GITHUB_SHA !== expectedSourceSha)
    fail('expected source SHA does not match GITHUB_SHA');
  if (productVersion !== expectedVersion) fail('expected version does not match repository version');
  return { productName, productVersion, sourceCommit, tag: `v${productVersion}` };
}

function parseArguments(argv) {
  if (
    argv.length !== 4 ||
    argv[0] !== '--expected-source-sha' ||
    argv[2] !== '--expected-version'
  ) {
    fail('expected --expected-source-sha <sha> --expected-version <version>');
  }
  return { expectedSourceSha: argv[1], expectedVersion: argv[3] };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const identity = await deriveReleaseIdentity(parseArguments(process.argv.slice(2)));
    if (process.env.GITHUB_OUTPUT) {
      await appendFile(
        process.env.GITHUB_OUTPUT,
        `product_name=${identity.productName}\nproduct_version=${identity.productVersion}\nsource_commit=${identity.sourceCommit}\ntag=${identity.tag}\n`,
        'utf8',
      );
    }
    process.stdout.write(`${JSON.stringify(identity)}\n`);
  } catch (error) {
    console.error(`Release identity verification failed: ${error.message}`);
    process.exitCode = 1;
  }
}

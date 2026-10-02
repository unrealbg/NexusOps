import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { REPOSITORY_ROOT, assertSemver } from './release-common.mjs';
import { deterministicReleaseTitle, validateReleaseNotes } from './release-publication-common.mjs';

export async function createDraftReleaseRequest(version, sourceCommit, root = REPOSITORY_ROOT) {
  assertSemver(version, 'release version');
  if (!/^[0-9a-f]{40}$/.test(sourceCommit)) throw new Error('release source SHA is invalid');
  const body = await validateReleaseNotes(root, version);
  return {
    tag_name: `v${version}`,
    target_commitish: sourceCommit,
    name: deterministicReleaseTitle(version),
    body,
    draft: true,
    prerelease: false,
    make_latest: 'false',
  };
}

function parse(argv) {
  if (argv.length !== 4 || argv[0] !== '--version' || argv[2] !== '--source-commit')
    throw new Error('expected --version <version> --source-commit <sha>');
  return { version: argv[1], sourceCommit: argv[3] };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const { version, sourceCommit } = parse(process.argv.slice(2));
    process.stdout.write(`${JSON.stringify(await createDraftReleaseRequest(version, sourceCommit))}\n`);
  } catch (error) {
    console.error(`Draft release request creation failed: ${error.message}`);
    process.exitCode = 1;
  }
}

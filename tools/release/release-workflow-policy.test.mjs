import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { REPOSITORY_ROOT } from './release-common.mjs';
import { validateReleaseWorkflowPolicy, verifyReleaseWorkflowPolicy } from './release-workflow-policy.mjs';

async function sources() {
  return Object.fromEntries(
    await Promise.all(
      [
        ['signing', '.github/workflows/signed-updater-candidate.yml'],
        ['draft', '.github/workflows/prepare-updater-release-draft.yml'],
        ['publish', '.github/workflows/publish-updater-release.yml'],
        ['publishedVerifier', 'tools/release/verify-published-release.mjs'],
      ].map(async ([key, path]) => [key, await readFile(join(REPOSITORY_ROOT, path), 'utf8')]),
    ),
  );
}

test('three release-authority workflows satisfy least-privilege source policy', async () => {
  assert.deepEqual(await verifyReleaseWorkflowPolicy(), { signingCommands: 3, workflows: 3 });
});

for (const [label, mutate, pattern] of [
  ['hard-coded signer version', (s) => { s.signing = s.signing.replace('"$PRODUCT_VERSION"', '0.1.1'); }, /verified product version|hard-codes/],
  ['direct input signer version', (s) => { s.signing = s.signing.replace('"$PRODUCT_VERSION"', '"${{ inputs.expected_version }}"'); }, /verified product version|directly/],
  ['removed identity verification', (s) => { s.signing = s.signing.replace('release-identity.mjs', 'removed.mjs'); }, /identity assertions/],
  ['removed source assertion', (s) => { s.signing = s.signing.replace('expected_source_sha:', 'removed_source:'); }, /required expected_source_sha/],
  ['removed version assertion', (s) => { s.signing = s.signing.replace('expected_version:', 'removed_version:'); }, /required expected_version/],
  ['signing contents write', (s) => { s.signing = s.signing.replace('contents: read', 'contents: write'); }, /permissions|mutation authority/],
  ['secret in non-signing step', (s) => { s.signing += '\n      - name: Leak\n        run: echo $TAURI_SIGNING_PRIVATE_KEY\n'; }, /escaped/],
  ['draft signing secret', (s) => { s.draft += '\n# TAURI_SIGNING_PRIVATE_KEY\n'; }, /mixes/],
  ['existing-release rejection removal', (s) => { s.draft = s.draft.replace('A release already exists for the exact tag', 'continue with existing release'); }, /existing release/],
  ['empty-draft assertion removal', (s) => { s.draft = s.draft.replace('r.assets.length !== 0', 'false'); }, /empty release/],
  ['publish signing secret', (s) => { s.publish += '\n# TAURI_SIGNING_PRIVATE_KEY_PASSWORD\n'; }, /mixes/],
  ['automatic publish trigger', (s) => { s.publish = s.publish.replace('  workflow_dispatch:', '  workflow_dispatch:\n  push:'); }, /automatic trigger/],
  ['publish id token', (s) => { s.publish = s.publish.replace('permissions:', 'permissions:\n  id-token: write'); }, /mixes|permissions/],
  ['publish administration', (s) => { s.publish = s.publish.replace('permissions:', 'permissions:\n  administration: write'); }, /mixes|permissions/],
  ['removed immutability credential', (s) => { s.publish = s.publish.replace('secrets.IMMUTABILITY_READ_TOKEN', 'secrets.REMOVED_TOKEN'); }, /credential/],
  ['immutability credential at workflow scope', (s) => {
    s.publish = `env:\n  GH_TOKEN: \${{ secrets.IMMUTABILITY_READ_TOKEN }}\n${s.publish.replace('          GH_TOKEN: ${{ secrets.IMMUTABILITY_READ_TOKEN }}', '          GH_TOKEN: unavailable')}`;
  }, /step-only/],
  ['immutability credential at job scope', (s) => {
    s.publish = s.publish
      .replace('  publish:\n    runs-on:', '  publish:\n    env:\n      GH_TOKEN: ${{ secrets.IMMUTABILITY_READ_TOKEN }}\n    runs-on:')
      .replace('          GH_TOKEN: ${{ secrets.IMMUTABILITY_READ_TOKEN }}', '          GH_TOKEN: unavailable');
  }, /step-only/],
  ['immutability credential in another step', (s) => { s.publish += '\n      - name: Leaked authority\n        env:\n          GH_TOKEN: ${{ secrets.IMMUTABILITY_READ_TOKEN }}\n        run: exit 1\n'; }, /exactly once/],
  ['publication token for immutable settings', (s) => { s.publish = s.publish.replace('secrets.IMMUTABILITY_READ_TOKEN', 'github.token'); }, /publication GITHUB_TOKEN|credential/],
  ['removed immutable API version', (s) => { s.publish = s.publish.replace("            -H 'X-GitHub-Api-Version: 2026-03-10' \\\n", ''); }, /API version/],
  ['removed immutable Accept header', (s) => { s.publish = s.publish.replace("            -H 'Accept: application/vnd.github+json' \\\n", ''); }, /Accept header/],
  ['changed immutable endpoint', (s) => { s.publish = s.publish.replace("'repos/unrealbg/NexusOps/immutable-releases'", "'repos/unrealbg/Other/immutable-releases'"); }, /endpoint changed/],
  ['removed enabled validation', (s) => {
    s.publish = s.publish.replace('validateImmutableReleaseStatus(JSON.parse(readFileSync(process.argv[1], "utf8")))', 'JSON.parse(readFileSync(process.argv[1], "utf8"))');
  }, /enabled=true/],
  ...['PUT', 'POST', 'PATCH', 'DELETE'].map((method) => [
    `${method} immutable request`,
    (s) => { s.publish = s.publish.replace('--method GET', `--method ${method}`); },
    /explicit GET|mutates/,
  ]),
  ['removed immutable gate', (s) => { s.publish = s.publish.replaceAll('immutable-releases', 'removed-gate'); }, /isolate|immutable-release gate/],
  ['removed ambiguous activation handling', (s) => { s.publish = s.publish.replace('activation_status=$?', 'removed_status=0'); }, /ambiguous activation/],
  ['publish asset upload', (s) => { s.publish += '\n# gh release upload\n'; }, /mutation/],
  ['floating action', (s) => { s.draft = s.draft.replace(/actions\/checkout@[0-9a-f]{40}/, 'actions/checkout@v4'); }, /floating/],
  ['direct workflow-input shell interpolation', (s) => { s.publish += '\n      - run: echo "${{ inputs.expected_version }}"\n'; }, /directly into a run command/],
  ['post-publication admin endpoint call', (s) => { s.publishedVerifier += '\n// immutable-releases\n'; }, /admin-only/],
  ['removed release immutable assertion', (s) => { s.publishedVerifier = s.publishedVerifier.replace('release.immutable !== true', 'false'); }, /release\.immutable/],
]) {
  test(`source policy rejects ${label}`, async () => {
    const value = await sources();
    mutate(value);
    assert.throws(() => validateReleaseWorkflowPolicy(value), pattern);
  });
}

test('source policy rejects immutable preflight after publication', async () => {
  const value = await sources();
  const marker = '      - name: Read immutable-release status with separate read-only authority';
  const start = value.publish.indexOf(marker);
  const end = value.publish.indexOf('\n      - ', start + marker.length);
  const step = value.publish.slice(start, end);
  value.publish = `${value.publish.slice(0, start)}${value.publish.slice(end)}\n${step}\n`;
  assert.throws(() => validateReleaseWorkflowPolicy(value), /must precede publication/);
});

function removeNamedStep(source, name) {
  const marker = `      - name: ${name}`;
  const start = source.indexOf(marker);
  assert.notEqual(start, -1);
  const next = source.indexOf('\n      - ', start + marker.length);
  return `${source.slice(0, start)}${next < 0 ? '' : source.slice(next + 1)}`;
}

test('source policy rejects missing draft release-request preflight', async () => {
  const value = await sources();
  value.draft = removeNamedStep(value.draft, 'Prepare deterministic draft release request');
  assert.throws(() => validateReleaseWorkflowPolicy(value), /lacks required release-request/);
});

test('source policy rejects draft release-request preparation after tag mutation', async () => {
  const value = await sources();
  const name = 'Prepare deterministic draft release request';
  const marker = `      - name: ${name}`;
  const start = value.draft.indexOf(marker);
  const next = value.draft.indexOf('\n      - ', start + marker.length);
  const step = value.draft.slice(start, next);
  const without = `${value.draft.slice(0, start)}${value.draft.slice(next + 1)}`;
  const tag = '      - name: Create or verify exact lightweight release tag';
  const tagStart = without.indexOf(tag);
  const tagNext = without.indexOf('\n      - ', tagStart + tag.length);
  value.draft = `${without.slice(0, tagNext + 1)}${step}\n${without.slice(tagNext + 1)}`;
  assert.throws(() => validateReleaseWorkflowPolicy(value), /before tag mutation/);
});

test('source policy rejects release-request generation only after tag mutation', async () => {
  const value = await sources();
  const command = /          node tools\/release\/release-request\.mjs[\s\S]*?            > "\$RUNNER_TEMP\/release-request\.json"\r?\n/;
  const match = value.draft.match(command);
  assert.ok(match);
  value.draft = value.draft.replace(match[0], '          test -f docs/release/notes/v0.1.2.md\n');
  value.draft = value.draft.replace(
    '          gh api --method POST "repos/$GITHUB_REPOSITORY/releases"',
    `${match[0]}          gh api --method POST "repos/$GITHUB_REPOSITORY/releases"`,
  );
  assert.throws(() => validateReleaseWorkflowPolicy(value), /preflight command changed or is missing/);
});

test('source policy rejects duplicate post-tag release-request regeneration', async () => {
  const value = await sources();
  value.draft = value.draft.replace(
    '          gh api --method POST "repos/$GITHUB_REPOSITORY/releases"',
    '          node tools/release/release-request.mjs --version "$PRODUCT_VERSION" --source-commit "$SOURCE_COMMIT" > "$RUNNER_TEMP/release-request.json"\n          gh api --method POST "repos/$GITHUB_REPOSITORY/releases"',
  );
  assert.throws(() => validateReleaseWorkflowPolicy(value), /exactly once/);
});

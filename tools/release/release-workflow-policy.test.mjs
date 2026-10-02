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
  ['removed immutable gate', (s) => { s.publish = s.publish.replaceAll('immutable-releases', 'removed-gate'); }, /immutable-release gate/],
  ['removed ambiguous activation handling', (s) => { s.publish = s.publish.replace('activation_status=$?', 'removed_status=0'); }, /ambiguous activation/],
  ['publish asset upload', (s) => { s.publish += '\n# gh release upload\n'; }, /mutation/],
  ['floating action', (s) => { s.draft = s.draft.replace(/actions\/checkout@[0-9a-f]{40}/, 'actions/checkout@v4'); }, /floating/],
  ['direct workflow-input shell interpolation', (s) => { s.publish += '\n      - run: echo "${{ inputs.expected_version }}"\n'; }, /directly into a run command/],
]) {
  test(`source policy rejects ${label}`, async () => {
    const value = await sources();
    mutate(value);
    assert.throws(() => validateReleaseWorkflowPolicy(value), pattern);
  });
}

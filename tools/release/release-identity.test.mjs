import assert from 'node:assert/strict';
import test from 'node:test';
import { deriveReleaseIdentity } from './release-identity.mjs';

const SHA = 'a'.repeat(40);
const version = async () => ({ productName: 'NexusOps', productVersion: '0.1.1' });
const source = () => SHA;

test('identity is derived from verified repository state and assertions only compare', async () => {
  assert.deepEqual(
    await deriveReleaseIdentity({
      expectedSourceSha: SHA,
      expectedVersion: '0.1.1',
      environment: { GITHUB_SHA: SHA },
      sourceVerifier: source,
      versionVerifier: version,
    }),
    { productName: 'NexusOps', productVersion: '0.1.1', sourceCommit: SHA, tag: 'v0.1.1' },
  );
});

for (const [label, override, pattern] of [
  ['missing source assertion', { expectedSourceSha: undefined }, /expected source SHA is missing/],
  ['malformed source assertion', { expectedSourceSha: 'abc' }, /full lowercase SHA/],
  ['wrong source assertion', { expectedSourceSha: 'b'.repeat(40) }, /does not match repository source/],
  ['wrong Actions SHA', { environment: { GITHUB_SHA: 'b'.repeat(40) } }, /does not match GITHUB_SHA/],
  ['missing version assertion', { expectedVersion: undefined }, /expected version is missing/],
  ['leading-v version', { expectedVersion: 'v0.1.1' }, /not exact SemVer|leading v/],
  ['malformed version', { expectedVersion: '01.1.0' }, /not exact SemVer/],
  ['wrong version', { expectedVersion: '0.1.2' }, /does not match repository version/],
  ['control injection', { expectedVersion: '0.1.1\nEVIL=1' }, /control characters/],
]) {
  test(`identity rejects ${label}`, async () => {
    await assert.rejects(
      deriveReleaseIdentity({
        expectedSourceSha: SHA,
        expectedVersion: '0.1.1',
        environment: { GITHUB_SHA: SHA },
        sourceVerifier: source,
        versionVerifier: version,
        ...override,
      }),
      pattern,
    );
  });
}

test('dirty source is rejected by the source verifier', async () => {
  await assert.rejects(
    deriveReleaseIdentity({
      expectedSourceSha: SHA,
      expectedVersion: '0.1.1',
      environment: { GITHUB_SHA: SHA },
      sourceVerifier: () => {
        throw new Error('release source working tree is dirty');
      },
      versionVerifier: version,
    }),
    /working tree is dirty/,
  );
});

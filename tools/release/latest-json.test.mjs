import assert from 'node:assert/strict';
import test from 'node:test';
import {
  MAX_LATEST_JSON_BYTES,
  RELEASE_TARGETS,
  canonicalLatestJson,
  createLatestJson,
  createLatestManifest,
  immutableArtifactUrl,
  releaseArtifactBasename,
  releaseAssetAllowlist,
  verifyLatestJson,
} from './latest-json.mjs';

const VERSION = '0.1.1';

function signature(target, version = VERSION, artifact = releaseArtifactBasename(target, version)) {
  const signed = Buffer.alloc(74, 3);
  signed.write('Ed', 0, 'ascii');
  const global = Buffer.alloc(64, 5);
  return Buffer.from(
    [
      'untrusted comment: signature from tauri secret key',
      signed.toString('base64'),
      `trusted comment: timestamp:1700000000\tfile:${artifact}\tversion:${version}`,
      global.toString('base64'),
      '',
    ].join('\n'),
  ).toString('base64');
}

function signatures() {
  return Object.fromEntries(RELEASE_TARGETS.map(({ target }) => [target, signature(target)]));
}

function bytes(value) {
  return Buffer.from(`${JSON.stringify(value, null, 2)}\n`);
}

test('canonical manifest has exact target order, URLs, signatures and seven-asset allowlist', () => {
  const expected = signatures();
  const encoded = createLatestJson(VERSION, expected);
  const manifest = verifyLatestJson(encoded, VERSION, expected);
  assert.deepEqual(Object.keys(manifest), ['version', 'platforms']);
  assert.deepEqual(Object.keys(manifest.platforms), RELEASE_TARGETS.map(({ target }) => target));
  assert.deepEqual(releaseAssetAllowlist(VERSION), [
    'NexusOps.app.tar.gz',
    'NexusOps.app.tar.gz.sig',
    'NexusOps_0.1.1_amd64.AppImage',
    'NexusOps_0.1.1_amd64.AppImage.sig',
    'NexusOps_0.1.1_x64-setup.exe',
    'NexusOps_0.1.1_x64-setup.exe.sig',
    'latest.json',
  ]);
  assert.ok(encoded.toString().endsWith('\n'));
  assert.ok(encoded.length < MAX_LATEST_JSON_BYTES);
});

function validManifest() {
  return createLatestManifest(VERSION, signatures());
}

for (const [label, mutate, pattern] of [
  ['wrong version', (m) => { m.version = '0.1.2'; }, /version differs/],
  ['leading-v version', (m) => { m.version = 'v0.1.1'; }, /not exact SemVer/],
  ['missing platform', (m) => { delete m.platforms['linux-x86_64']; }, /missing or unexpected/],
  ['extra platform', (m) => { m.platforms['windows-aarch64'] = m.platforms['windows-x86_64']; }, /missing or unexpected/],
  ['wrong platform', (m) => { m.platforms['macos-aarch64'] = m.platforms['darwin-aarch64']; delete m.platforms['darwin-aarch64']; }, /missing or unexpected/],
  ['unexpected top-level field', (m) => { m.notes = 'no'; }, /missing or unexpected/],
  ['unexpected entry field', (m) => { m.platforms['windows-x86_64'].sha256 = 'a'.repeat(64); }, /missing or unexpected/],
  ['moving latest payload URL', (m) => { m.platforms['windows-x86_64'].url = 'https://github.com/unrealbg/NexusOps/releases/latest/download/NexusOps_0.1.1_x64-setup.exe'; }, /immutable release URL/],
  ['HTTP URL', (m) => { m.platforms['windows-x86_64'].url = m.platforms['windows-x86_64'].url.replace('https:', 'http:'); }, /origin policy/],
  ['wrong host', (m) => { m.platforms['windows-x86_64'].url = m.platforms['windows-x86_64'].url.replace('github.com', 'example.com'); }, /origin policy/],
  ['wrong repository', (m) => { m.platforms['windows-x86_64'].url = m.platforms['windows-x86_64'].url.replace('/NexusOps/', '/Other/'); }, /immutable release URL/],
  ['wrong tag', (m) => { m.platforms['windows-x86_64'].url = m.platforms['windows-x86_64'].url.replace('/v0.1.1/', '/v0.1.2/'); }, /immutable release URL/],
  ['credentials', (m) => { m.platforms['windows-x86_64'].url = m.platforms['windows-x86_64'].url.replace('https://', 'https://user@'); }, /origin policy/],
  ['port', (m) => { m.platforms['windows-x86_64'].url = m.platforms['windows-x86_64'].url.replace('github.com', 'github.com:443'); }, /explicit port|origin policy/],
  ['query', (m) => { m.platforms['windows-x86_64'].url += '?x=1'; }, /origin policy/],
  ['fragment', (m) => { m.platforms['windows-x86_64'].url += '#x'; }, /origin policy/],
  ['percent encoding', (m) => { m.platforms['windows-x86_64'].url = m.platforms['windows-x86_64'].url.replace('NexusOps_', 'NexusOps%5F'); }, /ambiguous|immutable/],
  ['path traversal', (m) => { m.platforms['windows-x86_64'].url = m.platforms['windows-x86_64'].url.replace('/NexusOps_0.1.1', '/../NexusOps_0.1.1'); }, /ambiguous|immutable/],
  ['wrong Windows basename', (m) => { m.platforms['windows-x86_64'].url = m.platforms['windows-x86_64'].url.replace('_x64-setup', '_arm64-setup'); }, /immutable release URL/],
  ['wrong Linux basename', (m) => { m.platforms['linux-x86_64'].url = m.platforms['linux-x86_64'].url.replace('_amd64', '_aarch64'); }, /immutable release URL/],
  ['wrong macOS basename', (m) => { m.platforms['darwin-aarch64'].url = m.platforms['darwin-aarch64'].url.replace('NexusOps.app', 'Other.app'); }, /immutable release URL/],
  ['signature URL', (m) => { m.platforms['windows-x86_64'].signature = `${m.platforms['windows-x86_64'].url}.sig`; }, /signature/],
  ['signature filename', (m) => { m.platforms['windows-x86_64'].signature = 'NexusOps_0.1.1_x64-setup.exe.sig'; }, /signature/],
  ['malformed signature', (m) => { m.platforms['windows-x86_64'].signature = 'not-base64'; }, /signature/],
  ['wrong signature artifact binding', (m) => { m.platforms['windows-x86_64'].signature = signature('windows-x86_64', VERSION, 'Other.exe'); }, /artifact\/version binding/],
  ['wrong signed version', (m) => { m.platforms['windows-x86_64'].signature = signature('windows-x86_64', '0.1.2', 'NexusOps_0.1.1_x64-setup.exe'); }, /artifact\/version binding/],
]) {
  test(`manifest rejects ${label}`, () => {
    const manifest = validManifest();
    mutate(manifest);
    assert.throws(() => verifyLatestJson(bytes(manifest), VERSION), pattern);
  });
}

test('manifest rejects candidate signature substitution', () => {
  const expected = signatures();
  const manifest = validManifest();
  manifest.platforms['windows-x86_64'].signature = signature('windows-x86_64').replace(/A/, 'B');
  assert.throws(() => verifyLatestJson(bytes(manifest), VERSION, expected));
});

for (const [label, transform, pattern] of [
  ['noncanonical whitespace', (b) => Buffer.from(b.toString().replace(/  /g, '    ')), /not canonical/],
  ['wrong field order', (b) => {
    const m = JSON.parse(b);
    return Buffer.from(`${JSON.stringify({ platforms: m.platforms, version: m.version }, null, 2)}\n`);
  }, /not canonical/],
  ['missing final LF', (b) => b.subarray(0, b.length - 1), /not canonical/],
  ['BOM', (b) => Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), b]), /BOM/],
  ['oversized manifest', () => Buffer.alloc(MAX_LATEST_JSON_BYTES + 1, 32), /size/],
]) {
  test(`manifest rejects ${label}`, () => {
    const canonical = createLatestJson(VERSION, signatures());
    assert.throws(() => verifyLatestJson(transform(canonical), VERSION), pattern);
  });
}

test('URL generation is immutable and tag-specific', () => {
  assert.equal(
    immutableArtifactUrl(VERSION, 'windows-x86_64'),
    'https://github.com/unrealbg/NexusOps/releases/download/v0.1.1/NexusOps_0.1.1_x64-setup.exe',
  );
});

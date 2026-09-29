import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { REPOSITORY_ROOT } from './release-common.mjs';

const WORKFLOW = join(REPOSITORY_ROOT, '.github/workflows/signed-updater-candidate.yml');

async function workflow() {
  return readFile(WORKFLOW, 'utf8');
}

test('signed candidate workflow is manual, pinned, minimally privileged and has no bootstrap', async () => {
  const source = await workflow();
  assert.match(source, /^on:\s*\n  workflow_dispatch:\s*$/m);
  assert.doesNotMatch(source, /^  (?:push|pull_request|schedule|release|create):/m);
  assert.match(source, /^permissions:\s*\n  contents: read\s*\n  id-token: write\s*\n  attestations: write\s*$/m);
  assert.doesNotMatch(source, /(?:contents|packages|pull-requests|issues|deployments): write/);
  const uses = [...source.matchAll(/^\s*- uses: ([^\s]+)$/gm)].map((match) => match[1]);
  assert.deepEqual(uses.map((reference) => reference.split('@')[0]), [
    'actions/checkout', 'actions/setup-node', 'actions/attest', 'actions/upload-artifact',
  ]);
  for (const reference of uses) assert.match(reference, /@[0-9a-f]{40}$/);
  assert.match(source, /command -v rustup\b/);
  assert.match(source, /rustup toolchain install 1\.98\.1[^\r\n]*--no-self-update/);
  assert.doesNotMatch(source, /sh\.rustup\.rs|actions-rust-lang\/setup-rust-toolchain/i);
  assert.doesNotMatch(source, /\b(?:curl|wget)\b[^\r\n]*\|\s*(?:sh|bash)\b/i);
  assert.doesNotMatch(source, /\bInvoke-WebRequest\b[^\r\n]*\|\s*(?:Invoke-Expression|iex|sh|bash)\b/i);
});

test('signing secrets exist only in bundle steps after secret-free quality and build', async () => {
  const source = await workflow();
  const firstSecret = source.indexOf('secrets.TAURI_SIGNING_PRIVATE_KEY');
  const lastBuild = source.lastIndexOf('build --no-bundle -- --locked');
  assert.ok(lastBuild > 0 && firstSecret > lastBuild);
  assert.match(source, /prepare-updater-bundle-config\.mjs --output/);
  assert.doesNotMatch(source, /beforeBundleCommand|beforeBuildCommand/);
  const sections = source.split(/^      - name: /m).slice(1);
  const signing = sections.filter((section) => section.includes('secrets.TAURI_SIGNING_PRIVATE_KEY'));
  assert.equal(signing.length, 3);
  for (const section of sections) {
    if (!section.includes('secrets.TAURI_SIGNING_PRIVATE_KEY')) {
      assert.doesNotMatch(section, /TAURI_SIGNING_PRIVATE_KEY(?:_PASSWORD)?/);
      continue;
    }
    assert.match(section, /^Bundle and sign (?:NSIS updater \(Windows\)|AppImage updater \(Linux\)|macOS updater archive)/);
    assert.match(section, /node tools\/release\/require-updater-signing-env\.mjs/);
    assert.match(section, /bundle --bundles (?:nsis|appimage|app) --config "\$BUNDLE_CONFIG"/);
    assert.doesNotMatch(section, /npm|cargo|build --no-bundle/);
  }
  assert.match(source, /node tools\/release\/stage-signed-updater-candidate\.mjs/);
  assert.match(source, /node tools\/release\/verify-signed-updater-candidate\.mjs/);
  assert.match(source, /\$\{\{ steps\.stage\.outputs\.signature_path \}\}/);
  assert.match(source, /\$\{\{ steps\.stage\.outputs\.stage_dir \}\}\/signed-updater-candidate\.json/);
  assert.doesNotMatch(source, /\blatest\.json\b|gh release|git tag|npm publish/);
});

test('each platform selects only its updater bundle format', async () => {
  const source = await workflow();
  assert.match(source, /if: runner\.os == 'Windows'[\s\S]*?bundle --bundles nsis --config/);
  assert.match(source, /if: runner\.os == 'Linux'[\s\S]*?bundle --bundles appimage --config/);
  assert.match(source, /if: runner\.os == 'macOS'[\s\S]*?bundle --bundles app --config/);
  assert.doesNotMatch(source, /bundle --bundles (?:all|msi|deb|rpm)/);
});

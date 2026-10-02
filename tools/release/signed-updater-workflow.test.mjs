import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { REPOSITORY_ROOT } from './release-common.mjs';

const WORKFLOW = join(REPOSITORY_ROOT, '.github/workflows/signed-updater-candidate.yml');
const STAGER = join(REPOSITORY_ROOT, 'tools/release/stage-updater-artifact.mjs');

async function workflow() {
  return readFile(WORKFLOW, 'utf8');
}

function workflowSteps(source) {
  const [beforeSteps, ...steps] = source.split(/^      - /m);
  assert.ok(steps.length > 0);
  return { beforeSteps, steps };
}

function namedStep(steps, name) {
  const matches = steps.filter((step) => step.split(/\r?\n/, 1)[0] === `name: ${name}`);
  assert.equal(matches.length, 1, `expected exactly one workflow step named ${name}`);
  return matches[0];
}

const PLATFORM_STEPS = [
  {
    os: 'Windows',
    bundleName: 'Bundle unsigned NSIS updater (Windows)',
    signName: 'Sign staged NSIS payload (Windows)',
    bundleFormat: 'nsis',
    cli: 'tauri.cmd',
  },
  {
    os: 'Linux',
    bundleName: 'Bundle unsigned AppImage updater (Linux)',
    signName: 'Sign staged AppImage payload (Linux)',
    bundleFormat: 'appimage',
    cli: 'tauri',
  },
  {
    os: 'macOS',
    bundleName: 'Bundle unsigned macOS updater archive',
    signName: 'Sign staged macOS payload',
    bundleFormat: 'app',
    cli: 'tauri',
  },
];

test('signed candidate workflow is manual, pinned, minimally privileged and has no bootstrap', async () => {
  const source = await workflow();
  assert.match(source, /^on:\s*\n  workflow_dispatch:\s*\n    inputs:/m);
  assert.match(source, /^      expected_source_sha:\s*\n(?:        .+\n)*        required: true$/m);
  assert.match(source, /^      expected_version:\s*\n(?:        .+\n)*        required: true$/m);
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

test('signed updater staging uses the canonical workflow artifact-name helper', async () => {
  const source = await readFile(STAGER, 'utf8');
  assert.match(source, /\bupdaterCandidateArtifactName\b/);
  assert.match(source, /artifactName:\s*updaterCandidateArtifactName\(identity\)/);
  assert.doesNotMatch(source, /artifactName:\s*`NexusOps-updater-/);
});

test('bundling and staging are secret-free before three narrow detached signer steps', async () => {
  const source = await workflow();
  const { beforeSteps, steps } = workflowSteps(source);
  assert.doesNotMatch(beforeSteps, /TAURI_SIGNING_PRIVATE_KEY/);
  const identity = namedStep(steps, 'Derive verified release identity');
  assert.match(identity, /^        id: identity$/m);
  assert.match(identity, /release-identity\.mjs/);
  assert.match(identity, /^          EXPECTED_SOURCE_SHA: \$\{\{ inputs\.expected_source_sha \}\}$/m);
  assert.match(identity, /^          EXPECTED_VERSION: \$\{\{ inputs\.expected_version \}\}$/m);
  assert.match(identity, /--expected-source-sha "\$EXPECTED_SOURCE_SHA"/);
  assert.match(identity, /--expected-version "\$EXPECTED_VERSION"/);
  assert.doesNotMatch(identity, /run:[\s\S]*\$\{\{ inputs\./);
  assert.match(source, /prepare-updater-bundle-config\.mjs --output/);
  assert.doesNotMatch(source, /beforeBundleCommand|beforeBuildCommand/);
  const expectedSignerNames = PLATFORM_STEPS.map(({ signName }) => signName);
  const secretBearing = steps.filter((step) => /TAURI_SIGNING_PRIVATE_KEY/.test(step));
  assert.deepEqual(secretBearing.map((step) => /^name: ([^\r\n]+)/.exec(step)?.[1]), expectedSignerNames);
  const signerSteps = steps.filter((step) => /\btauri(?:\.cmd)?\s+signer sign\b/.test(step));
  assert.deepEqual(signerSteps, secretBearing);
  assert.equal([...source.matchAll(/\$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY \}\}/g)].length, 3);
  assert.equal([...source.matchAll(/\$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY_PASSWORD \}\}/g)].length, 3);
  for (const step of steps) {
    if (!secretBearing.includes(step)) assert.doesNotMatch(step, /TAURI_SIGNING_PRIVATE_KEY/);
  }

  const bundleSteps = steps.filter((step) => /\btauri(?:\.cmd)?\s+bundle\b/.test(step));
  assert.equal(bundleSteps.length, 3);
  const stageStep = namedStep(steps, 'Stage one unsigned updater payload');
  assert.match(stageStep, /node tools\/release\/stage-updater-artifact\.mjs --stage/);
  assert.match(stageStep, /^        id: stage$/m);
  const stageIndex = steps.indexOf(stageStep);
  assert.ok(stageIndex > Math.max(...bundleSteps.map((step) => steps.indexOf(step))));
  const lastBuild = source.lastIndexOf('build --no-bundle -- --locked');
  assert.ok(lastBuild > 0 && source.indexOf('secrets.TAURI_SIGNING_PRIVATE_KEY') > lastBuild);

  for (const { os, bundleName, signName, bundleFormat, cli } of PLATFORM_STEPS) {
    const bundle = namedStep(steps, bundleName);
    assert.ok(bundleSteps.includes(bundle));
    assert.match(bundle, new RegExp(`^        if: runner\\.os == '${os}'$`, 'm'));
    const bundleCommands = bundle.split(/\r?\n/).filter((line) => /\btauri(?:\.cmd)?\s+bundle\b/.test(line));
    assert.equal(bundleCommands.length, 1, `${bundleName} must call Tauri bundle once`);
    const bundleCommand = bundleCommands[0];
    assert.match(bundleCommand, new RegExp(`\\b${cli.replace('.', '\\.')} bundle\\b`));
    assert.match(bundleCommand, new RegExp(`\\bbundle --bundles ${bundleFormat}\\b`));
    assert.match(bundleCommand, /--no-sign\b/);
    assert.match(bundleCommand, /--config "\$BUNDLE_CONFIG"/);
    assert.doesNotMatch(bundle, /TAURI_SIGNING_PRIVATE_KEY|\bsecrets\./);

    const signer = namedStep(steps, signName);
    assert.ok(steps.indexOf(signer) > stageIndex);
    assert.match(signer, new RegExp(`^        if: runner\\.os == '${os}'$`, 'm'));
    assert.match(signer, /^          STAGED_ARTIFACT: \$\{\{ steps\.stage\.outputs\.artifact_path \}\}$/m);
    assert.match(signer, /^          PRODUCT_VERSION: \$\{\{ steps\.identity\.outputs\.product_version \}\}$/m);
    assert.match(signer, /^          TAURI_SIGNING_PRIVATE_KEY: \$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY \}\}$/m);
    assert.match(signer, /^          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: \$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY_PASSWORD \}\}$/m);
    assert.match(signer, /^\s*node tools\/release\/require-updater-signing-env\.mjs$/m);
    assert.equal([...signer.matchAll(/\$\{\{ secrets\./g)].length, 2);
    const signCommand = signer.split(/\r?\n/).find((line) => new RegExp(`\\b${cli.replace('.', '\\.')} signer sign\\b`).test(line));
    assert.ok(signCommand, `${signName} must call the detached Tauri signer`);
    assert.match(signCommand, /--app-version "\$PRODUCT_VERSION"/);
    assert.doesNotMatch(signCommand, /--app-version\s+(?:v?\d+\.\d+\.\d+|"?\$\{\{\s*inputs\.)/);
    assert.match(signCommand, /"\$STAGED_ARTIFACT"/);
    const runBody = signer.split(/^        run: \|\r?\n/m)[1];
    assert.ok(runBody, `${signName} needs a shell command block`);
    assert.deepEqual(runBody.trim().split(/\r?\n/).map((line) => line.trim()), [
      'node tools/release/require-updater-signing-env.mjs',
      `./node_modules/.bin/${cli} signer sign --app-version "$PRODUCT_VERSION" "$STAGED_ARTIFACT"`,
    ]);
    assert.doesNotMatch(signer, /\btauri(?:\.cmd)?\s+(?:bundle|build)\b/i);
    assert.doesNotMatch(signer, /\b(?:npm|npx|pnpm|yarn|bun|cargo|rustup|curl|wget|linuxdeploy|apt(?:-get)?|dnf|yum|pacman|apk|brew|choco|winget|scoop|pip3?|corepack)\b/i);
  }

  const metadataStep = namedStep(steps, 'Create and verify deterministic signed-candidate metadata');
  assert.ok(steps.indexOf(metadataStep) > Math.max(...secretBearing.map((step) => steps.indexOf(step))));
  assert.match(metadataStep, /node tools\/release\/create-signed-updater-candidate\.mjs --stage/);
  assert.match(metadataStep, /node tools\/release\/verify-signed-updater-candidate\.mjs --stage/);
  const attest = steps.find((step) => step.startsWith('uses: actions/attest@'));
  const upload = steps.find((step) => step.startsWith('uses: actions/upload-artifact@'));
  assert.ok(attest && upload);
  assert.ok(steps.indexOf(attest) > steps.indexOf(metadataStep));
  assert.ok(steps.indexOf(upload) > steps.indexOf(attest));
  const subjects = /^          subject-path: \|\r?\n((?:^            [^\r\n]+\r?\n?)*)/m.exec(attest)?.[1]
    ?.trim().split(/\r?\n/).map((line) => line.trim());
  assert.deepEqual(subjects, [
    '${{ steps.stage.outputs.artifact_path }}',
    '${{ steps.stage.outputs.signature_path }}',
    '${{ steps.stage.outputs.stage_dir }}/signed-updater-candidate.json',
  ]);
  assert.doesNotMatch(source, /bundle --bundles (?:all|msi|deb|rpm)|\blatest\.json\b|gh release|git tag|npm publish/);
});
